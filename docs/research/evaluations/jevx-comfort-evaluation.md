# jevx 導入効果評価：Jevあり / なしで何が変わるか

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な評価スナップショット
> 再現方法: コマンドは記録時点の履歴。現行導入はユーザー向けガイドを参照。
> 注意: 数値と条件は記録時点の観測値であり、現行環境の保証ではない。現行導入・再検証のcanonical手順は[現行compaction runbook](../../operators/jevx-compaction-operations.md)と[Latest評価baseline](../../developers/jevx-evaluation.md)を参照する。

## 結論

`jevx`をCodex CLIのSkill選択補助として使う価値は、今回の固定fixtureでは「ローカル候補を作る」ことよりも、「候補から選ぶ・選ばないをJevが安全側に判断する」ことにあったよ。

- 選択なしは正解率10.0%、ローカルキーワード方式は90.0%だった。
- jevx + Jevは単回40ケースで100.0%、5回×40ケースでは平均98.0%だった。
- `none` recall（互換JSONキー `nonePrecision`、`noneCorrect / expectedNone`）はjevxが100.0%、local_keywordが75.0%だった。
- 代わりにjevxは外部通信とToken使用量が必要で、成功ケースのJev p50/p95は単回407/673ms、5回集計427/610msだった。
- ローカル候補探索は5回測定のp95が2msで、今回の主な追加コストは探索ではなくJev/Gatewayの応答だった。

したがって、jevxは「常に速くなるツール」ではなく、Skill選択の精度・`none`の安全側判定・計測可能性を、外部通信コストと交換する補助線として評価するのが正確だよ。

## 比較対象

同じ40ケース、同じ9件の評価用Skillカタログ、同じ期待ラベルを次の方式で比較した。

| 方式 | 動作 | Jev/Gateway | 目的 |
| --- | --- | --- | --- |
| `none` | 常にSkillを選ばない | なし | Skill選択をしない下限ベースライン |
| `local_keyword` | SkillのID・name・descriptionの単純一致でTop-1を決める（fixture `keywords` は未使用） | なし | 外部通信なしの単純ベースライン |
| `local_rank` | jevxのローカル候補ランキングをTop-1として測る | なし | 候補探索と最終判断を切り分ける |
| `jevx` | ローカルで候補を絞り、Jev Choice + 確率/margin閾値で判断 | あり | 実運用候補 |

旧レポートでprecisionと呼んでいた値の実体は、互換JSONキー `nonePrecision` の none recall、つまり「期待ラベルが`none`のケースを`none`にできた割合」（`noneCorrect / expectedNone`）だよ。通常の「none予測全体を分母にするprecision」ではない。fixtureの期待ラベルは合成データであり、実ユーザーの品質保証やモデルの一般性能を意味しないよ。

## 実測結果

### 単回40ケース

APIキーありで40ケースを1回実行した。評価RunnerはTelemetryを無効化し、単回`eval`のcase JSONLには`id` / `kind` / `expected` / `decision` / `metrics` / `error`などだけを残す。prompt / keywords / response本文は保存せず、Jev requestの入力項目とは別の保存schemaとして扱う。

| モード | 件数 | 正解率 | `nonePrecision`（expectedNone分母のrecall） | candidate miss | error rate | Jev p50 / p95 | total p50 / p95 | 平均 input / output |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `none` | 40 | 10.00% | 100.00% | — | 0.00% | — | — | — |
| `local_keyword` | 40 | 90.00% | 75.00% | — | 0.00% | — | — | — |
| `jevx` | 40 | **100.00%** | **100.00%** | **0.00%** | **0.00%** | **407 / 673ms** | **409 / 675ms** | **2401.175 / 127.675** |

この回はlocal_keywordから正解率が10ポイント、`none` recallが25ポイント改善した。candidate missが0%なので、改善の主因は「期待Skillが候補に入った後のJev判断」と解釈できる。

### 5回×40ケース

同じ条件を5回繰り返し、合計200ケースで分散を確認した。

| モード | Accuracy 平均 | `nonePrecision`（expectedNone分母のrecall） | candidate miss | error rate | 探索 p50 / p95 | Jev p50 / p95 | total p50 / p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `none` | 10.0% | 100.0% | — | 0.0% | — | — | — |
| `local_keyword` | 90.0% | 75.0% | — | 0.0% | — | — | — |
| `local_rank` | 17.5% | 75.0% | 0.0% | 0.0% | 2 / 2ms | — | 2 / 2ms |
| `jevx` | **98.0%** | **100.0%** | **0.0%** | **2.0%** | **2 / 2ms** | **427 / 610ms** | **429 / 612ms** |

jevxの4件のProvider errorは正解率を下げた一方、Jev応答とusageが得られないため時間・Token分布は成功196件を分母にしている。これは「品質」と「サービス到達性」を分けて見るための扱いだよ。

### Token使用量

単回実測の平均はinput 2,401.175 / output 127.675 tokens、5回集計ではinput 2,401.1 / output 127.7 tokensだった。候補Skillの説明をChoice criteriaへ含める設計なので、Tokenコストは候補数・説明長・モデル・Gateway実装に依存する。

日常利用で見るべき値は、単なる1回の平均ではなく次の組み合わせ。

| 観測 | 判断 |
| --- | --- |
| `jevResponseMs` p95 | Hookへ同期接続しても体感を阻害しないか |
| `totalMs` p95 | Skill探索を含む実際の追加待ち時間 |
| `errorRate` | Gateway障害時にshadow継続できるか |
| input/output tokens | 候補説明と選択補助のコストに見合うか |
| `noneRate` / `selectedRate` | 過剰推薦・未推薦の変化がないか |

`jevx stats --json`はTelemetryから判定率、Jev/totalのp50・p95、平均Token、usage event数を集計する。prompt本文は保存しない。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  stats --input /tmp/jevx-events.jsonl --json
```

## HookとCompactionへの効果

Skill選択の比較とは別に、Codex CLIを快適にする運用面も測定対象にした。

### Hook設定

`hooks install`を使うと、既存の`hooks.json`を全置換せず、jevxが生成したhandlerだけを置き換えて次を登録できる。

| Event | 役割 | Jev送信 |
| --- | --- | --- |
| `SessionStart` | compact後のcheckpointを補助contextへ復元 | なし |
| `PreCompact` | compact前のmetadataを記録 | なし |
| `PostCompact` | compact後のmetadataを記録 | なし |
| `UserPromptSubmit` | Skill候補とJev判定をshadow観測 | APIキー設定時はあり |

初回の既存設定は`hooks.json.jevx.bak`に退避し、再実行は冪等。`--dry-run`ではファイルを書かない。これは「導入のしやすさ」と「既存Codex設定を壊さないこと」の改善で、Jevの精度改善とは別の価値だよ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope project --repo "$PWD" --dry-run --json
```

実行後はCodexの`/hooks`でreview/trustする。project-local HookはTrustされるまで発火しないことがあるため、設定ファイルの存在だけで成功判定しない。

### Compaction補助

`compact-assist`はLLM要約ではなく、次の決定的な処理を行う。

1. `PreCompact` / `PostCompact`をcheckpointとして記録する。
2. `SessionStart(source=compact)`で同じsessionの最新metadataを探す。
3. 任意の`.jevx/compact-context.md`をredactし、最大4,000文字の補助contextとして返す。
4. checkpointにはmanifest本文ではなく、redacted本文のSHA-256・文字数・有無だけを保存する。

```bash
mkdir -p .jevx
printf '%s\n' 'goal: preserve release checklist' 'next: run tests' \
  > .jevx/compact-context.md
printf '%s\n' \
  '{"hook_event_name":"SessionStart","source":"compact","cwd":"'"$PWD"'"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir /tmp/jevx-compaction
```

これは会話履歴の完全復元、Codexの公式Compaction、長い会話の意味保持を証明するものではない。`additionalContext`は「補助」と明記し、現在のリポジトリ・会話・制約を再確認する設計にしている。

## 再現手順

### 外部送信なし

```bash
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks compact-eval --runs 5 --json
```

### APIキーあり

APIキーはシェル環境にだけ設定し、値をコマンド履歴・ログ・Gitへ書かない。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --json \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --output /tmp/jevx-eval-cases.jsonl \
  > /tmp/jevx-eval-summary.json

cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat --runs 5 --json \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --output /tmp/jevx-repeat.json \
  > /tmp/jevx-repeat-summary.json
```

`--skill-dir`は既定のSkill rootを置き換えず、追加のrootとして扱われる。カタログを隔離して再現する場合は、[評価Runnerの隔離root手順](../../developers/jevx-evaluation.md#隔離したskill-rootでの実行)に従って一時`HOME`・`CODEX_HOME`・空のプロジェクトを用意すること。`eval-repeat --output`はcase JSONLではなくrun/mode集計JSONで、prompt / keywords / response本文を保存しない。

JSON確認には `jq`（macOSなら`brew install jq`）が必要です。確認する条件は次のとおり。

```bash
jq -e '.caseCount == 40 and .modes.jevx.status == "completed"' \
  /tmp/jevx-eval-summary.json
jq -s -e '([.[] | select(has("prompt") or has("keywords"))] | length) == 0' \
  /tmp/jevx-eval-cases.jsonl
```

### セキュリティ確認

- `AI_GATEWAY_API_KEY`が出力、Telemetry、レポートへ現れない。
- eval runnerの保存レポートへprompt本文、keywords、response本文、Skill本文、Tool結果、会話全文、Jevのprobabilityを保存しない。
- Hook recordのsession / turn / modelはSHA-256だけにする。
- compact checkpointに生のmanifestやfixture秘密値がない。
- 実測の生データは`/tmp`などの一時領域へ置き、評価後に削除する。

この確認はeval runner出力の範囲に限る。通常の`skills suggest`や`UserPromptSubmit` Telemetryを実測して安全性を保証するものではなく、Hookのrawな`trigger` / `source` / `selectedSkill`をhardening済みとみなすものでもない。

## 判断基準

jevxを日常導入する判断は、正解率だけで決めない。

| 導入してよい兆候 | 導入を止めて再測定する兆候 |
| --- | --- |
| local_keywordより精度・`none` recallが改善 | 実ユーザーに近いケースで改善が再現しない |
| total p95が許容内 | Gateway error / timeoutが継続する |
| candidate missが低い | Skill説明が長くTokenコストだけ増える |
| Hookがcontinueを返し、生データを保存しない | Hook Trustや設定mergeを検証できない |
| compact後に目的・next actionを補助できる | manifestを会話の完全な代替として扱ってしまう |

今回の固定fixtureでは導入候補と判断できるが、常用前に匿名化した実利用ケース、Gateway障害時、複数Hook、長い会話、コスト上限を追加評価するのが次の一手だよ。

## 参考

- [jevx APIキーあり単回実測](jevx-live-evaluation-2026-09-21.md)
- [jevx APIキーあり5回実測](jevx-variance-evaluation-2026-09-21.md)
- [jevx要件定義](../../developers/jevx-requirements.md)
- [jevxアーキテクチャ](../../developers/jevx-architecture.md)
- [Codex Hook shadow / compact評価](jevx-codex-hooks-evaluation.md)
- [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)
- [OpenAI Compaction公式ガイド](https://developers.openai.com/api/docs/guides/compaction)
