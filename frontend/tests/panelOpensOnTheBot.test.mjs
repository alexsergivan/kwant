// 28o panel-opens-on-the-bot, the three follow-ups from 28l's checks:
// 1. Mira's call 1: "Ben's browser · Browsing now" opened on Shared, "Browser idle", with Ben's page in the
//    strip (branch-1280-4). With nothing chosen, the pane now rests first on the newest listed page of the
//    bot whose panel it is, decided once when the first list lands; else the server's default.
// 2. Mira's call 2: the strip row is kept, empty, while the pane waits for the list, so the dark box
//    doesn't move down when the tabs arrive.
// 3. Rigel's note 6: only a 404 marks a team "no pages route"; a 5xx or a thrown fetch doesn't.
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
  export { notifyBrowserPages, pageOpened, restingPage, ownNewestPage, fetchBrowserPages } from './src/browserPages';
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
const botName = (id) => NAMES[id];

test('ownNewestPage: the newest listed page the bot owns, else null', () => {
  const { ownNewestPage } = pane;
  assert.equal(typeof ownNewestPage, 'function', 'browserPages.ts exports ownNewestPage');
  assert.equal(ownNewestPage(MOMENT_LATER, 'Ben', botName), BEN_PAGE);
  assert.equal(ownNewestPage(MOMENT_LATER, 'Ann', botName), null, 'Ann has no page');
  assert.equal(ownNewestPage(MOMENT_LATER, null, botName), null, 'no bot (a team panel)');
  assert.equal(ownNewestPage(null, 'Ben', botName), null, 'no list');
  // Two of Ben's pages: the list is newest first
  const newer = { ...BEN, page: 'o1#ben#c9', chat_id: 'c9' };
  assert.equal(ownNewestPage({ pages: [SHARED, newer, BEN] }, 'Ben', botName), 'o1#ben#c9');
  // The team page is never a bot's, even when its name is "Shared"
  assert.equal(ownNewestPage({ pages: [SHARED] }, 'Shared', botName), null);
});

test('restingPage: the first pick when nothing is held and it is listed; held still wins; else the default', () => {
  const { restingPage } = pane;
  assert.equal(restingPage(null, MOMENT_LATER, BEN_PAGE), BEN_PAGE, "first: Ben's page over the default");
  assert.equal(restingPage(null, MOMENT_LATER, null), TEAM, 'no page of his: the default');
  assert.equal(restingPage(null, MOMENT_LATER, 'o1#gone#c0'), TEAM, 'not listed: the default');
  assert.equal(restingPage(TEAM, MOMENT_LATER, BEN_PAGE), TEAM, 'a held page wins');
  assert.equal(restingPage(null, null, BEN_PAGE), null, 'no list: the old fallback');
});

test("Ben's panel opens on Ben's tab though the default is the team page, and stays (branch-1280-4)", async () => {
  const k = teamPages('o2');
  answer = k.MOMENT_LATER; // live: the team page, Shared listed first
  const before = sockets.length;
  const m = mount({ running: true, teamId: 'o2', browsingBot: 'Ben', botNames: NAMES });
  try {
    await settle();
    assert.equal(sockets.length, before + 1, 'one stream');
    assert.match(sockets.at(-1).url, /page=o2%23ben%23c1/, "it names Ben's page");
    assert.equal(currentTab(m.host), 'o2#ben#c1', "Ben's tab is current");
    assert.equal(/Browser idle/.test(m.host.textContent), false, 'not the idle team page');
    // Decided once: the default flipping, the list reordering and the header's bot changing don't move it
    answer = k.AFTER_RUN;
    pane.notifyBrowserPages('o2');
    await settle(300);
    answer = { ...k.MOMENT_LATER, pages: [ANN('o2'), k.SHARED, k.BEN] };
    m.render({ browsingBot: 'Ann' });
    await settle();
    assert.equal(currentTab(m.host), 'o2#ben#c1', "still Ben's tab");
    assert.equal(sockets.length, before + 1, 'no reconnect');
    // The person's pick still wins
    m.host.querySelector('[data-page-tab="o2"]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    await settle();
    assert.equal(currentTab(m.host), 'o2');
  } finally {
    m.done();
  }
});

test("a bot with no page in the list: the server's default, as before", async () => {
  const k = teamPages('o3');
  answer = k.MOMENT_LATER;
  const m = mount({ running: false, teamId: 'o3', browsingBot: 'Ann', botNames: NAMES });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'o3', "Ann has no page: the default, Shared");
    assert.match(sockets.at(-1).url, /page=o3(&|$)/);
  } finally {
    m.done();
  }
});

test('an open step still comes before the bot\'s own page', async () => {
  const k = teamPages('o4');
  answer = { ...k.MOMENT_LATER, pages: [k.SHARED, k.BEN, ANN('o4')] };
  const step = { id: 's1', status: 'open', kind: 'sign_in', page_key: 'o4#ann#c2', sandbox_target: 'o4' };
  const m = mount({ running: false, teamId: 'o4', browsingBot: 'Ben', botNames: NAMES, humanStep: step });
  try {
    await settle();
    assert.equal(currentTab(m.host), 'o4#ann#c2', "the step's page");
  } finally {
    m.done();
  }
});

test('the strip row is kept, empty, while the pane waits for the list; then the tabs take it', async () => {
  const k = teamPages('o5');
  answer = k.MOMENT_LATER;
  let open;
  gate = new Promise((r) => { open = r; });
  const m = mount({ running: false, teamId: 'o5', browsingBot: 'Ben', botNames: NAMES });
  try {
    await settle();
    const kept = m.host.querySelector('[data-page-strip-reserved]');
    assert.ok(kept, 'the row is kept during the hold');
    assert.equal(kept.getAttribute('aria-hidden'), 'true', 'nothing to read');
    assert.match(kept.className, /\bh-7\b/, 'the desktop tab height (28 px)');
    assert.equal(kept.textContent, '', 'empty');
    assert.ok(m.host.querySelector('[data-frame-holding]'), 'the dark hold');
    open(); gate = null;
    await settle();
    assert.equal(m.host.querySelector('[data-page-strip-reserved]'), null, 'gone once the list is known');
    assert.ok(m.host.querySelector('[data-page-strip]'), 'the tabs are in its place');
  } finally {
    m.done();
    gate = null;
  }
});

test('fetchBrowserPages: a 404 is "no route" (null); a 500 or a network error throws', async () => {
  const fetchWas = g.fetch;
  try {
    g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}) });
    assert.equal(await pane.fetchBrowserPages('o6'), null);
    g.fetch = g.window.fetch = async () => ({ ok: false, status: 500, json: async () => ({}) });
    await assert.rejects(pane.fetchBrowserPages('o6'));
    g.fetch = g.window.fetch = async () => ({ ok: false, status: 401, json: async () => ({}) });
    await assert.rejects(pane.fetchBrowserPages('o6'));
    g.fetch = g.window.fetch = async () => { throw new TypeError('Failed to fetch'); };
    await assert.rejects(pane.fetchBrowserPages('o6'));
    g.fetch = g.window.fetch = async () => ({ ok: true, status: 200, json: async () => MOMENT_LATER });
    assert.equal((await pane.fetchBrowserPages('o1'))?.live, TEAM);
  } finally {
    g.fetch = g.window.fetch = fetchWas;
  }
});

// Opens the pane against a failing pages route, closes it, then reopens it with the list on its way:
// whether that reopen waits for the list (the team is not marked "no pages route") or streams at once.
async function reopenAfter(team, failing) {
  const fetchWas = g.fetch;
  g.fetch = g.window.fetch = async (url) => (String(url).includes('/pages') ? failing() : fetchWas(url));
  const m = mount({ running: false, teamId: team, browsingBot: 'Ben', botNames: NAMES });
  try {
    await settle(pane.PAGES_WAIT_MS + 100);
    const first = sockets.at(-1)?.url ?? '';
    m.render({ isOpen: false });
    await settle();
    g.fetch = g.window.fetch = fetchWas;
    answer = teamPages(team).MOMENT_LATER;
    let open;
    gate = new Promise((r) => { open = r; });
    const before = sockets.length;
    m.render({ isOpen: true });
    await settle();
    const waited = sockets.length === before;
    open(); gate = null;
    await settle();
    return { first, waited, last: sockets.at(-1).url, tab: currentTab(m.host) };
  } finally {
    m.done();
    gate = null;
    g.fetch = g.window.fetch = fetchWas;
  }
}

test('a 500 does not mark the team "no pages route": the wait is still bounded, and the next open waits for the list', async () => {
  const r = await reopenAfter('o7', async () => ({ ok: false, status: 500, json: async () => ({}) }));
  assert.match(r.first, /\/o7\b/, 'after PAGES_WAIT_MS the pane streams anyway');
  assert.doesNotMatch(r.first, /page=/, "with no list, the server's default");
  assert.equal(r.waited, true, 'the reopen waits for the list (no blink)');
  assert.match(r.last, /page=o7%23ben%23c1/, "then Ben's page");
});

test('a thrown fetch does not mark the team either', async () => {
  const r = await reopenAfter('o8', async () => { throw new TypeError('Failed to fetch'); });
  assert.equal(r.waited, true, 'the reopen waits for the list');
  assert.equal(r.tab, 'o8#ben#c1');
});

test('a 404 does mark it: the next open does not wait', async () => {
  const r = await reopenAfter('o9', async () => ({ ok: false, status: 404, json: async () => ({}) }));
  assert.equal(r.waited, false, 'an older server: the reopen streams at once');
});
