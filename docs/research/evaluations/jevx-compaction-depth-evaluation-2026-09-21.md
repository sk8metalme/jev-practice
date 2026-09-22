# jevx Codex compaction深掘り評価

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な評価スナップショット
> 再現方法: コマンドは記録時点の履歴。現行導入はユーザー向けガイドを参照。
> 注意: 数値と条件は記録時点の観測値であり、現行環境の保証ではない。

2026-09-21時点のCodex CLI 0.155.1を使い、使い捨て`CODEX_HOME`でHook発火を直接確認した記録だよ。あわせて、認証付きApp Serverで2モデル×3ケースのcompaction評価を実施し、ツール履歴・失敗復旧・interrupt復旧・compact後のcache/token usageまで測定方法・品質結果・限界を整理する。

この文書の実測では、認証情報・生の会話本文・prompt本文・デコイ本文をリポジトリへ保存していない。Hook記録のセッション・ターン・モデル識別子はSHA-256相関値だけを保存する。

## 結論

- 実インタラクティブCodex CLIで、入力→`/compact`→後続入力までを完了させ、`PreCompact(trigger=manual)`、`PostCompact(trigger=manual)`、`SessionStart(source=compact)`の各発火を確認した。
- Hook記録には`session_id`、`turn_id`、`model`の生値を残さず、`sessionIdSha256`、`turnIdSha256`、`modelSha256`、組み合わせた`correlationIdSha256`だけを残せた。
- CLIの相関集計は6 record、重複グループ0、重複record 0だった。
- 認証付きApp Serverでは`gpt-5.6-sol`と`gpt-5.6-terra`を各3ケース、各8ターンで測定した。全6/6ケースでcompactと後続turnが完了し、必須事実30/30保持、デコイ漏えい0、エラー0だった。
- 6ケースでツール履歴12件（失敗6件）、interrupted turn 6件、復旧turn 18件を記録し、復旧完了率は100%だった。失敗ツール出力は`turn/start.toolOutput`で注入した合成fixtureであり、実コマンド実行の結果ではない。
- compact時間は`gpt-5.6-sol`がp50 9,204ms / p95 10,037ms、`gpt-5.6-terra`がp50 17,275ms / p95 21,003msだった。これは今回の固定シナリオ・時点における観測値で、モデルの一般的な性能保証ではない。
- compact後usageのcache hit率は全体p50 94.22% / p95 98.41%、uncached inputはp50 397 / p95 1,509 tokens、`uncached input + output`の推定billable proxyはp50 577 / p95 1,677 tokensだった。これは請求額ではなく比較用のtoken proxyだよ。
- UserPromptSubmitのJev応答は2件で、`jevResponseMs`はp50 550ms / p95 889msだった。CLI compact自体の所要時間とJev Hookの応答時間は別指標として扱う。

## 測定条件

| 項目 | 値 |
| --- | --- |
| 実行日 | 2026-09-21 |
| Codex CLI | 0.155.1 |
| jevx | ローカルビルド |
| CLI sandbox | read-only |
| CLI approval | never |
| CLI profile | `/private/tmp`配下の使い捨て`CODEX_HOME` |
| Hook設定 | `SessionStart` / `PreCompact` / `PostCompact` / `UserPromptSubmit` |
| Hookコマンド | `jevx hooks shadow --output <profile>/hook-records.jsonl` |
| CLI入力 | `READY` → `/compact` → `AFTER`の実インタラクティブ操作 |
| 会話本文 | リポジトリ・評価レポートへ保存しない |

App Serverの品質評価は、認証済みの`gpt-5.6-sol`と`gpt-5.6-terra`を各3ケースで実行した。外部送信については、今回の実測再開にあたり明示的な承認を得ている。会話は秘密情報を含まない合成fixtureで、後続回答の生テキストは集計後に保存していない。App Serverの`thread/tokenUsage/updated`から、pre-compactionとpost-compactionのusage snapshotを採取した。

Hook trustの確認をこの一時検証だけで省略するため、起動時に`--dangerously-bypass-hook-trust`を付けた。常用設定へそのまま持ち込むフラグではない。

## 1. 実CLI `/compact` のHook直接検証

### 実行手順

使い捨てプロファイルにHookを設定し、通常の`~/.codex`とは分離してCodexを起動した。既存プロファイルや認証ファイルを変更しないことを優先した。

```bash
CODEX_HOME=/private/tmp/<disposable-codex-home> \
  /opt/homebrew/bin/codex \
  --dangerously-bypass-hook-trust \
  --sandbox read-only \
  --ask-for-approval never \
  --no-alt-screen \
  --cd "$PWD"
```

起動後に短い入力を送り、`/compact`を実行してから後続入力を送った。画面には`Context compacted · 2s`と表示され、compact後の会話継続まで完了した。Hook recordは次のように安全なフィールドだけを集計できる。

```bash
jevx/target/debug/jevx hooks correlate \
  --input /private/tmp/<disposable-codex-home>/hook-records.jsonl \
  --json
```

### 発火結果（成功compact）

| イベント | 件数 | 結果 |
| --- | ---: | --- |
| `SessionStart(source=startup)` | 1 | 発火、Jevは呼ばない |
| `UserPromptSubmit` | 2 | 発火、Jev呼び出し2件 |
| `PreCompact(trigger=manual)` | 1 | **実CLIから発火を確認** |
| `PostCompact(trigger=manual)` | 1 | **成功compact後の発火を確認** |
| `SessionStart(source=compact)` | 1 | **成功compact後の発火を確認** |

前回の起動直後`/compact`単独試行では、Codexのremote compact taskへの接続失敗により`PreCompact`だけが記録された。今回の再開測定では、後続入力を含む実会話状態でcompactが成功し、成功経路の3イベントを確認できた。失敗試行のrecordと成功試行のrecordは別の使い捨てprofileに分離している。

### 実際に保存された相関情報

`PreCompact` recordには次の項目が存在した。

```json
{
  "hookEventName": "PreCompact",
  "trigger": "manual",
  "sessionIdSha256": "<sha256>",
  "turnIdSha256": "<sha256>",
  "modelSha256": "<sha256>",
  "correlationIdSha256": "<sha256>",
  "elapsedMs": 0
}
```

生のセッションID、ターンID、モデルIDはrecordにもこの文書にも書かない。`correlationIdSha256`は、`session_id:turn_id`の組み合わせをSHA-256化した値だよ。turn IDが存在しない`SessionStart`では、session IDのハッシュを相関値として使う。

成功compactを含む今回の6 recordを`hooks correlate`へ渡した結果は次のとおり。

| 指標 | 結果 |
| --- | ---: |
| record count | 6 |
| duplicate group count | 0 |
| duplicate record count | 0 |
| event counts | `SessionStart=2`（startup/compact各1）、`UserPromptSubmit=2`、`PreCompact=1`、`PostCompact=1` |

成功compact後の`SessionStart(source=compact)`は、compact直後の後続入力を送る前に発火した。したがって、今回のHook経路では「compact完了」と「compact後セッション再開」を別recordとして観測できる。

### CLI経路のJev応答時間

成功compactを含む実行で記録されたUserPromptSubmitは2件だった。`jevResponseMs`は`[550, 889]ms`、`totalMs`は`[551, 889]ms`で、jevxのnearest-rank集計に合わせると次の値になる。

| 指標 | p50 | p95 |
| --- | ---: | ---: |
| Jev応答 (`jevResponseMs`) | 550ms | 889ms |
| Hook全体 (`totalMs`) | 551ms | 889ms |
| ローカル探索 (`discoveryMs`) | 0ms | 1ms |

これは2件の小標本であり、日常利用の代表値ではない。Jevをcompact後のHookで使う場合も、compact処理時間とHook追加遅延は分けて測定する。

今回の1成功compactだけでは、複数Hook設定、Codexの再試行、同一turnでの複数発火を一般化できない。重複原因を調べるには、成功した長いCLI会話を複数回採取し、相関グループのイベント順とtrigger/sourceを比較する必要がある。

## 2. Hook相関分析の実装

`jevx hooks shadow`はCodexの共通Hook入力から次を読み取る。

| 入力 | recordに保存する値 |
| --- | --- |
| `session_id` | `sessionIdSha256` |
| `turn_id` | `turnIdSha256` |
| `model` | `modelSha256` |
| `session_id` + `turn_id` | `correlationIdSha256` |

新しいサブコマンドは次の形式。

```bash
jevx/target/debug/jevx hooks correlate \
  --input /tmp/jevx-hooks.jsonl \
  --json \
  --output /tmp/jevx-hook-correlation.json
```

出力はイベント数、重複グループ数、重複レコード数、相関キー別の件数だけで、prompt本文やHook入力全体を再出力しない。重複判定は相関値だけでなく、イベント名、trigger、source、model hashもキーへ含めるため、同一セッション内の異なるライフサイクルを誤って1グループへまとめない。

## 3. 実会話に近いcompaction品質評価

### 評価手順

App Serverの`thread/start`、`turn/start`、`thread/compact/start`を使い、次の順序で測定する。各ケースは6ターン以上、または10,000文字以上を満たす。今回のresilience batchは自動compactとの競合を避けるため入力を短くし、8ターン条件を採用した。

1. 目的、受け入れ条件、制約、次アクションを含む会話を送る。
2. 必須事実と出力禁止のデコイを含め、会話ターン数を6以上にする。
3. `thread/compact/start`を実行し、`contextCompaction`の開始・完了イベントを観測する。
4. compact後に失敗ツール出力を1件注入し、再試行・interrupt・復旧・成功ツール出力を含む後続turnを実行する。
5. 最終回答で必須事実、復旧状態、デコイ漏えいを検査し、各turnのtoken usageからpost-compaction cache/token指標を算出する。
6. 生の後続回答を一時JSONLへ置き、`hooks conversation-eval`で集計したら一時ファイルを削除する。

公式App Server仕様では、compactの進行は`contextCompaction` itemの`item/started` / `item/completed`で観測できる。HookのPre/Post発火とは別のイベント経路なので、同じ指標へ混ぜない。

一時fixtureの例:

```json
{"caseId":"model-a-case-1","model":"gpt-5.6-sol","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal: keep-context\nnext: verify\nrecovery: completed","secretMarkers":["LOCAL_DECOY_ONLY"],"failureRecoveryRequired":true,"toolHistoryItems":2,"toolFailureCount":1,"interruptedTurns":1,"recoveryTurns":3,"recoveryCompleted":true,"preCompactionUsage":{"inputTokens":13372,"cachedInputTokens":12672,"outputTokens":5,"reasoningOutputTokens":0,"totalTokens":13377},"compactionUsage":{"inputTokens":13372,"cachedInputTokens":12672,"outputTokens":5,"reasoningOutputTokens":0,"totalTokens":13377},"postCompactionUsage":{"inputTokens":15922,"cachedInputTokens":15616,"outputTokens":121,"reasoningOutputTokens":73,"totalTokens":16043},"compactionCompleted":true,"compactionDurationMs":10037,"inputChars":867,"conversationTurns":8,"contextChars":2030,"observedEvents":["functionCallOutput","turn/interrupted","contextCompaction","turn/completed"]}
```

集計コマンド:

```bash
jevx/target/debug/jevx hooks conversation-eval \
  --input /tmp/jevx-conversation-cases.jsonl \
  --json \
  --output /tmp/jevx-conversation-report.json
```

評価レポートへ残るのは、モデル識別子、case ID、ターン数、文字数、必須事実の総数・保持数、デコイ漏えい件数、compact完了、遅延、エラーコードだけ。`followUpText`、必須事実本文、デコイ本文はレポートへ含めない。

### 再開後の2モデル×3ケース実測

認証付きApp Serverの`thread/start`、`turn/start`、`thread/compact/start`を使い、各ケースへ8ターンを送り、5つの必須事実をcompact後に確認した。各ケースは8ターン条件を満たし、`contextChars`は2,011〜2,030文字だった。短い入力でもCodex内部のシステムコンテキストを含むため、pre-compactionの`inputTokens`は13,164〜13,374だった。

各ケースの順序は「事実台帳→明示compact→失敗ツール出力→再試行→interrupt→復旧→成功ツール出力→最終復旧→後続回答」。ツール出力はApp Serverの`turn/start.toolOutput`へ合成値を渡したもので、shellやファイル操作を実行していない。

#### ケース別結果

| モデル | ケース | ターン | context文字数 | compact | 保持 | ツール履歴/失敗 | interrupt/復旧 | 漏えい | compact時間 | post cache hit | uncached / billable proxy | 後続turn |
| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| gpt-5.6-sol | aurora-compaction | 8 | 2,030 | yes | 5/5 | 2 / 1 | 1 / 3 | 0 | 10,037ms | 98.08% | 306 / 427 | yes |
| gpt-5.6-sol | bug-triage | 8 | 2,011 | yes | 5/5 | 2 / 1 | 1 / 3 | 0 | 5,814ms | 94.22% | 926 / 1,081 | yes |
| gpt-5.6-sol | release-plan | 8 | 2,020 | yes | 5/5 | 2 / 1 | 1 / 3 | 0 | 9,204ms | 98.41% | 252 / 465 | yes |
| gpt-5.6-terra | aurora-compaction | 8 | 2,030 | yes | 5/5 | 2 / 1 | 1 / 3 | 0 | 10,873ms | 93.50% | 1,050 / 1,184 | yes |
| gpt-5.6-terra | bug-triage | 8 | 2,011 | yes | 5/5 | 2 / 1 | 1 / 3 | 0 | 17,275ms | 90.92% | 1,509 / 1,677 | yes |
| gpt-5.6-terra | release-plan | 8 | 2,020 | yes | 5/5 | 2 / 1 | 1 / 3 | 0 | 21,003ms | 97.60% | 397 / 577 | yes |

#### token usageのケース別内訳

`pre`はcompact直前に観測した累積usage、`post`はcompact後の最終後続turnで観測したusage。`compactionUsage`はApp Serverがcompact完了時に独立したusage値を通知しない場合があるため、実測ではcompact直前の最新累積snapshotを保持した。`cacheWriteInputTokens`は全6ケースで0だった。比較に使うcache/token指標はpost snapshotから算出している。

| モデル | ケース | pre input / cached | post input / cached | post output / reasoning | post total | cache hit | uncached input | billable proxy |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| gpt-5.6-sol | aurora-compaction | 13,372 / 12,672 | 15,922 / 15,616 | 121 / 73 | 16,043 | 98.08% | 306 | 427 |
| gpt-5.6-sol | bug-triage | 13,374 / 6,016 | 16,030 / 15,104 | 155 / 102 | 16,185 | 94.22% | 926 | 1,081 |
| gpt-5.6-sol | release-plan | 13,368 / 13,184 | 15,868 / 15,616 | 213 / 163 | 16,081 | 98.41% | 252 | 465 |
| gpt-5.6-terra | aurora-compaction | 13,164 / 0 | 16,154 / 15,104 | 134 / 85 | 16,288 | 93.50% | 1,050 | 1,184 |
| gpt-5.6-terra | bug-triage | 13,374 / 9,984 | 16,613 / 15,104 | 168 / 115 | 16,781 | 90.92% | 1,509 | 1,677 |
| gpt-5.6-terra | release-plan | 13,368 / 9,984 | 16,525 / 16,128 | 180 / 130 | 16,705 | 97.60% | 397 | 577 |

#### モデル別集計

| モデル | ケース | compact完了 | 必須事実 | 復旧完了 | ツール履歴 / 失敗 | interrupt / 復旧turn | デコイ漏えい | compact時間 p50 / p95 | post cache hit p50 / p95 | uncached input p50 / p95 | billable proxy p50 / p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| gpt-5.6-sol | 3 | 3/3 | 15/15（100%） | 3/3 | 6 / 3 | 3 / 9 | 0 | 9,204ms / 10,037ms | 98.08% / 98.41% | 306 / 926 | 465 / 1,081 |
| gpt-5.6-terra | 3 | 3/3 | 15/15（100%） | 3/3 | 6 / 3 | 3 / 9 | 0 | 17,275ms / 21,003ms | 93.50% / 97.60% | 1,050 / 1,509 | 1,184 / 1,677 |
| **合計** | **6** | **6/6** | **30/30（100%）** | **6/6** | **12 / 6** | **6 / 18** | **0** | **—** | **94.22% / 98.41%** | **397 / 1,509** | **577 / 1,677** |

この結果は「今回の3固定ケース、8ターン、各モデル1回という条件で、compact後の要点保持・失敗復旧・interrupt復旧・cache/token usageを確認した」もの。JevがCodexのcompact処理自体を高速化したこと、長時間の実運用で同じ品質になること、他のモデル・プラン・会話形状でも同じことまでは証明しない。p50/p95はjevxのnearest-rank方式で、モデル別n=3のためp95は最大値になる。

`post cache hit`は`cachedInputTokens / inputTokens`、`uncached input`は`inputTokens - cachedInputTokens`、`billable proxy`は`uncached input + outputTokens`。OpenAIのusage説明ではcached tokensはinput tokensに含まれ、reasoning tokensはoutput tokensに含まれるため、ここではreasoningを別に足し戻していない。いずれも実請求額ではなく、同じ測定条件内の比較用指標だよ。

### 測定の合格条件

2モデル×3ケースの実測を完了と判定するには、全6ケースについて次を満たす。

- 各ケースが6ターン以上、または10,000文字以上。
- `contextCompaction`完了率をモデル別に算出できる。
- 必須事実保持率をモデル別に算出できる。
- デコイ漏えい件数をモデル別に算出できる。
- compact時間のp50/p95をモデル別に算出できる。
- 失敗ツール・interrupt・復旧turnを安全な件数で集計できる。
- post-compaction usageが取得できるケースではcache hit率、uncached input、billable proxyのp50/p95を算出できる。
- 失敗ケースは成功扱いへ補正せず、エラーコードと未到達イベントを記録する。

## 4. 外部API送信と今回の実測範囲

今回の再開では、認証付きCodex外部サービスへの送信について明示的な承認を得たうえで、使い捨てprofileから実測した。CLIは成功compact経路、App Serverは2モデル×3ケースのresilience経路をそれぞれ完了できた。

実測時に維持した制約は次のとおり。

- `CODEX_HOME`は毎回`mktemp -d`で作成する。
- 通常の`~/.codex`を変更しない。
- 会話には秘密情報ではなく、検証専用のデコイマーカーだけを使う。
- sandboxはread-only、approvalはneverにする。
- 生の会話、Hook record、rollout、認証ファイルは測定後に一時領域から削除する。
- リポジトリへ保存するのは安全な集計値だけにする。

追加で、App Serverへ送った会話は秘密情報を含まない合成fixtureに限定し、後続回答の生テキストは集計後に保存していない。

### まだ確認できていないこと

- App Server経路の`thread/compact/start`は`contextCompaction`を発火させるが、同じ使い捨てprofileのCodex CLI Hook recordには`PreCompact`、`PostCompact`、`SessionStart(source=compact)`を追加しなかった。App ServerイベントとCLI Hookイベントは別経路として扱う。
- CLIの成功compactは1回、UserPromptSubmitは2件なので、Hook重複やJev遅延の統計的な代表性はない。
- 実ツールを実行して得た長い出力ではなく、`turn/start.toolOutput`へ注入した合成ツール出力を評価した。shell・MCP・ファイル編集などの実ツール履歴品質は未評価である。
- 各ケースは8ターンだが、`contextChars`は約2,011〜2,030文字だった。10,000文字級の長文コンテキストで同じcache・復旧特性になるかは未評価である。
- ChatGPT認証のApp Server実測のため、公開APIのモデル単価を掛けたUSD請求額は算出していない。

## 参考

- [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)
- [Codex App Server公式ドキュメント](https://developers.openai.com/codex/app-server)
- [OpenAI Agents APIのusage・token observability](https://developers.openai.com/api/docs/guides/agents-api/observability)
- [既存の実Codex / compaction評価](jevx-real-codex-compaction-evaluation-2026-09-21.md)
- [jevx Hook shadow仕様](jevx-codex-hooks-evaluation.md)
