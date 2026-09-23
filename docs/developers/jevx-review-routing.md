# jevx意味レビューとroute契約

## レビュー対象

`hooks review` は `prompt`、`plan`、`diff`、`final-answer`、`turn` のいずれかを対象にする。ローカルコードが対象候補、redaction、digest、budget、最終statusを所有し、Jevへは4カテゴリを1回のtyped requestで渡す。

| category | 問い | ローカル検出の例 |
| --- | --- | --- |
| `japanese_clarity` | 日本語として曖昧/わかりにくいか | `適宜`、`いい感じ`、`なるべく` |
| `text_contradiction` | 文章内の要求が矛盾していないか | 必須と不要の同居 |
| `code_contradiction` | コードの意味的契約が矛盾していないか | trueを要求してfalseを返す |
| `comment_implementation_drift` | コメントと実装が乖離していないか | returns trueコメントとfalse実装 |

ローカル検出はJevの結果と混同せず、Jev unavailableでも安全なfindingとして再現できる。外部送信なしの既定Hookでは本文を送らず、redact済みプロセス内検出だけを行い、statusは`degraded`で理由を残す。

## typed contractとstatus

4 predicate（threshold 0.80、margin 0.20）とroute scoreを `review-contract.v1` として送る。自由文、確信度、route推薦は権限・自動fixへ直接接続しない。

- `completed`: contractが解決し、findingがある。
- `none`: contractが解決し、findingがない。
- `degraded`: 本文opt-inなし、state budget超過、回答の一部欠落など。
- `failed`: timeout、provider error、契約不成立。
- `unknown`: 将来の不確定な入力状態を表す予約値。0件や成功へ変換しない。

receipt（schema 2）にはcontract version、request/content digest、文字数、task/session/turn SHA-256、status、finding count、route、latency、calls/retry/cache、通常Token、Codexの昇格追加Token／`additionalCost`、fallback、cost、fix status、replay IDを保存するが、本文は保存しない。`review-stats`はtask/turn/sessionごとの費用とfallback追加費用を集計し、unknown/unavailableや価格版・通貨の不一致を0へ変換しない。

## model/reasoning route

Jevのscoreをコード側で次へ変換する。

| difficulty | 推薦 | fallback |
| --- | --- | --- |
| low | Luna / max | Luna → Terra → Sol |
| medium | Terra / high | Terra → Sol |
| high | Sol / medium | Sol |

実際のCodex routeがpayloadのmodel/reasoningと完全一致したときだけ`applied`。証拠がない場合は`degraded`、不一致は`failed`。Hookがmodelを切り替えられないことを隠さない。

## fix契約

自動fixは既定無効。`--auto-fix --yes`、Completed review、全findingのconfidence 0.80以上、相対safe path、期待SHA-256一致のすべてを満たす場合だけ書く。絶対パス、`..`、`.git`、`legacy`、hidden/secrets（`.env`、`.pem`、`.key`など）、読み取り失敗、hash mismatch、timeout、state不足、契約不成立は書かない。backup/rollbackを作らないため、review-onlyを基本とする。

Jevなしに同じcode-side decisionをreplayできることが必須であり、CIやHookを最初からblockingにしない。まずshadow/warningで有用性・誤判定・費用・遅延を測る。
