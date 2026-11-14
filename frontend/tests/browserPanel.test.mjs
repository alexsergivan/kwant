import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { isBlankPage, browserFooterLabel } from './src/computerMode';
  export { VirtualMachinePane, chosenPaneTab, rememberPaneTab } from './src/components/VirtualMachinePane';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const settings = (mode, visible = false) => ({
  mode,
  available: { local: true, vm: true },
  reasons: { local: null, vm: null },
  window: { visible, available: true, reason: null },
  isolates: ['browser', 'kwnt_shell'],
});
const pane = (props) =>
  m.renderToStaticMarkup(m.createElement(m.VirtualMachinePane, { isOpen: true, onClose: () => {}, teamId: 'k1', screenshotBase64: 'AAAA', ...props }));

test('an empty or about:blank tab counts as not opened yet', () => {
  for (const url of ['', '  ', 'about:blank', null, undefined]) assert.equal(m.isBlankPage(url), true, String(url));
  for (const url of ['https://maps.google.com/', 'about:blank#x', 'http://127.0.0.1:8080/']) assert.equal(m.isBlankPage(url), false, url);
});

test('before the bot opens a page the panel says "Browser idle" instead of the blank frame', () => {
  for (const activeUrl of ['', 'about:blank']) {
    const html = pane({ activeUrl });
    assert.match(html, /Browser idle/);
    assert.doesNotMatch(html, /data-browser-watch[^>]*>\s*<img [^>]*alt="Team&#x27;s browser"/);
  }
});

test('once the bot has opened a page the live frame shows', () => {
  const html = pane({ activeUrl: 'https://www.google.com/maps' });
  assert.match(html, /data-browser-watch[^>]*>\s*<img [^>]*alt="Team&#x27;s browser"/);
  assert.match(html, /data:image\/jpeg;base64,AAAA/);
  assert.doesNotMatch(html, /Browser idle/);
});

test('the footer says whose browser it is and what it is doing, in sentence case', () => {
  const ben = { botName: 'Ben', browsing: true };
  assert.equal(m.browserFooterLabel(settings('local'), true, ben), "Live view of Ben's browser");
  assert.equal(m.browserFooterLabel(settings('vm'), true, ben), "Live view of Ben's browser");
  assert.equal(m.browserFooterLabel(settings('local', true), true, ben), "Ben's browser is open in a Chrome window");
  assert.equal(m.browserFooterLabel(settings('vm'), true, { botName: 'Ben', browsing: false }), "Ben's browser is idle");
  assert.equal(m.browserFooterLabel(settings('local'), false, ben), "Ben's browser is off");
  assert.equal(m.browserFooterLabel(settings('vm'), false, ben), "Ben's browser is off");
  // The team page has no bot owner.
  assert.equal(m.browserFooterLabel(settings('vm'), false), "The team's browser is off");
  assert.equal(m.browserFooterLabel(settings('vm'), true, { browsing: true }), "Live view of the team's browser");
  // Settings not loaded yet: no claim about the user's computer.
  assert.equal(m.browserFooterLabel(null, true, { botName: 'Ben' }), "Ben's browser is idle");
});

// On a phone, Back to the list unmounts the chat while the pane stays open; it must come back on
// the tab the person chose, not on Browser.
const pressedTab = (html) => [...html.matchAll(/aria-pressed="true"[^>]*>(?:<svg[\s\S]*?<\/svg>)?<span>([^<]+)<\/span>/g)].map((x) => x[1]);

test('the pane opens on Browser for a bot it has not shown before', () => {
  assert.equal(m.chosenPaneTab('fresh-bot'), 'browser');
  assert.equal(m.chosenPaneTab(undefined), 'browser');
  assert.deepEqual(pressedTab(pane({ teamId: 'fresh-bot' })), ['Browser']);
});

test('a remounted pane comes back on the tab chosen for that bot', () => {
  m.rememberPaneTab('bot-a', 'vault');
  assert.deepEqual(pressedTab(pane({ teamId: 'bot-a' })), ['Logins']);
  m.rememberPaneTab('bot-a', 'terminal');
  assert.deepEqual(pressedTab(pane({ teamId: 'bot-a' })), ['Terminal']);
  // Another bot keeps its own tab
  assert.deepEqual(pressedTab(pane({ teamId: 'bot-b' })), ['Browser']);
});

// ---------- 0.9.8 browser panel (design browser-panel/design.md C) ----------
const trail = [
  { at: '2026-11-13T10:46:00Z', url: 'https://roastco.example.com/cart', host: 'roastco.example.com', stepTitle: 'Your cart · RoastCo' },
  { at: '2026-11-13T10:45:00Z', url: 'https://roastco.example.com/shop/coffee/1kg', host: 'roastco.example.com', stepTitle: null },
];

test('the header says whose browser it is and whether it browses now; no Take over, Point, Key or Refresh up there', () => {
  const html = pane({ teamName: 'Spring launch', browsingBot: 'Ben', running: true, activeUrl: 'https://roastco.example.com/cart' });
  assert.match(html, /data-pane-title="">Ben&#x27;s browser</);
  assert.match(html, /data-pane-status="browsing"[\s\S]*?Browsing now · @Spring launch/);
  const header = html.slice(0, html.indexOf('aria-pressed'));
  assert.doesNotMatch(header, /data-takeover-button|data-point-button|Saved logins|Refresh/);
  assert.match(html, /data-pane-pill="live"[^>]*>[\s\S]*?Live</);
});

test('after the turn: Finished with the time, a muted frame and no Live pill; with no run, Idle and no pill', () => {
  const html = pane({ browsingBot: 'Ben', running: false, finishedAt: '2026-11-13T22:46:00Z', activeUrl: 'https://roastco.example.com/' });
  assert.match(html, /data-pane-status="finished"[\s\S]*?Finished \d{1,2}:46/);
  assert.match(html, /data-pane-pill="finished"/);
  assert.match(html, /data-frame-finished="true"[^>]*saturate-\[\.6\] brightness-\[\.82\]/);
  const idle = pane({ activeUrl: 'https://roastco.example.com/' });
  assert.match(idle, /data-pane-status="idle"[\s\S]*?Idle · @Team/);
  assert.match(idle, /data-pane-title="">The team&#x27;s browser</);
  assert.doesNotMatch(idle, /data-pane-pill/);
});

test('the trail lists the reply\'s pages, newest first, each opening in a new tab; the current one says Now while live', () => {
  const html = pane({ browsingBot: 'Ben', running: true, trail, activeUrl: 'https://roastco.example.com/cart' });
  assert.match(html, /Pages this reply · 2/);
  const rows = [...html.matchAll(/<a href="([^"]+)" target="_blank" rel="noopener noreferrer"/g)].map((x) => x[1]);
  assert.deepEqual(rows, ['https://roastco.example.com/cart', 'https://roastco.example.com/shop/coffee/1kg']);
  assert.match(html, /Your cart · RoastCo/);
  assert.equal((html.match(/> Now</g) ?? []).length, 1);
  assert.doesNotMatch(pane({ browsingBot: 'Ben', running: false, trail, activeUrl: 'https://roastco.example.com/' }), /> Now</);
  assert.doesNotMatch(pane({ activeUrl: 'https://roastco.example.com/' }), /Pages this reply/);
});

test('the frame has an Enlarge button; the footer offers Refresh and "Restart browser"', () => {
  const html = pane({ activeUrl: 'https://roastco.example.com/', onRefresh: () => {}, onRestartVm: () => {} });
  assert.match(html, /aria-label="Enlarge"/);
  assert.match(html, />Refresh</);
  assert.match(html, /Restart browser/);
  assert.doesNotMatch(html, /Restart Browser/);
});
