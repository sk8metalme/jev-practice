# jevx v1 要件定義

## 目的

`jevx`は、Codex CLIの日常利用を快適にしながら、Jevの実用性を測定するためのmacOS向けCLIである。

v1では、現在の依頼に適したCodex SkillをJevで提案する。Skillの自動ロード・実行やCodex Hookへの自動介入は行わず、Shadow Modeで結果を表示・記録する。

## 対象範囲

- Rust製CLIを`jevx/`に追加
- プロジェクトSkillとユーザー共通Skillを探索
- ローカル候補絞り込みとJevのChoice判定
- `none`による未推薦
- 人間向け出力と`--json`出力
- `jevResponseMs`、全体時間、token usageの表示
- メタデータのみのJSONL Telemetry
- CodexメタSkillとmacOS用セットアップスクリプト

以下は後続タスクとする。

- Codex Hook
- Compaction補助
- Tool Result削減
- Skillの自動ロード・実行
- MCPサーバー
- Windows対応

## CLI

```bash
jevx skills suggest --prompt "PDFを結合して" --json
jevx skills suggest --stdin
jevx skills suggest --input-json
jevx skills list --json
jevx stats --json
jevx doctor --json
```

Jev判定は`AI_GATEWAY_API_KEY`を使い、既存アプリと同じVercel AI Gatewayの`typesafe-ai/jev`へ送信する。

デフォルトの設定は次のとおり。

- 候補上限: 32件
- 選択確率の下限: 0.60
- 1位と2位の確率差: 0.10以上
- リクエストタイムアウト: 1,500ms
- Telemetry: `~/.jevx/events.jsonl`

## データ保護

Jevへ送るのは、マスキング済みの現在の依頼文、作業ディレクトリ、Skillの名前・説明だけとする。Skill本文、過去の会話、Tool結果、APIキーは送信しない。

Telemetryには生の依頼文を保存せず、SHA-256、文字数、候補数、判定、確率、遅延、usageだけを保存する。

## 成功条件

- ローカルSkill探索のp95が100ms以内
- Jevを含む全体処理のp95が2秒以内
- `jevResponseMs`が人間向け・JSON出力・Telemetryに存在
- Jev未設定・タイムアウト時に明示エラー
- 既存のローカルキーワード方式との精度・遅延・token usage比較が可能
- 新規Rust実行コードのラインカバレッジ98%以上

## 評価

合成ケース30件と、秘密情報を除いた匿名化実例10件を用意し、以下を比較する。

1. Skill選択なし
2. ローカルキーワード方式
3. jevxのJev方式

測定項目は、Top-1精度、`none`判定率、候補漏れ率、Jev p50/p95、全体p50/p95、入力token、エラー率とする。
