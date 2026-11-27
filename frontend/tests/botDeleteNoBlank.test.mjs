// #1301 (Pixel): deleting a bot blanked the whole page until a refresh, in Chromium and Firefox.
// Root cause: after DELETE /api/kwants/:id, App's handleKwantDeleted refetched /api/teams and stored it raw.
// The server sends each team's `member_kwants` as a JSON string, so the next render's devTeamRefs
// (App.tsx, every render) called `.filter` on a string:
//   Chromium: TypeError: (a.member_kwants ?? []).filter is not a function or its return value is not iterable
//   Firefox:  TypeError: (intermediate value).filter is not a function
// and with no error boundary React unmounted everything. Each test bundles its own imports, so on main
// the first test fails on that TypeError, not on a missing export.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

// /api/teams as the server answers it (a fresh space's @helpers, after Writer was deleted).
const serverTeams = [
  { id: 'team-1', name: 'helpers', icon: 'users', lead_kwant_id: 'k-assistant', member_kwants: '["k-assistant","k-researcher"]' },
];
const kwants = [
  { id: 'k-assistant', name: 'Assistant', avatar_sprite: 'product' },
  { id: 'k-researcher', name: 'Researcher', avatar_sprite: 'research' },
];

test('#1301: Dev mode team refs read the server’s member_kwants string instead of throwing', async () => {
  const m = await loadBundled(`export { devTeamRefs } from './src/devMode';`);
  let refs;
  assert.doesNotThrow(() => {
    refs = m.devTeamRefs(serverTeams, kwants);
  });
  assert.equal(refs[0].lead?.name, 'Assistant');
  assert.deepEqual(refs[0].members.map((b) => b.name), ['Assistant', 'Researcher']);
  // Junk never throws either.
  assert.doesNotThrow(() => m.devTeamRefs([{ id: 't', name: 'x', member_kwants: 'not json' }, { id: 'u', name: 'y', member_kwants: 7 }], kwants));
});

test('#1301: the delete handler stores /api/teams through normalizeTeams, never raw', () => {
  const src = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const start = src.indexOf('const handleKwantDeleted');
  assert.ok(start > 0, 'handleKwantDeleted is in App.tsx');
  const body = src.slice(start, src.indexOf('\n  };\n', start));
  assert.doesNotMatch(body, /setTeams\(await tRes\.json\(\)\)/);
  assert.match(body, /setTeams\(normalizeTeams/);
  // Every team load in App goes through it: no raw JSON.parse of member_kwants left.
  assert.doesNotMatch(src, /JSON\.parse\(t\.member_kwants/);
});

test('#1301: normalizeTeams and dropBotFromTeams', async () => {
  const m = await loadBundled(`export { normalizeTeams, memberIdsOfTeam, dropBotFromTeams } from './src/botDelete';`);
  assert.deepEqual(m.normalizeTeams(serverTeams)[0].member_kwants, ['k-assistant', 'k-researcher']);
  assert.deepEqual(m.normalizeTeams([{ id: 'a', member_kwants: ['x'] }])[0].member_kwants, ['x']);
  assert.deepEqual(m.normalizeTeams([{ id: 'a', member_kwants: '{oops' }, { id: 'b' }, null]).map((t) => t.member_kwants), [[], []]);
  assert.deepEqual(m.normalizeTeams({ error: 'nope' }), []);
  assert.deepEqual(m.memberIdsOfTeam('["a",3,"b"]'), ['a', 'b']);
  const before = [{ id: 't1', member_kwants: ['a', 'w'] }, { id: 't2', member_kwants: ['a'] }];
  const after = m.dropBotFromTeams(before, 'w');
  assert.deepEqual(after.map((t) => t.member_kwants), [['a'], ['a']]);
  assert.equal(after[1], before[1], 'teams it wasn’t in stay the same object');
});

test('#1301: where the view goes after a delete (team chat open, its own chat, no channels, no bots)', async () => {
  const m = await loadBundled(`export { chatAfterBotDelete } from './src/botDelete';`);
  const general = { id: 'c-gen', name: 'general' };
  const teamChat = { id: 'c-team', name: 'team-helpers' };
  const bots = [{ id: 'k-assistant' }, { id: 'k-researcher' }];
  const base = { deletedId: 'k-writer', generalChannelId: 'c-gen', botsLeft: bots };
  // (a) team chat open: stays, its messages there stay
  assert.deepEqual(m.chatAfterBotDelete({ ...base, selectedId: 'c-team', isChannel: true, channels: [teamChat, general] }), { kind: 'stay' });
  // another bot's chat open: stays
  assert.deepEqual(m.chatAfterBotDelete({ ...base, selectedId: 'k-assistant', isChannel: false, channels: [general] }), { kind: 'stay' });
  // (b) its own chat open: #general, even when the team chat is listed first
  assert.equal(m.chatAfterBotDelete({ ...base, selectedId: 'k-writer', isChannel: false, channels: [teamChat, general] }).channel, general);
  // (d) no #general: the first channel
  assert.equal(m.chatAfterBotDelete({ ...base, selectedId: 'k-writer', isChannel: false, channels: [teamChat] }).channel, teamChat);
  // no channels at all: the first bot left
  assert.deepEqual(m.chatAfterBotDelete({ ...base, selectedId: 'k-writer', isChannel: false, channels: [] }), { kind: 'bot', bot: bots[0] });
  // nothing left: none
  assert.deepEqual(m.chatAfterBotDelete({ ...base, botsLeft: [], selectedId: 'k-writer', isChannel: false, channels: [] }), { kind: 'none' });
});

test('#1301: a render crash shows one line and Reload, not a blank page; a new space key recovers', async () => {
  const m = await loadBundled(`
    import React from 'react';
    export { React };
    export { createRoot } from 'react-dom/client';
    export { flushSync } from 'react-dom';
    export { AppErrorBoundary, APP_CRASH_LINE } from './src/components/AppErrorBoundary';
  `);
  const { React, createRoot, flushSync, AppErrorBoundary, APP_CRASH_LINE } = m;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const Boom = ({ on }) => {
    if (on) throw new TypeError('(a.member_kwants ?? []).filter is not a function');
    return React.createElement('p', null, 'chat');
  };
  const quiet = console.error;
  console.error = () => {};
  try {
    flushSync(() => root.render(React.createElement(AppErrorBoundary, { resetKey: 's1' }, React.createElement(Boom, { on: true }))));
    const panel = host.querySelector('[data-app-crash]');
    assert.ok(panel, 'the panel shows');
    assert.equal(panel.getAttribute('role'), 'alert');
    assert.match(panel.textContent, new RegExp(APP_CRASH_LINE.replace('.', '\\.')));
    const buttons = [...panel.querySelectorAll('button')];
    assert.deepEqual(buttons.map((b) => b.textContent.trim()), ['Reload']);
    assert.equal(buttons[0].getAttribute('type'), 'button');
    assert.doesNotMatch(host.innerHTML, /#[0-9a-f]{6}\b/i, 'no hard-coded hex');
    // Same space: still the panel. Another space (the rail): a fresh try.
    flushSync(() => root.render(React.createElement(AppErrorBoundary, { resetKey: 's1' }, React.createElement(Boom, { on: false }))));
    assert.ok(host.querySelector('[data-app-crash]'));
    flushSync(() => root.render(React.createElement(AppErrorBoundary, { resetKey: 's2' }, React.createElement(Boom, { on: false }))));
    assert.equal(host.querySelector('[data-app-crash]'), null);
    assert.equal(host.textContent, 'chat');
  } finally {
    console.error = quiet;
    root.unmount();
  }
});

test('#1301: the boundary wraps the whole app and each space', () => {
  const main = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8');
  assert.match(main, /<AppErrorBoundary full>\s*<AuthGate>/);
  const shell = readFileSync(new URL('../src/Shell.tsx', import.meta.url), 'utf8');
  assert.match(shell, /<AppErrorBoundary resetKey=\{current\.id\}>\s*(\{\/\*[^]*?\*\/\}\s*)?<SpaceApp/);
});
