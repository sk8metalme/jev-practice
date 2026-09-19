import assert from 'node:assert/strict';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

import { createApp } from '../src/app.js';
import { escapeHtml, renderPage } from '../src/html.js';
import { listeningPort, resolvePort, runIfMain, startServer } from '../src/main.js';

async function withServer(evaluator, callback) {
  const server = createApp({ evaluator });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  const baseUrl = `http://127.0.0.1:${address.port}`;

  try {
    return await callback(baseUrl);
  } finally {
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
}

test('app serves the form and stylesheet', async () => {
  await withServer(async () => ({}), async baseUrl => {
    const page = await fetch(`${baseUrl}/`);
    const styles = await fetch(`${baseUrl}/styles.css`);

    assert.equal(page.status, 200);
    assert.match(await page.text(), /Jev Triage/);
    assert.equal(styles.status, 200);
    assert.match(await styles.text(), /--ink/);
  });
});

test('app evaluates a submitted inquiry and renders structured results', async () => {
  const received = [];
  await withServer(async state => {
    received.push(state);
    return {
      answers: {
        category: { type: 'choice', choice: 'billing', probabilities: { billing: 0.91 } },
        urgency: { type: 'score', score: 1.8 },
        refundRequested: { type: 'boolean', probability: 0.83 },
      },
      usage: { inputTokens: 42, outputTokens: 9 },
      responseMs: 124,
    };
  }, async baseUrl => {
    const response = await fetch(`${baseUrl}/evaluate`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ state: '<二重請求>されました。' }),
    });
    const html = await response.text();

    assert.equal(response.status, 200);
    assert.deepEqual(received, ['<二重請求>されました。']);
    assert.match(html, /請求・返金/);
    assert.match(html, /高/);
    assert.match(html, /83%/);
    assert.match(html, /&lt;二重請求&gt;されました。/);
    assert.match(html, /42 input \/ 9 output/);
    assert.match(html, /Jev response 124 ms/);
  });
});

test('app returns a validation error for a blank inquiry', async () => {
  let called = false;
  await withServer(async () => {
    called = true;
    return {};
  }, async baseUrl => {
    const response = await fetch(`${baseUrl}/evaluate`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ state: '   ' }),
    });

    assert.equal(response.status, 400);
    assert.match(await response.text(), /問い合わせ本文を入力してね/);
    assert.equal(called, false);
  });
});

test('app turns evaluator failures into a friendly gateway error', async () => {
  await withServer(async () => {
    throw new Error('private gateway detail');
  }, async baseUrl => {
    const response = await fetch(`${baseUrl}/evaluate`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ state: '本文' }),
    });

    assert.equal(response.status, 502);
    const html = await response.text();
    assert.match(html, /Jevへの接続に失敗しました/);
    assert.doesNotMatch(html, /private gateway detail/);
  });
});

test('app preserves a safe upstream status code', async () => {
  await withServer(async () => {
    throw Object.assign(new Error('temporarily unavailable'), { statusCode: 503 });
  }, async baseUrl => {
    const response = await fetch(`${baseUrl}/evaluate`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ state: '本文' }),
    });

    assert.equal(response.status, 503);
    assert.match(await response.text(), /temporarily unavailable/);
  });
});

test('app falls back for unsafe upstream status codes', async () => {
  for (const statusCode of [400, 600]) {
    await withServer(async () => {
      throw Object.assign(new Error('unsafe status'), { statusCode });
    }, async baseUrl => {
      const response = await fetch(`${baseUrl}/evaluate`, {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ state: '本文' }),
      });

      assert.equal(response.status, 502);
      assert.match(await response.text(), /unsafe status|Jevへの接続に失敗しました/);
    });
  }
});

test('app handles unknown routes and oversized request bodies', async () => {
  await withServer(async () => ({}), async baseUrl => {
    const notFound = await fetch(`${baseUrl}/unknown`);
    assert.equal(notFound.status, 404);

    const oversized = await fetch(`${baseUrl}/evaluate`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ state: 'a'.repeat(70000) }),
    });
    assert.equal(oversized.status, 413);
    assert.match(await oversized.text(), /大きすぎます/);
  });
});

test('app returns a server error when an unexpected request failure occurs', async () => {
  const server = createApp({ evaluator: async () => ({}), stylesheet: () => {} });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();

  try {
    const response = await fetch(`http://127.0.0.1:${address.port}/styles.css`);
    assert.equal(response.status, 500);
    assert.match(await response.text(), /サーバーでエラーが発生しました/);
  } finally {
    await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  }
});

test('app does not write a second response after headers are sent', async () => {
  const server = createApp({ evaluator: async () => ({}) });
  const response = {
    headersSent: true,
    writeHead() {
      throw new Error('must not write');
    },
    end() {
      throw new Error('must not end');
    },
  };

  server.emit('request', { method: 'GET', url: '%' }, response);
  server.emit('request', { method: 'GET', url: null }, response);
  await new Promise(resolve => setImmediate(resolve));
});

test('html helpers escape values and render incomplete results', () => {
  assert.equal(escapeHtml('&<>"\''), '&amp;&lt;&gt;&quot;&#39;');
  assert.equal(escapeHtml(undefined), '');

  const html = renderPage({
    state: '確認',
    result: {
      answers: {
        category: { choice: 'other' },
        urgency: { score: 99 },
        refundRequested: { probability: 2 },
      },
    },
  });
  assert.match(html, /<strong>other<\/strong>/);
  assert.match(html, /確信度は取得できませんでした/);
  assert.match(html, /確率は取得できませんでした/);
  assert.match(html, /使用量は取得できませんでした/);
  assert.match(html, /応答速度は取得できませんでした/);
});

test('createApp requires an evaluator', () => {
  assert.throws(() => createApp(), /evaluatorが必要/);
});

test('main starts a server and only auto-starts for its own module path', async () => {
  const server = startServer(0, () => {});
  await new Promise(resolve => server.once('listening', resolve));
  assert.equal(server.listening, true);
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));

  assert.equal(listeningPort({ port: 4321 }, 0), 4321);
  assert.equal(listeningPort(null, 4321), 4321);
  assert.equal(resolvePort('4321'), 4321);
  assert.equal(resolvePort(''), 3000);

  const moduleUrl = pathToFileURL('/tmp/main.js').href;
  let called = 0;
  runIfMain({
    argv: ['node', '/tmp/main.js'],
    moduleUrl,
    start: () => { called += 1; },
  });
  runIfMain({
    argv: ['node', '/tmp/other.js'],
    moduleUrl,
    start: () => { called += 1; },
  });
  runIfMain({
    argv: ['node'],
    moduleUrl,
    start: () => { called += 1; },
  });
  assert.equal(called, 1);
});
