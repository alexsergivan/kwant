// Rigel's D1 from the 0.6.9 takeover QA: the views acknowledge a frame from <img onLoad>, but an
// identical frame leaves the image's src unchanged, so no load event fires, no ack goes out and the
// server stops sending frames (after 15 s it drops the socket). The stream hook acks those itself.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; sockets.push(this); }
  send(text) { this.sent.push(JSON.parse(text)); }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { useBrowserStream } from './src/useBrowserStream';
`);
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };

function Probe() {
  m.useBrowserStream('k1', true);
  return null;
}

test('an identical frame is acknowledged at once; a changed one waits for the image to load', async () => {
  const root = m.createRoot(document.createElement('div'));
  m.flushSync(() => root.render(m.React.createElement(Probe)));
  await settle();
  const ws = sockets.at(-1);
  assert.ok(ws, 'the hook opened a socket');
  ws.onopen?.();
  const acks = () => ws.sent.filter((s) => s.action === 'frame_ack').map((s) => s.id);

  ws.receive({ type: 'frame', id: 1, image: 'AAAA', width: 1280, height: 800 });
  await settle();
  assert.deepEqual(acks(), [], 'a first frame is acked by the image load, not the hook');

  ws.receive({ type: 'frame', id: 2, image: 'AAAA', width: 1280, height: 800 });
  await settle();
  assert.deepEqual(acks(), [2], 'the same picture again is acked straight away');

  ws.receive({ type: 'frame', id: 3, image: 'BBBB', width: 1280, height: 800 });
  await settle();
  assert.deepEqual(acks(), [2], 'a new picture waits for its load event');

  // After a reconnect the image is drawn afresh, so its load event acks it
  ws.onclose?.();
  await new Promise((r) => setTimeout(r, 1100));
  const next = sockets.at(-1);
  assert.notEqual(next, ws, 'the hook reconnected');
  next.onopen?.();
  next.receive({ type: 'frame', id: 4, image: 'BBBB', width: 1280, height: 800 });
  await settle();
  assert.deepEqual(next.sent.filter((s) => s.action === 'frame_ack'), [], 'the first frame after a reconnect is not acked early');

  m.flushSync(() => root.unmount());
});

// Mobile 0.6.13 B2: the phone overlay opens fit-width when the browser keeps its own size, and
// gives the size back quietly when it closes without handing back.
test('a resize answered `ignored` is surfaced; restoreViewport is quiet on a closed socket', async () => {
  let stream;
  function Hold() {
    stream = m.useBrowserStream('k2', true);
    return null;
  }
  const root = m.createRoot(document.createElement('div'));
  m.flushSync(() => root.render(m.React.createElement(Hold)));
  await settle();
  const ws = sockets.at(-1);
  ws.onopen?.();
  assert.equal(stream.resizeIgnored, false);
  stream.send({ action: 'resize', width: 390, height: 748, dpr: 2 });
  const id = ws.sent.at(-1).id;
  ws.receive({ type: 'ack', id, ignored: true, viewport: { width: 1440, height: 900 } });
  await settle();
  assert.equal(stream.resizeIgnored, true);
  // a later resize the page takes clears it
  stream.send({ action: 'resize', width: 390, height: 748, dpr: 2 });
  ws.receive({ type: 'ack', id: ws.sent.at(-1).id, viewport: { width: 390, height: 748, dpr: 2 } });
  await settle();
  assert.equal(stream.resizeIgnored, false);
  // an ack for anything else leaves it alone
  ws.receive({ type: 'ack', id, ignored: true });
  stream.send({ action: 'mouse', type: 'down', x: 1, y: 1 }, true);
  ws.receive({ type: 'ack', id: ws.sent.at(-1).id });
  await settle();
  assert.equal(stream.resizeIgnored, false);

  stream.restoreViewport();
  assert.equal(ws.sent.at(-1).action, 'restore_viewport');
  ws.readyState = 3;
  const before = ws.sent.length;
  stream.restoreViewport();
  await settle();
  assert.equal(ws.sent.length, before, 'nothing sent on a closed socket');
  assert.equal(stream.error, null, 'and no "reconnecting" error for it');
  m.flushSync(() => root.unmount());
});
