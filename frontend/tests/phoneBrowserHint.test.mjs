// 30y phone-browser-hint: Mira's 0.9.31 UX pass, fix #4 (autonomy/ux-pass-0931/report.md "4."). On a phone the idle
// bot browser said "Enter a URL above to open a page", but the address bar is hidden below 640 px on purpose
// (typing a URL is a Take over job). On a phone the hint now points at what is on screen: ask a bot, or Take over
// below, and names Take over only while that button is under the preview. Wide keeps today's sentence.
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
let phone = false; // what the width media queries answer
g.window.matchMedia = (q) => ({ matches: phone && /max-width/.test(q), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });

const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; this.bufferedAmount = 0; sockets.push(this); }
  send(text) { this.sent.push(JSON.parse(text)); }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { VirtualMachinePane } from './src/components/VirtualMachinePane';
`);
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0)); };

const WIDE_HINT = 'Enter a URL above to open a page';
const PHONE_HINT = 'Ask a bot to open a page, or use Take over below';

/** The idle, ready pane; `live` sends a blank tab's frame so the stream is connected. */
async function idlePane({ narrow, live }) {
  phone = narrow;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.VirtualMachinePane, {
    isOpen: true, onClose: () => {}, teamId: 'k1', teamName: 'Rook', isRunning: true, activeUrl: 'about:blank',
  })));
  await settle();
  if (live) {
    const ws = sockets.at(-1);
    assert.ok(ws, 'the pane opened a live-view socket');
    ws.onopen?.();
    ws.receive({ type: 'location', url: 'about:blank', title: '' });
    ws.receive({ type: 'frame', id: 1, image: 'AAAA', width: 1280, height: 800 });
    await settle();
  }
  const text = host.textContent;
  const takeOver = host.querySelector('[data-takeover-button]');
  const done = () => { m.flushSync(() => root.unmount()); host.remove(); };
  return { text, takeOver, done };
}

test('phone, idle and live: no "Enter a URL above"; the hint names the Take over button that is under the preview', async () => {
  const p = await idlePane({ narrow: true, live: true });
  try {
    assert.match(p.text, /Browser idle/);
    assert.ok(!p.text.includes('Enter a URL above'), 'a phone has no address bar above the preview');
    assert.ok(p.text.includes(PHONE_HINT), `phone hint: ${p.text}`);
    assert.ok(p.takeOver, 'the Take over button shows in the idle state');
    assert.equal(p.takeOver.textContent.trim(), 'Take over', 'the hint uses the button\'s real label');
  } finally { p.done(); }
});

test('phone, ready but no live view yet: only "Ask a bot", never a Take over that is not there', async () => {
  const p = await idlePane({ narrow: true, live: false });
  try {
    assert.match(p.text, /Browser idle/);
    assert.equal(p.takeOver, null);
    assert.ok(p.text.includes('Ask a bot to open a page'));
    assert.ok(!p.text.includes('Take over below'));
    assert.ok(!p.text.includes('Enter a URL above'));
  } finally { p.done(); }
});

test('wide, idle: the address bar is there and the hint is unchanged', async () => {
  for (const live of [true, false]) {
    const p = await idlePane({ narrow: false, live });
    try {
      assert.match(p.text, /Browser idle/);
      assert.ok(p.text.includes(WIDE_HINT), `wide hint (live ${live}): ${p.text}`);
      assert.ok(!p.text.includes('Ask a bot to open a page'));
    } finally { p.done(); }
  }
});
