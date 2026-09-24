# jevx アーキテクチャ

> 対象読者: 開発者・保守担当
> 文書の状態: 現行仕様

## 目的と境界

jevxはCodex CLIの前段に置く「速度・意味レビュー・route・コンテキスト運用の補助線」だよ。依頼から対象を決定的に抽出し、必要な場合だけredact済みstateを1回のbatched typed requestとしてJevへ渡す。Hookを明示的に登録した場合は、Hook lifecycle、Compaction checkpoint、費用、route適用証拠も観測する。

Skill選択の通常出力は提案だけで、Skill本文のロード、実行権限の付与、Hookによる自動実行は行わない。`hooks install`は設定変更を伴うためopt-inで、`compact-assist`はLLM要約ではなく決定的なmetadata復元である。

## 処理フロー

```text
ユーザー依頼
    │
    ├─ 入力検証・秘密らしいtokenのマスキング
    ├─ .agents/skills / .codex/skills を探索
    ├─ name・descriptionのローカルスコアリング
    ├─ 上位32件へ縮約（超過はcandidate_window_overflow）
    ├─ redaction / byte budget / windowをStatePlanへ固定
    ├─ DecisionContractのChoice（候補 + none）を1回送信
    ├─ code-side probability / margin gate
    │       ├─ 条件成立: accepted → selected
    │       └─ それ以外: none / unknown / defer / degraded
    └─ JSON/human出力 + events.jsonl + decisions.jsonl receipt

意味レビュー/route（明示的にreviewまたはinstall --allow-content）
    │
    ├─ target（prompt/plan/diff/final-answer/turn）をコードで選ぶ
    ├─ local_contentはredact済みプロセス内だけ。外部contentはopt-in時だけ
    ├─ 4 predicate + route scoreを1回のtyped requestへまとめる
    ├─ code-side status / fallback / route evidence / fix gate
    └─ reviews.jsonlへ本文なしreceipt + Jev/Codex/total cost

Codex Hook（明示的にinstall）
    │
    ├─ UserPromptSubmit
    │    └─ hooks shadow ─ 候補探索 + Jevのchoice判定 + known eventのcontinue固定応答
    ├─ PreCompact / PostCompact
    │    └─ compact-assist ─ metadata + redacted context hashを記録
    └─ SessionStart(source=compact)
         └─ 最新checkpoint + redacted manifestをadditionalContextへ返す
```

## モジュール責務

| モジュール | 責務 | 外部依存 |
| --- | --- | --- |
| `discovery` | Skillの探索、Frontmatterの検証、優先順位重複排除 | filesystem |
| `decision` | typed Question/Answer、policy gate、safe status、cache、live/dry-run/replay実行 | serde / async-trait |
| `ranking` | ローカル候補順位、StatePlan、Choice adapter、結果互換変換 | `Judge` trait、Decision Contract |
| `gateway` | typed questionのHTTPリクエスト、429/529 retry、レスポンス変換 | reqwest |
| `recorder` | safe DecisionReceiptのJSONL追記、stats、replay mismatch検証 | filesystem |
| `telemetry` | JSONL追記、判定率、p50/p95、Token集計 | filesystem |
| `evaluation` | fixture読み込み、ベースライン比較、Jev実測、p50/p95集計 | `Judge` trait、filesystem |
| `cost` | 推定/実費、通貨/価格版、unknown/unavailable、Jev/Codex/合算 | pure functions |
| `review` | 4カテゴリのlocal finding、typed contract、receipt、task/turn/session集計、safe fix | `DecisionJudge`、filesystem |
| `route` | difficulty→model/reasoning/fallbackと適用証拠の判定 | pure functions |
| `hooks` | Codex Hook入力の検証、shadow record、相関集計 | filesystem |
| `hook_config` | user/project hooks.jsonの既存設定を保持するmerge、backup、冪等化 | filesystem |
| `compact_assist` | checkpoint、redaction、compact後の補助context | filesystem |
| `data` | `$JEVX_HOME` の管理ファイルの一覧・書き出し・削除 | filesystem |
| `cli`（`cli/mod.rs` と1コマンド1ファイル） | 入力形式、出力形式、終了コード、標準入力、doctorの診断 | clap |

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
    ├─ run_shadow: event契約を検証し、cwd・候補Skill・判定設定を含むkeyで正常なUserPromptSubmitを短期dedupeし、Hook recordを追記
    ├─ .jevx/compact-context.mdを読む（任意）
    ├─ usage/cost payloadをtyped metadataへ正規化し、前後Token削減量を計算
    ├─ redact → 4,000文字制限 → SHA-256
    ├─ hook-records.jsonl / checkpoints.jsonlへmetadataのみ追記
    └─ SessionStart(source=compact)なら additionalContext、statsはp50/p95とcost statusを集計（hit時のcall metricsは再利用しない）
```

checkpointへ保存するのはevent名、boundedなtrigger/source、各種SHA-256、文字数、context有無、任意のtyped usage/cost metadataだけ。Hook recordのselectedSkillもwrite前にbounded identifierへ正規化し、unsafeな値は欠損化する。appendはunsafe recordを拒否し、既存schema v1/v2のloadはtrigger/source/selectedSkillを正規化してから分析へ渡すため、過去ログ1件で全体を壊さない。redacted本文はcheckpointへ保存せず、compact後のHook応答を組み立てるプロセス内でだけ使う。contextがない場合は「metadata not found」を返すが、Codexを停止しない。

## Skill探索の優先順位

1. 現在のプロジェクト `.agents/skills`
2. 現在のプロジェクト `.codex/skills`
3. ユーザー `~/.agents/skills`
4. `$CODEX_HOME/skills` または `~/.codex/skills`
5. `--skill-dir` で指定した追加ディレクトリ

同じSkill名が複数に存在するときは、優先順位の小さいルートを採用する。SkillファイルはYAML Frontmatterに非空の`name`と`description`があるものだけを候補にする。`eval --skill-dir` はこの既定rootsを置き換えず追加するため、再現性が必要な評価では一時 `HOME`、`CODEX_HOME`、空のproject rootを用意して既定rootsを隔離する。

## Jevへ渡すデータ

渡すデータは、認識済みパターンをredactした依頼文、redactした作業ディレクトリ、候補SkillのID・名前、redactした説明だけ。prompt・cwd・descriptionのredactionは限定的なsecret patternに過ぎない。次のデータはv1で渡さない。

- Skill本文
- 過去の会話全文
- Tool結果やファイル内容
- APIキー
- 生のTelemetry本文

| データ | Gatewayへ送るか | 保存境界 |
| --- | --- | --- |
| prompt | 送る（redact後） | Telemetryには本文を保存せずSHA-256・文字数だけ |
| `cwd` | 送る（redact後） | receiptには本文を保存せずstate digestだけ |
| Skill ID / name | 送る（raw） | 候補識別に使用 |
| Skill description | 送る（redact後） | Skill本文は送らない |
| 会話全文 / raw Tool結果 / APIキー | 送らない | 常に対象外 |
| reviewで明示選択したprompt/plan/diff/final answer | `--allow-content`時だけredactして送る | Skill本文・設定本文は常に対象外。receiptにも本文を保存しない |
| probability | Gateway応答にはあり得るが送信対象ではない | Telemetry schemaへ保存しない |

Basic redactionは `Authorization=Basic <value>` / `Authorization:Basic <value>` / `Authorization: Basic <value>` の認識済み形式で値を保存・送信しない。未知のPIIや任意の `Basic` 文言まで除去するDLPではない。`--no-telemetry` はローカル記録を止めるだけで、Jev/Gatewayへの外部送信停止ではない。

Hook metadataの`trigger` / `source` / `selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなidentifierを拒否し、既存schema v1/v2のloadでは該当metadataを正規化・欠損化して分析互換性を保つ。Review receiptも本文を保存せず、task/turn/sessionはhashだけでcostを集計する。

Jev未設定時はローカル推測へフォールバックせず、`missing_api_key`をreceiptへ記録してから同じCLIエラーへ変換する。stateのbyte/candidate window超過はJevを呼ばず、receiptへ`degraded`として記録する。これは「Jevの有用性を測る」目的で、ローカルだけの結果をJev結果と混同しないためだよ。

## レイテンシ設計

- ローカル候補探索の目標: p95 100ms以下
- Jevを含む全体の目標: p95 2秒以下
- デフォルトHTTPタイムアウト: 1,500ms
- 出力する時刻: `discoveryMs`、`jevResponseMs`、`totalMs`
- Telemetry集計: 判定率、Jev/total p50・p95、平均input/output tokens、usage event数
- Decision/review receipt集計: accepted/none/unknown/defer/degraded、fallback率、cache hit、retry、latency p50/p95、Token、Jev/Codex/total cost、成功review/fix単価

Jevの呼び出しは候補ごとに繰り返さず、候補を1つのChoice質問へまとめる。これで候補数に比例したネットワーク往復を避けつつ、速度とtoken usageを測定できる。

## 安全側の判定

次の場合は`selected`を返さず安全側のstatusへ流す。

- Jevが`none`を選ぶ
- choiceが欠落する
- 未知のSkill IDを返す
- 1位の確率が0.60未満
- 1位とrunner-upの確率差が0.10未満
- state不足・window超過は`degraded`
- timeout、provider error、空回答、低確信度は`defer`または`unknown`

明示的な`--skill`指定はJevを呼ばず、`explicit`として返す。利用者の明示指定をモデルの推測で上書きしないためだよ。

## 失敗時の契約

| 状況 | JSON `error.code` | 終了コード |
| --- | --- | ---: |
| 入力不正、APIキーなし、JSON不正 | `invalid_input` / `missing_api_key` / `json_error` | 2 |
| Jev接続失敗、Jev応答エラー | `provider_error` | 3 |
| タイムアウト | `timeout` | 3 |
| ファイル・YAMLエラー | `io_error` / `yaml_error` | 2 |

判定結果の詳細は`decisions.jsonl`へ安全なメタデータとして記録する。receiptにはcontract/question/policy version（policy閾値を含むdigest付き）、state digest、実際のstate bytes、適用したstate/candidate予算、候補/window/omitted/redaction理由、typed answer digest、evidence、fallback、calls/retry、latency、token proxy cost、replay IDを含めるが、prompt本文、Skill本文、Tool結果、APIキー、生session IDは含めない。`replay_receipt`はversion、state digest、recorded answerとcode-side resultを検証し、不一致を`degraded`として返す。

既知Hook event（`SessionStart`、`PreCompact`、`PostCompact`、`UserPromptSubmit`）を受理した場合だけ、shadowは `{"continue":true,"suppressOutput":true}` の固定応答を返す。既知eventでJevが失敗しても現行recordと`continue: true`を優先する。不明event、event不一致、壊れた設定は固定応答を返さず入力契約違反としてエラーにする。

## 今後の境界

次は実ユーザーに近い匿名化ケースを増やし、Hookの連続利用遅延、Gateway rate limit、Jev/Codexの実費、compact後の再現性を測定する。Hookだけでmodel/reasoningを変更できない場合は、App Server/SDK/exec wrapperの境界を別要件として定義する。Skill自動ロード・実行、会話全文のLLM要約、Tool Result削減、MCP化は、権限境界と失敗時の復旧を別要件として定義する。

公式Hook契約は[Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)、Compaction仕様は[OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)を参照する。

## Jevを使う設計原則

複数の実装事例を比較すると、Jevを特別な処理の代わりにするのではなく、**決定的なローカルコードと意味判定の境界を狭く保つ**ことが品質・安全性・運用性を作っていた。jevxでは次の原則を共通の設計契約として扱うよ。

#### 実装事例から抽出したパターン

| 観察した設計パターン | 取り入れる観点 | そのまま移植しないもの |
| --- | --- | --- |
| 意味的なレビュー | matcherで対象を絞り、意味的な一文のpredicate・fixture・accepted baseline・warning-firstで運用する | parser / type checkerで確定できる欠陥のJev化、いきなりCIをblockingする運用 |
| 状態分割と予算管理 | stateをwindow化し、overlap・予算・cache・replay・costを明示する | 決定的なsyntax highlightingをJevへ置き換えること |
| 変更影響のScore評価 | `Score`で影響度を表し、uncertain / missing / dynamicを安全側で選び、runnerのexit codeを保持する | framework固有のrunner差分をjevxの共通契約へ混ぜること |
| typed policyとreplay | typed answer、atomic predicate、policy gate、transcript / replay、`defer`を一級の結果にする | Jevの確信度を権限やauto-allowへ直結させること |

ここでの比較結果は、jevxの全機能を一度に増やす指示ではなく、共通基盤を先に整えてから小さな縦機能を評価するための設計材料だよ。

#### 1. Jevは意味判定、コードは決定的な仕事

- 構文解析、差分抽出、対象列挙、テスト発見、window分割、redaction、コマンド実行はローカルコードで行う。
- Jevには「この候補が依頼の文脈に合うか」「この変更がこの対象へどの程度影響するか」のように、コードだけでは書きにくい意味的・文脈的な問いを渡す。
- 決定的なparserや型検査で確定できる問題をJevへ移さない。Jevを使う必然性を、縦機能ごとに説明できる状態にする。

#### 2. Jevの回答は推薦であり、最終決定ではない

Jevの回答は自由文ではなく、用途に合ったtyped answerとして扱う。現在のSkill選択は`Choice`を使うが、将来の機能では次の型を使い分ける設計候補がある。

| answer type | 向いている問い | 最終判断 |
| --- | --- | --- |
| `Choice` | 候補Skillから最も合うもの、または`none` | コード側の確信度・margin gate |
| ordered `Score` | 変更が各テストへ与える影響の大きさ | コード側のcutoff・unsure判定 |
| atomic predicate / `Noul` | 条件の成立、policyの構成要素 | コード側の保守的なcomposition |

Jevの確信度は「与えられたstateのもとでの判定の確からしさ」であり、操作が成功することや権限を与えてよいことの証明ではない。`selected`になってもSkillの自動ロード・実行や権限付与へ直結させない。

#### 3. Decision Contractで問い・結果・安全側挙動を束ねる

Skill選択の現行実装は、個別のJev呼び出しを増やさず、次のDecision Contractへ寄せている。新しい専用CLIコマンドは増やさず、Rust APIと既存の評価Runnerから利用する。

```text
State Builder → redaction / window plan → typed Jev answer
            → code-side threshold / margin / policy gate
            → accepted / none / unknown / defer / degraded
            → safe receipt + replayable evaluation
```

receiptには、少なくとも次の安全なメタデータを持たせる。prompt本文・Skill本文・Tool結果・APIキー・生のセッション識別子は含めない。

- contract / question / policyのバージョン（policyは閾値を含むdigest付き）
- state digest、実際のstate bytes、適用したstate/candidate予算、候補数、window数、omitted / redactionの理由
- 構造化されたanswer、code-side decision、threshold、margin、fallback、reason
- calls、retry、latency、input / output tokens、相対的なcost、replay ID

この形式にすると、同じfixtureとrecorded answerでJevなしにcode-side decisionを再生でき、質問や閾値を変更したときの差分もレビューできる。

#### 4. 不確実性・状態不足・障害は一級の結果にする

`unknown`、`defer`、`degraded`、`no answer`、`timeout`、`outage`を、成功や自動allowへ変換しない。特に権限Hookや破壊的操作は、必要なら`ask`または`defer`へ流し、fail-closedの挙動を優先する。

状態をtoken / byte / 候補数の上限で分割するときは、window、overlap、omitted項目、redaction理由、予算超過を可視化する。大きな入力を黙って切り捨てたまま、高い確信度だけを信頼しない。

#### 5. 評価・再現性・コストを機能の一部にする

新しいJev利用は、dry-run、fixture、accepted baseline、replay、必要に応じたcacheを用意してから運用へ進める。少なくとも次を同じ入力・候補・ネットワーク条件で比較する。

- Top-1 accuracy、`none` precision、candidate miss、uncertain / fallback率
- 誤って危険側へ進まない率、誤検出率、反復時のanswer / confidence variance
- Jev p50 / p95、全体p50 / p95、token、相対cost、retry、error rate

Jevありの単発精度だけで導入を決めず、Jevなしのbaseline、遅延、費用、失敗時の挙動を一緒に見る。既存評価の実行方法は [CLIリファレンスの評価Runner](jevx-cli-reference.md#skill選択の評価runner) を参照してね。

判断の優先順位と「やらないこと」は [jevx/PHILOSOPHY.md](../../jevx/PHILOSOPHY.md) を正本とする。今後の候補は [jevx-roadmap.md](jevx-roadmap.md) を参照。
