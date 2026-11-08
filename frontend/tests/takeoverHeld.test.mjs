// 0.7.6 "Takeover refused" (copy.md, Lin's #429): Take over in a second window while the person
// already holds the browser in another one used to say "@team is finishing a step…" and retry every
// second, which can never win. The server's refusal now carries code held_by_person: the pane shows
// Mira's sentence in the same violet box with a static Hand icon, no spinner and no retry. bot_busy
// and an older server's error with no code keep today's wait-and-retry.
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
  export { VirtualMachinePane, TAKEOVER_RETRY_MS } from './src/components/VirtualMachinePane';
  export { BOT_BUSY_TEXT, HELD_BY_PERSON_TEXT, takeoverRefusedText } from './src/useBrowserStream';
`);
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0)); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const HELD = "You're controlling this browser in another window. Press Hand back there to take over here.";
const BUSY_WIRE = 'Browser is controlled by a person. Retry after control is returned.';

test('the held sentence is Mira\'s copy, word for word', () => {
  assert.equal(m.HELD_BY_PERSON_TEXT, HELD);
});

test('a refusal maps by code; no code falls back to the text, then to the busy bot', () => {
  assert.equal(m.takeoverRefusedText({ code: 'held_by_person', error: HELD }), HELD);
  assert.equal(m.takeoverRefusedText({ code: 'bot_busy', error: BUSY_WIRE }), m.BOT_BUSY_TEXT);
  assert.equal(m.takeoverRefusedText({ code: 'failed', error: 'Something else' }), m.BOT_BUSY_TEXT);
  assert.equal(m.takeoverRefusedText({ error: BUSY_WIRE }), m.BOT_BUSY_TEXT, 'an older server: as today');
  assert.equal(m.takeoverRefusedText({ error: HELD }), HELD, 'the held sentence without a code still reads as held');
  // A code wins over the text: bot_busy never turns into the held note
  assert.equal(m.takeoverRefusedText({ code: 'bot_busy', error: 'in another window' }), m.BOT_BUSY_TEXT);
});

async function openPane() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.VirtualMachinePane, {
    isOpen: true, onClose: () => {}, teamId: 'k1', teamName: 'Rook', isRunning: true,
  })));
  await settle();
  const ws = sockets.at(-1);
  assert.ok(ws, 'the pane opened a live-view socket');
  ws.onopen?.();
  ws.receive({ type: 'location', url: 'https://example.com/', title: 'Example' });
  ws.receive({ type: 'frame', id: 1, image: 'AAAA', width: 1280, height: 800 });
  await settle();
  const takeOver = host.querySelector('[data-takeover-button]');
  assert.ok(takeOver && /Take over/.test(takeOver.textContent), 'the pane offers Take over');
  takeOver.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  const takes = () => ws.sent.filter((s) => s.action === 'take_control').length;
  assert.equal(takes(), 1, 'pressing Take over asks once');
  return { host, root, ws, takes };
}

test('held_by_person: the sentence in the violet box, a static Hand, no spinner and no retry', async () => {
  const { host, root, ws, takes } = await openPane();
  ws.receive({ type: 'error', error: HELD, code: 'held_by_person' });
  await settle();

  const box = host.querySelector('[data-takeover-wait]');
  assert.ok(box, 'the takeover notice shows');
  assert.ok(box.hasAttribute('data-takeover-held'));
  assert.equal(box.getAttribute('role'), 'status');
  assert.ok(box.textContent.includes(HELD));
  assert.ok(!/finishing a step/i.test(box.textContent));
  assert.ok(!/Trying every second/i.test(host.textContent), 'no "Trying every second" line');
  assert.equal(box.querySelector('.animate-spin'), null, 'no spinner');
  assert.ok(box.querySelector('svg.lucide-hand'), 'the static Hand icon');
  assert.ok(!host.querySelector('[role="alert"]')?.textContent.includes(HELD), 'not also shown as a red error');
  assert.ok(box.querySelector('[aria-label="Close"]'), 'the × close stays');

  await sleep(m.TAKEOVER_RETRY_MS * 2.5);
  await settle();
  assert.equal(takes(), 1, 'no retry while the other window holds it');

  // The other window hands back: nothing moves here until the person presses Take over again
  ws.receive({ type: 'control', active: false });
  await settle();
  await sleep(m.TAKEOVER_RETRY_MS * 1.5);
  assert.equal(takes(), 1, 'this window does not take over by itself');

  box.querySelector('[aria-label="Close"]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.equal(host.querySelector('[data-takeover-held]'), null, '× closes the note');
  root.unmount();
});

for (const [name, message] of [
  ['bot_busy', { type: 'error', error: BUSY_WIRE, code: 'bot_busy' }],
  ['no code (an older server)', { type: 'error', error: BUSY_WIRE }],
]) {
  test(`${name}: still "finishing a step" with the spinner, and it retries`, async () => {
    const { host, root, ws, takes } = await openPane();
    ws.receive(message);
    await settle();
    const box = host.querySelector('[data-takeover-wait]');
    assert.ok(box, 'the takeover notice shows');
    assert.equal(box.hasAttribute('data-takeover-held'), false);
    assert.ok(/Rook is finishing a step/.test(box.textContent));
    assert.ok(box.querySelector('.animate-spin'), 'the spinner');
    assert.ok(/Trying every second/.test(box.textContent));

    await sleep(m.TAKEOVER_RETRY_MS + 300);
    await settle();
    assert.ok(takes() >= 2, 'it asked again after a second');
    root.unmount();
  });
}
