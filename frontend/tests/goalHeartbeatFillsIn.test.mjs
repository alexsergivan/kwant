// 33z goal-heartbeat-fills-in (Mira's UX pass 0.9.40, fix #4): the server now logs one `checked` event per run.
// The heartbeat draws them; the timeline leaves them out (24 "Checked: OK" lines a day would bury the story);
// the detail heartbeat sizes to its content, so it no longer collapses and spills over "Checked just now · next in 1 h".
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'localStorage', 'sessionStorage']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const b = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { GoalDetailBody, GoalTimeline } from './src/components/GoalsStrip';
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { GoalsStrip } from './src/components/GoalsStrip';
  export * from './src/goals';
  export { heartbeatDots, heartbeatCount, heartbeatWord } from './src/signature';
  export const timelineHtml = (p) => renderToStaticMarkup(React.createElement(GoalTimeline, p));
  export const detailHtml = (p) => renderToStaticMarkup(React.createElement(GoalDetailBody, { busy: false, error: null, onAction() {}, ...p }));
`);

const NOW = Date.parse('2026-11-22T12:00:00Z');
const minsAgo = (m) => new Date(NOW - m * 60_000).toISOString();
const minsAhead = (m) => new Date(NOW + m * 60_000).toISOString();
const names = { k_cody: 'Cody' };
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/[^\S ]+/g, ' ').trim();

let n = 0;
const ev = (kind, detail, mins) => ({ id: String(++n), kind, actor: 'kwant', detail, created_at: minsAgo(mins) });
const ok = (mins) => ev('checked', { verdict: 'met', passed: 1, total: 1 }, mins);

const goalRaw = {
  id: 'g1', space_id: 's1', title: 'The site stays up', description: '', kwant_id: 'k_cody', status: 'met', mode: 'suggest',
  interval_minutes: 60, created_by: 'person', last_checked_at: minsAgo(0), next_check_at: minsAhead(60), created_at: minsAgo(600), updated_at: minsAgo(0),
  checks: [{ id: 'c1', label: 'Site answers', kind: 'http', spec: { url: 'https://kwant.si' }, required: true, freshness_minutes: null,
    latest: { status: 'passed', verdict: 'met', excerpt: 'HTTP 200', observed_at: minsAgo(0) } }],
  open_mission: null,
};
const parseGoalRow = (raw) => b.parseGoal(raw);
const events = [ev('created', {}, 600), ev('approved', {}, 599), ok(180), ev('met', {}, 120), ok(120), ok(60), ok(0)];

test('heartbeatDots over 3 checked-met events gives 3 ok squares: 3/3 ok, Steady', () => {
  const dots = b.heartbeatDots([ok(120), ok(60), ok(0)]);
  assert.deepEqual(dots.map((d) => d.tone), ['ok', 'ok', 'ok']);
  assert.equal(b.heartbeatCount(dots), '3/3 ok');
  assert.equal(b.heartbeatWord('met', dots), 'Steady');
});

test('the timeline renders no "Checked" rows; the story events stay', () => {
  const html = b.timelineHtml({ events, names, now: NOW });
  assert.doesNotMatch(html, /data-event="checked"/);
  assert.doesNotMatch(text(html), /Checked/);
  assert.match(html, /data-event="created"/);
  assert.match(html, /data-event="met"/);
});

test('a timeline of only checks says nothing has happened yet instead of an empty list', () => {
  const html = b.timelineHtml({ events: [ok(60), ok(0)], names, now: NOW });
  assert.match(text(html), /Nothing has happened yet/);
});

test('the detail: heartbeat has a width and draws the checks; the timeline has no Checked lines', () => {
  const detail = b.parseGoalDetail({ goal: goalRaw, checks: goalRaw.checks, spent_tokens: 0, events, fix_tasks: [], missions: [] });
  const html = b.detailHtml({ detail, names, now: NOW });
  const hb = html.match(/<span class="kws-heartbeat ([^"]*)" data-heartbeat="">/);
  assert.ok(hb, 'the detail draws a heartbeat');
  assert.match(hb[1], /\bkws-heartbeat-fit\b/, 'the heartbeat box sizes to its content instead of collapsing to 0 px');
  assert.equal((html.match(/data-dot="ok"/g) ?? []).length >= 4, true, 'four ok squares (desk view)');
  const timeline = html.slice(html.indexOf('data-goal-timeline'));
  assert.doesNotMatch(timeline, /data-event="checked"/);
});

// Rigel's QA: a healthy goal's row never reads the detail (only goals that may need a decision do), so the list
// row carries its own heartbeat events. Mounted like the board: one list read, and the row draws its squares.
test('a met goal\'s row draws its heartbeat and the First goal kept sticker from the list alone', async () => {
  localStorage.setItem('kwant.first.goal_met.s1', 'g1');
  localStorage.setItem('kwnt.goals.open', '1'); // the open list (it folds to one line while all is well)
  // On the wire newest first, as GET /api/goals sends them
  const heartbeat = [ok(0), ok(60), ok(120)];
  const calls = [];
  const api = {
    list: async () => (calls.push('list'), [parseGoalRow({ ...goalRaw, heartbeat })]),
    get: async (id) => (calls.push(`get ${id}`), b.parseGoalDetail({ goal: goalRaw, checks: goalRaw.checks, events: [], fix_tasks: [] })),
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = b.createRoot(host);
  root.render(b.React.createElement(b.GoalsStrip, { names, now: NOW, api, spaceId: 's1' }));
  for (let i = 0; i < 20 && !host.querySelector('[data-goal="g1"]'); i++) await new Promise((r) => setTimeout(r, 10));
  await new Promise((r) => setTimeout(r, 20));
  const row = host.querySelector('[data-goal="g1"]');
  assert.ok(row, 'the row');
  assert.deepEqual(calls, ['list'], 'a healthy goal needs no detail read');
  const hb = row.querySelector('[data-heartbeat]');
  assert.ok(hb, 'the row draws a heartbeat');
  assert.ok(hb.querySelectorAll('[data-dot="ok"]').length >= 3, hb.outerHTML);
  assert.match(hb.textContent, /Steady/);
  assert.match(row.textContent, /First goal kept/);
  root.unmount();
  host.remove();
  localStorage.removeItem('kwant.first.goal_met.s1');
  localStorage.removeItem('kwnt.goals.open');
});

test('the list row\'s heartbeat parses oldest first; a goal without one (an older server) has none', () => {
  const g = parseGoalRow({ ...goalRaw, heartbeat: [ev('fix_opened', {}, 0), ok(0), ok(60)] });
  assert.deepEqual(g.heartbeat.map((e) => e.kind), ['checked', 'checked', 'fix_opened'], 'same millisecond: logged order kept');
  assert.equal('heartbeat' in parseGoalRow(goalRaw), false, 'absent, so a detail merged over a row keeps the row\'s squares');
});
