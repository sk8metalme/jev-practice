# jev-triage（legacy）

過去にJevの検証に使ったNode.js製のWebアプリです。問い合わせ文を `choice`・`score`・`boolean` の3問で判定し、結果を画面に表示します。

**現行のサポート対象ではありません。** 現行のプロダクトは [`jevx/`](../../jevx/) のRust CLIです。このディレクトリは、[調査記録](../../docs/research/jev-research.md)から参照される歴史的なreferenceとして残しています。機能追加や不具合修正は行いません。

## 動かす場合

Node.js 20以上で、このディレクトリから実行します。

```bash
cd legacy/jev-triage
AI_GATEWAY_API_KEY=... npm start
npm test
npm run lint
```

`docs/screenshots/` には当時の画面を保存しています。
