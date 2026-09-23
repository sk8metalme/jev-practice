# jevx

`jevx` は、Codex CLIの作業を止めずに、**どのSkillを使うべきかをJevに提案させて、その判断の良し悪しと費用を測る** macOS向けのRust CLIです。

## なぜ作ったか

Codex CLIのSkillが増えてくると、依頼ごとに合うSkillを探す時間が気になるようになりました。Jevで自動的に選べないか試したかったのですが、LLMの判定をそのまま実行に使うのは怖い。そこで、**提案するだけで実行はしない（Shadow Mode）**、**Jevあり／なしを同じ条件で測れる**、の2つから始めたのがjevxです。

判断の軸（原則・やらないこと・互換性の約束）は [PHILOSOPHY.md](PHILOSOPHY.md) にまとめています。

## 誰のためのものか

- **向いている人**: macOSでCodex CLIを使っていて、Skillを複数持ち、その選び方と効果を数字で確かめたい人
- **向いていない人**: Skillを自動でロード・実行してほしい人（Codex標準のSkill選択を使ってください）、会話を要約してほしい人（Codex公式のCompactionを使ってください）、macOS以外の環境の人

## 5分で最初の提案まで

必要なもの: macOS、Rust stable と Cargo。Jevを使うなら `AI_GATEWAY_API_KEY`。

```bash
# 1. リポジトリのルートでビルドし、Codex用のSkillを入れる（install rootは $HOME/.local に固定）
JEVX_INSTALL_ROOT="$HOME/.local" sh jevx/scripts/setup.sh --scope user
export PATH="$HOME/.local/bin:$PATH"

# 2. 足りないものと次の一手を確認する
jevx doctor

# 3. 依頼に合うSkillを提案させる
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
jevx skills suggest --prompt "PDFを結合して内容を確認したい"
```

```text
jevx skill suggestion
Selected: pdf
Probability: 0.91
Candidates: 3
Jev response: 124 ms
Total: 138 ms
Mode: shadow
```

APIキーがなくても、`jevx skills list`、`jevx doctor`、`jevx eval --dry-run`（リポジトリのルートで実行）は動きます。`doctor` の `Next steps` に、次に実行するコマンドが表示されます。

## 何をするか

```text
依頼文
  ├─ project / user のSkillを探す（ローカル）
  ├─ 名前と説明で順位付けし、最大32件に絞る（ローカル）
  ├─ 候補と none を1回のJev Choiceへ送る
  ├─ 確率 0.60 以上・次点との差 0.10 以上なら selected、足りなければ none（ローカル）
  └─ JSON / 人間向けに出力し、秘密を含まないメタデータだけを記録する
```

`selected` は提案で、実行の許可ではありません。Skillを読み込むか決めるのは、Codexか利用者です。

## 主なコマンド

| コマンド | 用途 | 外部送信 |
| --- | --- | --- |
| `jevx skills suggest` | 依頼に合うSkillを提案する | APIキー設定時はJevへ送る |
| `jevx skills list` | 探索できるSkillの一覧 | なし |
| `jevx doctor` | 設定・導入状態・次の一手 | なし |
| `jevx stats` | ローカルTelemetryの集計 | なし |
| `jevx data path` / `export` / `purge` | ローカルデータの場所・書き出し・削除 | なし |
| `jevx hooks install` / `uninstall` | Codex Hookの登録・解除（`--dry-run` で差分確認） | なし |
| `jevx hooks compact-assist` | Compaction前後のcheckpointと補助context | なし |
| `jevx eval` / `eval-repeat` | Skill選択の品質・速度・ばらつきを測る | `--dry-run` 以外はJevへ送る |
| `jevx hooks shadow` / `correlate` / `compact-eval` / `conversation-eval` | Hookの観測と評価 | UserPromptSubmitのみJevへ送る |

各コマンドの詳細は [CLIリファレンス](../docs/developers/jevx-cli-reference.md) を見てください。

## データはどこにあり、どう持ち出すか

- jevxが自動で書くデータは `$JEVX_HOME`（既定 `~/.jevx`）の下だけです。`jevx data path` で一覧、`jevx data export` で書き出し、`jevx data purge --yes` で削除できます。例外は、利用者が明示したファイル（`hooks install` / `uninstall` が変更するCodexの `hooks.json`、`--output` で指定したレポート）です。
- Telemetryにはprompt本文・APIキー・Jevの確率を保存しません。止めるときは `JEVX_TELEMETRY=off`。
- Jevへ送るのは、redactしたprompt、作業ディレクトリ、候補SkillのID・名前・説明だけです。Skill本文、会話全文、Tool結果は送りません。
- Hookを入れたら `jevx hooks uninstall` で戻せます。jevxのhandlerだけを取り除き、ほかのHookは残します。

## やらないこと

- Skillの自動ロード・自動実行、権限の自動付与
- 会話の要約・書き換え、Codex公式Compactionの代替
- 設定オプションを増やすこと（閾値3つとDecision Contractの実行上限6つは環境変数で上書きできますが、範囲外は既定値に戻り、既定値の改善を優先します。一覧は[CLIリファレンス](../docs/developers/jevx-cli-reference.md#apiキーと設定)）

理由と代替手段は [PHILOSOPHY.md](PHILOSOPHY.md#やらないことnon-goals) にあります。

## 互換性の約束

CLIのサブコマンドと引数、`--json` のキー（`schemaVersion` 付き）、終了コード、ローカルデータのschemaを公開境界として守ります。Rustのライブラリ API、人間向け出力の文言、性能値は約束しません。詳しくは [PHILOSOPHY.md](PHILOSOPHY.md#互換性の約束)。

## ディレクトリ構成

```text
jevx/
├── PHILOSOPHY.md          # 判断の軸（正本）
├── skill/SKILL.md         # Codexへ入れるadvisory Skill
├── evals/                 # 評価用fixtureとSkillカタログ（実利用には使わない）
├── scripts/setup.sh       # ビルドとSkill・Hookの導入
├── src/cli/               # 1コマンド1ファイルのCLI
├── src/                   # 判定・Hook・データのライブラリ
└── tests/                 # 要件テストと公開JSONの契約テスト
```

## 開発

```bash
cargo fmt --manifest-path jevx/Cargo.toml -- --check
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
cargo llvm-cov --locked --manifest-path jevx/Cargo.toml --summary-only --fail-under-lines 98
```

同じチェックをGitHub Actionsでも実行しています。外部からのPull Requestは原則受け付けていません（[CONTRIBUTING.md](../CONTRIBUTING.md)）。

## さらに読む

- [ユーザー向けガイド](../docs/users/getting-started.md): 導入の判断、Jevあり／なしの比較、見送るべきケース
- [CLIリファレンス](../docs/developers/jevx-cli-reference.md): 全コマンド、設定、Telemetry、トラブルシューティング
- [アーキテクチャ](../docs/developers/jevx-architecture.md): 処理フロー、Jevを使う設計原則、安全側の判定
- [評価計画](../docs/developers/jevx-evaluation.md) / [ロードマップ](../docs/developers/jevx-roadmap.md)
- [Hook / Compaction運用runbook](../docs/operators/jevx-compaction-operations.md)
- [ドキュメント入口](../docs/README.md)
