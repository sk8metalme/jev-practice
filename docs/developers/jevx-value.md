# jevxの価値と採用判断

> 状態: 現行仕様の価値定義。数値の採否は実測レポートではなく、この文書の受け入れ基準で決める。

## 何を解くか

Codex CLIの作業が大きくなると、Skill選択、Compaction、計画・推論開始、文章・コードの意味確認、モデルやsubagentの選択に時間がかかる。Jevを追加すれば判断品質が上がる可能性がある一方、Jevの遅延・Token・費用、Codex側の追加ターン・昇格費用を同時に測れなければ、導入価値は分からない。

jevxの価値は「Jevを使うこと」ではなく、**速度・品質・安全性・Jev費用・Codex費用を同じ条件で比較でき、失敗時も誤って成功扱いしないこと**にある。

## 現行の4本柱

| 柱 | 利用者の困りごと | 現行の境界 | 成功の証拠 |
| --- | --- | --- | --- |
| 速度観測 | Skill選択、Compaction、計画開始の待ち時間が分からない | Hook/CLIで遅延を観測。公式Compactionを置換しない | 同一fixtureのp50/p95とbaseline差分 |
| 意味レビュー | 日本語の曖昧さ、文章/コードの矛盾、コメント乖離を見落とす | 決定的な候補抽出はローカル、Jevはtyped predicate | 4カテゴリのfixture、Jevなしreplay、uncertain率 |
| route候補 | 依頼に対してmodel/reasoningを選ぶ根拠がない | Jevは推薦。Hookで適用証拠がなければ`degraded` | fallback順、適用証拠、品質/速度/費用の比較 |
| 費用観測 | 速くなっても追加費用が分からない | 上限は設けず、Jev/Codex/合算を別々に記録。model昇格の追加Token／追加費用も分離 | 推定/実費、通貨/価格版、task/turn/session集計、baseline差分、fallback追加費用 |

## 使い方のモード

- **review-only**: 既定。本文はJevへ送らず、redact済みのローカル検出とmetadataだけを使う。
- **Jev review**: `hooks review --allow-content` または `hooks install --allow-content` を利用者が明示した場合だけ、選択対象のredact済み本文を1回のbatched typed requestへ渡す。
- **fix**: `--auto-fix --yes` の両方が必要。高信頼のCompleted review、相対パス、期待SHA-256一致をコード側で確認してからworkspace内へ書く。backup/rollbackは作らない。
- **route observation**: low/medium/highからLuna/Terra/Solとreasoningの候補、fallback chainを出す。Hook単体でmodel切替を保証せず、適用された証拠が完全一致した場合だけ`applied`。

## 速度・品質・費用を同時に読む

1つの指標だけで採用を決めない。最低限、同じ入力・環境・再試行・fixture hashで次を並べる。

| 観測 | 例 | 不明時の扱い |
| --- | --- | --- |
| 速度 | p50/p95、baseline差分、fallback追加時間 | 欠落は成功扱いにしない |
| 品質 | Top-1、none、4レビューカテゴリ、危険側誤判定 | uncertain/degradedを別集計 |
| 費用 | `jevCost`、`codexCost`、`totalCost`、成功review/fix単価 | `unknown`/`unavailable`。0ではない |
| 安全 | 外部送信されたsecret/raw Tool result、fix hash mismatch | 1件でも採用ゲートを止める |

速度20%以上短縮は必要条件の候補であり、単独の成功条件ではない。品質が改善しない、費用が計測不能、route適用を証明できない場合は採用しない。費用上限は現段階で置かず、まず速度短縮と追加費用の関係を可視化する。

## 受け入れ基準

- Skill選択、Compaction、計画/推論の3領域で、baseline比20%以上短縮を同条件で再現できる。
- 通常Hookの判定/指示生成p95が800ms以下で、Jev遅延とCodex追加時間を分離できる。
- 4カテゴリをfixtureで検出し、typed answerとreceiptからJevなしreplayができる。
- fallback順を検証でき、適用証拠なしのrouteを`applied`にしない。
- secret、API key、raw Tool result、保存本文が0件である。
- Jev/Codex/合算をtask・turn・session単位で確認し、推定/実費と価格版/通貨を追跡できる。
- `local_rank` baselineとの`totalMsDelta`・`speedupRate`・`additionalCost`を同じレポートで確認できる。
- `unknown`/`unavailable`を0円へ変換しない。
- 新規実行コードのline coverage 98%以上、fmt・clippy・unit・integration・contract・fixture評価が通る。

## 見送る条件と次の設計

HookがCodexのmodel/reasoning切替やcompact前処理を十分に制御できない場合、Hookを強化して黙って保証するのではなく、適用境界を持つApp Server/SDK/exec wrapperを別Issueで検討する。その場合も、利用者が明示的に導入し、失敗を`degraded`/`defer`として残し、費用と遅延を同時に測る。

自動Skill実行、auto-allow、会話全文の要約/書き換え、Tool Resultの削除、費用上限の導入はこの価値定義のNon-goalsである。詳細は [PHILOSOPHY.md](../../jevx/PHILOSOPHY.md) を参照する。
