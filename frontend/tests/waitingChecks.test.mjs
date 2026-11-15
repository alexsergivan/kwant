// Waiting checks (0.9.7, #564): a check that waits on an outside service shows as one line on the
// task card and in the task detail, "Waiting on <label> since 9:12 AM · next look 9:27 AM", plus the
// last line the script printed. Past its deadline the card moves to Needs you with a Wait N h more button.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const w = await loadBundled(`export * from './src/waitingChecks'; export { checksText, buildReceipt, checkStatusLabel, cardStatus, columnOfTask, stoppedWaiting, missionsApi, eventText } from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { TaskCard, EvidenceReceipt, TaskDetailBody } from './src/components/MissionBoardView';
  import { WaitingCheckLine } from './src/components/WaitingCheckLine';
  import { buildReceipt } from './src/missions';
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
  export const line = (p) => renderToStaticMarkup(React.createElement(WaitingCheckLine, p));
  export const detail = (p) => renderToStaticMarkup(React.createElement(TaskDetailBody, p));
  export const receipt = (d) => renderToStaticMarkup(React.createElement(EvidenceReceipt, { receipt: buildReceipt(d, {}), names: {} }));
`);

// Local times (no zone), so the clock reads the same in any time zone.
const NOW = Date.parse('2026-11-13T09:20:00');
const iso = (s) => new Date(Date.parse(s)).toISOString();
const check = (over = {}) => ({
  id: 'ch1', task_id: 't1', kind: 'command', spec: { command: './gate.sh', recheck_mins: 15, wait_hours: 72 }, required: true, approved: true,
  status: 'waiting', output_excerpt: 'run 2: not ready yet\nasking the store\njob 42: in_review (run 2)', checked_by: 'kwant', checked_by_name: null,
  requested_by: 'person', evidence_id: null, created_at: iso('2026-11-13T09:00:00'), started_at: iso('2026-11-13T09:12:00'),
  finished_at: iso('2026-11-13T09:12:05'), waiting_since: iso('2026-11-13T09:12:00'), next_run_at: iso('2026-11-13T09:27:00'),
  wait_until: iso('2026-11-16T09:12:00'), runs: 2, timed_out_at: null, ...over,
});
const task = (over = {}) => ({
  id: 't1', space_id: 's1', mission_id: 'm1', mission_title: 'Ship 1.2', title: 'Submit the app for review', acceptance: 'The store approves 1.2.',
  state: 'review', priority: 1, owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 50000, spent_tokens: 1200,
  attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1', channel_id: 'c1', direct_kwant_id: null,
  thread_id: null, created_by: 'person', created_at: iso('2026-11-13T09:00:00'), updated_at: iso('2026-11-13T09:12:05'), finished_at: null,
  depends_on: [], waiting_on: [], claim: null, needs_you: [],
  checks_summary: { status: 'pending', passed: 0, failed: 0, waiting: 1, needs_approval: 0, required_needs_approval: 0, total: 1 },
  ...over,
});

test('a waiting check reads as one line with its clock times, the deadline and the last printed line', () => {
  const l = w.waitLine(check(), NOW);
  assert.equal(w.waitLineText(l), 'Waiting on ./gate.sh since 9:12 AM · next look 9:27 AM');
  assert.equal(l.last, 'job 42: in_review (run 2)', "the script's last line, not kwant's run header");
  assert.match(l.deadline, /^If it isn’t ready by \S+ 9:12 AM, it stops and tells you\.$/, 'tells you, as in the 0.9.7 copy');
  assert.equal(l.timedOut, false);
});

test('a next look that is already due says soon; an http check waits on its address', () => {
  const l = w.waitLine(check({ next_run_at: iso('2026-11-13T09:15:00') }), NOW);
  assert.equal(w.waitLineText(l), 'Waiting on ./gate.sh since 9:12 AM · next look soon');
  const h = w.waitLine(check({ kind: 'http', spec: { url: 'https://status.example/jobs/42', waiting_status: 202 } }), NOW);
  assert.match(w.waitLineText(h), /^Waiting on https:\/\/status\.example\/jobs\/42 since/);
});

test('the last printed line skips the run header and is null when the script printed nothing', () => {
  assert.equal(w.lastPrinted('run 1: not ready yet\n'), null);
  assert.equal(w.lastPrinted('run 3: not ready yet\nfirst\n\nsecond  \n'), 'second');
  assert.equal(w.lastPrinted(null), null);
  assert.equal(w.waitLine(check({ output_excerpt: 'run 1: not ready yet' }), NOW).last, null);
});

test('past its deadline the card line says it did not answer, and the detail says No answer', () => {
  const l = w.waitLine(check({ status: 'error', next_run_at: null, timed_out_at: iso('2026-11-16T09:12:00') }), NOW);
  assert.equal(w.waitLineText(l), './gate.sh didn’t answer in 72 h.');
  assert.equal(l.hours, 72);
  assert.equal(w.detailLineText(l), 'No answer in 72 h.');
  assert.equal(w.detailLineText({ ...l, hours: null }), 'No answer in time.');
  assert.equal(w.detailLineText(w.waitLine(check(), NOW)), 'Waiting since 9:12 AM · next look 9:27 AM');
  const det = ui.line({ line: l });
  assert.ok(!det.includes('./gate.sh'), 'the detail row already shows the label');
  assert.ok(!det.includes('<svg'), 'no second warning icon in the detail');
  assert.ok(!det.includes('pl-4'), 'the last printed line lines up under the text');
  const card = ui.line({ line: l, compact: true });
  assert.ok(card.includes('<svg'), 'the card keeps its triangle');
  assert.ok(!card.includes('Verify the task by hand'));
  assert.equal(l.deadline, null);
  assert.equal(l.timedOut, true);
  assert.equal(w.waitLine(check({ status: 'error', timed_out_at: null }), NOW), null, 'an ordinary error is not a wait');
  assert.equal(w.waitLine(check({ status: 'passed' }), NOW), null);
});

test('long labels are cut on the card only', () => {
  const long = './scripts/' + 'a'.repeat(80) + '.sh';
  assert.equal([...w.shortLabel(long)].length, 48);
  assert.ok(w.shortLabel(long).endsWith('…'));
  const compact = ui.line({ line: w.waitLine(check({ spec: { command: long } }), NOW), compact: true });
  assert.ok(compact.includes(`title="${long}"`), 'the full label stays in the title');
  assert.ok(compact.includes(`Waiting on ${long} since`), 'and in the screen-reader sentence');
});

test('the line is labelled for screen readers and never renders HTML from the script', () => {
  const html = ui.line({ line: w.waitLine(check({ output_excerpt: 'run 1: not ready yet\n<img src=x onerror=alert(1)>' }), NOW) });
  assert.ok(!html.includes('<img'), 'output is text');
  assert.ok(html.includes('&lt;img'));
  assert.ok(html.includes('class="sr-only">Waiting since 9:12 AM, next look 9:27 AM<'));
  assert.ok(html.includes('>Waiting since 9:12 AM · next look 9:27 AM<'));
  assert.ok(!html.includes('text-zinc-500'), 'small grey text meets AA');
  assert.ok(ui.line({ line: w.waitLine(check(), NOW), compact: true }).includes('class="sr-only">Waiting on ./gate.sh since 9:12 AM · next look 9:27 AM<'));
  assert.ok(html.includes('It last printed: '));
  assert.ok(html.includes('data-waiting-deadline'));
  assert.ok(html.includes('aria-hidden="true"'));
});

test('the card says it waits before its checks load, and the chip counts waiting checks', () => {
  const html = ui.card({ task: task(), names: {}, now: NOW, onOpen: () => {} });
  assert.ok(html.includes('data-waiting-line="summary"'));
  assert.ok(html.includes('Waiting on an outside service'));
  assert.ok(html.includes('1 check waiting'));
  assert.equal(w.checksText(task().checks_summary).text, '1 check waiting');
  assert.equal(w.waitingFallback(2), 'Waiting on 2 outside services');
  const plain = ui.card({ task: task({ checks_summary: { status: 'pending', passed: 0, failed: 0, total: 1 } }), names: {}, now: NOW, onOpen: () => {} });
  assert.ok(!plain.includes('data-waiting-line'), 'an older server without the count shows nothing new');
  assert.ok(!ui.card({ task: task({ state: 'verified' }), names: {}, now: NOW, onOpen: () => {} }).includes('data-waiting-line'));
});

test('the detail receipt shows the waiting line in the check row, and a timed-out check as Stopped waiting', () => {
  const d = { task: task(), checks: [check()], evidence: [] };
  const html = ui.receipt(d);
  assert.ok(html.includes('data-check-status="waiting"'));
  assert.ok(html.includes('>Waiting<'), 'the status badge');
  assert.ok(html.includes('data-waiting-line="waiting"'));
  assert.ok(html.includes('job 42: in_review (run 2)'));
  const out = ui.receipt({ ...d, checks: [check({ status: 'error', timed_out_at: iso('2026-11-16T09:12:00') })] });
  assert.ok(out.includes('Stopped waiting'));
  assert.ok(!out.includes('Couldn’t run'));
  assert.ok(out.includes('data-waiting-line="timed-out"'));
});

test('the card looks again a minute after the earliest next look, never sooner than 30 s', () => {
  assert.equal(w.nextRefresh([check()], NOW), 8 * 60_000);
  assert.equal(w.nextRefresh([check({ next_run_at: iso('2026-11-13T09:00:00') })], NOW), 30_000);
  assert.equal(w.nextRefresh([check({ status: 'error' })], NOW), null);
});

const stopped = (over = {}) => task({
  needs_you: [{ id: 'ch1', kind: 'check_timed_out', text: './gate.sh didn’t answer in 72 h. Wait longer, or verify by hand.', check_label: './gate.sh', actions: ['wait_more', 'verify'], wait_more_hours: 24 }],
  ...over,
});

test('a waiting card says Bot says done; a stopped one moves to Needs you, amber, with its own chip', () => {
  assert.equal(w.cardStatus(task(), {}, NOW), 'Bot says done');
  assert.equal(w.columnOfTask(task()), 'review');
  const t = stopped();
  assert.equal(w.stoppedWaiting(t), 1);
  assert.equal(w.columnOfTask(t), 'needs_you');
  assert.equal(w.cardStatus(t, {}, NOW), 'Waiting for you: wait longer, or verify by hand');
  assert.deepEqual(w.checksText(t.checks_summary, 1), { text: '1 check stopped waiting', tone: 'amber' });
  assert.deepEqual(w.checksText({ ...t.checks_summary, waiting: 3 }, 2), { text: '2 checks stopped waiting', tone: 'amber' });
  const html = ui.card({ task: t, names: {}, now: NOW, onOpen: () => {} });
  assert.ok(/text-amber-200[^>]*>Waiting for you: wait longer/.test(html));
  assert.ok(html.includes('1 check stopped waiting'));
  assert.ok(!html.includes('1 check waiting'), 'the waiting count leaves the stopped one out');
});

test('the detail offers Wait 24 h more for a stopped check, 44 px on phones, labelled Stopped waiting', () => {
  let asked = null;
  const t = stopped();
  const p = { detail: { task: t, checks: [check({ status: 'error', timed_out_at: iso('2026-11-16T09:12:00') })], evidence: [], events: [] }, names: {}, now: NOW, onAction() {}, onApproveCheck() {}, onWaitMore: (id) => (asked = id) };
  const html = ui.detail(p);
  const btn = html.match(/<button[^>]*data-wait-more[^>]*>([\s\S]*?)<\/button>/);
  assert.ok(btn, 'the button');
  assert.ok(btn[0].includes('min-h-[44px] sm:min-h-[32px]'), '44 px on phones');
  assert.equal(btn[1].replace(/<!-- -->/g, ''), 'Wait 24 h more');
  const flat = html.replace(/<!-- -->/g, '');
  assert.ok(flat.includes('>Stopped waiting: <'), 'the row label');
  assert.ok(!flat.includes('Question: '));
  const six = ui.detail({ ...p, detail: { ...p.detail, task: stopped({ needs_you: [{ ...t.needs_you[0], wait_more_hours: 6 }] }) } });
  assert.ok(six.replace(/<!-- -->/g, '').includes('>Wait 6 h more<'));
  const noAction = ui.detail({ ...p, detail: { ...p.detail, task: stopped({ needs_you: [{ ...t.needs_you[0], actions: ['verify'] }] }) } });
  assert.ok(!noAction.includes('data-wait-more'), 'no button when the server does not offer it');
  assert.equal(typeof w.missionsApi.waitMore, 'function');
  assert.equal(asked, null);
});

// Round 3 (Rigel QA): the Activity trail must not say "A check failed" for each waiting poll, nor
// "A check couldn't run" when the check stops waiting. passed / failed / plain error read as before.
test('activity trail: waiting polls and the stop read as waiting, other results unchanged', () => {
  const ev = (detail) => ({ id: 1, task_id: 't1', mission_id: null, kind: 'check_result', actor: 'kwant', actor_name: null,
    from_state: null, to_state: null, detail, created_at: iso('2026-11-13T09:12:05') });
  assert.equal(w.eventText(ev({ status: 'waiting' })), 'A check is still waiting');
  assert.equal(w.eventText(ev({ status: 'error', timed_out: true })), 'A check stopped waiting');
  assert.equal(w.eventText(ev({ status: 'passed' })), 'A check passed');
  assert.equal(w.eventText(ev({ status: 'failed' })), 'A check failed');
  assert.equal(w.eventText(ev({ status: 'error' })), 'A check couldn’t run');
  assert.equal(w.eventText(ev({ status: 'error', timed_out: false })), 'A check couldn’t run');
});
