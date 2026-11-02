// Rigel's D1 (0.6.4 QA): keyboard focus reached the chat behind desktop Settings, so hidden text
// could be typed and sent. While Settings is open the sidebar and chat under it are inert, focus
// moves into Settings and comes back when it closes; the remove-member confirm keeps focus too.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
class FakeEvents {
  addEventListener() {}
  removeEventListener() {}
  close() {}
}
g.EventSource = g.window.EventSource = FakeEvents;

const bot = (id, name, title = 'Role') => ({ id, name, title, persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('k1', 'Ada', 'Lead'), bot('k2', 'Lin', 'Backend')];
const team = { id: 't1', name: 'Ship', icon: '🚢', lead_kwant_id: 'k1', member_kwants: ['k1', 'k2'] };
const routes = {
  '/api/kwants': kwants,
  '/api/channels': [{ id: 'ch1', name: 'general', kwant_ids: ['k1', 'k2'], created_at: '' }],
  '/api/teams': [team],
  '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
};
const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init) => {
  const path = String(url).split('?')[0];
  return json(path in routes ? routes[path] : init?.method === 'PUT' ? {} : []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { SpaceApp } from './src/App';
  export { Sidebar } from './src/components/Sidebar';
  export { SpaceSettings } from './src/components/SpaceSettings';
`);
const { React } = m;
const noop = () => {};
const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
/** Presses a key on whatever has focus; true when the app took it over (preventDefault). */
const key = async (k, shiftKey = false) => {
  const e = new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true });
  (document.activeElement ?? document.body).dispatchEvent(e);
  await settle();
  return e.defaultPrevented;
};
/** Identity check on the focused element (assert.equal would diff React's internals on failure). */
const focused = (el, msg) => assert.ok(el && document.activeElement === el, `${msg}: focus is on ${describe(document.activeElement)}`);
const describe = (el) => (el ? `<${el.tagName.toLowerCase()}> "${el.getAttribute('aria-label') ?? el.textContent.trim().slice(0, 30)}"` : 'nothing');

const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 2, ready_accounts: 1, unread: 0 } };

// Whatever a failed test left mounted is unmounted too, or its timers would keep the run going.
const mounted = new Set();
afterEach(() => {
  for (const unmount of mounted) unmount();
  document.body.replaceChildren();
});
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(C, props));
  await settle();
  const unmount = () => {
    if (!mounted.delete(unmount)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(unmount);
  return { host, unmount };
};
const mountSpace = (over = {}) => mount(m.SpaceApp, {
  space, spaces: [space], initialTarget: null, onSwitchSpace: noop, onCreateSpace: noop, onOpenArchived: noop,
  onSpacesChanged: noop, onSpaceArchived: noop, onChatChange: noop, ...over,
});
const q = (sel) => document.querySelector(sel);
const button = (label) => [...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === label || b.textContent.trim() === label);
const title = () => q('#space-settings-title');

// ---------------------------------------------------------------------------
// Desktop Settings: what's under it is inert, focus goes in and comes back

test('D1: while desktop Settings is open the sidebar and composer under it are inert; after it closes they work again', async () => {
  const v = await mountSpace();
  const composer = q('[data-composer]');
  const sidebar = q('aside');
  assert.ok(composer && sidebar, 'the chat and sidebar are on screen');
  assert.equal(composer.closest('[inert]'), null, 'nothing is inert before Settings opens');

  const gear = q('[data-space-settings-open]');
  gear.focus();
  await click(gear);
  const settings = q('[data-space-settings]');
  assert.equal(settings.getAttribute('role'), 'region', 'desktop Settings is a region beside the rail');
  const cover = composer.closest('[inert]');
  assert.ok(cover, 'the composer is inert: no Tab, click or typing reaches it');
  assert.ok(sidebar.closest('[inert]') === cover, 'so is the sidebar, with its gear');
  assert.ok(q('[data-space] [aria-label^="Message "]').closest('[inert]'), 'the whole message form');
  assert.equal(settings.closest('[inert]'), null, 'Settings itself is not');
  assert.ok(!cover.contains(settings), 'Settings sits outside the inert part');
  assert.equal(q('[data-space]').hasAttribute('inert'), false, 'only the part under Settings: the space rail beside it is not in it');

  await click(button('Back to chats'));
  assert.equal(q('[data-space-settings]'), null);
  assert.equal(document.querySelectorAll('[inert]').length, 0, 'nothing stays inert');
  focused(gear, 'focus is back on the gear that opened it');
  composer.focus();
  focused(composer, 'the composer takes focus again');
  v.unmount();
});

test('D1: opening Settings moves focus to its title; Close, Back to chats and Escape all return it to the gear', async () => {
  const v = await mountSpace();
  for (const close of ['Close settings', 'Back to chats', 'Escape']) {
    const gear = q('[data-space-settings-open]');
    gear.focus();
    await click(gear);
    focused(title(), `focus moved into Settings (${close})`);
    if (close === 'Escape') await key('Escape');
    else await click(button(close));
    assert.equal(q('[data-space-settings]'), null, `${close} closes Settings`);
    focused(gear, `${close}: focus back on the gear`);
  }
  v.unmount();
});

test('D1: Settings opened from a link puts focus in it, and closing lands on the composer', async () => {
  const v = await mountSpace({ initialView: { kind: 'settings', scope: 'space', tab: 'accounts' } });
  assert.ok(q('[data-space-settings]'));
  focused(title(), 'focus starts in Settings, not on the page');
  assert.ok(q('[data-composer]').closest('[inert]'), 'the chat under it is inert from the start');
  await key('Escape');
  assert.equal(q('[data-space-settings]'), null);
  focused(q('[data-composer]'), 'no gear opened it, so focus goes to the composer');
  v.unmount();
});

test('D1: the gear gone while Settings was open (a space switch): focus goes to the composer', async () => {
  const v = await mountSpace();
  const gear = q('[data-space-settings-open]');
  gear.focus();
  await click(gear);
  gear.remove();
  await click(button('Close settings'));
  focused(q('[data-composer]'), 'focus');
  v.unmount();
});

test('D1: switching space from the rail keeps focus on the rail; Ctrl/⌘ 1–9 and the rail gear move it into Settings', async () => {
  const props = {
    space, spaces: [space], scope: 'space', onScope: noop, tab: 'general', onTab: noop, onClose: noop,
    connections: 'C', appSettings: 'A', onOpenArchived: noop, kwants, channels: [], teams: [team],
    onSpaceSaved: noop, onArchived: noop, onNewBot: noop, onEditBot: noop, onOpenSkills: noop, onNewTeam: noop, onManageTeam: noop, onOpenTools: noop,
  };
  const rail = document.createElement('nav');
  rail.innerHTML = '<button data-rail-space="spc_b">B</button><button data-settings-opener>All spaces</button>';
  document.body.appendChild(rail);
  // A rail click: focus is on the space's button, and stays there.
  rail.firstChild.focus();
  let v = await mount(m.SpaceSettings, props);
  focused(rail.firstChild, 'focus');
  v.unmount();
  // Ctrl+2: focus was in the old space's Settings, which is gone.
  document.activeElement.blur();
  v = await mount(m.SpaceSettings, props);
  focused(title(), 'focus');
  v.unmount();
  // The rail's gear opens All spaces: focus moves in.
  rail.lastChild.focus();
  v = await mount(m.SpaceSettings, { ...props, scope: 'app' });
  focused(title(), 'focus');
  v.unmount();
  rail.remove();
});

test('D1: on phones Settings stays a full-screen dialog, with the space under it inert too', async () => {
  const prev = window.matchMedia;
  window.matchMedia = () => ({ matches: true, addEventListener: noop, removeEventListener: noop });
  try {
    const v = await mountSpace({ initialView: { kind: 'settings', scope: 'space', tab: 'general' } });
    const settings = q('[data-space-settings]');
    assert.equal(settings.getAttribute('role'), 'dialog');
    assert.equal(settings.getAttribute('aria-modal'), 'true');
    assert.match(settings.className, /\bfixed inset-0\b/);
    focused(title(), 'focus');
    const cover = q('[inert]');
    assert.ok(cover && !cover.contains(settings) && cover.querySelector('aside, [data-composer]'), 'the list or chat under it is inert');
    await click(button('Back to chats'));
    assert.equal(document.querySelectorAll('[inert]').length, 0);
    v.unmount();
  } finally {
    window.matchMedia = prev;
  }
});

// ---------------------------------------------------------------------------
// The desktop remove-member confirm

test('D1: the remove confirm keeps Tab inside, closes on Escape and gives focus back to the ✕; Enter on the ✕ only opens the confirm', async () => {
  const calls = [];
  const selected = [];
  const v = await mount(m.Sidebar, {
    space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
    kwants, channels: [], teams: [team], selectedId: '', isChannel: true, onSelectKwant: (k) => selected.push(k.id), onSelectChannel: noop, onSelectTeam: noop,
    onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenParallel: noop, onOpenMissions: noop, missionBoard: true,
    onEditKwant: noop, onUpdateTeamMembers: (t, ids) => calls.push([t.id, ids]),
  });
  const x = v.host.querySelector('[data-remove-member="k2"]');
  const inDialog = () => !!document.activeElement?.closest('[role="dialog"]');

  // Enter on the ✕ is for the ✕: it doesn't also open Lin's chat behind the confirm (the row's own Enter does).
  x.focus();
  await key('Enter');
  assert.deepEqual(selected, [], 'Enter on the ✕ leaves the row alone');
  const row = v.host.querySelector('[data-bot-row="k2"]');
  row.focus();
  await key('Enter');
  assert.deepEqual(selected, ['k2'], 'Enter on the row opens its chat');

  x.focus();
  await click(x);
  focused(button('Cancel'), 'focus starts on Cancel');
  // Between the two buttons the browser moves focus; the sheet only steps in at either end.
  assert.equal(await key('Tab', true), true, 'Shift+Tab on Cancel is caught');
  focused(button('Remove'), 'Shift+Tab past Cancel wraps to Remove, not out to the page');
  assert.equal(await key('Tab'), true, 'Tab on Remove is caught');
  focused(button('Cancel'), 'Tab past Remove wraps to Cancel');
  assert.equal(await key('Tab'), false, 'Tab on Cancel is left to the browser, which goes on to Remove');
  assert.ok(inDialog());

  await key('Escape');
  assert.equal(q('[role="dialog"]'), null, 'Escape closes it');
  focused(x, 'focus is back on the ✕');

  await click(x);
  await click(button('Cancel'));
  focused(x, 'Cancel also gives focus back to the ✕');
  assert.deepEqual(calls, [], 'and nothing was removed');
  v.unmount();
});

test('D1: after Remove, focus goes to the bot\'s row, since its ✕ left with the team', async () => {
  const v = await mountSpace();
  const x = q('[data-remove-member="k2"]');
  x.focus();
  await click(x);
  await click(button('Remove'));
  assert.equal(q('[role="dialog"]'), null);
  assert.equal(q('[data-remove-member="k2"]'), null, 'Lin is out of the team');
  focused(q('aside [data-bot-row="k2"]'), 'focus is on Lin\'s row, now outside the team');
  v.unmount();
});
