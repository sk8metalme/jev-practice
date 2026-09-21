# jevx Codex compaction深掘り評価

2026-09-21時点のCodex CLI 0.155.1を使い、使い捨て`CODEX_HOME`でHook発火を直接確認した記録だよ。あわせて、実会話に近いcompaction評価の測定方法と、現時点で確認できた品質ベースラインを整理する。

この文書の実測では、認証情報・生の会話本文・prompt本文・デコイ本文をリポジトリへ保存していない。Hook記録のセッション・ターン・モデル識別子はSHA-256相関値だけを保存する。

## 結論

- 実インタラクティブCodex CLIから、起動直後の`/compact`で`PreCompact(trigger=manual)`が発火した。
- Hook記録には`session_id`、`turn_id`、`model`の生値を残さず、`sessionIdSha256`、`turnIdSha256`、`modelSha256`、組み合わせた`correlationIdSha256`だけを残せた。
- 今回はcompact本体のremote taskがDNS/接続エラーになったため、`PostCompact`と`SessionStart(source=compact)`は未到達だった。したがって、CLIのcompact成功を証明する実測ではない。
- 既存のApp Server実測では、gpt-5.6-solの3ケースでcompact完了3/3、必須事実保持15/15、デコイ漏えい0件を確認済み。ただしこれは今回追加する2モデル×3ケースの完了結果ではない。
- 複数モデル・長時間会話の追加実測は、Codex外部サービスへ実会話を送るため、明示的な実行承認後に行う。

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
| 入力 | 起動直後の`/compact` |
| 会話本文 | リポジトリ・評価レポートへ保存しない |

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

起動後に`/compact`を入力した。Hook recordは次のように安全なフィールドだけを集計できる。

```bash
jevx/target/debug/jevx hooks correlate \
  --input /private/tmp/<disposable-codex-home>/hook-records.jsonl \
  --json
```

### 発火結果

| イベント | 件数 | 結果 |
| --- | ---: | --- |
| `SessionStart(source=startup)` | 1 | 発火、Jevは呼ばない |
| `UserPromptSubmit` | 1 | 発火、remote provider errorを安全な`errorCode`へ記録 |
| `PreCompact(trigger=manual)` | 1 | **実CLIから発火を確認** |
| `PostCompact(trigger=manual)` | 0 | compact remote taskの接続失敗で未到達 |
| `SessionStart(source=compact)` | 0 | compact未完了のため未到達 |

compact実行時の画面上の失敗は、ホスト名解決またはリクエスト送信失敗だった。Hookプロセス自体は`PreCompact`で終了成功し、安全なJSONL recordを追加できた。今回の失敗はHookコードの例外ではなく、Codexのremote compact taskへ接続できなかったことによる。

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

今回の3 recordを`hooks correlate`へ渡した結果は次のとおり。

| 指標 | 結果 |
| --- | ---: |
| record count | 3 |
| duplicate group count | 0 |
| duplicate record count | 0 |
| event counts | `SessionStart=1`, `UserPromptSubmit=1`, `PreCompact=1` |

この3件だけでは重複原因を推論できない。複数Hook設定、Codexの再試行、同一turnでの複数発火を区別するには、成功した長いCLI会話を複数回採取し、相関グループのイベント順とtrigger/sourceを比較する必要がある。

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

App Serverの`thread/start`、`turn/start`、`thread/compact/start`を使い、次の順序で測定する。

1. 目的、受け入れ条件、制約、次アクションを含む会話を6ターン以上、または10,000文字以上送る。
2. 必須事実を複数ターンへ分散させ、ノイズと出力禁止のデコイを混ぜる。
3. `thread/compact/start`を実行し、`contextCompaction`の開始・完了イベントを観測する。
4. compact後の後続ターンで必須事実を列挙させ、デコイが出ていないか検査する。
5. 生の後続回答を一時JSONLへ置き、`hooks conversation-eval`で集計したら一時ファイルを削除する。

公式App Server仕様では、compactの進行は`contextCompaction` itemの`item/started` / `item/completed`で観測できる。HookのPre/Post発火とは別のイベント経路なので、同じ指標へ混ぜない。

一時fixtureの例:

```json
{"caseId":"model-a-case-1","model":"gpt-5.6-sol","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal: keep-context\nnext: verify\ndecoy_marker: redacted","secretMarkers":["LOCAL_DECOY_ONLY"],"compactionCompleted":true,"compactionDurationMs":6200,"inputChars":12400,"conversationTurns":8,"contextChars":12400,"observedEvents":["contextCompaction","turn/completed"]}
```

集計コマンド:

```bash
jevx/target/debug/jevx hooks conversation-eval \
  --input /tmp/jevx-conversation-cases.jsonl \
  --json \
  --output /tmp/jevx-conversation-report.json
```

評価レポートへ残るのは、モデル識別子、case ID、ターン数、文字数、必須事実の総数・保持数、デコイ漏えい件数、compact完了、遅延、エラーコードだけ。`followUpText`、必須事実本文、デコイ本文はレポートへ含めない。

### 既存ベースライン

前回のApp Server実測では、gpt-5.6-solで3ケースを実行した。

| 指標 | ベースライン |
| --- | ---: |
| モデル | gpt-5.6-sol |
| ケース | 3 |
| compact完了 | 3/3 |
| 必須事実保持 | 15/15（100%） |
| デコイ漏えい | 0 |
| compact時間 p50 / p95 | 6,641ms / 6,856ms |

これは「この3ケース・このモデル・この時点で要点保持を確認した」結果であり、JevがCodexのcompactを高速化した証明ではない。また、2モデル比較や長時間会話の分散を表すものでもない。

### 追加実測の合格条件

2モデル×3ケースの実測を完了と判定するには、全6ケースについて次を満たす。

- 各ケースが6ターン以上、または10,000文字以上。
- `contextCompaction`完了率をモデル別に算出できる。
- 必須事実保持率をモデル別に算出できる。
- デコイ漏えい件数をモデル別に算出できる。
- compact時間のp50/p95をモデル別に算出できる。
- 失敗ケースは成功扱いへ補正せず、エラーコードと未到達イベントを記録する。

## 4. 外部API送信が必要な未完了作業

実インタラクティブCLIのcompact成功と、2モデル×3ケースのApp Server評価には、Codex外部サービスへのネットワーク接続が必要になる。現在の実行環境ではDNS/接続が拒否され、ネットワーク許可付き再試行は会話・リポジトリコンテキストを外部APIへ送るため、明示承認なしでは実行していない。

承認後も次の制約を維持する。

- `CODEX_HOME`は毎回`mktemp -d`で作成する。
- 通常の`~/.codex`を変更しない。
- 会話には秘密情報ではなく、検証専用のデコイマーカーだけを使う。
- sandboxはread-only、approvalはneverにする。
- 生の会話、Hook record、rollout、認証ファイルは測定後に一時領域から削除する。
- リポジトリへ保存するのは安全な集計値だけにする。

## 参考

- [Codex Hooks公式ドキュメント](https://developers.openai.com/codex/hooks)
- [Codex App Server公式ドキュメント](https://developers.openai.com/codex/app-server)
- [既存の実Codex / compaction評価](jevx-real-codex-compaction-evaluation-2026-09-21.md)
- [jevx Hook shadow仕様](jevx-codex-hooks-evaluation.md)
