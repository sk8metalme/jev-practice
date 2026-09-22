# ドキュメント入口

このディレクトリは、読む目的ごとに文書を分類しています。導入判断にはユーザー向けガイド、実装・保守には開発者向け文書、Codex CLIへの接続には運用者向けrunbookを使ってください。

現行仕様の正本は `jevx/` のRust CLIです。ルート `src/` のNode.js Webアプリと画面スクリーンショットはlegacy/referenceとして保持していますが、現行の要件・運用対象ではありません。

## 読者別の入口

| 読者 | 入口 | 内容 |
| --- | --- | --- |
| 利用者・導入判断担当 | [users/getting-started.md](users/getting-started.md) | jevxの機能、導入価値、Jevあり/なしの比較、セットアップ |
| 開発者・保守担当 | [developers/](developers/) | 要件、アーキテクチャ、TypeSafe API、評価Runner |
| Codex CLI運用担当 | [operators/](operators/) | Hook、Compaction checkpoint、導入・検証・復旧手順 |
| 調査・評価担当 | [research/](research/) | Jev活用調査、候補案、固定fixtureの評価記録 |

## 開発者向け

- [jevx 要件定義と実装状況](developers/jevx-requirements.md): 対象範囲、CLI契約、データ保護、成功条件
- [jevx アーキテクチャ](developers/jevx-architecture.md): Skill探索、Jev判定、Hook、Compaction補助の境界
- [jevx 評価計画](developers/jevx-evaluation.md): 指標、fixture、再現コマンド、品質ゲート
- [TypeSafe APIを直接使う](developers/typesafe-api.md): jevxの現行経路とTypeSafe直接APIを比較する参考資料

## 運用者向け

- [jevx Codex CLI compaction実運用runbook](operators/jevx-compaction-operations.md): `hooks install`、Trust確認、実セッション検証、ログ相関、復旧

## 調査・評価

### 調査

- [Jevを活用したアプリ候補案](research/jev-app-ideas.md): 実装候補の比較メモ
- [Jev活用アイデア徹底調査レポート](research/jev-research.md): 候補、出典、質問設計、評価方針

### 評価スナップショット

以下は記録時点の環境・fixtureに対する歴史的な測定記録です。数値は現行環境や実ユーザー入力への性能保証ではありません。

- [Jevあり/なしの導入効果比較](research/evaluations/jevx-comfort-evaluation.md): 固定40ケースの比較と導入判断
- [複数回・APIキーあり実測](research/evaluations/jevx-variance-evaluation-2026-09-21.md): 5回×40ケースの分散、遅延、Token、エラー率
- [APIキーあり単回実測](research/evaluations/jevx-live-evaluation-2026-09-21.md): 40ケース単回の実測
- [Codex Hook shadow / Compaction評価](research/evaluations/jevx-codex-hooks-evaluation.md): Hook shadow、設定、Compaction相当fixture
- [実Codex Hook / 実会話型Compaction評価](research/evaluations/jevx-real-codex-compaction-evaluation-2026-09-21.md): 実CLI・実会話での確認記録
- [Codex Compaction深掘り評価](research/evaluations/jevx-compaction-depth-evaluation-2026-09-21.md): Hook直接検証とCompaction評価

## 文書の読み方

- `developers/` と `operators/` は現行の仕様・運用を説明します。環境依存の値や導入前の注意点は各文書の冒頭とrunbookを確認してください。
- `research/evaluations/` は再現条件を残すためのスナップショットです。後から環境、モデル、Gateway、fixtureが変わっても、当時の観測値として扱います。
- JevxはShadow Modeが基本です。Skill本文の自動ロード・実行や会話の書き換えを行う機能ではありません。HookとCompaction補助は明示的な導入・Trustが必要です。
