// 28e: switching the bot browser pane to another page's tab showed a light-grey empty card with the
// "Finished" pill for about 233 ms before the page appeared (Mira, site-clips-0922 design-check :116).
// The new page's stream had no frame yet, so the pane fell back to the app-level screenshot, which isn't
// tied to any page (the blank one, dimmed to grey). While a chosen page's stream hasn't shown its own good
// frame, the pane now holds the dark pane instead. With no page stream, the screenshot is still the fallback.
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

const TEAM = 'k1';
const BEN_PAGE = 'k1#ben#c1';
const PAGES = {
  team_id: TEAM,
  live: TEAM,
  max_job_pages: 4,
  pages: [
    { page: TEAM, team_id: TEAM, kwant_id: null, chat_id: null, job: false, url: 'about:blank', title: '', held: false, idle_seconds: 3 },
    { page: BEN_PAGE, team_id: TEAM, kwant_id: 'ben', chat_id: 'c1', job: true, url: 'https://shop.example.com/trail/ridgeline-2p', title: 'Ridgeline 2P Tent', held: false, idle_seconds: 1 },
  ],
};
g.fetch = g.window.fetch = async (url) => (String(url).includes('/pages')
  ? { ok: true, status: 200, json: async () => PAGES, text: async () => '' }
  : { ok: false, status: 404, json: async () => ({}), text: async () => '' });

const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; this.bufferedAmount = 0; sockets.push(this); }
  send(text) { this.sent.push(JSON.parse(text)); }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

// Bundled apart, so on main (no helper yet) the pane test still runs and fails on what it shows
const pane = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { VirtualMachinePane } from './src/components/VirtualMachinePane';
`);
const steady = await loadBundled(`export * from './src/steadyFrame';`);
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0)); };

// The app-level screenshot: not tied to any page (here the blank one the clip showed dimmed)
const APP_SHOT = 'APPLEVELBLANKSHOT';
const SHOP = 'SHOPFRAMEOFBENSPAGE';
const OLD = 'SHAREDPAGEFRAME';

const pictures = (host) => [...host.querySelectorAll('img')].map((i) => i.getAttribute('src') || '');
const showsOnly = (host) => ({
  appShot: pictures(host).some((s) => s.includes(APP_SHOT)),
  oldFrame: pictures(host).some((s) => s.includes(OLD)),
  shop: pictures(host).some((s) => s.includes(SHOP)),
  hold: !!host.querySelector('[data-frame-holding]'),
  idle: /Browser idle|Browser unavailable/.test(host.textContent),
});

test('a switch to another page\'s tab: the dark pane until that page\'s own frame, never the app-level screenshot', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = pane.createRoot(host);
  pane.flushSync(() => root.render(pane.React.createElement(pane.VirtualMachinePane, {
    isOpen: true, onClose: () => {}, teamId: TEAM, teamName: 'Trailhead', isRunning: true,
    screenshotBase64: APP_SHOT, botNames: { ben: 'Ben' }, activeUrl: 'about:blank',
  })));
  try {
  await settle();
  const first = sockets.at(-1);
  assert.ok(first, 'the pane opened a live-view socket');
  first.onopen?.();
  first.receive({ type: 'location', url: 'https://example.com/shared', title: 'Shared' });
  first.receive({ type: 'frame', id: 1, image: OLD, width: 1280, height: 800 });
  await settle();

  const ben = host.querySelector(`[data-page-tab="${BEN_PAGE}"]`);
  assert.ok(ben && /^Ben's page:/.test(ben.getAttribute('aria-label')), "Ben's tab is in the strip");
  ben.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();

  const second = sockets.at(-1);
  assert.notEqual(second, first, "the pane reconnected to Ben's page");
  assert.match(second.url, /page=k1%23ben%23c1/);
  second.onopen?.();
  await settle();

  // Between the click and Ben's first frame (frames 122 to 128 of the clip)
  let now = showsOnly(host);
  assert.equal(now.appShot, false, 'the app-level screenshot (the grey "Finished" card) must not show');
  assert.equal(now.oldFrame, false, "the last page's picture must not show either");
  assert.equal(now.idle, false, 'not "Browser idle"');
  assert.equal(now.hold, true, 'the dark pane holds');
  assert.equal(host.querySelector('[data-pane-pill]'), null, 'no "Finished" pill over an empty card');

  // The address arrives before the picture (frames 126 to 128): still the dark pane
  second.receive({ type: 'location', url: 'https://shop.example.com/trail/ridgeline-2p', title: 'Ridgeline 2P Tent' });
  await settle();
  now = showsOnly(host);
  assert.equal(now.appShot, false, 'still no app-level screenshot once the URL is known');
  assert.equal(now.hold, true);

  // Ben's own first good frame: shown
  second.receive({ type: 'frame', id: 1, image: SHOP, width: 1280, height: 800 });
  await settle();
  now = showsOnly(host);
  assert.equal(now.shop, true, "Ben's page shows");
  assert.equal(now.appShot, false);
  assert.equal(now.hold, false);
  } finally {
    pane.flushSync(() => root.unmount());
    host.remove();
  }
});

test('with no page stream, the app-level screenshot is still the picture', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = pane.createRoot(host);
  // An older server: no pages route, no chosen page, and a stream that hasn't sent a frame
  const fetchWas = g.fetch;
  g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });
  pane.flushSync(() => root.render(pane.React.createElement(pane.VirtualMachinePane, {
    isOpen: true, onClose: () => {}, teamId: 'k2', teamName: 'Trailhead', isRunning: true,
    screenshotBase64: APP_SHOT, activeUrl: 'https://example.com/',
  })));
  try {
    await settle();
    assert.equal(showsOnly(host).appShot, true, 'the screenshot fallback still works');
  } finally {
    pane.flushSync(() => root.unmount());
    host.remove();
    g.fetch = g.window.fetch = fetchWas;
  }
});

test('paneFrame: the decision, case by case', () => {
  assert.equal(typeof steady.paneFrame, 'function', 'steadyFrame.ts exports paneFrame');
  const f = (image, id = 1) => ({ id, image, width: 1280, height: 800 });
  const base = { blank: false, frame: null, holding: false, streaming: true, streamPage: null, chosenPage: BEN_PAGE, screenshot: APP_SHOT };
  const pf = (o) => steady.paneFrame({ ...base, ...o });

  // A switch, with a screenshot and no stream frame yet: hold
  assert.deepEqual(pf({}), { hold: true });
  // The new stream's first frame being probed or held (useSteadyFrame: frame null, holding): still hold
  assert.deepEqual(pf({ holding: true, streamPage: BEN_PAGE }), { hold: true });
  // The last page's frame, before the new stream answers: hold, not that picture
  assert.deepEqual(pf({ frame: f(OLD), streamPage: TEAM }), { hold: true });
  // Ben's own first good frame: shown
  assert.deepEqual(pf({ frame: f(SHOP), streamPage: BEN_PAGE }), { image: SHOP });
  // An about:blank page: idle, as before
  assert.deepEqual(pf({ blank: true, frame: f(SHOP), streamPage: BEN_PAGE }), { idle: true });

  // No page stream in use (a real window, one on its way, or the browser not ready): the old fallback
  assert.deepEqual(pf({ streaming: false }), { image: APP_SHOT });
  assert.deepEqual(pf({ streaming: false, screenshot: null }), { idle: true });
  // No chosen page (the server's default): as before 28e
  assert.deepEqual(pf({ chosenPage: null }), { image: APP_SHOT });
  assert.deepEqual(pf({ chosenPage: null, frame: f(SHOP) }), { image: SHOP });
  assert.deepEqual(pf({ chosenPage: null, screenshot: '', holding: true }), { hold: true }, "22o: a held first frame is the dark pane, not 'Browser idle'");
  assert.deepEqual(pf({ chosenPage: null, screenshot: '' }), { idle: true });
});
