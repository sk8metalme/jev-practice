export const MAX_STATE_LENGTH = 4000;
export const MODEL_ID = 'typesafe-ai/jev';

export const CATEGORY_LABELS = Object.freeze({
  billing: '請求・返金',
  account: 'アカウント',
  technical: '不具合',
  shipping: '配送',
});

const QUESTIONS = Object.freeze({
  category: Object.freeze({
    type: 'choice',
    instructions: '問い合わせの主なカテゴリを1つ選んでください。',
    criteria: Object.freeze({
      billing: '請求、支払い、返金に関する内容',
      account: 'ログイン、アカウント、認証に関する内容',
      technical: 'アプリや機能の不具合に関する内容',
      shipping: '配送、到着、注文状況に関する内容',
    }),
  }),
  urgency: Object.freeze({
    type: 'score',
    instructions: '問い合わせの対応の緊急度を判定してください。',
    criteria: Object.freeze([
      'low: すぐの対応が不要',
      'medium: 早めの対応が望ましい',
      'high: 至急対応が必要',
    ]),
  }),
  refundRequested: Object.freeze({
    type: 'boolean',
    instructions: '返金を希望または要求していますか？',
    criteria: Object.freeze({
      true: '返金を希望または要求している',
      false: '返金を希望または要求していない',
    }),
  }),
});

export class InputError extends Error {
  constructor(message) {
    super(message);
    this.name = 'InputError';
  }
}

export function validateState(value) {
  if (typeof value !== 'string') {
    throw new InputError('問い合わせ本文を入力してね。');
  }

  const state = value.trim();
  if (state.length === 0) {
    throw new InputError('問い合わせ本文を入力してね。');
  }
  if (state.length > MAX_STATE_LENGTH) {
    throw new InputError('問い合わせ本文は4,000文字以内で入力してね。');
  }

  return state;
}

export function buildEvaluationRequest(value) {
  return {
    model: MODEL_ID,
    state: validateState(value),
    questions: QUESTIONS,
  };
}

function probabilityToPercent(value) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) {
    return null;
  }
  return Math.round(value * 100);
}

function categoryConfidence(answer) {
  if (!answer || !Object.hasOwn(CATEGORY_LABELS, answer.choice)) {
    return null;
  }
  return probabilityToPercent(answer.probabilities?.[answer.choice]);
}

function urgencyDisplay(score) {
  if (typeof score !== 'number' || !Number.isFinite(score) || score < 0 || score > 2) {
    return { label: '判定なし', score: null };
  }

  return {
    label: ['低', '中', '高'][Math.round(score)],
    score,
  };
}

export function summarizeAnswers(answers) {
  const safeAnswers = answers && typeof answers === 'object' ? answers : {};
  const categoryChoice = safeAnswers.category?.choice;
  const category = Object.hasOwn(CATEGORY_LABELS, categoryChoice)
    ? CATEGORY_LABELS[categoryChoice]
    : (typeof categoryChoice === 'string' && categoryChoice.length > 0 ? categoryChoice : '判定なし');
  const urgency = urgencyDisplay(safeAnswers.urgency?.score);

  return {
    category,
    categoryConfidence: categoryConfidence(safeAnswers.category),
    urgency: urgency.label,
    urgencyScore: urgency.score,
    refundProbability: probabilityToPercent(safeAnswers.refundRequested?.probability),
  };
}
