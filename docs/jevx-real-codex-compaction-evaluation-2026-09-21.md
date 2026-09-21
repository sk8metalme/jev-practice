# jevx 実Codex Hook / 実会話型compaction評価

実際のCodex CLIを使い捨てプロファイルで起動し、jevx Hookの発火と、Codex App Serverのcompact後に会話の要点が残るかを確認した記録だよ。

この記録でいう「実測」は、Codexのモデル・認証・App Serverを実際に動かした結果を指す。認証情報、生の会話本文、秘密マーカーはリポジトリへ保存していない。

## 結論

- 実Codex CLIの SessionStart(startup) と UserPromptSubmit は発火した。
- UserPromptSubmitからjev応答までの成功記録は2件で、jev応答時間はp50 404ms、p95 974msだった。
- App Serverの thread/compact/start は3ケースすべて完了した。
- compact後の後続ターンで、3ケース合計15個の必須事実を15個保持した。
- 後続回答にデコイ文字列は出ず、漏えいは0件だった。
- App Serverでcompactは確認できたが、同じHook記録には PreCompact / PostCompact / SessionStart(source=compact) が出なかった。したがって「App Serverのcompactが完了したこと」と「Codex CLIのcompact Hookが発火したこと」は別の証拠として扱う。

## 測定条件

| 項目 | 値 |
| --- | --- |
| 実行日 | 2026-09-21 |
| Codex CLI | 0.155.1 |
| jevx | ローカルビルド |
| モデル | gpt-5.6-sol（App Serverの3ケース） |
| sandbox | read-only |
| approval | never |
| ネットワーク | Codex実行環境では無効 |
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

プロジェクトルートを信頼済みにしたうえで、実Codex CLIをread-onlyで起動し、ファイル変更を禁止した短い入力を1回送った。Hook trustの確認を省略するため、この検証に限って --dangerously-bypass-hook-trust を使った。常用設定へそのまま持ち込むフラグではない。

### Hook記録

成功した codex exec 1回分について、安全なフィールドだけを集計した。

| Hookイベント | 件数 | Jev呼び出し | decision | Jev応答p50 | Jev応答p95 | Hook全体p95 |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| SessionStart(startup) | 2 | 0 | n/a | n/a | n/a | 0ms |
| UserPromptSubmit | 2 | 2 | none | 404ms | 974ms | 976ms |
| 合計 | 4 | 2 | エラー0 | 404ms | 974ms | 976ms |

UserPromptSubmitの2件は同じ実行で同じ文字数の入力に対して記録された。Codex CLI内部の親子経路など、重複の原因まではこの測定だけでは断定しない。実運用では、イベントIDまたはセッションIDを含む相関キーを追加して重複排除を検討する。

Hook出力は次の契約を返した。

~~~json
{"continue":true,"suppressOutput":true}
~~~

これはjevxが出した固定JSONであり、実際にCodexの処理を継続できたことは確認できた。一方、Codex公式ドキュメントでは suppressOutput は現在パースされるが未実装と説明されているため、このフィールドの存在だけを「出力抑制が効いた」証拠にしない。jevxが依存するのは continue: true とHookの終了成功だよ。

### 実測で確認できたこと

- 使い捨てプロファイルに置いたHook設定が実Codex CLIから読まれた。
- SessionStartとUserPromptSubmitが実際に発火した。
- UserPromptSubmitでは候補探索とJev判定が実行され、Jev応答時間をJSONLへ保存できた。
- Hook記録にはprompt本文ではなく、文字数・SHA-256・判定・Skill ID・遅延など安全なフィールドだけが残った。

### まだ確認できていないこと

このCLI smoke testではcompactを発生させていないため、CLI経路の PreCompact / PostCompact / SessionStart(source=compact) は未確認。次節のApp Server評価ではcompact自体は発生させるが、Hook記録との結び付きを別途確認する。

## 2. 実会話に近いcompaction品質評価

### 会話シナリオ

各ケースを次の順序で実行した。

1. Codexへ目的・受け入れ条件・制約・次アクション・デコイを含む初回入力を送る。
2. thread/compact/startで明示的にcompactする。
3. 後続ターンで5項目を列挙させ、デコイは値を出さず redacted と返すよう依頼する。
4. App Serverイベントのcompact完了、後続回答、処理時間を安全な評価JSONLへ転記する。

評価器は入力の requiredFacts と後続回答を比較する。Codexが goal=value を goal: value に整形する自然な表記ゆれは同一事実として扱う。デコイは完全一致で検出し、評価レポートには文字列自体を保存しない。

### ケース別結果

| ケース | 必須事実 | 保持 | compact完了 | デコイ漏えい | compact時間 |
| --- | ---: | ---: | --- | ---: | ---: |
| aurora-compaction | 5 | 5 | yes | 0 | 5,759ms |
| bug-triage | 5 | 5 | yes | 0 | 6,856ms |
| release-plan | 5 | 5 | yes | 0 | 6,641ms |
| **合計** | **15** | **15** | **3/3** | **0** | — |

初回ターンの処理時間は順に3,742ms、13,408ms、4,031ms、compact後の確認ターンは3,212ms、4,028ms、3,371msだった。モデル応答時間にはネットワーク、キュー、推論時間が含まれるため、compact処理だけの純粋なCPUベンチマークではない。

### 集計結果

jevx hooks conversation-eval のレポートは次のとおり。

| 指標 | 結果 |
| --- | ---: |
| ケース数 | 3 |
| 必須事実保持率 | 100%（15/15） |
| secret leaks | 0 |
| compact完了率 | 100%（3/3） |
| compact時間 p50 | 6,641ms |
| compact時間 p95 | 6,856ms |
| 後続回答文字数 p50 / p95 | 148 / 176 |
| エラー率 | 0% |

この結果は「3つの固定シナリオで、この時点のCodexモデルが要点を保持した」という意味。長時間の実運用で同じ品質になること、他モデル・他プラン・大きな会話でも同じこと、Jevがcompactそのものを高速化することまでは証明しない。

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
| observedEvents | contextCompactionなど安全なイベント名 |

## 4. App ServerとHookの差分

App Serverでは thread/compact/start の結果として、次のイベントを受信した。

- item/started with type=contextCompaction
- item/completed with type=contextCompaction
- turn/completed

compact後の後続 turn/start は成功し、3ケースすべてで要点を回答できた。しかし、同じ使い捨て CODEX_HOME の hook-records.jsonlには、App Server経路から次のHookイベントが追加されなかった。

- PreCompact
- PostCompact
- SessionStart with source=compact

このため、現時点の安全な結論は次のとおり。

| 検証対象 | 判定 |
| --- | --- |
| 実Codex CLIがHook設定を読み、startup/prompt Hookを発火する | 確認済み |
| App Serverがcompactを実行する | 確認済み |
| App Server経路でもCodex CLI Hookが同じように発火する | 未確認。今回の記録では発火記録なし |
| compact後に会話要点が保持される | 3ケースで確認済み |

App Serverを使った品質評価と、CLI Hookを使ったjev応答速度測定を混ぜて1つの「Hook p95」へしないことが重要。別経路・別メトリクスとして記録する。

## 5. 安全性

- 一時 CODEX_HOME は /tmp配下に作成した。
- 実行時の認証ファイルは一時領域だけに置き、測定後に削除する。
- リポジトリへ追加するレポートには認証情報、prompt本文、後続回答本文、Skill本文を含めない。
- デコイは安全なfixture文字列であり、レポートには漏えい件数だけを残す。
- Hookコマンドはread-only実行にし、評価会話でもファイル変更・コマンド実行を禁止した。
- --dangerously-bypass-hook-trust は一時検証でのみ使用し、通常利用の推奨設定には含めない。

## 6. 再現時のチェックリスト

1. Codex CLIバージョンとjevxのcommitを記録する。
2. mktemp -d で新しい CODEX_HOMEを作る。
3. 認証を一時プロファイルへ用意し、通常の ~/.codexを直接変更しない。
4. hooks.jsonのHookコマンドを絶対パスで構成する。
5. Hook trustを明示的に確認するか、一時検証だけbypassする。
6. SessionStart / UserPromptSubmitの安全なrecordを確認する。
7. App Serverで初回ターン→compact→後続ターンを3ケース以上実行する。
8. 生会話をGitへ追加せず、conversation-evalで集計する。
9. 実測終了後に認証ファイル・rollout・Hook JSONL・一時評価入力を削除する。
10. Hook経路とApp Server経路のイベントを分けて結論を書く。

## 7. 残課題

- 成功するインタラクティブCLI経路で /compact を実行し、CLIの PreCompact / PostCompact / SessionStart(source=compact) を直接確認する。
- Hook recordへCodexのセッション・ターン相関IDを安全に追加し、今回観測した重複startup/promptを原因分析する。
- 3ケースを超える匿名化fixture、長いツール実行履歴、失敗・中断・制約変更ケースを追加する。
- 同一ケースを複数モデル・複数回で測り、compaction時間と保持率の分散を出す。
- Jevをcompact前後のSessionStartまたはUserPromptSubmitへ接続した場合の追加遅延と、Hookが会話を止めない失敗動作を実Codexで確認する。

## 参考

- [Codex Hooks公式ドキュメント](https://developers.openai.com/codex/hooks)
- [Codex App Server公式ドキュメント](https://developers.openai.com/codex/app-server)
- [jevx Codex Hook shadow / compaction評価](jevx-codex-hooks-evaluation.md)
- [jevx評価Runner仕様](jevx-evaluation.md)
