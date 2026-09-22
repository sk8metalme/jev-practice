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
- `none`による未推薦と確率・marginの安全側閾値を持つ
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
jevx hooks shadow
jevx hooks compact-assist --state-dir /tmp/jevx-compaction
jevx hooks compact-eval --runs 5 --json
jevx hooks conversation-eval --input /tmp/conversation.jsonl --json
jevx hooks correlate --input /tmp/hooks.jsonl --json
```

Jev判定は`AI_GATEWAY_API_KEY`を使い、Vercel AI Gatewayの`typesafe-ai/jev`へ候補 + `none` の `choice` として送信する。旧Node.js Webアプリの歴史的なyes/no質問は `boolean`、TypeSafe直接APIのyes/no質問は `noul` であり、現行jevxの `choice` と混同しない。`hooks install`自体は設定生成だけで外部送信せず、インストール後に実行される`UserPromptSubmit`の`hooks shadow`だけがJev判定を行う。

セットアップスクリプトの主な引数は `setup.sh --scope user|project [--repo PATH] [--hooks]` で、ヘルプは `--help` / `-h` で表示できる。Rust/Cargoを用意し、install rootは `jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"` で決めて `JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user` のように実行する。単体 `jevx` 呼び出しには同rootの `bin` を `PATH`へ追加する。

デフォルトの設定は次のとおり。

- 候補上限: 32件
- 選択確率の下限: 0.60
- 1位と2位の確率差: 0.10以上
- リクエストタイムアウト: 1,500ms
- Telemetry: `~/.jevx/events.jsonl`
- Hook state: Telemetry親ディレクトリ配下の`hooks.jsonl` / `compaction/`

## 実装対応表

| 領域 | 現状 | 境界 |
| --- | --- | --- |
| Skill探索・ローカル順位付け・Jev判定 | 実装済み | Shadow Mode。Skill本文の自動ロード・実行はしない |
| Telemetry・stats・doctor | 実装済み | prompt本文、APIキー、Jevのprobabilityは保存しない |
| Hook shadow・install・compact-assist | 実装済み | 明示的な導入とCodex側のTrustが必要 |
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
| `cwd` | 送る（raw） | 依頼の作業ディレクトリをそのまま含む |
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

## Current / Latest baseline（2026-09-22）

現行コードの外部送信なし確認は、`JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- eval --fixtures jevx/evals/skill-selection.jsonl --skill-dir jevx/evals/skills --dry-run --json` を正本とする。2026-09-22の確認では、`local_keyword` の正解率は87.5%、`local_rank` は17.5%、`none`は10.0%、Jevモードは`not_run`だった。

| 項目 | 値 |
| --- | --- |
| 実行日時 | `2026-09-22T15:00:42+09:00` |
| 測定時の実装・fixture commit（docs同期前） | `ed0b93c023251400bcbbe2de8cdedd5451f08761` |
| fixture SHA-256 | `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb` |
| Rust / Cargo | `rustc 1.98.1 (48a229cea 2026-09-01)` / `cargo 1.98.1 (797e8a9bc 2026-08-05)` |
| OS | macOS 26.6.2（build 25G83） |
| 依存条件 | APIキーなし、`JEVX_TELEMETRY=off`、外部Gateway呼び出しなし。fixture 9件に既定rootsを加えた有効候補14件 |

evalの既定カタログは project `.agents/skills` / `.codex/skills`、`HOME/.agents/skills`、`CODEX_HOME/skills`、指定 `--skill-dir` の合成であり、`--skill-dir`だけでは隔離にならない。Latest値を再現する場合は記録時と同じrootsを保ち、fixture-only検証では一時HOME・CODEX_HOME・空のproject rootを用意して評価fixtureの `--skill-dir` を絶対パスで指定する（隔離時の値はLatestと異なり得る）。fixtureや実装が変わる場合は、このコマンドと条件を再実行して値を更新する。

## 評価方法

測定項目は、Top-1精度、`none`判定率、候補漏れ率、Jev p50/p95、全体p50/p95、入力Token、出力Token、エラー率、Hook継続率、checkpointの秘密値漏えい件数とする。APIキーありの実測はシークレット値・生prompt・生会話をリポジトリへ保存しない。

カバレッジ確認には追加ツール `cargo-llvm-cov`（`cargo install cargo-llvm-cov`）が必要です。JSON検証を行う箇所では `jq`（macOSなら`brew install jq`）も用意してください。

```bash
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --all-targets --fail-under-lines 98
```

公式のHook契約は[Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)、Compactionの概念とAPI仕様は[OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)を参照する。
