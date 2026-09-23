# jevx 要件定義と実装状況

> 対象読者: 開発者・保守担当
> 文書の状態: 現行仕様

## 目的

`jevx`は、Codex CLIの日常利用を快適にしながら、Jevの実用性を測定するためのmacOS向けCLIである。主目的は、Skill選択・Hook運用・Compaction後のコンテキスト補助について、Jevを使わない場合との差分を追跡可能な計測で確認すること。

Skill選択のコアはShadow Modeとし、`selected`になってもSkill本文の自動ロード・実行や会話の書き換えは行わない。Codex HookとCompaction補助は、利用者が設定をreview/trustした場合だけ動くopt-inの試作機能として境界を分ける。

## 対象範囲

- Rust製CLIを`jevx/`に提供する
- プロジェクトSkillとユーザー共通Skillを探索する
- ローカル候補絞り込みとJevのChoice判定を行う
- `Choice` / `Score` / `noul`を共通のDecision Contractとして扱い、最終statusをコード側で決める
- redaction・候補window・byte budgetを`StatePlan`へ固定し、欠落時は`degraded`として扱う
- `none`による未推薦と確率・marginの安全側閾値を持つ
- timeout・provider error・低確信度を`unknown` / `defer`へ流し、自動allowしない
- Decision receiptを安全なJSONLへ記録し、recorded answerをJevなしでreplayする
- retry・cache・dry-run・token proxy costとfallback率を評価できる
- 人間向け出力と`--json`出力に判定・Jev応答速度・全体速度を出す
- prompt本文を保存しないJSONL Telemetryとstats集計を提供する
- Codex向けadvisory SkillとmacOS用セットアップスクリプトを提供する
- 既存Hook設定を保持してjevx Hookを冪等に登録する `hooks install` を提供する
- `PreCompact` / `PostCompact` / `SessionStart(source=compact)` のcheckpointを記録する（IDはハッシュ化、raw metadataは現行制約の範囲で扱う）
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
jevx eval --dry-run --json
jevx eval-repeat --runs 5 --dry-run --json
jevx hooks install --scope user --dry-run --json
jevx hooks uninstall --scope user --dry-run --json
jevx data path --json
jevx data export --output /tmp/jevx-data.json
jevx data purge --yes
jevx hooks shadow
jevx hooks compact-assist --state-dir /tmp/jevx-compaction
jevx hooks compact-eval --runs 5 --json
jevx hooks conversation-eval --input /tmp/conversation.jsonl --json
jevx hooks correlate --input /tmp/hooks.jsonl --json
```

Jev判定は`AI_GATEWAY_API_KEY`を使い、Vercel AI Gatewayの`typesafe-ai/jev`へ候補 + `none` の `choice` として送信する。旧Node.js Webアプリの歴史的なyes/no質問は `boolean`、TypeSafe直接APIのyes/no質問は `noul` であり、現行jevxの `choice` と混同しない。`hooks install`自体は設定生成だけで外部送信せず、インストール後に実行される`UserPromptSubmit`の`hooks shadow`だけがJev判定を行う。

セットアップスクリプトの主な引数は `setup.sh --scope user|project [--repo PATH] [--hooks]` で、ヘルプは `--help` / `-h` で表示できる。Rust/Cargoを用意し、install rootは `jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"` で決めて `JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user` のように実行する。単体 `jevx` 呼び出しには同rootの `bin` を `PATH`へ追加する。

デフォルトの設定は次のとおり。

- 候補上限: 32件（`JEVX_MAX_CANDIDATES`、1〜256）
- 選択確率の下限: 0.60（`JEVX_MIN_PROBABILITY`、0〜1）
- 1位と2位の確率差: 0.10以上（`JEVX_MIN_MARGIN`、0〜1）
- 範囲外の上書き値は既定値へ戻し、`doctor` の `warnings` に出す
- リクエストタイムアウト: 1,500ms（リトライを含む呼び出し全体の予算）
- Telemetry: `~/.jevx/events.jsonl`
- Decision receipt: `~/.jevx/decisions.jsonl`
- state byte上限: 32,000 bytes（`JEVX_MAX_STATE_BYTES`、1,024〜262,144）
- retry: 429/529を最大1回、25ms backoff（`JEVX_MAX_RETRIES` 0〜3、`JEVX_RETRY_BACKOFF_MS` 0〜1,000）。締め切りを超えるbackoffは再試行しない
- cache: 既定無効（`JEVX_DECISION_CACHE_CAPACITY=0`）
- token proxy cost: input/outputとも既定weight 1.0
- Hook state: Telemetry親ディレクトリ配下の`hooks.jsonl` / `compaction/`

## 実装対応表

| 領域 | 現状 | 境界 |
| --- | --- | --- |
| Skill探索・ローカル順位付け・Jev判定 | 実装済み | Shadow Mode。Skill本文の自動ロード・実行はしない |
| Decision Contract・StatePlan・Recorder | 実装済み | typed answer、code-side gate、safe status、receipt、replayをRust APIで提供。権限Hookへ自動allowしない |
| Telemetry・stats・doctor | 実装済み | prompt本文、APIキー、Jevのprobabilityは保存しない。doctorは導入状態と `nextSteps` を返す |
| ローカルデータの確認・書き出し・削除（`data`） | 実装済み | `$JEVX_HOME` のjevx管理ファイルだけを扱い、`purge` は `--yes` まで削除しない |
| Hook shadow・install・uninstall・compact-assist | 実装済み | 明示的な導入とCodex側のTrustが必要。uninstallはjevx管理のhandlerだけを外す |
| 公開JSONの契約テスト | 実装済み | `tests/contract_requirements.rs` が `--json` のキーと終了コードを固定する |
| Skill選択・Compaction・会話評価Runner | 実装済み | controlled fixture metadataまたは検証済みJSONLを評価し、Codex/App Serverは起動しない |
| Skill自動実行・会話全文要約・Tool Result削除 | 対象外 | 別要件と安全性評価が必要 |

## Hook設定要件

`hooks install`は次のルールを満たす。known eventを `hooks shadow` が受理した場合だけ、`{"continue":true,"suppressOutput":true}` の固定応答を返す。未知event、event不一致、壊れたJSONは固定応答を返さずエラーにする。

| 要件 | 内容 |
| --- | --- |
| 保存先 | user: `$CODEX_HOME/hooks.json`、未設定時`~/.codex/hooks.json`; project: `<repo>/.codex/hooks.json` |
| 既存設定 | root、カスタムHook、未知のイベントを保持する |
| 重複 | 既存のjevx `shadow` / `compact-assist` handlerだけを置換し、再実行を冪等にする |
| 復旧 | 変更時の既存ファイルを`hooks.json.jevx.bak`へ初回だけ退避する |
| 安全確認 | `--dry-run`と`--json`で変更前に確認でき、実行後はCodex `/hooks`でTrustを確認する |
| matcher | `SessionStart`: `startup\|resume\|clear\|compact`; `PreCompact`/`PostCompact`: `manual\|auto` |

project-local HookはCodex側のプロジェクトTrustが必要であり、ファイル生成成功だけでは発火成功を意味しない。

## Compaction補助要件

`compact-assist`は要約器ではなく、次を行う決定的な補助とする。

1. Hook JSONを検証し、既存の安全なshadow record形式へ変換する。
2. session / turn / correlation / cwdはSHA-256、イベント識別子は安全なラベルだけ保存する。Hook recordの`trigger` / `source`はtrim後にASCII許可文字と最大長を検証し、`selectedSkill`も同じ境界（namespaceの`:`を含む）で検証する。新規write/appendでは空値・空白・制御文字・長さ超過を拒否し、既存schema v1のloadでは該当する任意metadataだけを正規化または欠損化して分析互換性を保つ。
3. `.jevx/compact-context.md`があればredact後に最大4,000文字まで利用する。
4. checkpointへmanifest本文や秘密値を保存しない。
5. `SessionStart(source=compact)`では、最新checkpoint metadataとredacted manifestだけを`additionalContext`へ返す。
6. contextがない場合もCodex処理を止めず、補助情報がないことを明示する。

公式Compactionの代替、会話全文の復元、現在のリポジトリ状態の保証はしない。

## データ保護

Jevへの送信境界は次のとおり。

| データ | Gatewayへ送るか | 実装上の扱い |
| --- | --- | --- |
| 現在のprompt | 送る（redact後） | 認識済みのsecret assignment、Bearer / Basic、`sk-` / `tsk-`をredact |
| `cwd` | 送る（redact後） | stateへ含める前にredactし、receiptにはstate digestだけ保存 |
| Skill ID / name | 送る（raw） | 候補識別子として含む |
| Skill description | 送る（redact後） | Skill本文は送らない |
| APIキー | 送らない | Bearer認証ヘッダーにのみ使用 |
| Skill本文、過去会話、Tool結果 | 送らない | v1の対象外 |

Basic redactionは `Authorization=Basic <value>` / `Authorization:Basic <value>` / `Authorization: Basic <value>` の認識済み形式で値を保存・送信しない。通常文中の単独`Basic`は意味を保つためredactしない。完全なDLPではない。Telemetryには生の依頼文を保存せず、SHA-256、文字数、候補数、判定、選択時の`selectedSkill`（raw ID）、遅延、usageを保存し、Jevのprobabilityは保存しない。Skill ID自体を秘密値として扱う設計ではない。`--no-telemetry` はローカル記録を止めるだけで、Gatewayへの外部送信停止ではない。Hook recordとCompaction checkpointには生のsession ID、turn ID、model、manifest本文を保存しない。

Hook metadataの`trigger` / `source` / `selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなidentifierを拒否し、既存schema v1のloadでは該当metadataを正規化・欠損化して分析互換性を保つ。

## 成功条件

- ローカルSkill探索のp95が100ms以内
- Jevを含む全体処理のp95が2秒以内
- `jevResponseMs`が人間向け・JSON出力・Telemetryに存在する
- Jev未設定・タイムアウト時に明示エラーを返し、Hook shadowはCodex処理を継続する
- 既存のローカルキーワード方式との精度・遅延・Token usage比較が可能
- `nonePrecision` が `expectedNone` 分母の recall として解釈でき、local_keyword / local_rankを含む4モードを比較できる
- fallback率、cache hit率、retry率、平均retry、relative cost、receiptのstatus別件数を比較できる
- `decisions.jsonl`をversion・state digest検証付きでreplayでき、不一致は`degraded`になる
- `hooks install --dry-run`が変更せず、実行後の再実行が冪等である
- Compaction checkpointとredacted manifestが秘密情報を再出力しない
- 新規Rust実行コードのラインカバレッジ98%以上

## Historical snapshot（2026-09-21）

以下は記録時点の環境・モデル・Gatewayに対する歴史的な観測値であり、現行環境の保証ではない。同梱40ケース（合成30、匿名化テンプレート10）の詳細は[`jevx-comfort-evaluation.md`](../research/evaluations/jevx-comfort-evaluation.md)に分けている。

| 方式 | 正解率 | `nonePrecision`（`expectedNone` 分母の recall） | 速度・コスト |
| --- | ---: | ---: | --- |
| 選択なし | 10.0% | 100.0% | 外部通信なし |
| ローカルキーワード | 90.0% | 75.0% | 外部通信なし |
| jevx + Jev（1回） | 100.0% | 100.0% | Jev p50/p95 407/673ms、平均input/output 2401.2/127.7 |
| jevx + Jev（5回平均） | 98.0% | 100.0% | Jev p50/p95 427/610ms、error rate 2.0%、探索p95 2ms |

これは固定fixtureの観測であり、利用者入力への精度・性能保証ではない。Jevの外部通信・Token使用量・Gatewayエラーを導入判断のコストとして同時に扱う。

## Current / Latest baseline（2026-09-23）

現行コードの外部送信なし確認は、`JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- eval --fixtures jevx/evals/skill-selection.jsonl --skill-dir jevx/evals/skills --dry-run --json` を正本とする。評価カタログは `--skill-dir` だけで、project・HOME・CODEX_HOMEのSkillは混ざらない。2026-09-23の確認では、`local_keyword` の正解率は100.0%、`local_rank` は20.0%、`none`は10.0%、Jevモードは`not_run`だった。

| 項目 | 値 |
| --- | --- |
| 実行日時 | `2026-09-23T22:11:53+09:00` |
| 測定時の実装・fixture commit | `391e18049fe89a0dadcb8d43f6b70e3c6673ddf1` |
| fixture SHA-256 | `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb` |

2026-09-22に記録した値（`local_keyword` 87.5%、`local_rank` 17.5%）は、既定のSkill rootが評価カタログに混ざった環境依存の値だったため、Historicalとして扱う。

## 評価方法

測定項目は、Top-1精度、`none`判定率、候補漏れ率、Jev p50/p95、全体p50/p95、入力Token、出力Token、エラー率、Hook継続率、checkpointの秘密値漏えい件数とする。APIキーありの実測はシークレット値・生prompt・生会話をリポジトリへ保存しない。

カバレッジ確認には追加ツール `cargo-llvm-cov`（`cargo install cargo-llvm-cov`）が必要です。JSON検証を行う箇所では `jq`（macOSなら`brew install jq`）も用意してください。

```bash
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --all-targets --fail-under-lines 98
```

公式のHook契約は[Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)、Compactionの概念とAPI仕様は[OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)を参照する。
