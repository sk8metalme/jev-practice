import { buildEvaluationRequest, MODEL_ID } from './evaluation.js';

export const EVALUATION_ENDPOINT = 'https://ai-gateway.vercel.sh/v1/evaluate';

export class GatewayError extends Error {
  constructor(message, statusCode = 502) {
    super(message);
    this.name = 'GatewayError';
    this.statusCode = statusCode;
  }
}

export function createGatewayEvaluator({
  apiKey = process.env.AI_GATEWAY_API_KEY,
  endpoint = EVALUATION_ENDPOINT,
  fetchImpl = globalThis.fetch,
  now = () => Date.now(),
} = {}) {
  if (typeof fetchImpl !== 'function') {
    throw new GatewayError('fetchが利用できません。', 500);
  }

  return async function evaluate(value) {
    const request = buildEvaluationRequest(value);
    if (typeof apiKey !== 'string' || apiKey.trim().length === 0) {
      throw new GatewayError('AI_GATEWAY_API_KEYを設定してね。', 500);
    }

    const startedAt = now();
    let response;
    try {
      response = await fetchImpl(endpoint, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${apiKey.trim()}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(request),
      });
    } catch {
      throw new GatewayError('Jevへの接続に失敗しました。');
    }

    let payload;
    try {
      payload = await response.json();
    } catch {
      throw new GatewayError('Jevの応答を読み取れませんでした。');
    }

    if (!response.ok) {
      const message = typeof payload?.error?.message === 'string'
        ? payload.error.message
        : 'Jevの評価に失敗しました。';
      throw new GatewayError(message);
    }

    if (!payload || typeof payload.answers !== 'object' || Array.isArray(payload.answers)) {
      throw new GatewayError('Jevの応答を読み取れませんでした。');
    }

    return {
      model: payload.model ?? MODEL_ID,
      answers: payload.answers,
      usage: payload.usage ?? null,
      responseMs: Math.max(0, Math.round(now() - startedAt)),
    };
  };
}
