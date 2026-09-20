# jevx evaluation fixtures

`skill-selection.jsonl`は、jevxのSkill選択を比較測定するための秘密情報なしのケース集だよ。

- `synthetic`: 30件の合成ケース
- `anonymized-template`: 10件の匿名化形式テンプレートケース

ケースは正解ラベルを含むので、Jevへ送信する入力からは`expected`と`keywords`を除外する。出力保存時もprompt本文を再保存せず、ID・判定・遅延・usageだけを残してね。

JSONLの検証:

```bash
jq -e -s 'length == 40 and all(.[]; .id and .prompt and .expected)' \
  jevx/evals/skill-selection.jsonl >/dev/null
```
