# jevx 実Codex Hook / 実会話型compaction評価

> 対象読者: 調査・導入判断・評価担当
> 文書の状態: 歴史的な評価スナップショット
> 再現方法: コマンドは記録時点の履歴。現行導入はユーザー向けガイドを参照。
> 注意: 数値と条件は記録時点の観測値であり、現行環境の保証ではない。本稿はHistorical snapshotで、未実装候補や現行ロードマップではない。現行Hook導入は[canonical runbook](../../operators/jevx-compaction-operations.md)を参照する。

実際のCodex CLIを使い捨てプロファイルで起動し、jevx Hookの発火と、Codex App Serverのcompact後に会話の要点が残るかを確認した記録だよ。再開後は、成功したCLI compactと、認証付き2モデル×3ケースの実会話型評価まで完了した。

この記録でいう「実測」は、Codexのモデル・認証・App Serverを実際に動かした結果を指す。認証情報、生の会話本文、秘密マーカーはリポジトリへ保存していない。

## 結論

- 実Codex CLIの`SessionStart(startup)`、`UserPromptSubmit`、`PreCompact(trigger=manual)`、`PostCompact(trigger=manual)`、`SessionStart(source=compact)`を成功compact経路で確認した。
- 成功compactを含むHook recordは6件で、相関IDの重複グループ0、重複record 0だった。
- UserPromptSubmitのJev応答は2件で、`jevResponseMs`はp50 550ms、p95 889msだった。
- App Serverの`thread/compact/start`は、`gpt-5.6-sol`と`gpt-5.6-terra`の各3ケース、合計6ケースすべて完了した。
- compact後の後続ターンで、合計30個の必須事実を30個保持した。後続回答のデコイ漏えいは0件、エラーは0件だった。
- 追加のresilience評価では各ケースを8ターンに拡張し、ツール履歴12件（失敗6件）、interrupt 6件、復旧turn 18件を観測した。復旧完了率は100%だった。
- 追加評価のpost-compaction cache hit率は全体p50 94.22% / p95 98.41%、uncached inputは397 / 1,509 tokens、billable proxyは577 / 1,677 tokens（各p50 / p95）だった。billable proxyは請求額ではない。
- App ServerのcompactイベントとCodex CLI Hookは別経路であり、App Server側のcompact実行だけではCLI Hook recordは追加されなかった。

## 測定条件

| 項目 | 値 |
| --- | --- |
| 実行日 | 2026-09-21 |
| Codex CLI | 0.155.1 |
| jevx | ローカルビルド |
| モデル | gpt-5.6-sol / gpt-5.6-terra（各3ケース） |
| sandbox | read-only |
| approval | never |
| ネットワーク | 明示承認済みの認証付きCodex外部サービス |
| プロファイル | /tmp配下の使い捨て CODEX_HOME |
| Hook | hooks.jsonへSessionStart / PreCompact / PostCompact / UserPromptSubmitを設定 |
| 評価対象 | Codex CLI Hook smoke test、App Serverの会話→compact→後続ターン |

使い捨て CODEX_HOME は通常の ~/.codex と分離した。ChatGPTログインを使う環境だったため、今回の実測では認証ファイルを一時プロファイルへコピーして実行したが、測定終了後に一時ディレクトリごと削除した。認証ファイルの内容、トークン、APIキーは表示・保存していない。

## 1. 実Codex CLIのHook発火

### 実行したこと

使い捨てプロファイルの hooks.jsonから、次の4イベントを同じjevx shadowコマンドへ接続した。

~~~json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/jevx hooks shadow --skill-dir /absolute/path/to/jevx/evals/skills --output /tmp/jevx-hook-records.jsonl"
          }
        ]
      }
    ],
    "PreCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/jevx hooks shadow --skill-dir /absolute/path/to/jevx/evals/skills --output /tmp/jevx-hook-records.jsonl"
          }
        ]
      }
    ],
    "PostCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/jevx hooks shadow --skill-dir /absolute/path/to/jevx/evals/skills --output /tmp/jevx-hook-records.jsonl"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/jevx hooks shadow --skill-dir /absolute/path/to/jevx/evals/skills --output /tmp/jevx-hook-records.jsonl"
          }
        ]
      }
    ]
  }
}
~~~

プロジェクトルートを信頼済みにしたうえで、実Codex CLIをread-onlyで起動し、`READY`、`/compact`、`AFTER`の順に操作した。画面上で`Context compacted · 2s`を確認し、compact後の応答まで完了した。Hook trustの確認を省略するため、この検証に限って --dangerously-bypass-hook-trust を使った。常用設定へそのまま持ち込むフラグではない。

### Hook記録

成功したインタラクティブCLI実行1回分について、hashed ID、イベント、decision、metricsなど現行schemaのbounded fieldsを集計した。

| Hookイベント | 件数 | Jev呼び出し | decision | Jev応答p50 | Jev応答p95 | Hook全体p95 |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| SessionStart(startup) | 1 | 0 | n/a | n/a | n/a | 0ms |
| UserPromptSubmit | 2 | 2 | none | 550ms | 889ms | 889ms |
| PreCompact(trigger=manual) | 1 | 0 | n/a | n/a | n/a | 0ms |
| PostCompact(trigger=manual) | 1 | 0 | n/a | n/a | n/a | 0ms |
| SessionStart(source=compact) | 1 | 0 | n/a | n/a | n/a | 0ms |
| **合計** | **6** | **2** | **エラー0** | **550ms** | **889ms** | **889ms** |

UserPromptSubmitの2件はcompact前後の入力に対応し、`jevResponseMs`は550msと889ms、`totalMs`は551msと889msだった。今回の6 recordを`hooks correlate`へ渡すと、duplicate group countは0、duplicate record countは0だった。1回の成功compactだけなので、長期運用の重複率や遅延分布までは断定しない。

Hook出力は次の契約を返した。

~~~json
{"continue":true,"suppressOutput":true}
~~~

これはjevxが出した固定JSONであり、実際にCodexの処理を継続できたことは確認できた。一方、Codex公式ドキュメントでは suppressOutput は現在パースされるが未実装と説明されているため、このフィールドの存在だけを「出力抑制が効いた」証拠にしない。jevxが依存するのは continue: true とHookの終了成功だよ。

### 実測で確認できたこと

- 使い捨てプロファイルに置いたHook設定が実Codex CLIから読まれた。
- SessionStart、UserPromptSubmit、PreCompact、PostCompact、SessionStart(source=compact)が実際に発火した。
- UserPromptSubmitでは候補探索とJev判定が実行され、Jev応答時間をJSONLへ保存できた。
- Hook記録にはprompt本文ではなく、文字数・SHA-256・判定・Skill ID・遅延などのmetricsが残る。ただし`trigger` / `source` / `selectedSkill`はwrite時にrawで記録され、trust boundary済みmetadataや値ではない。

### この経路での限界

- 成功compactは1回、UserPromptSubmitは2件なので、Hook重複やJev遅延の統計的な代表性はない。
- App Server経路のcompactイベントは、同じ使い捨てprofileのCLI Hook recordへ追加されなかった。`contextCompaction`とCLI Hookは別経路として扱う。

## 2. 実会話に近いcompaction品質評価

### 会話シナリオ

基礎評価では各ケースを次の順序で実行した。

1. Codexへ目的・受け入れ条件・制約・次アクション・デコイを含む初回入力を送る。
2. thread/compact/startで明示的にcompactする。
3. 後続ターンで5項目を列挙させ、デコイは値を出さず redacted と返すよう依頼する。
4. App Serverイベントのcompact完了、後続回答、処理時間を現行評価schemaのJSONLへ転記する。

評価器は入力の requiredFacts と後続回答を比較する。Codexが goal=value を goal: value に整形する自然な表記ゆれは同一事実として扱う。デコイは完全一致で検出し、評価レポートには文字列自体を保存しない。

### 追加: ツール履歴・失敗復旧・token usage評価

2026-09-21の再開測定では、上記の基礎評価に加えて、実会話に近い復旧シーケンスを2モデル×3ケースで実行した。各ケースの順序は次のとおり。

1. 5つの必須事実を含む初回turnを送る。
2. `thread/compact/start`を実行し、`contextCompaction`の`item/started`と`item/completed`を待つ。
3. `turn/start.toolOutput`で失敗ツール結果を1件注入する。
4. 再試行turn、interruptしたturn、interrupt後の復旧turnを実行する。
5. 成功ツール結果と最終復旧turnを追加し、後続回答で必須事実と復旧状態を確認する。

失敗・成功のtool outputは評価用の固定文字列であり、App Serverからshell、MCP、ファイル操作を実行したわけではない。この制約により、ツール履歴の「復旧制御」とtoken usageを安全に確認しつつ、実ツール副作用を避けている。

#### ケース別結果

| モデル | ケース | turns | contextChars | tool履歴 / 失敗 | interrupt / 復旧turn | compact時間 | post cache hit | uncached / billable proxy | 事実保持 | leak |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| gpt-5.6-sol | aurora-compaction | 8 | 2,030 | 2 / 1 | 1 / 3 | 10,037ms | 98.08% | 306 / 427 | 5/5 | 0 |
| gpt-5.6-sol | bug-triage | 8 | 2,011 | 2 / 1 | 1 / 3 | 5,814ms | 94.22% | 926 / 1,081 | 5/5 | 0 |
| gpt-5.6-sol | release-plan | 8 | 2,020 | 2 / 1 | 1 / 3 | 9,204ms | 98.41% | 252 / 465 | 5/5 | 0 |
| gpt-5.6-terra | aurora-compaction | 8 | 2,030 | 2 / 1 | 1 / 3 | 10,873ms | 93.50% | 1,050 / 1,184 | 5/5 | 0 |
| gpt-5.6-terra | bug-triage | 8 | 2,011 | 2 / 1 | 1 / 3 | 17,275ms | 90.92% | 1,509 / 1,677 | 5/5 | 0 |
| gpt-5.6-terra | release-plan | 8 | 2,020 | 2 / 1 | 1 / 3 | 21,003ms | 97.60% | 397 / 577 | 5/5 | 0 |

#### 追加評価の集計

| 指標 | gpt-5.6-sol | gpt-5.6-terra | 全体 |
| --- | ---: | ---: | ---: |
| compact完了 | 3/3 | 3/3 | 6/6 |
| 必須事実保持 | 15/15 | 15/15 | 30/30 |
| 復旧完了 | 3/3 | 3/3 | 6/6 |
| ツール履歴 / 失敗 | 6 / 3 | 6 / 3 | 12 / 6 |
| interrupted turns | 3 | 3 | 6 |
| recovery turns | 9 | 9 | 18 |
| compact時間 p50 / p95 | 9,204 / 10,037ms | 17,275 / 21,003ms | — |
| post cache hit p50 / p95 | 98.08% / 98.41% | 93.50% / 97.60% | 94.22% / 98.41% |
| uncached input p50 / p95 | 306 / 926 | 1,050 / 1,509 | 397 / 1,509 |
| billable proxy p50 / p95 | 465 / 1,081 | 1,184 / 1,677 | 577 / 1,677 |

usage snapshotはApp Serverの`thread/tokenUsage/updated`から取得した。`post cache hit`は`cachedInputTokens / inputTokens`、`uncached input`は`inputTokens - cachedInputTokens`、`billable proxy`は`uncached input + outputTokens`である。OpenAIのusage説明ではcached tokensはinput tokensに含まれ、reasoning tokensはoutput tokensに含まれるため、reasoningを別に加算していない。これらは比較用のtoken指標で、ChatGPT認証の実測からUSD請求額を推定したものではない。

### 先行の基礎評価: ケース別結果

各モデルで3ケースを実行した。各ケースは6ターン、約19,100文字の合成会話で、compact後に5つの必須事実を列挙させた。

| モデル | ケース | ターン | context文字数 | 必須事実 | 保持 | compact完了 | デコイ漏えい | compact時間 | 後続turn |
| --- | --- | ---: | ---: | ---: | ---: | --- | ---: | ---: | --- |
| gpt-5.6-sol | aurora-compaction | 6 | 19,127 | 5 | 5 | yes | 0 | 5,100ms | yes |
| gpt-5.6-sol | bug-triage | 6 | 19,108 | 5 | 5 | yes | 0 | 6,074ms | yes |
| gpt-5.6-sol | release-plan | 6 | 19,105 | 5 | 5 | yes | 0 | 6,340ms | yes |
| gpt-5.6-terra | aurora-compaction | 6 | 19,127 | 5 | 5 | yes | 0 | 10,331ms | yes |
| gpt-5.6-terra | bug-triage | 6 | 19,108 | 5 | 5 | yes | 0 | 14,086ms | yes |
| gpt-5.6-terra | release-plan | 6 | 19,105 | 5 | 5 | yes | 0 | 14,175ms | yes |
| **合計** | **6ケース** | **—** | **114,680** | **30** | **30** | **6/6** | **0** | **—** | **6/6** |

### 先行の基礎評価: 集計結果

jevx hooks conversation-eval のレポートは次のとおり。

| 指標 | 結果 |
| --- | ---: |
| ケース数 | 6（2モデル×3ケース） |
| 必須事実保持率 | 100%（30/30） |
| secret leaks | 0 |
| compact完了率 | 100%（6/6） |
| gpt-5.6-sol compact時間 p50 / p95 | 6,074ms / 6,340ms |
| gpt-5.6-terra compact時間 p50 / p95 | 14,086ms / 14,175ms |
| エラー率 | 0% |

この結果は「3つの固定シナリオを2モデルで各1回、各6ターン・約19,100文字で実行したとき、この時点のCodexモデルが要点を保持した」という意味。長時間の実運用で同じ品質になること、他モデル・他プラン・大きな会話でも同じこと、Jevがcompactそのものを高速化することまでは証明しない。p50/p95はjevxのnearest-rank方式で、モデルごとにn=3のためp95は最大値になる。

## 3. 評価器の使い方

実Codexの出力から、生の会話をリポジトリへ入れず、次のような一時JSONLを作る。

~~~json
{"caseId":"example","requiredFacts":["goal=keep-context","next=verify"],"followUpText":"goal: keep-context\nnext: verify\ndecoy_marker: redacted","secretMarkers":["LOCAL_DECOY_ONLY"],"compactionCompleted":true,"compactionDurationMs":6200,"inputChars":200,"observedEvents":["contextCompaction","turn/completed"]}
~~~

実行コマンド:

~~~bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks conversation-eval \
  --input /tmp/jevx-conversation-cases.jsonl \
  --json \
  --output /tmp/jevx-conversation-report.json
~~~

出力はケースID、イベント数、必須事実の件数、保持件数、デコイ件数、完了フラグ、文字数、遅延、エラーコードだけ。followUpText、requiredFacts、secretMarkersそのものは出力しない。

入力項目の意味:

| 項目 | 内容 |
| --- | --- |
| caseId | レポートへ残してよい短いASCII識別子 |
| requiredFacts | 保持を確認するキーと値 |
| followUpText | compact後の後続回答。評価後に破棄する一時入力 |
| secretMarkers | 絶対に出てほしくないデコイ。レポートには件数だけ残す |
| compactionCompleted | App Serverのcompact完了イベントを観測できたか |
| compactionDurationMs | contextCompaction開始から完了までの実測時間 |
| inputChars | 初回入力の文字数。本文ではなく数値だけを保存 |
| observedEvents | contextCompactionなど観測対象として許可したイベント名 |

## 4. App ServerとHookの差分

App Serverでは thread/compact/start の結果として、次のイベントを受信した。

- item/started with type=contextCompaction
- item/completed with type=contextCompaction
- turn/completed

compact後の後続 turn/start は成功し、6ケースすべてで要点を回答できた。しかし、同じ使い捨て CODEX_HOME の hook-records.jsonlには、App Server経路から次のHookイベントが追加されなかった。

- PreCompact
- PostCompact
- SessionStart with source=compact

このため、現時点の結論は次のとおり。

| 検証対象 | 判定 |
| --- | --- |
| 実Codex CLIがHook設定を読み、startup/prompt Hookを発火する | 確認済み |
| App Serverがcompactを実行する | 確認済み |
| App Server経路でもCodex CLI Hookが同じように発火する | 未確認。今回の記録では発火記録なし |
| compact後に会話要点が保持される | 6ケースで確認済み |

App Serverを使った品質評価と、CLI Hookを使ったjev応答速度測定を混ぜて1つの「Hook p95」へしないことが重要。別経路・別メトリクスとして記録する。

## 5. 安全性

- 一時 CODEX_HOME は /tmp配下に作成した。
- 実行時の認証ファイルは一時領域だけに置き、測定後に削除した。
- リポジトリへ追加するレポートには認証情報、prompt本文、後続回答本文、Skill本文を含めない。
- デコイは秘密情報を含まないcontrolled fixture文字列であり、レポートには漏えい件数だけを残す。
- Hookコマンドはread-only実行にし、評価会話でもファイル変更・コマンド実行を禁止した。
- --dangerously-bypass-hook-trust は一時検証でのみ使用し、通常利用の推奨設定には含めない。

## 6. 再現時のチェックリスト

1. Codex CLIバージョンとjevxのcommitを記録する。
2. mktemp -d で新しい CODEX_HOMEを作る。
3. 認証を一時プロファイルへ用意し、通常の ~/.codexを直接変更しない。
4. hooks.jsonのHookコマンドを絶対パスで構成する。
5. Hook trustを明示的に確認するか、一時検証だけbypassする。
6. SessionStart / UserPromptSubmit / PreCompact / PostCompactの現行schema recordを確認する。raw metadataを安全・信頼済みと解釈しない。
7. App Serverで初回ターン→compact→後続ターンを複数モデル・3ケース以上実行する。
8. 失敗ツール・interrupt・復旧を含む場合は、toolOutputを合成fixtureとして明記する。
9. 生会話をGitへ追加せず、conversation-evalで集計する。
10. 実測終了後に認証ファイル・rollout・Hook JSONL・一時評価入力を削除する。
11. Hook経路とApp Server経路のイベントを分けて結論を書く。

## 7. 残課題

- 同一ケースを複数回で測り、モデル別のcompaction時間・保持率・Jev遅延の分散を出す。
- 実ツールを実行して得た長い出力、MCP、ファイル編集を含む匿名化fixtureを追加する。
- 10,000文字級の長文コンテキストで、今回のcache・復旧特性が再現するかを確認する。
- App Server経路でJevをcompact前後へ接続する場合は、CLI Hookとは別の統合ポイントと追加遅延を設計・実測する。
- Gateway rate limit、API費用、認証期限切れ、Jev失敗時に会話を止めない動作を長時間実行で確認する。
- Hook recordの`trigger` / `source` / `selectedSkill`をwrite前にハッシュ化・allowlist化し、controlled fixture metadataと外部入力を分離するhardeningは未実装であり、別フォローアップとする。

## 参考

- [Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)
- [Codex App Server公式ドキュメント](https://developers.openai.com/codex/app-server)
- [OpenAI Agents APIのusage・token observability](https://developers.openai.com/api/docs/guides/agents-api/observability)
- [jevx Codex Hook shadow / compaction評価](jevx-codex-hooks-evaluation.md)
- [jevx評価Runner仕様](../../developers/jevx-evaluation.md)
