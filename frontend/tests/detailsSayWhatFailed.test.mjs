// 26d (autonomy/details-say-what-failed/brief.md): Details and the reply say which step failed or didn't run,
// and a routine run's row says it waits on you, needed your approval, or had a step fail (26b's statuses).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

process.env.TZ = 'UTC';
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const o = await loadBundled(`
  export * from './src/replyOutcome';
  export { parseActivityMeta, activitySummary, statusLabel } from './src/activity';
  export { runRowState, applyRunWait, liveWaitOf, recordRunWait, subscribeRunWaits, runWaitsNow } from './src/routineRow';
  export { routineRunNote } from './src/spendStop';
`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ReplyMeta } from './src/components/ReplyMeta';
  import { ReplyCheckLine } from './src/components/ReplyCheckLine';
  import { SavedActivityList } from './src/components/ActivityList';
  import { ChatView } from './src/components/ChatView';
  import { RunRow } from './src/components/RoutinesPanel';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const meta = r(ReplyMeta);
  export const line = r(ReplyCheckLine);
  export const steps = r(SavedActivityList);
  export const chat = r(ChatView);
  export const run = r(RunRow);
`);

const T0 = '2026-11-17T04:00:00Z';
const step = (id, status, tool, extra = {}) => ({ id, kind: 'tool', status, title: `Calling ${tool}`, tool, started_at: T0, ...extra });
const UNANSWERED_DETAIL = 'Waited 5 minutes for your answer; nothing ran';

// The brief's fixtures
const FAILED_THEN_OK = [step('s1', 'failed', 'ha_call_service', { detail: 'entity not found' }), step('s2', 'ok', 'ha_call_service')];
const FAILED_ONLY = [step('s1', 'failed', 'ha_call_service', { detail: 'entity not found' })];
const UNANSWERED_ONLY = [step('s1', 'unanswered', 'ha_get_states', { detail: UNANSWERED_DETAIL })];
const UNANSWERED_PLUS_OK = [step('s1', 'unanswered', 'ha_get_states', { detail: UNANSWERED_DETAIL }), step('s2', 'ok', 'web_search')];

test('replyOutcome: the four fixtures, most specific first', () => {
  assert.deepEqual(o.replyOutcome(FAILED_THEN_OK), { kind: 'recovered', count: 1, stepId: 's1' });
  assert.deepEqual(o.replyOutcome(FAILED_ONLY), { kind: 'failed', stepId: 's1' });
  assert.deepEqual(o.replyOutcome(UNANSWERED_ONLY), { kind: 'unanswered', stepId: 's1' });
  assert.deepEqual(o.replyOutcome(UNANSWERED_PLUS_OK), { kind: 'unanswered', stepId: 's1' }, 'an ok step of another tool does not clear it');
  assert.equal(o.replyOutcome([step('s1', 'ok', 'a')]), null);
  assert.equal(o.replyOutcome([]), null);
  assert.equal(o.replyOutcome(null), null);
});

test('replyOutcome: recovery needs a later ok of the same tool', () => {
  assert.equal(o.replyOutcome([step('s1', 'ok', 'a'), step('s2', 'failed', 'a')]).kind, 'failed', 'an ok before the failure does not count');
  assert.equal(o.replyOutcome([step('s1', 'failed', 'a'), step('s2', 'ok', 'b')]).kind, 'failed', 'another tool does not count');
  assert.equal(o.replyOutcome([step('s1', 'failed', undefined), step('s2', 'ok', undefined)]).kind, 'failed', 'no tool name never recovers');
  assert.equal(o.replyOutcome([step('s1', 'failed', 'a'), step('s2', 'declined', 'a')]).kind, 'failed', 'declined is not a success');
  // Rigel's D1: an approved approval card for the same tool is permission, not the retry working
  const approved = { id: 's2', kind: 'approval', status: 'done', title: 'Allow Bash?', tool: 'Bash', started_at: T0 };
  assert.equal(o.replyOutcome([step('s1', 'failed', 'Bash'), approved]).kind, 'failed', 'an approved card alone does not recover');
  assert.equal(o.replyOutcome([step('s1', 'failed', 'Bash'), approved, step('s3', 'ok', 'Bash')]).kind, 'recovered', 'the approved retry that ran does');
  const two = o.replyOutcome([step('s1', 'failed', 'a'), step('s2', 'failed', 'a'), step('s3', 'ok', 'a')]);
  assert.deepEqual(two, { kind: 'recovered', count: 2, stepId: 's1' });
  // One recovered and one stuck: the stuck one wins and Details opens at it
  assert.deepEqual(o.replyOutcome([step('s1', 'failed', 'a'), step('s2', 'ok', 'a'), step('s3', 'failed', 'b')]), { kind: 'failed', stepId: 's3' });
  // Unanswered beats a stuck failure
  assert.equal(o.replyOutcome([step('s1', 'failed', 'a'), step('s2', 'unanswered', 'b')]).kind, 'unanswered');
});

test('detailsLabel and checkLine words', () => {
  assert.equal(o.detailsLabel(null), 'Details');
  assert.equal(o.detailsLabel({ kind: 'unanswered', stepId: 'x' }), 'Details · a step didn’t run (no answer)');
  assert.equal(o.detailsLabel({ kind: 'failed', stepId: 'x' }), 'Details · a step failed');
  assert.equal(o.detailsLabel({ kind: 'recovered', count: 1, stepId: 'x' }), 'Details · 1 step failed, then worked');
  assert.equal(o.detailsLabel({ kind: 'recovered', count: 3, stepId: 'x' }), 'Details · 3 steps failed, then worked');
  assert.equal(o.checkLine({ kind: 'unanswered', stepId: 'x' }), 'A step didn’t run, so check this before relying on it.');
  assert.equal(o.checkLine({ kind: 'failed', stepId: 'x' }), 'A step failed, so check this before relying on it.');
  assert.equal(o.checkLine({ kind: 'recovered', count: 1, stepId: 'x' }), null, 'not for "failed, then worked"');
  assert.equal(o.checkLine(null), null);
});

test('activity: unanswered and rehearsed parse as their own statuses, never ok', () => {
  const a = o.parseActivityMeta({ activity: { steps: [...UNANSWERED_PLUS_OK, step('s3', 'rehearsed', 'x')], dropped: 0 } });
  assert.deepEqual(a.steps.map((s) => s.status), ['unanswered', 'ok', 'rehearsed']);
  assert.equal(o.statusLabel(a.steps[0]), 'Not approved in time');
  assert.equal(o.statusLabel(a.steps[2]), 'Dry run');
  assert.equal(o.activitySummary(a), '3 steps · 1 didn’t run');
  assert.equal(o.activitySummary({ steps: FAILED_THEN_OK, dropped: 0 }), '2 steps · 1 failed');
});

test('ReplyMeta: the folded toggle names the outcome; open, it reads Details', () => {
  const html = (outcome) => ui.meta({ name: 'Ada', ranOn: 'Claude · Opus 5.5', outcome });
  assert.match(html({ kind: 'unanswered', stepId: 's1' }), /data-details-outcome="unanswered"[^>]*>Details · a step didn’t run \(no answer\)/);
  assert.match(html({ kind: 'unanswered', stepId: 's1' }), /text-amber-300/);
  assert.match(html({ kind: 'failed', stepId: 's1' }), /data-details-outcome="failed"[^>]*>Details · a step failed/);
  assert.match(html({ kind: 'failed', stepId: 's1' }), /text-rose-300/);
  const rec = html({ kind: 'recovered', count: 1, stepId: 's1' });
  assert.match(rec, />Details · 1 step failed, then worked/);
  assert.doesNotMatch(rec, /text-rose-300|text-amber-300/, '"failed, then worked" stays quiet');
  assert.match(html(null), />Details</);
  assert.doesNotMatch(html(null), /data-details-outcome/);
  // The old boolean still works for callers that pass it
  assert.match(ui.meta({ name: 'Ada', ranOn: 'Codex', failed: true }), /Details · a step failed/);
  // At 390 px the long label must not clip: the name line may wrap when it names an outcome, and the label stays whole
  const recovered = html({ kind: 'recovered', count: 1, stepId: 's1' });
  assert.match(recovered, /data-name-line-wraps=""/);
  assert.match(recovered, /class="[^"]*\bflex-wrap\b[^"]*"[^>]*data-name-line-wraps/);
  assert.doesNotMatch(recovered, /class="[^"]*flex-nowrap[^"]*"[^>]*data-name-line-wraps/);
  assert.match(recovered, /whitespace-nowrap[^"]*"[^>]*data-details-outcome/);
  assert.match(html(null), /flex-nowrap/, 'a plain Details keeps the one-row phone line');
  assert.doesNotMatch(html(null), /data-name-line-wraps/);
});

test('ReplyCheckLine: only for a step that didn\'t run or failed for good, with a Details button', () => {
  const un = ui.line({ outcome: { kind: 'unanswered', stepId: 's1' }, onDetails: () => {} });
  assert.match(un, /data-reply-check="unanswered"/);
  assert.match(un, /A step didn’t run, so check this before relying on it\. <button type="button"[^>]*>Details<\/button>/);
  assert.match(un, /aria-hidden="true"/, 'the icon is decorative');
  const failed = ui.line({ outcome: { kind: 'failed', stepId: 's1' }, onDetails: () => {} });
  assert.match(failed, /A step failed, so check this before relying on it\./);
  assert.equal(ui.line({ outcome: { kind: 'recovered', count: 1, stepId: 's1' }, onDetails: () => {} }), '');
  assert.equal(ui.line({ outcome: null, onDetails: () => {} }), '');
});

test('Steps list: a failed step keeps "Failed" and an unanswered one says "Not approved in time", even after a success', () => {
  const rec = ui.steps({ activity: { steps: FAILED_THEN_OK, dropped: 0 }, idBase: 'a', defaultOpen: true });
  assert.match(rec, /data-step-mark="failed"[^>]*>Failed</);
  assert.match(rec, /aria-label="Failed"/, 'the icon still names it for screen readers');
  // Mira's design check: the step Details lands on shows a visible ring, and its detail never splits a word on a phone
  assert.match(rec, /focus-visible:outline-sky-400[^"]*"[^>]*data-step-status|data-step-status="[^"]*"[^>]*class="[^"]*focus-visible:outline-sky-400/);
  assert.match(rec, /\[overflow-wrap:anywhere\]/);
  assert.doesNotMatch(rec, /break-all/);
  const un = ui.steps({ activity: { steps: UNANSWERED_PLUS_OK, dropped: 0 }, idBase: 'a', defaultOpen: true, focusStep: 's1' });
  assert.match(un, /data-step-mark="unanswered"[^>]*>Not approved in time</);
  assert.match(un, /aria-label="Not approved in time"/);
  assert.match(un, /<li id="a-steps-s1" tabindex="-1" data-step-status="unanswered"[^>]*><details class="group" open=""/, 'opens at the step with its detail shown');
  assert.match(un, /Waited 5 minutes for your answer; nothing ran/);
  assert.equal((un.match(/data-step-mark=/g) ?? []).length, 1, 'the ok step has no mark');
});

const chatProps = (messages) => ({
  title: 'Ada', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
});
const reply = (id, text, steps) => ({ id, sender_role: 'kwant', sender_id: 'ada', text, created_at: T0, meta: JSON.stringify({ activity: { steps, dropped: 0 } }) });

test('ChatView: label on the reply and the line under it, per fixture', () => {
  const html = ui.chat(chatProps([
    reply('m1', 'The living room lamp is now turned on.', FAILED_THEN_OK),
    reply('m2', 'The lamp is on.', FAILED_ONLY),
    reply('m3', 'I checked, no lights are on.', UNANSWERED_ONLY),
    reply('m4', 'No lights are on, and the weather is fine.', UNANSWERED_PLUS_OK),
  ]));
  const part = (id) => html.split('data-message-id="').find((c) => c.startsWith(`${id}"`)) ?? '';
  assert.match(part('m1'), /Details · 1 step failed, then worked/);
  assert.doesNotMatch(part('m1'), /data-reply-check/, 'no line under a reply whose step failed, then worked');
  assert.match(part('m2'), /Details · a step failed/);
  assert.match(part('m2'), /data-reply-check="failed"/);
  assert.match(part('m3'), /Details · a step didn’t run \(no answer\)/);
  assert.match(part('m3'), /data-reply-check="unanswered"/);
  assert.match(part('m4'), /data-reply-check="unanswered"/);
  // The line sits under the reply's words, never above them
  assert.ok(part('m3').indexOf('I checked, no lights are on.') < part('m3').indexOf('data-reply-check'));
  // The reply text itself is untouched
  assert.match(html, /The living room lamp is now turned on\./);
});

// ---------------------------------------------------------------- routine run row

const routine = { id: 'rt1', kwant_id: 'k1', channel_id: null, thread_id: null, name: 'Lights check' };
const runOf = (status, error = null, extra = {}) => ({ id: `run-${status}`, routine_id: 'rt1', trigger: 'schedule', started_at: T0, ended_at: null, status, error, message_id: null, ...extra });

test('runRowState: each run status, plus waiting', () => {
  assert.deepEqual(o.runRowState({ status: 'running' }, { routine_id: 'rt1', waiting: true, approval_id: 'ap1' }), { label: 'Waiting for you', tone: 'waiting', cardId: 'ap1' });
  assert.deepEqual(o.runRowState({ status: 'running' }, { routine_id: 'rt1', waiting: false, approval_id: 'ap1' }), { label: 'Running', tone: 'running', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'running' }), { label: 'Running', tone: 'running', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'needs_you' }), { label: 'Needed your approval, nothing ran', tone: 'needs_you', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'needs_you' }, { routine_id: 'rt1', waiting: false, approval_id: 'ap1' }).cardId, 'ap1', 'the card stays open, so the link stays');
  assert.deepEqual(o.runRowState({ status: 'incomplete' }), { label: 'A step failed', tone: 'error', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'ok' }), { label: 'OK', tone: 'ok', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'error' }), { label: 'Failed', tone: 'error', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'skipped' }), { label: 'Skipped', tone: 'skipped', cardId: null });
  assert.deepEqual(o.runRowState({ status: 'ok' }, { routine_id: 'rt1', waiting: true, approval_id: 'ap1' }).tone, 'ok', 'a finished run never reads waiting');
});

test('applyRunWait: 26b\'s event order for an unanswered card', () => {
  let w = {};
  w = o.applyRunWait(w, { routine_id: 'rt1', run_id: 'r1', status: 'running', waiting: false });
  assert.deepEqual(w, {}, 'a plain running event adds nothing');
  w = o.applyRunWait(w, { routine_id: 'rt1', run_id: 'r1', status: 'running', waiting: true, approval_id: 'ap1' });
  assert.deepEqual(w.r1, { routine_id: 'rt1', waiting: true, approval_id: 'ap1' });
  assert.deepEqual(o.liveWaitOf(w, 'rt1'), w.r1);
  w = o.applyRunWait(w, { routine_id: 'rt1', run_id: 'r1', status: 'running', waiting: false });
  assert.deepEqual(w.r1, { routine_id: 'rt1', waiting: false, approval_id: 'ap1' }, 'the card id outlives the wait');
  assert.equal(o.liveWaitOf(w, 'rt1'), null);
  const same = o.applyRunWait(w, { routine_id: 'rt1', run_id: 'r1', status: 'needs_you' });
  assert.equal(same, w, 'no change keeps the same object');
  assert.equal(o.applyRunWait(w, null), w);
  assert.equal(o.applyRunWait(w, { waiting: true }), w, 'no run id');
});

test('RunRow: waiting links to the card; needs_you and incomplete read in words', () => {
  const card = () => {};
  const waiting = ui.run({ routine, run: runOf('running'), wait: { routine_id: 'rt1', waiting: true, approval_id: 'ap1' }, onOpenRunCard: card });
  assert.match(waiting, /data-run-state="waiting"/);
  assert.match(waiting, />Waiting for you</);
  assert.match(waiting, /<button type="button"[^>]*aria-label="Open the approval card the run on [^"]* waits on"[^>]*data-run-card="ap1">Open the card/);
  assert.doesNotMatch(waiting, /animate-spin/, 'a wait does not spin');

  const needs = ui.run({ routine, run: runOf('needs_you', 'A step is waiting for your approval, so it didn\'t run'), onOpenRunCard: card });
  assert.match(needs, />Needed your approval, nothing ran</);
  assert.doesNotMatch(needs, /A step is waiting for your approval/, 'the badge says it; no repeat');
  assert.doesNotMatch(needs, /data-run-card/, 'no card id seen this session, so no link');

  const inc = ui.run({ routine, run: runOf('incomplete', 'exit 7: the report server said 502') });
  assert.match(inc, /data-run-badge="error"[^>]*>A step failed</);
  assert.match(inc, /text-rose-300\/90">exit 7: the report server said 502</, 'the run\'s error line under it');

  assert.match(ui.run({ routine, run: runOf('ok') }), />OK</);
});

test('routineRunNote: needs_you has no note; incomplete keeps its reason', () => {
  assert.equal(o.routineRunNote({ status: 'needs_you', error: 'A step is waiting for your approval, so it didn\'t run' }), null);
  assert.deepEqual(o.routineRunNote({ status: 'incomplete', error: 'Bash failed' }), { text: 'Bash failed', quiet: false });
});

test('run waits outlive the Routines panel (Rigel D2): App records them, the panel reads them when it opens', () => {
  let calls = 0;
  const off = o.subscribeRunWaits(() => calls++);
  o.recordRunWait({ routine_id: 'r9', run_id: 'run9', status: 'running', waiting: true, approval_id: 'ap9' });
  assert.equal(calls, 1);
  assert.equal(o.liveWaitOf(o.runWaitsNow(), 'r9')?.approval_id, 'ap9');
  const before = o.runWaitsNow();
  o.recordRunWait({ routine_id: 'r9', run_id: 'run9', status: 'running', waiting: true, approval_id: 'ap9' });
  assert.equal(o.runWaitsNow(), before, 'the same event changes nothing');
  assert.equal(calls, 1);
  off();
  o.recordRunWait({ routine_id: 'r9', run_id: 'run9', status: 'needs_you', waiting: false });
  assert.equal(calls, 1, 'unsubscribed');
  assert.equal(o.runRowState({ status: 'needs_you' }, o.runWaitsNow().run9).cardId, 'ap9', 'the card id outlives the wait');
});
