# jevx費用観測契約

> 状態: 現行仕様。費用上限は設けず、速度・品質と同格の評価軸として記録する。

## 目的

速くなったように見える処理が、Jevの呼び出し、Codexの追加turn、model昇格、retry、fallbackによって高くなっていないかを同じ条件で比較する。費用を取得できないときは金額を推測して0にせず、`unknown`または`unavailable`を保持する。

## 型の意味

各 `CostEstimate` は次を持つ。

- `amount`: 金額。取得不能時は`null`。
- `currency`: ISO風の通貨ラベル。未設定なら`null`。
- `priceVersion`: Providerが返した価格・請求情報の版。未設定なら`null`。
- `status`: `available`、`unknown`、`unavailable`。
- `basis`: `estimated`（Token・Providerの推定値など）または`actual`（外部から渡された実績値）。

`cache hit`の追加呼び出しがないことをコード側で確定できる場合は、`available`かつ0円で記録できる。同様に、baselineのように外部モデルを呼んでいないことをコード側で確定できる観測は既知の0円として扱える。それ以外のusage欠落は0円にしない。JevとCodexの通貨または価格版が一致しない合算は`available`にしない。複数receiptの集計でも、unknown/unavailableや価格メタデータの不一致を既知の金額へ混ぜない。

## 記録項目

### Jev

`DecisionReceipt`、SkillのTelemetry、Review receiptに、calls、input/output Token、retry、fallback、cache hit、latency、price version、currency、Jev costを記録する。Review receiptは1回のbatched requestを基本とし、4カテゴリを候補ごとに個別呼び出ししない。

### Codex

OpenAI Responses API由来のusageを接続する場合は、input_tokens、output_tokens、total_tokensと、input_tokens_details.cached_tokens / cache_write_tokens、output_tokens_details.reasoning_tokensをcamelCase payloadへ正規化して受け取る。標準Codex Hookがusage/costを渡さない場合は、jevx側で推測せずunavailableのままにする。

TokenSavingsはbeforeTokens、afterTokens、savedTokens、reductionRate、source、status（measured / estimated / unavailable / degraded）を持つ。直接届いたHook payloadのbefore/afterは別リクエストの可能性があるため削減量を計算せず、compact-assistが未消費の同一cycleのPreCompact checkpointとPostCompactを対応付けられた場合だけ計算する。Measuredには`hook_checkpoint` sourceとsession/turn/correlationのcycle identityが必要で、turn IDが欠ける場合はUnavailableとする。Token削減量を請求額の削減とは解釈しない。

Hook payloadで任意に受け取る `codexUsage` は、model、reasoning、main turn、subagent数、input/output/total/reasoning Token、elapsed、fallback stage、costを保持する。Responses API形式のtop-level `usage` も受け付け、`input_tokens_details.cached_tokens` / `cache_write_tokens` と `output_tokens_details.reasoning_tokens` をそれぞれ `cachedInputTokens` / `cacheWriteInputTokens` / `reasoningTokens` へ正規化する。Responses `usage`の`input_tokens` / `output_tokens` / `total_tokens`は[公式API reference](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)に従う。Token usageはcached inputがinputを、reasoning outputがoutputを超えないこと、明示totalが既知componentsを下回らないこと、input/outputが両方明示されたときはtotalと厳密一致すること、加算がoverflowしないことを検証する。top-level `usage`にTokenはあるがcostがない場合はcostを`unknown`とする。model昇格やfallbackで追加された`additionalInputTokens`、`additionalOutputTokens`、`additionalReasoningTokens`、`additionalCost`も通常usageと分けて保持する。Skill選択・review・compact・planのどの観測に紐づくかは、対応するreceipt/eventの種類とtask/turn/session hashで区別する。欠落フィールドは欠落のままで、0に補完しない。`usageMeasuredRecords`はTokenフィールドが1つ以上あるrecordだけを数え、modelだけ・空objectは測定済みにしない。Token usage内の明示的な`totalTokens: 0`は欠落と区別して保持し、欠落totalは保存後も欠落のままにする。Responses usageが`total_tokens`だけを含む場合もusageありとして数えるが、costは`unknown`のままにする。

`compactionElapsedMs` は存在する場合に非負整数として検証し、`tokenSavings` はbefore/after/savedの算術一致とreductionRateの整合性を検証する。Hook recordのtotal costはJev/Codex componentsから再計算した値と照合し、不一致recordは拒否する。検証に失敗したrecordは集計へ入れず、暗黙に補正しない。

### 合算

すべての観測で次を同じ命名で扱う。

| 指標 | 定義 |
| --- | --- |
| `jevCost` | Jev componentの金額。全行が同一の価格版・通貨で既知のときだけ合算し、別途statusを数える |
| `codexCost` | Codex componentの金額 |
| `totalCost` | 通貨と価格版が一致したJev+Codex。basisは両方actualのときだけactual |
| task/turn/session | receiptのSHA-256 IDごとのcomponent合計。ID本文は保存しない |
| baseline delta | 同じfixture・環境・retry条件のbaselineとの差。絶対値と割合を別表示 |
| speed vs cost | p50/p95短縮率と追加/削減費用を同じ表に置く |
| fallback extra | 初回段階とfallback段階のcalls、追加Token（`additional*Tokens`）、latency、`additionalCost`の差 |
| successful review/fix | `Completed`/`None`のreview、`Applied`のfixに紐づくtotalCost。取得不能ならnull |

未知の行を集計から消さず、status count（例: `total:unknown`）に残す。平均値は同一通貨・価格版で対象行の費用がすべて比較可能な場合だけ出し、欠けた行や混在があれば`null`にする。

## 価格の出所

価格は利用者が追加設定する価格表ではなく、Jev／CodexのProvider usage/cost payloadから受け取る。実費がProviderから渡された場合は`actual`、Providerが返した推定値は`estimated`として、currencyとprice versionもpayloadの値を記録する。トークンだけで金額が渡されない経路は、勝手な単価を補わず`unknown`、usage自体がない経路は`unavailable`にする。

`doctor`の価格キーは公開JSON互換のため残しているが、`source=provider_usage`、単価は`null`、利用者が価格環境変数を追加設定する方式ではない。これにより、jevx独自の単価と実際のProvider請求額を混同しない。

## 保存先とschema

Hookの短期dedupe状態はJEVX_HOME直下のhook-dedupe.jsonに保存し、同時実行のclaimをhook-dedupe.lockで直列化する。生promptは保存せず、session/turn/model/event/trigger/source/promptに加えてcwd、候補Skill集合、判定設定のdigestをキーへ含め、正常な判定metadataだけを30秒保持する。完了済みcacheのTTLは30秒、評価中のpending claimのleaseは`JEVX_REQUEST_TIMEOUT_MS`の評価予算に1秒の安全余裕を加えた値で、長いJev評価中にcache TTLだけでownerを奪わない。cache hitはdecisionとselectedSkillだけを再利用し、元呼び出しのlatency/Tokenを重複計上しない。claim・待機・永続化の障害は`dedupeError`へ分離してHookの主エラーを隠さない。状態は専用の`.hook-dedupe-tmp/`へsync済み一時ファイルを書いてから原子的に置き換え、クラッシュ残骸も`data`の一覧・書き出し（metadataのみ）・削除対象へ含める。

compact-assistのcheckpoint追記と`data purge`は、`compaction/.compact-assist.lock`をdedupe lockとは別の安定lockとして使う。`data path/export/purge`はこのlockも管理対象として可視化し、削除対象があるpurgeだけdedupe→compactionの順で両lockを保持してから現在の管理対象を再列挙して消すが、安定lock自体は消さない。対象がないpurgeは既存compaction pathを検証するだけで、新しいdata directoryやlockを作らない。`$JEVX_HOME/compaction`またはstable lockがsymlink、あるいは実ディレクトリでない場合は、外部へlock作成・削除せずpurgeを拒否する。

hooks statsコマンドはHook件数、UserPromptSubmitのp50/p95、dedupe hitの待ち時間p50/p95、Token削減量、Jev/Codex/total費用の安全な合算、費用statusを集計する。eventCountsは全Hook eventを含むが、`latencyMsP50/P95`はUserPromptSubmitだけを対象にする。dedupe率の分母はkeyを持つUserPromptSubmitだけで、対象がなければnull。外部Jevを呼ばないHook eventのJev費用はコード側で確定した0円、Providerの実費がある場合はCodex/totalへ合算する。標準Hook payloadにusage/costがなければ、削減量・費用はnullまたはunavailableのままになる。

- `events.jsonl`: Skill選択Telemetry。`metrics.cost`にJev/Codex/totalとstatusを持つ。
- `decisions.jsonl`: Decision Contract receipt。従来schema v1を読み込み、現行v2で費用を記録する。opt-inしたCompaction判定も本文なしのtyped answer・usage・latency・cost・replay IDを記録する。
- `reviews.jsonl`: Review receipt（schema 2）。本文、Skill本文、Tool結果を含めず、digest・文字数・finding・route・fix・費用を記録する。`review-stats`には追加Token／fallback追加費用も出す。
- `hooks.jsonl` / `compaction/*`: Hook/Compactionのsafe metadataと任意のCodex usage/cost。
- `jevx hooks review-stats --input reviews.jsonl --json`: status、latency、calls/retry/cache、Token、追加Token、Jev/Codex/total、fallback追加費用、成功単価、task/turn/session hash単位の集計。

`eval` / `eval-repeat`のレポートschemaはbaseline比較の追加に伴いv2。`baselineMode`は既定で`local_rank`、`comparisons`の各modeに`totalMsDelta`（mode - baseline）、`speedupRate`（baselineからの短縮率）、`additionalCost`（mode - baseline）を出す。値が欠ける場合は`null`で、速度だけを成功扱いしない。Hook recordはschema v6、Compaction checkpointはschema v5、`jevx data path/export/purge`のinventoryはmanaged symlinkをno-followで扱う変更に伴いschema v7。Hook recordは旧schema v1〜v5を読み取り時に現行形式へ移行し、旧recordのtotal costはJev/Codex componentsから再計算する。checkpointはschema v1〜v4をv5へ移行し、旧snapshotのtotal未取得を0として復元せず、明示zero markerは保持する。未対応versionは拒否する。新規出力はv5に固定し、opt-in PreCompact receiptを示す`decisionReplayId`を保存する。data inventoryではmanaged fileや`.hook-dedupe-tmp` directory自体のsymlinkを一覧に残すが、リンク先を読まずrecord数は0とする。exportはsymlinkを開かずエラーにし、confirmed purgeはリンク自体だけを削除する。実directoryのtemp entryはno-follow FD相対で列挙・削除し、管理対象を実際に削除するpurgeでは空temp directoryも安定lock保持下でidentityを確認して片付ける。削除対象がないpurgeは新しいdirectory/lockを作らず、既存temp directoryにも触れない。Compaction purgeは保持中のdirectory FDから対象を削除し、lock取得後にdirectory identityが変わった場合は中止する。壊れたcheckpoint行は黙って読み飛ばさず、明示的なエラーとして扱う。receiptにprompt本文、会話全文、Skill本文、raw Tool result、API keyを保存しない。

## 評価手順

1. fixture hash、model、reasoning、retry、cache設定、価格版、通貨、環境を固定する。
2. baseline、Jevのみ、Codex昇格あり、review/fixありを同じ入力で複数回実行する。
3. p50/p95、品質、fallback/error、`unknown`/`unavailable`件数、Jev/Codex/totalを同じレポートへ出す。
4. 速度20%以上短縮でも、品質低下、secret漏えい、route未適用のapplied化、費用欠落があれば成功としない。

現行コードは観測と提案を実装している。Codex公式APIや実行wrapperから実費が渡されない経路ではCodex costは`unknown`/`unavailable`のままであり、値を作らない。これが解消されるまで、Jevxは「費用を最適化した」と主張しない。
