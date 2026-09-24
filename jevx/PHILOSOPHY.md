# Philosophy

jevxの機能・設定・文書・レビューを決める正本。好みではなく、問題・境界・測定結果で判断する。

## 解く問題

Codex CLIの作業が大きくなると、Skill選択、Compaction、計画・推論開始、文章・コードの意味確認、モデル／subagentの選択に時間がかかる。Jevを足せば品質が上がる可能性はあるが、Jevの遅延・Token・費用と、Codex側の追加ターン・昇格費用が見えないままでは導入価値を判断できない。

jevxは、**Codexの作業を止めずに、決定的なローカル処理とJevの意味判定を組み合わせ、速度・品質・安全性・Jev費用・Codex費用を同じ条件で比較できる状態を作る**。

## 価値の優先順位

1. **安全な正しさ** — Jevの回答は推薦。最終status、route適用、ファイル書き換え、権限はコードが決める。
2. **総価値の可視化** — 速度短縮だけで成功にせず、レビュー品質、誤って危険側へ進まない率、Jev／Codex／合算費用、失敗時の追加費用を同時に示す。
3. **明示的な境界** — 本文を外部へ送る、Hookを登録する、workspaceを修正する操作は利用者のopt-inを必要とする。
4. **決定的な局所性** — 抽出、列挙、diff、redaction、hash、budget、閾値、重複排除、最終適用はローカルコードで行う。
5. **再現可能な観測** — contract／question／policy、state digest、回答、fallback、latency、Token、price version、cost status、replay IDを本文なしで記録する。

## 対象機能

- 毎ターンのprompt／plan／diff／final answerレビュー
- 日本語の曖昧表現、文章の矛盾、コードの意味的矛盾、コメントと実装の乖離の検出
- Skill選択・Compaction・計画／推論開始のp50／p95とbaseline差分の観測
- Jevの難易度判定によるLuna／Terra／Solとreasoning候補の推薦
- Hookで実際に適用されたrouteだけをappliedとする観測
- 明示opt-inしたworkspace内fixのhash一致チェック付き適用
- Jev単体、Codex単体、合算の費用・Token・retry・fallback・cache・latency集計

## Jevとコードの境界

- 構文解析、対象列挙、candidate window、redaction、state digest、cache、ファイルhash、除外判定、最終allow／deny／writeはコードが所有する。
- Jevへは「候補が依頼に合うか」「このstateに意味的な矛盾があるか」「難易度はどの水準か」のような文脈的な問いだけをtyped contractで渡す。
- 自由文の回答、確信度、route推薦を権限・Skill自動実行・自動書き換えへ直接つなげない。
- timeout、provider error、低確信度、契約不成立、state不足、費用不明は成功や0円に変換しない。

## データ境界

既定では本文をJevへ送らない。hooks install --allow-content または hooks review --allow-content の明示opt-in時だけ、選択されたprompt／plan／diff／final answerをredactして送る。

- API key、資格情報、秘密値、raw Tool resultは常に除外する。
- Skill本文と設定本文は、opt-inの有無にかかわらず送らない。Skill descriptionなど、候補探索に必要な限定メタデータだけをコード側で選ぶ。
- $JEVX_HOMEへ保存するHook／review recordとreceiptには本文を入れず、digest、文字数、finding、route、latency、適用結果、費用だけを残す。
- unknown／unavailableの費用は0円にしない。推定費用と実費を分け、通貨とprice versionを保存する。

## やらないこと（Non-goals）

- Skill本文の黙ったロード・実行、Jev確信度によるauto-allow、権限付与
- Codex公式Compactionの置き換え、会話の勝手な要約・書き換え、Tool Resultの自動削除
- prompt／plan／diff／final answerの明示opt-inなしの外部送信、Skill本文／設定本文の外部送信
- API key、資格情報、秘密値、raw Tool resultの外部送信
- HookだけでCodexのmodel／reasoning切替が必ず成功したという主張。証拠がなければdegraded
- legacy/の現行機能化、Windows対応の約束、費用上限の導入（現段階は観測を優先）
- 速度だけ、精度だけ、費用だけを単独の成功条件にすること

自動fixは明示opt-inと--yesが必要。backup／rollbackは作らず、legacy/、.git、秘密ファイル、期待hash不一致、timeout、低確信度、入力欠落、契約不成立では書き換えない。このリスクを受け入れられない利用者にはreview-onlyを使う。

## 互換性の約束

守る公開境界は次のとおり。

- CLIサブコマンド、公開引数、終了コード
- --jsonのキーとschemaVersion
- $JEVX_HOME配下のJSONL schema
- hooks.jsonへ書く--jevx-managed付きcommandと、既存設定を保持するmerge／uninstall

既存のsuggestion／telemetry／decision receiptは各現行schemaと旧schemaを読み取り、hook recordはschema v1〜v5をschema v6へ、Compaction checkpointはschema v1〜v3を読み取りつつ新規出力をschema v4へ、data inventoryはschema v7へ安全に移行する。data inventoryはmanaged leaf symlinkをリンク先へ追従せず、purge時のCompaction directory identityをlock保持中に検証する。review／route／fixはそれぞれschemaを持ち、receiptには本文を含めない。キーを消す・意味を変える場合はschemaVersionを上げ、移行とjevx/tests/contract_requirements.rsを同時に更新する。名前と意味がずれるキーは消さず、正しいキーを追加する。

## 成功条件

- Skill選択、Compaction、計画／推論の3領域でbaseline比20%以上短縮を、同じ入力・環境・再試行条件で再現できる
- 通常Hookの判定・指示生成p95が800ms以下で、昇格経路の発生率・p95・品質差・追加費用を分離できる
- 4レビューカテゴリをfixtureで再現し、review receiptをJevなしにreplayできる
- Jev → Luna → Terra → Solのfallback順をfixtureで検証できる
- route未適用をappliedとして記録しない
- secrets、API key、raw Tool resultの外部送信が0件
- Jev費用、Codex費用、合算、task／turn／session集計、baseline差分、成功review／fix単価を提示できる
- 取得不能な費用がunknown／unavailableとして残り、0円に見えない
- 新規実行コードのline coverage 98%以上、fmt・clippy・unit・integration・contract・fixture評価が通る

数値の採否は docs/developers/jevx-value.md と docs/developers/jevx-cost-observability.md を正本とする。歴史的な実測値は現行保証に使わない。

## OSSとしての判断

問題・受け入れ基準・捨てるものをIssueまたは設計メモに書く。既定値で解ける問題に設定を追加しない。変更範囲を小さくし、実測がない機能はblockingにしない。設計原則のレビューにはoss-design Skillを使い、詳細なチェック結果は docs/developers/jevx-documentation-audit.md へ記録する。

## 貢献とライセンス

Open-Source, not Open-Contribution。利用・フォーク・再現手順付きのIssue・秘密を含まないfixtureは歓迎するが、外部Pull Requestは原則受け付けない。ライセンスはMIT。
