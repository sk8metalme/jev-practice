# jevx

`jevx` は、Codex CLI の作業を邪魔せずに Jev の有用性と速度を測るための、macOS 向け Rust CLI だよ。
Skill 選択のコアは **Shadow Mode**。依頼に合いそうな Skill を提案するだけで、Skill 本文の自動ロード・実行や会話の書き換えはしない。Hook と Compaction 補助は、利用者が明示的に登録したときだけ動くオプトイン機能だよ。

## ディレクトリ構成

実利用用のSkillは `skill/`、検証・評価用のfixtureとSkillカタログは `evals/` に分かれています。普段のCodexへ導入するものと、jevx自体を測定するためのものを混ぜない構成です。

```text
jevx/
├── skill/
│   └── SKILL.md               # 実利用用：Codexへ導入するjevx Skill
├── evals/
│   ├── README.md              # 検証手順・fixtureの説明
│   ├── skill-selection.jsonl  # 検証用fixture
│   └── skills/                # 検証専用のSkillカタログ
├── src/                       # jevx CLI本体
├── scripts/
│   └── setup.sh               # 実利用環境へのセットアップ
├── tests/                     # 自動テスト
├── Cargo.toml
└── README.md
```

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
- `hooks compact-assist`: Compaction前後のcheckpointを記録し、compact後に補助contextを返す
- `hooks correlate`: Hook の相関IDと重複を集計する
- `hooks compact-eval`: 合成 Compaction のベースラインを測る
- `hooks conversation-eval`: controlled fixtureから作ったJSONLを評価する
- `eval` / `eval-repeat`: Skill 選択の品質・速度・分散を測る
- `doctor` / `stats`: 設定と Telemetry を確認する

### v1 の境界

`jevx` の提案結果は、Codex が通常の Skill 選択ルールに従うための補助情報だよ。`selected` になっても Skill を自動で読み込んだり、コマンドを実行したりしない。Jev の結果を実行権限へ直結させないことが、このツールの安全上の前提。

`hooks install` は設定ファイルを変更するため、既定では何も実行しない。`--dry-run`で内容を確認してから明示的に実行し、CodexのHook Trustを確認して使う。`compact-assist`も会話を要約するLLMではなく、redactedなローカルmanifestとハッシュを復元する決定的な試作だよ。

## Jevを使う設計理念

複数の実装事例を比較すると、Jevを特別な処理の代わりにするのではなく、**決定的なローカルコードと意味判定の境界を狭く保つ**ことが品質・安全性・運用性を作っていた。jevxでは次の原則を共通の設計契約として扱うよ。

### 実装事例から抽出したパターン

| 観察した設計パターン | 取り入れる観点 | そのまま移植しないもの |
| --- | --- | --- |
| 意味的なレビュー | matcherで対象を絞り、意味的な一文のpredicate・fixture・accepted baseline・warning-firstで運用する | parser / type checkerで確定できる欠陥のJev化、いきなりCIをblockingする運用 |
| 状態分割と予算管理 | stateをwindow化し、overlap・予算・cache・replay・costを明示する | 決定的なsyntax highlightingをJevへ置き換えること |
| 変更影響のScore評価 | `Score`で影響度を表し、uncertain / missing / dynamicを安全側で選び、runnerのexit codeを保持する | framework固有のrunner差分をjevxの共通契約へ混ぜること |
| typed policyとreplay | typed answer、atomic predicate、policy gate、transcript / replay、`defer`を一級の結果にする | Jevの確信度を権限やauto-allowへ直結させること |

ここでの比較結果は、jevxの全機能を一度に増やす指示ではなく、共通基盤を先に整えてから小さな縦機能を評価するための設計材料だよ。

### 1. Jevは意味判定、コードは決定的な仕事

- 構文解析、差分抽出、対象列挙、テスト発見、window分割、redaction、コマンド実行はローカルコードで行う。
- Jevには「この候補が依頼の文脈に合うか」「この変更がこの対象へどの程度影響するか」のように、コードだけでは書きにくい意味的・文脈的な問いを渡す。
- 決定的なparserや型検査で確定できる問題をJevへ移さない。Jevを使う必然性を、縦機能ごとに説明できる状態にする。

### 2. Jevの回答は推薦であり、最終決定ではない

Jevの回答は自由文ではなく、用途に合ったtyped answerとして扱う。現在のSkill選択は`Choice`を使うが、将来の機能では次の型を使い分ける設計候補がある。

| answer type | 向いている問い | 最終判断 |
| --- | --- | --- |
| `Choice` | 候補Skillから最も合うもの、または`none` | コード側の確信度・margin gate |
| ordered `Score` | 変更が各テストへ与える影響の大きさ | コード側のcutoff・unsure判定 |
| atomic predicate / `Noul` | 条件の成立、policyの構成要素 | コード側の保守的なcomposition |

Jevの確信度は「与えられたstateのもとでの判定の確からしさ」であり、操作が成功することや権限を与えてよいことの証明ではない。`selected`になってもSkillの自動ロード・実行や権限付与へ直結させない。

### 3. Decision Contractで問い・結果・安全側挙動を束ねる

Skill選択の現行実装は、個別のJev呼び出しを増やさず、次のDecision Contractへ寄せている。新しい専用CLIコマンドは増やさず、Rust APIと既存の評価Runnerから利用する。

```text
State Builder → redaction / window plan → typed Jev answer
            → code-side threshold / margin / policy gate
            → accepted / none / unknown / defer / degraded
            → safe receipt + replayable evaluation
```

receiptには、少なくとも次の安全なメタデータを持たせる。prompt本文・Skill本文・Tool結果・APIキー・生のセッション識別子は含めない。

- contract / question / policyのバージョン
- state digest、候補数、window数、omitted / redactionの理由
- 構造化されたanswer、code-side decision、threshold、margin、fallback、reason
- calls、retry、latency、input / output tokens、相対的なcost、replay ID

この形式にすると、同じfixtureとrecorded answerでJevなしにcode-side decisionを再生でき、質問や閾値を変更したときの差分もレビューできる。

### 4. 不確実性・状態不足・障害は一級の結果にする

`unknown`、`defer`、`degraded`、`no answer`、`timeout`、`outage`を、成功や自動allowへ変換しない。特に権限Hookや破壊的操作は、必要なら`ask`または`defer`へ流し、fail-closedの挙動を優先する。

状態をtoken / byte / 候補数の上限で分割するときは、window、overlap、omitted項目、redaction理由、予算超過を可視化する。大きな入力を黙って切り捨てたまま、高い確信度だけを信頼しない。

### 5. 評価・再現性・コストを機能の一部にする

新しいJev利用は、dry-run、fixture、accepted baseline、replay、必要に応じたcacheを用意してから運用へ進める。少なくとも次を同じ入力・候補・ネットワーク条件で比較する。

- Top-1 accuracy、`none` precision、candidate miss、uncertain / fallback率
- 誤って危険側へ進まない率、誤検出率、反復時のanswer / confidence variance
- Jev p50 / p95、全体p50 / p95、token、相対cost、retry、error rate

Jevありの単発精度だけで導入を決めず、Jevなしのbaseline、遅延、費用、失敗時の挙動を一緒に見る。既存評価の実行方法は [Skill選択の評価Runner](#skill選択の評価runner) を参照してね。

### 実装知見からのjevxバックログ

次の順番は、共通基盤を先にしてから縦機能を増やす提案だよ。未実装の項目は設計候補であり、機能の存在を表さない。

| 優先度 | 候補 | 目的 | 受け入れの観点 |
| --- | --- | --- | --- |
| 完了 | Decision Contract & Recorder | `Choice` / `Score` / predicate、code-side gate、fallback、receipt、replayを共通化 | recorded answerで同じdecisionを再現し、timeout / outageが自動allowにならない |
| P1 | Skill Calibration Packs | Skillごとの代表例、none、境界例、期待answer、閾値、反復分散を評価 | 新Skillをpackなしで本番相当評価へ進めず、miss / none / p95を比較できる |
| P1 | State Window Planner | token / byte / 候補予算、window、overlap、omitted、degradedを管理 | 同じ入力・予算・版で同じwindowを生成し、候補漏れと遅延を測れる |
| P2 | Diff Impact / Semantic Review | 変更影響のScoreや意味的矛盾をShadow Modeで提示 | uncertain / missingを安全側で選択し、CI blockingは評価後に判断する |
| P2 | Policy Hook Gate | atomic predicateと`defer`を権限Hookへ適用する研究 | 脅威モデル・監査ログ・fail-closed検証が済むまで自動allowしない |
| P2 | Jev Lab / DSL | transcript、replay、cost、answer、decisionを比較する開発者体験 | 共通契約の重複が実証されてからDSL導入を判断する |

新機能の設計レビューでは、「Jevでなければ解きにくいか」「最終allow / executeをJevが直接決めていないか」「unknown / timeout / outageはどこへ流れるか」「同じdecisionを再現できるか」「秘密情報が境界外へ出ていないか」を必ず確認する。

## クイックスタート

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
| `JEVX_REQUEST_TIMEOUT_MS` | `1500` | Jev HTTPリクエストのタイムアウト |
| `JEVX_HOME` | `$HOME/.jevx` | Telemetry保存先の親ディレクトリ |
| `JEVX_TELEMETRY` | 有効 | `0` または `off` でTelemetryを無効化 |
| `JEVX_MAX_STATE_BYTES` | `32000` | 送信stateのbyte上限。超過時はomittedを記録してdegraded |
| `JEVX_MAX_RETRIES` | `1` | 429/529に対する最大retry回数 |
| `JEVX_RETRY_BACKOFF_MS` | `25` | retry backoffの基準ミリ秒 |
| `JEVX_DECISION_CACHE_CAPACITY` | `0` | process-local answer cacheの上限。0は無効 |
| `JEVX_INPUT_COST_WEIGHT` / `JEVX_OUTPUT_COST_WEIGHT` | `1.0` / `1.0` | token proxyの相対cost重み。通貨ではない |

設定を確認するには、APIキーの値そのものを表示しない `doctor` を使うよ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- doctor --json
```

### Skillを一覧表示する

探索の優先順位は、プロジェクトの `.agents/skills`、`.codex/skills`、ユーザーの `~/.agents/skills`、`$CODEX_HOME/skills`（`CODEX_HOME`未設定時は`~/.codex/skills`）、`--skill-dir` の順だよ。同じ名前のSkillは、優先順位が高いルートを採用する。`--skill-dir` は既定rootsの置換ではなく追加なので、評価では一時HOME・CODEX_HOME・空のproject rootを使ってカタログを隔離する。

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

既定では `$JEVX_HOME/events.jsonl` と `$JEVX_HOME/decisions.jsonl`（通常は `~/.jevx/` 配下）へ追記する。前者は既存Telemetry、後者はDecision Contractのreceiptで、依頼文そのものではなく、state digest、契約版、判定、fallback、retry、latency、Token使用量、token proxy costなどの安全なメタデータを保存する。Gatewayの生response本文は保存しない。

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

`--output` へ保存されるHook recordは、hashed IDsとprompt非保存を含む現行schemaの観測記録だよ。`trigger`、`source`、`selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなrecordを拒否し、既存schema v1のloadでは該当metadataを正規化・欠損化して分析互換性を保つよ。

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

`hooks compact-assist` はHook JSONをstdinから読み、`<state-dir>/hook-records.jsonl` と `checkpoints.jsonl` に安全なmetadataだけを追記する。Hook recordの `trigger` / `source` / `selectedSkill` はwrite前にbounded identifierへ正規化され、unsafeな値は欠損として扱われる。新規appendはunsafeなrecordを拒否し、既存schema v1のloadでは任意metadataを正規化・欠損化して過去ログを読み続ける。`SessionStart(source=compact)`のときは、同じセッションのcheckpointと、作業ディレクトリに任意で置いた `.jevx/compact-context.md` をredactして `additionalContext` に返す。

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

fixtureの `keywords` は期待根拠の注釈として読み込まれるが、現行 `local_keyword` の予測計算には使われない。現行方式はSkillのID・name・descriptionから一致を作り、`id`・`kind`・`expected`・`keywords` はJev requestへ含めない。単回`eval --output`では`expected`を判定基準としてcase outputへ保存するが、`keywords`は保存しない。

### APIキーなしのdry-run

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

`none`、`local_keyword`、`local_rank`、Jev（`not_run`）を同じfixtureで比較する。`nonePrecision` は `expectedNone` 分母の recall（`noneCorrect / expectedNone`）であり、通常のprecisionとは異なる。`eval --dry-run`ではJev/Gatewayへ外部送信しない。

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

レポートでは正解率・`none` recall（互換キー `nonePrecision`）・候補miss・fallback率・cache hit率・retry率・エラー率と、`discoveryMs`・`jevResponseMs`・`totalMs`・相対costの mean / p50 / p95 などを分けて確認できる。単回`eval --output`のcase JSONLには`id`・`kind`・`expected`・判定・metrics・error codeを保存し、prompt本文・fixtureの`keywords`・Jev response本文を保存しない。`eval-repeat --output`はcase JSONLではなくrun/mode集計だけを保存する。

### Current / Latest baseline（2026-09-22）

`JEVX_TELEMETRY=off` の外部送信なし確認では、`none` 10.0%、`local_keyword` 87.5%、`local_rank` 17.5%、`jevx` は `not_run` だった。`nonePrecision` は `expectedNone` 分母の recall（`noneCorrect / expectedNone`）。測定時の実装・fixture commitは `ed0b93c023251400bcbbe2de8cdedd5451f08761`（docs同期前）、fixture SHA-256 `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb`、`rustc/cargo 1.98.1`、macOS 26.6.2（build 25G83）、APIキーなし、fixture9件 + 既定rootsの有効候補14件。実行日時 `2026-09-22T15:00:42+09:00` と環境依存性をレポートへ併記する。隔離rootのfixture-only実行では値が変わるため、Latest値と混ぜない。

| モード | cases | accuracy | none recall（互換キー `nonePrecision`） | 外部通信 |
| --- | ---: | ---: | ---: | --- |
| `none` | 40 | 10.0% | 100.0%（4/4） | なし |
| `local_keyword` | 40 | 87.5% | 75.0%（3/4） | なし |
| `local_rank` | 40 | 17.5% | 75.0%（3/4） | なし |
| `jevx` | 0 | — | — | `not_run` |

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

5回測定ではローカル候補探索p95が2ms、全体p95が612msだった。今回のfixtureではJevの追加判断がlocal_keywordより良かった一方、Jevの応答時間・Gatewayエラー・Token使用量は常に発生するため、実運用では `stats` のp95とエラー率を一緒に見る。測定条件と「保証ではない」範囲は [`../docs/research/evaluations/jevx-comfort-evaluation.md`](../docs/research/evaluations/jevx-comfort-evaluation.md) にまとめているよ。

## セキュリティとデータの扱い

- Jevへ送るのは、マスキングしたprompt、redactした作業ディレクトリ、rawの候補Skill ID/name、redactしたdescription。Skill本文、過去会話全文、Tool結果、APIキーは送らない。
- `AI_GATEWAY_API_KEY` は環境変数からBearer認証へ使い、レスポンスやTelemetryへ書き出さない。
- Telemetryはprompt本文やprobabilityではなくハッシュ・文字数・判定・選択時の`selectedSkill`（raw ID）・計測値を保存するため、Skill ID自体を秘密値にしない。
- Basic redactionは `Authorization=Basic <value>` / `Authorization:Basic <value>` / `Authorization: Basic <value>` の認識済み形式で値を保存・送信しない。任意の `Basic` 文言や未知のPIIを除去する完全なDLPではない。
- Hookの`trigger` / `source` / `selectedSkill`はwrite前にtrim・許可文字・最大長を検証し、unsafeな値は欠損化する。新規appendはunsafeなrecordを拒否し、既存schema v1のloadでは該当metadataを正規化・欠損化して分析互換性を保つ。
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

- [`../docs/README.md`](../docs/README.md): 現行jevx文書の入口
- [`skill/SKILL.md`](skill/SKILL.md): Codexへ登録するadvisory Skill
- [`evals/README.md`](evals/README.md): 40ケース評価fixtureの仕様
- [`../docs/developers/jevx-requirements.md`](../docs/developers/jevx-requirements.md): 要件定義
- [`../docs/developers/jevx-architecture.md`](../docs/developers/jevx-architecture.md): アーキテクチャと安全側判定
- [`../docs/developers/jevx-evaluation.md`](../docs/developers/jevx-evaluation.md): 評価Runnerの仕様
- [`../docs/research/evaluations/jevx-codex-hooks-evaluation.md`](../docs/research/evaluations/jevx-codex-hooks-evaluation.md): Hook shadow評価
- [`../docs/research/evaluations/jevx-comfort-evaluation.md`](../docs/research/evaluations/jevx-comfort-evaluation.md): Jevあり/なし比較と導入判断
- [`../docs/research/evaluations/jevx-real-codex-compaction-evaluation-2026-09-21.md`](../docs/research/evaluations/jevx-real-codex-compaction-evaluation-2026-09-21.md): 実Codex / 実会話型Compaction評価
- [`../docs/research/evaluations/jevx-compaction-depth-evaluation-2026-09-21.md`](../docs/research/evaluations/jevx-compaction-depth-evaluation-2026-09-21.md): Hook直接検証とCompaction深掘り評価
- [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)
- [OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)
