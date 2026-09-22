# Jevを活用したアプリ候補案

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な調査メモ・未実装候補

> 検討日: 2026-09-20

この文書で扱う [`旧Jev Triage実装`](../../src/) は、問い合わせ文を `choice`・`score`・`boolean` の3問で評価し、カテゴリ・緊急度・返金要求をまとめて表示していた旧Node.js Webアプリだよ。現行のサポート対象は `jevx` Rust CLIであり、この文書では過去の実装を候補検討の参考として扱う。以下の候補案は未実装で、現行ロードマップやcanonical runbookではない。

公開事例・GitHub実装・ブログ・SNSまで横断した詳細調査は、[Jev活用アイデア徹底調査レポート](jev-research.md) にまとめているよ。候補を広く比較したいときや、質問設計・評価方法まで確認したいときはこちらを見てね。

## 結論

詳細調査レポートのランキングを、この2026-09-20時点の推奨順として扱う。次に作る候補は、目的別にこの3案がおすすめだが、実装開始の承認やロードマップ確定を意味しない。

1. **Jev固有の判断基盤を試す:** Confidence-Gated Agent Tool Router
2. **LLM回答の根拠を検証する:** Citation / Response Verifier
3. **旧Node.js Webアプリを最短で再設計する:** Bug / Support Triage Board

PR / CI Review GateとIncident Alert Routerは有力な代替候補として下の比較表に残している。候補の詳細設計、評価、出典の正本は[詳細調査レポート](jev-research.md)にあり、この短いメモの旧順位より優先する。

Jevは自由文を生成するよりも、同じ入力に対して複数の型付き質問へ構造化された回答を返す用途に向いている。分類、ルーティング、ルーブリック評価のように「判断項目を先に定義できる」アプリを優先した。

## 評価軸

- **Jevとの相性:** `choice`・`score`・`boolean` で判断項目を表現しやすいか
- **MVP難易度:** 入力画面、評価、結果表示までの実装量
- **期待価値:** 判定の標準化、対応時間短縮、見落とし防止への寄与
- **リスク:** 個人情報、誤判定、最終判断を自動化してはいけない領域

`responseMs` は旧Jev Triage実装と同じく、Gatewayへ送信してJevの構造化レスポンスを読み終わるまでのアプリ側計測値として表示する。ネットワークとGatewayを含む値なので、案どうしを比較する場合は同じ環境・入力条件で計測する。

## 候補比較

| 優先 | アプリ案 | 入力するstate | Jevへの質問例 | MVP難易度 | 期待価値 | 主なリスク |
| --- | --- | --- | --- | --- | --- | --- |
| P1 | **Bug / Support Triage Board** | 問い合わせ、再現手順、環境、顧客情報 | category、urgency、返金要求、再現可能性、顧客影響、範囲外シグナル | 低 | 高 | PII、P0の見落とし |
| P1 | **PR / CI Review Gate** | PR概要、差分要約、変更ファイル、CI結果 | リスク区分、リリース準備度、テスト合格、要レビュー | 中 | 高 | 誤ったブロック、秘密情報 |
| P1 | **Incident Alert Router** | アラート、サービス、直近デプロイ、ログ要約 | 担当ルート、緊急度、顧客影響、重複アラート | 中 | 高 | 誤ルーティング、偽陰性 |
| P2 | **Product Feedback Inbox** | 顧客の声、プラン、利用機能、発生頻度 | テーマ、機会スコア、機能要望、解約リスク | 低 | 中 | 顧客・市場の偏り |
| P2 | **Expense Policy Checker** | 支出内容、金額、領収書テキスト、規程 | 費目、規程適合度、領収書有無、承認要否 | 中 | 高 | 金融情報、誤拒否 |
| P2 | **Meeting / Support Action Extractor** | 議事録・通話メモ・チケット履歴 | アクション有無、担当者不足、緊急度、次アクション | 低 | 中 | PII、担当者の誤推定 |
| P2 | **Content / Brand Preflight** | 投稿文、掲載先、対象地域、根拠リンク | リスク区分、ブランド適合度、規制表現、要出典 | 低 | 中 | 法務・医療表現の誤判定 |
| P2 | **Release Readiness Dashboard** | リリース差分、チェックリスト、ロールバック計画 | 準備度、ロールバック有無、高リスク変更、担当者 | 中 | 高 | デプロイ判断の過度な自動化 |

## 既存候補の詳細

この節は、旧Node.js Webアプリからの拡張と開発・運用候補を具体化した旧メモとして残している。現行jevxの仕様説明ではなく、候補の設計比較として読むこと。上位候補3案と当時の順位は、冒頭と[詳細調査レポート](jev-research.md)を参照する。

### 1. Bug / Support Triage Board

旧Jev Triage実装を最も自然に拡張できる案。問い合わせを1件ずつ判定するだけでなく、複数件を一覧化して担当チーム・優先順位・対応期限まで見えるようにする。

#### Jevに渡すstateと質問

```json
{
  "message": "決済後に画面がエラーになり、注文履歴にも表示されません",
  "reproduction": "iOS 18 / アプリ 3.2.0 / 再起動後も再現",
  "customerTier": "business",
  "metadata": { "ticketId": "SUP-1234", "createdAt": "2026-09-20T09:00:00Z" }
}
```

- `category`: 旧Jev Triage実装と同じ請求、アカウント、技術、配送の `choice`
- `urgency`: 旧Jev Triage実装と同じ低・中・高の3段階 `score`
- `reproducible`: 再現手順またはログから再現可能性を判定する `boolean`
- `customerImpact`: 継続利用や複数ユーザーへの影響を判定する `boolean`
- `refundRequested`: 旧Jev Triage実装で使っていた返金要求の `boolean`
- `outOfScope`: 既存4カテゴリに該当しない問い合わせを判定する `boolean`

`needsHuman`はJevへ送る質問ではなく、上記6つの回答とconfidence、欠損状態をコードで集約して導出する。回答欠損、低confidence、範囲外、高緊急度、返金要求、顧客影響のいずれかは `needs-review` に送る。

#### MVP

1. 新規の入力画面（旧Node.js Webアプリを参考にする場合）に再現手順・顧客プランを追加する。
2. 1回の評価で6問を送信し、結果をカードと `Jev response xxx ms` で表示する。`needsHuman`は6問の結果からコードで計算する。
3. 結果をローカルの一覧に追加し、カテゴリ・深刻度で絞り込む。

#### 注意点

重大判定は自動クローズや自動返信に直結させず、人が確認する。顧客名・メールアドレス・注文番号などはstateへ送る前にマスキングする。

### 2. PR / CI Review Gate

Pull Requestの差分とCI結果をまとめ、レビューが必要な変更を早めに見つける案。コード生成ではなく、既にある差分を一定の観点でチェックするため、Jevの構造化評価と相性がよい。

#### Jevに渡すstateと質問

```json
{
  "title": "決済エラー時のリトライ処理を追加",
  "diffSummary": "決済クライアントと注文状態更新を変更。認証処理は未変更。",
  "changedFiles": ["src/payment.js", "test/payment.test.js"],
  "checks": {
    "unit": "passed",
    "lint": "passed",
    "coverage": { "status": "passed", "percent": 99.1, "requiredPercent": 98.0 }
  }
}
```

- `risk`: 低リスク、要レビュー、高リスクの `choice`
- `readiness`: 未準備・確認中・準備完了の `score`
- `securitySensitive`: 認証・決済・個人情報に触れる変更かの `boolean`

`testsPassing`はJevへの質問ではなく、unit・lintが`passed`、coverageの`status`が`passed`、かつ`percent >= requiredPercent`（この例では98.0）を満たす場合だけコードで`true`にする値。項目のmissing、failure、閾値未満はfalseとし、Jevがテスト合否を推測したり上書きしたりしない。

#### MVP

1. GitHub連携の前に、PR概要・差分要約・CI結果を貼れるフォームを作る。
2. 結果に「レビュー必須の理由」と各質問の確率を表示する。
3. 確信度が低い結果は `needs-review` として扱い、自動マージや自動ブロックは行わない。

#### 注意点

Jevの評価だけでマージ可否を決めない。既存のCI、CODEOWNERS、静的解析を主判定にし、Jevはレビュー観点の追加と優先順位付けに使う。差分全文や秘密情報をそのまま送らない設計も必要。

### 3. Incident Alert Router

監視アラートを担当チームへ振り分け、緊急度と顧客影響を揃えて表示する案。インシデント対応の初動で、通知先と優先順位を決める補助に使う。

#### Jevに渡すstateと質問

```json
{
  "alert": "注文APIの5xx率が8%を超過",
  "service": "order-api",
  "recentDeploy": "2026-09-20 08:45 JST: order-api v2.8.0",
  "logsSummary": "決済プロバイダのtimeoutが増加。直近15分のエラー率は上昇中。"
}
```

- `route`: backend、payments、security、supportなどの `choice`
- `urgency`: 通知不要・通常・至急の `score`
- `customerImpact`: 顧客操作に影響しているかの `boolean`
- `duplicate`: 既存インシデントと同一かの `boolean`

#### MVP

1. アラート内容とログ要約を貼れる入力画面を作る。
2. 担当ルート・緊急度・顧客影響を1回で評価する。
3. `responseMs` と判定結果を履歴に残し、対応者が修正できるようにする。

#### 注意点

偽陰性のコストが高いので、「通知不要」は自動抑制に使わず、まずはルーティング候補として表示する。実運用では固定ルールやオンコール設定を優先し、Jevには判断材料の要約と重複候補の提示を任せる。

## 共通MVPアーキテクチャ

1. **入力アダプター:** フォーム、Webhook、CSVなどから必要な項目だけ受け取る。
2. **state正規化:** 文字数、PII、ログのサイズをチェックし、Jevに渡す形式を揃える。
3. **一括評価:** 3〜5個の `choice`・`score`・`boolean` 質問を1リクエストで送る。
4. **結果表示:** ラベル、確率、理由、`responseMs`、使用量を表示する。
5. **人の確認:** 確信度が低い結果や高リスク領域は `needs-review` にする。
6. **評価ログ:** 人の修正結果を保存し、質問文・閾値・入力マスキングを改善する。

確率は自動決定の根拠ではなく、優先順位付けの補助信号として扱う。運用開始後にラベル付きサンプルで誤判定を測定し、閾値を決めるのが安全。

## 優先順位と次の一手

| 目的 | 選ぶ案 | 最初に作る画面 |
| --- | --- | --- |
| Jev固有の判断基盤を試す | Confidence-Gated Agent Tool Router | 要求・許可候補・確認ルートのシミュレーター |
| LLM回答の根拠を検証する | Citation / Response Verifier | claim・evidence・送信可否の検証画面 |
| 旧Node.js Webアプリを最短で再設計する | Bug / Support Triage Board | 問い合わせ一覧 + 追加評価フォーム |
| 開発フローに組み込む | PR / CI Review Gate | PR情報貼り付けフォーム + review結果 |
| 運用・SREに広げる | Incident Alert Router | アラート入力 + 担当ルート候補 |
| 大量処理とコード側検査を測る | CSV / Document Quality Auditor | CSV入力 + 行別監査結果 |

2026-09-20時点の候補順位では、詳細レポートの順に **Confidence-Gated Agent Tool Router** を第一候補とした。旧Node.js Webアプリを最短で再設計する場合だけ **Bug / Support Triage Board** を選び、当時の入力・評価・レスポンス時間表示・テストを参考にする。これは未実装候補の比較結果であり、実装着手やロードマップ確定ではない。PR / CI Review GateとIncident Alert Routerは、Tool Routerの安全ポリシー設計を開発・運用へ展開する次の候補とする。

採用・与信・医療・法務の最終判断のような高リスク用途は、現段階では候補から外す。Jevの結果は人の確認を支える情報に限定し、最終判断を自動化しない。

## 参考資料

- [Vercel AI Gateway Evaluation](https://vercel.com/docs/ai-gateway/modalities/evaluation)
- [Jev API, Pricing & Playground](https://vercel.com/ai-gateway/models/jev)
- [旧Jev Triage実装](../../src/)
