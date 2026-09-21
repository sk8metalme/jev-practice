# jevx Codex Hook shadow / compaction評価

## 目的と範囲

Codex CLIの日常利用へjevxを接続する前に、Hookへ入れても会話の進行を止めず、安全な観測だけを残せるかを確認する。対象はCodex公式Hook仕様に登場する次のイベントだよ。

- `PreCompact`: compaction前。`trigger` は `auto` または `manual`。
- `PostCompact`: compaction後。`trigger` は `auto` または `manual`。
- `SessionStart`: セッション開始・再開・compact後など。`source` に `startup` / `resume` / `clear` / `compact` が入る。
- `UserPromptSubmit`: ユーザー入力直前。shadow modeではJev判定を観測するだけ。

仕様の一次情報は[Codex Hooks公式ドキュメント](https://learn.chatgpt.com/docs/hooks)を参照した。

`hooks shadow`は引き続きCodex設定を変更しない観測モード。加えて、明示的な`hooks install`で既存設定を保持しながらjevx Hookを登録できるようになった。設定変更を伴うため、`--dry-run`、バックアップ、CodexのHook Trust確認を前提にする。このページのAPIキーあり結果はshadow入力に対するjevxの実行評価で、実Codex CLIとApp Serverを使った実測は[別の詳細記録](jevx-real-codex-compaction-evaluation-2026-09-21.md)へ分けているよ。

## Shadowの動作

```text
Codex Hook JSONL
        │ stdin
        ▼
jevx hooks shadow
        │
        ├─ SessionStart / PreCompact / PostCompact
        │    └─ metadataだけを記録、Jevは呼ばない
        │
        └─ UserPromptSubmit
             └─ 候補探索 + Jev判定を実行（shadow）

stdout: {"continue":true,"suppressOutput":true}
記録: promptのSHA-256・文字数・判定・Skill ID・遅延・usage・errorCodeのみ
```

成功・失敗に関係なく、shadowのstdoutは次の固定契約を返す。`additionalContext`を返さず、会話本文を書き換えず、HookからCodexの処理を停止しない。`suppressOutput`は互換性のため出力しているが、Codex公式ドキュメントでは現在パースされるだけで未実装と説明されているため、出力抑制の保証として扱わない。

```json
{"continue":true,"suppressOutput":true}
```

`UserPromptSubmit`でAPIキーが未設定、promptが空、Providerがエラーになった場合も、失敗理由を安全な`errorCode`へ変換して記録し、stdoutの`continue`は維持する。生prompt、APIキー、Skill本文は記録しない。

## ローカル実行例

プロジェクトルートで、Hook入力を1行JSONとして渡せる。実際のCodex設定にはまだ登録せず、まずshadow出力を確認する。

```bash
export AI_GATEWAY_API_KEY="<your-ai-gateway-api-key>"

printf '%s\n' \
  '{"hook_event_name":"UserPromptSubmit","prompt":"PDFを結合して内容を確認したい","cwd":"/tmp/sample-repo"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow \
      --event UserPromptSubmit \
      --skill-dir jevx/evals/skills \
      --output /tmp/jevx-hooks.jsonl
```

compaction lifecycle eventは、Jevを呼ばずに次のように検査できる。

```bash
printf '%s\n' '{"hook_event_name":"PreCompact","trigger":"auto"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow --event PreCompact --output /tmp/jevx-hooks.jsonl

printf '%s\n' '{"hook_event_name":"PostCompact","trigger":"manual"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow --event PostCompact --output /tmp/jevx-hooks.jsonl

printf '%s\n' '{"hook_event_name":"SessionStart","source":"compact"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks shadow --event SessionStart --output /tmp/jevx-hooks.jsonl
```

Codexの`hooks.json`へ手で接続する場合のshadow-only最小例は次のようになる。実際には`jevx`の絶対パス、書き込み先、プロジェクトのTrust設定を環境に合わせて決める。Compaction補助を含む安全なmergeは、下記の`hooks install`を明示的に実行する方法が使える。

```json
{
  "hooks": {
    "PreCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "jevx hooks shadow --event PreCompact --output $HOME/.jevx/hooks.jsonl"
          }
        ]
      }
    ],
    "PostCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "jevx hooks shadow --event PostCompact --output $HOME/.jevx/hooks.jsonl"
          }
        ]
      }
    ],
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "jevx hooks shadow --event SessionStart --output $HOME/.jevx/hooks.jsonl"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "jevx hooks shadow --event UserPromptSubmit --skill-dir <repo-root>/jevx/evals/skills --output $HOME/.jevx/hooks.jsonl"
          }
        ]
      }
    ]
  }
}
```

実運用で有効化する前に、Codex公式ドキュメントのHook trust・matcher・`additionalContext`上限を確認し、使い捨てのCodex profileでイベント発火とstdoutを検証すること。

## Hook設定の自動merge（opt-in）

既存のHookを保ちながらjevxを登録する場合は、まず生成結果を確認する。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks install --scope project --repo "$PWD" --dry-run --json
```

実書き込みでは、次の4イベントを追加する。

| イベント | matcher | command |
| --- | --- | --- |
| `SessionStart` | `startup\|resume\|clear\|compact` | `hooks compact-assist` |
| `PreCompact` | `manual\|auto` | `hooks compact-assist` |
| `PostCompact` | `manual\|auto` | `hooks compact-assist` |
| `UserPromptSubmit` | なし | `hooks shadow` |

user scopeの保存先は`$CODEX_HOME/hooks.json`（未設定時`~/.codex/hooks.json`）、project scopeは`<repo>/.codex/hooks.json`。既存root・未知イベント・カスタムhandlerを保持し、jevxのmarkerを含む古いcommandだけ置換する。変更前のファイルは初回だけ`hooks.json.jevx.bak`へ退避し、再実行は冪等だよ。

`jevx/scripts/setup.sh --scope user --hooks` はadvisory Skillの導入後に同じ登録を行う。設定を書けてもCodexが信頼したとは限らないので、`/hooks`でreview/trustし、使い捨て`CODEX_HOME`でstartup→prompt→`/compact`→後続入力の順に発火を確認する。

## `compact-assist` の動作と限界

`compact-assist`は、`PreCompact`と`PostCompact`でcheckpointを追記し、`SessionStart(source=compact)`で最新metadataを参照する。任意の`<cwd>/.jevx/compact-context.md`はredactして最大4,000文字まで補助contextへ入るが、生のmanifestはcheckpointへ保存しない。

```bash
mkdir -p .jevx
printf '%s\n' 'goal: preserve the release checklist' 'next: run tests' \
  > .jevx/compact-context.md
printf '%s\n' \
  '{"hook_event_name":"SessionStart","source":"compact","cwd":"'"$PWD"'"}' \
  | cargo run --locked --manifest-path jevx/Cargo.toml -- \
      hooks compact-assist --state-dir /tmp/jevx-compaction
```

出力の`hookSpecificOutput.additionalContext`は、`checkpoint event=...`とredacted manifestを含む補助情報になる。会話履歴を要約・復元するLLMではなく、Codex公式Compactionの代替でもないため、現在の会話とリポジトリを確認する指示を常に含める。`suppressOutput`は公式ドキュメント上パースされるだけで未実装なので、表示抑制の保証には使わない。

## APIキーありHook実測

### 条件

- `codex-cli 0.155.1` の存在だけを確認した。
- 実Codexの`hooks.json`は変更していない。
- `PreCompact` 1件（`auto`）、`PostCompact` 1件（`manual`）、`SessionStart` 1件（`compact`）をshadow入力した。
- `UserPromptSubmit`をAPIキーありで5回実行した。
- 8イベントすべてのstdoutをJSONとして検証し、prompt本文は記録・表示していない。

### イベント結果

| イベント | 件数 | Jev呼び出し | 安全なstdout | 結果 |
| --- | ---: | ---: | ---: | --- |
| `PreCompact` | 1 | 0 | 1/1 | `trigger=auto`を記録、continue |
| `PostCompact` | 1 | 0 | 1/1 | `trigger=manual`を記録、continue |
| `SessionStart` | 1 | 0 | 1/1 | `source=compact`を記録、continue |
| `UserPromptSubmit` | 5 | 5 | 5/5 | 5件すべてSkill選択、エラー0 |
| **合計** | **8** | **5** | **8/8** | **Codex処理を停止しない** |

### UserPromptSubmitの遅延とusage

| 指標 | p50 | p95 | 補足 |
| --- | ---: | ---: | --- |
| `discoveryMs` | 1ms | 1ms | ローカルSkill候補探索 |
| `jevResponseMs` | 493ms | 577ms | Jev応答のアプリ側経過時間 |
| `totalMs` | 494ms | 578ms | 探索 + Jevを含む処理 |
| `elapsedMs` | 494ms | 578ms | Hook処理全体 |
| input tokens平均 | 2,383 | 2,383 | 5件すべて同値 |
| output tokens平均 | 127 | 127 | 5件すべて同値 |

5件すべてが`decision=selected`、`selectedSkill=pdf`、`errorCode`なしだった。lifecycle 3件はJevを呼ばず、metadataとcontinue応答だけを確認した。

実際の記録には次のような安全フィールドだけが入る。

```json
{
  "schemaVersion": 1,
  "mode": "shadow",
  "hookEventName": "UserPromptSubmit",
  "promptSha256": "<sha256>",
  "promptChars": 20,
  "decision": "selected",
  "selectedSkill": "pdf",
  "discoveryMs": 1,
  "jevResponseMs": 493,
  "totalMs": 494,
  "inputTokens": 2383,
  "outputTokens": 127,
  "elapsedMs": 494
}
```

`promptSha256`は再現性の補助に使える一方向ハッシュであり、照合用の生promptは保存しない。`cwd`、会話本文、Skill本文、APIキーもrecordへコピーしない。

## Compaction相当のshadow評価

`hooks compact-eval`は、Codex内部の要約モデルを呼び出す機能ではない。秘密情報を含む固定transcriptへ既存のredaction処理を適用し、compaction後に保持したい事実と除外したいfixture秘密マーカーを反復検査する、契約・安全性の評価fixtureだよ。

```bash
cargo run --locked --manifest-path jevx/Cargo.toml -- \
  hooks compact-eval \
  --runs 5 \
  --json \
  --output /tmp/jevx-compaction.json
```

今回の5回実測結果は次のとおり。

| 指標 | 結果 |
| --- | ---: |
| run count | 5 |
| passed | 5 / 5 |
| 必須事実の保持率 | 100% |
| fixture秘密マーカー漏えい | 0 |
| error rate | 0% |
| duration p50 / p95 | 0 / 0ms |
| output chars p50 / p95 | 112 / 112 |

durationが0msなのは、固定文字列のredactionがmacOSのミリ秒時計分解能より短いからで、CodexやLLMのcompactionレイテンシーを表していない。したがってこの結果から「Codexの要約品質」や「実際のcompact速度」を結論づけてはいけない。

## 安全性・失敗時の挙動

| 状況 | record | stdout | Codexフロー |
| --- | --- | --- | --- |
| lifecycle event | event名とtrigger/source | continue + suppress | 継続 |
| APIキー未設定 | `errorCode=missing_api_key` | continue + suppress | 継続 |
| prompt不在・空 | `errorCode=invalid_input` | continue + suppress | 継続 |
| Providerエラー | `errorCode=provider_error`等 | continue + suppress | 継続 |
| 不明event・event不一致 | コマンドエラー | Hook設定側で検知 | 設定ミスとして停止候補 |

最後の不明eventだけは入力契約違反なので、shadow関数はエラーを返す。既知のイベントでJevが失敗した場合は、Codexの会話を止めないことを優先する設計だよ。

## 現時点の評価と残課題

### 確認できたこと

- Pre/Post compactとSessionStartを、Jevを呼ばずに安全に観測できる。
- UserPromptSubmitのJev判定をshadow実行しても、stdoutは固定のcontinue応答にできる。
- `hooks install`が既存設定を保持し、jevx handlerだけを置換しながら冪等にmergeできる。
- `compact-assist`がcheckpointへ生のmanifestや秘密値を保存せず、compact後だけredacted contextを返せる。
- APIキーありの5回実測で、Hook処理全体p95は578msだった。
- prompt本文を保存せず、選択結果・遅延・usage・ハッシュだけを保存できる。
- compaction相当fixtureで、必須事実保持とfixture秘密マーカー除去を5/5で確認した。
- 実Codex CLIでもstartup/prompt Hookと、成功compact経路の`PreCompact` / `PostCompact` / `SessionStart(source=compact)`を確認した。詳細は[実Codex Hook / 実会話型compaction評価](jevx-real-codex-compaction-evaluation-2026-09-21.md)を参照する。

### まだ証明していないこと

- `hooks install`を実行した使い捨てCodex profileで、trust後の発火順・終了コード・複数回の冪等性をまとめて採取すること。手動設定による実Codex発火は確認済みだが、installer経路は別の導入確認として扱う。
- Codex内部の要約結果が、長い会話の目的・制約・次アクションを保持すること。
- 連続利用時のHook累積遅延、Gateway rate limit、API費用、失敗時の再試行戦略。
- 実ユーザー入力の匿名化fixtureで同じ精度・レイテンシーになること。

次は、使い捨てCodex profileで`hooks install`後のtrust・発火順・終了コードを複数回採取し、長文・実利用に近い匿名化fixtureでcompact後の保持判定を増やす。

## 関連資料

- [複数回・APIキーあり実測](jevx-variance-evaluation-2026-09-21.md)
- [評価Runnerの仕様](jevx-evaluation.md)
- [前回の単回APIキーあり実測](jevx-live-evaluation-2026-09-21.md)
- [Jevあり/なしの導入効果比較](jevx-comfort-evaluation.md)
- [実Codex Hook / 実会話型compaction評価](jevx-real-codex-compaction-evaluation-2026-09-21.md)
