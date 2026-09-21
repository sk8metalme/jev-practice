# jevx 要件定義と実装状況

## 目的

`jevx`は、Codex CLIの日常利用を快適にしながら、Jevの実用性を測定するためのmacOS向けCLIである。主目的は、Skill選択・Hook運用・Compaction後のコンテキスト補助について、Jevを使わない場合との差分を安全な計測で確認すること。

Skill選択のコアはShadow Modeとし、`selected`になってもSkill本文の自動ロード・実行や会話の書き換えは行わない。Codex HookとCompaction補助は、利用者が設定をreview/trustした場合だけ動くopt-inの試作機能として境界を分ける。

## 対象範囲

- Rust製CLIを`jevx/`に提供する
- プロジェクトSkillとユーザー共通Skillを探索する
- ローカル候補絞り込みとJevのChoice判定を行う
- `none`による未推薦と確率・marginの安全側閾値を持つ
- 人間向け出力と`--json`出力に判定・Jev応答速度・全体速度を出す
- prompt本文を保存しないJSONL Telemetryとstats集計を提供する
- Codex向けadvisory SkillとmacOS用セットアップスクリプトを提供する
- 既存Hook設定を保持してjevx Hookを冪等に登録する `hooks install` を提供する
- `PreCompact` / `PostCompact` / `SessionStart(source=compact)` の安全なcheckpointを記録する
- 任意の`.jevx/compact-context.md`をredactし、compact後の補助contextとして返す
- 40ケースの比較評価と、APIキーありの単回・複数回実測を記録する

次の機能は本要件の対象外で、別途安全性を評価してから検討する。

- Skillの自動ロード・実行
- 会話全文を使ったLLM要約
- Tool Resultの自動削減・破棄
- MCPサーバー化
- Windows対応

## CLI

```bash
jevx skills suggest --prompt "PDFを結合して" --json
jevx skills suggest --stdin
jevx skills suggest --input-json
jevx skills list --json
jevx stats --json
jevx stats --input /tmp/events.jsonl --json
jevx doctor --json
jevx hooks install --scope user --dry-run --json
jevx hooks shadow
jevx hooks compact-assist --state-dir /tmp/jevx-compaction
```

Jev判定は`AI_GATEWAY_API_KEY`を使い、既存アプリと同じVercel AI Gatewayの`typesafe-ai/jev`へ送信する。`hooks install`は外部送信を行わず、`UserPromptSubmit`に登録された`hooks shadow`だけがJev判定を行う。

デフォルトの設定は次のとおり。

- 候補上限: 32件
- 選択確率の下限: 0.60
- 1位と2位の確率差: 0.10以上
- リクエストタイムアウト: 1,500ms
- Telemetry: `~/.jevx/events.jsonl`
- Hook state: Telemetry親ディレクトリ配下の`hooks.jsonl` / `compaction/`

## Hook設定要件

`hooks install`は次のルールを満たす。

| 要件 | 内容 |
| --- | --- |
| 保存先 | user: `$CODEX_HOME/hooks.json`、未設定時`~/.codex/hooks.json`; project: `<repo>/.codex/hooks.json` |
| 既存設定 | root、カスタムHook、未知のイベントを保持する |
| 重複 | 既存のjevx `shadow` / `compact-assist` handlerだけを置換し、再実行を冪等にする |
| 復旧 | 変更時の既存ファイルを`hooks.json.jevx.bak`へ初回だけ退避する |
| 安全確認 | `--dry-run`と`--json`で変更前に確認でき、実行後はCodex `/hooks`でTrustを確認する |
| matcher | `SessionStart`: `startup|resume|clear|compact`; `PreCompact`/`PostCompact`: `manual|auto` |

project-local HookはCodex側のプロジェクトTrustが必要であり、ファイル生成成功だけでは発火成功を意味しない。

## Compaction補助要件

`compact-assist`は要約器ではなく、次を行う決定的な補助とする。

1. Hook JSONを検証し、既存の安全なshadow record形式へ変換する。
2. session / turn / correlation / cwdはSHA-256、イベント識別子は安全なラベルだけ保存する。
3. `.jevx/compact-context.md`があればredact後に最大4,000文字まで利用する。
4. checkpointへmanifest本文や秘密値を保存しない。
5. `SessionStart(source=compact)`では、最新checkpoint metadataとredacted manifestだけを`additionalContext`へ返す。
6. contextがない場合もCodex処理を止めず、補助情報がないことを明示する。

公式Compactionの代替、会話全文の復元、現在のリポジトリ状態の保証はしない。

## データ保護

Jevへ送るのは、マスキング済みの現在の依頼文、作業ディレクトリ、Skillの名前・説明だけとする。Skill本文、過去の会話、Tool結果、APIキーは送信しない。

Telemetryには生の依頼文を保存せず、SHA-256、文字数、候補数、判定、確率、遅延、usageだけを保存する。Hook recordとCompaction checkpointには生のsession ID、turn ID、model、manifest本文を保存しない。

## 成功条件

- ローカルSkill探索のp95が100ms以内
- Jevを含む全体処理のp95が2秒以内
- `jevResponseMs`が人間向け・JSON出力・Telemetryに存在する
- Jev未設定・タイムアウト時に明示エラーを返し、Hook shadowはCodex処理を継続する
- 既存のローカルキーワード方式との精度・遅延・Token usage比較が可能
- `hooks install --dry-run`が変更せず、実行後の再実行が冪等である
- Compaction checkpointとredacted manifestが秘密情報を再出力しない
- 新規Rust実行コードのラインカバレッジ98%以上

## 評価結果（2026-09-21）

同梱40ケース（合成30、匿名化テンプレート10）では次の差分を観測した。詳細は[`jevx-comfort-evaluation.md`](jevx-comfort-evaluation.md)に分けている。

| 方式 | 正解率 | `none`精度 | 速度・コスト |
| --- | ---: | ---: | --- |
| 選択なし | 10.0% | 100.0% | 外部通信なし |
| ローカルキーワード | 90.0% | 75.0% | 外部通信なし |
| jevx + Jev（1回） | 100.0% | 100.0% | Jev p50/p95 407/673ms、平均input/output 2401.2/127.7 |
| jevx + Jev（5回平均） | 98.0% | 100.0% | Jev p50/p95 427/610ms、error rate 2.0%、探索p95 2ms |

これは固定fixtureの観測であり、利用者入力への精度・性能保証ではない。Jevの外部通信・Token使用量・Gatewayエラーを導入判断のコストとして同時に扱う。

## 評価方法

測定項目は、Top-1精度、`none`判定率、候補漏れ率、Jev p50/p95、全体p50/p95、入力Token、出力Token、エラー率、Hook継続率、checkpointの秘密値漏えい件数とする。APIキーありの実測はシークレット値・生prompt・生会話をリポジトリへ保存しない。

```bash
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --all-targets --fail-under-lines 98
```

公式のHook契約は[Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)、Compactionの概念とAPI仕様は[OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)を参照する。
