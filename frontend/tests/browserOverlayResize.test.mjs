// Mobile 0.6.13, B2 (Mira's audit P0-1): taking over on a phone showed the 1280 px page shrunk to a
// third of the screen. The overlay now sizes the page to its own stage, as the desktop stage does,
// opens a page that keeps its own size (a real window) at fit-width with a one-time pinch hint, and
// gives the bot its size back on every way it closes (Ada's ask), not only on Hand back.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLImageElement', 'Element', 'Node', 'Event', 'PopStateEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'history']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
// The stage's size is whatever the test says; ResizeObserver callbacks run when it asks
let stageSize = { width: 390, height: 748 };
for (const [prop, key] of [['clientWidth', 'width'], ['clientHeight', 'height']]) {
  Object.defineProperty(g.HTMLElement.prototype, prop, {
    configurable: true,
    get() { return this.hasAttribute?.('data-browser-stage') ? stageSize[key] : 0; },
  });
}
const observers = new Set();
g.ResizeObserver = g.window.ResizeObserver = class {
  constructor(cb) { this.cb = cb; }
  observe() { observers.add(this); }
  unobserve() {}
  disconnect() { observers.delete(this); }
};
g.window.devicePixelRatio = 3;
let reducedMotion = false;
g.window.matchMedia = (q) => ({ matches: q.includes('reduce') && reducedMotion, addEventListener() {}, removeEventListener() {} });
let visibility = 'visible';
Object.defineProperty(g.document, 'visibilityState', { configurable: true, get: () => visibility });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { BrowserControlOverlay, fitWidthView, PINCH_HINT_KEY, PINCH_HINT_MS, PINCH_HINT_FADE_MS, OVERLAY_RESIZE_DEBOUNCE_MS } from './src/components/BrowserControlOverlay';
  export { fitView, zoomToBox } from './src/browserGestures';
`);
const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
// Waits for `ok()` up to `ms`, so a loaded CI machine doesn't fail a timing step
const until = async (ok, ms = 2000) => {
  const end = Date.now() + ms;
  while (!ok() && Date.now() < end) await wait(10);
  await settle();
};

async function renderOverlay(props = {}) {
  const sent = [];
  const calls = { handBack: 0, restore: 0 };
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const base = {
    botName: 'Ada', pageUrl: 'https://gearhub.example/checkout', image: 'data:image/png;base64,AAAA', onFrameDone() {},
    send: (msg) => sent.push(msg), busy: false, error: null, tapFocus: null, captcha: null, solved: false,
    onHandBack: () => { calls.handBack += 1; }, restoreSize: () => { calls.restore += 1; },
    page: { width: 1280, height: 800 }, controlling: true,
  };
  let current = { ...base, ...props };
  const draw = () => root.render(m.React.createElement(m.BrowserControlOverlay, current));
  draw();
  await settle();
  return {
    sent, calls,
    resizes: () => sent.filter((s) => s.action === 'resize'),
    rerender: async (next) => { current = { ...current, ...next }; draw(); await settle(); },
    img: () => document.querySelector('[data-browser-fullscreen] img'),
    hint: () => document.querySelector('[data-pinch-hint]'),
    unmount: async () => { root.unmount(); host.remove(); await settle(); },
  };
}

test('the overlay sizes the page to its own stage as it opens', async () => {
  stageSize = { width: 390, height: 748 };
  const o = await renderOverlay();
  assert.deepEqual(o.resizes(), [{ action: 'resize', width: 390, height: 748, dpr: 2 }], 'one resize at once, dpr capped at 2');
  await o.unmount();
});

test('no resize before the person controls the page, one as soon as they do', async () => {
  stageSize = { width: 360, height: 700 };
  const o = await renderOverlay({ controlling: false });
  assert.deepEqual(o.resizes(), []);
  await o.rerender({ controlling: true });
  assert.deepEqual(o.resizes().map((r) => r.width), [360]);
  await o.unmount();
});

test('a rotate sizes the page again once it settles', async () => {
  stageSize = { width: 412, height: 780 };
  const o = await renderOverlay();
  stageSize = { width: 892, height: 330 };
  for (const ob of observers) ob.cb([]);
  await settle();
  assert.equal(o.resizes().length, 1, 'not straight away');
  await until(() => o.resizes().length > 1);
  assert.deepEqual(o.resizes().map((r) => [r.width, r.height]), [[412, 780], [892, 360]], 'height clamps to the 360 minimum');
  await o.unmount();
});

test('a page that keeps its own size opens fit-width and top-aligned, not fit-contain', async () => {
  // a phone on its side: contain would leave bands beside the page
  stageSize = { width: 844, height: 340 };
  const page = { width: 1280, height: 800 };
  const o = await renderOverlay({ resizeIgnored: true, page });
  const scale = 844 / 1280;
  assert.notEqual(m.fitView(stageSize, page).scale, scale, 'contain would differ here');
  assert.deepEqual(m.fitWidthView(stageSize, page), { scale, tx: 0, ty: 0 });
  assert.equal(o.img().style.transform, `translate(0px, 0px) scale(${scale})`);
  await o.unmount();
});

test('a known CAPTCHA box still wins over fit-width', async () => {
  stageSize = { width: 390, height: 748 };
  const page = { width: 1280, height: 800 };
  const target = { x: 500, y: 300, width: 300, height: 80 };
  const o = await renderOverlay({ resizeIgnored: true, page, captcha: { target } });
  const v = m.zoomToBox(target, stageSize, page);
  assert.equal(o.img().style.transform, `translate(${v.tx}px, ${v.ty}px) scale(${v.scale})`);
  await o.unmount();
});

test('the pinch hint shows once per device, for 4 s, then fades', async () => {
  localStorage.removeItem(m.PINCH_HINT_KEY);
  stageSize = { width: 390, height: 748 };
  const sized = await renderOverlay();
  assert.equal(sized.hint(), null, 'no hint when the page takes the stage size');
  await sized.unmount();

  const o = await renderOverlay({ resizeIgnored: true });
  const hint = o.hint();
  assert.ok(hint, 'the hint shows');
  assert.match(hint.textContent, /Pinch to zoom · drag to move/);
  assert.match(hint.className, /h-10/);
  assert.match(hint.className, /bottom-4/);
  assert.match(hint.className, /bg-\[#231d4b\]/);
  assert.match(hint.className, /text-\[13px\] leading-4 text-\[#ecebfa\]/);
  assert.match(hint.className, /rounded-full/);
  assert.ok(hint.querySelector('svg'), 'with the Move icon');
  assert.equal(localStorage.getItem(m.PINCH_HINT_KEY), '1');
  await wait(m.PINCH_HINT_MS - 500);
  assert.match(o.hint()?.className ?? '', /opacity-100/, 'still up just before 4 s');
  await until(() => /opacity-0/.test(o.hint()?.className ?? '') || !o.hint(), 2500);
  assert.match(o.hint()?.className ?? '', /opacity-0/, 'fading');
  await until(() => !o.hint());
  assert.equal(o.hint(), null, 'gone');
  await o.unmount();

  const again = await renderOverlay({ resizeIgnored: true });
  assert.equal(again.hint(), null, 'never twice on one device');
  await again.unmount();
});

test('under reduced motion the hint goes after 4 s with no fade', async () => {
  localStorage.removeItem(m.PINCH_HINT_KEY);
  reducedMotion = true;
  try {
    const o = await renderOverlay({ resizeIgnored: true });
    assert.ok(o.hint());
    let faded = false;
    await until(() => { faded ||= /opacity-0/.test(o.hint()?.className ?? ''); return !o.hint(); }, m.PINCH_HINT_MS + 2500);
    assert.equal(o.hint(), null);
    assert.equal(faded, false, 'never faded');
    await o.unmount();
  } finally {
    reducedMotion = false;
  }
});

test('the status line under the bar is 13/18 and keeps its colour', async () => {
  const o = await renderOverlay({ captcha: { target: null } });
  const line = document.querySelector('[data-browser-fullscreen] p[role="status"]');
  assert.match(line.className, /text-\[13px\] leading-\[18px\]/);
  assert.match(line.className, /text-\[#fcd34d\]/);
  assert.doesNotMatch(line.className, /text-xs/);
  await o.unmount();
});

test('the app going to the background gives the size back; coming back sizes the page again', async () => {
  stageSize = { width: 390, height: 748 };
  const o = await renderOverlay();
  visibility = 'hidden';
  document.dispatchEvent(new window.Event('visibilitychange'));
  await settle();
  assert.equal(o.calls.restore, 1);
  visibility = 'visible';
  document.dispatchEvent(new window.Event('visibilitychange'));
  await settle();
  assert.equal(o.resizes().length, 2, 'sized again at once');
  await o.unmount();
  assert.equal(o.calls.restore, 2, 'and closing gives it back once more');
});

test('closing the overlay any other way gives the size back once', async () => {
  const o = await renderOverlay();
  await o.unmount();
  assert.equal(o.calls.restore, 1);
  assert.equal(o.calls.handBack, 0);
});

test('Hand back leaves the restore to release_control: no second restore', async () => {
  const o = await renderOverlay();
  const button = [...document.querySelectorAll('button')].find((b) => /Hand back/.test(b.textContent));
  button.click();
  await settle();
  assert.equal(o.calls.handBack, 1);
  await o.unmount();
  assert.equal(o.calls.restore, 0);
});

test("the phone's back gesture hands back instead of leaving the chat", async () => {
  await until(() => !history.state?.kwBrowserOverlay);
  assert.ok(!history.state?.kwBrowserOverlay);
  const o = await renderOverlay();
  assert.ok(history.state?.kwBrowserOverlay, 'the overlay has its own history entry');
  history.back();
  await until(() => o.calls.handBack > 0);
  assert.equal(o.calls.handBack, 1);
  await o.unmount();
});

test('Hand back takes the overlay history entry off again', async () => {
  const o = await renderOverlay();
  assert.ok(history.state?.kwBrowserOverlay);
  await o.unmount();
  await until(() => !history.state?.kwBrowserOverlay);
  assert.ok(!history.state?.kwBrowserOverlay, 'back to the entry before the overlay');
  assert.equal(o.calls.handBack, 0, 'its own pop is not a back gesture');
});

test('no resize without a page size from the stage', async () => {
  stageSize = { width: 0, height: 0 };
  const o = await renderOverlay();
  assert.deepEqual(o.resizes(), []);
  await o.unmount();
  stageSize = { width: 390, height: 748 };
});
