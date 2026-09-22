# jevx evaluation fixtures

`skill-selection.jsonl`は、jevxのSkill選択を比較測定するための秘密情報なしのケース集だよ。

- `synthetic`: 30件の合成ケース
- `anonymized-template`: 10件の匿名化形式テンプレートケース

ケースは正解ラベルを含むので、Jevへ送信する入力からは`expected`と`keywords`を除外する。出力保存時もprompt本文を再保存せず、ID・判定・遅延・usageだけを残してね。

JSONLの検証:

```bash
jq -e -s 'length == 40 and all(.[]; .id and .prompt and .expected)' \
  jevx/evals/skill-selection.jsonl >/dev/null
```

## 評価Runner

リポジトリのルートから、APIキーなしでベースラインを再現できる。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

`--dry-run`では次の3モードを同じ40ケースで比較する。

- `none`: 常にSkillを選ばない
- `local_keyword`: Skillの`name`と`description`に対するローカル一致
- `jevx`: `status: not_run`。Jevのネットワーク呼び出しを行わない

実測するときは`--dry-run`を外し、APIキーと評価用Skillカタログを指定する。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-eval-results.jsonl
```

標準出力のレポートにはモード別の正解率、`none`精度、候補取りこぼし率、エラー率、Jev応答時間・全体時間のp50/p95、token平均が含まれる。`--output`のJSONLはケースID・種別・期待ラベル・ベースライン判定・Jev判定・遅延・usage・エラーコードを保存し、`prompt`と`keywords`は保存しない。

同一条件で分散を確認するには`eval-repeat`を使う。以下はAPIキーありで5回、合計200ケースを測定する例。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json --output /tmp/jevx-repeat.json
```

`eval-repeat`の集計はrun単位の品質（accuracy・error rateなど）と、全ケースの遅延・token分布（mean / stddev / min / max / p50 / p95）を分けて保持する。実測値は[`docs/research/evaluations/jevx-variance-evaluation-2026-09-21.md`](../../docs/research/evaluations/jevx-variance-evaluation-2026-09-21.md)を参照してね。
