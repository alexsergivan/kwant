// Quiz card live read (ux-pass-0952-pocket-quiz report fix #3, autonomy/quiz-card-live-link/brief.md):
// the chat block's join is null on purpose, so the card reads GET /api/pockets/:code on mount and every
// 15 s while ready or live; a box bound to 127.0.0.1 gets one "Make a link for phones" button that calls
// POST /tunnel; ended or deleted turns the card Ended and drops Open the remote, Big screen and End quiz.
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
  export { POLL_MS, TUNNEL_POLL_MS, GONE_LINE, readJoin, withRead } from './src/components/results/Pocket';
  export { ONLY_THIS_COMPUTER_LINE } from './src/components/results/pocketParts';
  export { pocketApi } from './src/pocketApi';
`);
const { React, createRoot } = m;
const h = React.createElement;
// Timers are mocked (setTimeout only); React and fetch promises settle through setImmediate.
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
// As the server sends it today: no join on the chat block (smart_results/pocket.rs "The join URLs stay null").
const card = (name, over = {}) =>
  mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [{ ...structuredClone(F[name].results.blocks[0]), join: null, ...over }] }, messageId: `m-${name}` }));
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const chip = (v) => v.host.querySelector('[data-pocket]').getAttribute('data-pocket');

const WIFI = 'http://192.168.1.20:8789/p/fq7k2m';
const QUICK = 'https://brave-otter-lamp.trycloudflare.com/p/fq7k2m';
const join = (over = {}) => ({ mode: 'wifi', anywhere_url: null, anywhere_kind: null, wifi_url: WIFI, wifi_note: null, quick_tunnel: true, tunnel: { state: 'off', url: null, note: null }, ...over });
const local = (tunnel = { state: 'off', url: null, note: null }, over = {}) =>
  join({ mode: tunnel.state === 'on' ? 'anywhere' : 'none', wifi_url: null, wifi_note: 'this_computer_only', anywhere_url: tunnel.state === 'on' ? QUICK : null, anywhere_kind: tunnel.state === 'on' ? 'quick' : null, tunnel, ...over });
const read = (state, j) => ({ pocket: { code: 'fq7k2m', kit: 'quiz', title: 'Friday Pub Quiz', state, ended_at: state === 'ended' ? '2026-11-27T21:12:00Z' : null }, summary: null, removed: 0, join: j });

/** A fake box: `answer()` is the next GET's reply; every call is logged. */
function box(answer) {
  const calls = [];
  g.fetch = async (url, init = {}) => {
    calls.push([init.method ?? 'GET', String(url), init.body ? JSON.parse(init.body) : undefined]);
    const r = answer(init.method ?? 'GET', String(url), init.body ? JSON.parse(init.body) : undefined);
    return r instanceof Response ? r : new Response(JSON.stringify(r), { status: 200 });
  };
  return calls;
}
const reads = (calls) => calls.filter(([meth, url]) => meth === 'GET' && url === '/api/pockets/fq7k2m').length;

test('read on mount: the live card uses the box’s join (QR and link) though the chat block has none', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  const calls = box(() => read('open', join()));
  const v = await card('live');
  assert.equal(reads(calls), 1);
  assert.equal(v.host.querySelector('input[aria-label="Join link"]').value, WIFI);
  assert.ok(v.host.querySelector('[data-qr] canvas, [data-qr] img, [data-qr] svg'));
  assert.equal(v.host.querySelector('[data-no-link]'), null);
});

test('StrictMode (the app’s dev double effect) still reads at once, not after 15 s', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', join()));
  const b = { ...structuredClone(F.live.results.blocks[0]), join: null };
  const v = await mount(h(React.StrictMode, null, h(m.MarkdownRenderer, { content: F.live.reply, results: { version: 1, blocks: [b] }, messageId: 'm-strict' })));
  assert.equal(v.host.querySelector('input[aria-label="Join link"]')?.value, WIFI);
});

test('ready card: the read’s join fills the join box and Try it as a player', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', join()));
  const v = await card('ready');
  assert.equal(v.host.querySelector('input[aria-label="Join link"]').value, WIFI);
  assert.equal(button(v, 'Try it as a player').getAttribute('href'), WIFI);
});

test('the poll reads every 15 s while live and stops once the box says ended: Ended, no remote, Big screen or End quiz', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  let state = 'open';
  const calls = box(() => read(state, join()));
  const v = await card('live');
  assert.equal(chip(v), 'live');
  await tick(m.POLL_MS - 1);
  assert.equal(reads(calls), 1, 'not before 15 s');
  await tick(1);
  assert.equal(reads(calls), 2);
  assert.ok(button(v, 'End quiz') && button(v, 'Open the remote') && button(v, 'Big screen'));
  state = 'ended';
  await tick(m.POLL_MS);
  assert.equal(reads(calls), 3);
  assert.equal(chip(v), 'ended');
  for (const t of ['End quiz', 'Open the remote', 'Big screen']) assert.equal(button(v, t), undefined, `${t} is gone`);
  assert.ok(button(v, 'See results'));
  assert.match(v.host.textContent, /Ended \d{1,2}:\d{2} [AP]M/);
  assert.match(v.host.textContent, /The link is closed\./);
  assert.equal(v.host.querySelector('[data-qr]'), null);
  await tick(m.POLL_MS * 4);
  assert.equal(reads(calls), 3, 'no reads after ended');
});

test('deleted (404 not_found): Ended with the deleted line, no End quiz or Keep the results, and the poll stops', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  let gone = false;
  const calls = box(() => (gone ? new Response(JSON.stringify({ error: 'not_found', message: "There's no pocket app here." }), { status: 404 }) : read('open', join())));
  const v = await card('live');
  gone = true;
  await tick(m.POLL_MS);
  assert.equal(chip(v), 'ended');
  assert.match(v.host.textContent, new RegExp(m.GONE_LINE.replace(/[.,]/g, '\\$&')));
  for (const t of ['End quiz', 'Open the remote', 'Big screen', 'Keep the results']) assert.equal(button(v, t), undefined, `${t} is gone`);
  await tick(m.POLL_MS * 3);
  assert.equal(reads(calls), 2);
});

test('pocket apps turned off (404 pocket_apps_off) or a failed read keeps the card as it was and keeps reading', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  const calls = box(() => new Response(JSON.stringify({ error: 'pocket_apps_off' }), { status: 404 }));
  const v = await card('live');
  assert.equal(chip(v), 'live');
  assert.ok(button(v, 'End quiz'));
  await tick(m.POLL_MS);
  assert.equal(reads(calls), 2);
});

test('an ended chat card is not read at all', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  const calls = box(() => read('ended', null));
  await card('ended');
  await tick(m.POLL_MS * 2);
  assert.equal(reads(calls), 0);
});

test('127.0.0.1 box: live opens on From anywhere with one "Make a link for phones" button that calls /tunnel; starting, then QR and link', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  let t = { state: 'off', url: null, note: null };
  const calls = box((meth, url) => {
    if (meth === 'POST' && url.endsWith('/tunnel')) t = { state: 'starting', url: null, note: 'Quick links are free.' };
    return read('open', local(t));
  });
  const v = await card('live');
  const make = v.host.querySelectorAll('[data-action="phone-link"]');
  assert.equal(make.length, 1, 'one button');
  assert.equal(make[0].textContent.trim(), 'Make a link for phones');
  assert.match(make[0].className, /kw-btn-primary/);
  // Mira D2: one violet button; Open the remote waits as a ghost until the link exists.
  assert.match(button(v, 'Open the remote').className, /kw-btn-ghost/);
  assert.doesNotMatch(button(v, 'Open the remote').className, /kw-btn-primary/);
  // Mira D4: nothing to scan yet, so no "late arrivals" promise.
  assert.equal(v.host.querySelector('[data-join="compact"] p').textContent.trim(), 'From anywhere');
  assert.equal(v.host.querySelector('[data-no-link]'), null, 'no "not set up yet" line');
  assert.equal(v.host.querySelector('[data-qr]'), null);
  await click(make[0]);
  assert.deepEqual(calls.find(([meth]) => meth === 'POST'), ['POST', '/api/pockets/fq7k2m/tunnel', { on: true }]);
  assert.match(v.host.querySelector('[data-phone-link="starting"]').textContent, /Making a link for phones/);
  assert.equal(v.host.querySelector('[data-action="phone-link"]'), null);
  // While it starts the card reads every 3 s, not 15.
  const before = reads(calls);
  t = { state: 'on', url: 'https://brave-otter-lamp.trycloudflare.com', note: 'Quick links are free.' };
  await tick(m.TUNNEL_POLL_MS);
  assert.equal(reads(calls), before + 1);
  assert.equal(v.host.querySelector('input[aria-label="Join link"]').value, QUICK);
  assert.ok(v.host.querySelector('[data-qr]'));
  assert.equal(v.host.querySelector('[data-phone-link]'), null);
  assert.match(button(v, 'Open the remote').className, /kw-btn-primary/, 'primary again once the link is on');
  assert.match(v.host.querySelector('[data-join="compact"] p').textContent, /From anywhere · late arrivals can still scan in/);
});

test('Mira D5: the starting line promises the QR code and link, not "the code"', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', local({ state: 'starting', url: null, note: null })));
  const v = await card('live');
  assert.equal(v.host.querySelector('[data-phone-link="starting"]').textContent.trim(), "Making a link for phones. This takes up to a minute; the QR code and link show here when it's ready.");
});

test('Mira D2: Open the remote before the phone link says to make the link first', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  const calls = box(() => read('open', local()));
  const v = await card('live');
  await click(button(v, 'Open the remote'));
  assert.match(v.host.querySelector('[data-link-panel]').textContent, /Make a link for phones first\. The remote and the big screen use it too\./);
  assert.equal(calls.filter(([meth, url]) => meth === 'POST' && url.endsWith('/link')).length, 0, 'no token minted');
});

test('Mira D3: End turns the chip Ended at once, not at the next read, and stops reading', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  const calls = box((meth, url) => (meth === 'POST' && url.endsWith('/end') ? { pocket: { code: 'fq7k2m' } } : read('open', join())));
  const v = await card('live');
  await click(button(v, 'End quiz'));
  await click(button(v, 'End and show winners'));
  assert.equal(chip(v), 'ended');
  assert.match(v.host.textContent, /Ended \d{1,2}:\d{2} [AP]M/);
  assert.ok(v.host.querySelector('[data-done="ok"]'), 'the done line stays');
  for (const t of ['End quiz', 'Open the remote', 'Big screen']) assert.equal(button(v, t), undefined, `${t} is gone`);
  const n = reads(calls);
  await tick(m.POLL_MS * 2);
  assert.equal(reads(calls), n, 'no reads after End');
});

test('127.0.0.1 box, ready: the join box opens on From anywhere; Same Wi-Fi shows one plain line (Mira D1)', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', local()));
  const v = await card('ready');
  const radios = [...v.host.querySelectorAll('[role="radio"]')];
  assert.equal(radios.find((r) => r.getAttribute('aria-checked') === 'true').textContent.trim(), 'From anywhere');
  assert.equal(v.host.querySelectorAll('[data-action="phone-link"]').length, 1);
  await click(radios[0]);
  assert.equal(v.host.querySelector('[data-no-link]').textContent.trim(), m.ONLY_THIS_COMPUTER_LINE);
  assert.equal(m.ONLY_THIS_COMPUTER_LINE, "Phones can't reach this box on Wi-Fi: it only answers this computer. Pick From anywhere to make a link for phones.");
  const box_ = v.host.querySelector('[data-join="full"]');
  assert.doesNotMatch(box_.textContent, /scan the code and play|At a pub/, 'no Wi-Fi promise or limit line');
  assert.equal(box_.textContent.match(/Pick From anywhere/g).length, 1, 'asked once');
  assert.equal(v.host.querySelector('[data-action="phone-link"]'), null);
  // The person's pick sticks through the next read.
  await tick(m.POLL_MS);
  assert.equal(radios[0].getAttribute('aria-checked'), 'true');
});

test('a box still starting keeps today’s Wi-Fi line; only this_computer_only changes it', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', join({ mode: 'none', wifi_url: null, wifi_note: 'no_network' })));
  const v = await card('ready');
  assert.match(v.host.querySelector('[data-no-link]').textContent, /hasn't said its Wi-Fi address yet/);
  assert.equal(v.host.querySelector('[data-action="phone-link"]'), null);
});

test('a failed quick link shows the box’s own words and Try again; a refused POST says why', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', local({ state: 'failed', url: null, note: "Cloudflare didn't answer. Try again in a minute." })));
  const v = await card('live');
  assert.match(v.host.querySelector('[data-phone-link="failed"]').textContent, /Cloudflare didn't answer/);
  assert.equal(button(v, 'Try again').getAttribute('data-action'), 'phone-link');

  g.fetch = async (url, init = {}) =>
    init.method === 'POST' ? new Response(JSON.stringify({ error: 'pocket_ended', message: 'This pocket app has ended.' }), { status: 409 }) : new Response(JSON.stringify(read('open', local())), { status: 200 });
  const w = await card('ready', { code: 'fq7k2m' });
  await click(w.host.querySelector('[data-action="phone-link"]'));
  assert.match(w.host.querySelector('[data-phone-link="failed"]').textContent, /Couldn't make a link for phones: This pocket app has ended\./);
});

test('a class quiz (quick_tunnel false) on a 127.0.0.1 box never offers the button', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  box(() => read('open', local(undefined, { quick_tunnel: false })));
  const v = await card('live');
  assert.equal(v.host.querySelector('[data-action="phone-link"]'), null);
  assert.ok(v.host.querySelector('[data-needs-stable]'));
});

test('pocketApi.tunnel: POST /api/pockets/:code/tunnel {on}', async () => {
  const calls = box(() => read('open', local({ state: 'starting', url: null, note: null })));
  const r = await m.pocketApi.tunnel('fq7k2m', true);
  await m.pocketApi.tunnel('fq7k2m', false);
  assert.deepEqual(calls, [['POST', '/api/pockets/fq7k2m/tunnel', { on: true }], ['POST', '/api/pockets/fq7k2m/tunnel', { on: false }]]);
  assert.equal(r.join.tunnel.state, 'starting');
});

test('readJoin: "none" and a 127.0.0.1 box open on From anywhere; junk is dropped', () => {
  assert.equal(m.readJoin(local()).mode, 'anywhere');
  assert.equal(m.readJoin(join()).mode, 'wifi');
  assert.equal(m.readJoin(join({ mode: 'none', wifi_url: null })).mode, 'wifi');
  assert.equal(m.readJoin({ ...join(), wifi_url: 'javascript:alert(1)' }), null);
  assert.equal(m.readJoin({ mode: 'pigeon' }), null);
  assert.equal(m.readJoin(join({ tunnel: { state: 'exploding' } })).tunnel, undefined);
});
