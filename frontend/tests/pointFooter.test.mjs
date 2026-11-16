// 17b point-footer-fits (autonomy/point-footer-fits/brief.md): in the browser panel the "Pointing · n picked" line
// stays in its slot instead of running under Refresh / Restart browser, and one pick through the real pane and
// point layer makes the footer say "1 picked" (the sink is built the way ChatView builds it, from the composer's uploads).
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLImageElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });

const box = { x: 15, y: 303, width: 328, height: 36 };
const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; this.bufferedAmount = 0; sockets.push(this); }
  // The point actions are answered the way the server answers them: an ack with the same id
  send(text) {
    const p = JSON.parse(text);
    this.sent.push(p);
    if (typeof p.id !== 'number') return;
    const el = { handle: 'k3f9.12', label: 'button.pay', box, viewport: { width: 1280, height: 800 }, text: 'Pay' };
    if (p.action === 'point_neighbour' || p.action === 'point_at') setTimeout(() => this.receive({ type: 'ack', id: p.id, el }), 0);
    if (p.action === 'point_pick') {
      setTimeout(() => this.receive({ type: 'ack', id: p.id, pick: { attachment_id: '6b0c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3', picture: true, url: 'https://shop.example.com/', host: 'shop.example.com', path: '/', label: 'button.pay', box, viewport: { width: 1280, height: 800 }, captured_at: '2026-11-15T05:00:00Z' } }), 0);
    }
  }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { VirtualMachinePane, POINT_FOOTER_SLOT } from './src/components/VirtualMachinePane';
  export { pointPicks, uploadSlotsLeft } from './src/attachments';
  export { footerText, POINT_COPY } from './src/point';
`);
const h = m.React.createElement;
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0)); };
const key = (target, k) => target.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));
/** What the footer shows on screen: its text without the screen-reader-only part. */
const shown = (f) => [...f.childNodes].filter((n) => !(n.nodeType === 1 && n.classList.contains('sr-only'))).map((n) => n.textContent).join('');

// ChatView's sink (ChatView.tsx pointSink): the picks are the composer uploads that carry `point`
function Host() {
  const [uploads, setUploads] = m.React.useState([]);
  const sink = {
    picks: m.pointPicks(uploads),
    filesLeft: m.uploadSlotsLeft(uploads),
    start: (label, n) => {
      const k = `point-${n}`;
      setUploads((l) => [...l, { key: k, name: `pointed-${n}.png`, size: 0, mime: 'image/png', progress: 0, state: 'uploading', point: { label } }]);
      return k;
    },
    done: (k, pick) => setUploads((l) => l.map((u) => (u.key === k ? { ...u, state: 'done', progress: 1, point: { label: u.point.label, pick } } : u))),
    fail: (k) => setUploads((l) => l.filter((u) => u.key !== k)),
  };
  return h('div', null,
    h('p', { 'data-composer-picks': '' }, String(m.pointPicks(uploads))),
    h(m.VirtualMachinePane, { isOpen: true, onClose: () => {}, teamId: 'k1', teamName: 'Rook', point: sink, onRefresh: () => {}, onRestartVm: () => {} }));
}

async function pointing() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(h(Host)));
  await settle();
  const ws = sockets.at(-1);
  ws.onopen?.();
  ws.receive({ type: 'location', url: 'https://shop.example.com/', title: 'Shop' });
  ws.receive({ type: 'frame', id: 1, image: 'AAAA', width: 1280, height: 800 });
  await settle();
  const button = host.querySelector('[data-point-button]');
  assert.ok(button, 'Point at it shows once a page is on screen');
  button.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  return { host, root, ws };
}

test('footer slot: one line that shrinks and truncates, so it cannot spill onto the buttons', async () => {
  const { host, root } = await pointing();
  const f = host.querySelector('[data-point-footer]');
  assert.ok(f, 'the pointing line shows while pointing');
  const cls = f.className.split(/\s+/);
  for (const c of ['min-w-0', 'truncate', 'whitespace-nowrap']) assert.ok(cls.includes(c), `slot keeps ${c}`);
  assert.ok(!cls.includes('flex-1') && !f.className.includes('overflow-wrap'), 'no grow-to-a-sliver slot, no wrapping');
  assert.equal(f.querySelectorAll('.whitespace-nowrap').length, 0, 'no nowrap halves wider than the slot inside it');
  const bar = f.parentElement;
  assert.match(bar.className, /\bh-10\b/, 'the bar stays one line high');
  const [left, , right] = bar.children;
  assert.ok(left.hasAttribute('data-browser-footer'));
  assert.match(left.className, /\bflex-1\b/, 'the status label yields first');
  assert.match(left.className, /\bmin-w-3\b/, 'it keeps room for its 12 px icon');
  assert.match(right.className, /\bshrink-0\b/, 'Refresh and Restart browser keep their size');
  assert.ok(/Refresh/.test(right.textContent) && /Restart browser/.test(right.textContent));
  // On screen: "Pointing · 0 picked"; the whole line stays in the title and for screen readers
  assert.equal(shown(f), 'Pointing · 0 picked');
  assert.equal(f.textContent, m.footerText(0));
  assert.equal(f.getAttribute('title'), m.footerText(0));
  m.flushSync(() => root.unmount());
  host.remove();
});

test('count: one pick through the point layer makes the footer say "1 picked", the same count the composer shows', async () => {
  const { host, root, ws } = await pointing();
  const view = host.querySelector('[data-point-layer]');
  assert.ok(view, 'the point layer is over the page');
  key(view, 'ArrowUp');
  await settle();
  key(view, 'Enter');
  await settle();
  assert.ok(ws.sent.some((s) => s.action === 'point_pick'), 'the pick went to the browser');
  const f = host.querySelector('[data-point-footer]');
  assert.equal(host.querySelector('[data-composer-picks]').textContent, '1');
  assert.equal(shown(f), 'Pointing · 1 picked');
  assert.equal(f.getAttribute('title'), 'Pointing · 1 picked · Esc to stop');
  m.flushSync(() => root.unmount());
  host.remove();
});

test('the takeover hand-back line follows the same one-line rule', () => {
  for (const c of ['min-w-0', 'truncate', 'whitespace-nowrap']) assert.ok(m.POINT_FOOTER_SLOT.split(' ').includes(c));
  assert.ok(!m.POINT_FOOTER_SLOT.split(' ').includes('flex-1'));
  // 18g: short enough to fit the slot at 390 next to both buttons
  assert.equal(m.POINT_COPY.handBack, 'Hand back to point');
});

test('the pick count uses tabular figures, so 1 → 3 picks does not nudge the line', () => {
  assert.ok(m.POINT_FOOTER_SLOT.split(' ').includes('tabular-nums'));
});
