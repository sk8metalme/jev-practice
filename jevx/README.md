# jevx

`jevx` は、Codex CLI の作業を邪魔せずに Jev の有用性と速度を測るための、macOS 向け Rust CLI だよ。
Skill 選択のコアは **Shadow Mode**。依頼に合いそうな Skill を提案するだけで、Skill 本文の自動ロード・実行や会話の書き換えはしない。Hook と Compaction 補助は、利用者が明示的に登録したときだけ動くオプトイン機能だよ。

## できること

```text
ユーザー依頼
    │
    ├─ プロジェクト / ユーザーの Skill を探索
    ├─ name・description をローカルで順位付け（最大32件）
    ├─ 候補と none を1回の Jev Choice へ送信
    ├─ 確信度と差分を確認して selected / none を決定
    └─ JSON / human 出力 + 秘密情報を含めないローカル計測
```

- `skills suggest`: 現在の依頼に合う Skill を Jev へ判定させる
- `skills list`: 探索できる Skill カタログを確認する
- `hooks shadow`: Codex Hook の発火を変更なしで記録する
- `hooks install`: 既存設定を保持しながら jevx Hook を明示的に登録する
- `hooks compact-assist`: Compaction前後の安全なcheckpointを記録し、compact後に補助contextを返す
- `hooks correlate`: Hook の相関IDと重複を集計する
- `hooks compact-eval`: 合成 Compaction のベースラインを測る
- `hooks conversation-eval`: 実測から作った安全な JSONL を評価する
- `eval` / `eval-repeat`: Skill 選択の品質・速度・分散を測る
- `doctor` / `stats`: 設定と Telemetry を確認する

### v1 の境界

`jevx` の提案結果は、Codex が通常の Skill 選択ルールに従うための補助情報だよ。`selected` になっても Skill を自動で読み込んだり、コマンドを実行したりしない。Jev の結果を実行権限へ直結させないことが、このツールの安全上の前提。

`hooks install` は設定ファイルを変更するため、既定では何も実行しない。`--dry-run`で内容を確認してから明示的に実行し、CodexのHook Trustを確認して使う。`compact-assist`も会話を要約するLLMではなく、redactedなローカルmanifestとハッシュを復元する決定的な試作だよ。

## クイックスタート

### 必要なもの

- macOS
- Rust stable と Cargo
- Jev を使う場合は `AI_GATEWAY_API_KEY`

APIキーなしでも、一覧表示・診断・dry-run評価・合成Compaction評価・既存JSONLの評価は実行できるよ。

### ビルドとローカル確認

リポジトリのルートから実行する例。

```bash
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo run --locked --manifest-path jevx/Cargo.toml -- doctor --json
```

ローカルビルドを直接使う場合は、次のパスになるよ。

```bash
cargo build --locked --manifest-path jevx/Cargo.toml --release
jevx/target/release/jevx --help
```

### APIキーと設定

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
export JEVX_TELEMETRY=1
```

設定値は次の環境変数で上書きできる。

| 環境変数 | 既定値 | 用途 |
| --- | --- | --- |
| `AI_GATEWAY_API_KEY` | なし | Vercel AI Gateway へのBearer認証。Jev判定時だけ必要 |
| `JEVX_GATEWAY_ENDPOINT` | `https://ai-gateway.vercel.sh/v1/evaluate` | Jev評価APIの送信先。モックサーバー検証にも使える |
| `JEVX_REQUEST_TIMEOUT_MS` | `1500` | Jev HTTPリクエストのタイムアウト |
| `JEVX_HOME` | `$HOME/.jevx` | Telemetry保存先の親ディレクトリ |
| `JEVX_TELEMETRY` | 有効 | `0` または `off` でTelemetryを無効化 |

設定を確認するには、APIキーの値そのものを表示しない `doctor` を使うよ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- doctor --json
```

### Skillを一覧表示する

探索の優先順位は、プロジェクトの `.agents/skills`、`.codex/skills`、ユーザーの `~/.agents/skills`、`$CODEX_HOME/skills`、`--skill-dir` の順だよ。同じ名前のSkillは、優先順位が高いルートを採用する。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  skills list \
  --cwd "$PWD" \
  --skill-dir "$PWD/.agents/skills" \
  --json
```

人間向けに見るときは `--json` を外してね。

## Skillを提案させる

### 文字列で渡す

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  skills suggest \
  --prompt "PDFを結合して内容を確認したい" \
  --skill-dir "$PWD/.agents/skills" \
  --json
```

### JSONで渡す

`--input-json` は `prompt` 必須、`cwd` と `explicit_skill` 任意。JSONフィールドはCLIオプションの `--skill` と違って `explicit_skill` だよ。

```bash
printf '%s\n' \
  '{"prompt":"テストを追加して失敗原因を調べたい","cwd":"/tmp/sample-repo"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      skills suggest --input-json --json --no-telemetry
```

標準入力から本文だけを読む場合は `--stdin` を使える。

```bash
printf '%s\n' 'READMEのリンク切れを確認して修正したい' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      skills suggest --stdin --json
```

### 明示的にSkillを指定する

ユーザーや上位エージェントがSkillを決めている場合は `--skill` を使う。Jevを呼ばず、候補カタログに存在するSkillを `explicit` として返すよ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  skills suggest \
  --prompt "スプレッドシートを確認したい" \
  --skill spreadsheets \
  --skill-dir "$PWD/.agents/skills" \
  --json
```

### 判定の意味

| `decision` | 意味 |
| --- | --- |
| `selected` | Jevが候補を選び、確率が `0.60` 以上、1位と次点の差が `0.10` 以上 |
| `explicit` | `--skill` で明示指定された |
| `none` | 確信度・差分が閾値未満、`none` 選択、未知の選択などで安全側に倒した |
| `no_candidates` | 探索できる候補がなかった |
| `error` | APIキー不足、タイムアウト、Jev応答エラーなど。JSONでは `error.code` を返す |

`selected` は自動実行の許可ではなく、提案の判定結果だよ。

## レスポンスと Jev の速度

`--json` の提案結果は `schemaVersion: 1` のJSON。`jevResponseMs` はJevへのHTTPリクエスト開始から構造化レスポンスの読み取り完了まで、`totalMs` はローカルSkill探索を含む `jevx` 全体の経過時間だよ。

```json
{
  "schemaVersion": 1,
  "decision": "selected",
  "selected": {
    "id": "testing",
    "name": "testing",
    "description": "Write and run tests",
    "path": "/Users/example/.agents/skills/testing/SKILL.md",
    "source": "user",
    "local_score": 25,
    "probability": 0.91
  },
  "candidates": [
    {
      "id": "testing",
      "name": "testing",
      "description": "Write and run tests",
      "path": "/Users/example/.agents/skills/testing/SKILL.md",
      "source": "user",
      "local_score": 25,
      "probability": 0.91
    }
  ],
  "metrics": {
    "discoveryMs": 7,
    "jevResponseMs": 124,
    "totalMs": 138,
    "candidateCount": 3,
    "inputTokens": 82,
    "outputTokens": 12
  },
  "mode": "shadow"
}
```

候補の全体を確認したいときは `candidates` を見る。実際のJSONにはローカル順位付けした候補が入り、Jevの確率が返った候補には `probability` が付くよ。

人間向け出力でも速度が表示される。

```text
jevx skill suggestion
Selected: testing
Probability: 0.91
Candidates: 3
Jev response: 124 ms
Total: 138 ms
Mode: shadow
```

`explicit`、`no_candidates`、APIキーなしのエラーではJevへ到達しないため、`jevResponseMs` が `0` またはJSONに含まれない場合がある。速度を比較するときは、同じ候補カタログ・同じ入力・同じネットワーク条件で `p50` / `p95` を見るのがおすすめ。

## Jev Gatewayへ直接 `curl` する

通常は `jevx` を使えばよいけれど、Jevのリクエストとレスポンス形状だけを確認したいときは、同じGatewayへ直接送れる。これは外部APIへ実際に送信するコマンドなので、秘密情報ではないテスト文だけで実行してね。

```bash
curl --fail-with-body --silent --show-error \
  -X POST "${JEVX_GATEWAY_ENDPOINT:-https://ai-gateway.vercel.sh/v1/evaluate}" \
  -H "Authorization: Bearer ${AI_GATEWAY_API_KEY}" \
  -H 'Content-Type: application/json' \
  --data @- <<'JSON'
{
  "model": "typesafe-ai/jev",
  "state": "{\"prompt\":\"テストを追加して失敗原因を調べたい\",\"cwd\":\"/tmp/sample-repo\",\"candidates\":[{\"id\":\"testing\",\"name\":\"testing\",\"description\":\"Write and run tests\"}]}",
  "questions": {
    "skill": {
      "type": "choice",
      "instructions": "現在の依頼に最も適したSkillを1つ選んでください。適合するSkillがなければnoneを選んでください。",
      "criteria": {
        "testing": "Write and run tests",
        "none": "候補Skillのどれも現在の依頼に適合しない"
      }
    }
  }
}
JSON
```

Gatewayの構造化レスポンスは、例えば次のようになる。

```json
{
  "answers": {
    "skill": {
      "choice": "testing",
      "probabilities": {
        "testing": 0.91,
        "none": 0.09
      }
    }
  },
  "usage": {
    "inputTokens": 82,
    "outputTokens": 12
  }
}
```

`jevx` はこの `answers.skill` を判定へ変換し、`usage` を `metrics` として返す。Jevの回答は自由文ではなく、候補・確率・Token使用量を機械的に扱える構造化結果だよ。

## Telemetryと統計

既定では `$JEVX_HOME/events.jsonl`、通常は `~/.jevx/events.jsonl` へ追記する。依頼文そのものではなく、SHA-256、文字数、判定、候補数、速度、Token使用量などのメタデータを保存する設計。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- stats --json
```

別のTelemetry JSONLを集計するときは `--input` を使える。JSONには判定率、Jev/totalのp50・p95、平均Token数、usageを持つイベント数が入る。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  stats --input /tmp/jevx-events.jsonl --json
```

例:

```json
{
  "events": 40,
  "selected": 36,
  "none": 4,
  "errors": 0,
  "selectedRate": 0.9,
  "noneRate": 0.1,
  "errorRate": 0.0,
  "averageJevResponseMs": 427.0,
  "jevResponseMsP50": 407,
  "jevResponseMsP95": 673,
  "totalMsP50": 409,
  "totalMsP95": 675,
  "averageInputTokens": 2401.2,
  "averageOutputTokens": 127.7,
  "usageEvents": 40
}
```

一回だけ記録を止める場合は `--no-telemetry`、常に止める場合は次のようにする。

```bash
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  skills suggest --prompt "テストを追加したい" --json
```

`--no-telemetry` はローカルの記録だけを止めるオプションで、Jevへの送信自体は止めないよ。外部送信を行いたくない場合は `--skill`、`--dry-run`、APIキーなしの一覧・診断コマンドを使ってね。

## Codex Skillとしてセットアップする

`jevx/skill/SKILL.md` は、Codexへ「必要なら jevx に現在の依頼だけを渡して提案を確認する」ための advisory Skill。Skill本文を自動ロードしたり、jevxの結果だけでコマンド実行を許可したりしない。

### ユーザー領域へインストール

```bash
sh jevx/scripts/setup.sh --scope user
```

インストール先は `${HOME}/.agents/skills/jevx/SKILL.md`。スクリプトは実際に使うインストール先を `--root` で固定するため、バイナリのrootは `JEVX_INSTALL_ROOT`、`CARGO_INSTALL_ROOT`、`CARGO_HOME`、`${HOME}/.cargo` の順で決まるよ。`--hooks` 付きならそのrootのバイナリを直接使うので、`PATH`へ追加する必要はない。単体で `jevx` を呼ぶ場合だけ、表示された `bin` ディレクトリを `PATH` に追加してね。

Skillのセットアップと同時に、Codex Hookも明示的に登録したい場合は `--hooks` を付ける。Hook設定の生成・Trust確認が発生するので、初回は `--dry-run` 付きのコマンドを先に実行するのがおすすめ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope user --dry-run --json
sh jevx/scripts/setup.sh --scope user --hooks
```

### このプロジェクトだけへインストール

```bash
sh jevx/scripts/setup.sh --scope project --repo "$PWD"
```

インストール先は `$PWD/.agents/skills/jevx/SKILL.md`。Codex側のSkill探索対象を明示したいときは `CODEX_HOME` も確認してね。

プロジェクトHookだけを登録する場合は次のとおり。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope project --repo "$PWD" --dry-run --json
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope project --repo "$PWD"
```

`hooks install` の保存先は、user scopeでは `$CODEX_HOME/hooks.json`（未設定なら `~/.codex/hooks.json`）、project scopeでは `<repo>/.codex/hooks.json`。既存のroot設定とカスタムhandlerを保持し、jevxが生成した古いhandlerだけを置き換える。変更時の初回バックアップは `hooks.json.jevx.bak` に保存し、再実行は冪等だよ。

## Codex Hookを安全にshadow検証する

`hooks shadow` は stdin のHook JSONを読み、Codexへは固定の継続レスポンスだけを返す。

```json
{"continue":true,"suppressOutput":true}
```

対応イベントは `SessionStart`、`PreCompact`、`PostCompact`、`UserPromptSubmit`。`UserPromptSubmit` のときだけ、promptが空でなく、Jev判定が可能ならSkill候補を評価する。

```bash
printf '%s\n' \
  '{"hook_event_name":"UserPromptSubmit","session_id":"sample-session","turn_id":"sample-turn","model":"sample-model","prompt":"PDFを結合したい","cwd":"/tmp/sample-repo"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow \
      --event UserPromptSubmit \
      --skill-dir jevx/evals/skills \
      --output /tmp/jevx-hooks.jsonl
```

`--output` へ保存されるHook recordには、次のような安全な値だけが残るよ。

- `sessionIdSha256`、`turnIdSha256`、`modelSha256`、`correlationIdSha256`
- `promptSha256`、`promptChars`
- `decision`、`selectedSkill`
- `discoveryMs`、`jevResponseMs`、`totalMs`、Token使用量、`errorCode`

生のセッションID・ターンID・モデル名・prompt本文は保存しない。Jevを使わないイベントでも、Hookの継続性を確認できる。

### Codex側の接続イメージ

CodexのHook設定へ接続するときは、絶対パスの `jevx hooks shadow` を command hook に指定する。まずは使い捨て `CODEX_HOME` と read-only / approvalなしの環境で確認してね。

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/jevx hooks shadow --skill-dir /absolute/path/to/jevx/evals/skills --output /tmp/jevx-hooks.jsonl"
          }
        ]
      }
    ]
  }
}
```

`SessionStart`、`PreCompact`、`PostCompact` も同じコマンドへ接続できるけれど、実環境へ入れる前にイベントごとの標準入力と終了コードを確認すること。`--dangerously-bypass-hook-trust` は一時検証専用で、常用設定へ持ち込まないでね。

### Hookを自動登録する（明示的なopt-in）

既存の `hooks.json` を手で編集せずに接続する場合は、まずdry-runを確認する。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope project --repo "$PWD" --dry-run --json
```

実行すると、次の4イベントにcommand hookを登録する。

| イベント | matcher | jevx処理 | 外部送信 |
| --- | --- | --- | --- |
| `SessionStart` | `startup\|resume\|clear\|compact` | checkpointを補助contextとして復元 | なし |
| `PreCompact` | `manual\|auto` | compact前のmetadataを記録 | なし |
| `PostCompact` | `manual\|auto` | compact後のmetadataを記録 | なし |
| `UserPromptSubmit` | なし | Skill選択をshadow観測 | Jev設定時はあり |

変更後はCodexで `/hooks` を開き、対象Hookをreview/trustしてから有効化してね。Codexのproject-local HookはプロジェクトTrustの影響を受けるため、設定ファイルを書けたこととHookが発火することは別に確認する。

### Compaction補助の試作

`hooks compact-assist` はHook JSONをstdinから読み、`<state-dir>/hook-records.jsonl` と `checkpoints.jsonl` に安全なmetadataだけを追記する。`SessionStart(source=compact)`のときは、同じセッションのcheckpointと、作業ディレクトリに任意で置いた `.jevx/compact-context.md` をredactして `additionalContext` に返す。

```bash
mkdir -p .jevx
printf '%s\n' 'goal: preserve the release checklist' 'next: run tests' \
  > .jevx/compact-context.md

printf '%s\n' \
  '{"hook_event_name":"SessionStart","source":"compact","session_id":"sample-session","cwd":"'"$PWD"'"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir /tmp/jevx-compaction
```

checkpointには生のmanifest本文を保存せず、redacted本文のSHA-256、文字数、event名、相関ハッシュだけを保存する。`additionalContext`も「補助情報」として返すだけで、Codexの会話履歴・現在のリポジトリ確認・公式Compactionの代替ではないよ。

この補助を使わず、発火だけを観測したい場合は従来どおり `hooks shadow` を使う。公式Hookのmatcherと出力契約は [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks) を確認してね。

### Hookの相関と重複を調べる

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks correlate \
  --input /tmp/jevx-hooks.jsonl \
  --json \
  --output /tmp/jevx-hook-correlation.json
```

出力は `recordCount`、`eventCounts`、`duplicateGroupCount`、`duplicateRecordCount` と相関グループだけ。イベント名、`trigger`、`source`、ハッシュ化されたIDを使ってグループ化し、prompt本文や生のIDを再表示しないよ。

## Compactionの評価

### 合成ベースライン

APIキーやネットワークなしで、固定の合成シナリオを繰り返せる。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks compact-eval --runs 5 --json \
  --output /tmp/jevx-compact-baseline.json
```

これは実Codexを起動するコマンドではなく、jevxの評価器と集計ロジックを検証するための `mode: shadow` のベースラインだよ。

### 実測した会話を安全に評価する

Codex CLI / App Serverで取得した生の会話をそのままリポジトリへ置かず、評価後に破棄する一時JSONLへ次の情報だけを整形する。

```json
{"caseId":"example","model":"model-a","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal: keep-context\nnext: verify\nLOCAL_DECOY_ONLY: redacted","secretMarkers":["LOCAL_DECOY_ONLY"],"compactionCompleted":true,"compactionDurationMs":6200,"inputChars":200,"conversationTurns":8,"contextChars":2030,"failureRecoveryRequired":true,"toolHistoryItems":2,"toolFailureCount":1,"interruptedTurns":1,"recoveryTurns":3,"recoveryCompleted":true,"preCompactionUsage":{"inputTokens":13372,"cachedInputTokens":12672,"outputTokens":5,"reasoningOutputTokens":0,"totalTokens":13377},"postCompactionUsage":{"inputTokens":15922,"cachedInputTokens":15616,"outputTokens":121,"reasoningOutputTokens":73,"totalTokens":16043},"observedEvents":["functionCallOutput","turn/interrupted","contextCompaction","turn/completed"]}
```

評価を実行する。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks conversation-eval \
  --input /tmp/jevx-conversation-cases.jsonl \
  --json \
  --output /tmp/jevx-conversation-report.json
```

レポートにはケースID、モデル、件数、保持率、デコイ漏えい件数、完了フラグ、遅延、Token集計だけが残り、`followUpText`、`requiredFacts`、`secretMarkers` の本文は再保存しない。

### Compaction後のToken指標

`postCompactionUsage` があるケースでは、次の比較用指標を出す。

| 指標 | 計算 | 意味 |
| --- | --- | --- |
| `postCompactionCacheHitRate` | `cachedInputTokens / inputTokens` | 入力のキャッシュ比率 |
| `postCompactionUncachedInputTokens` | `inputTokens - cachedInputTokens` | キャッシュされなかった入力Token |
| `postCompactionEstimatedBillableTokens` | `uncached input + outputTokens` | 請求額ではない相対比較用proxy |

`reasoningOutputTokens` は `outputTokens` に含まれる前提なので、proxyへ二重加算しない。モデル単価、契約、実際の請求処理を含まないため、USD料金とは解釈しないでね。

実Codex CLIのHook発火と、App Serverの `contextCompaction` は別イベント経路。両方を同じ結果として扱わず、Hook recordと会話評価JSONLを分けて保存する。

## Skill選択の評価Runner

40ケースのfixture（合成30件、匿名化テンプレート10件）は [`evals/README.md`](evals/README.md) に説明があるよ。

### APIキーなしのdry-run

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

`none`、ローカルキーワード、Jev（`not_run`）を同じfixtureで比較する。dry-runでは外部APIへ送信しない。

### Jevを使った1回評価

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-eval-results.jsonl
```

### 分散を見る複数回評価

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-repeat.json
```

レポートでは正解率・`none`精度・エラー率と、`discoveryMs`・`jevResponseMs`・`totalMs` の mean / p50 / p95 などを分けて確認できる。出力JSONLへはprompt本文とfixtureのキーワードを保存しない。

### Jevあり / なしの比較結果

同梱40ケースの固定fixtureで、導入効果と追加コストを同じ条件で確認した結果だよ。

| 方式 | 正解率 | `none`精度 | 追加コスト |
| --- | ---: | ---: | --- |
| `none` | 10.0% | 100.0% | 外部通信なし |
| `local_keyword` | 90.0% | 75.0% | 外部通信なし |
| `jevx`（1回） | 100.0% | 100.0% | Jev p50/p95 407/673ms、平均 2,401/128 tokens |
| `jevx`（5回平均） | 98.0% | 100.0% | Jev p50/p95 427/610ms、エラー率2.0% |

5回測定ではローカル候補探索p95が2ms、全体p95が612msだった。今回のfixtureではJevの追加判断がlocal_keywordより良かった一方、Jevの応答時間・Gatewayエラー・Token使用量は常に発生するため、実運用では `stats` のp95とエラー率を一緒に見る。測定条件と「保証ではない」範囲は [`../docs/jevx-comfort-evaluation.md`](../docs/jevx-comfort-evaluation.md) にまとめているよ。

## セキュリティとデータの扱い

- Jevへ送るのは、マスキングしたprompt、作業ディレクトリ、候補SkillのID・名前・説明。Skill本文、過去会話全文、Tool結果、APIキーは送らない。
- `AI_GATEWAY_API_KEY` は環境変数からBearer認証へ使い、レスポンスやTelemetryへ書き出さない。
- Telemetryはprompt本文ではなくハッシュ・文字数・判定・計測値を保存する。
- Hook recordはセッションID、ターンID、モデルをSHA-256化して保存する。
- `/tmp` の会話評価fixtureやCodexのrollout・認証ファイルは、評価後に削除する運用にする。
- `--no-telemetry` はローカル保存を止めるだけ。外部送信も止めたいときは、Jevを呼ぶコマンドを実行しない。
- 直接 `curl` では入力した `state` が送信されるため、秘密情報を含めない。

## トラブルシューティング

### `missing_api_key`

`AI_GATEWAY_API_KEY` が未設定。`doctor --json` で `apiKeyConfigured` を確認してね。明示SkillならAPIキーなしで動作する。

### `timeout` / `provider_error`

Gatewayへの接続、HTTPステータス、JSON形状を確認する。`JEVX_REQUEST_TIMEOUT_MS` は既定1,500msで、`jevResponseMs` はアプリ側のHTTP往復時間。

### `no_candidates`

探索対象に有効な `SKILL.md` がない状態。Frontmatterの `name` と `description` を確認し、必要なら `--skill-dir` を指定する。

### Hookが発火しない

使い捨て `CODEX_HOME` でHook設定を確認し、commandへ絶対パスを使う。stdinのイベント名が `hook_event_name` または `event` にあり、対応イベント名であることも確認してね。

## テストと品質ゲート

コードを変更したときは、少なくとも次をリポジトリルートで実行する。

```bash
cargo fmt --manifest-path jevx/Cargo.toml -- --check
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
```

カバレッジを確認できる環境では、Rustの行カバレッジ98%以上をゲートにする。

```bash
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml \
  --all-features --workspace --summary-only --fail-under-lines 98
```

## 既知の限界

- macOS向けのローカルCodex CLI運用を主対象にしている。
- Skill選択のコアはShadow Modeで、Skill自動ロード・自動実行はしない。`hooks install` と `compact-assist` は明示的に登録した場合だけ動く試作機能。
- APIキーありの `skills suggest` / `eval` / `eval-repeat` / UserPromptSubmit Hookは外部ネットワークへ送信する。
- `hooks install` は設定を変更するため、dry-run・バックアップ・Codex側のHook Trust確認が必要。
- `compact-assist` は会話本文の要約やCodex公式Compactionの代替ではなく、redactedなローカルmanifestとcheckpoint metadataを補助contextとして返すだけ。
- `hooks compact-eval` は合成データ。実CodexのCompaction時間や品質を示すものではない。
- `hooks conversation-eval` は、別途安全に整形したfixtureを評価するだけで、CodexやApp Serverを起動しない。
- `postCompactionEstimatedBillableTokens` は請求額ではなく、同じ条件内の比較用Token proxy。
- 小標本のp50 / p95 は代表的な性能保証ではない。モデル、ネットワーク、候補数、会話形状を固定して比較してね。

## さらに読む

- [`../README.md`](../README.md): プロジェクト全体と画面付きのJevアプリ例
- [`skill/SKILL.md`](skill/SKILL.md): Codexへ登録するadvisory Skill
- [`evals/README.md`](evals/README.md): 40ケース評価fixtureの仕様
- [`../docs/jevx-requirements.md`](../docs/jevx-requirements.md): 要件定義
- [`../docs/jevx-architecture.md`](../docs/jevx-architecture.md): アーキテクチャと安全側判定
- [`../docs/jevx-evaluation.md`](../docs/jevx-evaluation.md): 評価Runnerの仕様
- [`../docs/jevx-codex-hooks-evaluation.md`](../docs/jevx-codex-hooks-evaluation.md): Hook shadow評価
- [`../docs/jevx-comfort-evaluation.md`](../docs/jevx-comfort-evaluation.md): Jevあり/なし比較と導入判断
- [`../docs/jevx-real-codex-compaction-evaluation-2026-09-21.md`](../docs/jevx-real-codex-compaction-evaluation-2026-09-21.md): 実Codex / 実会話型Compaction評価
- [`../docs/jevx-compaction-depth-evaluation-2026-09-21.md`](../docs/jevx-compaction-depth-evaluation-2026-09-21.md): Hook直接検証とCompaction深掘り評価
- [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)
- [OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)
