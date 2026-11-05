// Phone Chats tab and Bots tab (spec 0.6.13 B1, B4). The Chats tab lists everything you talk to:
// Needs you (amber, only when something waits), Working, Channels, Team chats, Direct chats. No row
// on a phone stays lit as selected. On the Bots tab every row has one ⋯ (44×44) and no pencil.
import assert from 'node:assert/strict';
import test from 'node:test';
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
  export { needsYouRows, approvalLine } from './src/chatsTab';
`);
const { React } = m;
const noop = () => {};
const space = { id: 'spc_default', name: 'Trailhead', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name, title = 'Role') => ({ id, name, title, persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('tl', 'TechLead'), bot('qa', 'QA'), bot('re', 'Researcher'), bot('ar', 'Architect'), bot('de', 'Designer'), bot('lo', 'Loner')];
const team = { id: 't1', name: 'Product', icon: '🚢', lead_kwant_id: 'tl', member_kwants: ['tl', 'qa', 're', 'ar'] };
const channels = [{ id: 'c1', name: 'general', space_id: 'spc_default' }, { id: 'c2', name: 'design-review', space_id: 'spc_default' }];
const item = (kind, kwant_id, over = {}) => ({ kind, id: `${kind}-${kwant_id}`, kwant_id, kwant_name: kwant_id, title: '', summary: '', status: 'waiting',
  timestamp: '2026-11-05T00:00:00Z', link: { kind: 'direct', kwant_id }, ...over });
const waiting = [item('approval', 'tl', { tool_name: 'mcp__github__create_pull_request' }), item('question', 'qa')];
const steps = { re: 1 };
const rows = () => m.needsYouRows(waiting, steps, new Set(kwants.map((k) => k.id)));
const render = (over = {}) => m.renderToStaticMarkup(React.createElement(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels, teams: [team], selectedId: 'c1', isChannel: true, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
  onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onEditKwant: noop, onUpdateTeamMembers: noop,
  busyKwants: { ar: 'Searching the web', re: 'Needs you in the browser' }, needsYou: steps, ...over,
}));
const doc = (html) => new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
const chats = (over = {}) => doc(render({ phone: true, section: 'chats', needsYouRows: rows(), onOpenNeedsYou: noop, ...over }));
const sections = (d) => [...d.querySelectorAll('[data-chats-section]')].map((s) => s.getAttribute('data-chats-section'));
const rowNames = (s) => [...s.querySelectorAll('[data-bot-row] [data-bot-name]')].map((n) => n.textContent);

test('Needs you rows: one per bot, in words, approval before question before browser step', () => {
  assert.deepEqual(rows().map((r) => [r.kwant_id, r.line]), [
    ['tl', 'Wants to open a pull request'],
    ['qa', 'Asked you a question'],
    ['re', 'Needs you in the browser'],
  ]);
  // A bot that both asks and holds a browser step is listed once, with the question
  const both = m.needsYouRows([item('question', 're')], { re: 1 }, new Set(['re']));
  assert.deepEqual(both.map((r) => r.line), ['Asked you a question']);
  assert.equal(m.approvalLine('mcp__x__some_new_tool'), 'Wants your approval');
  // Delegations and routine runs aren't something you answer from the Chats tab
  assert.equal(m.needsYouRows([item('delegation', 'tl')], {}, new Set(['tl'])).length, 0);
});

test('phone Chats tab order: Needs you, Working, Channels, Team chats, Direct chats', () => {
  const d = chats();
  assert.deepEqual(sections(d), ['needs', 'working', 'channels', 'teams', 'direct']);
  const s = (k) => d.querySelector(`[data-chats-section="${k}"]`);
  assert.deepEqual(rowNames(s('needs')), ['TechLead', 'QA', 'Researcher']);
  assert.deepEqual(rowNames(s('working')), ['Architect']);
  assert.deepEqual(rowNames(s('direct')), ['Designer', 'Loner']);
  // Every bot can be opened from the Chats tab, once
  assert.equal(d.querySelectorAll('[data-chat-row]').length, kwants.length);
});

test('Needs you header and lines are amber (warn), never holo, and only when something waits', () => {
  const d = chats();
  const head = d.querySelector('#chats-needs-head');
  assert.equal(head.textContent, 'Needs you · 3');
  assert.match(head.getAttribute('class'), /!text-warn/);
  assert.match(head.getAttribute('class'), /!text-\[11px\]/);
  assert.match(head.getAttribute('class'), /tracking-\[0\.06em\]/);
  for (const r of d.querySelectorAll('[data-chat-row="needs"]')) {
    const line = r.querySelector('[data-bot-busy]');
    assert.match(line.getAttribute('class'), /\btext-warn\b/);
    assert.doesNotMatch(r.outerHTML, /holo/);
  }
  assert.equal(d.querySelector('[data-chat-row="needs"][data-bot-row="tl"] [data-bot-busy]').textContent, 'Wants to open a pull request');
  const none = chats({ needsYouRows: [], needsYou: {}, busyKwants: {} });
  assert.equal(none.querySelector('#chats-needs-head'), null);
  assert.deepEqual(sections(none), ['channels', 'teams', 'direct']);
});

test('Chats tab rows are 56 px launchers with 32 px avatars and no actions', () => {
  const d = chats();
  for (const r of d.querySelectorAll('[data-chat-row]')) {
    assert.match(r.getAttribute('class'), /\bmin-h-14\b/);
    assert.equal(r.querySelector('[data-row-more]'), null);
    assert.equal(r.querySelector('[aria-label^="Edit or delete"]'), null);
  }
});

test('no row on a phone has the selected style, even for the open chat', () => {
  for (const [selectedId, isChannel] of [['c1', true], ['de', false], ['t1', true]]) {
    for (const section of ['chats', 'bots']) {
      const html = render({ phone: true, section, selectedId, isChannel, selectedTeamId: selectedId === 't1' ? 't1' : null, needsYouRows: rows() });
      assert.doesNotMatch(html, /bg-accent-soft|bg-zinc-800 text-zinc-100 shadow-sm/, `${section} ${selectedId}`);
    }
  }
  // Desktop still shows where you are
  assert.match(render({ selectedId: 'de', isChannel: false }), /bg-accent-soft/);
});

test('phone Bots tab: every row has exactly one ⋯ (44×44, 16 px icon) and no pencil', () => {
  const d = doc(render({ phone: true, section: 'bots' }));
  const botRows = [...d.querySelectorAll('[data-bot-row]')];
  assert.equal(botRows.length, kwants.length);
  for (const r of botRows) {
    const more = r.querySelectorAll('[data-row-more]');
    assert.equal(more.length, 1, `${r.getAttribute('data-bot-row')} has one ⋯`);
    assert.match(more[0].getAttribute('class'), /\bw-11\b.*\bh-11\b/);
    assert.match(more[0].getAttribute('class'), /text-\[#a39dcc\]/);
    assert.match(more[0].innerHTML, /width="16"/);
    assert.equal(r.querySelector('.lucide-pencil, [class*="lucide-pencil"]'), null, 'no pencil in the row');
  }
  // Desktop keeps its hover Edit
  assert.match(render({ section: 'all' }), /lucide-pencil/);
});
