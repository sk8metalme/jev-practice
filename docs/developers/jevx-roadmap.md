# jevx ロードマップ

次の順番は、共通基盤を先にしてから縦機能を増やす提案だよ。未実装の項目は設計候補であり、機能の存在を表さない。

| 優先度 | 候補 | 目的 | 受け入れの観点 |
| --- | --- | --- | --- |
| 完了 | Decision Contract & Recorder | `Choice` / `Score` / predicate、code-side gate、fallback、receipt、replayを共通化 | recorded answerで同じdecisionを再現し、timeout / outageが自動allowにならない |
| 完了 | 費用観測基盤 | Jev/Codex/合算、推定/実費、通貨/価格版、status、task/turn/session集計 | 取得不能を0にせず、速度・品質と同じreceipt/レポートで比較できる |
| 完了 | 意味レビュー縦スライス | 日本語clarity、文章/コード矛盾、コメント乖離をlocal + batched typed Jevで検出 | 4カテゴリfixture、replay、本文非保存、degraded/failureを検証できる |
| 完了 | route候補とsafe fix | 難易度からmodel/reasoning/fallbackを推薦し、明示opt-in時のみfix | 適用証拠なしはdegraded、fixは`--yes`/confidence/hash/safe path gate |
| P1 | Skill Calibration Packs | Skillごとの代表例、none、境界例、期待answer、閾値、反復分散を評価 | 新Skillをpackなしで本番相当評価へ進めず、miss / none / p95 / costを比較できる |
| P1 | App Server / SDK route adapter | Hookで保証できないmodel/reasoning適用を明示境界へ移す | 実適用証拠、fallback追加費用、速度、品質、復旧を同時にfixture検証する |
| P1 | Cost-backed baseline runner | Skill、review、compact、planのJev/Codex usageを同一task/turn/sessionへ相関 | speed20%・品質改善・費用変化を同じレポートで採否判定する |
| P2 | Policy Hook Gate | atomic predicateと`defer`を権限Hookへ適用する研究 | 脅威モデル・監査ログ・fail-closed検証が済むまで自動allowしない |
| P2 | Jev Lab / DSL | transcript、replay、cost、answer、decisionを比較する開発者体験 | 共通契約の重複が実証されてからDSL導入を判断する |

新機能の設計レビューでは、「Jevでなければ解きにくいか」「最終allow / executeをJevが直接決めていないか」「unknown / timeout / outageはどこへ流れるか」「同じdecisionを再現できるか」「秘密情報が境界外へ出ていないか」を必ず確認する。

## 候補を追加・採用するときの基準

[jevx/PHILOSOPHY.md](../../jevx/PHILOSOPHY.md) の原則とNon-goalsに照らし、次をIssueまたは設計メモに書いてから着手する。

- **解く問題:** 誰が、どの作業で、何に困っているか（実在する事例）
- **受け入れ基準:** 何を測れば採用・見送りを判断できるか
- **捨てるもの:** 追加で増える外部送信・遅延・保守対象と、やらないこと

同じ種類の要望を3回見送ったら、理由と代替手段を PHILOSOPHY.md の「やらないこと」へ追記する。
