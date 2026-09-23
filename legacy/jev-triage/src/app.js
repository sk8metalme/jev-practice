import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';

import { InputError, validateState } from './evaluation.js';
import { renderPage } from './html.js';

const MAX_BODY_BYTES = 64 * 1024;
const STYLESHEET_URL = new URL('../public/styles.css', import.meta.url);

class RequestBodyError extends Error {
  constructor() {
    super('リクエストが大きすぎます。');
    this.name = 'RequestBodyError';
    this.statusCode = 413;
  }
}

function sendResponse(response, statusCode, contentType, body) {
  response.writeHead(statusCode, {
    'Content-Type': `${contentType}; charset=utf-8`,
    'Content-Length': Buffer.byteLength(body),
  });
  response.end(body);
}

async function readBody(request, maxBytes) {
  const chunks = [];
  let totalBytes = 0;

  for await (const chunk of request) {
    totalBytes += Buffer.byteLength(chunk);
    if (totalBytes > maxBytes) {
      throw new RequestBodyError();
    }
    chunks.push(chunk);
  }

  return Buffer.concat(chunks).toString('utf8');
}

function errorResponse(error) {
  if (error instanceof InputError) {
    return { statusCode: 400, message: error.message };
  }
  if (error instanceof RequestBodyError) {
    return { statusCode: error.statusCode, message: error.message };
  }
  if (typeof error?.statusCode === 'number' && error.statusCode >= 500 && error.statusCode < 600) {
    return { statusCode: error.statusCode, message: error.message };
  }
  return { statusCode: 502, message: 'Jevへの接続に失敗しました。' };
}

async function handleRequest(request, response, { evaluator, maxBodyBytes, stylesheet }) {
  const requestUrl = new URL(request.url ?? '/', 'http://localhost');

  if (request.method === 'GET' && requestUrl.pathname === '/') {
    sendResponse(response, 200, 'text/html', renderPage());
    return;
  }

  if (request.method === 'GET' && requestUrl.pathname === '/styles.css') {
    const css = stylesheet ?? await readFile(STYLESHEET_URL, 'utf8');
    sendResponse(response, 200, 'text/css', css);
    return;
  }

  if (request.method === 'POST' && requestUrl.pathname === '/evaluate') {
    try {
      const body = await readBody(request, maxBodyBytes);
      const state = validateState(new URLSearchParams(body).get('state') ?? '');
      const result = await evaluator(state);
      sendResponse(response, 200, 'text/html', renderPage({ state, result }));
    } catch (error) {
      const { statusCode, message } = errorResponse(error);
      sendResponse(response, statusCode, 'text/html', renderPage({ error: message }));
    }
    return;
  }

  sendResponse(response, 404, 'text/html', renderPage({ error: 'ページが見つかりません。' }));
}

export function createApp({ evaluator, maxBodyBytes = MAX_BODY_BYTES, stylesheet = null } = {}) {
  if (typeof evaluator !== 'function') {
    throw new TypeError('evaluatorが必要です。');
  }

  return createServer((request, response) => {
    handleRequest(request, response, { evaluator, maxBodyBytes, stylesheet }).catch(() => {
      if (!response.headersSent) {
        sendResponse(response, 500, 'text/html', renderPage({ error: 'サーバーでエラーが発生しました。' }));
      }
    });
  });
}
