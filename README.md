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

## TypeSafe APIへ直接curlする例

このアプリの現在の実装はVercel AI Gateway経由だけど、TypeSafeのAPIへ直接リクエストしてJevを試すこともできるよ。直接経路では `TYPESAFE_API_KEY` を使い、送信先は `https://api.typesafe.ai/v1/systemone`、モデルは `jev-latest` だよ。

TypeSafeのwaitlistに登録すると、タイミングによっては1時間ほどでアカウント作成の案内が届くこともあるよ。ただし所要時間は保証されないので、案内が届いたら [TypeSafe Console](https://console.typesafe.ai/) でAPIキーを発行してね。

APIキーをシェルの履歴へ残さず設定するには、次を実行するよ。

```bash
printf "TypeSafe API key: "
read -r -s TYPESAFE_API_KEY
printf "\n"
export TYPESAFE_API_KEY
```

最小の直接リクエストはこれ。JSON文字列の中に未エスケープの改行を入れないよう、ヒアドキュメントで送っているよ。

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

直接APIの `noul` は、質問への肯定度を `0〜1` で返すよ。レスポンスの例や、現在のアプリと同じカテゴリ・緊急度・返金要求を評価するリクエストは [`docs/typesafe-api.md`](docs/typesafe-api.md) にまとめているよ。

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
- [TypeSafe API直接経路の使い方](docs/typesafe-api.md)

## jevx: Codex CLI向けSkillセレクタ

`jevx/`には、今の依頼に合いそうなCodex SkillをJevで提案するmacOS向けRust CLIを入れているよ。v1はShadow Modeなので、Skillの自動ロード・実行や会話の書き換えはしない。提案結果とJevの速度を確認してから、Codex側が通常のSkillルールに従って利用する設計だよ。

### ローカルで動作確認

Rust stableとCargoを用意して、まずテストと設定診断を実行するよ。

```bash
cargo test --locked --manifest-path jevx/Cargo.toml
cargo run --locked --manifest-path jevx/Cargo.toml -- doctor --json
```

Skillを一覧表示するには、プロジェクトのSkillディレクトリを指定できるよ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  skills list --skill-dir "$PWD/.agents/skills" --json
```

Jevを使った提案には、既存アプリと同じ `AI_GATEWAY_API_KEY` が必要だよ。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  skills suggest --prompt "PDFを結合して内容を確認したい" --json
```

Codex Skillとして使う場合は、リポジトリのルートで次を実行すると、jevxバイナリと advisory Skillをユーザー領域へセットアップできるよ。

```bash
sh jevx/scripts/setup.sh --scope user
```

プロジェクトだけに入れる場合は `sh jevx/scripts/setup.sh --scope project --repo "$PWD"` を使ってね。Skillの探索対象はプロジェクトの `.agents/skills` / `.codex/skills` と、ユーザーの `~/.agents/skills` / `$CODEX_HOME/skills` だよ。

### リクエストとレスポンス例

入力をJSONで渡す場合は、`prompt` と任意の `cwd` / `explicit_skill` を指定するよ。

```bash
printf '%s\n' '{"prompt":"テストを追加して失敗原因を調べたい","cwd":"/tmp/sample-repo"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      skills suggest --input-json --json --no-telemetry
```

レスポンスは、Skill提案の根拠と計測値を機械的に扱える安定したJSONだよ。`jevResponseMs` がJevへのリクエスト開始から構造化レスポンスを受け取るまでのアプリ側計測値で、`totalMs` はSkill探索を含む全体時間だよ。

```json
{
  "schemaVersion": 1,
  "decision": "selected",
  "selected": {
    "id": "testing",
    "name": "testing",
    "description": "Write and run tests",
    "path": "/Users/example/.agents/skills/testing/SKILL.md",
    "source": "user",
    "local_score": 25,
    "probability": 0.91
  },
  "candidates": [
    {
      "id": "testing",
      "name": "testing",
      "description": "Write and run tests",
      "path": "/Users/example/.agents/skills/testing/SKILL.md",
      "source": "user",
      "local_score": 25,
      "probability": 0.91
    }
  ],
  "metrics": {
    "discoveryMs": 7,
    "jevResponseMs": 124,
    "totalMs": 138,
    "candidateCount": 1,
    "inputTokens": 82,
    "outputTokens": 12
  },
  "mode": "shadow"
}
```

確信度が低い場合は安全側に倒して `decision: "none"` になるよ。APIキー未設定やタイムアウトは、JSONなら `error.code` と終了コードで明示する。人間向け出力でも次のようにJevの速度を表示するよ。

```text
jevx skill suggestion
Selected: testing
Probability: 0.91
Candidates: 3
Jev response: 124 ms
Total: 138 ms
Mode: shadow
```

### Telemetry

既定では `~/.jevx/events.jsonl` に、依頼文そのものではなくSHA-256・文字数・判定・候補数・Jev応答時間・usageだけを追記するよ。保存を止めるときは `--no-telemetry`、集計を見るときは `jevx stats --json` を使ってね。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- stats --json
```

設計と評価ケースは [`docs/jevx-requirements.md`](docs/jevx-requirements.md)、[`docs/jevx-architecture.md`](docs/jevx-architecture.md)、[`docs/jevx-evaluation.md`](docs/jevx-evaluation.md)、[`jevx/evals/README.md`](jevx/evals/README.md) にまとめているよ。

### 40ケース評価Runner

Jevの導入効果を、固定ケースでベースラインと比較できるよ。APIキーなしのdry-runでは、`none`・ローカルキーワード・Jevの3モードを比較し、Jevモードは`not_run`と表示する。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

Jevを実測するときはAPIキーを設定して`--dry-run`を外す。`--output`にはケースID・種別・期待ラベル・ベースライン判定・Jev判定・`jevResponseMs`・`totalMs`・usage・エラーコードをJSONLで保存し、prompt本文とfixtureの`keywords`は保存しない。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-eval-results.jsonl
```

dry-runの出力例:

```json
{
  "caseCount": 40,
  "modes": {
    "none": { "status": "completed", "accuracy": 0.1, "nonePrecision": 1.0 },
    "local_keyword": { "status": "completed", "accuracy": 0.9, "nonePrecision": 0.75 },
    "jevx": { "status": "not_run", "accuracy": null }
  }
}
```

### 複数回実測とCodex Hook shadow

APIキーありで同じ40ケースを複数回測定する場合は`eval-repeat`を使う。実測レポートにはrunごとの分散、Jev/totalのp50・p95、token、ローカル探索p95、エラー率を記録しているよ。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json --output /tmp/jevx-repeat.json
```

- [複数回・APIキーあり実測レポート](docs/jevx-variance-evaluation-2026-09-21.md)
- [Codex Hook shadow / compaction評価](docs/jevx-codex-hooks-evaluation.md)
- [実Codex Hook発火・実会話型compaction評価](docs/jevx-real-codex-compaction-evaluation-2026-09-21.md)
- [`/compact` Hook直接検証・compaction深掘り評価](docs/jevx-compaction-depth-evaluation-2026-09-21.md)
- [評価Runnerの仕様](docs/jevx-evaluation.md)

Hookを接続する前のshadow確認は、Codex相当のJSONをstdinへ渡して実行できる。stdoutは`{"continue":true,"suppressOutput":true}`だけを返し、会話を書き換えない。

```bash
printf '%s\n' '{"hook_event_name":"UserPromptSubmit","prompt":"PDFを結合したい","cwd":"/tmp/sample-repo"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow --event UserPromptSubmit \
      --skill-dir jevx/evals/skills --output /tmp/jevx-hooks.jsonl
```

実Codexのcompact後回答を、生の会話本文なしで評価する場合は一時JSONLを作り、`hooks conversation-eval`へ渡すよ。必須事実の保持率、デコイ漏えい、compact完了率、compact p50/p95に加えて、ツール履歴・失敗・interrupt後の復旧率と、compact後のtoken usage/cache利用率をまとめて表示できる。

`postCompactionCacheHitRate`は`cachedInputTokens / inputTokens`、`postCompactionUncachedInputTokens`は`inputTokens - cachedInputTokens`、`postCompactionEstimatedBillableTokens`は`uncached input + output`の比較用proxy。これは請求額ではなく、モデル単価・契約・実際の請求処理を含まない安全な相対指標だよ。

~~~bash
cargo run --locked --manifest-path jevx/Cargo.toml -- hooks conversation-eval --input /tmp/jevx-conversation-cases.jsonl --json --output /tmp/jevx-conversation-report.json
~~~

実Codex Hookの相関ID重複を調べるときは、recordへ保存したSHA-256値だけを集計できるよ。生のセッションID・ターンID・prompt本文は再表示しない。

~~~bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks correlate \
  --input /tmp/jevx-hooks.jsonl \
  --json \
  --output /tmp/jevx-hook-correlation.json
~~~
