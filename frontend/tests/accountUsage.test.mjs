// account-usage (autonomy/account-usage/design.md): the 5-hour and weekly bars on a Claude/Codex sign-in's row,
// "checked … ago", and Check now (POST /api/profiles/:id/usage, only when pressed).
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { AccountList } from './src/components/EngineAccounts';
`);
const h = m.React.createElement;
const settle = (ms = 0) => new Promise((r) => setTimeout(r, ms));

const calls = [];
let reply = null;
g.fetch = g.window.fetch = async (url, init = {}) => {
  calls.push(`${init.method ?? 'GET'} ${url}`);
  if (url.endsWith('/usage') && reply) return reply();
  return new Response('{}', { status: 404 });
};

const ago = (min) => new Date(Date.now() - min * 60_000).toISOString();
const inMin = (min) => new Date(Date.now() + min * 60_000).toISOString();
const usage = (five, week, checkedMinAgo = 2) => ({
  checked_at: ago(checkedMinAgo), source: 'turn', status: 'ok',
  windows: [{ kind: 'five_hour', used_pct: five, resets_at: inMin(90) }, { kind: 'weekly', used_pct: week, resets_at: inMin(3 * 1440) }],
});
const acct = (over) => ({
  id: 'prf_w', engine: 'claude', name: 'Work', kind: 'cli_home', enabled: true, is_default: false, priority: 100, home_dir: null,
  has_key: false, signed_in: true, account_label: 'me@example.com', limited_until: null, last_error: null, ...over,
});
const engine = { id: 'claude', name: 'Claude Code CLI', kind: 'cli', status: 'ready', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: true, uses: { profile_id: 'prf_w', label: 'Work' } };

async function mount(accounts, eng = engine) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(h(m.AccountList, { engine: eng, accounts, onChanged: async () => {} })));
  await settle();
  return { host, done: () => { root.unmount(); host.remove(); } };
}
const rowOf = (host, id) => [...host.querySelectorAll('li')].find((li) => li.querySelector(`#account-${id}-name`));

test('bars fill with what is left, amber below 20%, nothing for an API key', async () => {
  const { host, done } = await mount([
    acct({ usage: usage(7, 56) }),
    acct({ id: 'prf_low', name: 'Low', usage: usage(88, 10) }),
    acct({ id: 'prf_key', name: 'Key', kind: 'api_key', has_key: true, usage: usage(7, 56) }),
  ]);
  const work = rowOf(host, 'prf_w');
  const meters = work.querySelectorAll('[role="meter"]');
  assert.equal(meters.length, 2);
  assert.equal(meters[0].getAttribute('aria-label'), '5-hour usage left');
  assert.equal(meters[0].getAttribute('aria-valuenow'), '93');
  assert.match(meters[0].getAttribute('aria-valuetext'), /^93% left, resets /);
  assert.equal(meters[0].firstElementChild.style.transform, 'scaleX(0.93)');
  assert.match(work.querySelector('[data-usage]').textContent, /^Usage· checked 2 min ago5-hour93% left · resets .+Weekly44% left · resets \S+ /);
  const low = rowOf(host, 'prf_low').querySelector('[data-usage-row="five_hour"]');
  assert.equal(low.dataset.usageTone, 'low');
  assert.match(low.innerHTML, /bg-amber-400/);
  assert.equal(rowOf(host, 'prf_key').querySelector('[data-usage]'), null);
  assert.doesNotMatch(host.textContent, /unknown|rate limit|utilization|window/i);
  assert.equal(calls.length, 0, 'never reads usage on load');
  done();
});

test('out: an empty amber-tinted track; reset passed: "reset at", no %; stale: dimmed with a text Check now', async () => {
  const stale = { ...usage(100, 49, 6 * 60), windows: [{ kind: 'five_hour', used_pct: 100, resets_at: ago(30) }, { kind: 'weekly', used_pct: 49, resets_at: inMin(2000) }] };
  const { host, done } = await mount([acct({ usage: usage(100, 69), limited_until: inMin(120) }), acct({ id: 'prf_s', name: 'Old', usage: stale })]);
  const out = rowOf(host, 'prf_w').querySelector('[data-usage-row="five_hour"]');
  assert.equal(out.dataset.usageTone, 'out');
  assert.match(out.querySelector('[role="meter"]').className, /bg-amber-500\/20/);
  assert.equal(out.querySelector('[role="meter"]').children.length, 0);
  assert.match(rowOf(host, 'prf_w').textContent, /Out of usage/);
  const s = rowOf(host, 'prf_s');
  assert.ok(s.querySelector('[data-usage-stale]'));
  assert.match(s.querySelector('[data-usage-note]').textContent, /checked 6 h ago/);
  assert.match(s.querySelector('[data-usage-row="five_hour"]').textContent, /^5-hourreset at /);
  assert.equal(s.querySelector('[data-usage-check]').textContent.trim(), 'Check now');
  done();
});

test('no numbers yet: Claude says when they come; a Codex sign-in gets the block and Check now', async () => {
  const { host, done } = await mount([acct({ usage: null }), acct({ id: 'prf_c', engine: 'codex', name: 'Codex', usage: null }), acct({ id: 'prf_out', name: 'Signed out', signed_in: false })]);
  assert.equal(rowOf(host, 'prf_w').querySelector('[data-usage]').textContent, 'Usage· shows after its next reply');
  assert.equal(rowOf(host, 'prf_w').querySelectorAll('[role="meter"]').length, 0);
  assert.equal(rowOf(host, 'prf_w').querySelector('[data-usage-check]').title, 'Check now. This uses a tiny bit of this account.');
  // Codex numbers come only from Check now (PR #604: its turns report none), so the button must be there.
  const codex = rowOf(host, 'prf_c');
  assert.equal(codex.querySelector('[data-usage-note]').textContent, '· press Check now');
  assert.equal(codex.querySelectorAll('[role="meter"]').length, 0);
  const btn = codex.querySelector('[data-usage-check]');
  assert.ok(btn, 'Codex sign-in with no numbers has a Check now button');
  assert.equal(btn.disabled, false);
  assert.equal(btn.title, 'Check now', 'no "uses a tiny bit" for Codex: its read makes no model call');
  assert.equal(rowOf(host, 'prf_out').querySelector('[data-usage]'), null);
  done();
  // A Codex row that can't check (turned off) still shows nothing.
  const off = await mount([acct({ id: 'prf_c', engine: 'codex', name: 'Codex', usage: null, enabled: false })]);
  assert.equal(rowOf(off.host, 'prf_c').querySelector('[data-usage]'), null);
  off.done();
});

test('a failed check shows the server reason; a 429 or no reason keeps the generic line', async () => {
  calls.length = 0;
  reply = async () => new Response('{"error":"Codex is signed out"}', { status: 502, headers: { 'Content-Type': 'application/json' } });
  const a = await mount([acct({ id: 'prf_c', engine: 'codex', name: 'Codex', usage: null })]);
  a.host.querySelector('[data-usage-check]').click();
  await settle(10);
  assert.deepEqual(calls, ['POST /api/profiles/prf_c/usage']);
  const note = a.host.querySelector('[data-usage-note]');
  assert.equal(note.textContent, '· Codex is signed out');
  assert.match(note.className, /text-rose-300/);
  a.done();

  reply = async () => new Response('{"error":"Wait a minute before checking again."}', { status: 429, headers: { 'Retry-After': '60', 'Content-Type': 'application/json' } });
  const b = await mount([acct({ usage: null })]);
  b.host.querySelector('[data-usage-check]').click();
  await settle(10);
  assert.equal(b.host.querySelector('[data-usage-note]').textContent, '· couldn’t check, try again in a minute');
  b.done();

  reply = async () => new Response('', { status: 502 });
  const c = await mount([acct({ usage: null })]);
  c.host.querySelector('[data-usage-check]').click();
  await settle(10);
  assert.equal(c.host.querySelector('[data-usage-note]').textContent, '· couldn’t check, try again in a minute');
  c.done();
  reply = null;
});

test('Check now posts once, shows checking…, then the new numbers; a failure keeps the old bars', async () => {
  calls.length = 0;
  let release;
  reply = () => new Promise((r) => { release = () => r(new Response(JSON.stringify({ ...usage(40, 60, 0), source: 'check' }), { status: 200, headers: { 'Content-Type': 'application/json' } })); });
  const { host, done } = await mount([acct({ usage: usage(7, 56, 5) })]);
  const btn = host.querySelector('[data-usage-check]');
  assert.equal(btn.getAttribute('aria-label'), 'Check usage now');
  assert.equal(btn.disabled, false);
  btn.click();
  await settle();
  assert.deepEqual(calls, ['POST /api/profiles/prf_w/usage']);
  assert.equal(host.querySelector('[data-usage-note]').textContent, '· checking…');
  assert.equal(btn.disabled, true);
  assert.match(btn.querySelector('svg').getAttribute('class'), /animate-spin/);
  assert.equal(host.querySelector('[role="meter"]').getAttribute('aria-valuenow'), '93', 'old bars stay while checking');
  release();
  await settle(10);
  assert.equal(host.querySelector('[role="meter"]').getAttribute('aria-valuenow'), '60');
  assert.equal(host.querySelector('[data-usage-note]').textContent, '· checked just now');
  // Read under a minute ago: it waits, so a second press can't spend more.
  assert.equal(host.querySelector('[data-usage-check]').disabled, true);
  assert.equal(host.querySelector('[data-usage-check]').title, 'You can check again in a minute');
  done();

  reply = async () => new Response('', { status: 502 });
  const second = await mount([acct({ usage: usage(7, 56, 5) })]);
  second.host.querySelector('[data-usage-check]').click();
  await settle(10);
  const note = second.host.querySelector('[data-usage-note]');
  assert.equal(note.textContent, '· couldn’t check, try again in a minute');
  assert.match(note.className, /text-rose-300/);
  assert.equal(second.host.querySelector('[role="meter"]').getAttribute('aria-valuenow'), '93');
  second.done();
  reply = null;
});
