# jevx APIキーあり実測レポート（2026-09-21）

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な評価スナップショット
> 再現方法: コマンドは記録時点の履歴。現行導入はユーザー向けガイドを参照。
> 注意: 数値と条件は記録時点の観測値であり、現行環境の保証ではない。

## 結論

同梱の固定40ケースを、`AI_GATEWAY_API_KEY` が設定された環境からJevへ送信して実測したよ。今回の1回のスナップショットでは、jevxは40/40件を成功処理し、正解率100%、エラー率0%、Jev応答 p95 673ms、全体 p95 675msだった。

Jevxの候補ミス率も0%だったため、このケースセットでは期待Skillがローカル候補に入らない問題は発生していない。local_keywordとの比較では、正解率が90%から100%へ、`none`精度が75%から100%へ改善した。

> このレポートは40ケース・1回の実測結果だよ。統計的な性能保証や、すべての利用者入力への精度保証を意味しない。再現性とレイテンシの分散は、複数回実測で別途確認する。

## 実測条件

| 項目 | 内容 |
| --- | --- |
| 実測日時 | 2026-09-21T03:33:08Z |
| 対象コミット | `221ba14b3dc075108a0eed4d43069a483bc86763` |
| 実行環境 | macOS / Darwin 25.6.0 arm64 |
| Rust / Cargo | rustc 1.98.1 / cargo 1.98.1 |
| Fixture | `jevx/evals/skill-selection.jsonl` 40件 |
| ケース内訳 | `synthetic` 30件、`anonymized-template` 10件 |
| 評価Skill | `jevx/evals/skills` の `SKILL.md` 9件 |
| APIキー | 設定済み。値は記録・表示しない |
| Gateway endpoint | `JEVX_GATEWAY_ENDPOINT` は未指定で既定値を使用 |
| timeout | 1,500ms |
| 最大候補数 | 32 |
| 判定閾値 | probability 0.60、margin 0.10 |
| Telemetry | `eval` コマンドが無効化 |

評価対象は、実際の利用者ログではなく、秘密情報を含まない合成ケースと匿名化形式のテンプレートケースだよ。期待ラベルが `none` のケースは全体で4件（synthetic 3件、anonymized-template 1件）だった。

## 全体結果

`none precision` は、既存Runnerの定義に合わせて「期待ラベルが `none` のケースを `none` と判定できた割合」としているよ。Jevxのレイテンシとusageは成功した40件の実測値から算出した。

| モード | 件数 | 正解率 | none precision | candidate miss rate | エラー率 | Jev p50 / p95 | total p50 / p95 | 平均 input / output |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| none | 40 | 10.00% | 100.00% | — | 0.00% | — | — | — |
| local_keyword | 40 | 90.00% | 75.00% | — | 0.00% | — | — | — |
| jevx（Jev実測） | 40 | **100.00%** | **100.00%** | **0.00%** | **0.00%** | **407 / 673ms** | **409 / 675ms** | **2401.175 / 127.675 tokens** |

### ケース種別ごとの結果

| ケース種別 | 件数 | 期待none | noneモード正解率 | local_keyword正解率 | jevx正解率 | jevx none precision | エラー率 | Jev p50 / p95 | total p50 / p95 | 平均 input / output |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| synthetic | 30 | 3 | 10.00% | 86.67% | **100.00%** | **100.00%** | 0.00% | 405 / 673ms | 407 / 675ms | 2400.967 / 127.7 |
| anonymized-template | 10 | 1 | 10.00% | 70.00% | **100.00%** | **100.00%** | 0.00% | 508 / 712ms | 510 / 714ms | 2401.8 / 127.6 |

## 品質ゲートの判定

| ゲート | 判定 | 根拠 |
| --- | --- | --- |
| ローカル探索 p95 <= 100ms | 未計測 | 現行の評価Runnerはローカル探索だけのp95を集計しない。今回の `total` はJev呼び出しを含む全体値。 |
| Jevを含む全体 p95 <= 2,000ms | **PASS** | `totalMsP95 = 675ms`。 |
| 低確信度の誤推薦を増やさない | **PASS（今回のケースセット）** | jevxの `none precision = 100%`。local_keywordの75%から改善。 |
| 成功結果に `jevResponseMs` が存在する | **PASS** | 成功40/40件で計測値あり。 |
| APIキー・Skill本文・生プロンプトをTelemetryへ出さない | **PASS** | `eval` 実行時にTelemetryを無効化し、出力にもprompt/keywords/レスポンス本文を保存していない。 |
| 新規Rust実行コードのラインカバレッジ >= 98% | **PASS** | 対象実行コードは前コミットの検証で99%以上。今回の変更はdocsのみ。 |

## 観測結果と解釈

1. **Jevの追加判断で正解率が10ポイント改善した。** `local_keyword` は36/40件、jevxは40/40件だった。候補ミス率が0%なので、今回の改善は候補探索ではなく、候補から最終選択するJev判定の効果として観測された。
2. **`none` の安全側判定が改善した。** local_keywordは期待none 4件中3件を正しく扱ったのに対し、jevxは4/4件だった。候補を無理に選ばず、低確信度を `none` に落とす設計がこのケースセットでは機能した。
3. **レイテンシはv1ゲート内だった。** Jev応答のp95は673ms、全体p95は675msで、全体のp95は2,000ms以下だった。今回の観測ではJev応答と全体の差は2msだったが、ネットワークやGatewayの状態で変動するため固定値とは扱わない。
4. **トークン使用量は候補説明を含む。** 平均はinput 2401.175、output 127.675 tokensだった。今後は候補数・Skill説明の圧縮・プロンプト形式を変えた比較を行うと、Codex CLI常用時のコスト効率を評価しやすい。

## 再現手順

APIキーの実値はシェル環境にだけ設定し、コマンドやGitへ書き込まないでね。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"

cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-live-eval-cases.jsonl \
  > /tmp/jevx-live-eval-summary.json
```

集計結果の最低限の確認は次のとおり。`--output` のJSONLはケースID・種別・判定・遅延・usageなどの安全なフィールドだけで、prompt・keywords・レスポンス本文を含まない。

```bash
jq -e \
  '.caseCount == 40 and .modes.jevx.status == "completed" and .modes.jevx.cases == 40' \
  /tmp/jevx-live-eval-summary.json

jq -s -e \
  '([.[] | select(has("prompt") or has("keywords"))] | length) == 0' \
  /tmp/jevx-live-eval-cases.jsonl
```

## 制約と次の測定

- 今回は単一実行なので、p50/p95の分散・失敗率の信頼区間・コールドスタート影響は評価していない。
- ケースは実ユーザーの会話ログではない。実データを保存せずに評価パイプラインを固定する目的のfixtureだよ。
- ローカル探索p95を判定するには、評価Runnerへ探索時間の個別計測を追加するか、別のベンチマークを用意する必要がある。
- 次は同じ40ケースを複数回実行し、Jev応答・全体時間・usageの分散を記録する。その後、Codex Hookのshadow実行とcompaction前後の評価へ進める。
