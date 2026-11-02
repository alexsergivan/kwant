// 0.6.3 polish (Rigel's #206 observations): Settings beside the space rail, Ctrl/⌘ 1–9 inside
// Settings, room for Edit and Remove beside a bot's name, and Remove from team always asks first.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });
g.window.fetch = g.fetch;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { act } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { Sidebar } from './src/components/Sidebar';
  export { SpaceSettings } from './src/components/SpaceSettings';
  export { ManageTeamModal } from './src/components/ManageTeamModal';
  export { viewAfterSpaceSwitch, sameView } from './src/nav';
  export { spacePath, parseSpacePath } from './src/spaces';
`);
const { React } = m;
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const noop = () => {};
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 } };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('k1', 'Ada-Lead'), bot('k2', 'Customer-Success-Manager-Extraordinaire'), bot('k3', 'Loner')];
const team = { id: 't1', name: 'Ship', icon: '🚢', lead_kwant_id: 'k1', member_kwants: ['k1', 'k2'] };
const sidebarProps = (over = {}) => ({
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [], teams: [team], selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
  onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenParallel: noop, onOpenMissions: noop, missionBoard: true,
  onEditKwant: noop,
  ...over,
});

const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(C, props));
  await settle();
  return { host, unmount: () => { root.unmount(); host.remove(); } };
};

// ---------------------------------------------------------------------------
// 1. Settings beside the rail

const settingsProps = (over = {}) => ({
  space, spaces: [space], scope: 'space', onScope: noop, tab: 'general', onTab: noop, onClose: noop,
  connections: 'C', appSettings: 'A', onOpenArchived: noop, kwants, channels: [], teams: [team],
  onSpaceSaved: noop, onArchived: noop, onNewBot: noop, onEditBot: noop, onOpenSkills: noop, onNewTeam: noop, onManageTeam: noop, onOpenTools: noop,
  ...over,
});

test('1: on desktop Settings fills the space area and leaves the space rail uncovered', () => {
  const html = m.renderToStaticMarkup(React.createElement(m.SpaceSettings, settingsProps()));
  const root = html.match(/<div[^>]*data-space-settings[^>]*>/)[0];
  assert.match(root, /class="absolute inset-0 /, 'positioned inside the space, not over the whole window');
  assert.doesNotMatch(root, /\bfixed\b/);
  assert.doesNotMatch(root, /aria-modal/, 'the rail beside it stays usable, so it is not a modal');
  assert.match(root, /role="region"/);
  // The space app it sits in is the positioning box; the rail is its sibling in the shell.
  assert.match(src('App.tsx'), /<div className="relative flex-1 min-w-0 h-full flex flex-col[^"]*" data-space=\{space\.id\}>/);
  assert.match(src('Shell.tsx'), /<SpaceRail [\s\S]*?\/>\s*<SpaceApp/);
});

test('1: on phones Settings is still a full-screen dialog', () => {
  const html = m.renderToStaticMarkup(React.createElement(m.SpaceSettings, settingsProps({ isPhone: true })));
  const root = html.match(/<div[^>]*data-space-settings[^>]*>/)[0];
  assert.match(root, /class="fixed inset-0 /);
  assert.match(root, /role="dialog"/);
  assert.match(root, /aria-modal="true"/);
});

test('1: picking another space keeps Settings open: This space follows the new space, All spaces stays', () => {
  const accounts = { kind: 'settings', scope: 'space', tab: 'accounts' };
  const app = { kind: 'settings', scope: 'app' };
  assert.deepEqual(m.viewAfterSpaceSwitch(accounts), accounts);
  assert.deepEqual(m.viewAfterSpaceSwitch(app), app);
  // Anything else lands in a chat, as before.
  for (const v of [null, undefined, { kind: 'inbox', tab: 'new' }, { kind: 'board' }, { kind: 'parallel' }]) assert.equal(m.viewAfterSpaceSwitch(v), null);
  // The new space opens on that view: its address names the new space and the same Settings tab.
  assert.equal(m.spacePath('spc_b', null, accounts), '/s/spc_b/settings/accounts');
  const r = m.parseSpacePath('/s/spc_b/settings/accounts');
  assert.equal(r.spaceId, 'spc_b');
  assert.ok(m.sameView(r.view, accounts));
});

// ---------------------------------------------------------------------------
// 2. Ctrl/⌘ 1–9 inside Settings behaves like the rail

test('2: the rail and Ctrl/⌘ 1–9 both go through the one switch, which carries Settings over', () => {
  const shell = src('Shell.tsx');
  assert.match(shell, /<SpaceRail [^>]*onSwitch=\{switchTo\}/, 'the rail switches with switchTo');
  const keys = shell.match(/\/\/ Ctrl\/⌘ 1–9 switches spaces\.[\s\S]*?\}, \[spaces, switchTo\]\);/)[0];
  assert.match(keys, /switchTo\(s\.id\)/, 'so does the shortcut');
  const sw = shell.match(/const switchTo = useCallback\([\s\S]*?\[open, reload\],/)[0];
  assert.match(sw, /open\(id, to, false, to \? null : viewAfterSpaceSwitch\(viewRef\.current\)\)/);
});

test('2: Back after a switch inside Settings steps back to the old space, then out of Settings, without looping', () => {
  const view = { kind: 'settings', scope: 'space', tab: 'accounts' };
  // History: chat in A → Settings in A → (Ctrl+2) Settings in B.
  const stack = ['/s/spc_a/c/ch1', m.spacePath('spc_a', null, view), m.spacePath('spc_b', null, m.viewAfterSpaceSwitch(view))];
  assert.equal(new Set(stack).size, 3, 'three distinct entries: Back always moves');
  const back1 = m.parseSpacePath(stack[1]);
  assert.equal(back1.spaceId, 'spc_a');
  assert.ok(m.sameView(back1.view, view), 'Back once: Settings in the old space');
  const back2 = m.parseSpacePath(stack[0]);
  assert.equal(back2.spaceId, 'spc_a');
  assert.equal(back2.view ?? null, null, 'Back twice: the chat');
  // The space opened from Back is not pushed again (no new entry, so Forward still works).
  const pop = src('Shell.tsx').match(/const onPop = \(\) => \{[\s\S]*?\n    \};/)[0];
  assert.doesNotMatch(pop, /pushState/);
});

// ---------------------------------------------------------------------------
// 3. Room for Edit and Remove

test('3: on desktop the name line makes room for the hover buttons; phone rows are unchanged', () => {
  const desk = m.renderToStaticMarkup(React.createElement(m.Sidebar, sidebarProps({ onUpdateTeamMembers: noop })));
  const lines = [...desk.matchAll(/<div data-bot-name-line="[^"]*" class="([^"]*)"/g)].map((x) => x[1]);
  assert.ok(lines.length >= 3);
  for (const cls of lines) {
    assert.match(cls, /group-hover\/fleet:pr-9/);
    assert.match(cls, /group-focus-within\/fleet:pr-9/);
  }
  const phone = m.renderToStaticMarkup(React.createElement(m.Sidebar, sidebarProps({ onUpdateTeamMembers: noop, phone: true })));
  for (const [, cls] of phone.matchAll(/<div data-bot-name-line="[^"]*" class="([^"]*)"/g)) assert.doesNotMatch(cls, /pr-9/);
});

// ---------------------------------------------------------------------------
// 4. Remove from team asks first, on every path

test('4: desktop ✕ on a team member asks first; Cancel keeps the bot, Remove takes it out', async () => {
  const calls = [];
  const v = await mount(m.Sidebar, sidebarProps({ onUpdateTeamMembers: (t, ids) => calls.push([t.id, ids]) }));
  const x = () => v.host.querySelector('[data-remove-member="k2"]');
  assert.ok(x(), 'a non-lead member has the ✕');
  assert.equal(v.host.querySelector('[data-remove-member="k1"]'), null, 'the lead has none');

  await click(x());
  assert.deepEqual(calls, [], 'the click alone removes nothing');
  let dlg = document.querySelector('[role="dialog"]');
  assert.ok(dlg, 'a confirm opens');
  assert.match(dlg.textContent, /Remove Customer-Success-Manager-Extraordinaire from @Ship\? The bot stays in this space and keeps its chats\./);
  const btn = (label) => [...document.querySelectorAll('[role="dialog"] button')].find((b) => b.textContent.trim() === label);
  assert.equal(document.activeElement, btn('Cancel'), 'focus starts on Cancel, so Enter does not remove');
  await click(btn('Cancel'));
  assert.deepEqual(calls, []);
  assert.equal(document.querySelector('[role="dialog"]'), null);

  await click(x());
  await click(btn('Remove'));
  assert.deepEqual(calls, [['t1', ['k1']]]);
  v.unmount();
});

test('4: phone member sheet still asks before removing', async () => {
  const calls = [];
  const v = await mount(m.Sidebar, sidebarProps({ phone: true, onUpdateTeamMembers: (t, ids) => calls.push([t.id, ids]) }));
  const more = [...v.host.querySelectorAll('button')].find((b) => /Customer-Success-Manager-Extraordinaire/.test(b.getAttribute('aria-label') ?? '') && /action|more|options/i.test(b.getAttribute('aria-label') ?? ''));
  assert.ok(more, 'the member row has its actions button');
  await click(more);
  const btn = (re) => [...document.querySelectorAll('[role="dialog"] button')].find((b) => re.test(b.textContent));
  await click(btn(/Remove from @Ship/));
  assert.deepEqual(calls, [], 'the menu item only asks');
  await click(btn(/^Remove$/));
  assert.deepEqual(calls, [['t1', ['k1']]]);
  v.unmount();
});

test('4: Manage team names who Save will remove before it happens', async () => {
  const saved = [];
  const v = await mount(m.ManageTeamModal, { isOpen: true, team, kwants, onClose: noop, onUpdateTeam: async (id, d) => saved.push(d.member_kwants), onDeleteTeam: async () => {} });
  assert.equal(document.querySelector('[data-team-removals]'), null);
  const row = [...document.querySelectorAll('[data-roster-row]')].find((d) => d.textContent.includes('Customer-Success-Manager-Extraordinaire'));
  await click(row);
  assert.deepEqual(saved, [], 'unticking saves nothing');
  assert.match(document.querySelector('[data-team-removals]').textContent, /Saving removes Customer-Success-Manager-Extraordinaire from @Ship\./);
  v.unmount();
});

test('4: no other code path takes a bot out of a team without a confirm', () => {
  const sidebar = src('components/Sidebar.tsx');
  // Every call that hands onUpdateTeamMembers a shorter list sits in the confirm's Remove handler.
  const removals = [...sidebar.matchAll(/onUpdateTeamMembers\([^;]*?\.filter\(\(id\) => id !== [^;]*;/g)];
  assert.equal(removals.length, 1, 'one removal call');
  const at = removals[0].index;
  assert.ok(sidebar.lastIndexOf('onRemove={', at) > sidebar.lastIndexOf('onClick={', at), 'and it is the confirm sheet\'s Remove');
});
