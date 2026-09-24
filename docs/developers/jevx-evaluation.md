# jevx 評価計画

> 対象読者: 開発者・保守担当
> 文書の状態: 現行仕様・Latest baselineの正本

## 目的

Jevを使う価値を、Skill選択・意味レビュー・route・Compaction/計画の速度、品質、安全性、Jev費用、Codex費用で確認する。単に「選ばれたか」だけでなく、Jevを使わない場合と比較して、日常のCodex CLIに追加する総コストに見合うかを測る。

## 比較対象

1. **選択なし (`none`)**: 常にSkillを選ばないベースライン
2. **ローカルキーワード (`local_keyword`)**: `name`・`description`の単純な一致だけでTop-1を決めるベースライン
3. **ローカル順位 (`local_rank`)**: 現行ランキング実装で候補を順位付けしTop-1を返す、外部送信なしのベースライン
4. **jevx Jev (`jevx`)**: ローカル候補絞り込み + batched Choice + 確率閾値
5. **review/route**: 4カテゴリのlocal/typed finding + route evidence + fix gate

`jevx/evals/skill-selection.jsonl`には、合成ケース30件と、秘密情報を含まない匿名化形式のケース10件を収録している。匿名化ケースは実際の利用者ログを再現したものではなく、実データを保存しない評価パイプラインを先に固定するためのテンプレートケースだよ。

## ケース形式

1行1ケースのJSONLで、次のフィールドを持つ。

| フィールド | 説明 |
| --- | --- |
| `id` | ケースID |
| `kind` | `synthetic` または `anonymized-template` |
| `prompt` | Skill選択の入力文。秘密情報を含めない |
| `expected` | 期待するSkill ID。不要なら`none` |
| `keywords` | fixture作成者が残す注釈・期待根拠。現行 `local_keyword` の計算には使われない |

`keywords` はJSONLから読み込まれますが、現行実装の `local_keyword` はSkillのID・名前・説明から語を作り、fixtureの `keywords` を参照しません。したがってキーワードを編集しても現行local_keywordの予測は変わらず、注釈・レビュー用に保持されるだけです。ケースの `id` / `kind` / `expected` / `keywords` はJev requestへ含めず、redact後の`prompt`・redact後の`cwd`・候補Skill metadataだけを送ります。ケース追加時は、期待ラベルの根拠をレビュー可能にし、実際のAPIキー・個人情報・リポジトリ固有の秘密を入れない。

### Requestと保存レポートの境界

| データ | Jev request | `eval --output` のcase JSONL |
| --- | --- | --- |
| fixture `id` / `kind` / `expected` / `keywords` | 含めない | `id` / `kind` / `expected`、判定、metricsを保存 |
| `prompt` | redact後に送る | 保存しない |
| candidate Skill ID/name/description | 候補metadataとして送る（descriptionはredact） | ケース出力へは再掲しない |
| Jev response本文 | 構造化応答を判定に使う | 保存しない。判定・metricsだけ保存 |

## 指標

| 指標 | 定義 |
| --- | --- |
| Top-1 accuracy | `selected`のSkill IDが`expected`と一致する割合 |
| none recall（JSONキー: `noneRecall`、互換キー: `nonePrecision`） | `expected: none` のケースを `none` とした割合。`noneCorrect / expectedNone` で計算するため、通常の「none予測全体を分母にするprecision」とは異なる |
| candidate miss rate | 期待SkillがJevへ渡す候補32件に入らなかった割合 |
| Jev p50/p95 | `metrics.jevResponseMs`の50/95パーセンタイル |
| total p50/p95 | `metrics.totalMs`の50/95パーセンタイル |
| input/output tokens | Gateway usageの分布と平均 |
| error rate | timeout/provider/errorの割合 |
| fallback rate | `accepted`以外へ安全側に流れたreceiptの割合 |
| cache hit rate | 同一contract・policy・state digestでcacheを再利用した割合 |
| retry rate / average retries | retryが発生した実行の割合と実行あたりのretry平均 |
| relative cost | input/output tokenへ設定weightを掛けたproxy cost。価格そのものではない |
| `jevCost` / `codexCost` / `totalCost` | Provider usage/cost payloadから得たcomponent/合算。`unknown`/`unavailable`を0にしない |
| cost basis | `estimated`（Providerの推定値など）と`actual`（外部実績値）を分離 |
| task/turn/session cost | receiptのhash IDごとのJev/Codex/total合計。raw IDや本文は保存しない |
| successful review/fix cost | Completed/None review、Applied fixに紐づくtotalCost |
| speed vs cost | `baselineMode=local_rank`に対する`totalMsDelta`、`speedupRate`、`additionalCost`を同じrun条件で提示。`eval-repeat`は各値を分布化 |

Decision Contractの実行結果は`decisions.jsonl`へ安全なreceiptとして記録する。`DecisionReceipt`はcontract/question/policy version（policy閾値を含むdigest付き）、state digest、実際のstate bytes、適用したstate/candidate予算、候補window、omitted/redaction理由、typed answer digest、status、fallback、calls/retry、latency、usage、relative cost、replay IDを持つが、prompt本文・Skill本文・Tool結果・APIキーを持たない。`read_decision_receipts`で読み込み、`replay_receipt`でversion、state metadata、recorded answerとcode-side resultを検証する。不一致は成功へ変換せず`degraded`として扱う。`jevx/evals/decision-contract-baseline.jsonl`と対応するstate fixtureは実際に生成した値で、受け入れテストからreplayまで検証する。

## 実行手順

まずJSONLが壊れていないことを確認する。次のJSON検証には `jq`（macOSなら`brew install jq`）、カバレッジ検証には任意で `cargo-llvm-cov`（`cargo install cargo-llvm-cov`）が必要です。

```bash
jq -e -s 'length == 40 and all(.[]; .id and .prompt and .expected)' \
  jevx/evals/skill-selection.jsonl >/dev/null
shasum -a 256 jevx/evals/skill-selection.jsonl
```

### 隔離したskill-rootでの実行

評価Runnerでベースラインを先に再現する。APIキーなしで実行でき、Jevモードは意図的に`not_run`になる。

```bash
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --dry-run --json
```

このコマンドが現行fixtureに対する外部送信なしのベースラインの正本である。`eval --dry-run` は `none`、`local_keyword`、`local_rank`、`jevx` の4モードを出力し、Jevモードは `status: not_run` になる。2026-09-23の確認では、`local_keyword` の正解率は1.0、`local_rank` は0.2、`none`は0.1だった。`noneRecall`（互換キー `nonePrecision`）は `expectedNone` 分母の recall である。2026-09-23以降の `eval` / `eval-repeat` は `--skill-dir` のカタログだけを使い、project・HOME・CODEX_HOMEのSkill rootを混ぜない。2026-09-22のLatest（`local_keyword` 0.875、`local_rank` 0.175）は既定rootsを含む有効候補14件での環境依存の値で、現行コードでは再現できないためHistoricalとして扱う。

### 現行baselineの固定条件

| 項目 | 値 |
| --- | --- |
| 実行日時 | `2026-09-23T22:11:53+09:00` |
| 測定時の実装・fixture commit | `391e18049fe89a0dadcb8d43f6b70e3c6673ddf1` |
| fixture SHA-256 | `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb` |
| Rust / Cargo | `rustc 1.98.1 (48a229cea 2026-09-01)` / `cargo 1.98.1 (797e8a9bc 2026-08-05)` |
| OS | macOS 26.6.2（build 25G83） |
| 実行環境 | `JEVX_TELEMETRY=off`、APIキーなし、外部Gateway呼び出しなし |
| カタログ | `--skill-dir jevx/evals/skills` の fixture 9 Skills だけ（project/HOME/CODEX_HOMEのrootは使わない） |

このcommitは、現行Rust実装とfixtureでbaselineを測定した時点の記録です。後続のdocs変更だけでは実行コード・fixtureのbaseline値は変わらないため、docsのcommitと同一であることは要求しません。

`eval` / `eval-repeat` は `--skill-dir` だけをカタログにするため、作業ディレクトリやHOMEにあるSkillは評価に混ざらない。Telemetryやビルドキャッシュまで含めて環境を分けたいときは、次のように一時ディレクトリで実行する。

```bash
(
  set -eu
  repo="$PWD"
  rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
  cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  eval_root=$(mktemp -d -t jevx-eval-isolated.XXXXXX)
  mkdir -p "$eval_root/home" "$eval_root/codex" "$eval_root/project" "$eval_root/jevx"
  HOME="$eval_root/home" RUSTUP_HOME="$rustup_home" CARGO_HOME="$cargo_home" \
    CODEX_HOME="$eval_root/codex" JEVX_HOME="$eval_root/jevx" JEVX_TELEMETRY=off \
    cargo run --locked --manifest-path "$repo/jevx/Cargo.toml" -- \
      eval --cwd "$eval_root/project" \
      --fixtures "$repo/jevx/evals/skill-selection.jsonl" \
      --skill-dir "$repo/jevx/evals/skills" \
      --dry-run --json
)
```

上の隔離コマンドも、通常のコマンドと同じ値（`local_keyword` 1.0、`local_rank` 0.2）になる。

Jevを含めた実測は`--dry-run`を外す。評価用Skillカタログは、通常のユーザーSkillと混ざらないよう`jevx/evals/skills`を明示する。`--skill-dir` で渡したカタログだけが候補になる。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-eval-results.jsonl
```

APIキーありで同梱40ケースを実際に測定した結果は、[jevx APIキーあり実測レポート](../research/evaluations/jevx-live-evaluation-2026-09-21.md)に記録しているよ。これはHistorical snapshotであり、上のLatest baselineと混ぜない。そこでは、全体・ケース種別ごとの正解率、`none` recall（互換キー `nonePrecision`）、candidate miss、Jev/totalのp50・p95、平均token、品質ゲート判定を確認できる。

同じ条件で複数回測定する場合は`eval-repeat`を使う。run単位の品質分散と、`local_rank` / `jevx` が計測するケースの`discoveryMs`・`jevResponseMs`・`totalMs`・token分布をまとめて出力する。`eval-repeat --output`はrun/mode集計（runごとのmode summaryを含む）だけを保存するJSONで、case JSONLやprompt/response本文は出力しない。

反復runではprocess共有Decision cacheを無効化してrun間の回答再利用を防ぐ。単回のcache hitは外部Jevを呼んでいないため`jevResponseMs=0`、relative costも`0.0`として集計し、ローカル処理時間は`totalMs`だけに含める。`eval`のreport schema 2は`baselineMode=local_rank`と`comparisons`を持ち、速度差分と追加費用を同じrun条件で出す。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat \
  --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-repeat.json
```

APIキーありで5回測定した結果は[複数回・APIキーあり実測レポート](../research/evaluations/jevx-variance-evaluation-2026-09-21.md)に記録している。jevxは5回×40ケースで正解率平均98.0%、Jev p50/p95 427/610ms、total p50/p95 429/612ms、ローカル探索p95 2msだった。Codex Hookのshadow測定とcompaction相当fixtureの結果は[Codex Hook shadow / compaction評価](../research/evaluations/jevx-codex-hooks-evaluation.md)に分けている。

標準出力のレポートは次のような構造になる。以下の `jevx` 数値は2026-09-21のHistorical exampleであり、現行dry-runの値ではない。現行実装ではreport schema 2として`baselineMode`と`comparisons`を持ち、各mode summaryへfallback/cache/retry/relative cost/費用statusの集計も含まれる。

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

上記は`eval` 1回分の構造例（2026-09-21のrun 1相当）だよ。`nonePrecision` は `expectedNone` 分母の recall。5回分のrun-level分散を含む実測値は[複数回・APIキーあり実測レポート](../research/evaluations/jevx-variance-evaluation-2026-09-21.md)に分けている。別環境ではProvider・ネットワーク・モデルの状態で変動するため、同じマシン・同じ候補数・同じタイムアウトで複数回測定し、初回起動やコンパイル時間を日常利用の処理時間に混ぜない。単回`eval --output`のcase JSONLには`id`・`kind`・`expected`・判定・metrics・error codeが入り、prompt本文・fixtureの`keywords`・Jev response本文は保存されない。`eval-repeat --output`はcase JSONLではなくrun/mode集計だけである。

実際の`eval --json`標準出力には、このsummaryに加えて各fixtureの詳細を持つ`cases`配列（`id`・`kind`・`expected`・各modeの判定とmetrics）が含まれる。上の例では配列の長さを省略している。`eval-repeat --output`はcase JSONLではなくrun/mode集計だけである。

## 判定ゲート

v1のリリース判断では、次を同時に満たすことを目安にする。

- ローカル探索 p95 <= 100ms
- Jevを含む全体 p95 <= 2,000ms
- `none`を許容し、低確信度の誤推薦を増やさない
- `jevResponseMs`が全成功結果に存在する
- APIキー・Skill本文・生プロンプト・probabilityがTelemetryへ出ない
- state byte/window超過、timeout、provider error、低確信度が`accepted`や自動allowへ変換されない
- recorded answerをJevなしで同じcode-side decisionへ再現できる
- 速度20%以上短縮、品質改善、費用変化を同じレポートで説明できる。速度だけでは合格にしない
- 4カテゴリのreview、route applied/degraded/failed、fix hash gate、推定/実費を確認できる
- 新規Rust実行コードのラインカバレッジ >= 98%

この評価は、精度が良くても遅すぎる・情報を送りすぎる場合を合格にしないためのものだよ。
