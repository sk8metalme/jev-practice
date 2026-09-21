# jevx 評価計画

## 目的

Jevを使う価値を、Skill選択の精度・速度・token usage・安全性で確認する。単に「選ばれたか」だけでなく、Jevを使わない場合と比較して、日常のCodex CLIに追加するコストに見合うかを測る。

## 比較対象

1. **選択なし**: 常にSkillを選ばないベースライン
2. **ローカルキーワード**: `name`・`description`の一致だけでTop-1を決めるベースライン
3. **jevx Jev**: ローカル候補絞り込み + batched Choice + 確率閾値

`jevx/evals/skill-selection.jsonl`には、合成ケース30件と、秘密情報を含まない匿名化形式のケース10件を収録している。匿名化ケースは実際の利用者ログを再現したものではなく、実データを保存しない評価パイプラインを先に固定するためのテンプレートケースだよ。

## ケース形式

1行1ケースのJSONLで、次のフィールドを持つ。

| フィールド | 説明 |
| --- | --- |
| `id` | ケースID |
| `kind` | `synthetic` または `anonymized-template` |
| `prompt` | Skill選択の入力文。秘密情報を含めない |
| `expected` | 期待するSkill ID。不要なら`none` |
| `keywords` | ローカルベースライン用の語 |

ケース追加時は、期待ラベルの根拠をレビュー可能にし、実際のAPIキー・個人情報・リポジトリ固有の秘密を入れない。

## 指標

| 指標 | 定義 |
| --- | --- |
| Top-1 accuracy | `selected`のSkill IDが`expected`と一致する割合 |
| none precision | `expected: none`を`none`とした割合 |
| candidate miss rate | 期待SkillがJevへ渡す候補32件に入らなかった割合 |
| Jev p50/p95 | `metrics.jevResponseMs`の50/95パーセンタイル |
| total p50/p95 | `metrics.totalMs`の50/95パーセンタイル |
| input/output tokens | Gateway usageの分布と平均 |
| error rate | timeout/provider/errorの割合 |

## 実行手順

まずJSONLが壊れていないことを確認する。

```bash
jq -e -s 'length == 40 and all(.[]; .id and .prompt and .expected)' \
  jevx/evals/skill-selection.jsonl >/dev/null
```

評価Runnerでベースラインを先に再現する。APIキーなしで実行でき、Jevモードは意図的に`not_run`になる。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

Jevを含めた実測は`--dry-run`を外す。評価用Skillカタログは、通常のユーザーSkillと混ざらないよう`jevx/evals/skills`を明示する。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-eval-results.jsonl
```

標準出力のレポートは次のような構造になる。

```json
{
  "schemaVersion": 1,
  "caseCount": 40,
  "modes": {
    "none": {
      "status": "completed",
      "cases": 40,
      "accuracy": 0.1,
      "nonePrecision": 1.0
    },
    "local_keyword": {
      "status": "completed",
      "cases": 40,
      "accuracy": 0.9,
      "nonePrecision": 0.75
    },
    "jevx": {
      "status": "completed",
      "accuracy": 0.0,
      "nonePrecision": 0.0,
      "candidateMissRate": 0.0,
      "errorRate": 0.0,
      "jevResponseMsP50": 124,
      "jevResponseMsP95": 188,
      "totalMsP50": 141,
      "totalMsP95": 207,
      "averageInputTokens": 82.0,
      "averageOutputTokens": 12.0
    }
  }
}
```

上記のJev数値と正解率はレスポンス例であり、実測値ではない。実際の評価では、同じマシン・同じ候補数・同じタイムアウトで複数回測定し、初回起動やコンパイル時間を日常利用の処理時間に混ぜない。`--output`へ出すJSONLにはケースID・種別・期待ラベル・ベースライン判定・Jev判定・遅延・usage・エラーコードが入り、prompt本文とfixtureの`keywords`は保存されない。

## 判定ゲート

v1のリリース判断では、次を同時に満たすことを目安にする。

- ローカル探索 p95 <= 100ms
- Jevを含む全体 p95 <= 2,000ms
- `none`を許容し、低確信度の誤推薦を増やさない
- `jevResponseMs`が全成功結果に存在する
- APIキー・Skill本文・生プロンプトがTelemetryへ出ない
- 新規Rust実行コードのラインカバレッジ >= 98%

この評価は、精度が良くても遅すぎる・情報を送りすぎる場合を合格にしないためのものだよ。
