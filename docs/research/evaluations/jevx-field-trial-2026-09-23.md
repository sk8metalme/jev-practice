# jevx 3日間の実利用検証（2026-09-23〜2026-09-26）

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な評価スナップショット
> 再現方法: 集計スクリプトは記録時点の履歴。現行導入は[ユーザー向けガイド](../../users/getting-started.md)を参照。
> 注意: 数値と条件は記録時点・個人1人の利用環境での観測値であり、現行環境の保証ではない。本稿はHistorical snapshotで、未実装候補や現行ロードマップではない。現行Hook導入は[canonical runbook](../../operators/jevx-compaction-operations.md)を参照する。

## 概要

- 事前に固定した成功条件1〜5は**すべて合格**（UserPromptSubmit 66件で、サンプル数の条件5も満たした）。
- 可用性（errorRate 4.5%、timeoutRate 1.5%）と遅延（Hook経過時間 p95 934ms）は、入れ替え前のbaselineより改善した。
- 一方、compact復元28回のうち補助context付きは0回で、Compaction補助はbaselineと同じく実質的に使われていない。

## 目的と期間

最新のjevxを普段のCodex利用に入れ、3日間の実利用で「Hookが壊れずに速く動き、危険側へ倒れず、記録が残るか」を統計で確認した。正誤の評価は目的に含めない。

| 項目 | 値 |
| --- | --- |
| 検証期間 | 2026-09-23 22:40 〜 2026-09-26 22:40 JST（計測開始点の記録は 22:40:49） |
| 集計日時 | 2026-09-26 22:40:03 JST |
| 成功条件の固定日 | 2026-09-23（後から緩めていない） |
| baseline | 入れ替え前（旧バイナリ）の蓄積データを同じスクリプトで集計したもの |

## 環境

| 項目 | 値 |
| --- | --- |
| jevxバイナリ | main `a51f3fb` のローカルビルド（`jevx 0.1.0`） |
| OS | macOS 26.6.2 |
| Codex CLI | 0.157.1（集計時点の版。期間中に更新があったかは記録なし） |
| Hook | ユーザーの `hooks.json` に jevx のhandler 4件（doctorの値）。期間中は UserPromptSubmit・SessionStart・PreCompact・PostCompact の記録がある |
| Jev endpoint | Vercel AI Gateway（`/v1/evaluate`）。APIキーは環境変数で渡し、値は記録していない |
| 閾値（既定値、未カスタマイズ） | minProbability 0.6、minMargin 0.1、maxCandidates 32 |
| 制限（既定値、未カスタマイズ） | requestTimeoutMs 1,500、maxRetries 1、retryBackoffMs 25、maxStateBytes 32,000、decisionCacheCapacity 0 |

集計時の `jevx doctor --json` は launchd から実行したため `apiKeyConfigured: false` を返した。期間中のHook記録にはJev応答とreceiptがあり、`missing_api_key` エラーは0件なので、これは集計用プロセスの環境の違いであり、検証期間の設定状態を表すものではないと判断した。

## 方法

- **期間の区切り**: jevxのHook記録（`hooks.jsonl`、`compaction/*.jsonl`）には日時の列がない。そこで開始時に各ファイルの行数を記録し、それより後ろの行を検証期間、前の行をbaselineとして集計した。開始時の行数は `hooks.jsonl` 102行、`decisions.jsonl` 1行、`events.jsonl` 18行、`compaction/checkpoints.jsonl` と `compaction/hook-records.jsonl` 各118行。
- **統計のみ**: 正解ラベル（どのSkillを選ぶべきだったか）は集めていない。評価は件数・割合・遅延・fallbackの分布だけで行った。
- **prompt本文なし**: 元データにはprompt本文・Skill本文・Tool結果が存在しない（jevxはmetadataだけを記録する）。本稿も集計値だけを載せる。
- **パーセンタイル**: 値を昇順に並べ、`round(q × (n − 1))` 番目を取る最近傍法。
- **unsafeAccepted**: decision receiptのうち、`decision = accepted` なのに `errorCode` か `fallback` を持つものの件数。
- 集計スクリプトの詳細は[再現方法](#再現方法)を参照。

## 成功条件の判定

対象は UserPromptSubmit Hook と decision receipt の、検証期間のデータ。

| # | 条件 | 基準 | 実測 | baseline | 判定 |
| --- | --- | --- | --- | --- | --- |
| 1 | 可用性 | errorRate ≤ 5%、timeoutRate ≤ 3% | 4.5%（3/66）、1.5%（1/66） | 5.9%、3.9% | 合格 |
| 2 | 遅延 | Jev応答 p95 ≤ 1,200ms、Hook経過時間 p95 ≤ 1,500ms | 898ms、934ms | 1,021ms、1,396ms | 合格 |
| 3 | 安全性 | unsafeAccepted = 0件 | 0件 | 記録なし | 合格 |
| 4 | 記録 | receipt件数 / 判定したprompt数 ≥ 0.95 | 1.015（67/66） | 記録なし（receipt 1件 / prompt 102件） | 合格 |
| 5 | サンプル数 | UserPromptSubmit ≥ 30件 | 66件 | 102件 | 合格 |

補足:

- 条件1のエラー内訳は `provider_error` 2件、`timeout` 1件。baselineは `timeout` 4件、`provider_error` 1件、`missing_api_key` 1件。
- 条件3と4のbaselineは、baseline集計時のスクリプトがこの指標を出力していないため「記録なし」とした。旧バイナリではreceiptが1件しか残っていない。
- 条件4が1を超えたのは、receiptの集計対象に手動の `suggest`（期間中1件）も含まれ得るため。本集計ではreceiptを呼び出し元で区別していない。仮に1件を除いても66/66 = 1.0で基準を満たす。
- receiptの `accepted` 18件は、Hookの `selected` 17件と手動suggestの `selected` 1件の合計と一致する。

## 観察

合否にしない項目。改善候補を見つけるために記録する。

### 判定の分布

| 指標 | 検証期間 | baseline |
| --- | --- | --- |
| selected率 | 25.8%（17/66） | 30.4%（31/102） |
| none率 | 69.7%（46/66） | 63.7%（65/102） |
| 判定なし（エラー） | 3件 | 6件 |

receiptの内訳（検証期間67件）は `none` 30、`defer` 19、`accepted` 18。fallback理由は `none_selected` 30、`below_probability_threshold` 16、`provider_error` 2、`timeout` 1。Hookの `none` 46件は、Jevが候補なしと答えた30件と、確率が閾値0.6に届かず見送った16件に分かれる。つまり判定したprompt 66件のうち約24%（16件）は、Jevが何らかの候補を挙げたがコード側のgateで採用しなかったケースである。

### Skillの偏り

| 指標 | 検証期間 | baseline |
| --- | --- | --- |
| 選ばれたSkill | `jevx` 9、`orca-cli` 8 | `orca-cli` 24、`jevx` 5、`publish-github-pages` 2 |
| 最多Skillのshare | 52.9%（`jevx`） | 77.4%（`orca-cli`） |

baselineで目立った `orca-cli` への偏り（77%）は、検証期間では53%まで下がり、`jevx` と `orca-cli` がほぼ半々になった。ただし選ばれたSkillは2種類だけで、selected 17件という小標本のため、偏りが解消したとは言えない。作業内容の違い（期間中にどんな作業をしたか）は記録していないので、変化の原因は分からない。

### Compaction補助

| 指標 | 検証期間 | baseline |
| --- | --- | --- |
| PreCompact / PostCompact | 34 / 31 | 31 / 31 |
| SessionStart（compact / startup / clear / resume） | 28 / 9 / 1 / 0 | 31 / 21 / 2 / 2 |
| compact復元の回数 | 28 | 31 |
| うち補助context付き | 0（0%） | 0（0%） |
| Compaction Hook経過時間 p95 | 0ms | 0ms |

compact復元は3日間で28回あったが、補助contextが付いた復元は0回だった。jevxは作業ディレクトリに利用者が置いた `.jevx/compact-context.md` があるときだけ補助contextを返す仕様（[ユーザー向けガイド](../../users/getting-started.md)）なので、manifestを置いていなかった可能性が高いが、本集計ではmanifestの有無を確認していない。Compaction Hook経過時間の記録値は0msで、ミリ秒未満への丸めか未計測かはこの集計では区別できない。

### retry・cache・state・tokens

| 指標 | 検証期間 | baseline |
| --- | --- | --- |
| receipt件数 | 67 | 1 |
| retry回数（合計） | 0 | 0 |
| cache hit | 0 | 0 |
| degraded / omitted | 0 | 0 |
| receipt latency p50 / p95 | 652ms / 898ms | 790ms / 790ms |
| Jev応答 p50（Hook記録） | 659ms | 597ms |
| state bytes p95 | 14,585 | 4,052 |
| 候補数 p50 | 7 | 7 |
| input tokens 平均 | 2,473.3 | 2,266.0 |

- cache hitが0なのは、`decisionCacheCapacity` が既定値0（cache無効）のため。
- retryは `maxRetries` 1が許されていたが、0回だった。timeout 1件・provider_error 2件はretryなしでエラーになった。retryしなかった理由は本集計では分からない。
- state bytes p95は14,585で、上限32,000の半分以下。切り詰め（omitted）は0件だった。
- baselineのreceipt系の値は1件だけから計算しているため、比較には使えない。

### 手動suggest

期間中の手動 `jevx suggest` は1件（selected 1）。baselineは18件（selected 10、none 8）。

## 限界

- **正誤は分からない。** 正解ラベルがないため、selectedが正しかったか、noneが見落としだったかは評価していない。
- **小標本。** 利用者1人・3日間・UserPromptSubmit 66件。selectedは17件しかなく、割合の差は偶然の範囲に収まり得る。
- **期間を日時で区切れない。** Hook記録に日時がなく、行オフセットで区切った。開始前後で書き込み中の行があった場合の境界は確認していない。
- **baselineは同じ条件ではない。** baselineは旧バイナリでの期間不明の蓄積データで、作業内容も期間の長さも違う。receipt系の指標はほぼ記録されていない。
- **作業内容の記録がない。** prompt本文を持たないため、Skillの偏りやnone率の変化が、jevxの変更によるものか作業内容の違いによるものかは区別できない。
- **Codex CLIの版は集計時点のもの。** 期間中の更新有無は記録していない。

## 次の改善候補

データから言えることだけを挙げる。採用するかどうかは[ロードマップ](../../developers/jevx-roadmap.md)の基準で判断する。

1. **Compaction補助の案内を見直す。** compact復元28回で補助context付きが0回（baselineも31回中0回）。manifestを置かないと補助contextが返らないことが、導入後に利用者へ伝わっていない可能性がある。まず `.jevx/compact-context.md` がない状態で compact復元が起きたことを利用者が知る手段（例: `doctor` や集計での表示）を検討する。原因がmanifest未作成かどうかは、次回の検証でmanifestの有無も記録して確かめる。
2. **閾値未満のdeferを調べる。** 判定したpromptの約24%（16件）が `below_probability_threshold` で見送られた。閾値を変える前に、該当receiptのrecorded answerをreplayして、候補の確率とmarginの分布を確認する。
3. **Skillの偏りは継続観測する。** 最多Skillのshareは77%から53%に下がったが、小標本で原因も分からない。選ばれたSkillが2種類に集中しているため、次回の検証でも同じ指標を取り、偏りが続くならSkill説明文の見直しを検討する。
4. **次回の検証では期間を日時で区切れるようにする。** Hook記録に日時がないため行オフセットに頼った。集計の側で開始・終了の行数を両方記録するか、記録に日時を足すか（公開境界の変更になるためschemaVersionと移行が必要）を比べる。

## 再現方法

集計は検証用の作業状態ディレクトリ（以下 `<agent-state>`）に置いた `trial_report.py` で行った。スクリプトはリポジトリに含めていない。

1. 開始時に `<agent-state>/trial-start.env` へ、開始日時・バイナリのcommit・jevxデータディレクトリ（既定は `$JEVX_HOME`、未設定なら `~/.jevx`）内の各ファイルの行数を書く。

   ```text
   start=<開始日時>
   binary_commit=<commit>
   events.jsonl=<行数>
   hooks.jsonl=<行数>
   decisions.jsonl=<行数>
   compaction/checkpoints.jsonl=<行数>
   compaction/hook-records.jsonl=<行数>
   ```

2. 期間の終わりに集計する。

   ```bash
   python3 <agent-state>/trial_report.py > <agent-state>/final-report.json            # 開始後（検証期間）
   python3 <agent-state>/trial_report.py --baseline > <agent-state>/baseline-report.json  # 開始前（baseline）
   jevx doctor --json > <agent-state>/final-doctor.json
   ```

スクリプトは各JSONLを行オフセットで切り、UserPromptSubmit（`hooks.jsonl` の `hookEventName`）の `decision` / `errorCode` / `selectedSkill` / `jevResponseMs` / `elapsedMs`、receipt（`decisions.jsonl`）の `decision` / `fallback` / `retries` / `cacheHit` / `latencyMs` / `stateBytes` / `candidateCount` / `inputTokens` / `omitted`、Compaction checkpointの `hookEventName` / `source` / `contextAvailable` を数える。外部への送信はしない。
