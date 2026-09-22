# TypeSafe APIを直接使う

> 対象読者: 開発者・保守担当
> 文書の状態: 現行仕様

このリポジトリのWebアプリと `jevx` は、現在はVercel AI Gateway経由でJevを呼び出しているよ。このページでは、Vercelを経由せずTypeSafe APIへ直接リクエストする方法をまとめるね。

## 利用開始

1. [TypeSafeのwaitlist](https://typesafe.ai/) に登録する。
2. アカウント作成の案内が届いたら、[TypeSafe Console](https://console.typesafe.ai/) にログインする。
3. ConsoleでAPIキーを発行する。

waitlist登録後、タイミングによっては1時間ほどでアカウント作成の案内が届くこともあるよ。これは利用者の体験上の目安で、公式の所要時間保証ではないので、案内が届くまでの時間は状況により変わるよ。TypeSafe公式もearly accessの利用者をwaitlistから順次案内していると説明しているよ（[ローンチ記事](https://typesafe.ai/blog/introducing-system-one-models-and-jev)）。

APIキーはリポジトリへ保存せず、実行するシェルの環境変数へ設定してね。

```bash
printf "TypeSafe API key: "
read -r -s TYPESAFE_API_KEY
printf "\n"
export TYPESAFE_API_KEY
```

設定済みかだけ確認する場合は、キーの値を表示しないで次を実行するよ。

```bash
if [ -n "${TYPESAFE_API_KEY:-}" ]; then
  echo "TYPESAFE_API_KEY is set"
else
  echo "TYPESAFE_API_KEY is not set" >&2
fi
```

## エンドポイントと認証

直接APIの評価エンドポイントは次のとおりだよ。

```text
POST https://api.typesafe.ai/v1/systemone
Authorization: Bearer <API_KEY>
Content-Type: application/json
```

リクエストの必須フィールドは `state`、`model`、`questions`。TypeSafeの公式ドキュメントでは、公開モデルの指定に `jev-latest` を使っているよ。

## 最小のcurl

まず、yes/no型の質問である `noul` を1つ送る例だよ。フィールド間の改行はJSONの空白として有効だけど、文字列の途中に実際の改行やタブを入れるとJSONエラーになるため、問い合わせ本文を複数行にしたいときは `\n` としてエスケープしてね。

```bash
curl -sS -X POST "https://api.typesafe.ai/v1/systemone" \
  -H "Authorization: Bearer ${TYPESAFE_API_KEY}" \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "state": "Stripeの連携に3日間失敗していて、売上にも影響が出ています。",
  "model": "jev-latest",
  "questions": {
    "is_urgent": {
      "type": "noul",
      "instructions": "このメッセージは緊急性を表していますか？"
    }
  }
}
JSON
```

レスポンスは、たとえば次のような構造になるよ。数値は実行ごとに変わるよ。

```json
{
  "model": "jev-1.13.0",
  "answers": {
    "is_urgent": {
      "type": "noul",
      "noul": 0.94
    }
  },
  "usage": {
    "input_tokens": 316,
    "output_tokens": 23
  }
}
```

`answers.is_urgent.noul` は「緊急性がある」に近いほど1、ないほど0になる確率値だよ。

## 現在のアプリに近い3軸の例

現在のWebアプリが表示しているカテゴリ・緊急度・返金要求を、TypeSafe直APIの質問型へ置き換えると次のようになるよ。

```bash
curl -sS -X POST "https://api.typesafe.ai/v1/systemone" \
  -H "Authorization: Bearer ${TYPESAFE_API_KEY}" \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "state": "二重請求されました。返金をお願いしたいです。",
  "model": "jev-latest",
  "questions": {
    "category": {
      "type": "choice",
      "instructions": "問い合わせの主なカテゴリを1つ選んでください。",
      "criteria": {
        "billing": "請求、支払い、返金に関する内容",
        "account": "ログイン、アカウント、認証に関する内容",
        "technical": "アプリや機能の不具合に関する内容",
        "shipping": "配送、到着、注文状況に関する内容"
      }
    },
    "urgency": {
      "type": "score",
      "instructions": "問い合わせの対応の緊急度を判定してください。",
      "criteria": [
        "low: すぐの対応が不要",
        "medium: 早めの対応が望ましい",
        "high: 至急対応が必要"
      ]
    },
    "refundRequested": {
      "type": "noul",
      "instructions": "返金を希望または要求していますか？",
      "criteria": {
        "true": "返金を希望または要求している",
        "false": "返金を希望または要求していない"
      }
    }
  }
}
JSON
```

TypeSafe直APIでは、現在のVercel経由の `boolean` の代わりに `noul` を使うよ。返金要求の結果は `answers.refundRequested.noul` に入るため、アプリ実装で使うときは内部の表示モデルへ変換する必要があるよ。

## 質問型

| 型 | 使い道 | 主な回答フィールド |
| --- | --- | --- |
| `noul` | yes/no判定 | `noul`（0〜1） |
| `choice` | 定義した選択肢から1つ選ぶ | `choice`、`probabilities`、`confidence` |
| `score` | 定義した段階で評価する | `score`、`legend`、`probabilities`、`confidence` |

詳しい入力形式は[TypeSafe API Reference](https://docs.typesafe.ai/api)、導入手順は[Quick start](https://docs.typesafe.ai/introduction/quickstart)を参照してね。

## Vercel AI Gateway経由との違い

| 項目 | 現在の経路 | TypeSafe直接経路 |
| --- | --- | --- |
| エンドポイント | `https://ai-gateway.vercel.sh/v1/evaluate` | `https://api.typesafe.ai/v1/systemone` |
| APIキー | `AI_GATEWAY_API_KEY` | `TYPESAFE_API_KEY` |
| モデル指定 | `typesafe-ai/jev` | `jev-latest` |
| yes/no質問 | `boolean` | `noul` |
| yes/no回答 | `probability` | `noul` |
| usageのキー例 | `inputTokens` / `outputTokens` | `input_tokens` / `output_tokens` |

このページを追加した時点では、アプリの実装経路はまだVercelのままだよ。実装をTypeSafe直APIへ変更する作業は、Node.jsアプリと `jevx` の両方でリクエスト・レスポンス変換・設定・テストを更新する別タスクとして扱うよ。

## エラーと安全な扱い

- APIキーはブラウザ、READMEの実値、コミット、ログへ出さない。
- `401 Unauthorized` はAPIキーが未指定または無効。
- `422 Unprocessable Entity` はリクエストJSONや質問定義の不備。
- `429 Too Many Requests` と `529 Overloaded` は短時間の即時再試行を避け、指数バックオフで再試行する。
- `state` に個人情報や秘密情報を送る場合は、TypeSafeの利用規約・プライバシー方針と自分のデータ取り扱い要件を確認する。

## 参考

- [TypeSafe API Reference](https://docs.typesafe.ai/api)
- [TypeSafe Quick start](https://docs.typesafe.ai/introduction/quickstart)
- [TypeSafe Introduction](https://docs.typesafe.ai/introduction)
- [TypeSafe Console](https://console.typesafe.ai/)
- [Introducing System One Models & Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
