# jevx CLIリファレンス

`jevx` の各コマンドの使い方、出力、設定、データの扱いをまとめた正本です。導入の判断や最初の手順は [jevx/README.md](../../jevx/README.md) と [ユーザー向けガイド](../users/getting-started.md) を、設計の背景は [アーキテクチャ](jevx-architecture.md) を参照してください。

コマンド例はリポジトリのルートで `cargo run --locked --manifest-path jevx/Cargo.toml -- <command>` として書いています。インストール済みなら `jevx <command>` に置き換えられます。

## インストールと設定

最短手順は [jevx/README.md](../../jevx/README.md#5分で最初の提案まで) にある。ここでは前提と設定値の詳細をまとめる。

### 必要なもの

- macOS
- Rust stable と Cargo（`cargo` がPATHにあること）
- docsのJSON検証には `jq`、カバレッジ確認には任意で `cargo-llvm-cov`（`cargo install cargo-llvm-cov`）
- Jev を使う場合は `AI_GATEWAY_API_KEY`

APIキーなしでも、一覧表示・診断・dry-run評価・合成Compaction評価・既存JSONLの評価は実行できるよ。

### Install rootとPATH

リポジトリのルートから `setup.sh` を使う場合の主な引数は `--scope user|project [--repo PATH] [--hooks]`。ヘルプは `--help` / `-h` で表示できる。ここではinstall rootを `$HOME/.local` に固定し、release binaryの `bin` をPATHへ追加する。

```bash
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user
export PATH="$jevx_install_root/bin:$PATH"
jevx doctor --json
```

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
| `JEVX_REQUEST_TIMEOUT_MS` | `1500` | Jev呼び出し全体の予算（リトライを含む）。締め切りを超えるbackoffは再試行しない |
| `JEVX_HOME` | `$HOME/.jevx` | Telemetry保存先の親ディレクトリ |
| `JEVX_TELEMETRY` | 有効 | `0` または `off` でTelemetryを無効化 |
| `JEVX_MAX_STATE_BYTES` | `32000` | 送信stateのbyte上限（1,024〜262,144）。超過時はomittedを記録してdegraded |
| `JEVX_MAX_RETRIES` | `1` | 429/529に対する最大retry回数（0〜3） |
| `JEVX_RETRY_BACKOFF_MS` | `25` | retry backoffの基準ミリ秒（0〜1,000）。n回目は基準×2^(n-1) |
| `JEVX_DECISION_CACHE_CAPACITY` | `0` | process-local answer cacheの上限（0〜10,000）。0は無効 |
| `JEVX_INPUT_COST_WEIGHT` / `JEVX_OUTPUT_COST_WEIGHT` | `1.0` / `1.0` | token proxyの相対cost重み（0〜1,000）。通貨ではない |
| `JEVX_MIN_PROBABILITY` | `0.60` | `selected` に必要なJev確率の下限（0〜1） |
| `JEVX_MIN_MARGIN` | `0.10` | 1位と次点の確率差の下限（0〜1） |
| `JEVX_MAX_CANDIDATES` | `32` | Jevへ送る候補数の上限（1〜256） |

閾値3つ（`JEVX_MIN_PROBABILITY` / `JEVX_MIN_MARGIN` / `JEVX_MAX_CANDIDATES`）とDecision Contractの実行上限6つ（`JEVX_MAX_STATE_BYTES` / `JEVX_MAX_RETRIES` / `JEVX_RETRY_BACKOFF_MS` / `JEVX_DECISION_CACHE_CAPACITY` / `JEVX_INPUT_COST_WEIGHT` / `JEVX_OUTPUT_COST_WEIGHT`）は、最後の逃げ道として用意している。費用はProvider usage/cost payloadを使い、jevx独自の単価設定は持たない。既定値は評価で決めた値なので、変える前に `eval` / `eval-repeat` で同じ条件の比較を取ってね。範囲外・数値でない値・`NaN` / `inf` は既定値へ戻り、`doctor` が警告を出す。新しい設定項目は増やさない（[PHILOSOPHY.md](../../jevx/PHILOSOPHY.md)）。

### 診断と次の一手（`doctor`）

`doctor` はAPIキーの値を表示せずに、設定と導入状態を確認する。足りないものがあると `nextSteps` に次に実行するコマンドが入る。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- doctor --json
```

| キー | 内容 |
| --- | --- |
| `apiKeyConfigured` / `endpoint` / `telemetryPath` / `telemetryEnabled` | Jev接続とTelemetryの設定 |
| `jevxOnPath` | `PATH` 上に `jevx` があるか |
| `skillInstalled` | project / user のSkill rootに `jevx/SKILL.md` があるか |
| `hooks.user` / `hooks.project` | `hooks.json` のパスと登録済みjevx handler数（ファイルがなければ `null`、壊れていれば `error`） |
| `thresholds` | 有効な閾値と、既定値から変えているか（`customized`） |
| `limits` | 有効なタイムアウト・state上限・retry・cache・cost重みと、既定値から変えているか（`customized`） |
| `warnings` | 既定値へ戻した環境変数の説明 |
| `nextSteps` | 次に実行するとよいコマンド |

### Skillを一覧表示する

探索の優先順位は、プロジェクトの `.agents/skills`、`.codex/skills`、ユーザーの `~/.agents/skills`、`$CODEX_HOME/skills`（`CODEX_HOME`未設定時は`~/.codex/skills`）、`--skill-dir` の順だよ。同じ名前のSkillは、優先順位が高いルートを採用する。`skills list` / `skills suggest` / `hooks shadow` の `--skill-dir` は既定rootsへの追加。`eval` / `eval-repeat` だけは `--skill-dir` のカタログしか使わない（評価の再現性のため）。

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
| `selected` | Jevが候補を選び、確率が `0.60` 以上、1位と次点の差が `0.10` 以上（既定値。`JEVX_MIN_PROBABILITY` / `JEVX_MIN_MARGIN` で変更可能） |
| `explicit` | `--skill` で明示指定された |
| `none` | 確信度・差分が閾値未満、`none` 選択、未知の選択などで安全側に倒した |
| `no_candidates` | 探索できる候補がなかった |
| `error` | APIキー不足、タイムアウト、Jev応答エラーなど。JSONでは `error.code` を返す |

`selected` は自動実行の許可ではなく、提案の判定結果だよ。

## レスポンスと Jev の速度

`--json` の提案結果は `schemaVersion: 2` のJSON。`jevResponseMs` はJevへのHTTPリクエスト開始から構造化レスポンスの読み取り完了まで、`totalMs` はローカルSkill探索を含む `jevx` 全体の経過時間だよ。`metrics.cost`はJev/Codex/合算を持ち、各componentは`status`（available/unknown/unavailable）、`basis`（estimated/actual）、通貨、価格版を含む。

```json
{
  "schemaVersion": 2,
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
    "outputTokens": 12,
    "cost": {
      "jev": {"amount": null, "currency": "USD", "priceVersion": "provider-usage", "status": "unknown", "basis": "estimated"},
      "codex": {"amount": null, "currency": null, "priceVersion": null, "status": "unavailable", "basis": "estimated"},
      "total": {"amount": null, "currency": "USD", "priceVersion": "provider-usage", "status": "unknown", "basis": "estimated"}
    }
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

## 意味レビュー・route・fix

`hooks review` はprompt/plan/diff/final-answer/turnを対象に、日本語clarity、文章矛盾、コード矛盾、コメントと実装の乖離を観測する。既定では本文をJevへ送らず、redact済みローカル検出だけを実行して`degraded`を返す。Jevへ送るには利用者が`--allow-content`を明示する必要がある。raw Tool result、API key、資格情報は常に対象外。

```bash
printf '%s\n' '{"hook_event_name":"UserPromptSubmit","prompt":"適宜対応して","taskId":"task-demo","sessionId":"session-demo","turnId":"turn-demo"}' \\
  | jevx hooks review --target prompt

# 選択した本文をredactしてJevへ送る明示opt-in
printf '%s\n' '{"hook_event_name":"PostToolUse","diff":"// always returns true\\nreturn false;"}' \\
  | jevx hooks review --target diff --allow-content

# fixは明示確認が必要。期待SHA-256とsafe pathもコード側で検証する
printf '%s\n' '{"diff":"return false;","fixes":[]}' \\
  | jevx hooks review --target diff --allow-content --auto-fix --yes

jevx hooks review-stats --input "${JEVX_HOME:-$HOME/.jevx}/reviews.jsonl" --json
```

Reviewの出力は`continue`/`suppressOutput`と`review`を持つ。`review.schemaVersion`は2、receiptには本文を含めず、request/content digest・文字数、finding、route、latency、calls/retry/cache、Token、fallback、cost、fix status、replay IDを保存する。`route.status`は適用証拠が完全一致したときだけ`applied`で、それ以外は`degraded`/`failed`。`codexUsage`が入力されればmodel、reasoning、main/subagent、通常Token、昇格追加Token、elapsed、fallback、通常cost、`additionalCost`もreceiptへ残る。

`review-stats`はstatus、finding、p50/p95、Jev/Codex/total、通常Token・昇格追加Token、fallback追加費用、推定/実費の元となるcost component、成功review/fix単価、task/turn/sessionのSHA-256単位を集計する。金額が取得できない行は`unknown`/`unavailable`の件数として残り、価格版・通貨が混在した合計を作らず、0にも変換しない。

`hooks install`でreview handlerを登録する場合は、まず通常のreview-onlyを使う。本文をJevへ送る設定を生成する場合だけ、次のように明示する。

```bash
jevx hooks install --scope user --dry-run --allow-content --json
jevx hooks install --scope user --allow-content --json
```

`--allow-content`はCodexのTool結果や会話全文の送信許可ではなく、Hook payloadの選択対象だけの許可。生成されたHookは`/hooks`でreview/trustし、不要になれば`hooks uninstall`でjevx管理handlerだけを外す。

## Jev Gatewayへ直接 `curl` する

通常は `jevx` を使えばよいけれど、Jevのリクエストとレスポンス形状だけを確認したいときは、同じGatewayへ直接送れる。これは外部APIへ実際に送信するコマンドなので、秘密情報ではないテスト文だけで実行してね。認証ヘッダーはprocess substitutionの一時config fdへ渡し、APIキーをcurlのargvへ展開しない形式にしている。

```bash
curl --fail-with-body --silent --show-error \
  --config <(
    printf '%s\n' \
      'request = POST' \
      "url = ${JEVX_GATEWAY_ENDPOINT:-https://ai-gateway.vercel.sh/v1/evaluate}" \
      "header = Authorization: Bearer ${AI_GATEWAY_API_KEY}" \
      'header = Content-Type: application/json'
  ) \
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

既定では `$JEVX_HOME/events.jsonl` と `$JEVX_HOME/decisions.jsonl`（通常は `~/.jevx/` 配下）へ追記する。前者は既存Telemetry、後者はDecision Contractのreceiptで、依頼文そのものではなく、state digest、契約版、判定、fallback、retry、cache、latency、Token使用量、Jev/Codex/total costなどの安全なメタデータを保存する。Gatewayの生response本文は保存しない。費用は`unknown`/`unavailable`を0へ変換せず、`basis`とprice version/currencyを残す。

receiptは `DecisionReceipt` / `JsonlDecisionRecorder` と `read_decision_receipts` / `replay_receipt` からRust APIとして扱える。`jevx/evals/decision-contract-baseline.jsonl` は秘密情報なしのaccepted/none比較用fixtureで、同じcontract・state digest・recorded answerを使えばJevなしにcode-side decisionを再評価できる。receiptの不一致は`replay_mismatch` / `degraded`となり、成功へ変換しない。

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

`--no-telemetry` はローカルの記録だけを止めるオプションで、Jevへの送信自体は止めないよ。Gatewayへ外部送信しない確認には `eval --dry-run --json`、`skills list`、`doctor`、または明示指定の `skills suggest --prompt "<non-secret prompt>" --skill <skill-id> --json` を使ってね。`--dry-run` は `eval` / `eval-repeat` のサブコマンドであり、単独の安全スイッチではない。

## ローカルデータの確認・書き出し・削除（`data`）

jevxがローカルに書くデータは、すべて `$JEVX_HOME`（既定 `~/.jevx`）の下にある。

| kind | パス | 書き込むコマンド |
| --- | --- | --- |
| hookDedupe | hook-dedupe.json | UserPromptSubmitの正常な判定結果の短期再利用 |
| hookDedupeLock | hook-dedupe.lock | UserPromptSubmitのclaim・完了書き込みの直列化 |
| hookDedupeTemps | .hook-dedupe-tmp/ | 原子的置換に使う一時ファイルのmetadata。クラッシュ残骸も管理対象 |
| compactionLock | `compaction/.compact-assist.lock` | Compaction checkpoint・recordの追記を直列化する安定lock |
| `telemetry` | `events.jsonl` | `skills suggest`（Telemetry有効時） |
| `hookRecords` | `hooks.jsonl` | `hooks install` で登録した `UserPromptSubmit` Hook |
| `compactionRecords` | `compaction/hook-records.jsonl` | `hooks compact-assist` |
| `compactionCheckpoints` | `compaction/checkpoints.jsonl` | `hooks compact-assist` |
| `decisionReceipts` | `decisions.jsonl` | `skills suggest` / `hooks shadow`（Decision Contractのreceipt。Telemetry有効時） |
| `reviewReceipts` | `reviews.jsonl` | `hooks review`（本文なしの意味レビュー、route、fix、費用receipt） |

```bash
jevx data path            # 場所・サイズ・レコード数（--jsonあり）
jevx data export > jevx-data.json   # 全レコードを1つのJSONで書き出す（--output FILEも可）
jevx data purge           # 削除対象を表示するだけ
jevx data purge --yes     # 実際に削除する
```

`purge` は上の10管理対象を確認し、削除対象がある場合はdedupe→compactionの安定lockを両方保持してから現行inventoryを再列挙し、hook-dedupe.json、クラッシュ残骸のtempファイル、通常のmanaged JSONLを削除する。実purgeでは空になったdedupe temp directoryも片付ける。削除対象がないconfirmed purgeはlockやdata directoryを新規作成せず、既存temp directoryにも触れない。`hook-dedupe.lock`と`compaction/.compact-assist.lock`自体は消さない。managed fileまたは`.hook-dedupe-tmp` directory自体のsymlinkはリンク先を読まず一覧に残し、exportは明示エラー、purgeはリンク自体だけを削除する。実directoryのtemp entryはno-follow FD相対で列挙・削除し、Compaction purgeもlock対象directoryのidentityを検証して保持FDから相対削除する。利用者が作る `.jevx/compact-context.md`、Codexの `hooks.json` と `hooks.json.jevx.bak`、`--output` で指定した評価レポートには触れない。`export` は壊れた行があると、その内容を表示せずにファイル名と行番号だけを返し、dedupe tempは内容を出さずpath/bytes metadataだけを返す。inventory/export/purgeのdata schemaはv7。保存契約の詳細は[費用観測契約](jevx-cost-observability.md)を参照してね。

## Codex Skillとしてセットアップする

`jevx/skill/SKILL.md` は、Codexへ「必要なら jevx に現在の依頼だけを渡して提案を確認する」ための advisory Skill。Skill本文を自動ロードしたり、jevxの結果だけでコマンド実行を許可したりしない。

### ユーザー領域へインストール

```bash
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user
```

インストール先は `${HOME}/.agents/skills/jevx/SKILL.md`。スクリプトは `JEVX_INSTALL_ROOT`、`CARGO_INSTALL_ROOT`、`CARGO_HOME`、`${HOME}/.cargo` の順でCargo install rootを決め、`cargo install --root`へ渡す。`--hooks` 付きならそのrootのバイナリをcommandへ絶対パスで登録するので、PATHへ追加しなくてもCodexから実行できる。単体で `jevx` を呼ぶ場合は、上のように同じrootの `bin` ディレクトリを `PATH` に追加してね。

Skillのセットアップと同時に、Codex Hookも明示的に登録したい場合は `--hooks` を付ける。Hook設定の生成・Trust確認が発生するので、初回は `--dry-run` 付きのコマンドを先に実行するのがおすすめ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope user --dry-run --json
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user --hooks
```

### このプロジェクトだけへインストール

```bash
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope project --repo "$PWD"
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

`hooks shadow` は stdin のHook JSONを読み、known eventを受理した場合だけCodexへ固定の継続レスポンスを返す。未知event、event不一致、JSON不正には固定応答を返さずエラーにする。

```json
{"continue":true,"suppressOutput":true}
```

対応イベントは `SessionStart`、`PreCompact`、`PostCompact`、`UserPromptSubmit`。`UserPromptSubmit` のときだけ、promptが空でなく、Jev判定が可能ならSkill候補を評価する。Jevが失敗してもknown eventの `continue: true` は維持する。

```bash
printf '%s\n' \
  '{"hook_event_name":"UserPromptSubmit","session_id":"sample-session","turn_id":"sample-turn","model":"sample-model","prompt":"PDFを結合したい","cwd":"/tmp/sample-repo"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow \
      --event UserPromptSubmit \
      --skill-dir jevx/evals/skills \
      --output /tmp/jevx-hooks.jsonl
```

`--output` へ保存されるHook recordはschema v6。hashed IDsとprompt非保存を含む現行schemaの観測記録だよ。任意の`codexUsage`/`codex`/Responses API形式の`usage` payloadがあればmodel、reasoning、通常/昇格Token、fallback、costも残し、nested detailsのcached/cache-write/reasoning Tokenもtyped fieldへ正規化する。`cost`にはJev/Codex/totalとestimated/actual、通貨・価格版を記録する。取得不能な費用は`unknown`/`unavailable`のままだよ。明示的な`totalTokens: 0`も測定値として保持し、`compactionElapsedMs`と`tokenSavings`の数値整合性を検証する。既知のinput/output Tokenが両方ある場合はtotalとの一致も検証する。`trigger`、`source`、`selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなrecordを拒否し、既存schema v1〜v5のloadでは該当metadataと旧usage presenceを移行して分析互換性を保つよ。詳しい保存・費用・dedupe契約は[費用観測契約](jevx-cost-observability.md)を参照してね。

- `sessionIdSha256`、`turnIdSha256`、`modelSha256`、`correlationIdSha256`
- `promptSha256`、`promptChars`
- `decision`、`selectedSkill`
- `discoveryMs`、`jevResponseMs`、`totalMs`、Token使用量、`errorCode`
- `dedupeHit`、`dedupeKeySha256`、`dedupeError`、`preCompactionUsage`、`compactionUsage`、`postCompactionUsage`。dedupe keyはcwd・候補Skill・判定設定のdigestも含み、hit時に元呼び出しのlatency/Tokenを複製しない。状態障害は主`errorCode`を上書きせず、`dedupeError`として残す
- `postCompactionCost`、`compactionElapsedMs`、`tokenSavings`（before/after/saved/reduction/status）

生のセッションID・ターンID・モデル名・prompt本文は保存しない。Jevを使わないイベントでも、Hookの継続性を確認できる。

### Hookを外す（`hooks uninstall`）

`hooks install` で登録したhandler（`--jevx-managed` 付き、または `jevx hooks shadow|compact-assist` を呼ぶcommand）だけを取り除き、他のHookと設定はそのまま残す。取り除いた結果、空になったgroupとイベントも片付ける。

```bash
jevx hooks uninstall --scope user --dry-run --json   # 取り除くhandler数と結果の設定を確認
jevx hooks uninstall --scope user
jevx hooks uninstall --scope project --repo "$PWD"
```

uninstallはbackupを作らない。取り除くのはjevxのhandlerだけで、jevx入りの設定をbackupにすると、そこから戻したときにHookが復活してしまうため。installが最初に残した `hooks.json.jevx.bak` はそのまま残る。`hooks.json` がない場合や、jevxのhandlerがない場合は何も変更せずに終了する。

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
| `PostCompact` | `manual\|auto` | compact後のmetadataを記録 + diff review | `--allow-content`時だけ対象本文 |
| `UserPromptSubmit` | なし | Skill選択をshadow観測 + prompt review | `--allow-content`時だけ対象本文 |
| `Stop` | なし | final-answer review | `--allow-content`時だけ対象本文 |

変更後はCodexで `/hooks` を開き、対象Hookをreview/trustしてから有効化してね。Codexのproject-local HookはプロジェクトTrustの影響を受けるため、設定ファイルを書けたこととHookが発火することは別に確認する。reviewの本文送信を許可しない場合は`--allow-content`なしでinstallする。

### Compaction補助の試作

`hooks compact-assist` はHook JSONをstdinから読み、`<state-dir>/hook-records.jsonl` と `checkpoints.jsonl` に安全なmetadataだけを追記する。Hook recordはschema v6、checkpointはschema v4。直接のHook payloadにbefore/afterが同時にあっても別リクエストの可能性があるため`tokenSavings.status=unavailable`とし、未消費の同一session・turn・correlation cycleのPreCompact checkpointとPostCompactを対応付けられた場合だけtokenSavingsをMeasuredにする。PreCompact/PostCompactが交互に進む場合も同じcycleだけを消費し、壊れたcheckpoint行は明示的なエラーにする。Hook recordの `trigger` / `source` / `selectedSkill` はwrite前にbounded identifierへ正規化され、unsafeな値は欠損として扱われる。新規appendはunsafeなrecordを拒否し、既存schema v1〜v5のloadでは任意metadataと旧usage presenceを正規化・欠損化して過去ログを読み続ける。checkpoint schema v1〜v3もusageの欠落値を保ったまま移行する。詳しい契約は[費用観測契約](jevx-cost-observability.md)を参照してね。`SessionStart(source=compact)`のときは、同じセッションのcheckpointと、作業ディレクトリに任意で置いた `.jevx/compact-context.md` をredactして `additionalContext` に返す。

```bash
mkdir -p .jevx
printf '%s\n' 'goal: preserve the release checklist' 'next: run tests' \
  > .jevx/compact-context.md

printf '%s\n' \
  '{"hook_event_name":"SessionStart","source":"compact","session_id":"sample-session","cwd":"'"$PWD"'"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir /tmp/jevx-compaction
```

checkpointには生のmanifest本文を保存せず、redacted本文のSHA-256、文字数、event名、相関ハッシュだけを保存する。Hook metadataもwrite前にbounded identifierへ正規化し、unsafeな値を保存しないよ。`additionalContext`も「補助情報」として返すだけで、Codexの会話履歴・現在のリポジトリ確認・公式Compactionの代替ではないよ。

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

### Hookの速度・費用・Token削減量を集計する

hooks statsは保存済みHook recordから観測値だけを集計する。標準Hook payloadにusage/costがなければ、削減量・費用はnullまたはstatus countのままになる。

~~~bash
jevx hooks stats \
  --input "$JEVX_HOME/hooks.jsonl" \
  --json \
  --output /tmp/jevx-hook-stats.json
~~~

JSONにはevent別件数、decision/error、UserPromptSubmitの全体およびdedupe hitのp50/p95、dedupe率、usage measured件数、before/after/saved Token、reduction rate、Jev/Codex/totalの費用合算とcost statusを含める。`latencyMsP50/P95`はUserPromptSubmitだけを対象にし、SessionStart/PreCompact/PostCompactの処理時間を混ぜない。費用が混在・欠落して安全に合算できない場合は金額をnullにする。

## Compactionの評価

### 合成ベースライン

APIキーやネットワークなしで、固定の合成シナリオを繰り返せる。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks compact-eval --runs 5 --json \
  --output /tmp/jevx-compact-baseline.json
```

これは実Codexを起動するコマンドではなく、jevxの評価器と集計ロジックを検証するための `mode: shadow` のベースラインだよ。Compaction評価レポートはHook schema v6としてToken削減量も含める。

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

レポートにはケースID、モデル、件数、保持率、デコイ漏えい件数、完了フラグ、遅延、Token集計、任意の`codexUsage`（reasoning、main/subagent、通常Token、昇格追加Token、elapsed、fallback、cost）が残り、`followUpText`、`requiredFacts`、`secretMarkers` の本文は再保存しない。`fallbackExtraCost`は取得できた同一価格版・通貨の追加費用だけを示し、欠落時は`null`のままになる。

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

40ケースのfixture（合成30件、匿名化テンプレート10件）は [`jevx/evals/README.md`](../../jevx/evals/README.md) に説明があるよ。

fixtureの `keywords` は期待根拠の注釈として読み込まれるが、現行 `local_keyword` の予測計算には使われない。現行方式はSkillのID・name・descriptionから一致を作り、`id`・`kind`・`expected`・`keywords` はJev requestへ含めない。単回`eval --output`では`expected`を判定基準としてcase outputへ保存するが、`keywords`は保存しない。

### APIキーなしのdry-run

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

`none`、`local_keyword`、`local_rank`、Jev（`not_run`）を同じfixtureで比較する。現行report schema 2は`baselineMode: "local_rank"`と`comparisons`を持ち、各modeについて`totalMsDelta`、`speedupRate`、`additionalCost`を出す。`noneRecall` は `expectedNone` 分母の recall（`noneCorrect / expectedNone`）。`nonePrecision` は同じ値を持つ互換キーで、名前と意味がずれているため新しい集計では `noneRecall` を使ってね。

既定の `--fixtures jevx/evals/skill-selection.jsonl` と `--skill-dir jevx/evals/skills` はリポジトリルートからの相対パス。見つからない場合は、黙って空の評価をせずに終了コード2で指定方法を表示する。`eval --dry-run`ではJev/Gatewayへ外部送信しない。

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

レポートでは正解率・`none` recall（互換キー `nonePrecision`）・候補miss・fallback率・cache hit率・retry率・エラー率と、`discoveryMs`・`jevResponseMs`・`totalMs`・相対cost・Jev/Codex/total monetary costの mean / p50 / p95 などを分けて確認できる。`comparisons`にはbaselineからの速度短縮率と追加費用が入り、いずれかが取得不能なら`null`のまま残る。単回`eval --output`のcase JSONLには`id`・`kind`・`expected`・判定・metrics・error codeを保存し、prompt本文・fixtureの`keywords`・Jev response本文を保存しない。`eval-repeat --output`はcase JSONLではなくrun/mode集計だけを保存する。

### Current / Latest baseline（2026-09-23）

`JEVX_TELEMETRY=off` の外部送信なし確認（`eval --dry-run --json`）の結果。測定時の実装 commit `391e18049fe89a0dadcb8d43f6b70e3c6673ddf1`、fixture SHA-256 `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb`、`rustc/cargo 1.98.1`、macOS 26.6.2（build 25G83）、APIキーなし、カタログは `jevx/evals/skills` の9件だけ。実行日時 `2026-09-23T22:11:53+09:00`。

| モード | cases | accuracy | `noneRecall`（互換キー `nonePrecision`） | 外部通信 |
| --- | ---: | ---: | ---: | --- |
| `none` | 40 | 10.0% | 100.0%（4/4） | なし |
| `local_keyword` | 40 | 100.0% | 100.0%（4/4） | なし |
| `local_rank` | 40 | 20.0% | 75.0%（3/4） | なし |
| `jevx` | 0 | — | — | `not_run` |

2026-09-22に記録した値（`local_keyword` 87.5%、`local_rank` 17.5%）は、既定のproject・HOME・CODEX_HOMEのSkill rootが評価カタログに混ざった環境依存の値だった。現行の `eval` は `--skill-dir` だけを使うため、その値はHistoricalとして扱う。

### Historical snapshot（2026-09-21、APIキーあり）

同梱40ケースの固定fixtureで、導入効果と追加コストを記録時点の条件で確認した結果だよ。これは2026-09-21のHistorical snapshotで、APIキーあり、当時の実装commitに対する値であり、現行Latestの保証ではない。

| snapshot metadata | 値 |
| --- | --- |
| 日付 | 2026-09-21 |
| API key | `AI_GATEWAY_API_KEY` 設定済み（値は記録・表示しない） |
| commit状態 | 当時の実装commit（現行 `ed0b93c...` とは別。詳細は各Historicalレポート） |

| 方式 | 正解率 | `nonePrecision`（`expectedNone` 分母の recall） | 追加コスト |
| --- | ---: | ---: | --- |
| `none` | 10.0% | 100.0% | 外部通信なし |
| `local_keyword` | 90.0% | 75.0% | 外部通信なし |
| `jevx`（1回） | 100.0% | 100.0% | Jev p50/p95 407/673ms、平均 2,401/128 tokens |
| `jevx`（5回平均） | 98.0% | 100.0% | Jev p50/p95 427/610ms、エラー率2.0% |

5回測定ではローカル候補探索p95が2ms、全体p95が612msだった。今回のfixtureではJevの追加判断がlocal_keywordより良かった一方、Jevの応答時間・Gatewayエラー・Token使用量は常に発生するため、実運用では `stats` のp95とエラー率を一緒に見る。測定条件と「保証ではない」範囲は [`../research/evaluations/jevx-comfort-evaluation.md`](../research/evaluations/jevx-comfort-evaluation.md) にまとめているよ。

## セキュリティとデータの扱い

- Jevへ送るのは、通常のSkill提案ではマスキングしたprompt、redactした作業ディレクトリ、rawの候補Skill ID/name、redactしたdescription。reviewで`--allow-content`を明示した場合だけ、対象として選択したprompt/plan/diff/final answerをredactして送る。Skill本文、設定本文、過去会話全文、raw Tool結果、APIキーは常に送らない。`hooks install`でUserPromptSubmit shadowを登録すると、APIキー設定時はredact済みpromptがreviewのopt-inとは独立して送信される。
- `AI_GATEWAY_API_KEY` は環境変数からBearer認証へ使い、レスポンスやTelemetryへ書き出さない。
- Telemetryはprompt本文やprobabilityではなくハッシュ・文字数・判定・選択時の`selectedSkill`（raw ID）・計測値を保存するため、Skill ID自体を秘密値にしない。
- Basic redactionは `Authorization=Basic <value>` / `Authorization:Basic <value>` / `Authorization: Basic <value>` の認識済み形式で値を保存・送信しない。任意の `Basic` 文言や未知のPIIを除去する完全なDLPではない。
- Hookの`trigger` / `source` / `selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなrecordを拒否し、既存schema v1〜v5のloadでは該当metadataとusage presenceを正規化・欠損化して分析互換性を保つ。
- Hook recordはセッションID、ターンID、モデルをSHA-256化して保存する。
- `/tmp` の会話評価fixtureやCodexのrollout・認証ファイルは、評価後に削除する運用にする。
- `--no-telemetry` はローカル保存を止めるだけ。外部送信も止めたいときは、Jevを呼ぶコマンドを実行しない。
- 直接 `curl` では入力した `state` が送信されるため、秘密情報を含めない。

## トラブルシューティング

### `missing_api_key`

`AI_GATEWAY_API_KEY` が未設定。`doctor --json` で `apiKeyConfigured` を確認してね。明示SkillならAPIキーなしで動作する。

### `timeout` / `provider_error`

Gatewayへの接続、HTTPステータス、JSON形状を確認する。`JEVX_REQUEST_TIMEOUT_MS` は既定1,500msで、リトライを含めた呼び出し全体の予算。`jevResponseMs` はアプリ側のHTTP往復時間。

### `no_candidates`

探索対象に有効な `SKILL.md` がない状態。Frontmatterの `name` と `description` を確認し、必要なら `--skill-dir` を指定する。

### `evaluation inputs not found`

`eval` / `eval-repeat` をリポジトリの外で実行した。リポジトリルートへ移動するか、`--fixtures <FILE>` と `--skill-dir <DIR>` を指定する。

### 閾値の警告が出る

`doctor` の `warnings` に出た環境変数が範囲外か数値ではない。値を直すか、環境変数を外して既定値に戻す。

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
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --all-targets \
  --ignore-filename-regex 'src/(cli/.*|compact_assist|decision|discovery|error|gateway|hook_config|ranking|redaction|storage|telemetry|types)\.rs' \
  --summary-only --fail-under-lines 98
```

公開している `--json` のキーと終了コードは `jevx/tests/contract_requirements.rs` が実バイナリで固定している。キーを消す・名前を変えるときは `schemaVersion` を上げ、[互換性の約束](../../jevx/PHILOSOPHY.md#互換性の約束)に従う。

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

- [`docs/README.md`](../README.md): 現行jevx文書の入口
- [`jevx-roadmap.md`](jevx-roadmap.md): 今後の候補と採用基準
- [`jevx/skill/SKILL.md`](../../jevx/skill/SKILL.md): Codexへ登録するadvisory Skill
- [`jevx/evals/README.md`](../../jevx/evals/README.md): 40ケース評価fixtureの仕様
- [`jevx-requirements.md`](jevx-requirements.md): 要件定義
- [`jevx-architecture.md`](jevx-architecture.md): アーキテクチャと安全側判定
- [`jevx-evaluation.md`](jevx-evaluation.md): 評価Runnerの仕様
- [`../research/evaluations/jevx-codex-hooks-evaluation.md`](../research/evaluations/jevx-codex-hooks-evaluation.md): Hook shadow評価
- [`../research/evaluations/jevx-comfort-evaluation.md`](../research/evaluations/jevx-comfort-evaluation.md): Jevあり/なし比較と導入判断
- [`../research/evaluations/jevx-real-codex-compaction-evaluation-2026-09-21.md`](../research/evaluations/jevx-real-codex-compaction-evaluation-2026-09-21.md): 実Codex / 実会話型Compaction評価
- [`../research/evaluations/jevx-compaction-depth-evaluation-2026-09-21.md`](../research/evaluations/jevx-compaction-depth-evaluation-2026-09-21.md): Hook直接検証とCompaction深掘り評価
- [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)
- [OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)
