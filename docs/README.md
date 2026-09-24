# ドキュメント入口

このディレクトリは、読む目的ごとに文書を分類しています。導入判断にはユーザー向けガイド、実装・保守には開発者向け文書、Codex CLIへの接続には運用者向けrunbookを使ってください。現行値と歴史値は入口から分けています。

現行仕様の正本は `jevx/` のRust CLIとそのCLI helpです。`legacy/jev-triage/` に隔離した旧Node.js Webアプリと画面スクリーンショットはreferenceとして保持していますが、現行の要件・運用対象ではありません。

## 判断の軸

jevxで「何をして、何をしないか」「何を互換性として守るか」の正本は [jevx/PHILOSOPHY.md](../jevx/PHILOSOPHY.md) です。機能追加や要望への回答、レビューはこの文書に照らして判断します。

## Current / Latest

ここが現行の導入・評価・運用の入口です。値を引用するときは、実行日時・commit・fixture hash・環境を併記し、歴史的なAPIキーあり測定と混ぜないでください。

| 用途 | 現行の入口 | 位置付け |
| --- | --- | --- |
| 導入 | [users/getting-started.md](users/getting-started.md) | Rust CLIの前提、install root/PATH、データ境界 |
| コマンド | [developers/jevx-cli-reference.md](developers/jevx-cli-reference.md) | 全コマンド、設定値、Telemetry、データ管理、トラブルシューティング |
| 評価 | [developers/jevx-evaluation.md](developers/jevx-evaluation.md) | 4モードのdry-run、再現条件、現行baseline |
| 価値・採用判断 | [developers/jevx-value.md](developers/jevx-value.md) | 速度・品質・安全性・Jev/Codex費用の同時評価 |
| 費用観測 | [developers/jevx-cost-observability.md](developers/jevx-cost-observability.md) | 推定/実費、価格版、task/turn/session集計 |
| 要件・設計 | [developers/jevx-requirements.md](developers/jevx-requirements.md)、[developers/jevx-architecture.md](developers/jevx-architecture.md) | Rust実装の現行契約 |
| 運用 | [operators/jevx-compaction-operations.md](operators/jevx-compaction-operations.md) | Hook/Compactionのcanonical runbook（2026-09-22時点） |

## Reference / Non-current

現行jevxの正本ではない参考資料です。直接APIの利用や旧Node.js Webアプリへの移行を示す現行手順ではありません。

| 資料 | 位置付け |
| --- | --- |
| [TypeSafe APIを直接使う](developers/typesafe-api.md) | TypeSafe直接APIの参考・非現行経路。現行jevxのGateway実装ではない |
| [`legacy/jev-triage/`](../legacy/jev-triage/README.md) のNode.js Webアプリ | 過去のJev検証用reference。現行の要件・運用対象ではない |

## 読者別の入口

| 読者 | 入口 | 内容 |
| --- | --- | --- |
| 利用者・導入判断担当 | [users/getting-started.md](users/getting-started.md) | jevxの機能、導入価値、Jevあり/なしの比較、セットアップ |
| 開発者・保守担当 | [developers/jevx-requirements.md](developers/jevx-requirements.md) | 要件、アーキテクチャ、評価Runner、TypeSafe直接APIの参考資料 |
| Codex CLI運用担当 | [operators/jevx-compaction-operations.md](operators/jevx-compaction-operations.md) | Hook、Compaction checkpoint、導入・検証・復旧手順 |
| 調査・評価担当 | [research/jev-research.md](research/jev-research.md) | 歴史的な調査、候補案、固定fixtureの評価記録 |

## 開発者向け

- [jevx 要件定義と実装状況](developers/jevx-requirements.md): 対象範囲、CLI契約、データ保護、成功条件
- [jevx アーキテクチャ](developers/jevx-architecture.md): Skill探索、Jev判定、Hook、Compaction補助の境界
- [jevx 評価計画](developers/jevx-evaluation.md): 指標、fixture、再現コマンド、品質ゲート
- [jevx 価値定義](developers/jevx-value.md): 速度向上、意味レビュー、route、費用の採用条件
- [jevx 費用観測契約](developers/jevx-cost-observability.md): Jev/Codex/合算を比較する正本
- [jevx 意味レビュー・route・fix](developers/jevx-review-routing.md): typed contractと安全境界
- [jevx 文書監査](developers/jevx-documentation-audit.md): OSS設計原則との照合
- [jevx CLIリファレンス](developers/jevx-cli-reference.md): 全コマンドの使い方と公開JSONの契約
- [jevx ロードマップ](developers/jevx-roadmap.md): 今後の候補と、採用するときの基準
- [TypeSafe APIを直接使う](developers/typesafe-api.md): jevxの現行経路とTypeSafe直接APIを比較する参考資料

## 運用者向け

- [jevx Codex CLI compaction実運用runbook](operators/jevx-compaction-operations.md): `hooks install`、Trust確認、実セッション検証、ログ相関、復旧

## 調査・評価

### Historical research / Reference

この2件は現行仕様ではなく、歴史的な調査・未実装候補の入口です。実装済みHookの現行手順は、上のCurrent/Latestにあるcanonical runbookを参照してください。

- [Jevを活用したアプリ候補案](research/jev-app-ideas.md): 実装候補の比較メモ
- [Jev活用アイデア徹底調査レポート](research/jev-research.md): 候補、出典、質問設計、評価方針

## Historical

以下は記録時点の環境・fixtureに対する歴史的な測定記録です。数値は現行環境や実ユーザー入力への性能保証ではありません。研究メモは歴史的な調査・未実装候補の記録、Hook評価は現行Hook実装に対する過去の測定記録であり、現行ロードマップやcanonical runbookそのものではありません。

- [Jevあり/なしの導入効果比較](research/evaluations/jevx-comfort-evaluation.md): 固定40ケースの比較と導入判断（2026-09-21）
- [複数回・APIキーあり実測](research/evaluations/jevx-variance-evaluation-2026-09-21.md): 5回×40ケースの分散、遅延、Token、エラー率
- [APIキーあり単回実測](research/evaluations/jevx-live-evaluation-2026-09-21.md): 40ケース単回の実測
- [Codex Hook shadow / Compaction評価](research/evaluations/jevx-codex-hooks-evaluation.md): Hook shadow、設定、Compaction相当fixture
- [実Codex Hook / 実会話型Compaction評価](research/evaluations/jevx-real-codex-compaction-evaluation-2026-09-21.md): 実CLI・実会話での確認記録
- [Codex Compaction深掘り評価](research/evaluations/jevx-compaction-depth-evaluation-2026-09-21.md): Hook直接検証とCompaction評価

## 文書の読み方

- `Current / Latest`、`developers/`、`operators/` は現行の仕様・運用を説明します。環境依存の値や導入前の注意点は各文書の冒頭とrunbookを確認してください。
- `Historical` と `research/evaluations/` は再現条件を残すためのスナップショットです。後から環境、モデル、Gateway、fixtureが変わっても、当時の観測値として扱います。
- `research/` の候補案は歴史的な調査・未実装候補、`research/evaluations/` のHook評価は現行Hook実装に対するHistorical snapshotです。実際に実行する手順は[canonical runbook](operators/jevx-compaction-operations.md)へ戻ってください。
- jevxはShadow Modeが基本です。Skill本文の自動ロード・実行や会話の書き換えを行う機能ではありません。HookとCompaction補助は明示的な導入・Trustが必要です。
