// PR 2 of the UX review: one Inbox, one bot list, one switcher, one Settings. Every old way in
// still lands somewhere, and old addresses are rewritten to the new ones.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const store = new Map();
globalThis.localStorage ??= { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)) };

const nav = await loadBundled(`export * from './src/nav'; export { parseSpacePath, spacePath } from './src/spaces';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { Sidebar, botGroups } from './src/components/Sidebar';
  import { SpaceSettings } from './src/components/SpaceSettings';
  import { InboxPage } from './src/components/InboxPage';
  import { ApiKeysPanel } from './src/components/ApiKeysPanel';
  export { botGroups };
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const sidebar = r(Sidebar);
  export const settings = r(SpaceSettings);
  export const inbox = r(InboxPage);
  export const apiKeys = r(ApiKeysPanel);
`);
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const noop = () => {};

// ---------------------------------------------------------------------------
// Addresses

test('every canonical address round-trips', () => {
  const views = [
    { kind: 'inbox', tab: 'new' },
    { kind: 'inbox', tab: 'loose' },
    { kind: 'board' },
    { kind: 'parallel' },
    { kind: 'settings', scope: 'app' },
    ...nav.SPACE_SETTINGS_TABS.map((tab) => ({ kind: 'settings', scope: 'space', tab })),
  ];
  for (const v of views) {
    const seg = nav.viewSegment(v);
    const back = nav.parseView(seg);
    assert.ok(back, seg);
    assert.equal(back.legacy, false, `${seg} is canonical`);
    assert.ok(nav.sameView(back.view, v), seg);
  }
  assert.equal(nav.viewSegment({ kind: 'inbox', tab: 'loose' }), 'inbox/loose-ends');
  assert.equal(nav.viewSegment({ kind: 'settings', scope: 'space', tab: 'accounts' }), 'settings/accounts');
});

test('old addresses resolve to the new place and are flagged for rewriting', () => {
  const cases = {
    away: 'inbox',
    'while-you-were-away': 'inbox',
    'loose-ends': 'inbox/loose-ends',
    connections: 'settings/connections',
    routines: 'settings/routines',
    'mission-board': 'board',
    missions: 'board',
    'parallel-universe': 'parallel',
    'app-settings': 'settings/app',
    'inbox/new': 'inbox',
    'inbox/loose': 'inbox/loose-ends',
    settings: 'settings/general',
    'settings/all': 'settings/app',
  };
  for (const [old, now] of Object.entries(cases)) {
    const p = nav.parseView(old);
    assert.ok(p, old);
    assert.equal(p.legacy, true, `${old} is legacy`);
    assert.equal(nav.viewSegment(p.view), now, old);
  }
  for (const bad of ['', 'nope', 'settings/nope', 'inbox/nope', 'board/x', 'settings/app/x']) assert.equal(nav.parseView(bad), null, bad);
});

test('space paths carry the view, and a view wins over a chat', () => {
  const r = nav.parseSpacePath('/s/spc_a/away');
  assert.equal(r.spaceId, 'spc_a');
  assert.equal(nav.viewSegment(r.view), 'inbox');
  assert.equal(r.legacy, true);
  const s = nav.parseSpacePath('/s/spc_a/settings/accounts');
  assert.equal(nav.viewSegment(s.view), 'settings/accounts');
  assert.ok(!s.legacy);
  assert.equal(nav.spacePath('spc_a', null, { kind: 'inbox', tab: 'loose' }), '/s/spc_a/inbox/loose-ends');
  assert.equal(nav.spacePath('spc_a', { channelId: 'c1' }, { kind: 'board' }), '/s/spc_a/board');
  assert.ok(!nav.spacePath('spc_a', { channelId: 'c1' }).includes('board'));
});

test('the shell rewrites a legacy address in place and keeps Back working', () => {
  const shell = src('Shell.tsx');
  assert.match(shell, /route\.view/, 'the first load opens the view in the address');
  assert.match(shell, /replaceState/);
  assert.match(shell, /pushState/);
  assert.match(shell, /popstate/);
});

// ---------------------------------------------------------------------------
// Item 8: the sidebar's top rows

const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 } };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('k1', 'Ada Lead'), bot('k2', 'Lin Backend'), bot('k3', 'Loner')];
const teams = [{ id: 't1', name: 'Ship', icon: '🚢', lead_kwant_id: 'k1', member_kwants: ['k2', 'k1'] }];
const base = {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants, channels: [], teams, selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
  onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenParallel: noop, onOpenMissions: noop, missionBoard: true,
};

test('the top rows are Search, Inbox, Mission board and More, nothing else', () => {
  const html = ui.sidebar(base);
  const rows = html.match(/data-utility-rows[\s\S]*?<\/nav>|data-utility-rows[\s\S]*$/)[0];
  for (const t of ['Search', 'Inbox', 'Mission board', 'More']) assert.match(rows, new RegExp(t), t);
  for (const gone of ['While you were away', 'Loose ends', 'Connections', 'Routines']) assert.doesNotMatch(html, new RegExp(gone), `${gone} is not a sidebar row`);
  assert.doesNotMatch(html, /Parallel Universe/, 'Parallel Universe sits behind More, closed by default');
});

test('the Inbox row keeps the unseen badge and marks the open page', () => {
  assert.match(ui.sidebar({ ...base, inboxCount: 4 }), />4<span class="sr-only"> new<\/span>/);
  assert.doesNotMatch(ui.sidebar({ ...base, inboxCount: 0 }), /sr-only"> new/);
  assert.match(ui.sidebar({ ...base, inboxActive: true }), /aria-current="page"[^>]*>(?:(?!<\/button>)[\s\S])*>Inbox</);
});

test('Install is a tip that can be dismissed, and stays in Settings › All spaces', () => {
  const tip = src('components/InstallAppButton.tsx');
  assert.match(tip, /dismissInstallTip\(\)/);
  assert.match(tip, /Install any time from Settings › All spaces/);
  assert.match(src('components/SpaceSettings.tsx'), /<InstallAppButton/);
  assert.equal(nav.installTipDismissed(), false);
  nav.dismissInstallTip();
  assert.equal(nav.installTipDismissed(), true);
});

// ---------------------------------------------------------------------------
// Item 9: one bot list, grouped by team

test('bots are grouped by team, lead first, unassigned last', () => {
  const g = ui.botGroups(teams, kwants);
  assert.deepEqual(g.map((x) => x.team?.id ?? null), ['t1', null]);
  assert.deepEqual(g[0].bots.map((k) => k.id), ['k1', 'k2']);
  assert.deepEqual(g[1].bots.map((k) => k.id), ['k3']);
});

test('the sidebar lists each bot once, in its team group', () => {
  const html = ui.sidebar(base);
  assert.equal((html.match(/data-bot-list/g) || []).length, 1, 'one bot list');
  assert.equal((html.match(/data-team-group/g) || []).length, 2, 'Ship and the unassigned group');
  for (const k of kwants) assert.equal((html.match(new RegExp(`data-bot-row="${k.id}"`, 'g')) || []).length, 1, `${k.name} once`);
  assert.ok(html.indexOf('Loner') > html.indexOf('Lin Backend'), 'unassigned bots come last');
});

test('the phone Bots tab uses the same grouped list', () => {
  const html = ui.sidebar({ ...base, phone: true, section: 'bots' });
  assert.match(html, /data-bot-list/);
  assert.equal((html.match(/data-team-group/g) || []).length, 2);
});

// ---------------------------------------------------------------------------
// Item 10: the header is the space name and its settings

test('desktop header: the space name and a settings entry, no switcher dropdown', () => {
  const html = ui.sidebar(base);
  assert.match(html, /data-space-header/);
  assert.match(html, /data-space-settings-open[^>]*aria-label="Settings for Default"|aria-label="Settings for Default"[^>]*data-space-settings-open/);
  assert.doesNotMatch(html, /data-space-toggle/);
});

test('phones keep exactly one switcher', () => {
  const html = ui.sidebar({ ...base, phone: true, section: 'chats' });
  assert.equal((html.match(/data-space-toggle/g) || []).length, 1);
});

// ---------------------------------------------------------------------------
// Item 11: one Settings

const settingsProps = (over = {}) => ({
  space, scope: 'space', onScope: noop, tab: 'connections', onTab: noop, onClose: noop,
  connections: 'CONNECTIONS-HERE', appSettings: 'APP-SETTINGS-HERE', onOpenArchived: noop, kwants, channels: [], teams,
  onSpaceSaved: noop, onArchived: noop, onNewBot: noop, onEditBot: noop, onOpenSkills: noop, onNewTeam: noop, onManageTeam: noop, onOpenTools: noop,
  ...over,
});

test('Settings has This space and All spaces, and Connections is a section of This space', () => {
  const html = ui.settings(settingsProps());
  assert.match(html, /data-settings-scope="space"[^>]*>|aria-selected="true"[^>]*data-settings-scope="space"/);
  assert.match(html, /This space/);
  assert.match(html, /All spaces/);
  assert.match(html, /data-settings-connections[^>]*>CONNECTIONS-HERE/);
  for (const t of ['General', 'Accounts', 'Connections', 'Routines']) assert.match(html, new RegExp(`>${t}<`), t);
  assert.doesNotMatch(html, /APP-SETTINGS-HERE/);
});

test('All spaces shows the app settings, archived spaces and install, not the space tabs', () => {
  const html = ui.settings(settingsProps({ scope: 'app' }));
  assert.match(html, /APP-SETTINGS-HERE/);
  assert.match(html, /Archived spaces/);
  assert.doesNotMatch(html, /CONNECTIONS-HERE/);
  assert.doesNotMatch(html, /aria-label="Default settings"/);
});

test('API keys live only in This space › Accounts; App settings links there', () => {
  const keys = ui.apiKeys({ defaultOpen: false });
  assert.match(keys, /API keys and Ollama/);
  assert.match(keys, /data-api-keys/);
  assert.match(src('components/SpaceSettings.tsx'), /<ApiKeysPanel/);
  const app = src('components/SettingsModal.tsx');
  assert.match(app, /data-api-keys-link/);
  assert.match(app, /onClick=\{onOpenApiKeys\}|onOpenApiKeys\(\)/);
  assert.match(app, /Settings › This space › Accounts/);
  assert.doesNotMatch(app, /Anthropic API Key|placeholder="sk-ant/, 'no key fields left in App settings');
  assert.match(src('App.tsx'), /onOpenApiKeys=\{\(\) => openSpaceSettings\('accounts'\)\}/);
});

test('the rail gear opens Settings › All spaces and the header entry opens This space', () => {
  assert.match(src('Shell.tsx'), /requestView\(\{ kind: 'settings', scope: 'app' \}\)/);
  assert.match(src('components/Spaces.tsx'), /Settings \(all spaces\)/);
  assert.match(src('App.tsx'), /onOpenSpaceSettings=\{\(\) => openSpaceSettings\(/);
});

// ---------------------------------------------------------------------------
// Inbox

test('Inbox has New and Loose ends tabs, with the badge on New', () => {
  const html = ui.inbox({ tab: 'new', onTab: noop, count: 3, newPage: 'NEW-PAGE', loosePage: 'LOOSE-PAGE' });
  assert.match(html, /role="tablist" aria-label="Inbox"/);
  assert.match(html, /data-inbox-tab="new"/);
  assert.match(html, /data-inbox-tab="loose"/);
  assert.match(html, />3<span class="sr-only"> new<\/span>/);
  assert.match(html, /NEW-PAGE/);
  assert.doesNotMatch(html, /LOOSE-PAGE/);
  const loose = ui.inbox({ tab: 'loose', onTab: noop, newPage: 'NEW-PAGE', loosePage: 'LOOSE-PAGE' });
  assert.match(loose, /LOOSE-PAGE/);
  assert.match(loose, /aria-selected="true"[^>]*data-inbox-tab="loose"|data-inbox-tab="loose"[^>]*aria-selected="true"|id="inbox-tab-loose" aria-selected="true"/);
  assert.doesNotMatch(loose, /Back to chats/, 'no back button on desktop');
  assert.match(ui.inbox({ tab: 'new', onTab: noop, onBack: noop, newPage: '', loosePage: '' }), /Back to chats/);
});

// ---------------------------------------------------------------------------
// Follow-ups from Rigel's #204 QA

test('QA a: a long bot name wraps to two lines instead of cutting off at ~20 characters', () => {
  const long = 'Customer-Success-Manager-Extraordinaire';
  const props = { ...base, kwants: [...kwants, bot('k4', long)], teams: [{ ...teams[0], member_kwants: ['k2', 'k1', 'k4'] }] };
  for (const phone of [false, true]) {
    const html = ui.sidebar({ ...props, phone });
    const names = [...html.matchAll(/<span data-bot-name="[^"]*" class="([^"]*)"[^>]*>([^<]*)</g)];
    assert.ok(names.length >= 4, `every bot row has a name (phone=${phone})`);
    for (const [, cls, name] of names) {
      assert.match(cls, /\bline-clamp-2\b/, name);
      assert.match(cls, /overflow-wrap:anywhere/, `${name}: a name with no spaces still breaks`);
      assert.doesNotMatch(cls, /\btruncate\b/, name);
    }
    assert.ok(names.some(([, , n]) => n === long), 'the team member row shows the whole name');
  }
});

test('QA c: Grok has an Install link under "Not available on this computer"', async () => {
  const engines = await loadBundled(`export { INSTALL_URL } from './src/engines';`);
  assert.equal(engines.INSTALL_URL.grok, 'https://x.ai/cli', "xAI's Grok CLI page, which serves the install.sh setup.rs uses");
  for (const id of ['claude', 'codex', 'ollama', 'grok']) assert.match(engines.INSTALL_URL[id], /^https:\/\//, id);
});

test('QA d: nothing tells the person to look in Settings → Profiles any more', () => {
  const turn = readFileSync(new URL('../../src/server/profile_turn.rs', import.meta.url), 'utf8');
  assert.doesNotMatch(turn, /Settings → Profiles/);
  assert.match(turn, /Sign in again \(or update the key\) under Settings › This space › Accounts\./);
  for (const f of ['App.tsx', 'components/SettingsModal.tsx', 'components/EnginesPanel.tsx', 'components/ModelPicker.tsx', 'automationDiscovery.ts']) {
    assert.doesNotMatch(src(f), /Settings (?:→|›|>) Profiles/, f);
  }
});
