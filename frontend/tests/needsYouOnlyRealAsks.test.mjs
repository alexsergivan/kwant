// 30z (autonomy/needs-you-only-real-asks/brief.md, Mira's ux-pass-0931 fix #2): one finished routine run must not
// cry wolf. The sidebar's pinned heading says amber "Needs you · n" with a hand only for real asks (ranks 1–2);
// with only rank 3–4 rows it is a calm zinc "New · n" with the Inbox icon. The Inbox count folds a routine's run
// into its review item, as the list does, so the run counts once. Every test here fails on main bd7b7f0b.
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
  export { inboxCount, sectionItems } from './src/doneAway';
`);
const { React } = m;
const noop = () => {};
const NOW = Date.parse('2026-11-20T06:00:00Z');
const at = (min) => new Date(NOW - min * 60_000).toISOString();
const space = { id: 'spc_default', name: 'Trailhead', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('lin', 'Lin'), bot('pix', 'Pixel'), bot('iri', 'Iris')];
const away = (kind, kwant_id, over = {}) => ({ kind, id: `${kind}-${kwant_id}`, kwant_id, kwant_name: kwant_id, title: 'Morning note', summary: '',
  status: 'completed', timestamp: at(3), link: { thread_id: null, channel_id: null, direct_kwant_id: kwant_id }, ...over });
const render = (attention) => m.renderToStaticMarkup(React.createElement(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [{ id: 'c1', name: 'general', space_id: 'spc_default' }], teams: [], selectedId: null, isChannel: false,
  onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop,
  busyKwants: {}, botStatus: {}, attention,
}));
const head = (attention) => new DOMParser().parseFromString(`<body>${render(attention)}</body>`, 'text/html').querySelector('[data-needs-attention] h2');

const review = { kwant: kwants[0], rank: 3, line: 'To review: Routine: Morning note', item: away('delegation', 'lin') };
const finished = { kwant: kwants[2], rank: 4, line: 'Finished: legend', item: away('delegation', 'iri') };
const ask = { kwant: kwants[1], rank: 1, line: 'Wants your OK', item: away('approval', 'pix', { status: 'waiting' }) };

test('only rank 3–4 pinned: a calm zinc "New · n" with the Inbox icon, no hand, no amber', () => {
  const h = head([review]);
  assert.ok(h, 'the block still shows its row');
  assert.equal(h.textContent, 'New · 1');
  assert.equal(h.getAttribute('aria-label'), 'New, 1');
  assert.match(h.getAttribute('class'), /text-zinc-400/);
  assert.doesNotMatch(h.getAttribute('class'), /amber/);
  assert.ok(h.querySelector('svg.lucide-inbox'), 'Inbox icon');
  assert.equal(h.querySelector('svg.lucide-hand'), null, 'no hand');
  assert.equal(head([review, finished]).textContent, 'New · 2');
});

test('one real ask turns it amber "Needs you · 1" with the hand; the count is the asks, the rows stay', () => {
  const html = render([ask, review, finished]);
  const d = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const h = d.querySelector('[data-needs-attention] h2');
  assert.equal(h.textContent, 'Needs you · 1');
  assert.equal(h.getAttribute('aria-label'), 'Needs you, 1');
  assert.match(h.getAttribute('class'), /text-amber-300/);
  assert.ok(h.querySelector('svg.lucide-hand'));
  assert.equal(h.querySelector('svg.lucide-inbox'), null);
  assert.equal(d.querySelectorAll('[data-pin-row]').length, 3, 'all three rows still pinned');
  // A failed or stuck bot (rank 2) is a real ask too
  assert.equal(head([{ ...review, rank: 2, line: 'Failed: crawl' }]).textContent, 'Needs you · 1');
});

// The Inbox after Run now on a read-only routine: its review item and the run it folds. The server's badge is
// unseen_total (1: the run) + review_listed (1) = 2.
const rev = { kind: 'review', id: 'rev-1', task_id: 'task-1', kwant_id: 'lin', kwant_name: 'Lin', title: 'Routine: Morning note', summary: '', status: 'pending',
  group: 'to_review', timestamp: at(2), unread: true, link: {}, held_actions: [] };
const run = { kind: 'routine_run', id: 'run-1', kwant_id: 'lin', kwant_name: 'Lin', title: 'Morning note', summary: '', status: 'completed',
  group: 'finished', timestamp: at(3), unread: true, link: {} };
const approval = { kind: 'approval', id: 'ap-1', kwant_id: 'pix', kwant_name: 'Pixel', title: 'Run a command', summary: '', status: 'waiting',
  group: 'waiting_on_you', timestamp: at(1), unread: true, link: {} };
const inbox = (items, badge) => ({ items, counts: { badge } });

test('Inbox count: a routine run folded into its review item counts once (1, not 2); a real approval adds 1', () => {
  assert.equal(m.sectionItems(inbox([rev, run], 2)).runFor['rev-1'].id, 'run-1', 'the list folds the run');
  assert.equal(m.inboxCount(inbox([rev, run], 2), { unseen: { total: 1 } }), 1);
  assert.equal(m.inboxCount(inbox([approval, rev, run], 3), { unseen: { total: 2 } }), 2);
  // Once the run is read it is no longer in unseen_total, so nothing more comes off
  assert.equal(m.inboxCount(inbox([rev, { ...run, unread: false }], 1), { unseen: { total: 0 } }), 1);
  // A run with no review item is not folded and keeps counting
  assert.equal(m.inboxCount(inbox([run], 1), { unseen: { total: 1 } }), 1);
  // Before the first fetch: today's fallback
  assert.equal(m.inboxCount(null, { unseen: { total: 3 } }, 1), 4);
  assert.equal(m.inboxCount(inbox([rev, run], 0), null), 0, 'never below 0');
});

test('App feeds the folded count to the sidebar, the Inbox page and the phone tab bar', () => {
  const src = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(src, /const inboxCount = useMemo\(\(\) => foldedInboxCount\(inboxState\.data,/);
  assert.doesNotMatch(src, /const inboxCount = inboxBadge\(/);
  for (const use of [/inboxCount=\{inboxCount\}/, /count=\{inboxCount\}/, /badge: inboxCount/]) assert.match(src, use);
});
