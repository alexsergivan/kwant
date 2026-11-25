// 28t (autonomy/folded-pair-counts-once/brief.md): since 28s folds a settled approval card into its command row,
// the step count and "didn't run" say what Details draws: a folded card and its command count once. "you said no to"
// and "failed" counts stay as they were. Tests marked "fails on 44bd873d" fail there.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList, LiveActivityList } from './src/components/ActivityList';
  export { parseActivityMeta, activitySummary, foldAnswers } from './src/activity';
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
  export const live = (props) => renderToStaticMarkup(React.createElement(LiveActivityList, props));
`);

const at = (min) => new Date(Date.parse('2026-11-17T18:00:00Z') + min * 60000).toISOString();
const AP = 'Waiting for your approval: Running a command';
const ap = (id, status, t, tool = 'Bash', detail) => ({ id, kind: 'approval', status, title: AP, tool, started_at: at(t), ended_at: at(t + 0.2), duration_ms: 12000, ...(detail ? { detail } : {}) });
const tool = (id, status, title, t, detail, name = 'Bash') => ({ id, kind: 'tool', status, title, tool: name, started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000, ...(detail ? { detail } : {}) });
const hs = (id, status, t) => ({ id, kind: 'human_step', status, title: 'Waiting for you: Sign in to tiktok.com in the browser panel.', tool: 'ask_human', ref_id: `h-${id}`, started_at: at(t), ended_at: at(t + 1), duration_ms: 60000 });
const note = (id, t) => ({ id, kind: 'note', status: 'ok', title: 'Claude Code doesn’t share its thinking.', started_at: at(t), ended_at: at(t) });
const LATE = 'Waited 5 minutes for your answer; nothing ran';
const parse = (raw, dropped = 0) => m.parseActivityMeta({ activity: { steps: raw, dropped } });
const rows = (html) => [...html.matchAll(/<li id="r1-steps-/g)].length;

// The 28s rig's reply: the 28h calls (allowed, denied in the old saved shape, unanswered) and the 28j human steps.
const RIG_RAW = () => [
  ap('tk1', 'ok', 0),
  tool('t1', 'ok', 'Running `npm run build`', 0.2),
  tool('t2', 'failed', 'Running `rm -rf dist/old`', 1, 'The user denied this tool call.'),
  ap('tk2', 'declined', 1.01),
  tool('t3', 'unanswered', 'Running `git push origin main`', 2, LATE),
  ap('tk3', 'unanswered', 2.01, 'Bash', LATE),
  hs('h1', 'done', 8),
  hs('h2', 'declined', 10),
  hs('h3', 'timed_out', 12),
  hs('h4', 'done', 20),
  hs('h5', 'done', 25),
];

test('the 28s reply reads "8 steps · 1 didn’t run · you said no to 1", over 8 rows (fails on 44bd873d)', () => {
  const a = parse(RIG_RAW());
  assert.equal(a.steps.length, 11, 'saved steps are all kept');
  assert.equal(m.activitySummary(a), 'Worked 26:00 · 8 steps · 1 didn’t run · you said no to 1');
  const html = m.saved({ activity: a, idBase: 'r1', defaultOpen: true });
  assert.equal(rows(html), 8, 'Details draws 8 rows');
  assert.match(html.replace(/<[^>]+>/g, ''), /8 steps · 1 didn’t run · you said no to 1/);
});

test('a run with no approvals counts as before', () => {
  const a = parse([tool('t1', 'ok', 'Read a.ts', 0, undefined, 'Read'), tool('t2', 'failed', 'Running `npm test`', 1), note('n1', 1.5), tool('t3', 'ok', 'Edit a.ts', 2, undefined, 'Edit')]);
  // 36a: the edit after the failed test run shows the bot went on, so it reads "retried".
  assert.equal(m.activitySummary(a), 'Worked 2:30 · 3 steps · 1 retried', 'the note still doesn’t count');
});

test('an approval with no matching command still counts once; a command and its unanswered card count once (fails on 5195ef80)', () => {
  // kwnt held the call: nobody answered, nothing ran
  const held = parse([ap('tk1', 'unanswered', 0, 'Bash', LATE)]);
  assert.equal(m.activitySummary(held), 'Worked 0:12 · 1 step · 1 didn’t run');
  // a command nobody approved and its card are one row, one step and one "didn't run" (fails on 5195ef80)
  const pair = parse([tool('t1', 'unanswered', 'Running `git push origin main`', 0, LATE), ap('tk1', 'unanswered', 0.01, 'Bash', LATE)]);
  assert.equal(m.activitySummary(pair), 'Worked 0:30 · 1 step · 1 didn’t run');
  assert.equal(rows(m.saved({ activity: pair, idBase: 'r1', defaultOpen: true })), 1);
  // a card for another tool doesn't fold into this command
  const other = parse([ap('tk1', 'ok', 0, 'Write'), tool('t1', 'ok', 'Running `ls`', 0.2)]);
  assert.equal(m.activitySummary(other), 'Worked 0:42 · 2 steps');
});

test('dropped still adds, on top of the folded count (fails on 44bd873d)', () => {
  const a = parse([ap('tk1', 'ok', 0), tool('t1', 'ok', 'Running `npm run build`', 0.2)], 10);
  assert.equal(m.activitySummary(a), 'Worked 0:42 · 11 steps');
});

test('"didn’t run" counts the folded pair once; "you said no to" and "failed" are unchanged (fails on 5195ef80)', () => {
  const a = parse(RIG_RAW());
  const s = m.activitySummary(a);
  assert.match(s, /· 1 didn’t run ·/, 'the card and its command count once');
  assert.match(s, /you said no to 1$/);
  const failedAfterOk = parse([ap('tk1', 'ok', 0), tool('t1', 'failed', 'Running `npm test`', 0.2)]);
  assert.equal(m.activitySummary(failedAfterOk), 'Worked 0:42 · 1 step · 1 failed');
});

test('live "Working · N steps" uses the same count as Details (fails on 44bd873d)', () => {
  const raw = RIG_RAW().slice(0, 6);
  const steps = parse(raw).steps;
  const html = m.live({ live: { turnId: 't', steps, thinking: {} }, idBase: 'x', folded: true });
  assert.match(html, />Working · 3 steps</);
  // While the card waits it is one step; once allowed and the command runs, still one
  const waiting = parse([ap('tk1', 'waiting', 0)]).steps;
  assert.match(m.live({ live: { turnId: 't', steps: waiting, thinking: {} }, idBase: 'x' }), /· 1 step</);
  const running = parse([ap('tk1', 'ok', 0), { ...tool('t1', 'running', 'Running `npm run build`', 0.2), ended_at: undefined, duration_ms: undefined }]).steps;
  assert.match(m.live({ live: { turnId: 't', steps: running, thinking: {} }, idBase: 'x' }), /· 1 step</);
});
