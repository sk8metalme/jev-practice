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

実環境の測定では、評価用Skillディレクトリを別途用意し、ケースごとに次のJSON入力を`jevx skills suggest --input-json --json`へ渡す。出力を保存するときも、プロンプト本文を結果ファイルやTelemetryへ再保存しない。

```bash
printf '%s\n' '{"prompt":"PDFを結合したい","cwd":"/tmp/jevx-eval"}' \
  | jevx skills suggest --input-json --json --no-telemetry
```

収集したJSONから`decision`、`selected.id`、`metrics`、`error.code`だけを抽出し、p50/p95は同じマシン・同じ候補数・同じタイムアウトで比較する。初回起動やコンパイル時間は日常利用の処理時間に混ぜない。

## 判定ゲート

v1のリリース判断では、次を同時に満たすことを目安にする。

- ローカル探索 p95 <= 100ms
- Jevを含む全体 p95 <= 2,000ms
- `none`を許容し、低確信度の誤推薦を増やさない
- `jevResponseMs`が全成功結果に存在する
- APIキー・Skill本文・生プロンプトがTelemetryへ出ない
- 新規Rust実行コードのラインカバレッジ >= 98%

この評価は、精度が良くても遅すぎる・情報を送りすぎる場合を合格にしないためのものだよ。
