// H5 attention order (release-090/designs/h5-attention-order.md): one order everywhere a list of bots
// or items is scanned. 1 Needs you · 2 Stuck or failed · 3 Finished, not read · 4 Working · 5 Waiting · 6 the rest.
// Desktop pins ranks 1–3 above Channels (copies); the phone's Chats tab moves bots into sections.
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
  export { Sidebar, PINNED_MAX } from './src/components/Sidebar';
  export { needsYouRows, attentionRows, attentionRank, attentionSort, rollupLabel, stuckLine } from './src/chatsTab';
  export { AWAY_SECTIONS, AWAY_SORT_HINT, sortAwaySection } from './src/away';
  export { AwayView } from './src/components/AwayView';
`);
const { React } = m;
const noop = () => {};
const NOW = Date.parse('2026-11-10T12:00:00Z');
const space = { id: 'spc_default', name: 'Trailhead', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('lin', 'Lin'), bot('pix', 'Pixel'), bot('jun', 'Juno'), bot('iri', 'Iris'), bot('rig', 'Rigel'), bot('ada', 'Ada'), bot('mir', 'Mira'), bot('veg', 'Vega')];
const team = { id: 't1', name: 'core', icon: '🚢', lead_kwant_id: 'ada', member_kwants: ['ada', 'lin', 'pix', 'jun', 'iri', 'rig', 'mir', 'veg'] };
const at = (min) => new Date(NOW - min * 60_000).toISOString();
const item = (kind, kwant_id, over = {}) => ({ kind, id: `${kind}-${kwant_id}-${over.timestamp ?? ''}`, kwant_id, kwant_name: kwant_id, title: '', summary: '',
  status: 'waiting', timestamp: at(5), link: { thread_id: null, channel_id: null, direct_kwant_id: kwant_id }, ...over });

// Lin asked 4 min ago, Pixel wants an OK; Juno failed; Iris finished; Rigel and Vega work; Mira is queued.
// Juno also asked nothing, but Lin also failed: Lin stays rank 1 (highest only).
const waiting = [item('question', 'lin', { timestamp: at(4) }), item('approval', 'pix', { timestamp: at(1) })];
const failed = [item('delegation', 'jun', { status: 'failed', title: 'Claude stopped answering', timestamp: at(30) }), item('delegation', 'lin', { status: 'failed', timestamp: at(50) })];
const finished = [item('delegation', 'iri', { status: 'completed', title: 'legend under the plot', timestamp: at(10) })];
const busy = { rig: 'Working on #ci', veg: 'Working here', mir: 'Waiting to work on #site', lin: 'Asked you a question' };
const status = { rig: 'working', veg: 'working', mir: 'queued', lin: 'needs_you' };
const rows = (over = {}) => m.attentionRows({ kwants, needs: m.needsYouRows(waiting, {}, new Set(kwants.map((k) => k.id))), failed, finished, busy, status, now: NOW, ...over });
const order = (rs) => rs.map((r) => `${r.rank}:${r.kwant.name}`);

test('the one order: each bot once, at its highest rank, ties by name', () => {
  assert.deepEqual(order(rows()), ['1:Lin', '1:Pixel', '2:Juno', '3:Iris', '4:Rigel', '4:Vega', '5:Mira', '6:Ada']);
  const r = rows();
  assert.equal(r.find((x) => x.kwant.id === 'jun').line, 'Failed: Claude stopped answering');
  assert.equal(r.find((x) => x.kwant.id === 'iri').line, 'Finished: legend under the plot');
  assert.equal(m.attentionRank({ needs: true, failed: true, working: true }), 1);
  assert.equal(m.attentionRank({ failed: true, finished: true }), 2);
  assert.equal(m.attentionRank({ queued: true }), 5);
  assert.equal(m.attentionRank({}), 6);
  assert.equal(m.stuckLine({ kind: 'delegation', status: 'stuck', title: 'Long crawl' }), 'Stuck: Long crawl');
});

test('inside groups: needs and stuck oldest first, finished newest first, rank 6 keeps its order', () => {
  const two = [item('delegation', 'jun', { status: 'failed', timestamp: at(10) }), item('delegation', 'rig', { status: 'failed', timestamp: at(40) })];
  const fin = [item('delegation', 'iri', { status: 'completed', timestamp: at(30) }), item('delegation', 'veg', { status: 'completed', timestamp: at(5) })];
  const r = rows({ failed: two, finished: fin, busy: {}, status: {} });
  assert.deepEqual(order(r).slice(0, 6), ['1:Lin', '1:Pixel', '2:Rigel', '2:Juno', '3:Vega', '3:Iris']);
  assert.deepEqual(order(r).slice(6), ['6:Ada', '6:Mira'], 'the stable sidebar order, as given');
  // attentionSort alone
  const s = m.attentionSort([{ kwant: { name: 'B' }, rank: 4 }, { kwant: { name: 'A' }, rank: 4 }, { kwant: { name: 'Z' }, rank: 1, at: 2 }, { kwant: { name: 'Y' }, rank: 1, at: 1 }]);
  assert.deepEqual(s.map((x) => x.kwant.name), ['Y', 'Z', 'A', 'B']);
});

test('rank 2 only counts failures from the last 24 h; rank 3 ends once the chat is opened', () => {
  const old = [item('delegation', 'jun', { status: 'failed', timestamp: at(25 * 60) })];
  assert.equal(rows({ failed: old }).find((x) => x.kwant.id === 'jun').rank, 6);
  // Opened Iris's chat after it finished: read
  assert.equal(rows({ openedAt: { iri: NOW - 60_000 } }).find((x) => x.kwant.id === 'iri').rank, 6);
  // Opened before it finished: still unread
  assert.equal(rows({ openedAt: { iri: NOW - 3600_000 } }).find((x) => x.kwant.id === 'iri').rank, 3);
});

test('rollup words: zeros left out, "1 needs you", working only when nothing else', () => {
  assert.equal(m.rollupLabel({ needs: 2, stuck: 1, finished: 1, working: 3 }), '2 need you, 1 stuck, 1 finished');
  assert.equal(m.rollupLabel({ needs: 1, stuck: 0, finished: 0 }), '1 needs you');
  assert.equal(m.rollupLabel({ needs: 0, stuck: 0, finished: 0, working: 2 }), '2 working');
  assert.equal(m.rollupLabel({ needs: 0, stuck: 0, finished: 0, working: 0 }), '');
});

const render = (over = {}) => m.renderToStaticMarkup(React.createElement(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [{ id: 'c1', name: 'general', space_id: 'spc_default' }], teams: [team], selectedId: 'iri', isChannel: false,
  onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop,
  busyKwants: busy, botStatus: status, attention: rows(), ...over,
}));
const doc = (html) => new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');

test('desktop: pinned "Needs attention · 4" above Channels, copies with "Also listed in @core"', () => {
  const d = doc(render());
  const block = d.querySelector('[data-needs-attention]');
  assert.ok(block);
  assert.equal(block.querySelector('h2').getAttribute('aria-label'), 'Needs attention, 4');
  assert.match(block.querySelector('h2').textContent, /Needs attention · 4/);
  assert.match(block.querySelector('h2').getAttribute('class'), /text-amber-300/);
  const pins = [...block.querySelectorAll('[data-pin-row]')];
  assert.deepEqual(pins.map((p) => p.getAttribute('data-pin-row')), ['lin', 'pix', 'jun', 'iri']);
  // The block sits before Channels, and the team list still has every bot
  const html = d.body.innerHTML;
  assert.ok(html.indexOf('data-needs-attention') < html.indexOf('data-chats-section="channels"'));
  assert.equal(d.querySelectorAll('[data-team-group] [data-bot-row]').length, kwants.length);
  // Copies are real buttons that point at their team; the pinned copy carries no data-bot-row
  for (const p of pins) {
    assert.equal(p.getAttribute('role'), 'button');
    assert.equal(p.hasAttribute('data-bot-row'), false);
    assert.equal(d.getElementById(p.getAttribute('aria-describedby')).textContent, 'Also listed in @core');
  }
  // Line tones: amber, rose, zinc-200
  const tone = (id) => block.querySelector(`[data-pin-row="${id}"] [data-bot-busy]`).getAttribute('class');
  assert.match(tone('lin'), /text-warn/);
  assert.match(tone('jun'), /text-rose-300/);
  assert.match(tone('iri'), /text-zinc-200/);
  assert.match(block.querySelector('[data-pin-row="iri"] [data-bot-busy]').textContent, /Finished: legend under the plot/);
  // The open chat (Iris) is lit in both places
  assert.match(block.querySelector('[data-pin-row="iri"]').getAttribute('class'), /bg-accent-soft/);
  assert.match(d.querySelector('[data-bot-row="iri"]').getAttribute('class'), /bg-accent-soft/);
});

test('desktop: more than 5 shows 5 rows and "and n more in Inbox"; none, or before the first fetch, shows nothing', () => {
  const many = kwants.map((k, i) => item('question', k.id, { timestamp: at(60 - i) }));
  const d = doc(render({ attention: rows({ needs: m.needsYouRows(many, {}, new Set(kwants.map((k) => k.id))) }) }));
  assert.equal(d.querySelectorAll('[data-pin-row]').length, m.PINNED_MAX);
  assert.equal(d.querySelector('[data-needs-attention-more]').textContent, 'and 3 more in Inbox');
  assert.equal(d.querySelector('[data-needs-attention] h2').getAttribute('aria-label'), 'Needs attention, 8');
  const calm = rows({ needs: [], failed: [], finished: [] });
  assert.equal(doc(render({ attention: calm })).querySelector('[data-needs-attention]'), null);
  assert.equal(doc(render({ attention: undefined })).querySelector('[data-needs-attention]'), null);
});

test('team header rollup: dots for ranks 1–3 with words for screen readers; working count when calm', () => {
  const d = doc(render());
  const r = d.querySelector('[data-team-rollup="t1"]');
  assert.equal(r.getAttribute('aria-label'), '2 need you, 1 stuck, 1 finished');
  assert.deepEqual([...r.querySelectorAll('.tabular-nums')].map((n) => n.textContent), ['2', '1', '1']);
  const calm = doc(render({ attention: rows({ needs: [], failed: [], finished: [] }) }));
  assert.equal(calm.querySelector('[data-team-rollup="t1"]').getAttribute('aria-label'), '3 working', 'Lin is still busy');
  const idle = doc(render({ attention: rows({ needs: [], failed: [], finished: [], busy: {} }) }));
  assert.equal(idle.querySelector('[data-team-rollup]'), null);
});

test('phone Chats tab: Needs you, Stuck, Finished, Working, Waiting, then channels; bots move, not copied', () => {
  const d = doc(render({ phone: true, section: 'chats' }));
  const secs = [...d.querySelectorAll('[data-chats-section]')].map((s) => s.getAttribute('data-chats-section'));
  assert.deepEqual(secs, ['needs', 'stuck', 'finished', 'working', 'waiting', 'channels', 'teams', 'direct']);
  const head = (k) => d.querySelector(`#chats-${k}-head`).textContent;
  assert.deepEqual(['needs', 'stuck', 'finished', 'working', 'waiting'].map(head), ['Needs you · 2', 'Stuck · 1', 'Finished · 1', 'Working · 2', 'Waiting · 1']);
  assert.match(d.querySelector('#chats-needs-head').getAttribute('class'), /!text-warn/);
  assert.match(d.querySelector('#chats-stuck-head').getAttribute('class'), /!text-rose-300/);
  const names = (k) => [...d.querySelectorAll(`[data-chats-section="${k}"] [data-bot-name]`)].map((n) => n.textContent);
  assert.deepEqual(names('working'), ['Rigel', 'Vega']);
  assert.deepEqual(names('direct'), ['Ada']);
  assert.equal(d.querySelectorAll('[data-chat-row]').length, kwants.length);
  assert.equal(d.querySelector('[data-needs-attention]'), null, 'no pinned block on the phone');
  // Empty sections are not drawn
  const calm = doc(render({ phone: true, section: 'chats', attention: rows({ needs: [], failed: [], finished: [] }) }));
  assert.deepEqual([...calm.querySelectorAll('[data-chats-section]')].map((s) => s.getAttribute('data-chats-section')).slice(0, 2), ['working', 'waiting']);
});

test('Inbox: Waiting on you, Failed or stuck, Finished; oldest first, finished unread first then newest', () => {
  assert.deepEqual(m.AWAY_SECTIONS.map((s) => s.key), ['waiting', 'failed', 'finished']);
  const lastSeen = at(20);
  const f = [item('delegation', 'a', { timestamp: at(30) }), item('delegation', 'b', { timestamp: at(10) }), item('delegation', 'c', { timestamp: at(40) }), item('delegation', 'd', { timestamp: at(5) })];
  assert.deepEqual(m.sortAwaySection('finished', f, lastSeen).map((i) => i.kwant_id), ['d', 'b', 'a', 'c']);
  assert.deepEqual(m.sortAwaySection('failed', f).map((i) => i.kwant_id), ['c', 'a', 'b', 'd']);
  assert.deepEqual(m.sortAwaySection('waiting', f).map((i) => i.kwant_id), ['c', 'a', 'b', 'd']);
  const data = { since: at(600), last_seen: lastSeen, now: at(0), stuck_after_minutes: 30, waiting: [item('question', 'lin', { timestamp: at(2) }), item('question', 'pix', { timestamp: at(9) })],
    failed: [], finished: f, unseen: { waiting: 2, finished: 4, failed: 0, total: 6 } };
  const html = m.renderToStaticMarkup(React.createElement(m.AwayView, { data, loading: false, error: null, now: NOW, onRefresh: noop, onOpen: noop, onAnswer: noop, onResolve: noop }));
  const d = doc(html);
  const hint = (k) => d.querySelector(`[data-away-hint="${k}"]`);
  assert.match(hint('waiting').textContent, /oldest first/);
  assert.match(hint('finished').textContent, /not read first, then newest/);
  assert.match(hint('finished').getAttribute('class'), /hidden md:inline/, 'desktop only');
  assert.equal(hint('failed'), null, 'no hint on an empty section');
  assert.ok(html.indexOf('id="away-waiting"') < html.indexOf('id="away-failed"') && html.indexOf('id="away-failed"') < html.indexOf('id="away-finished"'));
});
