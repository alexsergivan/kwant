// 31e goal-board-text-fits (Mira's 0.9.31 UX pass, "Also seen" d, e, f): the drafting line never reads
// "…HTTP 200.”.", a bot's draft goal card spans the strip so its two lines stop cutting off at 1440, and the
// Mission board tile says "Waiting on you".
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const b = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { GoalRow } from './src/components/GoalsStrip';
  import { InsightsStrip } from './src/components/MissionBoardView';
  export { quotedGoalText } from './src/components/GoalsStrip';
  export { parseGoal } from './src/goals';
  export const rowHtml = (p) => renderToStaticMarkup(React.createElement(GoalRow, { onOpen() {}, ...p }));
  export const insightsHtml = (p) => renderToStaticMarkup(React.createElement(InsightsStrip, p));
`);

const NOW = Date.parse('2026-11-20T08:00:00Z');
const ago = (m) => new Date(NOW - m * 60_000).toISOString();
const names = { k_mia: 'Mia' };
const goal = (over = {}) =>
  b.parseGoal({
    id: 'g1', space_id: 's1', title: 'Check kwant.si answers with HTTP 200', description: '', kwant_id: 'k_mia', status: 'met', mode: 'suggest',
    interval_minutes: 60, created_by: 'person', last_checked_at: ago(10), next_check_at: ago(-50), created_at: ago(600), updated_at: ago(10),
    checks: [{ id: 'c1', label: 'Site answers', kind: 'http', spec: { url: 'https://kwant.si', expect_status: 200 }, required: true, freshness_minutes: null, latest: { status: 'passed', verdict: 'met', excerpt: 'HTTP 200', observed_at: ago(10) } }],
    open_mission: null, ...over,
  });
const liClass = (html) => html.match(/^<li[^>]*class="([^"]*)"/)?.[1] ?? '';

test('the drafting line quotes the goal without its closing full stop, question or exclamation mark', () => {
  assert.equal(b.quotedGoalText('Check kwant.si answers with HTTP 200.'), 'Check kwant.si answers with HTTP 200');
  assert.equal(b.quotedGoalText('  Is it up?  '), 'Is it up');
  assert.equal(b.quotedGoalText('Keep it up!'), 'Keep it up');
  assert.equal(b.quotedGoalText('Wait for it…'), 'Wait for it');
  assert.equal(b.quotedGoalText('Backups run nightly'), 'Backups run nightly', 'no punctuation: unchanged');
  assert.equal(b.quotedGoalText('Version 0.9.33 ships'), 'Version 0.9.33 ships', 'dots inside the sentence stay');
  assert.equal(b.quotedGoalText('Really?!'), 'Really?', 'only one mark comes off');
  const long = `${'a'.repeat(90)}.`;
  assert.equal(b.quotedGoalText(long), `${'a'.repeat(79)}…`, 'over 80 characters: cut to 79 and "…"');
  assert.equal(b.quotedGoalText(`${'a'.repeat(80)}.`), 'a'.repeat(80), 'exactly 80 after the stop comes off: not cut');
  const src = readFileSync(new URL('../src/components/GoalsStrip.tsx', import.meta.url), 'utf8');
  assert.match(src, /is drafting checks for “\{quotedGoalText\(drafting\.text\)\}”\. The draft shows here\./, 'the line uses the helper');
});

test('a bot’s draft card spans the strip and its "proposed this" line wraps; a met card keeps its cell', () => {
  const draft = b.rowHtml({ goal: goal({ status: 'draft', created_by: 'k_mia', last_checked_at: null }), names, now: NOW });
  const cls = liClass(draft);
  assert.match(cls, /(^|\s)sm:col-span-2(\s|$)/);
  assert.match(cls, /(^|\s)xl:col-span-3(\s|$)/);
  assert.match(cls, /(^|\s)flex(\s|$)/, 'still the compact row layout, with Review on the right');
  const line = draft.match(/<p class="([^"]*)" data-goal-line="draft">([^<]*)</);
  assert.ok(line, draft);
  assert.doesNotMatch(line[1], /truncate/, 'the "proposed this" line is not cut off');
  assert.match(line[1], /break-words/);
  assert.equal(line[2], 'Mia proposed this. Nothing runs until you approve it.');
  assert.match(draft, /title="Check kwant.si answers with HTTP 200"/, 'the title keeps its tooltip');

  const met = liClass(b.rowHtml({ goal: goal(), names, now: NOW }));
  assert.doesNotMatch(met, /col-span/, 'a met goal stays in one cell');
  const failing = liClass(b.rowHtml({ goal: goal({ status: 'not_met' }), names, now: NOW }));
  assert.doesNotMatch(failing, /col-span/, 'a failing goal stays in one cell');
});

test('the Mission board tile reads "Waiting on you"; its hint still says these are board tasks', () => {
  const html = b.insightsHtml({ insights: { verified_7d: 0, false_success_7d: 0, needs_you_now: 2, blocked_now: 0, blocked_minutes_7d: 0, interventions_7d: 0, tokens_7d: 0, tokens_per_verified_7d: null }, needsYou: 2 });
  assert.match(html, />Waiting on you<\/dt>/);
  assert.doesNotMatch(html, /Board tasks waiting on you/);
  assert.match(html, /Board tasks waiting on a question, approval or step from you\. Your Inbox can hold more\./);
});
