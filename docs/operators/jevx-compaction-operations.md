# jevx Codex CLI compaction実運用runbook

> 対象読者: Codex CLIを導入・運用する担当者
> 文書の状態: 現行運用・canonical runbook（2026-09-22）

## 目的と境界

このrunbookは、jevxのHook、意味レビュー、費用観測、compact-assistをCodex CLIへ安全に接続し、速度・品質・費用を同時に確認するための手順だよ。設定変更と本文の外部送信はopt-inで行い、通常のCodex compactionや現在の会話・リポジトリ確認を置き換えない。

compact-assistが返すのは、checkpoint metadataと任意のredacted manifestだけ。会話全文の要約、公式compactionの再実装、Tool Resultの削除、現在の作業状態の保証はしない。

## Hook契約

`hooks shadow` が受理するknown eventは `SessionStart`、`PreCompact`、`PostCompact`、`UserPromptSubmit` だけです。known eventの受理時だけ `{"continue":true,"suppressOutput":true}` の固定応答を返します。未知event、event不一致、壊れたJSONは固定応答を返さずエラーにするため、「すべての入力に継続応答する」とは解釈しないでください。

| イベント | 役割 | jevxの動作 | Codexへ返すもの |
| --- | --- | --- | --- |
| `PreCompact` | compact前。`trigger` は `manual` / `auto` | session・turn・cwdのhashとredacted contextのmetadataをcheckpointへ追記 | `continue: true` |
| `PostCompact` | compact後。`trigger` は `manual` / `auto` | compact完了のcheckpointを追記 | `continue: true` |
| `SessionStart(source=compact)` | compact後、次のmodel request前 | 最新checkpointとredacted manifestを追加contextへ組み立てる | `hookSpecificOutput.additionalContext` |
| `UserPromptSubmit` | ユーザー入力の送信前 | Shadow Modeで候補探索・Jev判定・prompt review・metricsを記録 | `continue: true`。本文送信は`--allow-content`時だけ |
| `PostToolUse` / `Stop` | diff/final answer後 | review receipt、route、fix status、costを記録 | `continue: true`。本文送信は`--allow-content`時だけ |

`PreCompact` / `PostCompact` は追加contextを返す場所ではなく、compact後の補助contextは `SessionStart(source=compact)` へ限定する。`suppressOutput` は現行Codexではparseされるだけなので、表示抑制の保証に使わない。Jev/Gatewayエラー時もknown eventなら `continue: true` を優先するが、unknown/mismatch eventには固定応答を返さない。

公式仕様では、hookのmatcherはイベントごとの `source` / `trigger` に適用される。通常のcommand hookのtimeoutと、`SessionEnd` / `Interrupt` の1〜3秒制限は分けて扱う。現在のinstallerが生成するtimeout値を変更するときは、対象イベントの公式仕様を再確認すること。

## データ保護

- 実験・実測は合成入力だけで行い、APIキー、認証ファイル、生の会話、Tool Result、manifest本文をリポジトリへ保存しない。
- Hook recordとcheckpointにはprompt、session ID、turn ID、model ID、cwdを生で保存せず、hashまたは文字数だけを残す。
- `trigger` / `source` / `selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなidentifierを拒否する。既存schema v1のloadでは該当metadataを正規化・欠損化して読み続けるため、過去ログの分析を止めない。
- `SessionStart`の追加contextへ出すmanifestもredact後の最大4,000文字だけにし、秘密値を含むファイルを指定しない。
- `UserPromptSubmit`のJev判定失敗は `errorCode` に変換し、Hookは `continue: true` でCodexの処理を止めない。
- `hooks review`は既定で本文をJevへ送らず、redact済みローカル検出だけを行う。`--allow-content`を付けたinstall/reviewだけが選択対象を送る。raw Tool result、API key、資格情報は常に除外する。
- 費用はJev/Codex/totalを分け、推定`estimated`と実費`actual`、通貨、price version、`unknown`/`unavailable`をreceiptへ残す。費用上限や自動停止は行わない。
- 通常のuser profileを直接変更しない。実測では使い捨て `CODEX_HOME` と `JEVX_HOME` を指定する。

## 1. まずローカルfixtureだけで確認する

Rustのテストとfixture評価は外部サービスを使わずに実行できる。

品質検証まで行う場合、JSON確認に `jq`（macOSなら`brew install jq`）、カバレッジ確認に任意の `cargo-llvm-cov`（`cargo install cargo-llvm-cov`）が必要です。

~~~bash
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo fmt --manifest-path jevx/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --all-targets \
  --ignore-filename-regex 'src/(cli/.*|compact_assist|decision|discovery|error|gateway|hook_config|ranking|redaction|storage|telemetry|types)\\.rs' \
  --summary-only --fail-under-lines 98
~~~

Hookのstdout契約を確認する。

~~~bash
(
  set -eu
  fixture_dir=$(mktemp -d -t jevx-compaction-fixture.XXXXXX)
  trap 'rm -rf "$fixture_dir"' EXIT
  export JEVX_HOME="$fixture_dir/data"

  printf '%s\n' '{"hook_event_name":"PreCompact","trigger":"manual","session_id":"fixture-session","turn_id":"fixture-turn","cwd":"/tmp/controlled-fixture"}' \
    | cargo run --locked --manifest-path jevx/Cargo.toml -- \
        hooks compact-assist --state-dir "$fixture_dir/compaction"

  printf '%s\n' '{"hook_event_name":"PostCompact","trigger":"manual","session_id":"fixture-session","turn_id":"fixture-after","cwd":"/tmp/controlled-fixture"}' \
    | cargo run --locked --manifest-path jevx/Cargo.toml -- \
        hooks compact-assist --state-dir "$fixture_dir/compaction"

  printf '%s\n' '{"hook_event_name":"SessionStart","source":"compact","session_id":"fixture-session","cwd":"/tmp/controlled-fixture"}' \
    | cargo run --locked --manifest-path jevx/Cargo.toml -- \
        hooks compact-assist --state-dir "$fixture_dir/compaction"
)
~~~

3つの応答がJSONとして読み取れ、すべて `continue: true` ならHookの継続契約は満たす。最後の応答だけ `hookSpecificOutput.additionalContext` を持ち、`checkpoint event=PostCompact` または `checkpoint metadata was not found` を含む。

評価器を使うときは、本文を一時JSONLへ置いて集計後に破棄する。

~~~bash
(
  set -eu
  evaluation_dir=$(mktemp -d -t jevx-conversation-eval.XXXXXX)
  trap 'rm -rf "$evaluation_dir"' EXIT
  case_file="$evaluation_dir/conversation.jsonl"
  printf '%s\n' '{"caseId":"controlled-case","model":"fixture-model","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal=keep-context\nnext=verify\ndecoy=redacted","secretMarkers":["LOCAL_FIXTURE_SECRET"],"compactionCompleted":true,"compactionDurationMs":120,"inputChars":300,"conversationTurns":8,"contextChars":1200,"observedEvents":["contextCompaction","turn/completed"]}' > "$case_file"
  cargo run --locked --manifest-path jevx/Cargo.toml -- \
    hooks conversation-eval --input "$case_file" --json
)
~~~

レポートには保持件数・漏えい件数・遅延・エラーコードだけが残り、`followUpText`と秘密marker本文は再出力されないことを確認する。

## 2. installerの変更をdry-runでレビューする

生成先とデータ保存先を一時profileへ分離してから、まずdry-runだけを実行する。

~~~bash
(
  set -eu
  codex_profile=$(mktemp -d -t jevx-codex-profile.XXXXXX)
  trap 'rm -rf "$codex_profile"' EXIT
  export CODEX_HOME="$codex_profile"
  export JEVX_HOME="$codex_profile/jevx-data"
  # 認証キャッシュを一時profile内へ限定し、終了時にtrapで削除する。
  printf '%s\n' 'cli_auth_credentials_store = "file"' > "$CODEX_HOME/config.toml"

  cargo run --locked --manifest-path jevx/Cargo.toml -- \
    hooks install --scope user --repo "$PWD" --dry-run --json

  # 既存設定の保持とbackupを検証するための合成設定。秘密値は入れない。
  printf '%s\n' '{"description":"fixture","hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"custom-handler"}]}]}}' \
    > "$CODEX_HOME/hooks.json"

  first_install=$(cargo run --locked --manifest-path jevx/Cargo.toml -- \
    hooks install --scope user --repo "$PWD" --json)
  printf '%s\n' "$first_install"
  test -f "$CODEX_HOME/hooks.json.jevx.bak"

  second_install=$(cargo run --locked --manifest-path jevx/Cargo.toml -- \
    hooks install --scope user --repo "$PWD" --json)
  printf '%s\n' "$second_install"
  printf '%s\n' "$second_install" | jq -e '.changed == false'
)
~~~

dry-runでは `CODEX_HOME/hooks.json`、backup、Hook stateを作らない。JSONの次をレビューする。上の実install例では、合成した既存設定をjevxが保持し、初回だけbackupを作り、2回目に`changed=false`になることも確認する。

- `SessionStart` matcherが `startup|resume|clear|compact`
- `PreCompact` / `PostCompact` matcherが `manual|auto`
- 既存root、既存のカスタムhandler、未知イベントが保持される
- jevxの古いmanaged handlerだけが置換される
- `UserPromptSubmit` は候補提案のshadow用途で、Skill本文をロード・実行しない
- `compact-assist` は `SessionStart(source=compact)` へ追加contextを返すだけで、会話を停止しない
- review handlerは4カテゴリを1回のtyped requestへまとめ、route適用証拠がなければ`degraded`にする。fixは`--auto-fix --yes`・hash・safe pathの全gateが必要

dry-runのJSONに秘密値や生の入力が含まれていないことも確認する。空のprofileでは既存の`hooks.json`がないため、実installを初めて実行してもbackupが作られない。backupを確認する場合は、上のようにfixture用の既存設定を先に用意し、既存設定がある場合だけ初回installで`hooks.json.jevx.bak`が作られることを確認する。

### 常用user hooksはrelease配置から登録する

`cargo run`や`target/debug/jevx`から常用の`hooks.json`を登録すると、Hook commandがworktreeの絶対パスに固定される。fixtureとdry-run以外では、`cargo install --root`を使う`setup.sh`でreleaseバイナリを配置し、その絶対パスから登録する。

setup scriptの主な引数は `setup.sh --scope user|project [--repo PATH] [--hooks]` で、ヘルプは `--help` / `-h` で表示できる。install rootは `jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"` で決めて `JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user` のように実行し、単体 `jevx` もPATHから呼ぶ場合は同rootの `bin` を `PATH`へ追加する。

~~~bash
(
  set -eu
  jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
  JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user --hooks
  jevx_bin="$jevx_install_root/bin/jevx"
  test -x "$jevx_bin"
  "$jevx_bin" --version

  hooks_home="${CODEX_HOME:-$HOME/.codex}"
  hooks_file="$hooks_home/hooks.json"
  test -f "$hooks_file"
  jevx_commands=$(jq -r '.hooks | to_entries[] | .value[]? | .hooks[]? | select((.command? // "") | tostring | contains("jevx")) | .command // empty' "$hooks_file")
  printf '%s\n' "$jevx_commands"
  ! printf '%s\n' "$jevx_commands" | grep -q '/target/'
  printf '%s\n' "$jevx_commands" | grep -F "$jevx_bin" >/dev/null
)
~~~

Hook commandは絶対パスなので、`$HOME/.local/bin`を`PATH`へ追加しなくてもCodexから実行できる。初回の定義変更後は`/hooks`で内容をreviewしてtrustする。

## 3. Codex CLIでmanual compactをsmoke testする

実Codexを使う測定は、認証済みの外部サービスへ合成入力を送るため、必要な許可がある場合だけ行う。開始前に次を確認する。

起動後の順序:

1. `/hooks` を開き、生成されたHookのcommand・matcher・保存先をreviewしてtrustする。
2. 秘密値を含まない短い依頼を送り、Hookが処理を継続することを確認する。
3. `/compact` を実行し、compact完了表示を確認する。
4. compact後に、最初の依頼で指定したcontrolled fixtureのgoal・制約・next actionを再確認する。
5. Codexを終了し、Hook recordを相関分析する。

~~~bash
(
  set -eu
  codex_profile=$(mktemp -d -t jevx-codex-smoke.XXXXXX)
  trap 'rm -rf "$codex_profile"' EXIT
  export CODEX_HOME="$codex_profile"
  export JEVX_HOME="$codex_profile/jevx-data"

  # 常用設定と同じreleaseバイナリを使い、worktreeのtargetへ依存させない。
  jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
  jevx_bin="$jevx_install_root/bin/jevx"
  test -x "$jevx_bin"
  case "$jevx_bin" in
    */target/*) echo 'release binary is required for Codex hooks' >&2; exit 1 ;;
  esac

  "$jevx_bin" hooks install --scope user --repo "$PWD" --json

  codex --version
  test -n "$CODEX_HOME"
  test -n "$JEVX_HOME"

  # 空profileは通常profileの認証を引き継がない。未認証なら同じprofileでloginする。
  if ! codex login status >/dev/null 2>&1; then
    if test -n "${OPENAI_API_KEY:-}"; then
      printenv OPENAI_API_KEY | codex login --with-api-key >/dev/null
    else
      codex login
      # headless環境では上の `codex login` を `codex login --device-auth` に置き換える。
    fi
  fi
  codex login status
  unset OPENAI_API_KEY

  # ここでCodexが終了するまで、上の手順1〜4を対話的に実行する。
  codex --sandbox read-only --ask-for-approval never --no-alt-screen --cd "$PWD"

  records="$JEVX_HOME/compaction/hook-records.jsonl"
  test -s "$records"
  cargo run --locked --manifest-path jevx/Cargo.toml -- \
    hooks correlate --input "$records" --json
)
~~~

成功経路では少なくとも `PreCompact`、`PostCompact`、`SessionStart`（`source=compact`）を観測する。compaction Hook recordは `$JEVX_HOME/compaction/hook-records.jsonl` に保存され、UserPromptSubmitのshadow recordだけが `$JEVX_HOME/hooks.jsonl` に保存される。上の相関分析は前者を入力にする。UserPromptSubmitも含める場合は、両方を同じ一時JSONLへ結合してから `--input` に渡す。`duplicateRecordCount` は0を期待するが、1回のsmoke testだけで長期運用の統計的な重複率を保証しない。

Hookをtrustするためだけに `--dangerously-bypass-hook-trust` を常用しない。外部で定義をレビュー済みの一回限りの自動化で使う場合も、実行profile・入力・出力を分離して記録する。

## 4. 実測結果の判定

### ローカル合格

- Rustの全テスト、fmt、clippyが成功する。
- ラインカバレッジが98%以上である。
- dry-runがファイルを変更しない。
- 既存設定を用意した場合の初回backupと2回目の冪等性を確認できる。空profileでbackupがない場合も正常として扱う。
- fixture評価で必須事実保持率100%、秘密marker漏えい0件、Hook継続率100%になる。
- `reviews.jsonl`の本文非保存、unknown/unavailable非0化、Jev/Codex/totalとtask/turn/session集計を確認できる。

### 実Codex smoke test合格

- `PreCompact` / `PostCompact` / `SessionStart(source=compact)` が成功経路で発火する。
- Hook応答が `continue: true` で、compact後の会話が継続する。
- `SessionStart(source=compact)` の追加contextに秘密値・生ID・生manifestがない。
- Hook recordにprompt本文・生session/turn/model IDがない。
- `hooks correlate` の重複recordが0件である。

### 2026-09-22 実セッション再検証

release配置とcontext guard修正後、実際のCodex CLIで合成依頼→`/compact`→後続依頼を再実行した。

- 環境: `codex-cli 0.155.1`、`jevx 0.1.0`、`$HOME/.local/bin/jevx`。Codexはread-only、承認要求なしで起動した。
- 最初の依頼は`SMOKE_READY`、`/compact`は`Context compacted`、後続依頼は`COMPACT_CONTINUED`を返した。
- 最終セッションの前後で、`PreCompact` / `PostCompact` / `SessionStart(source=compact)` は各2→3、compact系checkpointは6→9へ増加した。
- `UserPromptSubmit`も実依頼2回分が記録され、Hookの`Running hooks`表示を確認した。
- `hooks correlate`は`duplicateGroupCount=0`、`duplicateRecordCount=0`。recordとcheckpointに生prompt・生session IDはなかった。
- 以前の実測で出た`invalid PreCompact hook JSON output`と`invalid stop hook JSON output`は、共有`context_guard.py`がCodex向けJSONではなく警告文をstdoutへ出していたことが原因だった。dotfiles側の[修正PR #36](https://github.com/sk8metalme/dotfiles/pull/36)で、Codexの`model`入力を検出した`PostToolUse` / `PreCompact` / `Stop`だけ`continue: true`と`systemMessage`を含むJSONへ切り替え、Claude向けの既存出力は維持した。

Codexは同じイベントに設定された複数Hookをすべて実行するため、jevx以外のHookを追加・変更した場合も、stdoutがイベントのJSON契約を満たすかを`/hooks`と合成入力で再確認すること。

### 記録上の限界

- 実CLIのHook lifecycleとApp Serverの `thread/compact/start` / `contextCompaction` は別経路。App ServerでcompactしただけではCLI Hook recordの発火を証明しない。
- compact-assistのredacted manifestは補助情報であり、Codex内部のopaqueなcompaction itemやcanonical contextの代替ではない。
- 少数ケースのp50/p95は、その時点のモデル・profile・入力条件の観測値であり、長期運用の品質保証ではない。
- OpenAI APIのstandalone compactionを使う経路では、公式仕様どおり返却されたcompact後windowを削除・再構成せず、次のrequestへそのまま渡す。

## 公式仕様

- [Codex Hooks](https://learn.chatgpt.com/docs/hooks)
- [Codex Authentication](https://learn.chatgpt.com/docs/auth)
- [OpenAI API Compaction](https://developers.openai.com/api/docs/guides/compaction)
