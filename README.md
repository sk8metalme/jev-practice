# jev-practice

Jev（TypeSafeの意味判定API）を、実際の開発作業で安全に使えるかを確かめるためのリポジトリです。現在のプロダクトは、Codex CLIの速度・意味レビュー・model/reasoning候補・費用を同じ条件で観測する **[jevx](jevx/)**（macOS向けRust CLI）です。

## なぜ作ったか

LLMの判定を開発ツールに組み込むとき、「本当に速く・正しくなったのか」「追加費用はいくらか」「失敗したら何が起きるのか」を説明できないまま導入しがちです。このリポジトリでは、Jevに意味の判定だけを任せ、実行や権限はコードが持つ、という線引きで、速度・品質・安全性・Jev費用・Codex費用を同じ条件で測っています。

## 原則（要約）

1. **自動化より安全側** — Jevの回答は提案。timeoutや低確信度を成功扱いにしない
2. **Jevの便利さより決定的なローカルコード** — コードで解ける処理はJevに渡さない
3. **精度単独より観測可能性と再現性** — 遅延・費用・失敗時の挙動まで同じ条件で測る

全文・やらないこと・互換性の約束は [jevx/PHILOSOPHY.md](jevx/PHILOSOPHY.md) にあります。

## まず読む

- [jevx](jevx/README.md): 5分で最初の提案まで
- [jevxの価値定義](docs/developers/jevx-value.md): 速度・レビュー・route・費用の採用判断
- [費用観測契約](docs/developers/jevx-cost-observability.md): 推定/実費、価格版、Jev/Codex/合算
- [ユーザー向けガイド](docs/users/getting-started.md): 導入の判断、Jevあり／なしの比較、見送るべきケース
- [ドキュメント入口](docs/README.md): 読者別の全体像

## リポジトリの構成

| パス | 内容 | サポート |
| --- | --- | --- |
| [`jevx/`](jevx/) | Codex CLI向けSkillセレクタとHook / Compaction補助 | 現行 |
| [`docs/`](docs/README.md) | 利用者・開発者・運用者向けの文書、調査と評価の記録 | 現行（`research/` は歴史的な記録） |
| [`legacy/jev-triage/`](legacy/jev-triage/README.md) | 過去にJevを検証したNode.js Webアプリ | 対象外（referenceとして保持） |
| `.agents/skills/`、`.claude/skills/` | 開発に使うAgent Skill（[APM](https://microsoft.github.io/apm/)で `apm.yml` から導入） | 開発用 |

## 開発に使うAgent Skill

OSSとしての判断の軸を保つため、[oss-design](https://github.com/sk8metalme/oss-design-skill) SkillをAPMで固定しています。

```bash
brew install apm
apm install --frozen   # apm.lock.yaml のとおりに .claude/skills と .agents/skills へ配置
```

## 貢献とライセンス

- Open-Source, not Open-Contribution です。不具合報告や困りごとの共有は歓迎しますが、外部PRは原則受け付けていません（[CONTRIBUTING.md](CONTRIBUTING.md)）。
- 個人が余暇で開発しています。返信には時間がかかることがあります。
- ライセンスは [MIT](LICENSE) です。
