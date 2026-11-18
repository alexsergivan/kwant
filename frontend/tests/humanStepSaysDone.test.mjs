// 28j (autonomy/human-step-says-done/brief.md): once the person did a human step (sign in, CAPTCHA, a code in the
// bot's browser), or it settled another way, its row stops saying "Waiting for you: <reason>", in Details and in the
// "What <bot> did" trail. The reason stays exactly as the bot wrote it. Tests marked "fails on 60d3f90f" fail there.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { readableStepTitle } from './src/toolLabel';
  export { trailFrom } from './src/doneAway';
  export { parseActivityMeta } from './src/activity';
  export const steps = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&amp;/g, '&').replace(/&#x27;/g, "'").replace(/\s+/g, ' ').trim();

const REASON = 'Ben needs you to sign in to tiktok.com in the browser panel, then it continues.';
const HS = `Waiting for you: ${REASON}`;

test('a settled human step says what happened, reason kept as written (fails on 60d3f90f)', () => {
  assert.equal(m.readableStepTitle(HS, 'done'), `You did it: ${REASON}`);
  assert.equal(m.readableStepTitle(HS, 'resolved_auto'), `Cleared by itself: ${REASON}`);
  assert.equal(m.readableStepTitle(HS, 'declined'), `You said no: ${REASON}`);
  assert.equal(m.readableStepTitle(HS, 'timed_out'), `Asked you: ${REASON}`);
  assert.equal(m.readableStepTitle(HS, 'stopped'), `Asked you: ${REASON}`);
});

test('guard: an open human step, or one with no status, reads exactly as today', () => {
  for (const st of [undefined, 'waiting', 'running']) assert.equal(m.readableStepTitle(HS, st), HS, String(st));
});

test('guard: 28h approval words and other titles are unchanged', () => {
  const AP = 'Waiting for your approval: Running a command';
  assert.equal(m.readableStepTitle(AP, 'ok'), 'You said OK: running a command');
  assert.equal(m.readableStepTitle(AP, 'declined'), 'You said no: running a command');
  assert.equal(m.readableStepTitle(AP, 'stopped'), 'Asked for your OK: running a command');
  assert.equal(m.readableStepTitle(AP, 'done'), 'Waiting for your OK: running a command');
  for (const st of ['done', 'resolved_auto', 'timed_out']) {
    assert.equal(m.readableStepTitle('Running `echo Waiting for you: x`', st), 'Running `echo Waiting for you: x`', st);
    assert.equal(m.readableStepTitle('Using `web_search`', st), 'Searching the web', st);
  }
});

const T0 = '2026-11-17T18:00:00Z';
const hs = (id, status) => ({ id, kind: 'human_step', status, title: HS, tool: 'ask_human', ref_id: `h-${id}`, started_at: T0, ended_at: T0 });
const saved = (statuses) => ({ activity: { steps: statuses.map((st, i) => hs(`s${i}`, st)), dropped: 0 } });

test('the server word survives parsing for a human step only; status (icon) is as before (fails on 60d3f90f)', () => {
  const a = m.parseActivityMeta(saved(['done', 'resolved_auto', 'timed_out']));
  assert.deepEqual(a.steps.map((s) => [s.status, s.outcome]), [['done', undefined], ['ok', 'resolved_auto'], ['ok', 'timed_out']]);
  const tool = m.parseActivityMeta({ activity: { steps: [{ ...hs('t', 'weird'), kind: 'tool' }], dropped: 0 } });
  assert.equal(tool.steps[0].outcome, undefined);
});

test('Details: done, cleared, said no, timed out and stopped human steps (fails on 60d3f90f)', () => {
  const a = m.parseActivityMeta(saved(['done', 'resolved_auto', 'declined', 'timed_out', 'stopped']));
  const t = text(m.steps({ activity: a, idBase: 'r1', defaultOpen: true }));
  for (const w of ['You did it', 'Cleared by itself', 'You said no', 'Asked you']) assert.ok(t.includes(`${w}: ${REASON}`), w);
  assert.doesNotMatch(t, /Waiting for you/);
});

test('trail: a settled human step reads as Details does (fails on 60d3f90f)', () => {
  const msg = (st) => ({ id: 'm1', text: 'Done.', created_at: '2026-11-17T18:01:00Z', meta: saved([st]) });
  assert.equal(m.trailFrom(msg('done'), null)[0].what, `You did it: ${REASON}`);
  assert.equal(m.trailFrom(msg('resolved_auto'), null)[0].what, `Cleared by itself: ${REASON}`);
  assert.equal(m.trailFrom(msg('timed_out'), null)[0].what, `Asked you: ${REASON}`);
  assert.equal(m.trailFrom(msg('waiting'), null)[0].what, HS);
});
