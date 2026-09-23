# jevx ロードマップ

次の順番は、共通基盤を先にしてから縦機能を増やす提案だよ。未実装の項目は設計候補であり、機能の存在を表さない。

| 優先度 | 候補 | 目的 | 受け入れの観点 |
| --- | --- | --- | --- |
| 完了 | Decision Contract & Recorder | `Choice` / `Score` / predicate、code-side gate、fallback、receipt、replayを共通化 | recorded answerで同じdecisionを再現し、timeout / outageが自動allowにならない |
| P1 | Skill Calibration Packs | Skillごとの代表例、none、境界例、期待answer、閾値、反復分散を評価 | 新Skillをpackなしで本番相当評価へ進めず、miss / none / p95を比較できる |
| P1 | State Window Planner | token / byte / 候補予算、window、overlap、omitted、degradedを管理 | 同じ入力・予算・版で同じwindowを生成し、候補漏れと遅延を測れる |
| P2 | Diff Impact / Semantic Review | 変更影響のScoreや意味的矛盾をShadow Modeで提示 | uncertain / missingを安全側で選択し、CI blockingは評価後に判断する |
| P2 | Policy Hook Gate | atomic predicateと`defer`を権限Hookへ適用する研究 | 脅威モデル・監査ログ・fail-closed検証が済むまで自動allowしない |
| P2 | Jev Lab / DSL | transcript、replay、cost、answer、decisionを比較する開発者体験 | 共通契約の重複が実証されてからDSL導入を判断する |

新機能の設計レビューでは、「Jevでなければ解きにくいか」「最終allow / executeをJevが直接決めていないか」「unknown / timeout / outageはどこへ流れるか」「同じdecisionを再現できるか」「秘密情報が境界外へ出ていないか」を必ず確認する。

## 候補を追加・採用するときの基準

[jevx/PHILOSOPHY.md](../../jevx/PHILOSOPHY.md) の原則とNon-goalsに照らし、次をIssueまたは設計メモに書いてから着手する。

- **解く問題:** 誰が、どの作業で、何に困っているか（実在する事例）
- **受け入れ基準:** 何を測れば採用・見送りを判断できるか
- **捨てるもの:** 追加で増える外部送信・遅延・保守対象と、やらないこと

同じ種類の要望を3回見送ったら、理由と代替手段を PHILOSOPHY.md の「やらないこと」へ追記する。
