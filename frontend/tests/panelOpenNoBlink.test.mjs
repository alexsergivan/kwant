// 28l: right after a bot's run, opening the bot browser panel showed Ben's tab with the lit shop, then the
// strip moved to Shared and dropped to "Browser idle" (Mira, site-clips-0922 design-check :115; frames 46-50).
// With nothing chosen, the pane followed the server's default page (`live`), which still pointed at Ben's job
// page and flipped back to the team page a moment later, and with no named page the picture could be the
// app-level screenshot (Ben's last shot) under any tab. Now, once the pages list is known, the pane always
// names its page (restingPage): the one it already shows, while it is listed and no run is going.
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

// A team's pages, as the server lists them (each test has its own team: a pick is remembered per team)
const teamPages = (team) => {
  const shared = { page: team, team_id: team, kwant_id: null, chat_id: null, job: false, url: 'about:blank', title: '', held: false, idle_seconds: 3 };
  const ben = { page: `${team}#ben#c1`, team_id: team, kwant_id: 'ben', chat_id: 'c1', job: true, url: 'https://shop.example.com/trail/ridgeline-2p', title: 'Ridgeline 2P Tent', held: false, idle_seconds: 1 };
  return {
    SHARED: shared,
    BEN: ben,
    // Just after Ben's run: the default is still his page, newest first
    AFTER_RUN: { team_id: team, live: ben.page, max_job_pages: 4, pages: [ben, shared] },
    // A moment later: the team page was used last, so it is the default and comes first
    MOMENT_LATER: { team_id: team, live: team, max_job_pages: 4, pages: [shared, ben] },
  };
};
const TEAM = 'k1';
const BEN_PAGE = 'k1#ben#c1';
const { SHARED, BEN, AFTER_RUN, MOMENT_LATER } = teamPages(TEAM);

let answer = AFTER_RUN;
let gate = null; // when set, the pages route waits for it
g.fetch = g.window.fetch = async (url) => {
  if (!String(url).includes('/pages')) return { ok: false, status: 404, json: async () => ({}), text: async () => '' };
  if (gate) await gate;
  const body = answer;
  return { ok: true, status: 200, json: async () => body, text: async () => '' };
};

const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; this.bufferedAmount = 0; sockets.push(this); }
  send(text) { this.sent.push(JSON.parse(text)); }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

const pane = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { VirtualMachinePane } from './src/components/VirtualMachinePane';
  export { notifyBrowserPages, restingPage } from './src/browserPages';
`);
const settle = async (ms = 0) => { for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0)); if (ms) await new Promise((r) => setTimeout(r, ms)); };

const APP_SHOT = 'APPLEVELSHOTOFBENSSHOP';
const SHOP = 'SHOPFRAMEOFBENSPAGE';
const pictures = (host) => [...host.querySelectorAll('img')].map((i) => i.getAttribute('src') || '');
const currentTab = (host) => host.querySelector('[data-page-tab][aria-current="true"]')?.getAttribute('data-page-tab') ?? null;

function mount(props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = pane.createRoot(host);
  const render = (p) => pane.flushSync(() => root.render(pane.React.createElement(pane.VirtualMachinePane, {
    isOpen: true, onClose: () => {}, teamId: TEAM, teamName: 'Trailhead', isRunning: true,
    screenshotBase64: APP_SHOT, botNames: { ben: 'Ben' }, activeUrl: 'https://shop.example.com/trail/ridgeline-2p', ...props, ...p,
  })));
  render({});
  return { host, render: (p) => render(p), done: () => { pane.flushSync(() => root.unmount()); host.remove(); } };
}

test('restingPage: the page already shown while it is listed, else the default; null with no list', () => {
  const { restingPage } = pane;
  assert.equal(typeof restingPage, 'function', 'browserPages.ts exports restingPage');
  // No list (an older server, or before the first answer): the old fallback
  assert.equal(restingPage(null, null), null);
  assert.equal(restingPage(BEN_PAGE, null), null);
  // Nothing held yet (the pane just opened): the server's default
  assert.equal(restingPage(null, AFTER_RUN), BEN_PAGE);
  assert.equal(restingPage(null, MOMENT_LATER), TEAM);
  // The default moved a moment later: stays on the page it opened on
  assert.equal(restingPage(BEN_PAGE, MOMENT_LATER), BEN_PAGE);
  assert.equal(restingPage(TEAM, AFTER_RUN), TEAM);
  // That page closed (idle, or the cap made room): the default
  assert.equal(restingPage(BEN_PAGE, { live: TEAM, pages: [SHARED] }), TEAM);
});

test('opened right after a run: one tab from the first frame, never the app-level screenshot, no jump when the default moves', async () => {
  answer = AFTER_RUN;
  let open;
  gate = new Promise((r) => { open = r; });
  const before = sockets.length;
  const m = mount({ running: false });
  try {
    await settle();
    // Before the list answers: no stream to "whatever the default is", and no stale picture
    assert.equal(sockets.length, before, 'the stream waits for the pages list');
    assert.equal(pictures(m.host).some((s) => s.includes(APP_SHOT)), false, 'no app-level screenshot while the list loads');
    assert.ok(m.host.querySelector('[data-frame-holding]'), 'the dark pane while the list loads');
    open(); gate = null;
    await settle();

    assert.equal(sockets.length, before + 1, 'one stream, opened once');
    const ws = sockets.at(-1);
    assert.match(ws.url, /page=k1%23ben%23c1/, "it names Ben's page");
    assert.equal(currentTab(m.host), BEN_PAGE, "Ben's tab is current");
    ws.onopen?.();
    await settle();
    assert.equal(pictures(m.host).some((s) => s.includes(APP_SHOT)), false, "before Ben's first frame: the dark pane, not the app-level shot");
    ws.receive({ type: 'location', url: BEN.url, title: BEN.title });
    ws.receive({ type: 'frame', id: 1, image: SHOP, width: 1280, height: 800 });
    await settle();
    assert.ok(pictures(m.host).some((s) => s.includes(SHOP)), "Ben's own frame shows");

    // A moment later the server's default flips to the team page, and the strip's list reorders
    answer = MOMENT_LATER;
    pane.notifyBrowserPages(TEAM);
    await settle(300);
    assert.equal(currentTab(m.host), BEN_PAGE, "still Ben's tab");
    assert.equal(sockets.length, before + 1, 'no reconnect');
    assert.equal(/Browser idle/.test(m.host.textContent), false, 'not "Browser idle"');
    assert.ok(pictures(m.host).some((s) => s.includes(SHOP)), "still Ben's page");

    // The person picks Shared: that wins
    m.host.querySelector(`[data-page-tab="${TEAM}"]`).dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    await settle();
    assert.equal(currentTab(m.host), TEAM);
    assert.match(sockets.at(-1).url, /page=k1(&|$)/);
  } finally {
    m.done();
    gate = null;
  }
});

test('the page it rests on closes: it moves to the default', async () => {
  const k = teamPages('k3');
  answer = k.AFTER_RUN;
  const m = mount({ running: false, teamId: 'k3' });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'k3#ben#c1');
    answer = { team_id: 'k3', live: 'k3', max_job_pages: 4, pages: [k.SHARED, { ...k.BEN, page: 'k3#ann#c2', kwant_id: 'ann', chat_id: 'c2' }] };
    pane.notifyBrowserPages('k3');
    await settle(300);
    assert.equal(currentTab(m.host), 'k3', "Ben's page closed: Shared");
    assert.match(sockets.at(-1).url, /page=k3(&|$)/);
  } finally {
    m.done();
  }
});

test('during a run it still follows the default (the page the bot uses)', async () => {
  const k = teamPages('k4');
  answer = k.MOMENT_LATER;
  const m = mount({ running: true, teamId: 'k4' });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'k4');
    answer = k.AFTER_RUN;
    pane.notifyBrowserPages('k4');
    await settle(300);
    assert.equal(currentTab(m.host), 'k4#ben#c1', 'the bot opened its page: the pane follows it');
    assert.match(sockets.at(-1).url, /page=k4%23ben%23c1/);
    // The run ends; the default moves back: the pane stays
    m.render({ running: false });
    answer = k.MOMENT_LATER;
    pane.notifyBrowserPages('k4');
    await settle(300);
    assert.equal(currentTab(m.host), 'k4#ben#c1', 'after the run it rests on the page it showed');
  } finally {
    m.done();
  }
});

test('an older server with no pages route: no wait past its answer, and the app-level screenshot is the picture', async () => {
  const fetchWas = g.fetch;
  g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });
  const before = sockets.length;
  const m = mount({ running: false, teamId: 'k2' });
  try {
    await settle();
    assert.equal(sockets.length, before + 1, 'the stream opens once the route answered');
    assert.doesNotMatch(sockets.at(-1).url, /page=/, "the server's default");
    assert.ok(pictures(m.host).some((s) => s.includes(APP_SHOT)), 'the screenshot fallback still works');
  } finally {
    m.done();
    g.fetch = g.window.fetch = fetchWas;
  }
});
