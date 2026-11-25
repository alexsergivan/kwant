// 31d (autonomy/needs-you-polish/brief.md, Mira's design check of #909 fixes 2–4): leaving the chat doesn't move the
// sidebar (the inactive channel row keeps a transparent border, like the team and bot rows), the one lit chat row says
// aria-current="page", and the Inbox page's "New" count is amber only when something really needs you.
// Every test here fails at #909's head 13c30698.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { Sidebar } from './src/components/Sidebar';
  export { InboxPage } from './src/components/InboxPage';
`);
const { React } = m;
const noop = () => {};
const space = { id: 'spc_default', name: 'Trailhead', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('lin', 'Lin'), bot('pix', 'Pixel'), bot('iri', 'Iris')];
const team = { id: 't1', name: 'Builders', space_id: 'spc_default', lead_kwant_id: 'lin', member_kwants: ['lin', 'pix'] };
const channels = [{ id: 'c1', name: 'general', space_id: 'spc_default' }, { id: 'c2', name: 'random', space_id: 'spc_default' }];

const doc = (props) => new DOMParser().parseFromString(`<body>${m.renderToStaticMarkup(React.createElement(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels, teams: [], selectedId: null, isChannel: false,
  onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop,
  busyKwants: {}, botStatus: {}, inboxCount: 1, ...props,
}))}</body>`, 'text/html');
const channelRow = (d, name = 'general') => [...d.querySelectorAll('button')].find((b) => b.textContent.trim() === name);
const cls = (el) => el.getAttribute('class').split(/\s+/);
const borderW = (el) => (cls(el).includes('border') ? 1 : 0);
const here = (d) => [...d.querySelectorAll('[aria-current="page"]')];

test('(2) the channel row keeps a 1 px border lit or not, so leaving the chat moves nothing below it', () => {
  const onChannel = { selectedId: 'c1', isChannel: true };
  const lit = channelRow(doc({ ...onChannel, onChat: true }));
  const off = channelRow(doc({ ...onChannel, onChat: false, inboxActive: true }));
  assert.equal(borderW(lit), 1);
  assert.equal(borderW(off), 1, 'inactive #general has a border too');
  assert.ok(cls(off).includes('border-transparent'), 'and it is transparent');
  assert.ok(cls(channelRow(doc({ ...onChannel, onChat: true }), 'random')).includes('border-transparent'), 'every unlit channel row');
  // Team and bot rows already did this; keep them that way
  const onTeam = { teams: [team], channels: [], selectedId: 't1', selectedTeamId: 't1', isChannel: true };
  const teamRow = (d) => d.querySelector('[data-team-row="t1"]');
  assert.equal(borderW(teamRow(doc({ ...onTeam, onChat: true }))), 1);
  assert.equal(borderW(teamRow(doc({ ...onTeam, onChat: false }))), 1);
  const botRow = (d) => d.querySelector('[data-bot-row="iri"]');
  assert.equal(borderW(botRow(doc({ selectedId: 'iri', onChat: true }))), 1);
  assert.equal(borderW(botRow(doc({ selectedId: 'iri', onChat: false }))), 1);
});

test('(3) exactly one aria-current="page": the lit channel, team or bot row in a chat, the Inbox row on Inbox', () => {
  const ch = doc({ selectedId: 'c1', isChannel: true, onChat: true });
  assert.deepEqual(here(ch), [channelRow(ch)]);
  assert.equal(channelRow(ch, 'random').getAttribute('aria-current'), null);

  // aria-current sits on the row's one real button, not the data-team-row/data-bot-row div around it
  // (Pixel's FIX on #1186: that div isn't focusable or clickable, so it isn't what's "current").
  const tm = doc({ teams: [team], channels: [], selectedId: 't1', selectedTeamId: 't1', isChannel: true, onChat: true });
  assert.deepEqual(here(tm), [tm.querySelector('[data-team-row="t1"] > button')]);

  const bt = doc({ selectedId: 'iri', onChat: true });
  assert.deepEqual(here(bt), [bt.querySelector('[data-bot-row="iri"] > button')]);

  // A bot pinned under Needs you shows twice; only its own row says "here"
  const ask = { kwant: kwants[1], rank: 1, line: 'Wants your OK', item: { kind: 'approval', id: 'a-pix', kwant_id: 'pix', kwant_name: 'pix',
    title: 'Deploy', summary: '', status: 'waiting', timestamp: new Date().toISOString(), link: {} } };
  const pinned = doc({ selectedId: 'pix', onChat: true, attention: [ask] });
  assert.ok(pinned.querySelector('[data-pin-row="pix"]'), 'the pinned copy renders');
  assert.deepEqual(here(pinned), [pinned.querySelector('[data-bot-row="pix"] > button')]);

  const inbox = doc({ selectedId: 'c1', isChannel: true, onChat: false, inboxActive: true });
  const h = here(inbox);
  assert.equal(h.length, 1);
  assert.match(h[0].textContent, /Inbox/);
});

const inboxPage = (props) => new DOMParser().parseFromString(`<body>${m.renderToStaticMarkup(React.createElement(m.InboxPage, {
  tab: 'new', onTab: noop, count: 1, newPage: null, loosePage: null, ...props,
}))}</body>`, 'text/html');
const newCount = (d) => d.querySelector('[data-inbox-tab="new"] span.rounded-full');

test('(4) Inbox page "New" count: neutral for a finished routine run, amber only for a real ask (same rule as the pill)', () => {
  for (const props of [{ ask: false }, {}]) {
    const c = newCount(inboxPage(props));
    assert.equal(c.textContent, '1 new');
    assert.doesNotMatch(c.getAttribute('class'), /amber/);
    assert.match(c.getAttribute('class'), /bg-white\/10 text-zinc-200/);
  }
  assert.match(newCount(inboxPage({ ask: true })).getAttribute('class'), /bg-amber-500 text-zinc-950/);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const at = (re) => app.search(re);
  assert.ok(at(/const inboxAsk = realAskCount\(attention\) > 0;/) > 0);
  assert.ok(at(/const inboxAsk = /) < at(/<InboxPage /), 'inboxAsk is declared before InboxPage reads it');
  assert.match(app, /<InboxPage [^\n]*ask=\{inboxAsk\}/);
});
