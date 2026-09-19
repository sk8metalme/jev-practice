import assert from 'node:assert/strict';
import test from 'node:test';

import {
  EVALUATION_ENDPOINT,
  GatewayError,
  createGatewayEvaluator,
} from '../src/gateway.js';

const successPayload = {
  model: 'typesafe-ai/jev',
  answers: {
    category: { type: 'choice', choice: 'technical' },
    urgency: { type: 'score', score: 1.2 },
    refundRequested: { type: 'boolean', probability: 0.08 },
  },
  usage: { inputTokens: 12, outputTokens: 8 },
};

test('gateway evaluator sends the documented request to Jev', async () => {
  const calls = [];
  const evaluate = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async (...args) => {
      calls.push(args);
      return {
        ok: true,
        async json() {
          return successPayload;
        },
      };
    },
  });

  const result = await evaluate('アプリが起動しません。');
  const [url, options] = calls[0];

  assert.equal(url, EVALUATION_ENDPOINT);
  assert.equal(options.method, 'POST');
  assert.equal(options.headers.Authorization, 'Bearer test-key');
  assert.equal(options.headers['Content-Type'], 'application/json');
  const body = JSON.parse(options.body);
  assert.equal(body.model, 'typesafe-ai/jev');
  assert.equal(body.state, 'アプリが起動しません。');
  assert.deepEqual(Object.keys(body.questions), ['category', 'urgency', 'refundRequested']);
  assert.equal(body.questions.category.type, 'choice');
  assert.equal(body.questions.urgency.type, 'score');
  assert.equal(body.questions.refundRequested.type, 'boolean');
  assert.deepEqual(result, successPayload);
});

test('gateway evaluator rejects when API key is missing', async () => {
  let called = false;
  const evaluate = createGatewayEvaluator({
    apiKey: ' ',
    fetchImpl: async () => {
      called = true;
    },
  });

  await assert.rejects(evaluate('本文'), error => {
    assert.equal(error.name, 'GatewayError');
    assert.equal(error.statusCode, 500);
    assert.match(error.message, /AI_GATEWAY_API_KEY/);
    return true;
  });
  assert.equal(called, false);
});

test('gateway evaluator converts network errors into GatewayError', async () => {
  const evaluate = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async () => {
      throw new Error('network details must not leak');
    },
  });

  await assert.rejects(evaluate('本文'), error => {
    assert(error instanceof GatewayError);
    assert.equal(error.statusCode, 502);
    assert.equal(error.message, 'Jevへの接続に失敗しました。');
    return true;
  });
});

test('gateway evaluator reports an API error without exposing credentials', async () => {
  const evaluate = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async () => ({
      ok: false,
      status: 429,
      async json() {
        return { error: { message: 'rate limited' } };
      },
    }),
  });

  await assert.rejects(evaluate('本文'), error => {
    assert.equal(error.statusCode, 502);
    assert.equal(error.message, 'rate limited');
    assert.doesNotMatch(error.message, /test-key/);
    return true;
  });
});

test('gateway evaluator uses safe fallbacks for incomplete upstream responses', async () => {
  const fallbackError = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async () => ({
      ok: false,
      async json() {
        return {};
      },
    }),
  });
  await assert.rejects(fallbackError('本文'), error => {
    assert.equal(error.message, 'Jevの評価に失敗しました。');
    return true;
  });

  const fallbackResponse = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async () => ({
      ok: true,
      async json() {
        return { answers: {} };
      },
    }),
  });
  assert.deepEqual(await fallbackResponse('本文'), {
    model: 'typesafe-ai/jev',
    answers: {},
    usage: null,
  });
});

test('gateway evaluator handles invalid responses and malformed JSON', async () => {
  const missingAnswers = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async () => ({
      ok: true,
      async json() {
        return { usage: {} };
      },
    }),
  });
  await assert.rejects(missingAnswers('本文'), /Jevの応答を読み取れません/);

  const malformedJson = createGatewayEvaluator({
    apiKey: 'test-key',
    fetchImpl: async () => ({
      ok: true,
      async json() {
        throw new Error('invalid json');
      },
    }),
  });
  await assert.rejects(malformedJson('本文'), /Jevの応答を読み取れません/);
});

test('gateway evaluator reports missing fetch implementation', () => {
  assert.throws(
    () => createGatewayEvaluator({ apiKey: 'test-key', fetchImpl: null }),
    /fetchが利用できません/,
  );
});

test('gateway evaluator rejects a non-string API key', async () => {
  const evaluate = createGatewayEvaluator({ apiKey: null, fetchImpl: async () => ({}) });
  await assert.rejects(evaluate('本文'), /AI_GATEWAY_API_KEY/);
});
