# jevxの始め方

このページは、Codex CLIへ `jevx` を導入するか判断したい人向けの入口です。jevxが何を補助し、Jevを使わない場合と何が変わるか、どの条件で導入を見送るべきかを、現在の実装と記録済みの評価に基づいて説明します。

## 先に結論

`jevx`は、Codex CLIのSkill選択を提案し、選択しない `none` も含めて判断結果を計測するmacOS向けRust CLIです。基本動作はShadow Modeで、提案されたSkillを自動でロード・実行したり、会話を書き換えたりしません。

固定fixtureの評価では、記録済みのjevx + Jev snapshotはローカルキーワード方式より高い正解率を示しました。一方で、Jev/Gatewayへの外部通信、Token使用量、応答遅延、Provider errorが追加されます。このため、jevxは「必ず速くなる自動化」ではなく、Skill選択の判断と導入効果を観測可能にする補助線として検討してください。`nonePrecision` は実装上の互換JSONキーで、意味は `expected: none` の正解数を分母にした recall（`noneCorrect / expectedNone`）です。

## jevxの機能

jevxは、依頼文と利用可能なSkillの説明を使って「どのSkillが合いそうか」を提案します。現在の実装では次の順で処理します。

1. プロジェクトとユーザー領域のSkillを探索する。
2. Skillの名前・説明をローカルでスコアリングし、最大32件の候補に絞る。
3. 候補と `none` を1回のJev Choiceへ渡す。
4. 確率が0.60以上、かつ1位と2位の差が0.10以上のときだけ `selected` とする。
5. JSONまたは人間向け出力と、遅延・Tokenなどのメタデータを返す。

Jevが利用できない場合はローカル推測をJev結果として扱わず、`missing_api_key`・`timeout`・`provider_error` などを明示します。Jevが正常に応答したうえで確信度が足りない、または通常の候補なしと判定した場合だけ、結果を `none` に倒します。つまり、エラーと通常の安全側判定は別の状態です。

### `error` と通常の `none` を分けて読む

`decision: error` はAPIキー未設定、timeout、Provider応答不正などで判定処理が完了していない状態です。`decision: none`（または `no_candidates`）は処理が完了した正常な未推薦で、`below_probability_threshold`、`below_margin_threshold`、`unknown_choice`、`missing_choice` など、閾値未満や未知のchoiceを安全側に倒した結果を含みます。したがって、`none`をエラー率へ足し戻したり、APIキー・timeoutのエラーを正常な`none`判定として数えたりしないでください。

### 自動実行との境界

通常のSkill提案はShadow Modeです。`selected` になっても、次の処理は行いません。

- Skill本文の自動ロード
- Skillの自動実行や権限付与
- 過去の会話の書き換え
- Tool結果や会話全文の要約・削除

Codex Hookを使う場合は、利用者が明示的に `hooks install` を実行し、生成された設定をreview/trustします。Hookの `UserPromptSubmit` は候補・Jev判定を観測し、Compaction関連Hookは決定的なcheckpointとredacted contextを扱います。Compaction補助は会話履歴の完全復元や公式Compactionの代替ではありません。

## 実装済み機能の利用者向け説明

実装対応表の「実装済み」は、次の4領域です。いずれも、利用者が出力と設定を確認しながら使うための観測・補助機能であり、Skillや会話を勝手に操作する機能ではありません。

| 機能 | できること | 代表コマンド | 利用時の境界 |
| --- | --- | --- | --- |
| Skill探索・ローカル順位付け・Jev判定 | project/user Skillを探し、候補と`none`をJev Choiceで判定する | `jevx skills list` / `jevx skills suggest` | `selected`でもSkill本文を自動ロード・実行しない |
| Telemetry・stats・doctor | 設定状態、判定数、`none`率、遅延、Token集計を確認する | `jevx doctor --json` / `jevx stats --json` | prompt本文、APIキー、Jevのprobabilityはローカル記録へ保存しない |
| Hook shadow・install・compact-assist | Hook発火を観測し、明示導入したHookでcompact前後のcheckpointを補助する | `jevx hooks shadow` / `jevx hooks install` / `jevx hooks compact-assist` | 設定変更はopt-inで、Codex側のHook Trustが必要。公式Compactionを置き換えない |
| Skill選択・Compaction・会話評価Runner | 固定fixtureや安全なJSONLで精度、分散、保持事実、秘密値漏えい、重複を測る | `eval` / `eval-repeat` / `hooks compact-eval` / `hooks conversation-eval` / `hooks correlate` | Codex/App Serverを起動せず、会話全文のLLM要約もしない |

### Telemetry・stats・doctor

`jevx doctor` は、APIキーが設定されているか、Gateway endpoint、Telemetryの保存先・有効状態、macOS向け設定を確認します。APIキーの値そのものは表示しません。`jevx stats` は既定の `~/.jevx/events.jsonl`、または `--input` で指定したJSONLから、判定数、選択率、`none`率、エラー率、Jev/全体の遅延、Token集計を計算します。

```bash
jevx doctor --json
jevx stats --json
jevx stats --input /tmp/jevx-events.jsonl --json
```

`--no-telemetry` はローカルJSONLへの追記だけを止めます。Jev/Gatewayへの外部送信も避けたい場合は、APIキーなしの `eval --dry-run`、Skill一覧、診断コマンドを使ってください。

### HookとCompaction補助

Hookを使う場合は、まず変更内容をdry-runで確認し、問題がなければ明示的に導入します。user scopeは `$CODEX_HOME/hooks.json`（未設定なら `~/.codex/hooks.json`）、project scopeは `<repo>/.codex/hooks.json` を更新します。既存の設定は保持され、既存ファイルがある場合は初回変更時にbackupが作られます。

```bash
jevx hooks install --scope user --dry-run --json
jevx hooks install --scope user --json
```

導入されるイベントの役割は次のとおりです。

| Codex event | jevxの役割 |
| --- | --- |
| `UserPromptSubmit` | Skill候補の探索とJev判定をShadow Modeで記録する。Skill本文や追加の会話contextは返さない |
| `PreCompact` / `PostCompact` | session・turnなどのhashとredacted contextのmetadataをcheckpointへ記録する |
| `SessionStart(source=compact)` | 最新checkpointとredacted manifestを追加contextとして返す。contextがなければその旨を返し、Codex処理は止めない |

`hooks shadow` が受け付けるknown eventは `SessionStart`、`PreCompact`、`PostCompact`、`UserPromptSubmit` です。未知event、event不一致、壊れたJSONは固定の継続応答を返さずエラーになります。Hook recordにはprompt、session/turn ID、cwd、model、manifest本文をそのまま保存せず、hash・文字数・安全な識別ラベルだけを残します。

`compact-assist` はLLM要約器ではありません。会話全文の復元、公式Compactionの再実装、Tool Resultの削除、現在のリポジトリ状態の保証は行わず、決定的なcheckpoint metadataとredacted contextだけを補助します。Hookの発火確認、backup、Trustを含む運用手順は[Codex CLI compaction実運用runbook](../operators/jevx-compaction-operations.md)を参照してください。

### 評価Runner

評価Runnerは、固定fixtureや管理されたJSONLを使って、Skill選択・Compaction・会話評価の品質と安全性を確認するための機能です。通常のCodexセッションやApp Serverを起動するものではありません。

| コマンド | 目的 | 外部通信と出力 |
| --- | --- | --- |
| `eval --dry-run --json` | `none`、ローカルキーワード、ローカル順位付けのbaselineを確認する | Gatewayへ送らず、Jevモードは`not_run`。case出力へprompt本文・keywords・Jev response本文を保存しない |
| `eval-repeat --runs 5 --dry-run --json` | 同じfixtureを繰り返し、run間の精度・遅延・Token分布を確認する | `--output`はrun/mode集計で、case本文を保存しない。APIキーありで実行するとJevも測定する |
| `hooks compact-eval --runs 5 --json` | 固定Compaction fixtureで保持事実、秘密値漏えい、完了率、復旧、Tokenを確認する | Codex内部の要約モデルやApp Serverは呼び出さない |
| `hooks conversation-eval --input <file> --json` | 管理された会話評価JSONLから保持率、漏えい、遅延、復旧を集計する | レポートへfollow-up本文やsecret marker本文を再出力しない。入力JSONLは測定後に破棄する |
| `hooks correlate --input <file> --json` | Hook recordのイベント数と重複を確認する | 生のpromptやIDではなく、保存済みhashと安全なmetadataを集計する |

Skill選択の評価計画と、入力・出力スキーマの詳細は[jevx評価計画](../developers/jevx-evaluation.md)にまとめています。外部サービスを使う実測は、APIキーを環境変数だけで設定し、合成・匿名化fixtureに限定してください。

## 現行baseline（Latest）

次の値は、外部送信なしの `eval --dry-run --json` を2026-09-22に実行した現行baselineです。`nonePrecision` は上記のとおり `expectedNone` 分母の recall であり、通常の「noneを予測した全件を分母にするprecision」ではありません。

| 項目 | 値 |
| --- | --- |
| 実行日時 | `2026-09-22T15:00:42+09:00` |
| commit | `ed0b93c023251400bcbbe2de8cdedd5451f08761` |
| fixture SHA-256 | `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb` |
| Rust / Cargo | `rustc 1.98.1 (48a229cea 2026-09-01)` / `cargo 1.98.1 (797e8a9bc 2026-08-05)` |
| OS | macOS 26.6.2（build 25G83） |
| 実行条件 | `JEVX_TELEMETRY=off`、APIキーなし、fixture 9 Skill + 既定roots（有効候補14）、外部Gateway呼び出しなし |
| カタログ条件 | 評価用 `--skill-dir jevx/evals/skills` を指定。Latest値の再現は同じHOME/CODEX_HOME/project roots、fixture-only再現は隔離root |

| 方式 | cases | 正解率 | `nonePrecision`（`expectedNone` 分母の recall） | 外部通信 |
| --- | ---: | ---: | ---: | --- |
| `none` | 40 | 10.0% | 100.0%（4/4） | なし |
| `local_keyword` | 40 | 87.5% | 75.0%（3/4） | なし |
| `local_rank` | 40 | 17.5% | 75.0%（3/4） | なし |
| `jevx` | 0 | — | — | `not_run`（dry-run） |

再現コマンドは次のとおりです。

```bash
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --dry-run --json
```

このLatest baselineは実行時のSkillカタログ、OS、Rust/Cargo、fixture、環境変数に依存します。とくにevalはproject `.agents/skills` / `.codex/skills`、`HOME/.agents/skills`、`CODEX_HOME/skills`も探索するため、Latest値を再現するには記録時と同じrootsを保持してください。一時HOME・CODEX_HOME・空のproject rootと明示したfixture Skill rootは、fixture-onlyの隔離再現に使います（値はLatestと異なり得ます）。手順は[評価計画](../developers/jevx-evaluation.md)にあります。

## Historical snapshot（2026-09-21）

同梱の9件の評価用Skillと40ケース（合成30件、匿名化テンプレート10件）を使った、APIキーありの歴史的snapshotです。単回結果は40ケースを1回、複数回結果は同じ条件を5回、合計200ケースで測定しました。実装コミット、Gateway/model、OS、fixture hashなどは各レポートの測定条件を参照し、現行baselineと混ぜないでください。

| 方式 | 正解率 | `nonePrecision`（`expectedNone` 分母の recall） | 外部通信・遅延・使用量 |
| --- | ---: | ---: | --- |
| 選択なし (`none`) | 10.0% | 100.0% | 外部通信なし |
| ローカルキーワード | 90.0% | 75.0% | 外部通信なし |
| jevx + Jev（1回） | 100.0% | 100.0% | Jev p50/p95 407/673ms、平均 2,401.175 input / 127.675 output tokens |
| jevx + Jev（5回平均） | 98.0% | 100.0% | エラー率2.0%、探索 p50/p95 2/2ms、Jev p50/p95 427/610ms |

5回測定では、200ケース中4件がProvider errorでした。Jevが成功した196件を対象に、全体時間のp50/p95は429/612msです。したがって、jevxの主な追加コストはローカル探索ではなく、Jev/Gatewayへの通信とそのToken使用量です。

この表は固定fixture・特定のモデル・Gateway・ネットワーク条件に対する観測値です。実ユーザーの依頼全般、将来のモデル、長期SLO、費用を保証するものではありません。評価の条件・再現手順・限界は[導入効果評価](../research/evaluations/jevx-comfort-evaluation.md)と[複数回実測レポート](../research/evaluations/jevx-variance-evaluation-2026-09-21.md)に記録しています。現行値はこの表からではなく、上のLatest baselineから取得してください。

### 導入の動機になりやすいケース

次のような場合は、jevxの評価価値があります。

- 利用可能なSkillが増え、依頼ごとの手動選択に負担がある。
- `none`を含む候補判断を、同じ指標で比較・再測定したい。
- Jevへの送信、Token使用量、遅延、Provider errorを運用上受け入れられる。
- Shadow Modeで提案だけを受け、最終的なSkill利用を自分で確認したい。
- HookやCompaction補助を、既存のCodex設定を確認しながら段階的に導入できる。

### 先に見送るべきケース

- 外部通信を一切許可できない、またはAPIキーを外部サービスへ送れない。
- 追加の数百ms規模の応答時間やToken使用量を許容できない。
- Skillを自動実行する仕組みを期待している。
- 固定fixtureの数値を自分の実ユーザーケースの保証として扱う必要がある。
- HookのTrust、バックアップ、ログの保存場所をレビューできない。

## データの扱いと安全性

Jevへの外部リクエスト境界は次のとおりです。

| データ | Gatewayへ送るか | 実装上の扱い |
| --- | --- | --- |
| 現在のprompt | 送る（redact後） | `token` / `api_key` / `secret` / `password` / `authorization` のキーに続く値、単独のBearer値、`sk-` / `tsk-` tokenを固定パターンでredact |
| `cwd` | 送る（raw） | 作業ディレクトリはハッシュ化せず入力に含める。パス名に秘密を置かない |
| Skill ID / name | 送る（raw） | 候補識別のため送る |
| Skill description | 送る（redact後） | promptと同じ限定パターンのredaction。Skill本文は送らない |
| APIキー | 送らない | GatewayのBearer認証ヘッダーにだけ使う |
| Skill本文、過去の会話、Tool結果 | 送らない | v1の送信対象外 |

Basic redactionは、`Authorization=Basic <value>`、`Authorization:Basic <value>`、`Authorization: Basic <value>` の認識済み形式を保護します。通常文中の単独の単語 `Basic` は意味を保つためredactしません。未知のPII・ラベルなし秘密まで除去する完全なDLPではないため、送信前に内容を確認してください。

メールアドレス、電話番号、顧客情報、ラベルのない認証情報など、上記パターンに該当しない機密情報は自動除去されません。外部送信してよい内容だけを入力し、必要に応じて送信前に匿名化してください。

既定Telemetryは `~/.jevx/events.jsonl` に、依頼文そのものではなくハッシュ、文字数、判定、候補数、遅延、Token使用量を保存します。Gatewayの回答に含まれる候補確率（probability）はTelemetry schemaへ保存しません。保存を無効にする場合は `--no-telemetry` を使い、集計は `jevx stats --json` で確認します。

```bash
jevx stats --json
jevx skills suggest --prompt "テストを追加して失敗原因を調べたい" --json --no-telemetry
```

`--no-telemetry` はローカルJSONL記録を止めるだけで、Gatewayへの外部送信を止めるスイッチではありません。外部送信も避ける場合は、APIキーなしの `eval --dry-run`、一覧、診断、または明示Skillを使ってください。

Hookを導入するとCodex設定ファイルが変更されます。最初は `--dry-run` で内容を確認し、使い捨ての `CODEX_HOME` で発火とログ相関を検証してください。詳細は[Codex CLI compaction実運用runbook](../operators/jevx-compaction-operations.md)にあります。

## jevxを試す

### ユーザー領域へセットアップ

Rust stableとCargoを用意し、リポジトリのルートで実行します。`setup.sh` の主な引数は `--scope user|project [--repo PATH] [--hooks]` で、ヘルプは `--help` / `-h` で表示できます。ここではworktreeの`target`に依存しないrelease配置を `$HOME/.local/bin` に固定します。別の配置を使う場合は `JEVX_INSTALL_ROOT` を変更してください。

```bash
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope user
export PATH="$jevx_install_root/bin:$PATH"
jevx doctor --json
```

Jevを使った提案にはAPIキーが必要です。値をコミット、ログ出力、画面共有しないでください。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
jevx skills suggest --prompt "PDFを結合して内容を確認したい" --json
```

プロジェクトだけに導入する場合は次を使います。`--repo` はproject scopeのSkill配置先を指定します。

```bash
jevx_install_root="${JEVX_INSTALL_ROOT:-$HOME/.local}"
JEVX_INSTALL_ROOT="$jevx_install_root" sh jevx/scripts/setup.sh --scope project --repo "$PWD"
```

### 外部送信なしで確認する

APIキーなしで評価Runnerの構造とローカルベースラインを確認するには、`eval --dry-run --json` を使います。Jevモードは `not_run` と表示されます。

```bash
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

### 導入前の確認項目

- [ ] `jevx doctor --json` が成功する。
- [ ] APIキーを環境変数だけで設定し、履歴やログへ出していない。
- [ ] `--no-telemetry` と `jevx stats --json` の違いを確認した。
- [ ] `--no-telemetry` は外部送信停止ではなく、Telemetryのprobabilityも保存されないことを確認した。
- [ ] Jev送信境界（raw cwd、Skill metadata、redacted prompt/description、非送信の会話・Tool結果）を確認した。
- [ ] 固定fixtureの結果を自分のケースの保証として扱っていない。
- [ ] Hook導入前に `hooks install --dry-run --json` を確認した。
- [ ] 実セッションではCodexの `/hooks` でTrustを確認した。

## 次に読む

- [ドキュメント入口](../README.md)
- [jevx 要件定義と実装状況](../developers/jevx-requirements.md)
- [jevx アーキテクチャ](../developers/jevx-architecture.md)
- [jevx 評価計画](../developers/jevx-evaluation.md)
- [Codex CLI compaction実運用runbook](../operators/jevx-compaction-operations.md)
- [Jevあり/なしの導入効果評価](../research/evaluations/jevx-comfort-evaluation.md)
