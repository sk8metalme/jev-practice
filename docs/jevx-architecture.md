# jevx v1 アーキテクチャ

## 目的と境界

jevxはCodex CLIの前段に置く「Skill選択の補助線」だよ。現在の依頼から候補Skillを探索し、ローカルの軽量スコアで上位候補を絞ってから、候補を1回のbatched ChoiceとしてJevへ渡す。

v1の出力は提案だけで、Skill本文のロード、実行、Hookによる自動介入、Compaction、過去会話の要約は行わない。これにより、Jevの誤判定がCodexの実行権限へ直接つながらない。

## 処理フロー

```text
ユーザー依頼
    │
    ├─ 入力検証・秘密らしいtokenのマスキング
    ├─ .agents/skills / .codex/skills を探索
    ├─ name・descriptionのローカルスコアリング
    ├─ 上位32件へ縮約
    ├─ 1回のJev Choice（候補 + none）
    ├─ probability >= 0.60 かつ margin >= 0.10 を確認
    │       ├─ 条件成立: selected
    │       └─ それ以外: none（理由コード付き）
    └─ JSON/human出力 + メタデータTelemetry
```

## モジュール責務

| モジュール | 責務 | 外部依存 |
| --- | --- | --- |
| `discovery` | Skillの探索、Frontmatterの検証、優先順位重複排除 | filesystem |
| `ranking` | ローカル候補順位、Jev結果の閾値判定、計測 | `Judge` trait |
| `gateway` | Vercel AI GatewayへのHTTPリクエストとレスポンス変換 | reqwest |
| `telemetry` | JSONL追記とローカル集計 | filesystem |
| `evaluation` | fixture読み込み、ベースライン比較、Jev実測、p50/p95集計 | `Judge` trait、filesystem |
| `cli` | 入力形式、出力形式、終了コード | clap |

中心の`ranking`は`Judge` traitへ依存するオニオン型の内側に置き、テストではネットワークを使わないStub Judgeへ差し替えられるようにしているよ。HTTP境界のレスポンス形状はローカルTCPテストで検証する。

## Skill探索の優先順位

1. 現在のプロジェクト `.agents/skills`
2. 現在のプロジェクト `.codex/skills`
3. ユーザー `~/.agents/skills`
4. `$CODEX_HOME/skills` または `~/.codex/skills`
5. `--skill-dir` で指定した追加ディレクトリ

同じSkill名が複数に存在するときは、優先順位の小さいルートを採用する。SkillファイルはYAML Frontmatterに非空の`name`と`description`があるものだけを候補にする。

## Jevへ渡すデータ

渡すデータは、マスキング済みの依頼文、作業ディレクトリ、候補SkillのID・名前・説明だけ。次のデータはv1で渡さない。

- Skill本文
- 過去の会話全文
- Tool結果やファイル内容
- APIキー
- 生のTelemetry本文

Jev未設定時はローカル推測へフォールバックせず、`missing_api_key`を返す。これは「Jevの有用性を測る」目的で、ローカルだけの結果をJev結果と混同しないためだよ。

## レイテンシ設計

- ローカル候補探索の目標: p95 100ms以下
- Jevを含む全体の目標: p95 2秒以下
- デフォルトHTTPタイムアウト: 1,500ms
- 出力する時刻: `discoveryMs`、`jevResponseMs`、`totalMs`

Jevの呼び出しは候補ごとに繰り返さず、候補を1つのChoice質問へまとめる。これで候補数に比例したネットワーク往復を避けつつ、速度とtoken usageを測定できる。

## 安全側の判定

次の場合は`selected`を返さず`none`にする。

- Jevが`none`を選ぶ
- choiceが欠落する
- 未知のSkill IDを返す
- 1位の確率が0.60未満
- 1位とrunner-upの確率差が0.10未満

明示的な`--skill`指定はJevを呼ばず、`explicit`として返す。利用者の明示指定をモデルの推測で上書きしないためだよ。

## 失敗時の契約

| 状況 | JSON `error.code` | 終了コード |
| --- | --- | ---: |
| 入力不正、APIキーなし、JSON不正 | `invalid_input` / `missing_api_key` / `json_error` | 2 |
| Jev接続失敗、Jev応答エラー | `provider_error` | 3 |
| タイムアウト | `timeout` | 3 |
| ファイル・YAMLエラー | `io_error` / `yaml_error` | 2 |

## 後続拡張

v1で境界を固定したうえで、次は別タスクとしてHook shadow連携、Compaction補助、コンテキスト探索キャッシュ、Tool Result削減を追加する。Skillの自動ロード・実行は、精度・誤作動・権限境界の評価が終わるまで有効化しない。
