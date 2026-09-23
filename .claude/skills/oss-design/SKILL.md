---
name: oss-design
description: >-
  Use when creating a new OSS project or reviewing an existing repository's
  design philosophy against 14 cross-cutting principles distilled from 100
  popular OSS projects. Triggers: "OSSを作る", "OSSの哲学", "PHILOSOPHY.md",
  "Non-goals", "互換性の約束", "CONTRIBUTING", "OSSとして見直し", "README作り直し",
  "open source design", "project philosophy". Produces PHILOSOPHY.md /
  README冒頭 / CONTRIBUTING / AGENTS.md の判断基準, or a gap analysis with
  prioritized feature and documentation improvements.
---

# OSS Design

100のOSSが「何を約束し、何を断ってきたか」から抽出した14の傾向を使って、OSSの判断の軸を明文化する。
出典: https://sk8metalme.github.io/thinking/oss/oss-design-philosophy.html

哲学の文書は飾りではなく運営の道具。要望・議論・レビューに「個人の好み」ではなく「方針」で答えるために書く。

## モードを選ぶ

| 依頼 | モード |
| --- | --- |
| 新しいOSSを始める、哲学・README・CONTRIBUTINGを書きたい | **Create** |
| 既存リポジトリを原則と照らし合わせて改善点を出したい | **Review** |
| 両方（見直しの結果から哲学を書き直す） | Review → Create |

参照ファイルは必要なものだけ読む。

- `references/principles.md`: 14傾向の定義・効く理由・落とし穴・取り入れ方と、リポジトリで確認する観点
- `references/checklist.md`: 見直しチェックリスト10項目と判定基準
- `references/catalog.md`: 100プロジェクトの学び・動機・やらないこと（具体例を探すとき）
- `templates/PHILOSOPHY.md`、`templates/CONTRIBUTING.md`、`templates/AGENTS-section.md`: 生成に使う雛形

## Create モード

1. **材料を集める。** 既存のREADMEやコードがあれば先に読む。足りない項目だけユーザーに聞く（一度にまとめて聞く）。
   - 解く問題（既存の選択肢への不満として1文で）
   - 誰のためか／誰のためではないか（対象外には代替手段）
   - 原則3〜5個を「AよりB」形式で（捨てる側も書く）
   - Non-goals（理由と代替手段つき）
   - 互換性の約束の境界（CLI引数・JSONキー・接頭辞・ディレクトリなど）と約束しないもの
   - 貢献モデル（歓迎する／受け入れにくい、Open-Contributionにするか）
   - 判断が割れたときのタイブレーク規則
   - 作者の関わり方（時間・頻度）、ライセンス、関われなくなったときの方針
2. **生成する。**
   - `templates/PHILOSOPHY.md` を埋めて `PHILOSOPHY.md` を作る。
   - READMEの冒頭に「なぜ作ったか（作者の言葉）」「対象／対象外」「5分で最初の成功」「やらないこと」「PHILOSOPHY.mdへのリンク」を置く。
   - `templates/CONTRIBUTING.md` から、選んだ貢献モデルの節だけを残す。
   - LICENSEファイルがなければ、宣言済みのライセンスと揃えて追加する（ライセンスの一方的な変更は提案しない）。
   - `templates/AGENTS-section.md` をAGENTS.md（またはCLAUDE.md）に追記し、エージェントが変更前に使う判断基準にする。
3. **検証する。** `references/checklist.md` の10項目がすべて ✅ になるまで直す。

## Review モード

1. **事実を集める。** README、docs、PHILOSOPHY/CONTRIBUTING/LICENSE、CLIのhelpやpublic API、設定の既定値、CI、Telemetryやデータの保存先、install/uninstall経路、最大ファイルの行数を読む。推測で埋めない。
2. **14原則で照合する。** `references/principles.md` の「リポジトリで確認する観点」を使い、次の表を作る。

   | 原則 | 現状（ファイル:行などの根拠） | ギャップ | 改善案（機能） | 改善案（docs） |

   該当しない原則は「該当なし（理由）」と書く。全部を満たそうとしない。
3. **チェックリストで判定する。** 10項目を ✅ / ❌ / 部分的 で示し、❌ には具体的な修正を添える。
4. **優先度をつけて提案する。** P0（信頼・安全・データ）→ P1（最初の5分・互換性の約束・Non-goals）→ P2（局所性・CI・性能）の順に並べる。それぞれに「実在する問題」「受け入れ基準」「捨てるもの」を書く。
5. 具体例が要るときは `references/catalog.md` から近い分野のプロジェクトを2〜3件引く。「知識ベース」の項目は、一次情報を確認してから引用する。

## 守ること

- 抽象的な原則だけで終わらせない。具体例とトレードオフを必ず添える。
- 最初から全部を書かせない。まず「やらないこと」を3つ書くところから始めてもよい。
- 互換性の約束は小さく明確にする。守れない広い約束は書かない。
- オプションを増やす提案の前に、既定値の改善で解けないかを検討する。上書きの逃げ道はふさがない。
- 断る提案には理由と代替手段をセットにする。
- 書き込みや外部公開（リポジトリ作成、push、ライセンス変更）は、ユーザーの確認を取ってから行う。
