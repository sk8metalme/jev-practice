import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CATEGORY_LABELS,
  MAX_STATE_LENGTH,
  buildEvaluationRequest,
  summarizeAnswers,
  validateState,
} from '../src/evaluation.js';

test('validateState trims a valid inquiry', () => {
  assert.equal(validateState('  二重請求されました。  '), '二重請求されました。');
});

test('validateState rejects non-string, empty, and overlong input', () => {
  assert.throws(() => validateState(null), /問い合わせ本文を入力してね/);
  assert.throws(() => validateState(' \n '), /問い合わせ本文を入力してね/);
  assert.throws(() => validateState('a'.repeat(MAX_STATE_LENGTH + 1)), /4,000文字以内/);
});

test('buildEvaluationRequest creates Jev typed questions', () => {
  const request = buildEvaluationRequest('ログインできず、返金も希望しています。');

  assert.equal(request.model, 'typesafe-ai/jev');
  assert.equal(request.state, 'ログインできず、返金も希望しています。');
  assert.deepEqual(request.questions.category, {
    type: 'choice',
    instructions: '問い合わせの主なカテゴリを1つ選んでください。',
    criteria: {
      billing: '請求、支払い、返金に関する内容',
      account: 'ログイン、アカウント、認証に関する内容',
      technical: 'アプリや機能の不具合に関する内容',
      shipping: '配送、到着、注文状況に関する内容',
    },
  });
  assert.equal(request.questions.urgency.type, 'score');
  assert.equal(request.questions.refundRequested.type, 'boolean');
});

test('summarizeAnswers maps Jev answers into display values', () => {
  const summary = summarizeAnswers({
    category: {
      type: 'choice',
      choice: 'billing',
      probabilities: { billing: 0.91, account: 0.09 },
    },
    urgency: {
      type: 'score',
      score: 1.8,
      probabilities: { '0': 0.01, '1': 0.05, '2': 0.94 },
    },
    refundRequested: { type: 'boolean', probability: 0.83 },
  });

  assert.deepEqual(summary, {
    category: '請求・返金',
    categoryConfidence: 91,
    urgency: '高',
    urgencyScore: 1.8,
    refundProbability: 83,
  });
});

test('summarizeAnswers provides safe fallbacks for partial or unknown answers', () => {
  assert.deepEqual(summarizeAnswers(), {
    category: '判定なし',
    categoryConfidence: null,
    urgency: '判定なし',
    urgencyScore: null,
    refundProbability: null,
  });

  assert.deepEqual(summarizeAnswers({
    category: { choice: 'other', probabilities: { other: 1 } },
    urgency: { score: 99 },
    refundRequested: { probability: 2 },
  }), {
    category: 'other',
    categoryConfidence: null,
    urgency: '判定なし',
    urgencyScore: null,
    refundProbability: null,
  });

  assert.equal(CATEGORY_LABELS.technical, '不具合');
});
