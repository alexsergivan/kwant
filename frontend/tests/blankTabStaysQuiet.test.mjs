// 28u blank-tab-stays-quiet: Mira's follow-up to 28r (address-follows-the-tab/design-check.md:17, :21-23) and
// Rigel's qa.md:30, :36. (1) A blank tab's own `location` (about:blank) flipped the field from the placeholder
// to `about:blank` about 200 ms after the pick. (2) The title (the stage address field's tooltip) used
// `stream.location` with no page check, so on the pick render it still carried the tab just left.
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


const stageInput = (host) => document.querySelector('[data-stage-url] input');

// The title the stage address field carries right after a tap: the render the tap commits, where the old
// stream's \`location\` is still in state (its socket closes in that render's effects)
const tapTitle = (host, page) => { click(host, page); return stageInput().getAttribute('title'); };

test("a blank tab's own about:blank keeps the field empty; a real url for the chosen tab still fills it", async () => {
  const k = threeTabs('u1');
  answer = k.LIST;
  const m = mount({ teamId: 'u1', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'u1#ben#c1');
    click(m.host, 'u1');
    assert.equal(field(m.host).value, '', 'the blank tab: empty at the pick');
    await settle();
    const sharedSocket = sockets.at(-1);
    sharedSocket.receive({ type: 'location', url: 'about:blank', title: '' });
    await settle();
    assert.equal(field(m.host).value, '', "its own about:blank location keeps the placeholder");
    sharedSocket.receive({ type: 'location', url: '  about:blank ', title: '' });
    await settle();
    assert.equal(field(m.host).value, '', 'about:blank with spaces: still empty');
    // The bot opens a real page on that tab: its own location fills the field, as before
    sharedSocket.receive({ type: 'location', url: 'https://maps.example.com/trail', title: 'Map' });
    await settle();
    assert.equal(field(m.host).value, 'https://maps.example.com/trail', 'a real url for the chosen tab fills it');
    // And back to blank (the page closed): quiet again
    sharedSocket.receive({ type: 'location', url: 'about:blank', title: '' });
    await settle();
    assert.equal(field(m.host).value, '');
    // The picture still sees the blank tab: Browser idle, no frame shown
    assert.ok(!pictures(m.host).some((s) => s.includes(SHOP)), 'no picture for a blank tab');
  } finally { m.done(); }
});

test("the title never shows the tab just left after a pick", async () => {
  const k = threeTabs('u2');
  answer = k.LIST;
  const m = mount({ teamId: 'u2', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {}, solveRequest: 1 });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'u2#ben#c1');
    const benSocket = sockets.at(-1);
    benSocket.receive({ type: 'location', url: 'https://shop.example.com/trail/cart', title: 'Cart' });
    await settle();
    assert.equal(stageInput().getAttribute('title'), 'Cart', "Ben's own title");
    assert.equal(tapTitle(m.host, 'u2#ann#c2'), 'News', "Ann's listed title in the same frame as the pick, not Ben's");
    await settle();
    assert.equal(stageInput().getAttribute('title'), 'News', "Ann's listed title until her stream speaks");
    // A late message from Ben's closed stream doesn't name Ann's tab either
    benSocket.receive({ type: 'location', url: 'https://shop.example.com/trail/cart', title: 'Cart' });
    await settle();
    assert.equal(stageInput().getAttribute('title'), 'News', "Ben's late title is ignored");
    const annSocket = sockets.at(-1);
    annSocket.receive({ type: 'location', url: 'https://news.example.com/story/7', title: 'Story' });
    await settle();
    assert.equal(stageInput().getAttribute('title'), 'Story', "Ann's own title takes over");
    // To the blank tab: no title, so the tooltip falls back to its own address, never Ann's title
    assert.equal(tapTitle(m.host, 'u2'), 'about:blank', "Shared has no title: its own address, never Ann's title");
    await settle();
    assert.equal(stageInput().getAttribute('title'), 'about:blank');
  } finally { m.done(); }
});

test('no pages route (404): the default stream still names the title and writes the field, as before', async () => {
  const saved = g.fetch;
  g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });
  const m = mount({ teamId: 'u3', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {}, solveRequest: 2, pageTitle: 'App title' });
  try {
    await settle();
    assert.equal(m.host.querySelector('[data-page-tab]'), null, 'no strip');
    assert.equal(field(m.host).value, OTHER, 'activeUrl, as before');
    assert.equal(stageInput().getAttribute('title'), 'App title', 'the app title before the stream speaks, as before');
    sockets.at(-1).receive({ type: 'location', url: 'https://default.example.com/', title: 'D' });
    await settle();
    assert.equal(field(m.host).value, 'https://default.example.com/', "the default stream's location writes it");
    assert.equal(stageInput().getAttribute('title'), 'D', "and names the title");
  } finally { m.done(); g.fetch = g.window.fetch = saved; }
});
