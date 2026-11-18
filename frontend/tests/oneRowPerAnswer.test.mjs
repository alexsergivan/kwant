// 28s (autonomy/one-row-per-answer/brief.md): in Details, a settled approval card folds into the command row of the
// same tool it answered (nearest, paired the way readDenials pairs), and that row carries the answer as its chip.
// Draw time only: saved steps and the summary line stay the same. Tests marked "fails on 5411bbd7" fail there.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { parseActivityMeta, statusLabel, activitySummary } from './src/activity';
  export const steps = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const at = (min) => new Date(Date.parse('2026-11-17T18:00:00Z') + min * 60000).toISOString();
const AP = 'Waiting for your approval: Running a command';
const ap = (id, status, t, tool = 'Bash', detail) => ({ id, kind: 'approval', status, title: AP, tool, started_at: at(t), ended_at: at(t + 0.2), duration_ms: 12000, ...(detail ? { detail } : {}) });
const tool = (id, status, title, t, detail, name = 'Bash') => ({ id, kind: 'tool', status, title, tool: name, started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000, ...(detail ? { detail } : {}) });
// `status` is the server's human-step status; parseStep reads it into status + outcome.
const hs = (id, status) => ({ id, kind: 'human_step', status, title: 'Waiting for you: Sign in to tiktok.com in the browser panel.', tool: 'ask_human', ref_id: `h-${id}`, started_at: at(0), ended_at: at(1), duration_ms: 60000 });
const LATE = 'Waited 5 minutes for your answer; nothing ran';
const parse = (raw) => m.parseActivityMeta({ activity: { steps: raw, dropped: 0 } });
const open = (activity) => m.steps({ activity, idBase: 'r1', defaultOpen: true });
const rowIds = (html) => [...html.matchAll(/<li id="r1-steps-([^"]+)"/g)].map((x) => x[1]);
const row = (html, id) => {
  const start = html.indexOf(`id="r1-steps-${id}"`);
  assert.ok(start >= 0, `row ${id}`);
  return html.slice(start, html.indexOf('</li>', start));
};
const chip = (r) => {
  const c = r.match(/<span( aria-hidden="true")? data-step-mark="([^"]+)" class="([^"]*)"[^>]*>([^<]+)</);
  return c && { hidden: !!c[1], mark: c[2], cls: c[3], text: c[4] };
};
const GREY = /text-zinc-300 border-zinc-500\/40/;
const AMBER = /text-amber-300 border-amber-500\/30/;

// The 28h rig's reply: allowed, denied (old saved shape, command above its card) and unanswered.
const RIG = () =>
  parse([
    ap('tk1', 'ok', 0),
    tool('t1', 'ok', 'Running `npm run build`', 0.2),
    tool('t2', 'failed', 'Running `rm -rf dist/old`', 1, 'The user denied this tool call.'),
    ap('tk2', 'declined', 1.01),
    tool('t3', 'unanswered', 'Running `git push origin main`', 2, LATE),
    ap('tk3', 'unanswered', 2.01, 'Bash', LATE),
  ]);

test('Deny: one row, the command, with the grey "You said no" chip (fails on 5411bbd7)', () => {
  const html = open(RIG());
  assert.ok(!rowIds(html).includes('tk2'), 'the card row folds away');
  const c = chip(row(html, 't2'));
  assert.equal(c.text, 'You said no');
  assert.match(c.cls, GREY);
  assert.match(row(html, 't2'), /aria-label="You said no"/);
  assert.doesNotMatch(html.replace(row(html, 't2'), ''), /You said no/, 'said on one row, not twice');
});

test('Allow: one row, the command, no visible chip; "You said OK" is read after the title (fails on 5411bbd7)', () => {
  const html = open(RIG());
  assert.ok(!rowIds(html).includes('tk1'));
  const r = row(html, 't1');
  assert.equal(chip(r), null, 'no visible chip (Mira 28s fix 1)');
  assert.doesNotMatch(r, /data-step-mark="ok"/);
  assert.match(r, /<span class="sr-only">You said OK<\/span>/);
  assert.match(r, /aria-label="Done"/);
});

test('unanswered with a command row: one amber row "Not approved in time" (fails on 5411bbd7)', () => {
  const html = open(RIG());
  assert.deepEqual(rowIds(html), ['t1', 't2', 't3']);
  const c = chip(row(html, 't3'));
  assert.equal(c.text, 'Not approved in time');
  assert.match(c.cls, AMBER);
});

test('unanswered with no command row: the card stays its own row, as before', () => {
  const html = open(parse([ap('tk3', 'unanswered', 2, 'Bash', LATE), tool('r1', 'ok', 'Reading `notes.md`', 3, undefined, 'Read')]));
  assert.deepEqual(rowIds(html), ['tk3', 'r1']);
  const c = chip(row(html, 'tk3'));
  assert.equal(c.text, 'Not approved in time');
  assert.match(c.cls, AMBER);
  assert.equal(chip(row(html, 'r1')), null, 'a call of another tool takes no answer');
});

test('a declined card with no command row stays, no chip, and its icon says "You said no", not "Denied" (fails on 5411bbd7)', () => {
  const html = open(parse([ap('tk2', 'declined', 1)]));
  assert.deepEqual(rowIds(html), ['tk2']);
  assert.equal(chip(row(html, 'tk2')), null);
  assert.match(row(html, 'tk2'), /aria-label="You said no"/);
  assert.doesNotMatch(html, /Denied/);
  assert.match(html, /You said no: running a command/i);
});

test('two calls of the same tool, one allowed and one denied: each command row gets its own answer (fails on 5411bbd7)', () => {
  const steps = parse([
    tool('a1', 'ok', 'Running `ls dist`', 0),
    ap('c1', 'ok', 0.01),
    tool('a2', 'failed', 'Running `rm -rf dist`', 1, 'The user denied this tool call.'),
    ap('c2', 'declined', 1.01),
  ]);
  const html = open(steps);
  assert.deepEqual(rowIds(html), ['a1', 'a2']);
  assert.equal(chip(row(html, 'a1')), null);
  assert.match(row(html, 'a1'), /sr-only">You said OK</);
  assert.equal(chip(row(html, 'a2')).text, 'You said no');
});

test('an allowed call that then failed keeps "Failed" as its chip, and the answer is read after it (fails on 5411bbd7)', () => {
  const html = open(parse([ap('c1', 'ok', 0), tool('a1', 'failed', 'Running `npm test`', 0.1, 'exit 1')]));
  assert.deepEqual(rowIds(html), ['a1']);
  const r = row(html, 'a1');
  assert.equal(chip(r).text, 'Failed');
  assert.match(r, /<span class="sr-only">You said OK<\/span>/);
});

test('an open card stays its own row; an approved call still running reads "You said OK" (fails on 5411bbd7)', () => {
  const waiting = open(parse([tool('a1', 'running', 'Running `npm ci`', 0), ap('c1', 'waiting', 0.01)]));
  assert.deepEqual(rowIds(waiting), ['a1', 'c1']);
  const running = open(parse([ap('c1', 'ok', 0), tool('a1', 'running', 'Running `npm ci`', 0.1)]));
  assert.deepEqual(rowIds(running), ['a1']);
  assert.match(row(running, 'a1'), /sr-only">You said OK</);
});

test('a declined human step has no chip: its title already says "You said no" (fails on 5411bbd7)', () => {
  const html = open(parse([hs('h1', 'declined'), hs('h2', 'timed_out')]));
  const r = row(html, 'h1');
  assert.equal(chip(r), null);
  assert.match(r, /aria-label="You said no"/);
  assert.match(r, /You said no: /);
  assert.equal(chip(row(html, 'h2')).text, 'Not done in time', '28n stays');
});

test('saved steps are unchanged by the fold; the summary is the one Details shows (28t counts the rows)', () => {
  const steps = RIG();
  assert.equal(steps.steps.length, 6);
  assert.deepEqual(steps.steps.map((s) => s.status), ['ok', 'ok', 'declined', 'declined', 'unanswered', 'unanswered']);
  const html = open(steps);
  assert.match(html, new RegExp(m.activitySummary(steps).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  assert.equal(m.activitySummary(steps), m.activitySummary(RIG()));
});
