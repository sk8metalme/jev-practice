# Contributing

このリポジトリの判断基準は [jevx/PHILOSOPHY.md](jevx/PHILOSOPHY.md) にあります。提案の前に「やらないこと」と「互換性の約束」を確認してください。

## 貢献モデル: Open-Source, not Open-Contribution

jevxは自由に利用・改変・フォークできますが、**外部からのPull Requestは原則受け付けていません**。安全側に倒す設計（Jevの回答を実行権限にしない、失敗を成功扱いにしない）を、一人で一貫して追跡できる状態を保つためです。

## 歓迎するもの（Issue）

- **再現手順つきの不具合報告**: 実行したコマンド、`jevx doctor --json` の出力（APIキーの値は表示されませんが、HOME配下の絶対パスにユーザー名が含まれるので、貼る前に伏せてください）、期待した結果と実際の結果
- **実在する困りごとの説明**: 誰が、どの作業で、何に困っているか。解決方法の提案はなくて構いません
- **匿名化した評価fixture**: jevxが誤った提案・review・routeをした依頼の例。秘密情報、会話全文、Skill本文、raw Tool resultは含めないでください。費用を添える場合は通貨・price version・`estimated`/`actual`を併記してください

## 受け入れにくいもの

- 外部PR（内容が妥当なら、作者が改めて実装します）
- [やらないこと（Non-goals）](jevx/PHILOSOPHY.md#やらないことnon-goals)に当たる要望（Skillの自動実行、auto-allow、会話の要約など）
- 設定オプションの追加（まず既定値の改善で解けないかを検討します）
- `legacy/` 配下の機能追加や修正

## 変更時の観測要件

Jevを使う変更は、Jevの推薦を最終権限にしないtyped contract、timeout/provider error時の安全側status、receipt/replay fixtureを含めます。速度を主張する変更は、品質とJev/Codex/合算費用、`unknown`/`unavailable`、retry/fallback/cacheを同じ条件で記録します。本文を外部送信する変更は明示opt-inとredaction境界を設計し、receiptへ本文を保存しません。自動fixは`--yes`、高信頼度、期待hash一致、safe pathをコード側でgateします。

実装前後に、必要な正本（[価値定義](docs/developers/jevx-value.md)、[費用観測](docs/developers/jevx-cost-observability.md)、[review/route](docs/developers/jevx-review-routing.md)）を更新し、Historical資料の数値を現行保証として再利用しないでください。

同じ種類の要望を3回見送ったら、理由と代替手段をPHILOSOPHY.mdの「やらないこと」へ追記します。

## セキュリティ上の問題

APIキーや秘密情報が漏れる可能性のある不具合は、公開Issueに再現手順や値を書かないでください。「セキュリティ上の報告がある」とだけIssueで知らせてもらえれば、非公開で詳細を受け取る方法を案内します。

## 返信について

個人が余暇で開発しています。返信や修正には時間がかかることがあります。
