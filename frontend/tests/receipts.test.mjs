// "Download receipts" in This week (Ada's receipts-export-ui brief, for Lin's #611 route): the UTC
// window, the file name, the truncated header, the three error texts, and the button's busy, done,
// truncated and error states in a live region.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const r = await loadBundled(`export * from './src/receipts';`);
const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { ReceiptsDownload, RECEIPTS_DONE_MS } from './src/components/ReceiptsDownload';
  export { MeasuresPanel } from './src/components/MissionBoardView';
`);
const { React } = m;

// ---------- the window ----------

test('to is today in UTC and from is to minus the range, whatever the local clock says', () => {
  const at = (iso) => new Date(iso);
  assert.deepEqual(r.receiptsWindow(30, at('2026-11-14T03:00:00Z')), { from: '2026-10-15', to: '2026-11-14' });
  assert.deepEqual(r.receiptsWindow(90, at('2026-11-14T03:00:00Z')), { from: '2026-08-16', to: '2026-11-14' });
  assert.deepEqual(r.receiptsWindow(365, at('2026-11-14T03:00:00Z')), { from: '2025-11-14', to: '2026-11-14' });
  // The last and first instants of a UTC day
  assert.deepEqual(r.receiptsWindow(30, at('2026-11-14T23:59:59.999Z')), { from: '2026-10-15', to: '2026-11-14' });
  assert.deepEqual(r.receiptsWindow(30, at('2026-11-15T00:00:00Z')), { from: '2026-10-16', to: '2026-11-15' });
  // 23:30 in New York on Nov 13 is already Nov 14 in UTC
  assert.deepEqual(r.receiptsWindow(30, at('2026-11-13T23:30:00-05:00')), { from: '2026-10-15', to: '2026-11-14' });
  // Across a leap day and a year end
  assert.deepEqual(r.receiptsWindow(30, at('2028-03-01T12:00:00Z')), { from: '2028-01-31', to: '2028-03-01' });
  assert.deepEqual(r.receiptsWindow(365, at('2029-03-01T12:00:00Z')), { from: '2028-03-01', to: '2029-03-01' });
  assert.deepEqual(r.receiptsWindow(30, at('2027-01-05T00:00:00Z')), { from: '2026-12-06', to: '2027-01-05' });
});

test('the ranges are 30 days (default), 90 days and 12 months, and the URL asks for closed tasks', () => {
  assert.deepEqual(r.RECEIPT_RANGES.map((x) => [x.days, x.label]), [[30, 'Last 30 days'], [90, 'Last 90 days'], [365, 'Last 12 months']]);
  assert.equal(r.receiptsUrl(90, new Date('2026-11-14T03:00:00Z')), '/api/tasks/receipts.csv?from=2026-08-16&to=2026-11-14&state=closed');
});

// ---------- the download ----------

const calls = [];
let clicked = [];
let reply = null;
g.fetch = g.window.fetch = async (url) => {
  calls.push(String(url));
  if (reply instanceof Error) throw reply;
  return reply;
};
g.URL.createObjectURL = () => 'blob:receipts';
g.URL.revokeObjectURL = () => {};
window.HTMLAnchorElement.prototype.click = function () {
  clicked.push(this.download);
};
const csv = (headers = {}) => new Response('task_id,title\r\n', { status: 200, headers: { 'Content-Type': 'text/csv; charset=utf-8', ...headers } });
const fail = (status, body) => new Response(body === undefined ? 'oops' : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

afterEach(() => {
  calls.length = 0;
  clicked = [];
});

test('the file is saved under the server name, or kwant-receipts.csv when there is none', async () => {
  reply = csv({ 'Content-Disposition': 'attachment; filename="kwant-receipts-spc_default-2026-11-14.csv"' });
  assert.deepEqual(await r.downloadReceipts(30, new Date('2026-11-14T03:00:00Z')), { name: 'kwant-receipts-spc_default-2026-11-14.csv', truncated: false });
  assert.deepEqual(calls, ['/api/tasks/receipts.csv?from=2026-10-15&to=2026-11-14&state=closed']);
  assert.deepEqual(clicked, ['kwant-receipts-spc_default-2026-11-14.csv']);
  assert.equal(document.querySelector('a'), null, 'the temporary link is removed');

  reply = csv();
  assert.deepEqual(await r.downloadReceipts(30), { name: 'kwant-receipts.csv', truncated: false });
  reply = csv({ 'Content-Disposition': 'attachment' });
  assert.equal((await r.downloadReceipts(30)).name, 'kwant-receipts.csv');
});

test('X-Kwant-Truncated: true is reported, anything else is not', async () => {
  reply = csv({ 'X-Kwant-Truncated': 'true' });
  assert.equal((await r.downloadReceipts(365)).truncated, true);
  reply = csv({ 'X-Kwant-Truncated': 'false' });
  assert.equal((await r.downloadReceipts(365)).truncated, false);
  reply = csv();
  assert.equal((await r.downloadReceipts(365)).truncated, false);
});

test('errors read as plain words: network, the server’s own 400 text, and a retry line for the rest', async () => {
  const err = async () => {
    try {
      await r.downloadReceipts(30);
    } catch (e) {
      return [e.name, e.status, e.message];
    }
    assert.fail('no error');
  };
  reply = new TypeError('Failed to fetch');
  assert.deepEqual(await err(), ['ReceiptsError', 0, 'Couldn’t reach the server. Check your connection and try again.']);
  reply = fail(400, { error: 'from is after to' });
  assert.deepEqual(await err(), ['ReceiptsError', 400, 'from is after to']);
  reply = fail(400);
  assert.deepEqual(await err(), ['ReceiptsError', 400, 'The download didn’t work. Try again in a moment.']);
  for (const st of [401, 404, 500, 503]) {
    reply = fail(st, { error: 'internal detail' });
    assert.deepEqual(await err(), ['ReceiptsError', st, 'The download didn’t work. Try again in a moment.'], `HTTP ${st}`);
  }
  assert.deepEqual(clicked, [], 'nothing is saved on an error');
  assert.equal(r.receiptsErrorText(400, '  unknown state: use verified, failed, cancelled or closed '), 'unknown state: use verified, failed, cancelled or closed');
});

// ---------- the button ----------

const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((res) => setTimeout(res, 0));
};
const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const mount = async (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(m.ReceiptsDownload, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
};

test('busy then done: a spinner and aria-busy, a second click does nothing, then "Saved …" in the live region, gone after ~4 s', async (t) => {
  const asks = [];
  let d = deferred();
  const host = await mount({ download: (days) => (asks.push(days), d.promise) });
  const button = host.querySelector('[data-receipts-download]');
  const select = host.querySelector('[data-receipts-range]');
  const live = host.querySelector('[aria-live="polite"]');
  assert.equal(button.tagName, 'BUTTON');
  assert.equal(button.textContent.trim(), 'Download receipts');
  assert.equal(button.getAttribute('aria-busy'), 'false');
  assert.equal(select.value, '30');
  assert.deepEqual([...select.options].map((o) => o.textContent), ['Last 30 days', 'Last 90 days', 'Last 12 months']);
  assert.equal(document.querySelector(`label[for="${select.id}"]`).textContent.trim(), 'Receipts from');
  assert.match(document.getElementById(button.getAttribute('aria-describedby')).textContent, /CSV of finished tasks: who checked, how, the verdict and the evidence\./);
  assert.equal(live.textContent, '');

  select.value = '90';
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  button.focus();
  button.click();
  await settle();
  assert.equal(button.getAttribute('aria-busy'), 'true');
  assert.equal(button.disabled, false, 'stays focusable');
  assert.ok(document.activeElement === button, 'focus stays on the button');
  assert.ok(button.querySelector('svg.animate-spin.motion-reduce\\:animate-none'), 'spinner in place of the icon, still under reduced motion');
  button.click();
  await settle();
  assert.deepEqual(asks, [90], 'the second click while busy does nothing');

  d.resolve({ name: 'kwant-receipts-spc_default-2026-11-14.csv', truncated: false });
  await settle();
  assert.equal(button.getAttribute('aria-busy'), 'false');
  assert.equal(live.textContent, 'Saved kwant-receipts-spc_default-2026-11-14.csv');
  assert.equal(host.firstElementChild.getAttribute('data-receipts'), 'done');

  // "Saved …" clears after RECEIPTS_DONE_MS. Only setTimeout is mocked; React's scheduler runs on
  // setImmediate in node, so `flush` lets it render.
  const flush = async () => {
    for (let i = 0; i < 10; i++) await new Promise((res) => setImmediate(res));
  };
  t.mock.timers.enable({ apis: ['setTimeout'] });
  d = deferred();
  button.click();
  d.resolve({ name: 'b.csv', truncated: false });
  await flush();
  assert.equal(live.textContent, 'Saved b.csv');
  t.mock.timers.tick(m.RECEIPTS_DONE_MS - 1);
  await flush();
  assert.equal(live.textContent, 'Saved b.csv');
  t.mock.timers.tick(1);
  await flush();
  t.mock.timers.reset();
  assert.equal(live.textContent, '');
  assert.equal(host.firstElementChild.getAttribute('data-receipts'), 'idle');
});

test('truncated and error lines stay until the next try', async () => {
  let next;
  const host = await mount({ download: () => next() });
  const button = host.querySelector('[data-receipts-download]');
  const live = host.querySelector('[aria-live="polite"]');

  next = async () => ({ name: 'kwant-receipts-spc_default-2026-11-14.csv', truncated: true });
  button.click();
  await settle();
  assert.equal(live.querySelector('[data-receipts-saved]').textContent, 'Saved kwant-receipts-spc_default-2026-11-14.csv');
  assert.equal(live.querySelector('[data-receipts-truncated]').textContent, 'Only the oldest 10,000 tasks fit. Pick a shorter range for the rest.');
  assert.match(live.querySelector('[data-receipts-truncated]').className, /text-amber-300/);

  next = async () => {
    throw new Error('from is after to');
  };
  button.click();
  await settle();
  assert.equal(live.textContent, 'from is after to');
  assert.equal(live.querySelector('[data-receipts-truncated]'), null);
  assert.equal(button.getAttribute('aria-busy'), 'false');

  next = async () => ({ name: 'ok.csv', truncated: false });
  button.click();
  await settle();
  assert.equal(live.textContent, 'Saved ok.csv', 'the error clears on the next try');
});

test('the footer sits in MeasuresPanel and shows only when the panel does', () => {
  const NOW = Date.parse('2026-11-01T00:00:00Z');
  const weeks = [0, 0, 1, 3, 0, 0, 4, 2].map((verified, i) => ({
    start: new Date(NOW - (8 - i) * 7 * 86_400_000).toISOString(),
    end: new Date(NOW - (7 - i) * 7 * 86_400_000).toISOString(),
    verified,
    active: verified > 0,
  }));
  const base = { verified_7d: 2, false_success_7d: 0, needs_you_now: 0, blocked_now: 0, interventions_7d: 0, tokens_7d: 0, tokens_per_verified_7d: null };
  const panel = (insights) => m.renderToStaticMarkup(React.createElement(m.MeasuresPanel, { insights, timeZone: 'UTC' }));
  const html = panel({ ...base, verified_weekly: weeks, active_weeks: 4, verified_per_active_week: 2.5, cost_7d_usd: 0.65, cost_per_verified_7d_usd: 0.325, verified_observed_7d: 2, verified_priced_7d: 2, settled_7d: 4, unpriced_7d: 0 });
  assert.match(html, /data-receipts-download/);
  assert.ok(html.indexOf('Cost per verified result') < html.indexOf('Download receipts'), 'below the measures');
  assert.equal(panel(base), '', 'no measures, no button');
  assert.equal(panel(null), '');
});
