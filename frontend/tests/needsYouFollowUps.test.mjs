// 31c (autonomy/needs-you-follow-ups/brief.md, Mira's design check of #907 "Not blocking" 1 and 3): the Inbox count
// pill and the phone's Inbox badge are amber only when something really needs you (rank 1–2); otherwise a calm
// neutral count. And while Inbox (or any page but the chat) is open, no channel, team or bot row also says
// "you are here". Every test here fails at #907's head 17c83852.
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
  export { PhoneTabBar } from './src/components/PhoneTabBar';
`);
const { React } = m;
const noop = () => {};
const space = { id: 'spc_default', name: 'Trailhead', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('lin', 'Lin'), bot('pix', 'Pixel'), bot('iri', 'Iris')];
const team = { id: 't1', name: 'Builders', space_id: 'spc_default', lead_kwant_id: 'lin', member_kwants: ['lin', 'pix'] };
const item = (kind, kwant_id) => ({ kind, id: `${kind}-${kwant_id}`, kwant_id, kwant_name: kwant_id, title: 'Morning note', summary: '',
  status: 'completed', timestamp: new Date().toISOString(), link: {} });
const review = { kwant: kwants[0], rank: 3, line: 'To review: Routine: Morning note', item: item('delegation', 'lin') };
const finished = { kwant: kwants[2], rank: 4, line: 'Finished: legend', item: item('delegation', 'iri') };
const ask = { kwant: kwants[1], rank: 1, line: 'Wants your OK', item: { ...item('approval', 'pix'), status: 'waiting' } };
const failed = { ...review, rank: 2, line: 'Failed: crawl' };

const doc = (props) => new DOMParser().parseFromString(`<body>${m.renderToStaticMarkup(React.createElement(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [{ id: 'c1', name: 'general', space_id: 'spc_default' }], teams: [], selectedId: null, isChannel: false,
  onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop,
  busyKwants: {}, botStatus: {}, inboxCount: 1, ...props,
}))}</body>`, 'text/html');
const inboxRow = (d) => [...d.querySelectorAll('button')].find((b) => b.querySelector('svg.lucide-inbox') && /Inbox/.test(b.textContent));
const pill = (d) => inboxRow(d).querySelector('span.rounded-full');
const ACTIVE = /bg-accent-soft|bg-zinc-800 text-zinc-100/;

test('(a) Inbox pill: neutral with only rank 3–4 rows (one finished routine run), amber once a real ask waits', () => {
  for (const attention of [[review], [review, finished], undefined]) {
    const p = pill(doc({ attention }));
    assert.ok(p, 'the count still shows');
    assert.equal(p.textContent, '1 new');
    assert.doesNotMatch(p.getAttribute('class'), /amber/, 'no amber when nothing needs you');
    assert.match(p.getAttribute('class'), /bg-white\/10 text-zinc-200/);
    assert.equal(p.getAttribute('data-inbox-pill'), 'calm');
  }
  for (const attention of [[ask, review], [failed]]) {
    const p = pill(doc({ attention }));
    assert.match(p.getAttribute('class'), /bg-amber-500 text-zinc-950/);
    assert.equal(p.getAttribute('data-inbox-pill'), 'ask');
  }
});

const tabs = (tone) => new DOMParser().parseFromString(`<body>${m.renderToStaticMarkup(React.createElement(m.PhoneTabBar, {
  label: 'Kwant', active: 'chats', onSelect: noop,
  tabs: [{ id: 'chats', label: 'Chats', icon: null }, { id: 'inbox', label: 'Inbox', icon: null, badge: 1, badgeTone: tone }],
}))}</body>`, 'text/html').querySelector('[data-tab="inbox"] [data-tab-badge]');

test('(a) phone: the Inbox tab badge has a neutral tone (not amber) and App picks it by the same rank 1–2 rule', () => {
  const calm = tabs('neutral');
  assert.equal(calm.getAttribute('data-tone'), 'neutral');
  assert.doesNotMatch(calm.getAttribute('class'), /fcd34d/);
  assert.equal(tabs('amber').getAttribute('data-tone'), 'amber');
  assert.match(tabs('amber').getAttribute('class'), /fcd34d/);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /id: 'inbox', label: 'Inbox'[^\n]*badgeTone: realAskCount\(attention\) > 0 \? 'amber' : 'neutral'/);
  assert.match(app, /onChat=\{page === 'chat'\}/);
});

test('(b) one "you are here": #general, a team or a bot row is lit only while the chat is the page', () => {
  const channelRow = (d) => [...d.querySelectorAll('button')].find((b) => b.textContent.trim() === 'general');
  const onChannel = { selectedId: 'c1', isChannel: true };
  assert.match(channelRow(doc({ ...onChannel, onChat: true })).getAttribute('class'), ACTIVE);
  const inbox = doc({ ...onChannel, onChat: false, inboxActive: true });
  assert.doesNotMatch(channelRow(inbox).getAttribute('class'), ACTIVE, 'Inbox open: #general is not lit');
  assert.equal(inboxRow(inbox).getAttribute('aria-current'), 'page');
  assert.equal(inbox.querySelectorAll('[aria-current="page"]').length, 1, 'one row says "here"');

  const teamRow = (d) => d.querySelector('[data-team-row="t1"]');
  const onTeam = { teams: [team], channels: [], selectedId: 't1', selectedTeamId: 't1', isChannel: true };
  assert.match(teamRow(doc({ ...onTeam, onChat: true })).getAttribute('class'), /bg-accent-soft/);
  assert.doesNotMatch(teamRow(doc({ ...onTeam, onChat: false, missionsActive: true })).getAttribute('class'), /bg-accent-soft/);

  const botRow = (d) => d.querySelector('[data-bot-row="iri"]');
  const onBot = { selectedId: 'iri', isChannel: false };
  assert.match(botRow(doc({ ...onBot, onChat: true })).getAttribute('class'), /bg-accent-soft/);
  assert.doesNotMatch(botRow(doc({ ...onBot, onChat: false, inboxActive: true })).getAttribute('class'), /bg-accent-soft/);
});
