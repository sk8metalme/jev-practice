import { summarizeAnswers } from './evaluation.js';

const HTML_ESCAPE_MAP = Object.freeze({
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
});

export function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, character => HTML_ESCAPE_MAP[character]);
}

function renderResult(result) {
  const summary = summarizeAnswers(result?.answers);
  const categoryConfidence = summary.categoryConfidence === null
    ? '確信度は取得できませんでした'
    : `確信度 ${summary.categoryConfidence}%`;
  const refundProbability = summary.refundProbability === null
    ? '確率は取得できませんでした'
    : `${summary.refundProbability}%の可能性`;
  const usage = result?.usage && Number.isFinite(result.usage.inputTokens)
    && Number.isFinite(result.usage.outputTokens)
    ? `${result.usage.inputTokens} input / ${result.usage.outputTokens} output`
    : '使用量は取得できませんでした';

  return `
    <section class="results" aria-labelledby="result-title">
      <div class="section-heading">
        <div>
          <p class="eyebrow">EVALUATION RESULT</p>
          <h2 id="result-title">判定できたよ</h2>
        </div>
        <span class="status-dot">Jev online</span>
      </div>
      <div class="result-grid">
        <article class="result-card result-card--accent">
          <span class="result-label">カテゴリ</span>
          <strong>${escapeHtml(summary.category)}</strong>
          <small>${escapeHtml(categoryConfidence)}</small>
        </article>
        <article class="result-card">
          <span class="result-label">緊急度</span>
          <strong>${escapeHtml(summary.urgency)}</strong>
          <small>${summary.urgencyScore === null ? 'スコアは取得できませんでした' : `スコア ${summary.urgencyScore.toFixed(1)}`}</small>
        </article>
        <article class="result-card">
          <span class="result-label">返金要求</span>
          <strong>${escapeHtml(refundProbability)}</strong>
          <small>boolean probability</small>
        </article>
      </div>
      <p class="usage">${escapeHtml(usage)}</p>
    </section>`;
}

export function renderPage({ state = '', result = null, error = null } = {}) {
  const feedback = error
    ? `<p class="feedback feedback--error" role="alert">${escapeHtml(error)}</p>`
    : result
      ? renderResult(result)
      : '<p class="empty-state">問い合わせ文を入れて、Jevに聞いてみよ。</p>';

  return `<!doctype html>
<html lang="ja">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="description" content="Jevで問い合わせをすばやく仕分けるミニアプリ">
    <title>Jev Triage</title>
    <link rel="stylesheet" href="/styles.css">
  </head>
  <body>
    <main class="shell">
      <header class="hero">
        <div class="brand-mark" aria-hidden="true">J</div>
        <p class="eyebrow">TYPED AI EVALUATION</p>
        <h1>問い合わせを、<span>いい感じに仕分け。</span></h1>
        <p class="hero-copy">Jevに文章を読んでもらって、カテゴリ・緊急度・返金要求を一度にチェックするよ。</p>
      </header>
      <section class="panel" aria-labelledby="input-title">
        <div class="section-heading">
          <div>
            <p class="eyebrow">YOUR INQUIRY</p>
            <h2 id="input-title">問い合わせ本文</h2>
          </div>
          <span class="limit">最大 4,000文字</span>
        </div>
        <form method="post" action="/evaluate">
          <label class="sr-only" for="state">問い合わせ本文</label>
          <textarea id="state" name="state" maxlength="4000" placeholder="例：昨日注文した商品が届かず、返金もお願いしたいです。">${escapeHtml(state)}</textarea>
          <div class="form-footer">
            <p>入力内容はJevの判定にだけ使われます。</p>
            <button type="submit">Jevで判定する <span aria-hidden="true">→</span></button>
          </div>
        </form>
      </section>
      ${feedback}
      <footer>Powered by <strong>Jev</strong> via Vercel AI Gateway</footer>
    </main>
  </body>
</html>`;
}
