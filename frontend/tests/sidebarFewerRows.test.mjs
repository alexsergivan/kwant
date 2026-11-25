// 36i sidebar-fewer-rows (Pixel): Search, Inbox and Routines stay at the top; Mission board, Idea jar and
// Parallel Universe wait under one closed "More" until opened in this space or live.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const store = new Map();
const mem = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };
Object.defineProperty(globalThis, 'localStorage', { value: mem, configurable: true, writable: true });

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { Sidebar } from './src/components/Sidebar';
  export * from './src/sidebarRows';
  export const sidebar = (p) => renderToStaticMarkup(React.createElement(Sidebar, p));
`);
const noop = () => {};
const all = { missions: true, jar: true, parallel: true };

test('a fresh space: all three wait under More, in their order', () => {
  assert.deepEqual(m.placeRows({ present: all, used: [] }), { top: [], more: ['missions', 'jar', 'parallel'] });
});

test('a row opened before in this space moves up and stays up', () => {
  assert.deepEqual(m.placeRows({ present: all, used: ['parallel'] }), { top: ['parallel'], more: ['missions', 'jar'] });
  assert.deepEqual(m.placeRows({ present: all, used: new Set(['missions', 'jar', 'parallel']) }), { top: ['missions', 'jar', 'parallel'], more: [] });
});

test('live rows are at the top: jar takes ready, or the page open now', () => {
  assert.deepEqual(m.placeRows({ present: all, used: [], jarCount: 2 }).top, ['jar']);
  assert.deepEqual(m.placeRows({ present: all, used: [], active: { missions: true } }).top, ['missions']);
  assert.deepEqual(m.placeRows({ present: all, used: [], jarCount: 0 }).top, []);
});

test('Mission board off and phones: a row that is not there is in neither list', () => {
  assert.deepEqual(m.placeRows({ present: { jar: true, parallel: true }, used: ['missions'] }), { top: [], more: ['jar', 'parallel'] });
  // phone: the jar has its own tab
  assert.deepEqual(m.placeRows({ present: { missions: true, parallel: true }, used: [], jarCount: 3 }), { top: [], more: ['missions', 'parallel'] });
});

test('use is remembered per space, and broken storage is ignored', () => {
  store.clear();
  assert.deepEqual(m.markRowUsed('spc_a', 'parallel'), ['parallel']);
  assert.deepEqual(m.markRowUsed('spc_a', 'missions'), ['missions', 'parallel']);
  assert.deepEqual(m.markRowUsed('spc_a', 'missions'), ['missions', 'parallel']);
  assert.deepEqual(m.readUsedRows('spc_a'), ['missions', 'parallel']);
  assert.deepEqual(m.readUsedRows('spc_b'), [], 'another space starts fresh');
  store.set('kw.sidebar.used.spc_c', '{not json');
  assert.deepEqual(m.readUsedRows('spc_c'), []);
  store.set('kw.sidebar.used.spc_c', '["jar","nope"]');
  assert.deepEqual(m.readUsedRows('spc_c'), ['jar']);
  assert.deepEqual(m.readUsedRows('x', undefined), []);
});

const space = (id) => ({ id, name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 } });
const props = (id, over = {}) => ({
  space: space(id), spaces: [space(id)], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants: [], channels: [], teams: [], selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
  onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenRoutines: noop, onOpenParallel: noop, onOpenMissions: noop, missionBoard: true,
  onOpenIdeaJar: noop, onDropIdea: async () => {}, ...over,
});
const top = (html) => html.slice(html.indexOf('data-utility-rows'), html.indexOf('data-more-rows') > 0 ? html.indexOf('data-more-rows') : undefined);

test('the sidebar: a fresh space shows Search, Inbox, Routines and a closed More', () => {
  store.clear();
  const h = m.sidebar(props('spc_fresh'));
  for (const t of ['Search Default', '>Inbox<', '>Routines<', '>More<']) assert.match(top(h) + h, new RegExp(t), t);
  assert.match(h, /<button type="button" aria-expanded="false" aria-controls="kw-sidebar-more"[^>]*data-more-toggle/);
  assert.match(h, /id="kw-sidebar-more" hidden=""/);
  for (const t of ['Mission board', 'Idea jar', 'Parallel Universe']) assert.doesNotMatch(h, new RegExp(t), t);
});

test('the sidebar: used rows sit at the top in their place, and More goes when it holds nothing', () => {
  store.clear();
  m.markRowUsed('spc_used', 'missions');
  let h = m.sidebar(props('spc_used'));
  assert.ok(top(h).indexOf('Mission board') > 0 && top(h).indexOf('Mission board') < top(h).indexOf('>Routines<'), 'Mission board above Routines, as before');
  assert.match(h, />More</);
  m.markRowUsed('spc_used', 'jar');
  m.markRowUsed('spc_used', 'parallel');
  h = m.sidebar(props('spc_used'));
  for (const t of ['Mission board', 'Idea jar', 'Parallel Universe']) assert.match(h, new RegExp(t), t);
  assert.doesNotMatch(h, /data-more-toggle|>More</);
});

test('the sidebar: a jar with takes ready is at the top with its badge and its + quick-add', () => {
  store.clear();
  const h = m.sidebar(props('spc_jar', { ideaJarCount: 3 }));
  assert.match(top(h), /data-idea-jar-row[\s\S]*Idea jar[\s\S]*data-jar-badge[^>]*>3[\s\S]*data-jar-add/);
  assert.match(h, />More</, 'Mission board and Parallel Universe still wait');
});

test('the sidebar on a phone: no jar row, More holds Mission board and Parallel Universe, or is gone', () => {
  store.clear();
  const h = m.sidebar(props('spc_phone', { phone: true, ideaJarCount: 4 }));
  assert.doesNotMatch(h, /Idea jar/);
  assert.match(h, /data-more-toggle/);
  assert.doesNotMatch(m.sidebar(props('spc_phone', { phone: true, missionBoard: false, onOpenParallel: undefined })), /data-more-toggle/);
});
