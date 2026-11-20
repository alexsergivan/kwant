// 29x (autonomy/deny-reason-on-command-row/brief.md): when a declined card folds into its command row (28s, case A:
// the claude CLI), the reason the person typed with Deny (the card's detail, `You said why: “…”`, set by 29s) shows as
// a second line under "You said no; nothing ran", in the same box. Nothing is carried for an OK or unanswered card, or
// a declined card with no detail. Saved steps, the chip, the summary line and the counts stay as they were.
// Tests marked "fails on b9cacde3" fail on main there.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { parseActivityMeta, activitySummary, foldAnswers, DENIED_STEP_DETAIL } from './src/activity';
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const at = (min) => new Date(Date.parse('2026-11-19T06:00:00Z') + min * 60000).toISOString();
const AP = 'Waiting for your approval: Running a command';
const ap = (id, status, t, detail) => ({ id, kind: 'approval', status, title: AP, tool: 'Bash', started_at: at(t), ended_at: at(t + 0.2), duration_ms: 21000, ...(detail ? { detail } : {}) });
const tool = (id, status, title, t, detail) => ({ id, kind: 'tool', status, title, tool: 'Bash', started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000, ...(detail ? { detail } : {}) });
const parse = (raw) => m.parseActivityMeta({ activity: { steps: raw, dropped: 0 } });
const SHORT = 'You said why: “push without --force, it’s a shared branch”';
const LONG = 'You said why: “' + 'Please keep the shared branch as it is; '.repeat(13).slice(0, 483) + '…”';
const PUSH = 'Running `git push --force origin main`';
const ADJ = 'Running `git push origin feature`';
// Case A as the claude CLI leaves it (the failed denied step; readDenials reads it as the person's no) and as 28b saves it.
const caseA = (reason, shape = 'cli') => [
  shape === 'cli' ? tool('t1', 'failed', PUSH, 0, 'The user denied this tool call.') : tool('t1', 'declined', PUSH, 0, 'You said no; nothing ran'),
  ap('k1', 'declined', 0.01, reason),
  ap('k2', 'ok', 1),
  tool('t2', 'ok', ADJ, 1.2),
];
const html = (raw) => m.saved({ activity: parse(raw), idBase: 'r1', defaultOpen: true });
const box = (h, id) => {
  const li = h.match(new RegExp(`<li id="r1-steps-${id}"[\\s\\S]*?</li>`));
  assert.ok(li, `row ${id} drawn`);
  const pre = li[0].match(/<pre class="([^"]*)">([\s\S]*?)<\/pre>/);
  return { li: li[0], cls: pre?.[1], text: pre?.[2] };
};
const unescape = (s) => s.replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>');

for (const shape of ['cli', 'saved-28b'])
  test(`case A (${shape}): the reason is the second line under "You said no; nothing ran", in the same box (fails on b9cacde3)`, () => {
    const r = box(html(caseA(SHORT, shape)), 't1');
    assert.equal(unescape(r.text), `You said no; nothing ran\n${SHORT}`);
    assert.equal((r.li.match(/<pre/g) || []).length, 1, 'one box, not a second one');
    for (const c of ['font-sans', 'text-xs', 'whitespace-pre-wrap', '[overflow-wrap:anywhere]', 'max-h-40', 'overflow-y-auto', 'text-zinc-300', 'bg-zinc-950/80', 'border-zinc-800'])
      assert.ok(r.cls.split(' ').includes(c), `box keeps ${c}`);
    assert.ok(!r.cls.includes('font-mono'), 'the no is a sentence, not mono');
  });

test('the collapsed row stays the same: chip "You said no", time, chevron; the reason is not in the summary', () => {
  const withR = box(html(caseA(SHORT)), 't1');
  const without = box(html(caseA(undefined)), 't1');
  const summary = (li) => li.match(/<summary[\s\S]*?<\/summary>/)[0];
  assert.equal(summary(withR.li), summary(without.li), 'summary markup identical with and without a reason');
  assert.match(summary(withR.li), /data-step-mark="declined"[^>]*>You said no</);
  assert.ok(!summary(withR.li).includes('You said why'));
});

test('a 500-char reason stays whole in the box, closing quote kept (fails on b9cacde3)', () => {
  assert.equal(LONG.length, 500);
  assert.ok(LONG.endsWith('…”'));
  const r = box(html(caseA(LONG)), 't1');
  assert.equal(unescape(r.text), `You said no; nothing ran\n${LONG}`);
});

test('a declined card with no detail adds nothing: the row reads exactly as today', () => {
  const r = box(html(caseA(undefined)), 't1');
  assert.equal(unescape(r.text), 'You said no; nothing ran');
  assert.equal(m.foldAnswers(parse(caseA(undefined)).steps).reasons?.size ?? 0, 0);
});

test('OK and unanswered cards carry nothing onto their command rows', () => {
  const LATE = 'Waited 5 minutes for your answer; nothing ran';
  const raw = [ap('k1', 'ok', 0, 'You said why: “should never show”'), tool('t1', 'ok', ADJ, 0.2), tool('t3', 'unanswered', PUSH, 2, LATE), ap('k3', 'unanswered', 2.01, LATE)];
  const f = m.foldAnswers(parse(raw).steps);
  assert.equal(f.answers.size, 2, 'both cards folded');
  assert.equal(f.reasons?.size ?? 0, 0);
  const h = html(raw);
  assert.ok(!h.includes('should never show'));
  assert.equal(unescape(box(h, 't3').text), LATE);
});

test('foldAnswers maps the command step id to the declined card’s detail (fails on b9cacde3)', () => {
  const f = m.foldAnswers(parse(caseA(SHORT)).steps);
  assert.deepEqual([...f.reasons], [['t1', SHORT]]);
});

test('rows, the step count and "you said no to 1" are unchanged by a reason; saved steps untouched', () => {
  const a = parse(caseA(SHORT));
  const b = parse(caseA(undefined));
  assert.equal(m.foldAnswers(a.steps).rows.length, m.foldAnswers(b.steps).rows.length);
  assert.equal(m.foldAnswers(a.steps).rows.length, 2);
  assert.equal(m.activitySummary(a), m.activitySummary(b));
  assert.match(m.activitySummary(a), /2 steps · you said no to 1/);
  assert.equal(a.steps.find((s) => s.id === 't1').detail, 'You said no; nothing ran', 'saved command detail as readDenials makes it');
  assert.equal(a.steps.find((s) => s.id === 'k1').detail, SHORT, 'card detail kept');
});
