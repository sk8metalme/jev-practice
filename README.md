# Jev Triage

問い合わせ文をJevに評価してもらい、カテゴリ・緊急度・返金要求を一度に確認する小さなWebアプリだよ。

## できること

- `choice`: 問い合わせを「請求・返金 / アカウント / 不具合 / 配送」に分類
- `score`: 対応の緊急度を「低 / 中 / 高」で評価
- `boolean`: 返金要求の可能性を確率で表示
- Jevのレスポンス速度をミリ秒で表示
- 3つの質問を1回の評価リクエストで送信

Jevの評価モデルは自由文の返答ではなく、型付き質問に対する構造化された回答を返すため、分類やルーティングのような処理に向いているよ。

## 準備

- Node.js 20以上
- Vercel AI GatewayのAPIキー

Vercel AI GatewayでプロジェクトとAPIキーを準備して、実行するシェルの環境変数に設定するよ。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
```

キーを表示せず、設定済みかだけ確認するには次を実行するよ。

```bash
if [ -n "${AI_GATEWAY_API_KEY:-}" ]; then
  echo "AI_GATEWAY_API_KEY is set"
else
  echo "AI_GATEWAY_API_KEY is not set" >&2
fi
```

## 起動

追加パッケージなしで動くよ。

```bash
npm test
npm run dev
```

ブラウザで http://localhost:3000 を開いて、問い合わせ文を入力してね。`npm run start`でも起動できるよ。

APIキーが未設定でも画面は表示できるけど、判定ボタンを使うには `AI_GATEWAY_API_KEY` が必要だよ。

## curlでのリクエストとレスポンス例

ブラウザを使わず、ローカルサーバーの /evaluate を curl から呼び出すこともできるよ。先に npm run dev を起動して、別のターミナルで実行してね。

```bash
curl -i --request POST http://localhost:3000/evaluate \
  --header 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'state=二重請求されました。返金をお願いしたいです。'
```

このアプリの /evaluate は、Jevの結果を画面に表示するためのHTMLを返すよ。curl の出力全体は長いので、結果部分を抜粋すると次のようになるよ（数値は説明用で、実行ごとに変わるよ）。

```http
HTTP/1.1 200 OK
Content-Type: text/html; charset=utf-8

...
<span class="result-label">カテゴリ</span>
<strong>請求・返金</strong>
<small>確信度 91%</small>
<span class="result-label">緊急度</span>
<strong>高</strong>
<small>スコア 1.8</small>
<span class="result-label">返金要求</span>
<strong>83%の可能性</strong>
<p class="usage">42 input / 9 output</p>
<p class="response-time">Jev response 124 ms</p>
...
```

Jevは自由文の回答ではなく、1つの問い合わせに対して型付きの結果を返すよ。この例では、カテゴリを choice、緊急度を score、返金要求を boolean として評価しているよ。

Gatewayからアプリが受け取る構造化レスポンスのイメージは次のとおり。アプリはこの値をユーザー向けの日本語表示に変換しているよ。

```json
{
  "model": "typesafe-ai/jev",
  "answers": {
    "category": {
      "type": "choice",
      "choice": "billing",
      "probabilities": { "billing": 0.91 }
    },
    "urgency": {
      "type": "score",
      "score": 1.8
    },
    "refundRequested": {
      "type": "boolean",
      "probability": 0.83
    }
  },
  "usage": { "inputTokens": 42, "outputTokens": 9 }
}
```

| 表示 | Jevの回答 | この例で分かること |
| --- | --- | --- |
| 請求・返金 | choice: billing | 問い合わせの主なカテゴリ |
| 高 | score: 1.8 | 対応の緊急度 |
| 83%の可能性 | probability: 0.83 | 返金要求の可能性 |
| Jev response 124 ms | アプリ側の計測値 | Gatewayへ送信して構造化レスポンスを受け取るまでの時間 |

responseMs はJevプロバイダ内部だけの処理時間ではなく、GatewayへのHTTP往復とJSON応答の読み取りを含むアプリ側の経過時間だよ。

## 動作例

問い合わせ文を入力してJevで判定すると、カテゴリ・緊急度・返金要求の可能性・使用量をまとめて確認できるよ。

### 配送に関する問い合わせ

![配送に関する問い合わせの判定例](docs/screenshots/jev-triage-shipping.png)

### 請求・返金に関する問い合わせ

![請求・返金に関する問い合わせの判定例](docs/screenshots/jev-triage-billing.png)

### 身に覚えのない請求に関する問い合わせ

![身に覚えのない請求に関する問い合わせの判定例](docs/screenshots/jev-triage-invisible-charge.png)

## 実装の流れ

ブラウザからの入力はNode.jsサーバーが受け取り、サーバー側からVercel AI Gatewayの評価APIへ送信するよ。APIキーをブラウザへ渡さないのがポイント。

送信先は `https://ai-gateway.vercel.sh/v1/evaluate` で、次の形式のリクエストを組み立てているよ。

```json
{
  "model": "typesafe-ai/jev",
  "state": "問い合わせ本文",
  "questions": {
    "category": { "type": "choice", "criteria": {} },
    "urgency": { "type": "score", "criteria": [] },
    "refundRequested": { "type": "boolean", "criteria": {} }
  }
}
```

実際の質問文と判定基準は [`src/evaluation.js`](src/evaluation.js) にまとまっているよ。

結果画面の `Jev response xxx ms` は、Gatewayへリクエストを送ってからJevの構造化レスポンスを受け取り終わるまでの時間だよ。

## 参考

- [Vercel AI Gateway Evaluation](https://vercel.com/docs/ai-gateway/modalities/evaluation)
- [Jev API, Pricing & Playground](https://vercel.com/ai-gateway/models/jev)
- [Vercel AI SDKの評価モデル実装](https://github.com/vercel/ai/blob/main/packages/gateway/src/gateway-evaluation-model.ts)
- [Vercel AI Gateway経由でJevを利用する準備（Zenn）](https://zenn.dev/shinyaa31/articles/97581a58a3a76b)
