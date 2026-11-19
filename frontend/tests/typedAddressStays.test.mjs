// 28x typed-address-stays: Mira's follow-up in blank-tab-stays-quiet/design-check.md:18-26 and Rigel's
// blank-tab-stays-quiet/qa.md section 3 item 3. The server re-reads the page address every 2 s and sends a
// `location` each time, even when nothing changed; each one rewrote the address field, so text the person was
// typing was wiped. Plus Rigel's 28r P1 (address-follows-the-tab/qa.md:48): a revisited tab showed its old
// stream-written address until its own stream spoke again.
// 28y title-change-keeps-typing (Mira, typed-address-stays/design-check.md:19; Rigel's Q1): a page whose title
// changes on its own no longer wipes what the person types; only a new url or page writes the field.
// 29f typed-text-own-tab (Rigel's Q4 and R6; his Q2): typed text belongs to its tab, so another tab's stream never
// wipes it; Enter hands the field back to the page and shows the full address it was sent to.
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
const setVal = (el, v) => { const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set; pane.flushSync(() => { set.call(el, v); el.dispatchEvent(new window.Event('input', { bubbles: true })); }); };
const tick = async (socket, url, title) => { socket.receive({ type: 'location', url, title }); await settle(); };

test('typing on a blank tab survives two identical location ticks', async () => {
  const k = threeTabs('x1');
  answer = k.LIST;
  const m = mount({ teamId: 'x1', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    click(m.host, 'x1');
    await settle();
    const shared = sockets.at(-1);
    await tick(shared, 'about:blank', '');
    assert.equal(field(m.host).value, '');
    setVal(field(m.host), 'booktun.ing');
    assert.equal(field(m.host).value, 'booktun.ing', 'typing shows');
    await tick(shared, 'about:blank', '');
    assert.equal(field(m.host).value, 'booktun.ing', 'first tick: typing kept');
    await tick(shared, 'about:blank', '');
    assert.equal(field(m.host).value, 'booktun.ing', 'second tick: typing kept');
    // A real change of address still replaces the field, as a browser's own address bar does
    await tick(shared, 'https://maps.example.com/trail', 'Map');
    assert.equal(field(m.host).value, 'https://maps.example.com/trail', 'a new url replaces typed text');
  } finally { m.done(); }
});

test('typing on a tab with a real url survives identical ticks and title-only changes; a new url still lands', async () => {
  const k = threeTabs('x2');
  answer = k.LIST;
  const m = mount({ teamId: 'x2', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'x2#ben#c1');
    const ben = sockets.at(-1);
    await tick(ben, 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/cart');
    setVal(field(m.host), 'booktun.ing');
    await tick(ben, 'https://shop.example.com/trail/cart', 'Cart');
    await tick(ben, 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'booktun.ing', 'two identical ticks: typing kept');
    await tick(ben, 'https://shop.example.com/trail/checkout', 'Checkout');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/checkout', 'a new url replaces it');
    // 28y: same url, new title (a page that renames itself, "(3) Inbox"): typed text stays, the title line follows
    setVal(field(m.host), 'booktun.ing');
    await tick(ben, 'https://shop.example.com/trail/checkout', 'Checkout - step 2');
    assert.equal(field(m.host).value, 'booktun.ing', 'a title-only change keeps typed text');
    assert.ok(m.host.textContent.includes('Checkout - step 2'), 'the title line shows the new title');
    await tick(ben, 'https://shop.example.com/trail/checkout', '(3) Checkout - step 2');
    assert.equal(field(m.host).value, 'booktun.ing', 'a second title change keeps it too');
    assert.ok(m.host.textContent.includes('(3) Checkout - step 2'));
    await tick(ben, 'https://shop.example.com/trail/done', 'Done');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/done', 'a new url still replaces it');
  } finally { m.done(); }
});

test('28y: after a stream reset the first location with the same url still writes the field', async () => {
  const k = threeTabs('x5');
  answer = k.LIST;
  const m = mount({ teamId: 'x5', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'x5#ben#c1');
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/cart');
    setVal(field(m.host), 'booktun.ing');
    // Close and reopen the pane: the stream resets (location null) and a new socket opens on the same page
    const before = sockets.length;
    m.render({ isOpen: false });
    await settle();
    m.render({ isOpen: true });
    await settle();
    assert.ok(sockets.length > before, 'a new socket after the reset');
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/cart', 'null -> same url writes the address again');
    // and the 2 s tick after it is still quiet
    setVal(field(m.host), 'booktun.ing');
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'booktun.ing');
  } finally { m.done(); }
});

test("Rigel's P1: Ben -> Ann -> Ben after Ben navigated shows his newer listed url in the tap render", async () => {
  const k = threeTabs('x3');
  answer = k.LIST;
  const m = mount({ teamId: 'x3', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    await tick(sockets.at(-1), 'https://shop.example.com/OLD', 'Old');
    assert.equal(field(m.host).value, 'https://shop.example.com/OLD');
    click(m.host, 'x3#ann#c2');
    assert.equal(field(m.host).value, 'https://news.example.com/', "Ann's listed url at the pick");
    await settle();
    // Ben navigates while the person looks at Ann; the list says so. Ann's stream has not spoken yet.
    answer = { ...k.LIST, pages: [k.SHARED, { ...k.BEN, url: 'https://shop.example.com/NEW' }, k.ANN] };
    pane.notifyBrowserPages('x3');
    await settle(400); // the list reloads 250 ms after a notice
    click(m.host, 'x3#ben#c1');
    assert.equal(field(m.host).value, 'https://shop.example.com/NEW', "Ben's newer listed url in the tap render, not OLD");
    await settle();
    await tick(sockets.at(-1), 'https://shop.example.com/NEW', 'New');
    assert.equal(field(m.host).value, 'https://shop.example.com/NEW');
  } finally { m.done(); }
});

test('text the person typed is kept when they leave the tab and come back', async () => {
  const k = threeTabs('x4');
  answer = k.LIST;
  const m = mount({ teamId: 'x4', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    setVal(field(m.host), 'booktun.ing');
    click(m.host, 'x4#ann#c2');
    assert.equal(field(m.host).value, 'https://news.example.com/', "Ann's own address under her tab");
    await settle();
    click(m.host, 'x4#ben#c1');
    assert.equal(field(m.host).value, 'booktun.ing', 'the typed text is still there on return');
  } finally { m.done(); }
});

test("29f Q4: typed on Ben, Ann's own stream reports, back on Ben: the typed text is still there", async () => {
  const k = threeTabs('x6');
  answer = k.LIST;
  const m = mount({ teamId: 'x6', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'x6#ben#c1');
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    setVal(field(m.host), 'booktun.ing');
    click(m.host, 'x6#ann#c2');
    await settle();
    await tick(sockets.at(-1), 'https://news.example.com/today', 'News today');
    assert.equal(field(m.host).value, 'https://news.example.com/today', "Ann's own address under her tab");
    click(m.host, 'x6#ben#c1');
    assert.equal(field(m.host).value, 'booktun.ing', 'the tap render shows what was typed on Ben');
    await settle();
    // Ben's new socket reports where Ben still is: the same url is no change, the typed text stays
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'booktun.ing', "Ben's own same-url report keeps it");
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    assert.equal(field(m.host).value, 'booktun.ing', 'and so does the 2 s tick');
    // A new url from Ben's own stream still replaces it
    await tick(sockets.at(-1), 'https://shop.example.com/trail/checkout', 'Checkout');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/checkout');
  } finally { m.done(); }
});

test("29f: Ben's page moved while the person was on Ann: back on Ben, his new url replaces the typed text", async () => {
  const k = threeTabs('x7');
  answer = k.LIST;
  const m = mount({ teamId: 'x7', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    await tick(sockets.at(-1), 'https://shop.example.com/trail/cart', 'Cart');
    setVal(field(m.host), 'booktun.ing');
    click(m.host, 'x7#ann#c2');
    await settle();
    await tick(sockets.at(-1), 'https://news.example.com/', 'News');
    click(m.host, 'x7#ben#c1');
    await settle();
    await tick(sockets.at(-1), 'https://shop.example.com/trail/paid', 'Paid');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/paid', 'a new url from his own stream wins');
  } finally { m.done(); }
});

test("29f: a closed page forgets what was typed on it", async () => {
  const k = threeTabs('x8');
  answer = k.LIST;
  const m = mount({ teamId: 'x8', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: () => {} });
  try {
    await settle();
    click(m.host, 'x8#ann#c2');
    await settle();
    setVal(field(m.host), 'booktun.ing');
    click(m.host, 'x8#ben#c1');
    await settle();
    // Ann's page closes, then a page with the same key opens again: it starts from its own address
    answer = { ...k.LIST, pages: [k.SHARED, k.BEN] };
    pane.notifyBrowserPages('x8');
    await settle(400);
    answer = k.LIST;
    pane.notifyBrowserPages('x8');
    await settle(400);
    click(m.host, 'x8#ann#c2');
    assert.equal(field(m.host).value, 'https://news.example.com/', 'no typed text left from the closed page');
  } finally { m.done(); }
});

test("29f Q2: Enter on the page's own address without https shows the full address, and the tick keeps it", async () => {
  const k = threeTabs('x9');
  answer = k.LIST;
  const went = [];
  const m = mount({ teamId: 'x9', browsingBot: 'Ben', botNames: NAMES, activeUrl: OTHER, onNavigate: (url, page) => went.push([url, page]) });
  try {
    await settle();
    const ben = sockets.at(-1);
    await tick(ben, 'https://shop.example.com/trail/ridgeline-2p', 'Ridgeline 2P Tent');
    setVal(field(m.host), 'shop.example.com/trail/ridgeline-2p');
    pane.flushSync(() => field(m.host).dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    assert.deepEqual(went, [['https://shop.example.com/trail/ridgeline-2p', 'x9#ben#c1']]);
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/ridgeline-2p', 'the full address right after Enter');
    // The page was already there: the 2 s tick brings the same url, and the field stays in step
    await tick(ben, 'https://shop.example.com/trail/ridgeline-2p', 'Ridgeline 2P Tent');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/ridgeline-2p');
    // Enter to a new place: the page's own report of where it landed shows
    setVal(field(m.host), 'shop.example.com/trail');
    pane.flushSync(() => field(m.host).dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
    assert.equal(field(m.host).value, 'https://shop.example.com/trail');
    await tick(ben, 'https://shop.example.com/trail/', 'Trail');
    assert.equal(field(m.host).value, 'https://shop.example.com/trail/');
  } finally { m.done(); }
});
