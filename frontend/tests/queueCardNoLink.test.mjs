// queue-card-no-link (ux-pass-queue-ticket #3 + m5): on a default box (127.0.0.1) the open queue card has no link, so
// it says why in the join box's place, with the Funnel How to; with no note it says the address is coming. The greyed
// Big screen and sign point up at that line, and the card says "this computer", never "this Mac".
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'HTMLTextAreaElement', 'HTMLInputElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { validPocket, kitLabel } from './src/components/results/Pocket';
  export { validQueue, queueFromRead, queueMood, QueueCard, hourLabel, QUEUE_UNDO_MS, QUEUE_CLOSE_DEFAULT, QUEUE_POLL_MS, oneLine } from './src/components/results/PocketQueue';
  export { QUEUE_FIXTURES } from './src/components/results/fixtures';
  export { THIS_COMPUTER_WIFI_LINE, QUEUE_STAYS_LINE } from './src/components/results/pocketParts';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  g.fetch = realFetch;
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
  assert.ok(el, 'element to click');
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const F = m.QUEUE_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withJoin = (name, join) => {
  const b = raw(name);
  b.queue = { ...b.queue, join };
  return b;
};
const calls = [];
function stub(view, port = 18991) {
  calls.length = 0;
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    calls.push([method, String(url)]);
    const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'content-type': 'application/json' } });
    if (String(url).endsWith('/api/pockets')) return json({ pockets: [], pocket_port: port });
    return json({ pocket: { state: 'live' }, summary: {}, removed: [], host_view: view.queue, join: view.queue.join });
  };
}
const card = async (b) => {
  const p = m.validPocket(b);
  assert.ok(p, 'validates');
  stub(b);
  const v = await mount(h(m.QueueCard, { block: p, ask: () => {} }));
  v.$ = (s) => v.host.querySelector(s);
  return v;
};
const LOCAL = { mode: 'none', anywhere_kind: null, anywhere_url: null, wifi_url: null, wifi_note: 'this_computer_only' };
const NONE = { mode: 'none', anywhere_kind: null, anywhere_url: null, wifi_url: null };
const SAYS =
  "Phones can't reach this box on Wi-Fi: it only answers this computer. A queue runs for days, so it needs an address that stays: turn on Same Wi-Fi, or use your own https address or Tailscale Funnel.";

test('the stays line is shared in pocketParts', () => {
  assert.equal(`${m.THIS_COMPUTER_WIFI_LINE} ${m.QUEUE_STAYS_LINE}`, SAYS);
});

for (const name of ['empty', 'waiting', 'paused']) {
  test(`${name}, default box (127.0.0.1): no QR, one plain line why, and the How to with the port`, async () => {
    const v = await card(withJoin(name, LOCAL));
    assert.equal(v.$('[data-qr]'), null, 'no QR');
    assert.equal(v.$('[data-join="full"]'), null);
    assert.equal(v.$('[data-join="compact"]'), null);
    const row = v.$('[data-join="none"]');
    assert.ok(row, 'the no-link row');
    assert.equal(row.dataset.noLink, 'this-computer-only');
    assert.equal(row.querySelector('[data-stays-line]').textContent, SAYS);
    assert.equal(row.querySelector('.text-warn, [data-tone="warn"], .kw-chip'), null, 'nothing failed: no warn chip');
    const how = row.querySelector('[data-action="how-to"]');
    assert.ok(how, 'How to');
    assert.equal(how.getAttribute('aria-expanded'), 'false');
    await click(how);
    const steps = row.querySelector('[data-how-to]').textContent;
    assert.match(steps, /tailscale funnel --bg 18991/);
    assert.ok(!/Santa|family chat/.test(steps), steps);
    for (const b of v.host.querySelectorAll('[data-later]')) {
      if (/Big screen|Print/.test(b.textContent)) assert.equal(b.dataset.later, "Needs the customers' link first: see above.");
    }
    // The line comes before the greyed buttons it points to.
    const big = [...v.host.querySelectorAll('[data-later]')].find((b) => /Big screen/.test(b.textContent));
    assert.ok(big, 'greyed Big screen');
    assert.ok(row.compareDocumentPosition(big) & dom.window.Node.DOCUMENT_POSITION_FOLLOWING, 'see above');
  });
}

test('empty, no Wi-Fi note: NoLinkYet says the address is coming; no How to', async () => {
  const v = await card(withJoin('empty', NONE));
  const row = v.$('[data-join="none"]');
  assert.ok(row);
  assert.equal(row.dataset.noLink, 'not-yet');
  assert.match(row.textContent, /^Your box hasn't said its Wi-Fi address yet\. It shows here as soon as it does\.$/);
  assert.equal(row.querySelector('[data-action="how-to"]'), null);
  assert.equal(v.$('[data-qr]'), null);
  assert.ok(!calls.some((c) => c[1].endsWith('/api/pockets')), 'no port lookup without the note');
});

test('a box on 0.0.0.0 (Wi-Fi link): the QR box is back and there is no no-link row', async () => {
  const v = await card(withJoin('empty', { mode: 'wifi', anywhere_kind: null, anywhere_url: null, wifi_url: 'http://192.168.1.20:18991/p/bakery' }));
  assert.ok(v.$('[data-join="full"] [data-qr]'));
  assert.equal(v.$('[data-join="none"]'), null);
  assert.ok(v.$('[data-action="print-sign"]'));
});

test('m5: the card says "this computer", never "this Mac"', async () => {
  for (const name of ['setup', 'empty', 'waiting', 'paused', 'closed']) {
    const v = await card(withJoin(name, LOCAL));
    assert.ok(!/this Mac/.test(v.host.textContent), `${name}: ${v.host.textContent.match(/.{40}this Mac.{20}/)?.[0]}`);
    v.done();
  }
  const v = await card(withJoin('empty', LOCAL));
  assert.match(v.host.textContent, /Tickets stay on this computer\./);
  assert.match(v.host.textContent, /run on this computer, past your daily limit/);
});
