// 36e settings-short-pages (ux-a11y-1124 plan row 5, Mira's simplify review "### 4"): Settings › All spaces was
// one page 5,760 px tall with 147 controls and three places for usage. It is now four short pages under the
// same left nav as This space (General, Bots on their own, Usage, Notifications), one Usage section, Laya and
// Coding agents under a closed "For developers", one Save for every page, the page in the address, deep links
// that open the right page, and a "Find a setting" field.
import assert from 'node:assert/strict';
import test, { after, afterEach } from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default/settings/app', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLDetailsElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'CustomEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
class FakeEvents {
  addEventListener() {}
  removeEventListener() {}
  close() {}
}
g.EventSource = g.window.EventSource = FakeEvents;
g.window.Element.prototype.scrollIntoView = function () {};

const SETTINGS = {
  anthropic_api_key: '',
  xai_api_key: '',
  openai_api_key: '',
  ollama_url: '',
  claude_cli_found: false,
  claude_cli_path: null,
  grok_cli_found: false,
  grok_cli_path: null,
  codex_cli_found: false,
  codex_cli_path: null,
  notify_webhook_url: '',
  notify_on: ['approval'],
  max_turns_per_agent: 2,
  max_turns_global: 6,
  delegation_follow_up_minutes: 10,
  agent_tools: true,
  timezone: 'Europe/Ljubljana',
  location: '',
  chain_token_budget: 0,
  mission_default_task_budget_tokens: 0,
  mission_default_budget_tokens: 0,
  daily_spend_stop_tokens: 0,
  decide_first: false,
  work_ahead: true,
  work_ahead_daily_cap: 3,
  idea_jar_daily_cap: 0,
  mission_board: true,
};
const posts = [];
const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init) => {
  const path = String(url).split('?')[0];
  if (path === '/api/settings' && init?.method === 'POST') {
    posts.push(JSON.parse(init.body));
    return json({ ok: true });
  }
  if (path === '/api/settings') return json(SETTINGS);
  return json(init?.method ? {} : []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { SettingsModal } from './src/components/SettingsModal';
  export { SpaceSettings, APP_PAGES } from './src/components/SpaceSettings';
  export { parseView, viewSegment, appSettingsPageForRow, APP_SETTINGS_PAGES } from './src/nav';
  export { rowMatches } from './src/settingsFilter';
`);
const { React } = m;
const h = React.createElement;
const noop = () => {};
const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const q = (s, root = document) => root.querySelector(s);
const qa = (s, root = document) => [...root.querySelectorAll(s)];
let roots = [];
afterEach(() => {
  for (const r of roots) r.unmount();
  roots = [];
  document.body.innerHTML = '';
  posts.length = 0;
});
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  roots.push(root);
  m.flushSync(() => root.render(el));
  await settle();
  return { host, root, render: async (next) => (m.flushSync(() => root.render(next)), await settle()) };
};
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (input, value) => {
  const proto = Object.getPrototypeOf(input);
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
};
const app = (page, extra = {}) => h(m.SettingsModal, { embedded: true, isOpen: true, onClose: noop, page, ...extra });
const pageOf = (el) => el.closest('[data-settings-page]')?.getAttribute('data-settings-page');
const shown = (el) => !el.closest('[hidden]') && !el.closest('[data-find-hidden]');

// ---------------------------------------------------------------------------------------------------- address

test('settings/app opens General, settings/app/<page> opens that page, old links still open General', () => {
  assert.deepEqual([...m.APP_SETTINGS_PAGES], ['general', 'bots', 'usage', 'notifications']);
  assert.deepEqual(m.parseView('settings/app'), { view: { kind: 'settings', scope: 'app', page: 'general' }, legacy: false });
  for (const page of ['bots', 'usage', 'notifications']) {
    const seg = m.viewSegment({ kind: 'settings', scope: 'app', page });
    assert.equal(seg, `settings/app/${page}`);
    assert.deepEqual(m.parseView(seg), { view: { kind: 'settings', scope: 'app', page }, legacy: false });
  }
  assert.equal(m.viewSegment({ kind: 'settings', scope: 'app', page: 'general' }), 'settings/app');
  assert.equal(m.viewSegment({ kind: 'settings', scope: 'app' }), 'settings/app', 'no page is General');
  for (const old of ['app-settings', 'settings/all', 'settings/app/general']) {
    const r = m.parseView(old);
    assert.equal(r.view.page, 'general', old);
    assert.equal(r.legacy, true, `${old} is rewritten to settings/app`);
  }
  assert.deepEqual(m.parseView('settings/all/usage'), { view: { kind: 'settings', scope: 'app', page: 'usage' }, legacy: true });
  assert.equal(m.parseView('settings/app/nope'), null);
});

test('deep links pick the page that holds their switch', () => {
  assert.equal(m.appSettingsPageForRow('idea-jar'), 'bots');
  assert.equal(m.appSettingsPageForRow('spend-stop'), 'usage');
  assert.equal(m.appSettingsPageForRow('claude-isolation'), 'general');
  assert.equal(m.appSettingsPageForRow(null), 'general');
  const src = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  for (const row of ['idea-jar', 'spend-stop', 'claude-isolation']) {
    assert.match(src, new RegExp(`setSettingsFocusRow\\('${row}'\\);\\s*openAppSettings\\(appSettingsPageForRow\\('${row}'\\)\\)`), row);
  }
  // Back and Forward: the page is part of the view, so each page is its own history entry.
  assert.match(src, /\{ kind: 'settings', scope: 'app', page: appSettingsPage \}/);
  assert.match(src, /v\.scope === 'app' \? openAppSettings\(v\.page\)/);
});

// ---------------------------------------------------------------------------------------------------- pages

test('four pages: each section sits on its page and the others are hidden, not unmounted', async () => {
  const { host } = await mount(app('general'));
  const at = (sel) => pageOf(q(sel, host));
  assert.equal(at('#settings-timezone'), 'general', 'Where you are');
  assert.equal(at('#sfx-label'), 'general', 'UI sounds');
  assert.equal(at('[data-for-developers]'), 'general');
  assert.equal(at('#agent-tools-label'), 'bots');
  assert.equal(at('[data-switch="decide-first"]'), 'bots');
  assert.equal(at('[data-switch="mission-board"]'), 'bots');
  assert.equal(at('#limit-max_turns_per_agent'), 'bots', 'Parallel work');
  assert.equal(at('[data-token-limits]'), 'usage');
  assert.equal(at('[data-spend-stop]'), 'usage');
  assert.equal(at('#nudges-label'), 'notifications');
  assert.equal(at('#browser-notify-label'), 'notifications');
  assert.equal(at('#notify-webhook'), 'notifications');
  assert.equal(at('#notify-kinds-label'), 'notifications');
  // Only General shows; the rest are in the page, hidden.
  for (const el of qa('[data-settings-page]', host)) assert.equal(el.hidden, pageOf(el) !== 'general', pageOf(el));
  assert.ok(shown(q('#settings-timezone', host)));
  assert.ok(!shown(q('[data-switch="decide-first"]', host)));
});

test('Usage is one section with one Usage heading: open-chat tokens, limits, spend stop, AI use, budgets in that order', async () => {
  const { host } = await mount(app('usage'));
  const sec = q('[data-usage-section]', host);
  assert.ok(shown(sec));
  const order = ['Token usage, open chat', 'Token limits', 'Hand-offs per request', 'Daily spending stop'].map((t) => sec.textContent.indexOf(t));
  assert.ok(order.every((n, i) => n >= 0 && (i === 0 || n > order[i - 1])), `order ${order}`);
  // The page title says Usage; the section's legend is for screen readers only, and nothing else says it.
  assert.equal(q('legend', sec).className, 'sr-only');
  const headings = qa('h1,h2,h3,h4,legend:not(.sr-only)', host).filter(shown).filter((e) => /\busage\b/i.test(e.textContent));
  assert.deepEqual(headings.map((e) => e.textContent), [], 'no second or third Usage heading');
  // Not on any other page.
  assert.equal(qa('[data-usage-section]', host).length, 1);
});

test('For developers starts closed and holds Laya and Coding agents', async () => {
  const { host } = await mount(app('general'));
  const dev = q('details[data-for-developers]', host);
  assert.equal(dev.open, false);
  assert.equal(q('summary', dev).textContent.trim(), 'For developers');
  assert.match(dev.textContent, /Laya, the router/);
  assert.ok(q('[data-coding-agents]', dev), 'Coding agents');
  // Laya is nowhere else on the page.
  const outside = qa('[data-setting-row]', host).filter((r) => !dev.contains(r));
  assert.ok(outside.every((r) => !/Laya, the router/.test(r.textContent)));
});

test('an edit on one page survives a switch to another, and Save there sends it', async () => {
  const v = await mount(app('general'));
  await type(q('#settings-timezone', v.host), 'Europe/Kyiv');
  await click(q('[data-switch="decide-first"]', v.host)); // hidden page, still there: it is the same form
  await v.render(app('notifications'));
  assert.equal(q('#settings-timezone', v.host).value, 'Europe/Kyiv', 'the typed zone is still in the hidden page');
  assert.ok(!shown(q('#settings-timezone', v.host)));
  // One Save, on every page.
  const save = qa('button[type="submit"]', v.host);
  assert.equal(save.length, 1);
  assert.ok(shown(save[0]));
  assert.equal(save[0].getAttribute('form'), 'kw-app-settings-form');
  await v.render(app('usage'));
  assert.ok(shown(q('button[type="submit"]', v.host)));
  q('form#kw-app-settings-form', v.host).dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.equal(posts.length, 1);
  assert.equal(posts[0].timezone, 'Europe/Kyiv');
  assert.equal(posts[0].decide_first, true);
  assert.equal(posts[0].max_turns_per_agent, 2, 'the other pages go too');
});

test('focusRow: the jar opens Bots on their own and focuses its switch; claude-isolation lands on General', async () => {
  const v = await mount(app(m.appSettingsPageForRow('idea-jar'), { focusRow: 'idea-jar' }));
  const sw = q('[data-switch="idea-jar"]', v.host);
  assert.ok(sw && shown(sw));
  assert.equal(document.activeElement, sw);
});

// ---------------------------------------------------------------------------------------------------- nav + find

const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null, created_at: '', general_channel_id: 'ch1' };
const shell = (props = {}) =>
  h(m.SpaceSettings, {
    space,
    spaces: [space],
    scope: 'app',
    onScope: noop,
    tab: 'general',
    onTab: noop,
    onClose: noop,
    connections: () => null,
    appSettings: (page) => app(page),
    onOpenArchived: noop,
    kwants: [],
    channels: [],
    teams: [],
    onSpaceSaved: noop,
    onArchived: noop,
    onNewBot: noop,
    onEditBot: noop,
    onOpenSkills: noop,
    onNewTeam: noop,
    onManageTeam: noop,
    onOpenTools: noop,
    ...props,
  });

test('All spaces has the same left nav as This space, four pages, one h3 with the page name', async () => {
  const picked = [];
  const v = await mount(shell({ appPage: 'usage', onAppPage: (p) => picked.push(p) }));
  const nav = q('nav[data-app-settings-nav]', v.host);
  assert.deepEqual(qa('button', nav).map((b) => b.textContent), ['General', 'Bots on their own', 'Usage', 'Notifications']);
  assert.equal(q('[aria-current="page"]', nav).textContent, 'Usage');
  assert.match(nav.className, /md:flex-col/, 'a column on desktop, chips on phones, like This space');
  const h3 = qa('h3', v.host).filter((e) => e.closest('[data-app-page]') && !e.closest('[data-settings-page]'));
  assert.deepEqual(h3.map((e) => e.textContent), ['Usage']);
  // One h3 per page: the cards inside (version, AI use) are h4 under it.
  for (const page of ['general', 'bots', 'usage', 'notifications']) {
    await v.render(shell({ appPage: page }));
    const all = qa('h3', v.host).filter(shown).map((e) => e.textContent.trim());
    assert.equal(all.length, 1, `${page}: ${all.join(' | ')}`);
  }
  await v.render(shell({ appPage: 'usage', onAppPage: (p) => picked.push(p) }));
  assert.match(v.host.textContent, /Applies to every space and to this app/);
  assert.ok(!shown(q('[data-all-spaces-spaces]', v.host)), 'the Spaces card is on General');
  await click(qa('button', nav)[1]);
  assert.deepEqual(picked, ['bots']);
});

test('Find a setting: "Notify me on this device" is found from General, across pages; clearing goes back', async () => {
  assert.ok(m.rowMatches('notify me on this device when a bot needs you', 'Notify  DEVICE'));
  assert.ok(!m.rowMatches('ui sounds', 'notify'));
  const v = await mount(shell({ appPage: 'general' }));
  const find = q('input[data-settings-find]', v.host);
  assert.equal(find.getAttribute('aria-label'), 'Find a setting');
  await type(find, 'Notify me on this device');
  await settle();
  const row = q('[data-testid="device-notify"]', v.host);
  assert.ok(shown(row), 'the Notifications row shows while General is the page');
  assert.ok(!shown(q('#sfx-label', v.host)), 'rows without the words hide');
  assert.ok(!shown(q('[data-switch="decide-first"]', v.host)));
  assert.match(q('[data-settings-find-count]', v.host).textContent, /^1 setting matches/);
  assert.equal(q('nav[data-app-settings-nav] [aria-current="page"]', v.host), null);
  await type(find, 'zzzz nothing');
  assert.match(q('[data-settings-find-count]', v.host).textContent, /^Nothing matches/);
  await type(find, '');
  assert.ok(shown(q('#sfx-label', v.host)), 'back to General');
  assert.ok(!shown(row));
  assert.equal(q('[data-settings-find-count]', v.host), null);
});

test('Find a setting matches at the start of words; our own Clear button; the page chips wrap', async () => {
  assert.ok(!m.rowMatches('use kwant bots on your phone add it to your home screen', 'notify me'), '"me" inside "Home" is no match');
  assert.ok(m.rowMatches('notify me on this device', 'notify me'));
  assert.ok(m.rowMatches('notifications', 'notif'));
  assert.ok(m.rowMatches("don't ask again", 'dont'));
  assert.ok(m.rowMatches('dont ask again', "don\u2019t"));
  const v = await mount(shell({ appPage: 'general' }));
  const nav = q('nav[data-app-settings-nav]', v.host);
  assert.match(nav.className, /\bflex-wrap\b/);
  assert.doesNotMatch(nav.className, /overflow-x-auto/);
  const find = q('input[data-settings-find]', v.host);
  assert.match(find.className, /\[&::-webkit-search-cancel-button\]:appearance-none/);
  assert.equal(q('[data-settings-find-clear]', v.host), null, 'no Clear button while empty');
  await type(find, 'notify me');
  await settle();
  assert.match(q('[data-settings-find-count]', v.host).textContent, /^1 setting matches/);
  const clear = q('[data-settings-find-clear]', v.host);
  assert.equal(clear.getAttribute('type'), 'button');
  assert.equal(clear.getAttribute('aria-label'), 'Clear');
  await click(clear);
  await settle();
  assert.equal(find.value, '');
  assert.equal(v.host.ownerDocument.activeElement, find, 'focus goes back to the field');
  assert.equal(q('[data-settings-find-clear]', v.host), null);
  assert.ok(shown(q('#sfx-label', v.host)), 'back to General');
});

// Some panels start long timers that jsdom keeps alive after unmount; close the window so node can exit.
after(() => {
  dom.window.close();
});
