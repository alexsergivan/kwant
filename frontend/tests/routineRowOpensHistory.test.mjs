// routine-row-opens-history (autonomy/routine-row-opens-history/brief.md; Mira's ux-pass-0945 Minor f): a routine's
// run in the chat ("Routine · <name>") gets one small History control beside Show/Hide, outside the toggle button. It
// opens Routines on that routine with its History open, focusing the History toggle, never Run now. Matching is the
// chat's bot plus the name (#1121's routineOfNotice rule); in a channel, the name among that channel's routines, and
// only when exactly one matches. No match opens Routines unfocused. Red at #1121's head 5f7732ff.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
g.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as rra from './src/routineRunAgain';
  export { RoutineRow as ChatRoutineRow } from './src/components/RoutineRow';
  export { RoutineRow as PanelRoutineRow } from './src/components/RoutinesPanel';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const NOW = '2026-11-23T19:00:00Z';
const routine = (id, name, extra = {}) => ({
  id, kwant_id: 'k-lead', channel_id: null, name, instructions: 'Write the digest.', schedule: { type: 'daily', time: '08:00', tz: 'UTC' },
  enabled: true, next_run_at: null, last_run_at: null, last_status: null, created_at: NOW, ...extra,
});

const mount = (props) => {
  document.body.innerHTML = '';
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  const render = (p) => root.render(h(m.ChatRoutineRow, { messageId: 'm1', name: 'Morning shop digest', text: 'Write the digest.\nKeep it short.', at: NOW, ...p }));
  render(props);
  return { el, root, render };
};

test('chat row: a named routine gets a History control beside Show/Hide, outside the toggle button', async () => {
  const opened = [];
  const { el, root } = mount({ onOpenHistory: () => opened.push('history') });
  await settle();
  const toggle = el.querySelector('button[aria-expanded]');
  const hist = el.querySelector('[data-routine-history]');
  assert.ok(hist, 'a History control');
  assert.equal(hist.tagName, 'BUTTON');
  assert.equal(hist.getAttribute('type'), 'button');
  assert.match(hist.textContent, /History/);
  assert.equal(toggle.contains(hist), false, 'not nested inside the toggle');
  assert.equal(hist.querySelector('button'), null);
  assert.match(hist.getAttribute('aria-label') ?? '', /Morning shop digest/, 'names the routine for screen readers');
  assert.match(hist.className, /min-h-\[44px\]/, '44 px tall: a touch target');
  assert.match(hist.className, /min-w-\[44px\]/, 'and 44 px wide when the word hides on a phone');
  assert.match(hist.querySelector('span:last-child').className, /hidden sm:inline/, 'the word shows from sm up; the phone shows the icon');
  assert.match(el.querySelector('[data-routine-label]').textContent, /^Routine · Morning shop digest$/, 'the label stays as it is');

  await click(hist);
  assert.deepEqual(opened, ['history'], 'opens the routine’s History');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false', 'History doesn’t toggle the instructions');

  await click(toggle);
  assert.equal(toggle.getAttribute('aria-expanded'), 'true', 'the toggle still shows the instructions');
  assert.equal(el.querySelector('[data-routine-body]').hidden, false);
  await click(toggle);
  assert.equal(toggle.getAttribute('aria-expanded'), 'false', 'and hides them');
  assert.deepEqual(opened, ['history']);
  root.unmount();
});

test('chat row: an unnamed routine, or no way to open Routines, gets no History control', async () => {
  const a = mount({ name: '', onOpenHistory: () => {} });
  await settle();
  assert.equal(a.el.querySelector('[data-routine-history]'), null, 'name === "" → none');
  a.root.unmount();
  const b = mount({});
  await settle();
  assert.equal(b.el.querySelector('[data-routine-history]'), null, 'no handler → none');
  b.root.unmount();
});

test('routineByName: the chat’s bot plus the name; in a channel, exactly one of its routines', () => {
  const list = [
    routine('rt-other-bot', 'Morning shop digest', { kwant_id: 'k-else' }),
    routine('rt-1', 'Morning shop digest', { last_run_at: '2026-11-22T08:00:00Z' }),
    routine('rt-2', 'Daily'),
    routine('rt-ch', 'Standup', { kwant_id: 'k-a', channel_id: 'ch-1' }),
    routine('rt-ch2a', 'Recap', { kwant_id: 'k-a', channel_id: 'ch-1' }),
    routine('rt-ch2b', 'Recap', { kwant_id: 'k-b', channel_id: 'ch-1' }),
    routine('rt-elsewhere', 'Standup', { kwant_id: 'k-a', channel_id: 'ch-2' }),
  ];
  assert.equal(m.rra.routineByName({ kwantId: 'k-lead' }, 'Morning shop digest', list), 'rt-1');
  assert.equal(m.rra.routineByName({ kwantId: 'k-else' }, 'Morning shop digest', list), 'rt-other-bot');
  assert.equal(m.rra.routineByName({ kwantId: 'k-lead' }, 'Gone', list), null, 'deleted or renamed');
  assert.equal(m.rra.routineByName({ kwantId: 'k-lead' }, '', list), null, 'unnamed');
  // Same name twice on one bot: the one that ran last.
  const twice = [routine('rt-a', 'Daily', { last_run_at: '2026-11-20T08:00:00Z' }), routine('rt-b', 'Daily', { last_run_at: NOW })];
  assert.equal(m.rra.routineByName({ kwantId: 'k-lead' }, 'Daily', twice), 'rt-b');
  // A channel: no bot known, so the name among that channel's routines, only when exactly one matches.
  assert.equal(m.rra.routineByName({ channelId: 'ch-1' }, 'Standup', list), 'rt-ch');
  assert.equal(m.rra.routineByName({ channelId: 'ch-1' }, 'Recap', list), null, 'two match: unfocused');
  assert.equal(m.rra.routineByName({ channelId: 'ch-3' }, 'Standup', list), null);
  assert.equal(m.rra.routineByName({}, 'Standup', list), null, 'no place known');
  // routineOfNotice keeps its rule (bot plus name, failed first).
  assert.equal(m.rra.routineOfNotice({ kwant_id: 'k-lead', title: 'Routine "Morning shop digest" failed' }, list), 'rt-1');
});

test('openRoutineHistory: Routines opens at once, then focuses the match; no match or no list stays unfocused', async () => {
  const list = [routine('rt-1', 'Morning shop digest')];
  const did = [];
  const deps = { routines: async () => list, openRoutines: () => did.push('open'), focusRoutine: (x) => did.push(x) };
  assert.equal(await m.rra.openRoutineHistory({ kwantId: 'k-lead' }, 'Morning shop digest', deps), 'rt-1');
  assert.deepEqual(did, ['open', 'rt-1']);
  did.length = 0;
  assert.equal(await m.rra.openRoutineHistory({ kwantId: 'k-lead' }, 'Gone', deps), null);
  assert.equal(await m.rra.openRoutineHistory({ kwantId: 'k-lead' }, 'Morning shop digest', { ...deps, routines: async () => { throw new Error('offline'); } }), null);
  assert.deepEqual(did, ['open', 'open']);
});

test('App and ChatView: the chat row’s History focuses the routine with mode history', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  // assert.ok, not assert.match: a failing match prints all of App.tsx.
  assert.ok(/openRoutineHistory\(/.test(app), 'App wires openRoutineHistory');
  assert.ok(/mode: 'history'/.test(app), "App focuses with mode 'history'");
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.ok(/onOpenHistory=\{onOpenRoutineHistory && routine \? \(\) => onOpenRoutineHistory\(routine\) : undefined\}/.test(chat), 'ChatView hands it to the row');
  const panel = readFileSync(new URL('../src/components/RoutinesPanel.tsx', import.meta.url), 'utf8');
  assert.ok(/mode: 'edit' \| 'pause' \| 'run' \| 'history'/.test(panel), 'the history mode');
});

test('Routines: the history focus opens History, highlights the row and focuses the History toggle, not Run now', async () => {
  document.body.innerHTML = '';
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  const r = routine('rt-1', 'Morning shop digest', { last_status: 'ok' });
  const base = { routine: r, placeLabel: 'direct chat', tick: 0, onEdit: () => {}, onChanged: () => {} };
  root.render(h('ul', null, h(m.PanelRoutineRow, base)));
  await settle();
  const hist = () => [...el.querySelectorAll('button')].find((b) => /History/.test(b.textContent));
  assert.equal(hist().getAttribute('aria-expanded'), 'false');
  root.render(h('ul', null, h(m.PanelRoutineRow, { ...base, focusHistory: 1 })));
  await settle();
  assert.equal(hist().getAttribute('aria-expanded'), 'true', 'History open');
  assert.match(el.querySelector('li').className, /border-sky-500\/60/, 'highlighted');
  assert.equal(document.activeElement, hist(), 'the History toggle has focus');
  assert.notEqual(document.activeElement, el.querySelector('[data-routine-run="rt-1"]'), 'never Run now');
  assert.doesNotMatch(el.textContent, /Turn the switch off/);
  root.unmount();
});
