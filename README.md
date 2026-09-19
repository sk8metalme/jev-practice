# Jev Triage

問い合わせ文をJevに評価してもらい、カテゴリ・緊急度・返金要求を一度に確認する小さなWebアプリだよ。

## できること

- `choice`: 問い合わせを「請求・返金 / アカウント / 不具合 / 配送」に分類
- `score`: 対応の緊急度を「低 / 中 / 高」で評価
- `boolean`: 返金要求の可能性を確率で表示
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

## 参考

- [Vercel AI Gateway Evaluation](https://vercel.com/docs/ai-gateway/modalities/evaluation)
- [Jev API, Pricing & Playground](https://vercel.com/ai-gateway/models/jev)
- [Vercel AI SDKの評価モデル実装](https://github.com/vercel/ai/blob/main/packages/gateway/src/gateway-evaluation-model.ts)
- [Vercel AI Gateway経由でJevを利用する準備（Zenn）](https://zenn.dev/shinyaa31/articles/97581a58a3a76b)
