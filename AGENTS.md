# Agent Instructions

## コードレビュー

- 必ず日本語でコードレビューを行ってください。
- 指摘事項は簡潔に、実行可能な形式で提示してください。

## OSS設計原則（jevx全体）

判断の正本は [`jevx/PHILOSOPHY.md`](jevx/PHILOSOPHY.md)。この節は、100のOSSの設計思想（https://sk8metalme.github.io/thinking/oss/oss-design-philosophy.html）から、エージェントが変更前に確認する判断基準を抜き出したもの。OSSとしての見直しや新規作成には `oss-design` Skill（`apm.yml` で導入）を使う。

機能・設定・文書を変更する前に次を確認し、PR本文に根拠を残す。

- **問題が先。** 誰が・どの作業で・何に困るかを説明できるか。「問題を探している解決策」になっていないか。受け入れ基準と捨てるものを [ロードマップ](docs/developers/jevx-roadmap.md) の基準で書けるか。
- **Non-goalsに抵触しないか。** Skillの自動実行、auto-allow、会話の要約・書き換え、会話全文やSkill本文の外部送信、設定オプションの追加は実装しない。代わりの手段を提示する。同じ種類の要望を3回見送ったら、理由をPHILOSOPHY.mdの「やらないこと」へ追記する。
- **オプションより既定値。** 設定を増やす前に、既定値の改善で解けないか検討する。既存の上書き手段（`JEVX_MIN_PROBABILITY` など）はふさがない。
- **公開境界を壊さない。** CLIの引数、`--json` のキー、終了コード、ローカルデータのschema、`--jevx-managed` マーカーを変えるときは `schemaVersion` を上げて移行を書き、`jevx/tests/contract_requirements.rs` を更新する。名前と意味がずれたキーは消さずに正しい名前のキーを足す。
- **明示的に。** 黙った切り詰め・フォールバック・自動書き換えをしない。設定やデータを変えるコマンドには `--dry-run` か確認フラグ（`--yes`）を付ける。
- **ユーザー主権。** ローカルデータは `$JEVX_HOME` の下だけに書き、`jevx data` で一覧・書き出し・削除できる状態を保つ。入れたもの（Hook、Skill）は戻せるようにする。
- **最初の5分。** `jevx/README.md` を入口として短く保ち（目安150行）、インストールから最初の提案までの手順と `doctor` の `nextSteps` を壊さない。詳細は `docs/` の正本へ書き、READMEからリンクする。
- **局所性。** CLIは `jevx/src/cli/` の1コマンド1ファイルを保ち、1ファイルで上から読めるようにする。文書の内容は正本を1か所にし、ほかはリンクにする。
- **スコープ。** `legacy/` 配下は現行の対象外。機能追加や修正をしない。
- **タイブレーク。** 安全側と互換性を優先する。ただし、利用者の体験が大きく良くなり、安全側の結果と公開境界を壊さない場合は使いやすさを優先する。

## Jev / jevx 設計原則

複数のJev利用事例から得た次の原則を、jevxの実装・設計・レビューで守ってください。

- **Jevは意味判定に限定する。** 構文解析、列挙、差分抽出、テスト発見、権限付与、コマンド実行など決定的に解ける処理はローカルコードで行い、Jevにはコードだけでは書きにくい文脈的・意味的な問いを渡す。
- **対象・状態・最終決定はコードが所有する。** 送信前の候補抽出、redaction、stateの組み立て、閾値・margin・severity、最終的なallow / deny / executeはJevの自由文や確信度に委ねない。
- **Jevの回答は推薦であり権限ではない。** `Choice`、順序付き`Score`、atomic predicateを構造化されたtyped answerとして受け取り、コード側のgateで`selected` / `none` / `unknown` / `defer` / `degraded`を決める。
- **失敗時は安全側へ倒す。** timeout、APIエラー、空回答、低確信度、state不足、window超過、outageを成功や自動allowに変換しない。破壊的操作や権限Hookは、必要なら`ask`または`defer`へ流す。
- **判定は証拠付きで再現可能にする。** contract / question / policyの版、state digest、回答、閾値、margin、fallback、理由、latency、token / cost、replay IDをreceiptまたは安全なTelemetryへ記録する。prompt本文・Skill本文・Tool結果・秘密情報は保存しない。
- **状態の切り詰めを黙って行わない。** token / byte / 候補数の上限、window、overlap、omitted項目、redaction理由を明示し、情報が欠けた場合は`degraded`として扱う。
- **評価は精度だけで決めない。** Top-1、`none`精度、候補漏れ、uncertain / fallback率、誤って危険側へ進まない率、p50 / p95遅延、token / cost、エラー率、反復時の分散を同じ条件で測る。

## Jev / jevx 変更時のレビュー項目

Jevを使う新機能や既存契約の変更では、実装前に次を確認し、PR本文または設計メモへ根拠を残してください。

- その問いはJevでなければ解きにくいか。決定的なローカル処理をJev化していないか。
- 送信する対象とstateをコードで再現できるか。秘密、会話全文、Skill本文、Tool結果を境界外へ出していないか。
- `Choice` / `Score` / predicateの型、question version、閾値、margin、fallbackが明示されているか。
- unknown、timeout、outage、state不足、window超過がどの安全側結果へ流れるか。自動allowになっていないか。
- recorded answer / fixture / replayで、Jevなしに同じcode-side decisionを再現できるか。
- dry-run、cache、retry、Telemetryでコスト・遅延・誤判定・分散を追跡できるか。
- CIやHookを最初からblockingにせず、まずShadow Mode・warning・accepted baselineで有用性を確認しているか。

Jevを使う処理を追加する場合は、REDで失敗ケースとfallbackを定義し、GREENで最小のtyped contractを通し、REFACTORで共通のDecision Contract・receipt・評価器へ寄せてください。権限Hookは、脅威モデルとfail-closed / deferの検証が終わるまで自動許可を実装しないでください。
