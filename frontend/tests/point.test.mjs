// 0.9.0 G3 Point at it (release-090/designs/g3-point.md, release-090/g3-contract.md rev 2):
// the pure helpers, the stream's request/answer path, the composer's n/of, and the layer, chip and row.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
// The view measures itself: give every element a 640 × 400 box
Object.defineProperty(dom.window.HTMLElement.prototype, 'clientWidth', { get: () => 640, configurable: true });
Object.defineProperty(dom.window.HTMLElement.prototype, 'clientHeight', { get: () => 400, configurable: true });

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
  export * from './src/point';
  export { useBrowserStream } from './src/useBrowserStream';
  export { readyAttachments, parseAttachments, pointPicks } from './src/attachments';
  export { PointLayer } from './src/components/PointLayer';
  export { PointChip, PointRow } from './src/components/Attachments';
`);
const h = (...a) => m.React.createElement(...a);
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };
const UUID = '6b0c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3';
const UUID2 = '7b0c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3';

const el = (over = {}) => ({ handle: 'k3f9.12', label: 'button.pay', selector: 'main > form#checkout > button.pay', box: { x: 15, y: 303, width: 328, height: 36 }, viewport: { width: 1280, height: 800 }, text: 'Pay €42.00', frame: null, ...over });
const pickWire = (over = {}) => ({ attachment_id: UUID, picture: true, url: 'https://shop.example.com/checkout', host: 'shop.example.com', path: '/checkout', label: 'button.pay', selector: 'main > form#checkout > button.pay', box: { x: 15, y: 303, width: 328, height: 36 }, viewport: { width: 1280, height: 800 }, frame: null, captured_at: '2026-11-10T14:04:09Z', thumb_url: `/api/attachments/${UUID}/content`, ...over });

test('error codes: route on code; an older server without one hides Point; anything else is failed', () => {
  assert.equal(m.pointErrorCode({ code: 'stale', error: 'x' }), 'stale');
  assert.equal(m.pointErrorCode({ error: 'Invalid browser input' }), 'unsupported');
  assert.equal(m.pointErrorCode({ code: 'weird', error: 'Invalid browser input' }), 'failed');
  assert.equal(m.pointErrorCode({ error: 'boom' }), 'failed');
});

test('parsing: a thumb only from our attachment route; frame_host only for a cross-site frame', () => {
  assert.equal(m.parsePointPick(pickWire({ thumb_url: 'https://evil.example/x.png' })).thumb_url, null);
  assert.equal(m.parsePointPick(pickWire()).thumb_url, `/api/attachments/${UUID}/content`);
  assert.equal(m.parsePointPick({ picture: true }), null, 'no attachment id, no pick');
  assert.equal(m.parsePointEl({ handle: 'a', label: 'div' }), null, 'no box, no element');
  const same = m.pointMetaOf(m.parsePointPick(pickWire({ frame: { host: 'shop.example.com', cross_site: false } })), 1, 1);
  assert.equal(same.frame_host, null);
  const cross = m.pointMetaOf(m.parsePointPick(pickWire({ frame: { host: 'pay.example.net', cross_site: true } })), 2, 2);
  assert.equal(cross.frame_host, 'pay.example.net');
});

test('copy: tag, spoken line, chip line, bubble row, footer and limits', () => {
  assert.equal(m.tagText(el()), 'button.pay 328 × 36');
  assert.equal(m.tagText(el({ frame: { host: 'pay.example.net', cross_site: true } })), 'iframe pay.example.net');
  assert.equal(m.spokenText(el()), 'button.pay, 328 by 36, Pay €42.00');
  assert.equal(m.chipLine(m.parsePointPick(pickWire())), 'shop.example.com/checkout');
  assert.equal(m.chipLine(m.parsePointPick(pickWire({ picture: false }))), 'No picture · the code was added');
  assert.equal(m.chipLine(m.parsePointPick(pickWire({ frame: { host: 'pay.example.net', cross_site: true } }))), 'Embedded page from pay.example.net');
  assert.equal(m.chipLine(m.parsePointPick(pickWire({ host: '', path: 'about:blank' }))), 'about:blank');
  const row = m.rowLine(m.pointMetaOf(m.parsePointPick(pickWire()), 1, 1), 'en-GB');
  assert.match(row, /^shop\.example\.com\/checkout · 328 × 36 · \d\d:\d\d$/);
  assert.equal(m.footerText(2), 'Pointing · 2 picked · Esc to stop');
  assert.equal(m.pickRefusal(5, 5), 'Up to 5 parts per message');
  assert.equal(m.pickRefusal(2, 0), 'Up to 10 files per message');
  assert.equal(m.pickRefusal(4, 1), null);
  assert.doesNotMatch(Object.values(m.POINT_COPY).join(' '), /\p{Extended_Pictographic}/u, 'no emoji in copy');
});

test('geometry: object-contain letterbox, view to page and back, clipped outline', () => {
  const fit = m.containFit({ width: 640, height: 500 }, { width: 1280, height: 800 });
  assert.deepEqual(fit, { scale: 0.5, left: 0, top: 50 });
  assert.equal(m.toPage(fit, { width: 1280, height: 800 }, 100, 20), null, 'the letterbox is not the page');
  assert.deepEqual(m.toPage(fit, { width: 1280, height: 800 }, 100, 150), { x: 200, y: 200 });
  assert.deepEqual(m.toView(fit, { width: 1280, height: 800 }, { x: -20, y: 790, width: 100, height: 40 }), { x: 0, y: 445, width: 40, height: 5 });
  assert.equal(m.tagPlacement({ x: 0, y: 4, width: 10, height: 10 }, 400), 'below');
  assert.equal(m.tagPlacement({ x: 0, y: 4, width: 10, height: 395 }, 400), 'inside');
});

test('composer: n/of by the picks\' order, other files not counted; the bubble parses it back', () => {
  const pick = (id, n) => ({ key: id, name: `pointed-${n}.png`, size: 0, progress: 1, state: 'done', point: { label: 'button.pay', pick: m.parsePointPick(pickWire({ attachment_id: id })) }, attachment: { id, name: `pointed-${n}.png`, mime: 'image/png', size: 0 } });
  const file = { key: 'f', name: 'a.txt', size: 3, progress: 1, state: 'done', attachment: { id: '8b0c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3', name: 'a.txt', mime: 'text/plain', size: 3 } };
  const capturing = { key: 'c', name: 'pointed-4.png', size: 0, progress: 0, state: 'uploading', point: { label: 'div' } };
  const items = [pick(UUID2, 2), file, pick(UUID, 3), capturing];
  assert.equal(m.pointPicks(items), 3, 'a capturing pick counts toward the 5');
  const refs = m.readyAttachments(items);
  assert.deepEqual(refs.map((r) => r.name), ['pointed-2.png', 'a.txt', 'pointed-3.png'], 'files keep their names');
  assert.deepEqual(refs.map((r) => r.point && [r.point.n, r.point.of]), [[1, 2], undefined, [2, 2]]);
  const back = m.parseAttachments(JSON.stringify({ attachments: refs }));
  assert.equal(back[0].point.selector, 'main > form#checkout > button.pay');
  assert.equal(back[1].point, undefined);
  assert.equal(m.parseAttachments({ attachments: [{ id: UUID, name: 'p.png', mime: 'image/png', size: 1, point: { n: 'x' } }] })[0].point, undefined, 'a malformed point is a plain file');
});

test('stream: point answers resolve or reject by id and never touch the panel banner', async () => {
  let stream;
  function Hold() {
    stream = m.useBrowserStream('p1', true);
    return null;
  }
  const root = m.createRoot(document.createElement('div'));
  m.flushSync(() => root.render(h(Hold)));
  await settle();
  const ws = sockets.at(-1);
  ws.onopen?.();
  await settle();

  const a = stream.request({ action: 'point_at', x: 1, y: 2 });
  const sentA = ws.sent.at(-1);
  assert.equal(sentA.action, 'point_at');
  ws.receive({ type: 'ack', id: sentA.id, el: el() });
  assert.equal((await a).el.handle, 'k3f9.12');

  const b = stream.request({ action: 'point_neighbour', dir: 'centre' });
  ws.receive({ type: 'error', id: ws.sent.at(-1).id, error: 'No element there', code: 'nothing' });
  await assert.rejects(b, (e) => e.code === 'nothing');
  await settle();
  assert.equal(stream.error, null, 'the panel banner stays clear');

  const c = stream.request({ action: 'point_at', x: 1, y: 2 }, 30);
  await assert.rejects(c, (e) => e.code === 'failed', 'no answer in time is failed');

  const d = stream.request({ action: 'point_neighbour', dir: 'centre' });
  ws.receive({ type: 'error', id: ws.sent.at(-1).id, error: 'Invalid browser input' });
  await assert.rejects(d, (e) => e.code === 'unsupported');
  await settle();
  assert.ok(stream.unsupported.has('point'), 'an older server hides Point');

  const e = stream.request({ action: 'point_pick', handle: 'k', n: 1 });
  ws.onclose?.();
  await assert.rejects(e, (x) => x.code === 'failed', 'a close fails what was in flight');
  m.flushSync(() => root.unmount());
});

function fakeRequest() {
  const calls = [];
  const fn = (payload) => new Promise((resolve, reject) => calls.push({ payload, resolve, reject }));
  return { fn, calls };
}
function fakeSink(picks = 0, filesLeft = 10) {
  const log = [];
  return {
    log,
    sink: {
      picks,
      filesLeft,
      start: (label, n, o) => { log.push(['start', label, n, o.fromMouse]); return `k${n}`; },
      done: (key, p) => log.push(['done', key, p.attachment_id]),
      fail: (key) => log.push(['fail', key]),
    },
  };
}
const key = (node, k) => node.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: k, bubbles: true }));

test('layer: keyboard walks from the centre, Enter picks, Esc stops; announced in the live region', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const req = fakeRequest();
  const { sink, log } = fakeSink();
  let exits = 0;
  m.flushSync(() => root.render(h(m.PointLayer, { request: req.fn, page: { width: 1280, height: 800 }, frameId: 1, pageUrl: 'https://shop.example.com/checkout', phone: false, sink, onExit: () => exits++ })));
  await settle();
  const view = host.querySelector('[role="application"]');
  assert.equal(view.getAttribute('aria-label'), 'Page view. Arrows choose a part, Enter adds it, Escape stops');
  assert.equal(document.activeElement, view, 'the view takes focus');

  key(view, 'ArrowUp');
  assert.deepEqual(req.calls[0].payload, { action: 'point_neighbour', dir: 'centre' }, 'the first arrow starts from the centre');
  req.calls[0].resolve({ type: 'ack', el: el() });
  await settle();
  assert.equal(host.querySelector('[data-point-tag]').textContent, 'button.pay 328 × 36');
  assert.match(host.querySelector('[data-point-tag]').className, /\bbg-violet-600\b/, 'tag is violet-600 for AA (M1)');
  assert.match(view.className, /focus-visible:ring-\[3px\] focus-visible:ring-sky-300/, 'view focus ring (S4)');
  assert.equal(host.querySelector('[data-point-live]').textContent, 'button.pay, 328 by 36, Pay €42.00');

  key(view, 'ArrowUp');
  assert.deepEqual(req.calls[1].payload, { action: 'point_neighbour', handle: 'k3f9.12', dir: 'parent' });
  req.calls[1].resolve({ type: 'ack', el: el(), edge: true });
  await settle();
  assert.equal(host.querySelector('[data-point-tag]').textContent, 'button.pay 328 × 36', 'an edge keeps the selection');

  key(view, 'Enter');
  const pickCall = req.calls.at(-1);
  assert.deepEqual(pickCall.payload, { action: 'point_pick', handle: 'k3f9.12', n: 1 });
  assert.deepEqual(log[0], ['start', 'button.pay', 1, false]);
  pickCall.resolve({ type: 'ack', pick: pickWire() });
  await settle();
  assert.deepEqual(log[1], ['done', 'k1', UUID]);
  assert.equal(host.querySelector('[data-point-live]').textContent, 'Added button.pay');

  key(view, 'Escape');
  assert.equal(exits, 1);
  m.flushSync(() => root.unmount());
  host.remove();
});

test('layer: a 6th pick is refused before asking the server; a failed pick takes its chip back', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const req = fakeRequest();
  const full = fakeSink(5);
  m.flushSync(() => root.render(h(m.PointLayer, { request: req.fn, page: { width: 1280, height: 800 }, frameId: 1, phone: false, sink: full.sink, onExit() {} })));
  await settle();
  const view = host.querySelector('[role="application"]');
  key(view, 'ArrowDown');
  req.calls[0].resolve({ type: 'ack', el: el() });
  await settle();
  key(view, 'Enter');
  await settle();
  assert.equal(req.calls.length, 1, 'no point_pick sent');
  assert.equal(host.querySelector('[data-point-bar="limit"]').textContent, 'Up to 5 parts per message');
  assert.doesNotMatch(host.querySelector('[data-point-bar="limit"]').className, /amber/, 'the limit bar is neutral (C2)');

  const one = fakeSink(1);
  m.flushSync(() => root.render(h(m.PointLayer, { request: req.fn, page: { width: 1280, height: 800 }, frameId: 1, phone: false, sink: one.sink, onExit() {} })));
  key(view, 'Enter');
  const pickCall = req.calls.at(-1);
  assert.equal(pickCall.payload.n, 2);
  pickCall.reject(Object.assign(new m.PointError('failed')));
  await settle();
  assert.deepEqual(one.log.at(-1), ['fail', 'k2']);
  assert.equal(host.querySelector('[data-point-bar="fail"]').getAttribute('role'), 'alert');
  assert.equal(host.querySelector('[data-point-bar="fail"]').textContent, "Couldn't read that part of the page. Try again.");
  m.flushSync(() => root.unmount());
  host.remove();
});

test('layer: a hover answer that is no longer the newest is not drawn', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const req = fakeRequest();
  m.flushSync(() => root.render(h(m.PointLayer, { request: req.fn, page: { width: 1280, height: 800 }, frameId: 1, phone: false, sink: fakeSink().sink, onExit() {} })));
  await settle();
  const view = host.querySelector('[role="application"]');
  key(view, 'ArrowUp');
  key(view, 'ArrowUp');
  req.calls[1].resolve({ type: 'ack', el: el({ handle: 'new', label: 'form#checkout' }) });
  await settle();
  req.calls[0].resolve({ type: 'ack', el: el({ handle: 'old', label: 'div.old' }) });
  await settle();
  assert.match(host.querySelector('[data-point-tag]').textContent, /^form#checkout/);
  m.flushSync(() => root.unmount());
  host.remove();
});

test('sheet line 2 and the short pick name (S2, S3)', () => {
  assert.equal(m.sheetLine('Pay €42.00'), '"Pay €42.00" · tap another part to change it');
  assert.equal(m.sheetLine(''), 'Tap another part to change it');
  assert.equal(m.sheetLine(null), 'Tap another part to change it');
  assert.equal(m.pickName({ label: 'button.pay', selector: 'main > button.pay' }), 'button.pay');
  assert.equal(m.pickTitle({ label: 'button.pay', selector: 'main > button.pay' }), 'main > button.pay');
  assert.equal(m.pickName({ label: '', selector: 'main > button.pay' }), 'main > button.pay');
});

test('chip and row: capturing, ready, no picture, embedded page; remove names the part; crop opens the viewer', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const chip = (item) => {
    m.flushSync(() => root.render(h('ul', null, h(m.PointChip, { item, onRemove() {} }))));
    return host.querySelector('[data-point-chip]');
  };
  const base = { key: 'a', name: 'pointed-1.png', size: 0, progress: 0, state: 'uploading', point: { label: 'button.pay' } };
  let c = chip(base);
  assert.equal(c.dataset.pointChip, 'capturing');
  assert.match(c.textContent, /Taking a picture…/);
  c = chip({ ...base, state: 'done', point: { label: 'button.pay', pick: m.parsePointPick(pickWire()) } });
  assert.equal(c.dataset.pointChip, 'ready');
  assert.match(c.textContent, /shop\.example\.com\/checkout/);
  assert.equal(c.querySelector('button').getAttribute('aria-label'), 'Remove button.pay');
  assert.equal(c.querySelector('.font-mono').textContent, 'button.pay', 'line 1 shows the short label (S2)');
  assert.equal(c.querySelector('.font-mono').getAttribute('title'), 'main > form#checkout > button.pay', 'the full selector is the title');
  assert.equal(c.querySelector('img').getAttribute('src'), `/api/attachments/${UUID}/content`, "the chip uses its own pick's thumbnail (C5)");
  c = chip({ ...base, state: 'done', point: { label: 'x', pick: m.parsePointPick(pickWire({ picture: false, thumb_url: null })) } });
  assert.equal(c.dataset.pointChip, 'no-picture');
  c = chip({ ...base, state: 'done', point: { label: 'x', pick: m.parsePointPick(pickWire({ frame: { host: 'pay.example.net', cross_site: true } })) } });
  assert.equal(c.dataset.pointChip, 'frame');
  assert.match(c.textContent, /Embedded page from pay\.example\.net/);

  const ref = { id: UUID, name: 'pointed-1.png', mime: 'image/png', size: 10, point: m.pointMetaOf(m.parsePointPick(pickWire()), 1, 1) };
  m.flushSync(() => root.render(h(m.PointRow, { file: ref })));
  const open = host.querySelector('[data-point-row] button');
  assert.equal(open.getAttribute('aria-label'), 'Open the picture of button.pay');
  const rowName = host.querySelector('[data-point-row] .font-mono');
  assert.equal(rowName.textContent, 'button.pay');
  assert.equal(rowName.getAttribute('title'), 'main > form#checkout > button.pay');
  m.flushSync(() => open.click());
  await settle();
  assert.ok(document.querySelector('[role="dialog"]'), 'the image viewer opened');
  m.flushSync(() => root.unmount());
  host.remove();
});
