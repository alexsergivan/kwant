// santa-join-on-local-box (autonomy/santa-join-on-local-box/brief.md, Mira's ux-pass-santa minor h): on a default
// Mac box that only answers 127.0.0.1, Santa refuses the quick tunnel, so the join is none. The card says why
// (the quiz card's this-computer line), says Santa needs an address that stays, and opens the Funnel how-to with
// the guest port from the pocket list. A box with a Wi-Fi address and an https/Funnel box look as before.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
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
  export { validPocket } from './src/components/results/Pocket';
  export { validSantaJoin, santaFromHost } from './src/components/results/PocketSanta';
  export { ONLY_THIS_COMPUTER_LINE, THIS_COMPUTER_WIFI_LINE, SANTA_STAYS_LINE } from './src/components/results/pocketParts';
  export { SANTA_FIXTURES, santaHostView, SANTA_LIVE_JOIN } from './src/components/results/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const WIFI_LINE = "Phones can't reach this box on Wi-Fi: it only answers this computer.";
const STAYS_LINE = 'Santa runs for weeks, so phones need an address that stays: your own https address or Tailscale Funnel.';

const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  g.fetch = realFetch;
});
/** The pocket list answers the guest port; the card's own live read fails, so it keeps the chat block. */
function box(port = 7481) {
  const calls = [];
  g.fetch = async (url, init) => {
    calls.push([init?.method ?? 'GET', url]);
    if (url === '/api/pockets') return new Response(JSON.stringify({ enabled: true, pocket_port: port, kits: [], pockets: [] }), { status: 200 });
    return new Response('{}', { status: 503 });
  };
  return calls;
}
const F = m.SANTA_FIXTURES;
async function card(name) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [structuredClone(F[name].results.blocks[0])] }, messageId: `m-${name}` }));
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const squash = (t) => t.replace(/\s+/g, ' ');

test('one line per kit: the quiz line is unchanged, Santa shares its first half', () => {
  assert.equal(m.THIS_COMPUTER_WIFI_LINE, WIFI_LINE);
  assert.equal(m.ONLY_THIS_COMPUTER_LINE, `${WIFI_LINE} Pick From anywhere to make a link for phones.`, 'the quiz card says what it said');
  assert.equal(m.SANTA_STAYS_LINE, STAYS_LINE);
});

test('the join keeps the this-computer note; a quick tunnel is still refused', () => {
  const none = { mode: 'none', anywhere_kind: null, anywhere_url: null, wifi_url: null, wifi_note: 'this_computer_only' };
  assert.equal(m.validSantaJoin(none).wifi_note, 'this_computer_only');
  assert.equal(m.validSantaJoin({ ...none, wifi_note: 'something else' }).wifi_note, undefined, 'only the note the card words');
  assert.equal(m.validSantaJoin({ ...none, mode: 'wifi', wifi_url: 'http://192.168.1.20:7480/p/x' }).wifi_note, undefined, 'a Wi-Fi link wins over the note');
  assert.equal(m.validSantaJoin({ ...none, quick_tunnel: true }), undefined, 'quick tunnel on: still refused');

  // The live read on a 127.0.0.1 box where a quick tunnel is running: dropped, and the note stays.
  const read = { ...m.SANTA_LIVE_JOIN, mode: 'anywhere', anywhere_url: 'https://x.trycloudflare.com/p/of6sn7', anywhere_kind: 'quick', wifi_url: null, wifi_note: 'this_computer_only', quick_tunnel: true };
  const s = m.santaFromHost(m.santaHostView(0), { now: Date.parse('2099-12-01T12:00:00'), join: read });
  assert.equal(s.join.mode, 'none');
  assert.equal(s.join.anywhere_url, null);
  assert.equal(s.join.wifi_note, 'this_computer_only');
});

test('127.0.0.1 box: the this-computer line, the stays line, and the Funnel how-to open with the guest port', async () => {
  const calls = box(7481);
  const v = await card('localBox');
  const join = v.host.querySelector('[data-join="full"]');
  assert.equal(join.dataset.joinMode, 'none');
  assert.equal(join.querySelector('[data-qr]'), null, 'no dead QR');
  assert.equal(join.querySelectorAll('[role=radio]').length, 0, 'nothing to pick');
  const said = join.querySelector('[data-no-link="this-computer-only"]');
  assert.ok(said, 'the this-computer note is said');
  assert.ok(said.querySelector('svg.lucide-wifi-off'), 'a Wi-Fi-off glyph, not a Wi-Fi one');
  assert.equal(said.querySelector('svg.lucide-wifi'), null);
  assert.match(squash(said.textContent), new RegExp(`Only this Mac can open the page right now\\. ${WIFI_LINE.replace(/[.:']/g, '\\$&')}`));
  assert.equal(join.querySelector('[data-stays-line]').textContent, `${STAYS_LINE} Funnel is free and takes about 5 minutes.`);
  assert.doesNotMatch(join.textContent, /hasn't said its Wi-Fi address/);
  assert.doesNotMatch(join.textContent, /No address that stays the same is set up/, 'one reason, not two');

  const how = button(v, 'How to');
  assert.equal(how.getAttribute('aria-expanded'), 'true', 'the how-to is open');
  const steps = [...join.querySelectorAll('[data-how-to] li')].map((li) => li.textContent);
  assert.equal(steps.length, 5);
  assert.match(steps[2], /tailscale funnel --bg 7481/, 'the guest port comes from the pocket list');
  assert.ok(calls.some(([meth, url]) => meth === 'GET' && url === '/api/pockets'), 'read the pocket list once');
  assert.equal(calls.filter(([, url]) => url === '/api/pockets').length, 1);

  assert.match(join.textContent, /A quick link isn't offered/);
  assert.equal([...v.host.querySelectorAll('button')].some((b) => /quick|phone link/i.test(b.textContent)), false, 'Santa never offers a quick-link button');
  await click(how);
  assert.equal(how.getAttribute('aria-expanded'), 'false', 'it still closes');
});

test('127.0.0.1 box, no pocket list: the how-to still opens and says where the port is', async () => {
  g.fetch = async () => new Response('{}', { status: 503 });
  const v = await card('localBox');
  const steps = [...v.host.querySelectorAll('[data-how-to] li')].map((li) => li.textContent);
  assert.equal(steps.length, 5);
  assert.match(steps[2], /tailscale funnel --bg </);
});

test('127.0.0.1 box with Funnel on: From anywhere as today, the Wi-Fi tab says why instead of "not yet"', async () => {
  box();
  const v = await card('localBoxFunnel');
  const join = v.host.querySelector('[data-join="full"]');
  assert.equal(join.dataset.joinMode, 'anywhere');
  assert.match(join.textContent, /Uses your Mac's Tailscale Funnel address\. It stays the same/);
  assert.equal(join.querySelector('input').value, 'https://oleks-mac.tail4b2e.ts.net/p/sn4tq9');
  assert.ok(join.querySelector('[data-qr]'));

  await click(button(v, 'Same Wi-Fi'));
  assert.equal(join.querySelector('[data-qr]'), null, 'no QR for a link phones cannot open');
  assert.equal(join.querySelector('input'), null);
  assert.equal(squash(join.querySelector('[data-no-link="this-computer-only"]').textContent), `${WIFI_LINE} Send the From anywhere link instead: it works on Wi-Fi too.`);
  assert.doesNotMatch(join.textContent, /hasn't said its Wi-Fi address/);
});

test('a box with a Wi-Fi address is unchanged: its QR, the closed how-to, the Wi-Fi port, no list read', async () => {
  const calls = box();
  const v = await card('noAddress');
  const join = v.host.querySelector('[data-join="full"]');
  assert.equal(join.dataset.joinMode, 'wifi');
  assert.ok(join.querySelector('[data-qr]'), 'the Wi-Fi QR');
  assert.equal(join.querySelector('input').value, 'http://192.168.1.20:7480/p/sn4tq9');
  assert.match(join.textContent, /Same Wi-Fi only, for now\./);
  assert.match(join.textContent, /No address that stays the same is set up on this Mac\./);
  assert.equal(join.querySelector('[data-no-link="this-computer-only"]'), null);
  assert.equal(join.querySelector('[data-stays-line]'), null);
  const wifiLine = [...join.querySelectorAll('p')].find((p) => /Same Wi-Fi only, for now\./.test(p.textContent));
  assert.ok(wifiLine.querySelector('svg.lucide-wifi'), 'the Wi-Fi box keeps its Wi-Fi glyph');
  const how = button(v, 'How to');
  assert.equal(how.getAttribute('aria-expanded'), 'false');
  await click(how);
  assert.match(join.querySelectorAll('[data-how-to] li')[2].textContent, /tailscale funnel --bg 7480/);
  assert.equal(calls.filter(([, url]) => url === '/api/pockets').length, 0);
});
