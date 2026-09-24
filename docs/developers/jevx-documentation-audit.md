# jevx文書監査

> 実施方針: `oss-design` Skillの14原則と `jevx/PHILOSOPHY.md` を照合し、現行仕様とHistorical資料を混ぜない。

## 監査結果

| 原則 | 状態 | 根拠/次の作業 |
| --- | --- | --- |
| 問題が先 | 完了 | [jevx-value.md](jevx-value.md)に利用者の困りごと、採用条件、捨てるものを記載 |
| Non-goals | 完了 | Skill自動実行、auto-allow、会話書換、Tool Result削除、費用上限を正本へ記載 |
| 既定値優先 | 完了 | content外部送信とfixを既定無効。費用はProvider usage/cost payloadを使い、単価設定を追加しない |
| 公開境界 | 完了 | CLI、JSON、receipt、Hook marker、Hook record v4、checkpoint v2、data schema v5と移行テストを確認 |
| ユーザー主権 | 完了 | `$JEVX_HOME`管理、data export/purge、`--dry-run`、`--yes`、明示opt-in |
| 最初の5分 | 維持 | `jevx/README.md`からvalue/cost/CLIへリンク。導入とdoctorのnextStepsを保持 |
| 局所性 | 完了 | `src/cli/`のcommand境界、review/route/costの責務分離、正本リンクを追加 |
| 安全性 | 完了 | redaction、本文非保存、unknown/unavailable、fail-closed fix/route |
| 再現性 | 完了 | typed contract、digest、receipt、fixture、replay、価格版を記録 |
| 運用可能性 | 継続 | review-statsと費用statusを追加。App Server/SDK経路は未実装として明記 |

## 文書の正本

- 判断基準: [`jevx/PHILOSOPHY.md`](../../jevx/PHILOSOPHY.md)
- 価値/採用ゲート: [`jevx-value.md`](jevx-value.md)
- 費用型・価格・集計: [`jevx-cost-observability.md`](jevx-cost-observability.md)
- review/route/fix: [`jevx-review-routing.md`](jevx-review-routing.md)
- CLI/JSON/設定: [`jevx-cli-reference.md`](jevx-cli-reference.md)
- 実装要件: [`jevx-requirements.md`](jevx-requirements.md)
- アーキテクチャ: [`jevx-architecture.md`](jevx-architecture.md)
- 評価: [`jevx-evaluation.md`](jevx-evaluation.md)
- Hook/Compaction運用: [`../operators/jevx-compaction-operations.md`](../operators/jevx-compaction-operations.md)

同じ仕様を複数文書へ複製せず、入口文書は上の正本へリンクする。CLIのJSON例は実装のschemaVersion・キーと契約テストを一致させる。

## Historical資料の扱い

`docs/research/evaluations/` と `docs/research/` は、記録時点のfixture・環境・モデルに対する調査であり、現行性能保証ではない。数値の書き換えはせず、入口と各資料にHistoricalであることを示す。`legacy/`配下は現行対象外で、機能追加・修正・移行はしない。

## 監査時のチェックコマンド

```bash
rg -n "schemaVersion|JEVX_.*PRICE|review|unknown|unavailable|legacy" \
  README.md jevx/README.md jevx/PHILOSOPHY.md docs
cargo fmt --manifest-path jevx/Cargo.toml -- --check
cargo clippy --locked --manifest-path jevx/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path jevx/Cargo.toml --all-targets
```
