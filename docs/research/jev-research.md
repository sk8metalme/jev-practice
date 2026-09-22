# Jev活用アイデア徹底調査レポート

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な調査レポート

> 調査日: 2026-09-20
>
> 目的: 公開情報をもとにJevの実際の使われ方と未充足の実装機会を整理し、このリポジトリで次にローカル実装する候補を選べるようにする。

## 先に結論

JevはLLMの代替というより、**コードが制御するワークフロー内の判断部品**として使うと価値が出やすい。入力stateを小さく分け、Jevには一つずつの判断を複数問まとめて依頼し、結果の組み合わせ・閾値・副作用は通常のコードが担当する。

このリポジトリで次に作る候補は、次の順がおすすめ。

| 順位 | 候補 | 推す理由 |
| --- | --- | --- |
| 1 | **Confidence-Gated Agent Tool Router** | Jevの「選ぶ・危険度を測る・不確実なら止める」を最小のローカルデモで検証でき、実際の副作用をシミュレーションに閉じ込められる。 |
| 2 | **Citation / Response Verifier** | 生成LLMとJevを組み合わせるCascadeを可視化でき、正解データと「根拠があるか」を分離して評価しやすい。 |
| 3 | **Bug / Support Triage Board** | 既存のJev Triageをそのまま拡張でき、問い合わせ・緊急度・人手確認・レスポンス時間を再利用できる。 |
| 4 | **CSV / Document Quality Auditor** | 大量データを1行ずつ意味判定するJevの経済性を検証でき、数値・日付の厳密処理をコードへ分離しやすい。 |
| 5 | **PR / CI Review Gate** | 開発者向け価値が高く、公開実例も多い。ただしコードレビュー領域は競合が多く、最終判断は既存のCIと人に残す必要がある。 |

ブラウザ操作、ゲーム、ロボティクス、トレーディングはJevの速度を見せるデモとして魅力的だけど、最初の実装としては外部状態・副作用・検証コストが大きい。採用・与信・医療・法務の最終判断は、高リスクかつ公平性・説明責任の検証が必要なため、現段階では実装候補から外す。

## 1. 調査範囲と証拠レベル

### 1.1 調査した情報源

- TypeSafe AIの公式ブログ・公式ドキュメント・ワークフロー評価
- Vercel AI GatewayのJev仕様・モデルページ
- 公開GitHubリポジトリ、README、実装構成、テスト・計測結果
- 技術ブログ、Qiitaなどの実装・日本語検証記事
- X投稿を集約する公開ギャラリー、Redditの開発者投稿・コメント
- このリポジトリの実装と既存の [`jev-app-ideas.md`](jev-app-ideas.md)

調査スナップショットは2026-09-20時点。Jevの公開直後であるため、初週に投稿されたデモや自己申告の数値が多く、長期運用の証拠はまだ少ない。

### 1.2 証拠レベル

| レベル | 意味 | レポートでの扱い |
| --- | --- | --- |
| A | TypeSafe / Vercelの公式資料 | 仕様・公式の設計意図・公式評価として引用する。性能値は評価条件も併記する。 |
| B | 公開コードと再現手順があるGitHub実装 | 実装パターンの根拠にする。作者のベンチマークは自己申告として扱う。 |
| C | 技術ブログ・実測記事 | 設計上の発見や失敗例として使う。測定条件とサンプル数を確認する。 |
| D | SNS・コミュニティ投稿・アイデア段階 | 需要や発想のシグナルとして扱い、一般的な性能や市場性の証明にはしない。 |

「型どおりに返る」と「意味的に正しい」は別。さらに「正しい判定」と「その判定に基づく副作用を実行してよい」は別なので、候補案ごとにこの3つを分けて記録する。

## 2. Jevの仕様から分かる適性

### 2.1 基本モデル

Jevはstateとtyped questionsを受け取り、自由文ではなく型付きの判断を返す。TypeSafe公式の説明では、JevはSystem Oneモデルとして、ソフトウェアが直接利用できる構造化された判断を目的にしている。

| 質問 | 使う場面 | 主な回答 |
| --- | --- | --- |
| `Choice` | 固定された選択肢から1つを選ぶ | `choice`、選択肢ごとの`probabilities`、`confidence` |
| `Score` | 順序のある評価軸に置く | `score`、`legend`、レベルごとの`probabilities`、`confidence` |
| `Noul` | ある命題が真である確率を得る | `noul`（0〜1） |

TypeSafe SDKでは`Noul`、Vercel AI Gatewayの評価APIでは同じ概念のyes/no判定を`boolean`として扱う例がある。このリポジトリの現行実装もGateway形式の`boolean`を使っているため、本文ではSDKの概念を`Noul / boolean`と併記する。

複数の質問は同じstateに対して独立に評価され、1リクエスト内で並列処理される。質問同士の組み合わせ、重み、閾値、実際の処理はコード側で定義するのが基本になる。

### 2.2 Jevに向く判断

- 問い合わせやメールの意図・担当部署・優先度の分類
- 危険・PII・規約違反・緊急性の検知
- 深刻度・関連度・品質・顧客の不満などのScore評価
- deterministic code、専門LLM、人へ振り分けるルーティング
- LLMの入力・出力・ツール呼び出し・引用根拠の検証
- 大量の行・文書・候補の意味判定と再ランキング
- ゲームやUIの状態を見た次アクション選択

### 2.3 Jevに向かない判断

公式の`jev-1.13`既知の弱点と公開実装の失敗例を合わせると、次はコードへ戻すべき領域。

- 四則演算、集計、長いリストの正確なカウント
- 日付の前後・期間・タイムゾーンの厳密な比較
- 複数段の参照や二重否定を含む複雑な推論
- 質問と関係ない大量のstateを渡すこと
- stateに埋め込まれた誘導文やプロンプトインジェクションへの無対策
- 自由な文章・コード・説明の生成
- 医療・法務・与信・採用など、判定単体に最終権限を持たせること

Jevに計算させるのではなく、コードで計算した結果をJevの意味判断に渡す。例えば日時はコードでISO形式へ正規化し、Jevには「この期限は近いか」のような境界の意味判断だけを依頼する。

### 2.4 速度・コスト・confidenceの読み方

TypeSafeのローンチ記事は、Jevの入力価格を`$0.042 / 1M input tokens`、出力を無料、エンドツーエンドを70〜500msとして説明している。Vercelのモデルページは入力価格を約`$0.04 / 1M tokens`と表示している。これはプロバイダ・時点・経路で変わり得るため、本レポートでは「低コスト・低遅延という公式主張」として扱い、実アプリでは`usage`と実測値を記録する。

現在のアプリの`responseMs`は、GatewayへのHTTP往復とJSON応答の読み取りを含むアプリ側の経過時間。Jevのプロバイダ処理時間そのものではない。候補同士を比較する場合は、同じ地域・入力サイズ・質問数・Gateway経路でp50/p95を取る。

confidenceは「正しさの保証」ではなく、行動を変えるための補助信号。低confidenceは人へ、高confidenceでも破壊的な処理にはより高い閾値や確認を要求する。閾値は候補ごとのラベル付きデータで決める。

boolean / Noulの確率 `p` をconfidenceとしてそのまま扱わない。命題が真である確率が `p` のとき、命題の確信度は `max(p, 1-p)` とし、`p` 自体は「真に寄せる閾値判定」に使う。例えば `p >= 0.9` は真として進める候補、`p <= 0.1` は偽として進める候補、`0.1 < p < 0.9` は不確実として確認へ送る。Choiceは選択肢の最大確率、Scoreはlegendの最大確率をconfidenceとして記録する。

## 3. 公開事例から見えた利用パターン

### 3.1 エージェントの行動選択・ガードレール

| 実例 | Jevの役割 | 調査で分かった設計 |
| --- | --- | --- |
| [Jev Ultrafast](https://github.com/browser-use/jev-ultrafast) | DOMの観測状態から操作と対象要素を選ぶ | Jevは操作選択、小型LLMは入力文字列、コードは対象の再検証を担当する。モデル出力をそのままセレクタやコードにしない。 |
| [TypeSafe Playgroundのtool router](https://github.com/TypeSafeAI/typesafe-playground/blob/main/docs/tool-router.md) | 許可されたグラフの次ノードを選ぶ | 候補集合をコードで制限し、秘密情報を含む要求をルールで先に遮断し、confidence不足は確認へ送る。 |
| [pi-warden](https://github.com/DevMortimer/pi-warden) | 破壊的操作、ルール違反、停滞、未検証の完了宣言を監視 | 既知パターンをローカルルールで処理し、判断が必要なケースだけJevへ送り、通常はエージェントへ修正を返す。 |
| [OpenRouterのJev-Verified Cascade](https://openrouter.ai/docs/cookbook/evaluate-and-optimize/jev-verified-cascade) | 生成された回答が根拠に支えられているか検証 | 安価なモデル→Jev検証→失敗時だけ強いモデルまたは人、という段階構成。 |

**共通する新しい発想:** Jevを「全部を実行するエージェント」にせず、候補を選ぶ・危険度を測る・不確実性を示す部品に限定する。実行可能な候補、認可、引数検証、再確認はコード側に置く。

### 3.2 開発者・運用向けの意味判定

公開GitHubには、次のような開発者向け実装が複数ある。

- [Jev Review](https://github.com/devagrawal09/jev-review): 差分・コードベースを段階的に見て、リスク、証拠、深刻度、担当ルートを構造化評価する。
- [clarity-judge](https://github.com/TypeSafeAI/clarity-judge): 文章の複数品質軸を個別の判定として表示し、custom checkを追加できる。
- [awesome-jevの開発者ツール一覧](https://github.com/kraayenjon/awesome-jev#applications): semantic code search、semantic lint、secret detection、commit分類、ログトリアージ、停滞検知などを列挙している。

コードレビュー単体は既に競争があるため、作るなら「Jevがレビューを書いてくれる」ではなく、「どのチェックをいつ人へ渡すか」「不確実性をどう可視化するか」「CIや既存ルールとどう合成するか」に焦点を置く。

### 3.3 検索・データ・大量処理

- [pg-jev](https://github.com/realZachi/pg-jev)は、PostgreSQLの行に対して自然言語の条件・分類・スコアを適用し、SQLの`WHERE`、`ORDER BY`、`GROUP BY`と組み合わせる実験をしている。
- [TypeSafe Jev Examples](https://github.com/rajivkuriakose/typesafe-jev-examples)は、問い合わせルーティング、候補のScore、再ランキング、独自データに対する評価とポリシーの単体テストを分ける構成を示している。
- [TypeSafe公式のUse Case Map](https://docs.typesafe.ai/concepts/use-case-map)は、検索、再ランキング、論文スクリーニング、引用検証、CSV検証、特徴量抽出、知識グラフを候補領域として挙げる。

大量処理でのポイントは「1件ずつJevへ投げる」ではなく、stateの構造化・バッチサイズ・キャッシュ・ローカル前処理・正解データによる評価を設計すること。データの完全性や数値制約はJevに丸投げしない。

### 3.4 業務・コンテンツ・コミュニティ

公開事例・公式マップ・コミュニティ投稿から、次の用途が繰り返し現れる。

- Support / email intent routing
- Invoice・expense・claimの分類と要確認判定
- Lead scoring、広告・投稿の品質評価、ブランド安全性
- Discord・SNS・ゲーム内チャットのmoderation
- CV・応募書類・論文タイトル/抄録のスクリーニング
- 引用が主張を支持するかのverification

これらは業務価値を説明しやすい反面、PII、差別・公平性、法的影響、誤ブロックのリスクが高い。人の確認、監査ログ、異議申し立て、モデル以外のルールを最初から含める必要がある。

### 3.5 ゲーム・リアルタイム・デバイス

TypeSafe公式はDoomやWikiracingを、コミュニティはMario、Pac-Man、Tetris、Snake、チェス、Android操作、macOS Computer Use、Home Assistant、ドローンなどを公開している。

この領域は「数百ミリ秒の判断をループへ入れる」というJevの特徴を伝えるのに適している。一方で、勝率や安全性の評価はゲームごとに異なり、デモの成功を一般的なエージェント能力の証拠にしてはいけない。現実のデバイスや金銭を動かす場合はシミュレータ・安全境界・kill switchが必須。

## 4. 候補アイデア20件

| # | 候補 | Jevが決めること | stateの中心 | MVP難易度 | 証拠 / 先行例 | 主な注意 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | **Confidence-Gated Agent Tool Router** | 次の許可済みツール、危険度、要確認 | request、現在ノード、許可候補、ルール | 中 | A/B: [公式Intent Routing](https://docs.typesafe.ai/patterns/intent-routing)、[tool router](https://github.com/TypeSafeAI/typesafe-playground/blob/main/docs/tool-router.md)、[pi-warden](https://github.com/DevMortimer/pi-warden) | 選択結果を実行コマンドに直結しない |
| 2 | **Citation / Response Verifier** | 根拠が支持・部分支持・不支持か | 質問、抜粋、ドラフト回答、出典 | 中 | A/B: [公式Use Case Map](https://docs.typesafe.ai/concepts/use-case-map)、[OpenRouter Cascade](https://openrouter.ai/docs/cookbook/evaluate-and-optimize/jev-verified-cascade) | 支持判定は真実性の証明ではない |
| 3 | **Bug / Support Triage Board** | 種別、緊急度、再現性、顧客影響、担当 | ticket、account、product、既知障害 | 低 | A/B: [Workflow Evals](https://evals.typesafe.ai/)、[既存アプリ](../../README.md)、[Jev Examples](https://github.com/rajivkuriakose/typesafe-jev-examples) | P0・返金・PIIは人手確認 |
| 4 | **CSV / Document Quality Auditor** | 意味的な欠損、異常、分類、要確認 | row、schema、policy、source metadata | 中 | A/B: [jev-curate](https://github.com/AkashPriyadarshii/jev-curate)、[pg-jev](https://github.com/realZachi/pg-jev)、[TiAb Review](https://github.com/youkiti/tiab-review-plugin) | 数値・日付・件数はコードで検証 |
| 5 | **PR / CI Review Gate** | リスク、レビュー観点、担当、準備度 | diff要約、変更ファイル、CI、ポリシー | 中 | B: [jev-review](https://github.com/devagrawal09/jev-review)、[clarity-judge](https://github.com/TypeSafeAI/clarity-judge) | 自動merge・自動blockをしない |
| 6 | **Incident / Log Triage Router** | アラートの担当、優先度、顧客影響、重複 | alert、service、deploy、ログ要約 | 中 | B/C: [Jev Logs](https://github.com/reachjalil/jevlogs)、[log triage記事](https://dev.to/reachjalil/how-we-tuned-typesafe-jev-for-log-triage-without-alert-storms-1ei0) | 通知抑制の唯一の根拠にしない |
| 7 | **LLM Tool-Call Firewall** | jailbreak、secret、policy violation、harm | prompt、tool call、resource、policy | 中 | A/B: [LLM guardrails](https://docs.typesafe.ai/cookbooks/llm_guardrails)、[tripwire](https://github.com/noelzappy/tripwire)、[jev-secret-detection](https://github.com/teyhouse/jev-secret-detection) | fail closedと監査ログが必要 |
| 8 | **Agent Progress / Stagnation Gate** | 継続、警告、再計画、停止 | 目標、履歴、直近結果、反復回数 | 中 | B: [ProgressGate](https://github.com/AshutoshVJTI/progressgate)、[pi-warden](https://github.com/DevMortimer/pi-warden) | 停止判定で正当な長時間処理を壊す可能性 |
| 9 | **Model Router** | 小型LLM、強いLLM、コード、人のどれへ送るか | request、domain、difficulty、risk、budget | 中 | B: [jev-router](https://github.com/gargpratyush/jev-router)、[公式Intent Routing](https://docs.typesafe.ai/patterns/intent-routing) | 高難度を安価モデルへ誤送しない |
| 10 | **Context Compaction / Relevance Filter** | 残すtool result、捨てるノイズ、再取得要否 | agent history、tool result、goal | 中 | B: [fast-jev-compaction](https://github.com/tamaratran/fast-jev-compaction)、[SkillRanker](https://github.com/Dicklesworthstone/skillranker) | 重要な証拠を削除しない評価が必要 |
| 11 | **Browser Action Selector** | click、select、scroll、wait、done、blocked | DOM/ARIA要素表、目標、直近結果 | 高 | B:Jev Ultrafast、jev-browser | Webサイトごとの変化、認証、副作用 |
| 12 | **Semantic Code / Document Search** | 候補ファイル・関数・文書の関連度 | query、candidate、必要な証拠 | 低〜中 | B: [Every](https://github.com/sufianetaouil/every)、[blink](https://github.com/ellipsis-dev/blink)、[Jev Search](https://github.com/superagents-lab/jev-search) | 検索漏れとランキングを別評価 |
| 13 | **Natural-Language SQL Filter** | 行が条件に合うか、分類、順位 | row、自然言語条件、既存列 | 中 | B: [pg-jev](https://github.com/realZachi/pg-jev)、[sqlite3-jev](https://github.com/mattn/sqlite3-jev) | 大量行のコストとデータ漏洩 |
| 14 | **Research Paper Screening** | inclusion、exclusion、topic、evidence quality | title、abstract、criteria、metadata | 中 | B: [TiAb Review](https://github.com/youkiti/tiab-review-plugin)、[1kpapers](https://madewithjev.com/builds/1kpapers) | 医学判断ではなく文献選別に限定 |
| 15 | **Invoice / Expense Policy Checker** | 費目、規程適合、領収書、承認要否 | invoice、receipt、order、policy | 中 | A: [Workflow Evals](https://evals.typesafe.ai/)、[Use Case Map](https://docs.typesafe.ai/concepts/use-case-map) | 支払いの最終承認を自動化しない |
| 16 | **Email / Lead Intent Router** | 問い合わせ種別、購入意図、担当、緊急度 | email、account、campaign、ICP | 低〜中 | B:email workflow、Xのlead分析投稿 | 顧客評価のバイアス、個人情報 |
| 17 | **Content / Brand Preflight** | ブランド適合、規制表現、根拠、audience fit | post、channel、region、policy | 低〜中 | C/D: [Made with Jev](https://madewithjev.com/)、[Jev Web Analyzer](https://github.com/replynodes/jev-web-analyzer) | 法務・医療の最終判断は人へ |
| 18 | **Moderation / Community Safety** | allow、warn、review、block、危険度 | message、前後文脈、channel rules、user state | 中 | A/B: [Use Case Map](https://docs.typesafe.ai/concepts/use-case-map)、[Jev Moderation Bot](https://github.com/brainstormity/Jev-Moderation-Bot) | 誤ブロック、異議申し立て、公平性 |
| 19 | **Structured Record Normalizer** | boundedな属性・カテゴリ・欠損の判定 | raw record、schema、allowed values | 低 | A:公式Structured Data Extraction | 自由文の完全な抽出・生成には使わない |
| 20 | **Realtime Game / Device Decision Lab** | 次の行動、状態遷移、危険回避 | 構造化された環境状態、候補行動 | 高 | A/B:Doom、Mario、Browser/robotics実例 | 現実の機器・資金に接続しない |

## 5. 上位5案の詳細設計

詳細節は設計を比較しやすい順に掲載している。候補の推奨順位と合計点の正本は、冒頭の結論と第7節のスコア表であり、詳細節の番号順とは別に扱う。

### 5.1 Confidence-Gated Agent Tool Router

#### 何を作るか

ユーザーの要求と現在のワークフロー状態を受け取り、「許可済み候補のどれを次に実行するか」「確認が必要か」「危険なので停止するか」を決めるローカルのルーター。実際のshell・ファイル・外部APIは実行せず、シミュレーション結果だけを表示する。

#### state

```json
{
  "request": "設定ファイルのタイムアウトを確認して",
  "currentNode": "operations",
  "allowedActions": [
    { "id": "read_config", "description": "設定を読む", "risk": "read-only" },
    { "id": "modify_config", "description": "設定を変更する", "risk": "approval-required" }
  ],
  "blockedActions": [
    { "id": "export_secret", "description": "秘密情報を外部へ送る", "risk": "blocked" }
  ],
  "projectRules": ["秘密情報の外部送信は禁止", "変更は承認後のみ"],
  "recentResults": []
}
```

Stage Aの回答後、コード側で候補IDを解決し、検証済みの引数と元の要求を含む`selectedInvocation`をStage Bへ渡す。Jevには未検証の引数や任意の実行コマンドを生成させない。

```json
{
  "selectedAction": {
    "id": "read_config",
    "description": "設定を読む",
    "risk": "read-only"
  },
  "selectedInvocation": {
    "actionId": "read_config",
    "validatedArguments": { "path": "config/app.json" },
    "requestContext": {
      "request": "設定ファイルのタイムアウトを確認して",
      "currentNode": "operations"
    }
  },
  "projectRules": ["秘密情報の外部送信は禁止", "変更は承認後のみ"]
}
```

#### 質問とポリシー

- **Stage Aの`nextAction`**: `Choice`。`allowedActions`の候補IDに加えて`clarify`と`stop`を含める。候補はコード側で生成し、Jevが候補外や`blockedActions`のIDを返しても拒否する。
- **Stage Bの`riskLevel`**: `Score`。Stage Aでコードが解決した`selectedAction`だけをstateに入れ、read-only、可逆変更、承認必須、不可逆・禁止の意味を評価する。選択前の候補一覧をまとめて評価しない。
- **Stage Bの`containsSecretEgress`**: `Noul / boolean`。検証済み引数と元の要求を含む`selectedInvocation`が秘密情報を外部へ送るかを評価する。候補一覧や無関係なblocked actionは参照しない。
- **Stage Bの`requiresHumanApproval`**: `Noul / boolean`。「このselectedActionはユーザー確認なしに実行してよいか」ではなく、「このselectedActionには人の承認が必要か」を直接問う。trueは必ず確認ルートに送る。

コード側の既定動作は、blocked actionと禁止ルールをJev評価前に停止し、Stage Aの候補外・回答欠損・未知IDも停止する。Stage Aで選ばれたIDをコード側で`selectedAction`へ解決し、引数を検証した`selectedInvocation`を作ってからStage Bを呼ぶ。コード定義のriskを権威ある値として扱い、引数・宛先・元の要求によって秘密情報の外部送信になる場合もStage Bで検出する。Stage Bのsecret egressは停止、`requiresHumanApproval`は確認、低confidenceは再取得または人手確認へ送る。Jevのrisk評価は選択済みactionの意味的な補助判定であり、単独で実行許可を与えない。

#### ローカルMVP

1. 要求、現在ノード、候補アクションを入力するフォーム。
2. ルーティング結果、各確率、confidence、適用したルール、`responseMs`を表示。
3. 「許可」「確認」「停止」の最終ポリシーをコードで再計算。
4. JSONで判断履歴をエクスポート。

#### 評価

- 読み取り、可逆変更、不可逆変更、秘密情報、曖昧な依頼を含む30件以上のfixture。
- 候補外アクションが一度も実行扱いにならないこと。
- blocked actionを候補に混ぜても、read-onlyの`read_config`が誤って停止されないこと。
- selected actionのsecret egressがすべて停止または人手確認になること。
- 許可されたactionでも、検証済み引数や宛先によってsecret egressになるfixtureが停止または人手確認になること。
- `requiresHumanApproval`がtrue、低confidence、回答欠損のケースが実行扱いにならないこと。
- Stage Aの候補外IDとStage Bのselected action不一致が実行扱いにならないこと。
- golden labelに対するroute accuracy、危険操作のrecall、confidence帯ごとの誤判定を記録。

#### 差別化とリスク

単なる「危険かどうか」の1問ではなく、候補制限・ルール・confidence・承認を分離する。Jevの判断だけで実行しないため、デモでも安全境界を説明しやすい。

### 5.2 Citation / Response Verifier

#### 何を作るか

検索結果や社内文書の抜粋と、生成モデルが作った回答を入力し、「回答の各主張が与えられた根拠で支持されているか」を判定する。真実かどうかではなく、**渡された証拠がその主張を支えるか**を評価する。

#### state

```json
{
  "question": "返金期限は何日ですか？",
  "draftAnswer": "購入から30日以内なら返金できます。",
  "claims": [
    { "id": "claim-1", "text": "購入から30日以内なら返金できます。", "citedEvidenceIds": ["doc-12"] }
  ],
  "evidence": [
    { "id": "doc-12", "text": "未使用品は購入後30日以内に返品できます。" },
    { "id": "doc-19", "text": "セール品は返品対象外です。" }
  ],
  "policy": "回答は提示された証拠だけを根拠にする。"
}
```

#### 質問とポリシー

- `support`: `Choice`。supported、partially-supported、unsupported、insufficient-evidence。
- `evidenceCoverage`: `Score`。主要主張の大半を支える、部分的、ほぼ支えない、の段階。
- `containsUncitedClaim`: `Noul / boolean`。根拠にない追加主張があるか。
- `needsEscalation`: `Noul / boolean`。人または強いモデルの再確認が必要か。
- `supports_<claimId>_<evidenceId>`: claimとevidenceの組み合わせごとに生成するboundedな`Noul / boolean`。trueになったIDをコード側で`supportingEvidenceIds`へ集約し、選択された根拠を明示する。claim数とevidence数に上限を置き、質問キーはコードで生成する。

質問の作成、claimの分割、IDの対応付け、`supportingEvidenceIds`の集約はコード側で行い、Jevに自由なIDや回答文を生成させない。`send`は、全claimに1つ以上のsupportingEvidenceIdsがあり、supportがsupported、uncited claimが高確信でfalse、needsEscalationが高確信でfalse、かつ各閾値を満たす場合だけ許可する。insufficient-evidence、根拠ID欠損、シグナル間の不一致、低confidenceは`retrieve-more`または`handoff`へ送る。

#### ローカルMVP

1. 質問、根拠抜粋、ドラフト回答を貼り付ける画面。
2. claimごとのsupportingEvidenceIds、支持状態、confidence、確認理由を表示。
3. `send`、`retrieve-more`、`handoff`の3ルートを表示。
4. 同じ入力で質問文を変えたA/B比較を保存。

#### 評価

- 50件以上の合成QAを、supported / partially-supported / unsupported / insufficient-evidenceの4クラスで手動ラベルする。
- 誤ってsendしたunsupported回答、未引用主張、needsEscalationを見逃した割合を最重要指標にする。
- claimごとのsupportingEvidenceIdsのprecision / recallと、過剰handoffになっていないかを測る。
- evidenceの追加・削除、複数claim、1claimに複数evidence、根拠なしclaimで判定がどう変わるかをテストする。

#### 差別化とリスク

生成モデルの回答品質を直接採点するのではなく、証拠選択・支持判定・送信可否を分ける。引用の存在だけで内容の正しさを保証しないことをUIとREADMEに明記する。

### 5.3 Bug / Support Triage Board

#### 何を作るか

現在のJev Triageを、単一問い合わせの結果表示から複数チケットの一覧・担当・優先度・人手確認まで拡張する。

#### stateと質問

```json
{
  "ticket": {
    "subject": "決済後に注文が表示されない",
    "message": "3日間注文履歴に反映されず、業務に影響しています。",
    "reproduction": "iOS 18 / アプリ3.2.0 / 再起動後も再現"
  },
  "account": { "tier": "business", "region": "jp" },
  "product": { "knownIncidents": ["payment-timeout"] }
}
```

- `category`: `Choice`。現行アプリと同じ`billing`、`account`、`technical`、`shipping`を使う。
- `urgency`: `Score`。現行アプリと同じ0〜2の`low`、`medium`、`high`を使う。
- `refundRequested`: `Noul / boolean`。現行アプリと同じ返金要求の命題を使う。
- `reproducible`: `Noul / boolean`。再現手順と環境情報から再現可能性を判定する追加質問。
- `customerImpact`: `Noul / boolean`。継続利用や複数ユーザーへの影響を判定する追加質問。
- `outOfScope`: `Noul / boolean`。4つの既存カテゴリに該当しない問い合わせかを判定するboundedな追加シグナル。

カテゴリ・緊急度・返金要求は既存アプリのschema、ラベル、Score範囲と互換にする。`intent`という別名や4段階のseverityをそのまま導入しない。プロダクト固有の細かい分類が必要なら、既存`category`からコード側のmappingを明示的に作り、保存済みデータと表示ロジックを移行してから使う。`outOfScope`は既存の保存値を置き換えず、未知カテゴリを`needs-review`へ送るための別シグナルとして扱う。

`needsHuman`はJevへ尋ねず、`category`・`urgency`・`refundRequested`・`reproducible`・`customerImpact`・`outOfScope`の回答、各confidence、欠損状態をコードで検証した後に導出する。たとえば、回答欠損、低confidence、`outOfScope=true`、`urgency=high`、`refundRequested=true`、`customerImpact=true`のいずれかがあればtrueとし、trueは`needs-review`へ送る。これにより、独立評価されたJev回答が人手確認の必要性を上書きしない。P0や返金はconfidenceが高くても自動処理しない。

#### ローカルMVP

- 現行フォームに再現手順・顧客プラン・既知障害を追加。
- 1リクエストのfan-outで質問を評価。
- 結果をローカル一覧へ追加し、カテゴリ・緊急度・needs-humanで絞り込む。
- `responseMs`、usage、判定修正を履歴として表示。

#### 評価

- 日本語の問い合わせ60件以上を手動ラベル。
- `category` macro-F1、`outOfScope`のrecall、重大障害recall、返金要求のprecisionを測る。
- 低confidence・欠損回答・`outOfScope=true`を`needs-review`へ送るテストを追加。
- PIIマスキング前後で判定が変わるケースを用意する。

#### 差別化とリスク

最短で実装できるが、サポートトリアージ自体は公開例が多い。差別化は質問の品質、修正ログ、confidenceの見える化、既知障害とのコード側統合に置く。

### 5.4 PR / CI Review Gate

#### 何を作るか

PRの差分要約・CI結果・変更ファイルから、レビューの観点と担当を優先順位付きで提示する。Jevはレビューコメントを生成せず、コードや既存の静的解析が見逃しやすい意味的な確認を担当する。

#### stateと質問

```json
{
  "pullRequest": {
    "title": "決済エラー時のリトライ処理を追加",
    "diffSummary": "決済クライアントと注文状態更新を変更",
    "changedFiles": ["src/payment.js", "test/payment.test.js"]
  },
  "checks": {
    "unit": "passed",
    "lint": "passed",
    "coverage": { "status": "passed", "percent": 99.1, "requiredPercent": 98.0 }
  },
  "policies": ["認証・決済変更は担当者レビュー必須"]
}
```

- `risk`: `Choice`。low、review-required、high、blocked。
- `readiness`: `Score`。未準備、確認中、準備完了。
- `securitySensitive`: `Noul / boolean`。
- `reviewRoute`: `Choice`。通常レビュー、security、owner、human escalation。

`testsPassing`はJevの質問ではなく、必須CIの`unit`、`lint`、`coverage`をコードで読み、`unit`と`lint`が`passed`、`coverage.status`が`passed`、かつ`coverage.percent >= coverage.requiredPercent`（この例では98.0）を満たした場合だけ`true`にする決定的な値。項目のmissing、failure、閾値未満はfalseとする。

CI・CODEOWNERS・secret scannerが主判定で、`testsPassing`もCIの決定的な結果からコードで作る。Jevの結果は意味的なレビュー観点と担当ルートの優先順位に使い、Jev単独でmerge・deploy・blockを確定しない。

#### ローカルMVP

- PR概要、差分要約、CI結果を貼れるフォーム。
- risk matrix、review route、confidence、確認理由を表示。
- 「人が確認済み」の修正結果を保存し、同じfixtureを再評価できるようにする。

#### 評価

- low-risk、認証変更、決済変更、テスト欠落、秘密情報混入のfixtureを30件以上。
- high-risk変更のrecallと、通常変更を過剰にblockしないprecisionを測る。
- CIがpass、failure、missingの各ケースで、Jevがテスト合否を上書きできないことを確認する。
- diffに未信頼の指示文が含まれるprompt injectionケースをテストする。

#### 差別化とリスク

公開実例が多いため、一般的なAI code reviewではなく「ポリシーと証拠の優先順位付け」に絞る。差分全文や秘密情報をそのままJevへ送らない。

### 5.5 CSV / Document Quality Auditor

#### 何を作るか

CSVや文書レコードを行単位で検査し、機械的に判定できる問題はコード、意味的な問題だけをJevへ送るローカル監査アプリ。

#### stateと質問

```json
{
  "row": {
    "description": "海外出張のタクシー代",
    "amount": 12800,
    "currency": "JPY",
    "receiptText": "Tokyo Taxi 2026-09-18"
  },
  "schema": { "category": "travel", "currency": "JPY" },
  "duplicateCandidates": [
    {
      "id": "EXP-1042",
      "date": "2026-09-17",
      "amount": 12750,
      "currency": "JPY",
      "merchant": "Tokyo Taxi"
    }
  ],
  "policy": "出張交通費は領収書の内容と一致し、業務目的が説明できること。"
}
```

- `semanticMatch`: `Noul / boolean`。説明と領収書・規程の意味が一致するか。
- `hasMissingEvidence`: `Noul / boolean`。必要な領収書・説明・出典が不足しているか。
- `hasPolicyMismatch`: `Noul / boolean`。規程に反する意味的な不一致があるか。
- `hasPossibleDuplicate`: `Noul / boolean`。stateに渡した最大5件の`duplicateCandidates`と意味的に重複する可能性があり、別レコードとの確認が必要か。候補がない場合はこの質問を送らず、コード側で`duplicateCheck=not-run`として扱う。
- `severity`: `Score`。情報、要修正、要確認、処理停止。
- `needsHuman`: `Noul / boolean`。

金額合計、日付形式、必須列、重複ID、通貨換算はコードで検証する。重複の意味判定には、検索・絞り込み済みの最大5件の`duplicateCandidates`だけを渡し、候補がないのにJevへ推測させない。Jevには1つの`issueType`を選ばせず、独立したissue flagを評価させる。コード側でtrueになったflagを`issueTypes`の配列へ集約し、全flagがfalseのときだけ`none`とする。これにより、証拠不足・規程違反・重複候補が同時に存在する行を隠さない。

#### ローカルMVP

- CSVをアップロードし、schemaと簡単な規程を入力。
- 機械的検査を先に実行し、残った行だけJevで評価。
- 行別の判定、確率、問題種別、レビュー対象を一覧化。
- CSV/JSONレポートをダウンロード。

#### 評価

- 正常、欠損、意味不一致、重複候補、複数問題、曖昧な説明を含む100行以上。
- issue flagごとのprecision / recallと、複数flagを同時に検出できる割合を測る。
- 問題検出のprecision / recall、100行あたりのusage、p50/p95を測る。
- 同じレコードを再評価したときの結果・キャッシュ方針を確認する。

#### 差別化とリスク

大量データとコード側前処理を組み合わせるため、Jevの「低コストな意味フィルタ」という強みを測りやすい。金融・経費の最終承認や支払いは対象外にする。

## 6. 共通実装パターン

上位候補に共通する安全なデータフローは次のとおり。

```text
入力アダプター
  ↓
正規化・サイズ制限・PIIマスキング・機械的検査
  ↓
必要なstateだけを構成
  ↓
Choice / Score / Noul(boolean) を1リクエストで評価
  ↓
確率・confidenceをコードで合成
  ↓
自動処理 / 可逆処理 / 人手確認 / 停止
  ↓
判断・修正・responseMs・usageを監査ログへ保存
```

### 質問設計のチェックリスト

- 1問につき1つの判断になっているか。
- Choiceには`other`または`none`が必要か確認したか。
- Scoreの各レベルに境界を文章で書いたか。
- Noul / booleanの命題が肯定形で明確か。真偽確率とconfidenceを混同せず、`max(p, 1-p)`などの変換と閾値を決めたか。
- 複数の根拠・問題・安全シグナルを単一Choiceへ押し込めず、boundedなboolean群をコード側で集合へ戻しているか。
- 同じ意味をNoulとChoiceで重複評価し、確率の合計を期待していないか。
- 質問が参照するstateのパスを明示したか。
- 数値計算、日付比較、集計をコードへ戻したか。
- CIの合否、認可、候補の存在、上限値など決定的に取得できる事実をJevへ推測させていないか。
- state内のユーザー入力・文書・ログを開発者指示として扱っていないか。
- 欠損回答、低confidence、Gatewayエラーを人手確認または安全なfallbackへ送るか。

### 典型的なポリシー

| 状態 | read-only / 可逆 | 金銭・公開・破壊的操作 |
| --- | --- | --- |
| 高confidence | 自動実行候補 | 追加確認つき実行候補 |
| 中confidence | ユーザー確認または再取得 | 人手レビュー |
| 低confidence・欠損 | 人手・別モデルへ | 実行せず停止 |

閾値は仮の初期値としてコードに置き、ラベル付きデータで調整する。公式ドキュメントに例示される閾値を、そのまま別ドメインへコピーしない。

## 7. 候補の優先順位

5点満点で、現在のリポジトリへの適合性、Jevらしさ、ローカルでの検証可能性、安全性、公開実例との差別化を評価した。これは事業規模や市場売上の予測ではない。

| 候補 | リポジトリ適合 | Jevらしさ | 検証可能性 | 安全に試せる度 | 差別化余地 | 合計 | 判定 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Confidence-Gated Tool Router | 5 | 5 | 5 | 4 | 5 | 24 | **次の第一候補** |
| Citation / Response Verifier | 4 | 5 | 5 | 4 | 4 | 22 | **第二候補** |
| Bug / Support Triage Board | 5 | 4 | 5 | 4 | 3 | 21 | **最短実装候補** |
| CSV / Document Quality Auditor | 4 | 4 | 5 | 3 | 4 | 20 | データ処理候補 |
| PR / CI Review Gate | 4 | 4 | 4 | 4 | 3 | 19 | 開発者向け候補 |
| Browser Action Selector | 2 | 5 | 3 | 2 | 3 | 15 | 後回し |
| Trading / Finance Decision App | 2 | 4 | 2 | 1 | 3 | 12 | 対象外 |

### 選択ガイド

- **Jev固有の強みを見たい:** Confidence-Gated Agent Tool Router
- **LLMとの組み合わせを見たい:** Citation / Response Verifier
- **今あるアプリを最短で育てたい:** Bug / Support Triage Board
- **開発ワークフローへ広げたい:** PR / CI Review Gate
- **大量処理とコストを測りたい:** CSV / Document Quality Auditor

## 8. 評価・検証の実行計画

### 8.1 モデル評価とポリシー評価を分ける

公開実装で一貫して有効だったのは、Jevのライブ呼び出しをすべてのテストに埋め込まず、次を分離する構成。

1. **モデル評価:** ラベル付き入力に対するJevのChoice / Score / Noulの結果を記録する。
2. **ポリシー評価:** 保存済みの回答fixtureを入力し、閾値・認可・fallbackが正しく動くかを高速な単体テストで検証する。
3. **E2E smoke:** APIキーがあるときだけ少数の代表入力をライブ評価する。

### 8.2 最低限の指標

- Choice: accuracy、macro-F1、`other`のrecall
- Score: MAE、重大度のrecall、境界付近の誤差
- Noul / boolean: precision、recall、PR-AUCまたは閾値別の混同行列
- Confidence: confidence帯ごとのempirical accuracy、calibration curve
- 安全性: 危険操作のfalse negative、誤ブロック、human-review率
- 運用: `responseMs` p50/p95、usage、429率、fallback率

公式のworkflow evalは、Security Incidents、Agent Trace Observability、Invoice Processing、Customer Serviceを例に、複数の小さな質問とコード側の処理を評価している。自分の候補でも、モデル同士の大きな倍率を引用する前に、自分の入力・ラベル・閾値・ネットワーク条件を固定する。

## 9. 今回の調査から得た判断

### 価値が出る条件

1. 判定結果をすぐコードの分岐へ渡せる。
2. 選択肢・尺度・真偽を事前に定義できる。
3. 誤判定時に、可逆・確認・停止へ逃がせる。
4. 1件だけでなく大量・高頻度に判断する理由がある。
5. 最終的な正解ラベルまたは人の修正結果を集められる。

### 価値が薄くなる条件

1. 人が読む長文をそのまま生成してほしい。
2. 厳密な計算やデータベース制約だけで解ける。
3. 判定後の副作用を取り消せない。
4. 結果を検証するラベルや担当者がいない。
5. stateに大量の無関係情報や秘密情報が混ざる。

### 最終推奨

このリポジトリの次の1タスクは、**Confidence-Gated Agent Tool Router**を最有力とする。理由は、既存の問い合わせトリアージと同じ「型付き質問→結果表示→responseMs」の基盤を再利用しながら、Jevの特徴であるconfidence、許可候補、コード側ポリシー、人手確認を一つの画面で体験できるから。

安全性の説明と評価データを重視するなら、**Citation / Response Verifier**を選ぶ。最短で成果を出すなら、既存の**Bug / Support Triage Board**を拡張する。

## 10. 出典台帳

### 公式資料（証拠レベルA）

- [TypeSafe AI: Introducing System One Models & Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
- [TypeSafe AI: Introduction](https://docs.typesafe.ai/introduction)
- [TypeSafe AI: Primitives](https://docs.typesafe.ai/primitives)
- [TypeSafe AI: Patterns](https://docs.typesafe.ai/patterns)
- [TypeSafe AI: Confidence](https://docs.typesafe.ai/confidence)
- [TypeSafe AI: Confidence-Gated Routing](https://docs.typesafe.ai/patterns/confidence-routing)
- [TypeSafe AI: Composite Scoring](https://docs.typesafe.ai/patterns/composite-scoring)
- [TypeSafe AI: Intent Routing](https://docs.typesafe.ai/patterns/intent-routing)
- [TypeSafe AI: Example Use Cases](https://docs.typesafe.ai/concepts/use-case-map)
- [TypeSafe AI: Jev 1.13 Jaggedness](https://docs.typesafe.ai/model-jaggedness/jev-1.13)
- [TypeSafe AI: Workflow Evals](https://evals.typesafe.ai/)
- [TypeSafe AI: LLM guardrails cookbook](https://docs.typesafe.ai/cookbooks/llm_guardrails)
- [Vercel AI Gateway: Evaluation](https://vercel.com/docs/ai-gateway/modalities/evaluation)
- [Vercel AI Gateway: Jev model page](https://vercel.com/ai-gateway/models/jev)

### GitHub実装（証拠レベルB）

- [kraayenjon/awesome-jev](https://github.com/kraayenjon/awesome-jev)
- [anandi1989/awesome-jev-usecases](https://github.com/anandi1989/awesome-jev-usecases)
- [rajivkuriakose/typesafe-jev-examples](https://github.com/rajivkuriakose/typesafe-jev-examples)
- [browser-use/jev-ultrafast](https://github.com/browser-use/jev-ultrafast)
- [TypeSafeAI/typesafe-playground](https://github.com/TypeSafeAI/typesafe-playground)
- [devagrawal09/jev-review](https://github.com/devagrawal09/jev-review)
- [TypeSafeAI/clarity-judge](https://github.com/TypeSafeAI/clarity-judge)
- [realZachi/pg-jev](https://github.com/realZachi/pg-jev)
- [DevMortimer/pi-warden](https://github.com/DevMortimer/pi-warden)
- [jkudish/jev-mcp](https://github.com/jkudish/jev-mcp)
- [youkiti/tiab-review-plugin](https://github.com/youkiti/tiab-review-plugin)
- [reachjalil/jevlogs](https://github.com/reachjalil/jevlogs)
- [noelzappy/tripwire](https://github.com/noelzappy/tripwire)
- [teyhouse/jev-secret-detection](https://github.com/teyhouse/jev-secret-detection)
- [leepokai/jev-guard](https://github.com/leepokai/jev-guard)
- [AshutoshVJTI/ProgressGate](https://github.com/AshutoshVJTI/progressgate)
- [gargpratyush/jev-router](https://github.com/gargpratyush/jev-router)
- [tamaratran/fast-jev-compaction](https://github.com/tamaratran/fast-jev-compaction)
- [Dicklesworthstone/SkillRanker](https://github.com/Dicklesworthstone/skillranker)
- [sufianetaouil/every](https://github.com/sufianetaouil/every)
- [ellipsis-dev/blink](https://github.com/ellipsis-dev/blink)
- [superagents-lab/jev-search](https://github.com/superagents-lab/jev-search)
- [mattn/sqlite3-jev](https://github.com/mattn/sqlite3-jev)
- [AkashPriyadarshii/jev-curate](https://github.com/AkashPriyadarshii/jev-curate)
- [replynodes/jev-web-analyzer](https://github.com/replynodes/jev-web-analyzer)

### ブログ・実装記事（証拠レベルC）

- [AI Native: TypeSafe Jev徹底解説](https://www.ai-native.jp/blog/typesafe-jev-system-one-model-guide)
- [Qiita: 文章を生成しないAI Jevを日本語で検証](https://qiita.com/harupython/items/fbe98be6d136d5b9d14a)
- [TypeSafe Jevの日本語実測記事](https://tameshita.net/t/jev)
- [DEV: How to Use Jev](https://dev.to/valyuai/how-to-use-jev-a-practical-guide-to-typesafes-system-one-model-g5e)
- [DEV: Jev Does Not Replace the LLM](https://dev.to/miruky/jev-does-not-replace-the-llm-3n6)
- [DEV: Jev + Pi probability gate](https://dev.to/jomatsu/jev-pi-a-probability-gate-for-my-coding-agents-shell-commands-95d)
- [DEV: Log triage without alert storms](https://dev.to/reachjalil/how-we-tuned-typesafe-jev-for-log-triage-without-alert-storms-1ei0)

### SNS・コミュニティ（証拠レベルD）

- [Made with Jev](https://madewithjev.com/): X投稿を含む公開ビルドの集約。広告分析、投稿スコアリング、リード評価、メール分類、AI slop検出などを確認した。
- [Reddit: KillMyIdea](https://www.reddit.com/r/SideProject/comments/1wiw6tk/i_built_a_side_project_to_test_typesafes_jev/): 10問前後のアイデア評価、スコアの重み付け、Goalによる結果の変化、保存方針への議論を確認した。
- [Reddit: browser / realtime / local model discussions](https://www.reddit.com/r/SideProject/comments/1wk08l6/i_tried_jevstyle_typed_decisions_with_a_350m_browser/): typed outputの正しさとsemantic correctnessの差、ローカルモデルでの再現実験、リアルタイム用途への議論を確認した。

SNSに掲載された「何件を何秒・何ドルで処理した」という数値は、本文中では自己申告値としてのみ扱い、独立した性能保証や市場実績とはみなさない。
