// 33l (autonomy/desktop-shows-who-is-working/brief.md, Mira's ux-pass-0938b section 4): at 1440 you couldn't see who
// is working without scrolling. On a computer, right under the pinned Needs-you block, a calm "Working · n" block
// shows copies of the working bots' rows (attention rank 5): at most 3, then "+n more", which scrolls to and focuses
// the first working bot in the list below. Hidden at 0. The phone's Chats tab is unchanged. Red on main 36f164ec.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const scrolled = [];
dom.window.HTMLElement.prototype.scrollIntoView = function () { scrolled.push(this); };

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { Sidebar } from './src/components/Sidebar';
`);
const { React } = m;
const noop = () => {};
const space = { id: 'spc_default', name: 'Trailhead', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const names = ['Ada', 'Lin', 'Pixel', 'Iris', 'Juno', 'Vega', 'Rigel', 'Mira', 'Scout'];
const kwants = names.map((n) => bot(n.toLowerCase(), n));
const k = (id) => kwants.find((b) => b.id === id);
const teams = [{ id: 't1', name: 'Ship', space_id: 'spc_default', lead_kwant_id: 'ada', member_kwants: ['ada', 'lin', 'pixel', 'iris', 'juno', 'rigel', 'vega'], channel_id: null }];
const asks = [
  { kwant: k('mira'), rank: 1, line: 'Wants your OK', needs: { kwant_id: 'mira', kind: 'approval' } },
  { kwant: k('rigel'), rank: 2, line: 'Failed: QA run' },
  { kwant: k('ada'), rank: 4, line: 'Finished: roadmap' },
];
const working = (...ids) => ids.map((id) => ({ kwant: k(id), rank: 5 }));
const props = (attention, over = {}) => ({
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [{ id: 'c1', name: 'general', space_id: 'spc_default' }], teams, selectedId: null, isChannel: false,
  onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop,
  busyKwants: Object.fromEntries(attention.filter((a) => a.rank === 5).map((a) => [a.kwant.id, 'working'])), botStatus: {}, attention, ...over,
});
const doc = (attention, over) => new DOMParser().parseFromString(`<body>${m.renderToStaticMarkup(React.createElement(m.Sidebar, props(attention, over)))}</body>`, 'text/html');

test('desktop, 3 working: "Working · 3" right under Needs you, with copies of their rows', () => {
  const d = doc([...asks, ...working('pixel', 'juno', 'scout')]);
  const block = d.querySelector('[data-working-now]');
  assert.ok(block, 'a Working block on desktop');
  const h = block.querySelector('h2');
  assert.equal(h.textContent, 'Working · 3');
  assert.equal(h.getAttribute('aria-label'), 'Working, 3');
  assert.equal(block.getAttribute('aria-labelledby'), h.id);
  assert.notEqual(h.id, 'needs-attention-head', 'its own heading id');
  assert.doesNotMatch(block.outerHTML, /amber|lucide-hand/, 'calm: no amber, no hand');
  assert.equal(block.querySelector('[aria-live]'), null, 'no live region');
  assert.deepEqual([...block.querySelectorAll('[data-pin-row]')].map((r) => r.getAttribute('data-pin-row')), ['pixel', 'juno', 'scout']);
  assert.equal(block.querySelector('[data-bot-row]'), null, 'copies only: the bots stay in their places below');
  assert.match(block.textContent, /Also listed in @Ship/);
  // Order: Needs you, then Working, then Channels
  const needs = d.querySelector('[data-needs-attention]');
  assert.equal(needs.nextElementSibling, block, 'directly under Needs you');
  assert.equal(needs.querySelector('h2').textContent, 'Needs you · 2', 'Needs you unchanged: Working never joins it');
  assert.equal(needs.querySelectorAll('[data-pin-row]').length, 3);
  assert.equal(block.querySelector('[data-working-more]'), null, 'no "+n more" at 3');
});

test('desktop, working with nothing pinned: the Working block still shows, before Channels', () => {
  const d = doc(working('lin'));
  assert.equal(d.querySelector('[data-needs-attention]'), null);
  assert.equal(d.querySelector('[data-working-now] h2').textContent, 'Working · 1');
  assert.equal(d.querySelector('[data-working-now]').nextElementSibling.getAttribute('data-chats-section'), 'channels');
});

test('desktop, 5 working: 3 rows and "+2 more"', () => {
  const block = doc([...asks, ...working('lin', 'pixel', 'iris', 'juno', 'scout')]).querySelector('[data-working-now]');
  assert.equal(block.querySelector('h2').textContent, 'Working · 5');
  assert.equal(block.querySelectorAll('[data-pin-row]').length, 3);
  assert.equal(block.querySelector('[data-working-more]').textContent, '+2 more');
});

test('desktop, nobody working: no block', () => {
  const d = doc(asks);
  assert.equal(d.querySelector('[data-working-now]'), null);
  assert.equal(d.querySelector('[data-needs-attention] h2').textContent, 'Needs you · 2');
});

test('"+n more" scrolls to and focuses the first working bot in the list below', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(React.createElement(m.Sidebar, props([...asks, ...working('vega', 'lin', 'pixel', 'iris', 'scout')]))));
  await new Promise((r) => setTimeout(r, 0));
  const more = host.querySelector('[data-working-more]');
  assert.equal(more.textContent, '+2 more');
  scrolled.length = 0;
  m.flushSync(() => more.click());
  // The team list draws Lin before Vega (and Scout, in No team, last), so Lin is the first working bot below
  const first = host.querySelector('[data-bot-row="lin"]');
  assert.equal(document.activeElement, first, 'focus on the first working bot\'s own row');
  assert.deepEqual(scrolled, [first]);
  root.unmount();
  host.remove();
});

test('phone Chats tab: no Working block; its own Working section as before', () => {
  const d = doc([...asks, ...working('lin', 'pixel', 'iris', 'juno', 'scout')], { phone: true, section: 'chats' });
  assert.equal(d.querySelector('[data-working-now]'), null);
  assert.equal(d.querySelector('[data-needs-attention]'), null);
  const sec = d.querySelector('[data-chats-section="working"]');
  assert.ok(sec);
  assert.equal(sec.querySelectorAll('[data-bot-row]').length, 5, 'all five, moved not copied');
});
