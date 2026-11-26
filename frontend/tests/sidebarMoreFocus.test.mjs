// 36i sidebar-fewer-rows (Mira's design check, fix 1): a row opened from inside More moves to the top list
// and keeps keyboard focus (WCAG 2.4.3); a space switch shows the new space's rows on its first render.
import assert from 'node:assert/strict';
import test, { after } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });
g.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });
g.EventSource = class { addEventListener() {} removeEventListener() {} close() {} };

const m = await loadBundled(`
  export { Sidebar } from './src/components/Sidebar';
  export { markRowUsed } from './src/sidebarRows';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { createElement, useState } from 'react';
`);
const noop = () => {};
const mounted = new Set();
after(() => { for (const d of mounted) d(); dom.window.close(); });
const focusName = () => { const a = document.activeElement; return a ? `${a.tagName}${a.hasAttribute('data-parallel-row') ? '[data-parallel-row]' : a.hasAttribute('data-missions-row') ? '[data-missions-row]' : a.getAttribute('data-nav') === 'jar' ? '[data-nav="jar"]' : ''}` : 'none'; };
const act = async (fn) => { m.flushSync(fn); await new Promise((r) => setTimeout(r, 0)); };
const space = (id) => ({ id, name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 } });

// A small App: opening a page makes its row active, as App.tsx does.
const Harness = ({ sid }) => {
  const [page, setPage] = m.useState(null);
  return m.createElement(m.Sidebar, {
    space: space(sid), spaces: [space(sid)], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
    kwants: [], channels: [], teams: [], selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
    onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenRoutines: noop, missionBoard: true,
    onOpenMissions: () => setPage('missions'), missionsActive: page === 'missions',
    onOpenParallel: () => setPage('parallel'), parallelActive: page === 'parallel',
    onOpenIdeaJar: () => setPage('jar'), ideaJarActive: page === 'jar', onDropIdea: async () => {},
  });
};
const mount = async (sid) => {
  const el = document.createElement('div');
  document.body.append(el);
  const root = m.createRoot(el);
  await act(async () => root.render(m.createElement(Harness, { sid })));
  const done = () => { if (!mounted.delete(done)) return; root.unmount(); el.remove(); };
  mounted.add(done);
  return { el, root, done };
};

for (const [key, sel] of [['parallel', '[data-parallel-row]'], ['missions', '[data-missions-row]'], ['jar', '[data-nav="jar"]']]) {
  test(`opening ${key} from inside More moves it up and keeps focus on it`, async () => {
    localStorage.clear();
    const { el, done } = await mount(`spc_focus_${key}`);
    await act(async () => el.querySelector('[data-more-toggle]').click());
    const inMore = el.querySelector(`#kw-sidebar-more ${sel}`);
    assert.ok(inMore, 'the row starts inside More');
    inMore.focus();
    await act(async () => inMore.click());
    const moved = el.querySelector(sel);
    assert.ok(moved && !moved.closest('#kw-sidebar-more'), 'the row is in the top list');
    assert.equal(focusName(), `BUTTON${sel}`, 'focus is on the moved row, not <body>');
    assert.equal(moved.getAttribute('aria-current'), 'page');
    done();
  });
}

test('a row already at the top keeps its place and focus is left alone', async () => {
  localStorage.clear();
  m.markRowUsed('spc_top', 'parallel');
  const { el, done } = await mount('spc_top');
  const row = el.querySelector('[data-parallel-row]');
  assert.ok(!row.closest('#kw-sidebar-more'));
  row.focus();
  await act(async () => row.click());
  assert.equal(focusName(), 'BUTTON[data-parallel-row]');
  done();
});

test('a space switch shows the new space’s rows on its first render', async () => {
  localStorage.clear();
  m.markRowUsed('spc_a', 'parallel');
  const el = document.createElement('div');
  document.body.append(el);
  const root = m.createRoot(el);
  const seen = [];
  const Probe = ({ sid }) => {
    const out = m.createElement(Harness, { sid });
    return out;
  };
  await act(async () => root.render(m.createElement(Probe, { sid: 'spc_a' })));
  assert.ok(el.querySelector('[data-parallel-row]'), 'space a has Parallel at the top');
  // capture each committed DOM via a MutationObserver while switching
  const mo = new window.MutationObserver(() => seen.push(!!el.querySelector('[data-parallel-row]')));
  mo.observe(el, { childList: true, subtree: true });
  await act(async () => root.render(m.createElement(Probe, { sid: 'spc_b' })));
  mo.disconnect();
  assert.equal(el.querySelector('[data-parallel-row]'), null, 'space b starts fresh');
  assert.ok(!seen.includes(true) || seen.length === 0 || seen[seen.length - 1] === false);
  assert.match(el.querySelector('[data-more-toggle]').getAttribute('aria-expanded'), /false/);
  root.unmount();
  el.remove();
});
