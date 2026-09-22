# Jev Triage / jevx

Jev Triageは、Jevの型付き評価で問い合わせのカテゴリ・緊急度・返金要求を確認する小さなWebアプリです。同じリポジトリに、Codex CLIのSkill選択とCompaction運用を補助するmacOS向けCLI `jevx`を収録しています。

## まず読む

- [ユーザー向けjevxガイド](docs/users/getting-started.md): 機能、導入価値、Jevあり/なしの比較、セットアップ
- [ドキュメント入口](docs/README.md): 読者別の全体像と文書一覧

## 最短の導入

```bash
sh jevx/scripts/setup.sh --scope user
jevx doctor --json
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
jevx skills suggest --prompt "PDFを結合して内容を確認したい" --json
```

Jevを使う提案には `AI_GATEWAY_API_KEY` が必要です。送信データ、Telemetry、導入前後の比較条件は[ユーザー向けjevxガイド](docs/users/getting-started.md)で確認してください。

## 画面例

- [配送に関する問い合わせ](docs/screenshots/jev-triage-shipping.png)
- [請求・返金に関する問い合わせ](docs/screenshots/jev-triage-billing.png)
- [身に覚えのない請求](docs/screenshots/jev-triage-invisible-charge.png)

## リポジトリ内の主な領域

- [Webアプリ](src/): Jev Triageの実装
- [jevx](jevx/): Codex CLI向けSkillセレクタとHook/Compaction補助
- [開発者向け文書](docs/developers/): 要件、設計、API、評価Runner
- [運用者向け文書](docs/operators/): Codex HookとCompactionの運用手順
- [調査・評価記録](docs/research/): 導入判断の根拠と歴史的な実測スナップショット
