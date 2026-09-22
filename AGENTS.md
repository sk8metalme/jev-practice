# Agent Instructions

## コードレビュー

- 必ず日本語でコードレビューを行ってください。
- 指摘事項は簡潔に、実行可能な形式で提示してください。

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
