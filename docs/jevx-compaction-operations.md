# jevx Codex CLI compaction実運用runbook

## 目的と境界

このrunbookは、jevxのHookとcompact-assistをCodex CLIへ安全に接続し、compact前後の状態を観測・補助するための手順だよ。設定変更はopt-inで行い、通常のCodex compactionや現在の会話・リポジトリ確認を置き換えない。

compact-assistが返すのは、checkpoint metadataと任意のredacted manifestだけ。会話全文の要約、公式compactionの再実装、Tool Resultの削除、現在の作業状態の保証はしない。

## Hook契約

| イベント | 役割 | jevxの動作 | Codexへ返すもの |
| --- | --- | --- | --- |
| `PreCompact` | compact前。`trigger` は `manual` / `auto` | session・turn・cwdのhashとredacted contextのmetadataをcheckpointへ追記 | `continue: true` |
| `PostCompact` | compact後。`trigger` は `manual` / `auto` | compact完了のcheckpointを追記 | `continue: true` |
| `SessionStart(source=compact)` | compact後、次のmodel request前 | 最新checkpointとredacted manifestを追加contextへ組み立てる | `hookSpecificOutput.additionalContext` |
| `UserPromptSubmit` | ユーザー入力の送信前 | Shadow Modeで候補探索・Jev判定・安全なmetricsを記録 | `continue: true`。会話本文は追加しない |

`PreCompact` / `PostCompact` は追加contextを返す場所ではなく、compact後の補助contextは `SessionStart(source=compact)` へ限定する。`suppressOutput` は現行Codexではparseされるだけなので、表示抑制の保証に使わない。

公式仕様では、hookのmatcherはイベントごとの `source` / `trigger` に適用される。通常のcommand hookのtimeoutと、`SessionEnd` / `Interrupt` の1〜3秒制限は分けて扱う。現在のinstallerが生成するtimeout値を変更するときは、対象イベントの公式仕様を再確認すること。

## データ保護

- 実験・実測は合成入力だけで行い、APIキー、認証ファイル、生の会話、Tool Result、manifest本文をリポジトリへ保存しない。
- Hook recordとcheckpointにはprompt、session ID、turn ID、model ID、cwdを生で保存せず、hashまたは文字数だけを残す。
- `SessionStart`の追加contextへ出すmanifestもredact後の最大4,000文字だけにし、秘密値を含むファイルを指定しない。
- `UserPromptSubmit`のJev判定失敗は `errorCode` に変換し、Hookは `continue: true` でCodexの処理を止めない。
- 通常のuser profileを直接変更しない。実測では使い捨て `CODEX_HOME` と `JEVX_HOME` を指定する。

## 1. まずローカルfixtureだけで確認する

Rustのテストとfixture評価は外部サービスを使わずに実行できる。

~~~bash
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo fmt --manifest-path jevx/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --all-targets --fail-under-lines 98
~~~

Hookのstdout契約を確認する。

~~~bash
fixture_dir=$(mktemp -d -t jevx-compaction-fixture.XXXXXX)
export JEVX_HOME="$fixture_dir/data"

printf '%s\n' '{"hook_event_name":"PreCompact","trigger":"manual","session_id":"fixture-session","turn_id":"fixture-turn","cwd":"/tmp/safe-fixture"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir "$fixture_dir/compaction"

printf '%s\n' '{"hook_event_name":"PostCompact","trigger":"manual","session_id":"fixture-session","turn_id":"fixture-after","cwd":"/tmp/safe-fixture"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir "$fixture_dir/compaction"

printf '%s\n' '{"hook_event_name":"SessionStart","source":"compact","session_id":"fixture-session","cwd":"/tmp/safe-fixture"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir "$fixture_dir/compaction"
~~~

3つの応答がJSONとして読み取れ、すべて `continue: true` ならHookの継続契約は満たす。最後の応答だけ `hookSpecificOutput.additionalContext` を持ち、`checkpoint event=PostCompact` または `checkpoint metadata was not found` を含む。

評価器を使うときは、本文を一時JSONLへ置いて集計後に破棄する。

~~~bash
case_file="$fixture_dir/conversation.jsonl"
printf '%s\n' '{"caseId":"safe-case","model":"fixture-model","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal=keep-context\nnext=verify\ndecoy=redacted","secretMarkers":["LOCAL_FIXTURE_SECRET"],"compactionCompleted":true,"compactionDurationMs":120,"inputChars":300,"conversationTurns":8,"contextChars":1200,"observedEvents":["contextCompaction","turn/completed"]}' > "$case_file"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks conversation-eval --input "$case_file" --json
~~~

レポートには保持件数・漏えい件数・遅延・エラーコードだけが残り、`followUpText`と秘密marker本文は再出力されないことを確認する。

## 2. installerの変更をdry-runでレビューする

生成先とデータ保存先を一時profileへ分離してから、まずdry-runだけを実行する。

~~~bash
codex_profile=$(mktemp -d -t jevx-codex-profile.XXXXXX)
export CODEX_HOME="$codex_profile"
export JEVX_HOME="$codex_profile/jevx-data"

cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope user --repo "$PWD" --dry-run --json
~~~

dry-runでは `CODEX_HOME/hooks.json`、backup、Hook stateを作らない。JSONの次をレビューする。

- `SessionStart` matcherが `startup|resume|clear|compact`
- `PreCompact` / `PostCompact` matcherが `manual|auto`
- 既存root、既存のカスタムhandler、未知イベントが保持される
- jevxの古いmanaged handlerだけが置換される
- `UserPromptSubmit` は候補提案のshadow用途で、Skill本文をロード・実行しない
- `compact-assist` は `SessionStart(source=compact)` へ追加contextを返すだけで、会話を停止しない

dry-runのJSONに秘密値や生の入力が含まれていないことも確認する。実書き込みを行う場合は、同じ一時 `CODEX_HOME` に対して明示的に再実行する。初回だけ `hooks.json.jevx.bak` が作られ、2回目は `changed=false` になることを確認する。

## 3. Codex CLIでmanual compactをsmoke testする

実Codexを使う測定は、認証済みの外部サービスへ合成入力を送るため、必要な許可がある場合だけ行う。開始前に次を確認する。

~~~bash
codex --version
test -n "$CODEX_HOME"
test -n "$JEVX_HOME"
~~~

一時profileのまま、read-only・approvalなしでCodexを起動する。

~~~bash
codex --sandbox read-only --ask-for-approval never --no-alt-screen --cd "$PWD"
~~~

起動後の順序:

1. `/hooks` を開き、生成されたHookのcommand・matcher・保存先をreviewしてtrustする。
2. 秘密値を含まない短い依頼を送り、Hookが処理を継続することを確認する。
3. `/compact` を実行し、compact完了表示を確認する。
4. compact後に、最初の依頼で指定した安全なgoal・制約・next actionを再確認する。
5. Codexを終了し、Hook recordを相関分析する。

~~~bash
records="$JEVX_HOME/hooks.jsonl"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks correlate --input "$records" --json
~~~

成功経路では少なくとも `PreCompact`、`PostCompact`、`SessionStart`（`source=compact`）を観測する。相関分析の `duplicateRecordCount` は0を期待するが、1回のsmoke testだけで長期運用の統計的な重複率を保証しない。

Hookをtrustするためだけに `--dangerously-bypass-hook-trust` を常用しない。外部で定義をレビュー済みの一回限りの自動化で使う場合も、実行profile・入力・出力を分離して記録する。

## 4. 実測結果の判定

### ローカル合格

- Rustの全テスト、fmt、clippyが成功する。
- ラインカバレッジが98%以上である。
- dry-runがファイルを変更しない。
- installの初回backupと2回目の冪等性を確認できる。
- fixture評価で必須事実保持率100%、秘密marker漏えい0件、Hook継続率100%になる。

### 実Codex smoke test合格

- `PreCompact` / `PostCompact` / `SessionStart(source=compact)` が成功経路で発火する。
- Hook応答が `continue: true` で、compact後の会話が継続する。
- `SessionStart(source=compact)` の追加contextに秘密値・生ID・生manifestがない。
- Hook recordにprompt本文・生session/turn/model IDがない。
- `hooks correlate` の重複recordが0件である。

### 記録上の限界

- 実CLIのHook lifecycleとApp Serverの `thread/compact/start` / `contextCompaction` は別経路。App ServerでcompactしただけではCLI Hook recordの発火を証明しない。
- compact-assistのredacted manifestは補助情報であり、Codex内部のopaqueなcompaction itemやcanonical contextの代替ではない。
- 少数ケースのp50/p95は、その時点のモデル・profile・入力条件の観測値であり、長期運用の品質保証ではない。
- OpenAI APIのstandalone compactionを使う経路では、公式仕様どおり返却されたcompact後windowを削除・再構成せず、次のrequestへそのまま渡す。

## 公式仕様

- [Codex Hooks](https://learn.chatgpt.com/docs/hooks)
- [OpenAI API Compaction](https://developers.openai.com/api/docs/guides/compaction)
