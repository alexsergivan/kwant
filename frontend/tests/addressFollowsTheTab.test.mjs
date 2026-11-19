// 28r address-follows-the-tab: after a pick of another tab the address field kept the old tab's address until
// the new stream's first `location`, and on open it showed `activeUrl` (any bot's last address). Mira's
// browser-switch-no-flash/design-check.md:31 and Rigel's panel-opens-on-the-bot/qa.md follow-up.
// The field now follows the chosen page: its listed url at once, then only that page's own stream.
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
const TEAM = 'o1';
const BEN_PAGE = 'o1#ben#c1';
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
  export { notifyBrowserPages, pageOpened, restingPage, ownNewestPage, fetchBrowserPages, addressToShow } from './src/browserPages';
  export { PAGES_WAIT_MS } from './src/useBrowserPages';
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



const ANN = (team) => ({ page: `${team}#ann#c2`, team_id: team, kwant_id: 'ann', chat_id: 'c2', job: true, url: 'https://news.example.com/', title: 'News', held: false, idle_seconds: 9 });
const NAMES = { ben: 'Ben', ann: 'Ann' };
const OTHER = 'https://elsewhere.example.org/last-bot-address';
const field = (host) => host.querySelector('input[placeholder="https://..."]');
const click = (host, page) => pane.flushSync(() => host.querySelector(`[data-page-tab="${page}"]`).dispatchEvent(new window.MouseEvent('click', { bubbles: true })));
const threeTabs = (team) => { const k = teamPages(team); return { ...k, ANN: ANN(team), LIST: { team_id: team, live: team, max_job_pages: 4, pages: [k.SHARED, k.BEN, ANN(team)] } }; };

test('addressToShow: the chosen page own stream, else its listed url, else (nothing chosen) activeUrl', () => {
  const { addressToShow } = pane;
  assert.equal(typeof addressToShow, 'function', 'browserPages.ts exports addressToShow');
  const loc = (url, page) => ({ url, page });
  assert.equal(addressToShow({ location: loc('https://shop.example.com/cart', 'p#ben'), chosen: 'p#ben', listedUrl: 'https://shop.example.com/', activeUrl: OTHER }), 'https://shop.example.com/cart');
  assert.equal(addressToShow({ location: loc('https://shop.example.com/cart', 'p#ben'), chosen: 'p', listedUrl: 'about:blank', activeUrl: OTHER }), 'about:blank', "the page just left doesn't count");
  assert.equal(addressToShow({ location: null, chosen: 'p#ann', listedUrl: 'https://news.example.com/', activeUrl: OTHER }), 'https://news.example.com/');
  assert.equal(addressToShow({ location: null, chosen: 'p#ann', listedUrl: undefined, activeUrl: OTHER }), '', 'no listed url: blank, never activeUrl');
  assert.equal(addressToShow({ location: null, chosen: null, listedUrl: undefined, activeUrl: OTHER }), OTHER, 'no page chosen: as before');
  assert.equal(addressToShow({ location: loc('https://a.example/', null), chosen: null, listedUrl: undefined, activeUrl: OTHER }), 'https://a.example/', 'no pages route: the default stream');
});

test("on open the field shows the chosen tab's listed address, not another bot's last one", async () => {
  const k = threeTabs('r1');
  answer = k.LIST;
  const m = mount({ teamId: 'r1', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    assert.equal(field(m.host).value, '', 'while the list is on its way: empty, not activeUrl');
    await settle();
    assert.equal(currentTab(m.host), 'r1#ben#c1');
    assert.equal(field(m.host).value, BEN.url, "Ben's listed address before any location");
  } finally { m.done(); }
});

test("a pick shows the new tab's address at once; a blank tab clears it; its own location then takes over", async () => {
  const k = threeTabs('r2');
  answer = k.LIST;
  const m = mount({ teamId: 'r2', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    const benSocket = sockets.at(-1);
    benSocket.receive({ type: 'location', url: 'https://shop.example.com/trail/cart', title: 'Cart' });
    await settle();
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/cart', "Ben's own location sets it");
    click(m.host, 'r2#ann#c2');
    assert.equal(field(m.host).value, 'https://news.example.com/', "Ann's listed address in the same frame as the pick");
    await settle();
    const annSocket = sockets.at(-1);
    assert.notEqual(annSocket, benSocket, 'a new stream for Ann');
    assert.equal(field(m.host).value, 'https://news.example.com/', 'still Ann, before her stream speaks');
    // A late message from the stream being closed never writes Ben's address
    benSocket.receive({ type: 'location', url: 'https://shop.example.com/trail/cart', title: 'Cart' });
    await settle();
    assert.equal(field(m.host).value, 'https://news.example.com/', "Ben's late location is ignored");
    annSocket.receive({ type: 'location', url: 'https://news.example.com/story/7', title: 'Story' });
    await settle();
    assert.equal(field(m.host).value, 'https://news.example.com/story/7', "Ann's own location takes over");
    // The team page has no address (about:blank): the field clears, the placeholder shows
    click(m.host, 'r2');
    assert.equal(field(m.host).value, '', 'blank tab: empty field');
    await settle();
    assert.equal(field(m.host).value, '');
    // activeUrl is any bot's last address: it never overwrites the chosen tab's
    m.render({ activeUrl: 'https://another.example.net/' });
    await settle();
    assert.equal(field(m.host).value, '', 'an activeUrl change while a page is chosen is ignored');
  } finally { m.done(); }
});

test('no pages route (404): the field follows activeUrl and the default stream, as before', async () => {
  const saved = g.fetch;
  g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });
  const m = mount({ teamId: 'r3', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    assert.equal(m.host.querySelector('[data-page-tab]'), null, 'no strip');
    assert.equal(field(m.host).value, OTHER, 'activeUrl, as before');
    m.render({ activeUrl: 'https://next.example.net/' });
    await settle();
    assert.equal(field(m.host).value, 'https://next.example.net/', 'an activeUrl change still writes it');
    sockets.at(-1).receive({ type: 'location', url: 'https://default.example.com/', title: 'D' });
    await settle();
    assert.equal(field(m.host).value, 'https://default.example.com/', "the default stream's location writes it");
  } finally { m.done(); g.fetch = g.window.fetch = saved; }
});
