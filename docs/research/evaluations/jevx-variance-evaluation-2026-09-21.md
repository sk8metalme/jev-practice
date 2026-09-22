# jevx 複数回・APIキーあり実測レポート

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な評価スナップショット
> 再現方法: コマンドは記録時点の履歴。現行導入はユーザー向けガイドを参照。
> 注意: 数値と条件は記録時点の観測値であり、現行環境の保証ではない。現行値は[Latest評価baseline](../../developers/jevx-evaluation.md)から取得し、現行Hook手順は[canonical runbook](../../operators/jevx-compaction-operations.md)を参照する。

## 結論

同梱の40ケースを、同じSkillカタログと設定で5回繰り返し、合計200ケースをJevへ送信した。今回の環境では、jevxは正解率平均98.0%、`none` recall（互換JSONキー `nonePrecision`、`noneCorrect / expectedNone`）100%、candidate miss rate 0%、エラー率2.0%だった。成功したケースのJev応答時間はp50 427ms / p95 610ms、Skill探索を含む全体時間はp50 429ms / p95 612msだった。

ローカル候補探索はp50 2ms / p95 2msで、既存の品質ゲート `p95 <= 100ms` を満たした。Jevを追加する主なコストはローカル探索ではなく、Gatewayを経由するJevの応答時間と、その分散だと確認できたよ。

この結果は一つの5回バッチに対する実測であり、モデル・Gateway・ネットワークの長期SLOを保証するものではない。4件のProviderエラーも含めて、日常利用へ導入する前に定期測定が必要。

## 測定条件

| 項目 | 条件 |
| --- | --- |
| 実測日 | 2026-09-21 UTC |
| 実装コミット | `aa4a940` |
| ブランチ | `feat/jevx-benchmark-hooks-evaluation` |
| ホスト | macOS Darwin 25.6.0 / arm64 |
| Rust | `rustc 1.98.1` / `cargo 1.98.1` |
| Codex CLI | `codex-cli 0.155.1` |
| ケース | 40件（`synthetic` 30件 + `anonymized-template` 10件） |
| 評価用Skill | `jevx/evals/skills` の9件 |
| 反復回数 | 5回、合計200ケース |
| 候補数 | 最大32件 |
| Request timeout | 1,500ms |
| 選択閾値 | probability 0.60、margin 0.10 |
| Telemetry | 無効化（評価Runnerが常に無効化） |
| APIキー | `AI_GATEWAY_API_KEY` を実行環境へ設定。値は記録・表示していない |

評価対象コードはコミット済みの `aa4a940` と同一で、APIキーやGatewayの生レスポンスはリポジトリに保存していない。

## 再現コマンド

APIキーの値を表示しないまま設定確認をしてから、固定fixtureを5回測定する。

```bash
if [ -z "${AI_GATEWAY_API_KEY:-}" ]; then
  echo "AI_GATEWAY_API_KEY is not set" >&2
  exit 1
fi

cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat \
  --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-repeat-live.json \
  > /tmp/jevx-repeat-live.stdout.json
```

`--skill-dir`は既定のSkill rootを置き換えず、追加のrootとして扱われる。カタログを隔離して再現する場合は、[評価Runnerの隔離root手順](../../developers/jevx-evaluation.md#隔離したskill-rootでの実行)を使うこと。

`eval-repeat`は、各runのモード別サマリーと、`local_rank` / `jevx` が計測するケースを対象にしたrun横断の分布（`mean` / `stddev` / `min` / `max` / `p50` / `p95`）を出力する。`--output`はcase JSONLではなくrun/mode集計JSONで、ケースのprompt本文・fixtureの`keywords`・Jevレスポンス本文を含めない。

## 全体結果

割合の分布は5回分のrun-level値、時間とtokenの分布は各runの全ケースから収集した観測値を対象にする。Jevの時間とtokenはProviderエラーで値が得られなかった4件を除く196件で集計した。

| モード | Accuracy（平均 / min–max） | `nonePrecision`（expectedNone分母のrecall） | candidate miss | error rate | 探索 p50 / p95 | Jev p50 / p95 | total p50 / p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `none` | 10.0% / 10.0–10.0% | 100.0% | — | 0.0% | — | — | — |
| `local_keyword` | 90.0% / 90.0–90.0% | 75.0% | — | 0.0% | — | — | — |
| `local_rank` | 17.5% / 17.5–17.5% | 75.0% | 0.0% | 0.0% | 2 / 2ms | — | 2 / 2ms |
| `jevx` | 98.0% / 95.0–100.0% | 100.0% | 0.0% | 2.0% | 2 / 2ms | 427 / 610ms | 429 / 612ms |

`local_rank`は本番の候補ランキングをTop-1へ切り出した測定で、fixture用の単純なキーワードベースライン（`local_keyword`）とは目的が異なる。`local_rank`のAccuracyが低くてもcandidate missは0%なので、今回の失敗は「期待Skillが候補32件へ入らない」問題ではなく、ランキングまたはJev判断の改善対象として切り分けられる。

### jevx分布の詳細

| 指標 | 平均 | 標準偏差 | 最小 | 最大 | p50 | p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Accuracy（run単位） | 98.0% | 1.87pp | 95.0% | 100.0% | 97.5% | 100.0% |
| `none` recall（`nonePrecision`、run単位） | 100.0% | 0.00pp | 100.0% | 100.0% | 100.0% | 100.0% |
| candidate miss rate（run単位） | 0.0% | 0.00pp | 0.0% | 0.0% | 0.0% | 0.0% |
| error rate（run単位） | 2.0% | 1.87pp | 0.0% | 5.0% | 2.5% | 5.0% |
| `discoveryMs` | 1.58ms | 0.49ms | 1ms | 2ms | 2ms | 2ms |
| `jevResponseMs` | 442.7ms | 90.4ms | 301ms | 678ms | 427ms | 610ms |
| `totalMs` | 444.3ms | 90.4ms | 302ms | 680ms | 429ms | 612ms |
| input tokens | 2,401.1 | 2.94 | 2,393 | 2,407 | 2,401 | 2,406 |
| output tokens | 127.7 | 1.09 | 127 | 130 | 127 | 130 |

エラーは5回で合計4件（run 1: 2件、run 2: 1件、run 3: 1件、run 4–5: 0件）だった。エラーはAccuracyを下げる一方、エラーケースにはJev応答時間とusageがないため、時間・token統計は成功した196件を分母にしている。

### Run別結果

| Run | Accuracy | エラー | error rate | Jev p50 / p95 | total p50 / p95 | input / output 平均 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 95.0% | 2 | 5.0% | 429 / 643ms | 430 / 644ms | 2,401.0 / 127.7 |
| 2 | 97.5% | 1 | 2.5% | 406 / 643ms | 408 / 644ms | 2,401.2 / 127.7 |
| 3 | 97.5% | 1 | 2.5% | 466 / 609ms | 468 / 610ms | 2,401.2 / 127.7 |
| 4 | 100.0% | 0 | 0.0% | 403 / 605ms | 405 / 607ms | 2,401.2 / 127.7 |
| 5 | 100.0% | 0 | 0.0% | 404 / 582ms | 406 / 583ms | 2,401.2 / 127.7 |

### ローカル探索p95

`local_rank`はJev APIを呼ばず、Skill候補をスコアリングして最大32件へ絞る。本実測では5回×40件の200観測で次の結果になった。

| 指標 | 結果 |
| --- | ---: |
| 平均 | 1.90ms |
| 標準偏差 | 0.44ms |
| 最小–最大 | 1–3ms |
| p50 | 2ms |
| p95 | 2ms |
| 品質ゲート `p95 <= 100ms` | **合格** |

探索は全体p95 612msの約0.3%であり、今回の条件ではローカル探索の最適化よりJev応答の安定化を優先するのが合理的。

## 判定ゲート

| ゲート | 基準 | 実測 | 判定 |
| --- | ---: | ---: | --- |
| ローカル探索 | p95 <= 100ms | 2ms | **PASS** |
| Jev込み全体 | p95 <= 2,000ms | 612ms | **PASS** |
| candidate miss | 0%を目標 | 0% | **PASS** |
| `none` recall（`nonePrecision`） | 期待noneを取りこぼさない | 100% | **PASS** |
| 新規Rust行カバレッジ | >= 98% | 全体99.29% | **PASS** |
| eval-repeat出力にAPIキー・生prompt・Jevレスポンス本文を保存しない | 保存しない | run/mode集計値のみ | **PASS（eval runner出力のみ）** |

## 安全性の確認

- APIキーは環境変数から読み、標準出力・レポート・Gitへ書き込んでいない。
- 評価fixtureの`id` / `kind` / `expected` / `keywords`はrequest境界で扱いを分け、promptはredactしてJevへ送るが、保存レポートへ本文をコピーしない。
- `eval-repeat`レポートはcase JSONLではなくrun/mode集計だけを保存する。単回`eval`のcase outputに保存するのはid / kind / expected / decision / metrics / errorなどで、prompt / keywords / response本文は保存しない。
- 実測JSONは`/tmp`へ出力し、リポジトリへ追加していない。
- Telemetryは評価Runner内で無効化した。Telemetry schemaにはJevのprobabilityを保存しない。

ここでのPASSはeval runnerが生成した保存レポートの範囲だけを指す。通常の`skills suggest`や`UserPromptSubmit` Telemetryの安全性を実測した保証、またはHook recordのraw metadataに対するhardening完了を意味しない。

## 解釈と次の測定

今回の5回測定から、jevx導入の実用性について次を確認できる。

1. 同梱fixtureでは、jevxはローカルキーワードベースラインの90.0%を上回り、98.0%まで改善した。
2. `none` recall（`nonePrecision`）100.0%とcandidate miss 0.0%を維持しており、低確信度の推薦を抑えながら候補の入口も失っていない。
3. ただし、4件のProviderエラーと最大678msのJev応答があるため、Hookへ同期的に組み込む場合はタイムアウト時のshadow継続が必要。
4. fixtureは実ユーザーログではないため、次は秘密情報を除去した実利用に近い匿名化ケースを追加し、同じ5回以上の測定を行う。
5. CIでAPIキーあり測定を定期実行する場合は、シークレット管理・費用上限・レート制限・結果の匿名化を別途設計する。

## 関連資料

- [評価Runnerの仕様と指標](../../developers/jevx-evaluation.md)
- [Codex Hook shadow実測](jevx-codex-hooks-evaluation.md)
- [前回の単回APIキーあり実測](jevx-live-evaluation-2026-09-21.md)
- [評価fixtureの使い方](../../../jevx/evals/README.md)
