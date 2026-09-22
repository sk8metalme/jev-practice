# jevx 評価計画

> 対象読者: 開発者・保守担当
> 文書の状態: 現行仕様

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

このコマンドが現行fixtureに対する外部送信なしのベースラインの正本である。2026-09-22の確認では、`local_keyword` の正解率は0.875、`local_rank` は0.175、`none`は0.1、`jevx`は`status: not_run`だった。以下のAPIキーあり数値とJSON例は、記録済みスナップショットとして扱う。

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

APIキーありで同梱40ケースを実際に測定した結果は、[jevx APIキーあり実測レポート](../research/evaluations/jevx-live-evaluation-2026-09-21.md)に記録しているよ。そこでは、全体・ケース種別ごとの正解率、`none`精度、candidate miss、Jev/totalのp50・p95、平均token、品質ゲート判定を確認できる。

同じ条件で複数回測定する場合は`eval-repeat`を使う。run単位の品質分散と、全ケースの`discoveryMs`・`jevResponseMs`・`totalMs`・token分布をまとめて出力する。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat \
  --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-repeat.json
```

APIキーありで5回測定した結果は[複数回・APIキーあり実測レポート](../research/evaluations/jevx-variance-evaluation-2026-09-21.md)に記録している。Jevxは5回×40ケースで正解率平均98.0%、Jev p50/p95 427/610ms、total p50/p95 429/612ms、ローカル探索p95 2msだった。Codex Hookのshadow測定とcompaction相当fixtureの結果は[Codex Hook shadow / compaction評価](../research/evaluations/jevx-codex-hooks-evaluation.md)に分けている。

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
    "local_rank": {
      "status": "completed",
      "cases": 40,
      "accuracy": 0.175,
      "candidateMissRate": 0.0,
      "discoveryMsP50": 2,
      "discoveryMsP95": 3,
      "totalMsP50": 2,
      "totalMsP95": 2
    },
    "jevx": {
      "status": "completed",
      "accuracy": 0.95,
      "nonePrecision": 1.0,
      "candidateMissRate": 0.0,
      "errorRate": 0.05,
      "discoveryMsP50": 1,
      "discoveryMsP95": 2,
      "jevResponseMsP50": 429,
      "jevResponseMsP95": 643,
      "totalMsP50": 430,
      "totalMsP95": 644,
      "averageInputTokens": 2401.0,
      "averageOutputTokens": 127.7
    }
  }
}
```

上記は`eval` 1回分の構造例（2026-09-21のrun 1相当）だよ。5回分のrun-level分散を含む実測値は[複数回・APIキーあり実測レポート](../research/evaluations/jevx-variance-evaluation-2026-09-21.md)に分けている。別環境ではProvider・ネットワーク・モデルの状態で変動するため、同じマシン・同じ候補数・同じタイムアウトで複数回測定し、初回起動やコンパイル時間を日常利用の処理時間に混ぜない。`--output`へ出すJSONLにはケースID・種別・期待ラベル・ベースライン判定・Jev判定・遅延・usage・エラーコードが入り、prompt本文とfixtureの`keywords`は保存されない。

## 判定ゲート

v1のリリース判断では、次を同時に満たすことを目安にする。

- ローカル探索 p95 <= 100ms
- Jevを含む全体 p95 <= 2,000ms
- `none`を許容し、低確信度の誤推薦を増やさない
- `jevResponseMs`が全成功結果に存在する
- APIキー・Skill本文・生プロンプトがTelemetryへ出ない
- 新規Rust実行コードのラインカバレッジ >= 98%

この評価は、精度が良くても遅すぎる・情報を送りすぎる場合を合格にしないためのものだよ。
