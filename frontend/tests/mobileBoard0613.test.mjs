// Mobile 0.6.13, B3 (Mira's audit P1-6): on a phone the board put eight stat tiles, a chart, Facts and
// Goals above the first task card. Now a phone shows Live now, then the lanes as 44 px tabs that open on
// the lane that matters, then a folded "This week" line, then Facts and Goals. Desktop keeps its order.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default/board', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { React };
  import { MissionBoardView, Board, InsightsStrip, ThisWeek, thisWeekSummary } from './src/components/MissionBoardView';
  export { ThisWeek, thisWeekSummary };
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const view = (p) => r(MissionBoardView, p);
  export const board = (p) => r(Board, p);
  export const insights = (p) => r(InsightsStrip, p);
  export const thisWeek = (p) => r(ThisWeek, p);
`);
const parse = (html) => {
  const d = document.createElement('div');
  d.innerHTML = html;
  return d;
};
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};

const NOW = Date.parse('2026-11-05T12:00:00Z');
const noop = () => {};
const names = { k_lin: 'Lin', k_tw: 'TechWriter' };
let n = 0;
const task = (over = {}) => ({
  id: `t${++n}`, space_id: 's1', mission_id: null, mission_title: null, title: 'Write the pricing copy', acceptance: '',
  state: 'ready', priority: 1, owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 0,
  spent_tokens: 0, attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'person', source_id: null,
  channel_id: null, direct_kwant_id: 'k_lin', thread_id: null, created_by: 'person', created_at: '2026-11-05T10:00:00Z',
  updated_at: '2026-11-05T11:00:00Z', finished_at: null, depends_on: [], waiting_on: [], claim: null, needs_you: [],
  checks_summary: { status: 'not_checked', passed: 0, failed: 0, total: 0 }, flags: [],
  ...over,
});
const insightsData = { verified_7d: 0, false_success_7d: 0, needs_you_now: 0, blocked_now: 1, blocked_minutes_7d: 7 * 24 * 60, interventions_7d: 0, tokens_7d: 1200, tokens_per_verified_7d: null };
// Mira's seed: Ready 2 · Working 1 · Needs you 0 · Review 1
const seed = () => [task(), task({ title: 'Draft the FAQ' }), task({ state: 'working', title: 'Check the links' }), task({ state: 'review', title: 'Fix the footer' })];

test('the phone opens on Needs you when something waits on the person, else Working, else Ready', () => {
  assert.equal(m.defaultColumn([]), 'ready');
  assert.equal(m.defaultColumn([task()]), 'ready');
  assert.equal(m.defaultColumn(seed()), 'working');
  assert.equal(m.defaultColumn([...seed(), task({ state: 'needs_you' })]), 'needs_you');
  // a failed task waiting on a fix is in Needs you, as the lane counts it
  assert.equal(m.defaultColumn([task({ state: 'failed', needs_you: [{ kind: 'fix_needed', id: 'f', text: 'Sign in again.' }] })]), 'needs_you');
  assert.equal(m.defaultColumn([task({ state: 'review' })]), 'ready', 'Review alone does not pick itself: the spec order is Needs you, Working, Ready');
});

test('the lane tabs are a segmented row of 44 px, 13/16 tabs with counts that match the lanes', () => {
  const tasks = seed();
  const d = parse(ui.board({ tasks, names, column: 'working', now: NOW, onColumn: noop, onOpen: noop }));
  const list = d.querySelector('[role="tablist"]');
  assert.match(list.className, /rounded-xl border/, 'one segmented row');
  const tabs = [...list.querySelectorAll('[role="tab"]')];
  assert.equal(tabs.length, 5);
  for (const t of tabs) {
    assert.match(t.className, /min-h-\[44px\]/);
    assert.match(t.className, /text-\[13px\] leading-4/);
    assert.doesNotMatch(t.className, /min-h-\[40px\]|text-xs/);
  }
  const counts = Object.fromEntries(tabs.map((t) => [t.textContent.replace(/\d+$/, '').trim(), Number(t.textContent.match(/\d+$/)[0])]));
  const g = m.groupByColumn(tasks);
  assert.deepEqual(counts, { Ready: g.ready.length, Working: g.working.length, 'Needs you': g.needs_you.length, Review: g.review.length, Verified: 0 });
  assert.deepEqual([counts.Ready, counts.Working, counts['Needs you'], counts.Review], [2, 1, 0, 1]);
  assert.match(tabs[0].querySelector('span:last-child').className, /text-\[12px\] leading-4/, 'the badge reads at 12/16 in a tab');
  // only the picked lane's cards show under the tabs
  const panel = d.querySelector('[data-board="tabs"] [role="tabpanel"]');
  assert.match(panel.textContent, /Check the links/);
  assert.doesNotMatch(panel.textContent, /Draft the FAQ|Fix the footer/);
});

test('the Needs you tab count is amber when something waits; desktop lane pills keep their size', () => {
  const tasks = [...seed(), task({ state: 'needs_you', title: 'Answer a question' })];
  const d = parse(ui.board({ tasks, names, column: 'needs_you', now: NOW, onColumn: noop, onOpen: noop }));
  const tab = [...d.querySelectorAll('[role="tab"]')].find((t) => /Needs you/.test(t.textContent));
  assert.match(tab.querySelector('span:last-child').className, /bg-amber-500/);
  const desk = d.querySelector('[data-board="columns"], [data-board="grid"]') ?? d.querySelector('.lg\\:grid, .hidden.lg\\:block, .hidden.lg\\:grid');
  assert.ok(desk, 'the desktop board is still rendered');
  assert.ok([...desk.querySelectorAll('span')].some((s) => /text-\[10px\]/.test(s.className)), 'its count pills stay 10 px');
  assert.ok(![...desk.querySelectorAll('span')].some((s) => /text-\[12px\] leading-4/.test(s.className)));
});

test('"This week" folds the tiles and chart to one line on a phone; from lg up they show as before', () => {
  assert.equal(ui.thisWeekSummary(insightsData), 'Verified 0 · Blocked 7 d');
  assert.equal(ui.thisWeekSummary({ ...insightsData, blocked_minutes_7d: undefined, verified_7d: 3 }), 'Verified 3 · Blocked now 1', 'an older server without blocked minutes');
  const d = parse(ui.thisWeek({ insights: insightsData, needsYou: 0 }));
  const toggle = d.querySelector('[data-this-week-toggle]');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  assert.match(toggle.className, /lg:hidden/, 'desktop has no toggle');
  assert.match(toggle.className, /min-h-\[44px\]/);
  assert.equal(toggle.querySelector('[data-this-week-summary]').textContent, 'Verified 0 · Blocked 7 d');
  const body = d.querySelector('[data-this-week-body]');
  assert.equal(toggle.getAttribute('aria-controls'), body.id);
  assert.match(body.className, /(^|\s)hidden(\s|$)/, 'tiles hidden on a phone until opened');
  assert.match(body.className, /lg:block/, 'and always shown from lg up');
  assert.ok(body.querySelector('[data-insights]'), 'the tiles are inside the fold');
  assert.equal(d.querySelectorAll('[data-insights]').length, 1, 'rendered once, not a phone copy and a desktop copy');
  assert.equal(ui.thisWeek({ insights: null }), '', 'nothing before the server answers');
});

test('tapping "This week" opens the tiles and tapping again folds them', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  root.render(ui.React.createElement(ui.ThisWeek, { insights: insightsData, needsYou: 0 }));
  await settle();
  const toggle = host.querySelector('[data-this-week-toggle]');
  const body = () => host.querySelector('[data-this-week-body]');
  toggle.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.equal(toggle.getAttribute('aria-expanded'), 'true');
  assert.doesNotMatch(body().className, /(^|\s)hidden(\s|$)/);
  assert.match(body().querySelector('[data-insights] dl, [data-insights] .grid')?.className ?? body().innerHTML, /grid-cols-2/, 'tiles in two columns on a phone');
  toggle.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  assert.match(body().className, /(^|\s)hidden(\s|$)/);
  root.unmount();
  host.remove();
});

test('the tile reads "Board tasks waiting on you", not the Inbox-like "Needs you now"', () => {
  const html = ui.insights({ insights: insightsData, needsYou: 2 });
  assert.match(html, /Board tasks waiting on you<\/dt>/);
  assert.doesNotMatch(html, /Needs you now/);
});

test('on a phone the board comes right after Live now; This week, Facts and Goals follow it; desktop keeps its order', () => {
  const html = ui.view({
    tasks: seed(), missions: [], insights: insightsData, names, missionId: null, column: 'working', loading: false, error: null, now: NOW,
    onMission: noop, onColumn: noop, onOpenTask: noop, onApproveMission: noop, onCancelMission: noop, onRefresh: noop,
    live: m.liveBots([{ turn_id: 'u1', kwant_id: 'k_tw', context_id: 'k_tw', thread_id: null, state: 'running', since: '2026-11-05T11:55:00Z' }], { names, teams: [] }),
    teams: [],
  });
  const d = parse(html);
  const page = d.querySelector('[data-board-page]');
  assert.match(page.className, /flex flex-col/);
  assert.match(page.className, /lg:block lg:space-y-4/, 'desktop is a plain stack again');
  const kids = [...page.children];
  const order = (el) => (/(^|\s)order-2(\s|$)/.test(el.className) ? 2 : /(^|\s)order-1(\s|$)/.test(el.className) ? 1 : 0);
  for (const el of kids) if (order(el)) assert.match(el.className, /lg:order-none/, 'every reorder is undone from lg up');
  const name = (el) =>
    el.matches('[aria-label="Live now"]') ? 'live'
      : el.querySelector('[data-board="tabs"]') || el.matches('[data-board="tabs"]') ? 'board'
      : el.matches('[data-this-week]') ? 'week'
      : el.matches('[data-board-notes]') ? 'notes'
      : null;
  const phone = kids.map((el, i) => ({ el, i })).sort((a, b) => order(a.el) - order(b.el) || a.i - b.i).map(({ el }) => name(el)).filter(Boolean);
  assert.deepEqual(phone, ['live', 'board', 'week', 'notes']);
  const desktop = kids.map(name).filter(Boolean);
  assert.deepEqual(desktop, ['live', 'week', 'notes', 'board'], 'source order, which desktop shows, is as before');
});
