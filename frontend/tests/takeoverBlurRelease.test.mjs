// Rigel's L2 (0.6.9 QA): a button held on the takeover stage when the window loses focus. The
// window never sees that release, so the stage sends it on blur; before, it only forgot the button,
// the page still held it, and the next press gave the page an extra mouseup and click.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLImageElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
// The page image is drawn 1:1 at the top left, so client points are page points
g.HTMLImageElement.prototype.getBoundingClientRect = () => ({ left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0 });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { BrowserTakeoverStage } from './src/components/BrowserTakeoverStage';
`);
const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const act = async (fn) => {
  await fn();
  await settle();
};

async function renderStage() {
  const sent = [];
  const stream = {
    connected: true, controlling: true, busy: false, error: null, frame: { id: 1, width: 800, height: 600 }, location: null, tapFocus: null,
    viewport: null, select: null, dialog: null, cursor: 'default', unsupported: new Set(),
    send: (msg) => sent.push(msg), acknowledgeFrame() {}, clearSelect() {}, clearDialog() {}, clearError() {},
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  await act(async () => {
    root.render(m.React.createElement(m.BrowserTakeoverStage, {
      botName: 'Ada', pageUrl: 'https://www.tiktok.com/login', stream, image: 'data:image/png;base64,AAAA', onHandBack() {}, onRetryControl() {},
    }));
  });
  const stage = document.querySelector('[data-browser-stage]');
  const img = document.querySelector('[data-browser-page]');
  assert.ok(stage && img, 'the stage and page image are drawn');
  const mouse = (target, type, x, y, button = 0) => act(async () => {
    target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button, detail: 1 }));
  });
  const blur = () => act(async () => { window.dispatchEvent(new FocusEvent('blur')); });
  const unmount = () => act(async () => { root.unmount(); host.remove(); });
  const presses = () => sent.filter((s) => s.action === 'mouse' && (s.type === 'down' || s.type === 'up')).map((s) => `${s.type} ${s.button} ${s.x},${s.y} buttons=${s.buttons}`);
  return { img, mouse, blur, unmount, presses, sent };
}

test('a button held when the window loses focus is released on the page', async () => {
  const s = await renderStage();
  await s.mouse(s.img, 'mousedown', 100, 120);
  await s.mouse(window, 'mousemove', 140, 160);
  await s.blur();
  assert.deepEqual(s.presses(), ['down left 100,120 buttons=1', 'up left 140,160 buttons=0'], 'blur sends the release the window never saw');
  // The next click is one clean down and up: no extra mouseup, no stray click
  await s.mouse(s.img, 'mousedown', 300, 300);
  await s.mouse(window, 'mouseup', 300, 300);
  assert.deepEqual(s.presses().slice(2), ['down left 300,300 buttons=1', 'up left 300,300 buttons=0']);
  await s.unmount();
});

test('every held button is released, and a blur with none held sends nothing', async () => {
  const s = await renderStage();
  await s.blur();
  assert.deepEqual(s.presses(), []);
  await s.mouse(s.img, 'mousedown', 10, 20, 0);
  await s.mouse(s.img, 'mousedown', 10, 20, 2);
  await s.blur();
  assert.deepEqual(s.presses().slice(2), ['up left 10,20 buttons=2', 'up right 10,20 buttons=0']);
  // A release that arrives later anyway isn't sent twice
  await s.mouse(window, 'mouseup', 10, 20, 0);
  await s.blur();
  assert.equal(s.presses().length, 4);
  await s.unmount();
});
