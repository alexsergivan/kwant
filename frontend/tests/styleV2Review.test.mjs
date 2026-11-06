// Style v2 §6 review card (IR-3, 0.7.1, Iris).
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export * from './src/dev/types';
  export * from './src/dev/logic';
  export * as fx from './src/dev/fixtures';
  export { LandedCard, LandedList } from './src/components/dev/LandedCard';
  export { Rollup, rollupTitle, rollupTiles } from './src/components/dev/Rollup';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { ReleaseNotesSheet } from './src/components/dev/ReleaseNotesSheet';
  export { ProdFloorCard } from './src/components/dev/ProdFloorCard';
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));
const NOW = fx.FIXTURE_NOW;

const landed = (raw, over = {}) => ({ ...m.parseLanded(raw), ...over });
const review = (raw, over = {}) => ({ ...m.parseReview(raw), ...over });

// ---------------------------------------------------------------------------------------------------
// DOM helpers

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(C, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const button = (host, text) => [...host.querySelectorAll('button')].find((b) => b.textContent.replace(/\s+/g, ' ').trim().startsWith(text));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};


// Style v2 §6 (IR-3): branch flow, reading line, LED security tiles, tamper chip, LGTM stamp.
// Every assertion compares strings or booleans: never hand a DOM element to a failing assert (it util.inspects
// the whole jsdom window; see tests/postGithub.test.mjs, 2026-11-05).
const base = () => review(fx.REVIEW_294);
const flat = (el) => (el ? el.textContent.replace(/\s+/g, ' ').trim() : '');
const clean = (over = {}) =>
  review(fx.REVIEW_294, {
    id: 'dvr_clean_' + Math.random().toString(36).slice(2),
    risk: { level: 'low', reasons: ['+12 −3 in 2 files'] },
    tamper: { flags: [], unread: [] },
    intent: { ...base().intent, rows: base().intent.rows.filter((r) => r.state !== 'missing') },
    findings: base().findings.map((f) => ({ ...f, state: 'sent' })),
    ...over,
  });
const props = (card) => ({ card, onFinding: async () => card, forge: 'github', now: NOW });

test('lgtm holds only when every §6 condition holds', () => {
  assert.equal(m.lgtm(clean()), true, 'clean ready card');
  assert.equal(m.lgtm(base()), false, '#294 has open findings, a tamper flag and a missing row');
  assert.equal(m.lgtm(clean({ status: 'pending' })), false, 'not ready');
  assert.equal(m.lgtm(clean({ reviewer: null })), false, 'no reviewer');
  assert.equal(m.lgtm(clean({ same_engine: true })), false, 'same engine');
  assert.equal(m.lgtm(clean({ findings: [{ ...base().findings[0], state: 'open' }] })), false, 'an open finding');
  assert.equal(m.lgtm(clean({ tamper: base().tamper })), false, 'a tamper flag');
  assert.equal(m.lgtm(clean({ intent: base().intent })), false, 'an intent row missing');
  assert.equal(m.lgtm(clean({ security: { ...base().security, secrets: { state: 'hit', count: 1 } } })), false, 'secrets hit');
  assert.equal(m.lgtm(clean({ security: { ...base().security, packages: { state: 'hit', count: 2 } } })), false, 'packages hit');
  assert.equal(m.lgtm(clean({ intent: null })), true, 'no intent block is fine');
  assert.equal(m.lgtmLabel('Rigel'), 'Looks good to me: Rigel found nothing to fix');
  assert.equal(m.readingLine('Cody'), 'Cody is reading the diff');
  assert.equal(m.readingLine(null), 'A reviewer is reading the diff');
});

test('LGTM stamp: role img, label, ink content, no stamp on warnings', () => {
  const out = html(m.ReviewCard, props(clean()));
  assert.match(out, /class="dv-stamp" role="img" aria-label="Looks good to me: Rigel found nothing to fix"/);
  assert.match(out, /<b aria-hidden="true">LGTM<\/b><small aria-hidden="true">Rigel · 0 to fix<\/small>/);
  assert.equal(html(m.ReviewCard, props(base())).includes('dv-stamp'), false, 'never on a card with warnings');
  assert.equal(html(m.ReviewCard, props(clean({ same_engine: true }))).includes('dv-stamp'), false, 'same engine: no stamp');
  assert.equal(html(m.ReviewCard, props(clean({ status: 'pending' }))).includes('dv-stamp'), false, 'pending: no stamp');
});

test('LGTM stamp-in plays once per card, never on a re-render or a remount', async () => {
  const card = clean();
  const a = await mount(m.ReviewCard, props(card));
  assert.equal(a.querySelector('.dv-stamp')?.hasAttribute('data-stamp-in'), true, 'first ready render stamps in');
  const b = await mount(m.ReviewCard, props({ ...card }));
  const s = b.querySelector('.dv-stamp');
  assert.equal(!!s, true);
  assert.equal(s.hasAttribute('data-stamp-in'), false, 'remount: already stamped');
  assert.equal(s.style.animation, 'none');
});

test('branch flow: icon, head, arrow, base; screen readers hear "into"', () => {
  const host = document.createElement('div');
  host.innerHTML = html(m.ReviewCard, props(base()));
  const b = host.querySelector('[data-branch]');
  assert.equal(b?.className, 'dv-branch');
  assert.equal(!!b.querySelector('svg.i[aria-hidden="true"]'), true, 'branch icon');
  assert.equal(b.querySelector('.h')?.textContent, 'pixel-live-redact-n3');
  assert.equal(b.querySelector('.arr')?.getAttribute('aria-hidden'), 'true');
  assert.equal(flat(b), 'pixel-live-redact-n3→ into main', 'the arrow is hidden; screen readers hear "… into main"');
});

test('pending: reading line with a run LED and caret, two static skeleton bars, no ETA', () => {
  const host = document.createElement('div');
  host.innerHTML = html(m.ReviewCard, props(review(fx.REVIEW_294, { status: 'pending', findings: [], tamper: { flags: [], unread: [] } })));
  const r = host.querySelector('[data-reading] [role="status"]');
  assert.equal(flat(r), 'Rigel is reading the diff');
  assert.equal(!!r.querySelector('.dv-led[data-s="run"]'), true);
  assert.equal(r.querySelector('.dv-caret')?.getAttribute('aria-hidden'), 'true');
  assert.equal(host.querySelectorAll('[data-reading] > [aria-hidden="true"] > div').length, 2);
  assert.equal(/minute|second|ETA|Reviewing this PR/.test(host.textContent), false);
  const none = document.createElement('div');
  none.innerHTML = html(m.ReviewCard, props(review(fx.REVIEW_294, { status: 'pending', reviewer: null })));
  assert.equal(flat(none.querySelector('[data-reading] [role="status"]')), 'A reviewer is reading the diff');
});

test('security tiles: LED by state, tool in mono top right', () => {
  const host = document.createElement('div');
  host.innerHTML = html(m.ReviewCard, props(review(fx.REVIEW_294, { security: { ...base().security, secrets: { state: 'hit', count: 1 }, packages: { state: 'not_run', count: 0 } } })));
  const led = (tool) => host.querySelector(`[data-sec="${tool}"] .dv-led`)?.getAttribute('data-s');
  assert.equal(led('Gitleaks'), 'bad');
  assert.equal(led('Registry check'), 'off');
  assert.equal(led('OSV'), 'off');
  assert.equal(led('Semgrep'), 'off');
  assert.equal(flat(host.querySelector('[data-sec="Gitleaks"] p')), 'Secrets checked by gitleaks');
  assert.equal(flat(host.querySelector('[data-sec="Gitleaks"] p + p')), '1 secret · see finding');
  assert.equal(flat(host.querySelector('[data-sec="Registry check"] .font-mono')), 'checked by registry');
  const ok = document.createElement('div');
  ok.innerHTML = html(m.ReviewCard, props(base()));
  assert.equal(ok.querySelector('[data-sec="Gitleaks"] .dv-led')?.getAttribute('data-s'), 'ok');
});

test('tamper: file:line as a sha chip, removed line on the del tokens', () => {
  const host = document.createElement('div');
  host.innerHTML = html(m.ReviewCard, props(base()));
  const t = host.querySelector('[data-tamper="loosened"]');
  assert.equal(t.querySelector('[data-tamper-ref]')?.className.split(' ')[0], 'dv-sha');
  assert.equal(flat(t.querySelector('[data-tamper-ref]')), 'src/server/activity.rs:418');
  const del = [...t.querySelectorAll('pre > div')].find((d) => d.textContent.includes('assert_eq!'));
  assert.equal(del?.className.includes('bg-[var(--dv-del-bg)] text-[var(--dv-del)]'), true);
  assert.equal(flat(del).startsWith('Removed: -'), true);
});
