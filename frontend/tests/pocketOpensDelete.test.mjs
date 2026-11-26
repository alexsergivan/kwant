// Quiz card follow-ups (autonomy/quiz-card-opens-and-delete/brief.md; Mira's ux-pass-0952-pocket-quiz minors b and h):
// before `opens_at` the host's live card says "Opens Fri 7:30 PM", not Live, and turns Live at that moment by itself;
// the quiz results card gets Delete now (confirm, DELETE /api/pockets/:code, then the deleted state).
import assert from 'node:assert/strict';
import test, { afterEach, mock } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { POCKET_FIXTURES } from './src/components/results/fixtures';
  export { POLL_MS, RESULTS_DELETE, opensAhead, opensLabel, fromRead } from './src/components/results/Pocket';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
};
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  g.fetch = realFetch;
  mock.timers.reset();
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const tick = async (ms) => {
  mock.timers.tick(ms);
  await settle();
};
const F = m.POCKET_FIXTURES;
const card = (name, over = {}) =>
  mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [{ ...structuredClone(F[name].results.blocks[0]), join: null, ...over }] }, messageId: `m-${name}` }));
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const chips = (v) => [...v.host.querySelectorAll('[data-chip]')].map((c) => c.getAttribute('data-chip'));

const WIFI = 'http://192.168.1.20:8789/p/fq7k2m';
const join = { mode: 'wifi', anywhere_url: null, wifi_url: WIFI, wifi_note: null, quick_tunnel: true, tunnel: { state: 'off', url: null, note: null } };
const read = (opens_at) => ({ pocket: { code: 'fq7k2m', kit: 'quiz', title: 'Friday Pub Quiz', state: 'open', opens_at, ended_at: null }, summary: null, removed: 0, join });
function box(answer) {
  const calls = [];
  g.fetch = async (url, init = {}) => {
    calls.push([init.method ?? 'GET', String(url)]);
    const r = answer(init.method ?? 'GET', String(url));
    return r instanceof Response ? r : new Response(r === undefined ? null : JSON.stringify(r), { status: r === undefined ? 204 : 200 });
  };
  return calls;
}

// Wed Nov 25 2026 12:00 UTC; the doors open Fri Nov 27 19:30 UTC.
const NOW = Date.parse('2026-11-25T12:00:00Z');
const OPENS = '2026-11-27T19:30:00Z';

test('opensLabel: under six days away, "Fri 7:30 PM" in the person’s own time zone (same shape as the Gather card)', () => {
  const at = Date.parse(OPENS);
  const want = new Date(at).toLocaleString('en-US', { weekday: 'short', hour: 'numeric', minute: '2-digit' }).replace(',', '');
  assert.equal(m.opensLabel(at, NOW), want);
  assert.match(m.opensLabel(at, NOW), /^[A-Z][a-z]{2} \d{1,2}:\d{2} [AP]M$/);
  assert.match(m.opensLabel(NOW + 6 * 86_400_000 - 60_000, NOW), /^[A-Z][a-z]{2} \d{1,2}:\d{2} [AP]M$/, 'a minute under six days: still weekday only');
});

test('opensLabel: six days or more away, the date too ("Fri, Dec 4, 7:30 PM"), so it can’t name the wrong Friday', () => {
  const at = Date.parse('2026-12-04T19:30:00Z');
  const want = new Date(at).toLocaleString('en-US', { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
  assert.equal(m.opensLabel(at, NOW), want);
  assert.match(m.opensLabel(at, NOW), /^[A-Z][a-z]{2}, [A-Z][a-z]{2} \d{1,2}, \d{1,2}:\d{2} [AP]M$/);
  assert.equal(m.opensLabel(NOW + 6 * 86_400_000, NOW).split(',').length, 3, 'exactly six days: dated');
});

test('opensAhead: only a live card whose read says open with opens_at still ahead; past, missing or junk opens_at stays Live', () => {
  const b = { ...F.live.results.blocks[0] };
  const r = (opens_at) => m.fromRead('fq7k2m', read(opens_at));
  assert.equal(m.opensAhead(b, r(OPENS), NOW), Date.parse(OPENS));
  assert.equal(m.opensAhead(b, r('2026-11-25T11:00:00Z'), NOW), null, 'past');
  assert.equal(m.opensAhead(b, r(null), NOW), null, 'missing');
  assert.equal(m.opensAhead(b, r('soon'), NOW), null, 'junk');
  assert.equal(m.opensAhead(b, null, NOW), null, 'no read yet');
  assert.equal(m.opensAhead({ ...b, state: 'ready' }, r(OPENS), NOW), null, 'ready keeps Ready');
  assert.equal(m.opensAhead({ ...b, code: 'zz9zz9' }, r(OPENS), NOW), null, 'another pocket’s read');
});

test('live before the doors open: a calm "Opens Fri 7:30 PM" chip, no pulse; at opens_at it turns Live by itself', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: NOW });
  box(() => read(OPENS));
  const v = await card('live');
  assert.deepEqual([...new Set(chips(v))], ['opens']);
  const c = v.host.querySelector('[data-chip="opens"]');
  assert.equal(c.textContent.trim(), `Opens ${m.opensLabel(Date.parse(OPENS))}`);
  assert.doesNotMatch(c.className, /bg-sky-600/);
  assert.equal(v.host.querySelector('.kw-pulse'), null, 'no pulse before the doors open');
  // A minute before: still Opens. Just after the moment: Live with its pulse (the timer, not a read).
  mock.timers.setTime(Date.parse(OPENS) - 60_000);
  await settle();
  await tick(m.POLL_MS);
  assert.deepEqual([...new Set(chips(v))], ['opens']);
  await tick(Date.parse(OPENS) - Date.now() + 100);
  assert.deepEqual([...new Set(chips(v))], ['live']);
  assert.ok(v.host.querySelector('.kw-pulse'));
});

test('live with a past or missing opens_at: Live as before', async () => {
  for (const at of ['2026-11-25T11:00:00Z', null]) {
    mock.timers.enable({ apis: ['setTimeout', 'Date'], now: NOW });
    box(() => read(at));
    const v = await card('live');
    assert.deepEqual([...new Set(chips(v))], ['live'], String(at));
    v.done();
    mounted.delete(v);
    mock.timers.reset();
  }
});

test('results card: Delete now asks first (safe answer focused), Escape and Keep until then cancel', async () => {
  const calls = box(() => undefined);
  const v = await card('results');
  const del = button(v, 'Delete now');
  assert.ok(del);
  assert.ok(button(v, 'Run it again next Friday') && button(v, 'Reuse with new questions'));
  await click(del);
  const dlg = v.host.querySelector('[role="alertdialog"][data-confirm="delete"]');
  assert.ok(dlg);
  assert.equal(dlg.getAttribute('aria-label'), m.RESULTS_DELETE.q);
  assert.match(dlg.textContent, /Players' names and answers are deleted from this Mac now, not (on .+|on the deletion date)\. This card stays in your chat as it is\. This can't be undone\./);
  assert.doesNotMatch(dlg.textContent, /scores/, 'the card keeps showing scores, so the copy doesn’t promise they go');
  assert.equal(document.activeElement?.textContent.trim(), 'Keep until then');
  assert.equal(calls.length, 0, 'nothing sent before Yes');
  dlg.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(v.host.querySelector('[role="alertdialog"]'), null);
  await click(button(v, 'Delete now'));
  await click(button(v, 'Keep until then'));
  assert.equal(v.host.querySelector('[role="alertdialog"]'), null);
  assert.ok(button(v, 'Delete now'));
  assert.equal(calls.length, 0);
});

test('results card: Delete it all calls DELETE /api/pockets/:code once, then the deleted line and no actions', async () => {
  const calls = box(() => undefined);
  const v = await card('results');
  await click(button(v, 'Delete now'));
  await click(button(v, 'Delete it all'));
  assert.deepEqual(calls, [['DELETE', '/api/pockets/fq7k2m']]);
  assert.equal(v.host.querySelector('[role="status"][data-done="ok"]').textContent.trim(), "Deleted. Players' names and answers are gone from this Mac.");
  for (const t of ['Delete now', 'Run it again next Friday', 'Reuse with new questions', 'Keep the results']) assert.equal(button(v, t), undefined, `${t} is gone`);
  assert.equal(v.host.querySelector('footer').textContent.trim(), 'Only this card is left, in your chat.');
  assert.ok(v.host.querySelector('[data-winner]'), 'the results stay on the card');
});

test('results card: a failed delete says why and keeps the confirm open to try again (as Santa and Wall); already gone (404 not_found) counts as deleted', async () => {
  box(() => new Response(JSON.stringify({ error: 'busy', message: 'The box is busy.' }), { status: 500 }));
  const v = await card('results');
  await click(button(v, 'Delete now'));
  await click(button(v, 'Delete it all'));
  assert.match(v.host.querySelector('[data-done="error"]').textContent, /^Couldn't delete it: .+ Try again\.$/);
  assert.ok(button(v, 'Delete it all') && !button(v, 'Delete it all').disabled);
  box(() => new Response(JSON.stringify({ error: 'not_found', message: "There's no pocket app here." }), { status: 404 }));
  await click(button(v, 'Delete it all'));
  assert.equal(v.host.querySelector('[data-done="ok"]').textContent.trim(), m.RESULTS_DELETE.done);
  assert.equal(button(v, 'Delete now'), undefined);
});
