# jevx

`jevx`は、Codex CLIのSkill選択とCompaction運用を補助するmacOS向けRust/Cargo CLIです。

このリポジトリの現行サポート対象は、`jevx/` にあるRust製CLIです。ルートの `src/` に残る旧Node.js Webアプリと `docs/screenshots/` は、過去のJev検証用referenceであり、現在のjevx要件・運用手順には含めません。

## まず読む

- [ユーザー向けjevxガイド](docs/users/getting-started.md): 機能、導入価値、Jevあり/なしの比較、セットアップ
- [ドキュメント入口](docs/README.md): 読者別の全体像と文書一覧

## 最短の導入

macOS、Rust stable、Cargoを用意し、リポジトリのルートで実行します。`setup.sh` の主な引数は `--scope user|project [--repo PATH] [--hooks]` で、ヘルプは `--help` / `-h` で表示できます。docsのJSON検証には `jq`、カバレッジ確認には任意で `cargo-llvm-cov`（`cargo install cargo-llvm-cov`）を使います。既定のinstall rootはCargo環境に依存するため、ここでは `$HOME/.local` に固定し、単体コマンド用に同じrootの `bin` を `PATH` へ追加します。

```bash
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user
export PATH="$jevx_install_root/bin:$PATH"
jevx doctor --json
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
jevx skills suggest --prompt "PDFを結合して内容を確認したい" --json
```

Jevを使う提案には `AI_GATEWAY_API_KEY` が必要です。APIキーなしの現行評価は `JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- eval --fixtures jevx/evals/skill-selection.jsonl --skill-dir jevx/evals/skills --dry-run --json` で確認できます。送信データ、Telemetry、導入前後の比較条件は[ユーザー向けjevxガイド](docs/users/getting-started.md)で確認してください。

## リポジトリ内の主な領域

- [jevx](jevx/): Codex CLI向けSkillセレクタとHook/Compaction補助
- [開発者向け文書](docs/developers/jevx-requirements.md): 要件、設計、API、評価Runner
- [運用者向け文書](docs/operators/jevx-compaction-operations.md): Codex HookとCompactionの運用手順
- [調査・評価記録](docs/research/jev-research.md): 導入判断の根拠、歴史的な調査、実測スナップショット（現行値とは分離）
