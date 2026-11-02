// Rigel's minor findings from the 0.6.4 QA, fixed after the launch: Escape never throws away typed
// text in Settings, Escape closes the bot editor and Manage team (focus goes back to what opened
// them), the Manage team roster works from the keyboard, and the checkpoint list reloads after a
// restore so "Undo this restore" finds its checkpoint.
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
const kwants = [{ ...bot('k1', 'Ada', 'Tech lead'), covers: ['invoices'], avoids: [] }, bot('k2', 'Lin', 'Backend'), bot('k3', 'Rigel', 'QA')];
const team = { id: 't1', name: 'Ship', icon: '🚢', lead_kwant_id: 'k1', member_kwants: ['k1', 'k2'] };
const routes = {
  '/api/kwants': kwants,
  '/api/channels': [{ id: 'ch1', name: 'general', kwant_ids: ['k1', 'k2'], created_at: '' }],
  '/api/teams': [team],
  '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
  '/api/automation-discovery': { setting: 'off', daily_cap: 3, blocked_by: [], last_scan_at: null, suggestions: [], stats: {} },
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
  export { ManageTeamModal } from './src/components/ManageTeamModal';
  export { RestoreSection } from './src/components/MissionBoardView';
  export { isTextField } from './src/escape';
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
/** Presses a key on whatever has focus; true when something took it over (preventDefault). */
const key = async (k) => {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true });
  (document.activeElement ?? document.body).dispatchEvent(e);
  await settle();
  return e.defaultPrevented;
};
/** Types into a controlled field the way React sees it. */
const type = async (el, text) => {
  el.focus();
  const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, text);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
};
/** Identity check on the focused element (assert.equal would diff React's internals on failure). */
const focused = (el, msg) => assert.ok(el && document.activeElement === el, `${msg}: focus is on ${describe(document.activeElement)}`);
const describe = (el) => (el ? `<${el.tagName.toLowerCase()}> "${el.getAttribute('aria-label') ?? el.textContent.trim().slice(0, 30)}"` : 'nothing');

const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 3, ready_accounts: 1, unread: 0 } };

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
const settingsOpen = () => !!q('[data-space-settings]');
const settingsAt = (tab) => mountSpace({ initialView: { kind: 'settings', scope: 'space', tab } });
const botEditor = () => q('[role="dialog"][aria-labelledby="agent-modal-title"]');
const teamDialog = () => q('[role="dialog"][aria-labelledby="manage-team-title"]');

// ---------------------------------------------------------------------------
// 1. Escape in Settings never throws away typed text

test('after launch 1: Escape in a New routine form with a draft keeps the draft and Settings, wherever focus is in the form', async () => {
  const v = await settingsAt('routines');
  const newBtn = q('[data-routine-new]');
  assert.ok(newBtn, 'Settings › Routines has New routine');
  newBtn.focus();
  await click(newBtn);
  const form = q('form[aria-label="New routine"]');
  const text = form.querySelector('textarea');
  await type(text, 'Read the support inbox and list what needs me');

  assert.equal(await key('Escape'), true, 'the form takes Escape');
  assert.ok(settingsOpen(), 'Settings stays open');
  assert.ok(form.isConnected, 'the form stays open');
  assert.equal(text.value, 'Read the support inbox and list what needs me', 'with the draft');

  // Focus on a button in the form, not the field: still kept.
  button('Cancel').focus();
  await key('Escape');
  assert.ok(settingsOpen() && form.isConnected && text.value.startsWith('Read the support'), 'kept from anywhere in the form');

  // Cancel throws it away on purpose, and focus goes back to New routine.
  await click(button('Cancel'));
  assert.ok(!q('form[aria-label="New routine"]'));
  focused(q('[data-routine-new]'), 'after Cancel');
  v.unmount();
});

test('after launch 1: Escape closes an untouched routine form first, then Settings (focus as in D1)', async () => {
  const v = await settingsAt('routines');
  await click(q('[data-routine-new]'));
  assert.ok(q('form[aria-label="New routine"]'));
  await key('Escape');
  assert.ok(!q('form[aria-label="New routine"]'), 'the untouched form closes');
  assert.ok(settingsOpen(), 'Settings is still open');
  focused(q('[data-routine-new]'), 'focus is back on New routine');
  await key('Escape');
  assert.equal(settingsOpen(), false, 'the next Escape closes Settings');
  focused(q('[data-composer]'), 'and focus lands on the composer');
  v.unmount();
});

test('after launch 1: the space rename keeps a changed name on Escape; Settings closes once it matches again', async () => {
  const v = await settingsAt('general');
  const nameField = q('[data-space-settings] form input[type="text"], [data-space-settings] form input:not([type])');
  assert.ok(nameField, 'the space name field');
  await type(nameField, 'Renamed');
  await key('Escape');
  assert.ok(settingsOpen(), 'a changed name keeps Settings open');
  assert.equal(nameField.value, 'Renamed');
  await type(nameField, 'Default');
  await key('Escape');
  assert.equal(settingsOpen(), false, 'unchanged: Escape closes Settings');
  v.unmount();
});

test('after launch 1: telling a text field apart needs no global Element (test pages without one threw on every focus)', () => {
  const was = g.Element;
  delete g.Element;
  try {
    assert.equal(m.isTextField(document.createElement('textarea')), true);
    assert.equal(m.isTextField(document.createElement('input')), true);
    const box = document.createElement('input');
    box.type = 'checkbox';
    assert.equal(m.isTextField(box), false);
    assert.equal(m.isTextField(document.createElement('button')), false);
    assert.equal(m.isTextField(null), false);
  } finally {
    g.Element = was;
  }
});

// ---------------------------------------------------------------------------
// 2. Escape closes the bot editor and Manage team; focus goes back to the opener

test('after launch 2: over Settings, Escape closes a clean bot editor only, and focus goes back to its Edit button', async () => {
  const v = await settingsAt('bots');
  const edit = button('Edit Ada');
  edit.focus();
  await click(edit);
  assert.ok(botEditor(), 'the bot editor is open');
  assert.ok(botEditor().contains(document.activeElement), 'focus moved into it');

  await key('Escape');
  assert.ok(!botEditor(), 'Escape closes the editor');
  assert.ok(settingsOpen(), 'and only the editor: Settings under it stays');
  focused(edit, 'focus is back on Edit Ada');

  // With a changed name it does nothing (there's no Discard dialog in the app).
  await click(edit);
  const name = q('#agent-name');
  await type(name, 'Ada2');
  await key('Escape');
  assert.ok(botEditor(), 'a changed bot stays open');
  assert.equal(q('#agent-name').value, 'Ada2');
  assert.ok(settingsOpen());
  // Focus leaves the name and comes back before it's typed back as it was.
  botEditor().querySelector('textarea').focus();
  await key('Escape');
  assert.ok(botEditor(), 'still changed with focus elsewhere: kept');
  await type(q('#agent-name'), 'Ada');
  await key('Escape');
  assert.ok(!botEditor(), 'changed back: Escape closes it again');
  focused(edit, 'focus');
  v.unmount();
});

test('after launch 2: a changed Good for list keeps the bot editor open on Escape', async () => {
  const v = await settingsAt('bots');
  const edit = button('Edit Ada');
  await click(edit);
  assert.ok(button('Remove invoices'), 'Ada\'s Good for list is in the editor');
  await key('Escape');
  assert.ok(!botEditor(), 'untouched, Escape closes it');
  await click(edit);
  await click(button('Remove invoices'));
  await key('Escape');
  assert.ok(botEditor(), 'a removed Good for phrase keeps it open');
  assert.ok(settingsOpen());
  v.unmount();
});

test('after launch 2: on a phone, where Settings is itself a dialog, Escape closes the bot editor and Manage team over it', async () => {
  const was = window.matchMedia;
  window.matchMedia = (media) => ({ matches: media === '(max-width: 767px)', media, addEventListener() {}, removeEventListener() {} });
  try {
    const v = await settingsAt('bots');
    assert.equal(q('[data-space-settings]')?.getAttribute('role'), 'dialog', 'phone Settings is a dialog');
    const edit = button('Edit Ada');
    edit.focus();
    await click(edit);
    assert.ok(botEditor(), 'the bot editor is open');
    await key('Escape');
    assert.ok(!botEditor(), 'Escape closes the editor');
    assert.ok(settingsOpen(), 'and only the editor');
    focused(edit, 'focus is back on Edit Ada');
    v.unmount();

    const t = await settingsAt('teams');
    const manage = button('Manage @Ship');
    manage.focus();
    await click(manage);
    assert.ok(teamDialog(), 'Manage team is open');
    await key('Escape');
    assert.ok(!teamDialog(), 'Escape closes Manage team');
    assert.ok(settingsOpen(), 'and only Manage team');
    focused(manage, 'focus is back on Manage @Ship');
    t.unmount();
  } finally {
    window.matchMedia = was;
  }
});

test('after launch 2: over Settings, Escape closes a clean Manage team only and focus goes back to Manage; a change keeps it open', async () => {
  const v = await settingsAt('teams');
  const manage = button('Manage @Ship');
  manage.focus();
  await click(manage);
  assert.ok(teamDialog(), 'Manage team is open');
  assert.ok(teamDialog().contains(document.activeElement), 'focus moved into it');
  await key('Escape');
  assert.ok(!teamDialog(), 'Escape closes it');
  assert.ok(settingsOpen(), 'Settings stays');
  focused(manage, 'focus is back on Manage @Ship');

  await click(manage);
  await click(q('[data-roster-row="k2"]'));
  assert.ok(q('[data-team-removals]'), 'Lin is unticked');
  await key('Escape');
  assert.ok(teamDialog(), 'an unsaved roster change keeps it open');
  assert.ok(q('[data-team-removals]'), 'with the change');
  await click(button('Close'));
  focused(manage, 'the ✕ gives focus back too');
  v.unmount();
});

test('after launch 2: from the sidebar, Escape closes Manage team and focus goes back to the team gear', async () => {
  const v = await mountSpace();
  const gear = button('Team settings for @Ship');
  gear.focus();
  await click(gear);
  assert.ok(teamDialog());
  await key('Escape');
  assert.ok(!teamDialog());
  focused(gear, 'focus is back on the gear');
  v.unmount();
});

test('after launch 2: Tab stays inside Manage team', async () => {
  const v = await mount(m.ManageTeamModal, { isOpen: true, team, kwants, onClose: noop, onUpdateTeam: async () => {}, onDeleteTeam: async () => {} });
  const d = teamDialog();
  const items = [...d.querySelectorAll('button:not([disabled]), input:not([disabled]), textarea, select')];
  items.at(-1).focus();
  assert.equal(await key('Tab'), true, 'Tab past the last control is caught');
  assert.ok(d.contains(document.activeElement), 'and focus wraps inside the dialog');
  v.unmount();
});

// ---------------------------------------------------------------------------
// 3. The Manage team roster works from the keyboard

test('after launch 3: each roster row is a toggle button (Tab reaches it, Space and Enter press it), with its state and a visible focus ring', async () => {
  const saved = [];
  const v = await mount(m.ManageTeamModal, { isOpen: true, team, kwants, onClose: noop, onUpdateTeam: async (id, d) => saved.push(d.member_kwants), onDeleteTeam: async () => {} });
  const rows = [...document.querySelectorAll('[data-roster-row]')];
  assert.equal(rows.length, 3);
  for (const r of rows) {
    // A real <button>: browsers put it in the Tab order and press it with Space and Enter.
    assert.equal(r.tagName, 'BUTTON', 'a button, not a clickable div');
    assert.equal(r.getAttribute('type'), 'button', 'that never submits the form');
    assert.notEqual(r.tabIndex, -1, 'in the Tab order');
    assert.match(r.className, /focus-visible:outline/, 'with a focus ring');
  }
  const lin = q('[data-roster-row="k2"]');
  const rigel = q('[data-roster-row="k3"]');
  assert.equal(lin.getAttribute('aria-pressed'), 'true', 'a member is pressed');
  assert.equal(rigel.getAttribute('aria-pressed'), 'false');
  assert.match(teamDialog().textContent, /Select a bot to add or remove it/);
  assert.doesNotMatch(teamDialog().textContent, /Click to add/, 'the hint doesn\'t assume a mouse');
  assert.ok(q('[role="group"][aria-labelledby="team-roster-label"]'), 'the rows are a labelled group');

  await click(lin);
  assert.equal(lin.getAttribute('aria-pressed'), 'false');
  assert.match(q('[data-team-removals]').textContent, /Saving removes Lin from @Ship/, 'the removals line updates');
  await click(rigel);
  assert.equal(rigel.getAttribute('aria-pressed'), 'true');
  await click(lin);
  assert.ok(!q('[data-team-removals]'));
  await click(rigel);

  // The lead rules are unchanged: unticking the lead hands Lead to the next member, and the last
  // member can't be unticked.
  await click(q('[data-roster-row="k1"]'));
  assert.doesNotMatch(q('[data-roster-row="k1"]').textContent, /Lead/);
  assert.match(q('[data-roster-row="k2"]').textContent, /Lead/);
  await click(lin);
  assert.equal(lin.getAttribute('aria-pressed'), 'true', 'the last member stays');
  assert.match(teamDialog().textContent, /A team needs at least one bot\./);
  assert.deepEqual(saved, [], 'toggling saves nothing');
  v.unmount();
});

// ---------------------------------------------------------------------------
// 4. The checkpoint list reloads after a restore

test('after launch 4: after a restore the checkpoint list loads again, with the before-restore checkpoint, and Undo finds it', async () => {
  const cp = (id, kind) => ({ id, kind, attempt: 1, created_at: '2026-11-01T10:00:00Z', files: 2, effects_after: { external: [] }, restorable: true, error: null });
  let cps = [cp('cp1', 'before_run')];
  const calls = [];
  const api = {
    checkpoints: async (id) => {
      calls.push(['list', id]);
      return cps;
    },
    restorePreview: async (id, c) => {
      calls.push(['preview', c]);
      return { checkpoint_id: c, preview_sha256: `sha-${c}`, changes: [{ path: 'a.txt', action: 'overwrite' }], conflicts: [], skipped: [], external_not_undone: [], not_undone_note: '', effects_known: true, task_state_after: 'ready', refused: null };
    },
    restore: async (id, c) => {
      calls.push(['restore', c]);
      cps = [...cps, cp('cp_before', 'before_restore')];
      return { before_restore_checkpoint_id: 'cp_before', restored: 1, removed: 0, task: { id, state: 'ready' } };
    },
  };
  const task = { id: 'tk1', state: 'failed', checkpoint_count: 1 };
  const v = await mount(m.RestoreSection, { task, api, now: Date.parse('2026-11-01T12:00:00Z'), titleOf: () => undefined });
  await click(q('[data-restore-open]'));
  await click(q('[data-checkpoint="cp1"]'));
  await click(q('[data-restore]'));
  assert.deepEqual(calls.filter((c) => c[0] === 'list').length, 2, 'the list is fetched again after the restore');
  assert.doesNotMatch(v.host.textContent, /Loading checkpoints/, 'and doesn\'t hang on Loading');
  assert.ok(q('[data-checkpoint="cp_before"]'), 'it shows the checkpoint taken before the restore');

  await click(q('[data-restore-undo]'));
  assert.deepEqual(calls.at(-1), ['preview', 'cp_before'], 'Undo this restore previews that checkpoint');
  assert.ok(q('[data-restore-preview]'), 'and shows what it would put back');
  v.unmount();
});
