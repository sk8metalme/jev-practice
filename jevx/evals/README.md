# jevx evaluation fixtures

`skill-selection.jsonl`は、jevxのSkill選択を比較測定するための秘密情報なしのケース集だよ。

- `synthetic`: 30件の合成ケース
- `anonymized-template`: 10件の匿名化形式テンプレートケース

ケースは正解ラベルを含むので、Jev requestからはfixtureの`id`・`kind`・`expected`・`keywords`を除外する。`prompt`と`cwd`はredact後、候補Skill metadataはrequestへ送る。`keywords` はfixture作成者の注釈・期待根拠として読み込まれるが、現行 `local_keyword` の計算には使われず、Skill ID/name/descriptionから作られるローカル一致とは別物である。単回`eval --output`のcase JSONLには`id`・`kind`・`expected`・判定・metrics・error codeを保存するが、prompt本文・keywords・Jev response本文は保存しない。

JSONLの検証:

```bash
jq -e -s 'length == 40 and all(.[]; .id and .prompt and .expected)' \
  jevx/evals/skill-selection.jsonl >/dev/null
```

## 評価Runner

リポジトリのルートから、APIキーなしでCurrent / Latest baselineを再現できる。jevxの現行Rust CLIとCLI helpに合わせたcanonical commandは次のとおり。

```bash
JEVX_TELEMETRY=off cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval --dry-run --json
```

2026-09-22時点のCurrent baselineは commit `ed0b93c023251400bcbbe2de8cdedd5451f08761`、fixture SHA-256 `a450a48fac7b49b544002f8c539b961fef0352951cde950f692768b4ea0748fb`、`rustc/cargo 1.98.1`、macOS 26.6.2（build 25G83）、APIキーなし、外部Gateway呼び出しなし。値は実行日時、OS、catalog rootsに依存するため、完全な隔離条件は [評価計画](../../docs/developers/jevx-evaluation.md) のCurrent baseline表を参照する。

2026-09-21付のAPIキーあり単回・複数回の表はHistorical snapshotで、現行baselineではない。歴史値を引用するときは各レポートの実行日・API key設定有無・commit状態を併記する。

`--dry-run`では次の4モードを同じ40ケースで比較する。

- `none`: 常にSkillを選ばない
- `local_keyword`: Skillの`name`と`description`に対するローカル一致
- `local_rank`: 現行ローカルランキングでTop-1を返す（外部送信なし）
- `jevx`: `status: not_run`。Jevのネットワーク呼び出しを行わない

レポートの `nonePrecision` は互換JSONキーで、意味は `noneCorrect / expectedNone` の recall（`expected: none` を正しく `none` とした割合）です。通常のprecisionとして解釈しないでください。

再現時は注意が必要です。`--skill-dir` は project `.agents/skills` / `.codex/skills`、`$HOME/.agents/skills`、`$CODEX_HOME/skills` を置き換えず追加します。したがって、fixtureだけのcatalogを再現するには一時HOME・CODEX_HOME・空の `--cwd` project rootを用意し、`--fixtures` と `--skill-dir` に絶対パスを渡してください。詳細な隔離コマンドと現行baselineのcommit、fixture SHA-256、Rust/Cargo、macOS、実行条件は [評価計画](../../docs/developers/jevx-evaluation.md) を正本とします。

実測するときは`--dry-run`を外し、APIキーと評価用Skillカタログを指定する。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json \
  --output /tmp/jevx-eval-results.jsonl
```

標準出力のレポートにはモード別の正解率、`none` recall（互換キー `nonePrecision`）、候補取りこぼし率、エラー率、fallback率、cache hit率、retry率、平均retry、Jev応答時間・全体時間のp50/p95、token平均、relative costが含まれる。単回`eval --output`のcase JSONLは`id`・`kind`・`expected`・判定・metrics・error codeを保存し、`prompt`・`keywords`・Jev response本文は保存しない。Telemetry schemaにもGatewayのprobabilityを保存しない。

Decision Contractの安全なreceipt schemaを確認するfixtureは [`decision-contract-baseline.jsonl`](decision-contract-baseline.jsonl) である。これは実APIの応答ログではなく、秘密情報を含まないaccepted/noneの記録例で、prompt本文・Skill本文・Tool結果・APIキーを含めない。対応するpayloadと予算は [`decision-contract-baseline-state.jsonl`](decision-contract-baseline-state.jsonl) に分離し、`decision_contract_requirements`のテストで実際の`DecisionReceipt::from_result`生成値と`replay_receipt`を検証する。実行時のreceiptは通常 `$JEVX_HOME/decisions.jsonl` に追記される。

同一条件で分散を確認するには`eval-repeat`を使う。以下はAPIキーありで5回、合計200ケースを測定する例。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  eval-repeat --runs 5 \
  --fixtures jevx/evals/skill-selection.jsonl \
  --skill-dir jevx/evals/skills \
  --json --output /tmp/jevx-repeat.json
```

`eval-repeat --output`はcase JSONLではなく、run/mode集計（runごとのmode summaryを含む）と、`local_rank` / `jevx` が計測するケースの遅延・token分布（mean / stddev / min / max / p50 / p95）をJSONで保持する。prompt・keywords・response本文・case JSONLは保存しない。実測値は[`docs/research/evaluations/jevx-variance-evaluation-2026-09-21.md`](../../docs/research/evaluations/jevx-variance-evaluation-2026-09-21.md)を参照してね。

反復runの独立性を保つため、`evaluate_repeated`はprocess共有Decision cacheを無効化して各runを実行する。cache hitの単回実行では、外部Jevの`jevResponseMs`とrelative costを0として集計し、ローカル処理時間は`totalMs`へ残す。
