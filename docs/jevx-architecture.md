# jevx アーキテクチャ

## 目的と境界

jevxはCodex CLIの前段に置く「Skill選択とコンテキスト運用の補助線」だよ。依頼から候補Skillを探索し、ローカルの軽量スコアで上位候補を絞ってから、候補を1回のbatched ChoiceとしてJevへ渡す。Hookを明示的に登録した場合は、同じCLIでHook lifecycleとCompaction checkpointも観測する。

Skill選択の通常出力は提案だけで、Skill本文のロード、実行権限の付与、Hookによる自動実行は行わない。`hooks install`は設定変更を伴うためopt-inで、`compact-assist`はLLM要約ではなく決定的なmetadata復元である。

## 処理フロー

```text
ユーザー依頼
    │
    ├─ 入力検証・秘密らしいtokenのマスキング
    ├─ .agents/skills / .codex/skills を探索
    ├─ name・descriptionのローカルスコアリング
    ├─ 上位32件へ縮約
    ├─ 1回のJev Choice（候補 + none）
    ├─ probability >= 0.60 かつ margin >= 0.10 を確認
    │       ├─ 条件成立: selected
    │       └─ それ以外: none（理由コード付き）
    └─ JSON/human出力 + メタデータTelemetry

Codex Hook（明示的にinstall）
    │
    ├─ UserPromptSubmit
    │    └─ hooks shadow ─ 候補探索 + Jev判定 + continue固定応答
    ├─ PreCompact / PostCompact
    │    └─ compact-assist ─ metadata + redacted context hashを記録
    └─ SessionStart(source=compact)
         └─ 最新checkpoint + redacted manifestをadditionalContextへ返す
```

## モジュール責務

| モジュール | 責務 | 外部依存 |
| --- | --- | --- |
| `discovery` | Skillの探索、Frontmatterの検証、優先順位重複排除 | filesystem |
| `ranking` | ローカル候補順位、Jev結果の閾値判定、計測 | `Judge` trait |
| `gateway` | Vercel AI GatewayへのHTTPリクエストとレスポンス変換 | reqwest |
| `telemetry` | JSONL追記、判定率、p50/p95、Token集計 | filesystem |
| `evaluation` | fixture読み込み、ベースライン比較、Jev実測、p50/p95集計 | `Judge` trait、filesystem |
| `hooks` | Codex Hook入力の検証、shadow record、相関集計 | filesystem |
| `hook_config` | user/project hooks.jsonの安全なmerge、backup、冪等化 | filesystem |
| `compact_assist` | checkpoint、redaction、compact後の補助context | filesystem |
| `cli` | 入力形式、出力形式、終了コード、標準入力 | clap |

中心の`ranking`は`Judge` traitへ依存するオニオン型の内側に置き、テストではネットワークを使わないStub Judgeへ差し替えられる。HTTP境界はローカルTCPモックで検証し、CLIの成功経路も別テストで確認する。

## Hook設定の境界

`hook_config`は次の順に設定を処理する。

1. scopeから `$CODEX_HOME/hooks.json` / `~/.codex/hooks.json` / `<repo>/.codex/hooks.json` を決定する。
2. 既存JSONを読み、rootと未知のフィールドを保持する。
3. jevxが以前生成したcommand handlerだけをmarkerで除去する。
4. `SessionStart`、`PreCompact`、`PostCompact`、`UserPromptSubmit`の生成groupを追加する。
5. 実書き込み時だけ親ディレクトリを作成し、既存ファイルの初回backupを作成する。

`--dry-run`は4まで実行して生成後のJSONを返すが、backup・ディレクトリ作成・ファイル書き込みを行わない。したがって利用者は変更内容と書き込み先を先にレビューできる。

## Compaction補助のデータフロー

```text
Codex Hook JSON
    │
    ├─ run_shadow: event契約とsafe recordを検証
    ├─ .jevx/compact-context.mdを読む（任意）
    ├─ redact → 4,000文字制限 → SHA-256
    ├─ hook-records.jsonl / checkpoints.jsonlへmetadataのみ追記
    └─ SessionStart(source=compact)なら additionalContext
```

checkpointへ保存するのはevent名、safeなtrigger/source、各種SHA-256、文字数、context有無だけ。redacted本文はcheckpointへ保存せず、compact後のHook応答を組み立てるプロセス内でだけ使う。contextがない場合は「metadata not found」を返すが、Codexを停止しない。

## Skill探索の優先順位

1. 現在のプロジェクト `.agents/skills`
2. 現在のプロジェクト `.codex/skills`
3. ユーザー `~/.agents/skills`
4. `$CODEX_HOME/skills` または `~/.codex/skills`
5. `--skill-dir` で指定した追加ディレクトリ

同じSkill名が複数に存在するときは、優先順位の小さいルートを採用する。SkillファイルはYAML Frontmatterに非空の`name`と`description`があるものだけを候補にする。

## Jevへ渡すデータ

渡すデータは、マスキング済みの依頼文、作業ディレクトリ、候補SkillのID・名前・説明だけ。次のデータはv1で渡さない。

- Skill本文
- 過去の会話全文
- Tool結果やファイル内容
- APIキー
- 生のTelemetry本文

Jev未設定時はローカル推測へフォールバックせず、`missing_api_key`を返す。これは「Jevの有用性を測る」目的で、ローカルだけの結果をJev結果と混同しないためだよ。

## レイテンシ設計

- ローカル候補探索の目標: p95 100ms以下
- Jevを含む全体の目標: p95 2秒以下
- デフォルトHTTPタイムアウト: 1,500ms
- 出力する時刻: `discoveryMs`、`jevResponseMs`、`totalMs`
- Telemetry集計: 判定率、Jev/total p50・p95、平均input/output tokens、usage event数

Jevの呼び出しは候補ごとに繰り返さず、候補を1つのChoice質問へまとめる。これで候補数に比例したネットワーク往復を避けつつ、速度とtoken usageを測定できる。

## 安全側の判定

次の場合は`selected`を返さず`none`にする。

- Jevが`none`を選ぶ
- choiceが欠落する
- 未知のSkill IDを返す
- 1位の確率が0.60未満
- 1位とrunner-upの確率差が0.10未満

明示的な`--skill`指定はJevを呼ばず、`explicit`として返す。利用者の明示指定をモデルの推測で上書きしないためだよ。

## 失敗時の契約

| 状況 | JSON `error.code` | 終了コード |
| --- | --- | ---: |
| 入力不正、APIキーなし、JSON不正 | `invalid_input` / `missing_api_key` / `json_error` | 2 |
| Jev接続失敗、Jev応答エラー | `provider_error` | 3 |
| タイムアウト | `timeout` | 3 |
| ファイル・YAMLエラー | `io_error` / `yaml_error` | 2 |

既知Hook eventでJevが失敗しても、shadow/compact-assistはsafe recordと`continue: true`を優先する。不明eventや壊れた設定は入力契約違反としてエラーにする。

## 今後の境界

次は実ユーザーに近い匿名化ケースを増やし、Hookの連続利用遅延、Gateway rate limit、API費用、compact後の再現性を測定する。Skill自動ロード・実行、会話全文のLLM要約、Tool Result削減、MCP化は、権限境界と失敗時の復旧を別要件として定義する。

公式Hook契約は[Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)、Compaction仕様は[OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)を参照する。
