// Swarm Auditor UI (Phase D2, docs/mission-control.md §18): the task panel's "Open risks" lists each
// audit and each risk with its evidence and the owner's answer; only the person closes a risk, and
// after a 403 those buttons go. A live audit stalled on an approval card says so. "No risks raised"
// is never shown as a pass. Board cards count unresolved risks; Settings has the off-by-default switch.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const a = await loadBundled(`export * from './src/auditor';`);
const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { RisksSection } from './src/components/RisksSection';
  import { TaskCard, TaskDetailBody } from './src/components/MissionBoardView';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const risks = (p) => r(RisksSection, p);
  export const detail = (p) => r(TaskDetailBody, p);
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
`);

const names = { k_ann: 'Ann', k_arch: 'Architect' };
const noop = () => {};
const risk = (over = {}) => ({
  id: 'r1', audit_id: 'a1', task_id: 't1', seq: 1, risk: 'The yearly discount is not in the copy.', evidence: 'docs/pricing.md has no “yearly”.',
  evidence_ids: [], stance: null, answer: null, answer_evidence: null, status: 'open', resolved_by: null, resolved_at: null, note: null,
  created_at: '2026-10-29T10:00:00Z', ...over,
});
const audit = (over = {}) => ({
  id: 'a1', task_id: 't1', attempt: 1, trigger: 'high_impact', challenger_kwant_id: 'k_arch', owner_kwant_id: 'k_ann', status: 'done', reason: null,
  challenge_tokens: 12000, rebuttal_tokens: 3400, created_at: '2026-10-29T10:00:00Z', finished_at: '2026-10-29T10:05:00Z', risks: [risk()], ...over,
});
const data = (audits) => ({ audits, open_risks: audits.flatMap((x) => x.risks).filter(a.unresolved).length });
const approval = (over = {}) => ({
  kind: 'approval', id: 'ap1', kwant_id: 'k_arch', kwant_name: 'Architect', title: 'Use Bash', summary: '', status: 'waiting',
  timestamp: '2026-10-29T10:01:00Z', link: { thread_id: null, channel_id: null, direct_kwant_id: null }, ...over,
});

test('none: a task never audited shows no section at all', () => {
  assert.equal(ui.risks({ data: null, names }), '');
  assert.equal(ui.risks({ data: { audits: [], open_risks: 0 }, names }), '');
});

test('no risks raised reads as not a pass', () => {
  const html = ui.risks({ data: data([audit({ risks: [] })]), names });
  assert.match(html, /Architect raised no risks\. That isn’t a pass\./);
  assert.match(html, /“no risks raised” is not a pass/);
  assert.match(html, /data-open-risks="0"/);
  assert.doesNotMatch(html, /Verified|passed it/i);
});

test('open: risk, evidence, count, tokens and person-only buttons', () => {
  let got = null;
  const html = ui.risks({ data: data([audit()]), names, onResolve: (r, how) => (got = [r.id, how]) });
  assert.match(html, /Open risks/);
  assert.match(html, /data-open-risks="1"/);
  assert.match(html, /The yearly discount is not in the copy\./);
  assert.match(html, /Evidence: <\/span>docs\/pricing\.md/);
  assert.match(html, /Architect double-checked Ann’s task: it’s marked high priority</);
  assert.doesNotMatch(html, /Swarm Auditor|Audit by/);
  assert.match(html, /15\.4k tokens/);
  assert.match(html, /data-risk-status="open"/);
  assert.match(html, />Resolve</);
  assert.match(html, />Dismiss</);
  assert.equal(got, null);
});

test('disputed: shows the owner’s answer and evidence, and still counts as unresolved', () => {
  const r = risk({ stance: 'disputed', answer: 'It is in the FAQ block.', answer_evidence: 'docs/pricing.md line 40', status: 'disputed' });
  const html = ui.risks({ data: data([audit({ risks: [r] })]), names, onResolve: noop });
  assert.match(html, /Disputed/);
  assert.match(html, /Ann disputes it: <\/span>It is in the FAQ block\./);
  assert.match(html, /Their evidence: <\/span>docs\/pricing\.md line 40/);
  assert.match(html, /data-open-risks="1"/);
  assert.match(html, /data-risk-actions/);
  const accepted = ui.risks({ data: data([audit({ risks: [risk({ stance: 'accepted', answer: 'Fair, will fix.' })] })]), names });
  assert.match(accepted, /Ann accepts it: /);
});

test('resolved and dismissed: closed, no buttons, nothing open', () => {
  const html = ui.risks({
    data: data([audit({ risks: [risk({ status: 'resolved', note: 'Fixed in the next task' }), risk({ id: 'r2', status: 'dismissed' })] })]),
    names,
    onResolve: noop,
  });
  assert.match(html, /data-open-risks="0"/);
  assert.match(html, /You resolved it: Fixed in the next task/);
  assert.match(html, /You dismissed it\./);
  assert.doesNotMatch(html, /data-risk-actions/);
});

test('403: Resolve and Dismiss are hidden, the risks still show', () => {
  const html = ui.risks({ data: data([audit()]), names, canResolve: false, onResolve: noop });
  assert.match(html, /The yearly discount/);
  assert.doesNotMatch(html, /data-risk-actions/);
  assert.doesNotMatch(html, />Resolve</);
});

test('waiting: a live audit stalled on an approval card says so and links to it', () => {
  const live = audit({ status: 'challenge', risks: [], finished_at: null });
  const w = a.auditWaiting([live], [approval()]);
  assert.deepEqual(w, { audit_id: 'a1', kwant_id: 'k_arch', approval_id: 'ap1', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k_arch' } });
  const html = ui.risks({ data: data([live]), names, waiting: w, onOpenWaiting: noop });
  assert.match(html, /Audit waiting for your answer/);
  assert.match(html, /Architect asked for your OK\. The audit stays paused until you answer\./);
  assert.match(html, /data-open-audit-card/);
  // 14a: the heading already says who is double-checking, so the status line doesn't repeat it.
  assert.doesNotMatch(html, /is challenging the work/);
  assert.doesNotMatch(html, /data-audit-line/);
  assert.match(html, /data-audit-who="true">Architect is double-checking Ann’s task/);
  assert.deepEqual(a.auditCardPlace(w), { channelId: null, kwantId: 'k_arch', threadId: null });
});

test('waiting: only the bot whose turn it is, only cards asked since the audit began', () => {
  const challenge = audit({ status: 'challenge', risks: [] });
  assert.equal(a.auditWaiting([challenge], [approval({ kwant_id: 'k_ann' })]), null, 'owner card during the challenge');
  assert.equal(a.auditWaiting([challenge], [approval({ timestamp: '2026-10-29T09:00:00Z' })]), null, 'card from before the audit');
  assert.equal(a.auditWaiting([challenge], [approval({ kind: 'question' })]), null, 'questions are not approvals');
  assert.equal(a.auditWaiting([audit()], [approval()]), null, 'finished audits wait on nothing');
  const rebuttal = audit({ status: 'rebuttal' });
  const w = a.auditWaiting([rebuttal], [approval({ kwant_id: 'k_ann', link: { thread_id: 'th1', channel_id: null, direct_kwant_id: 'k_ann' } })]);
  assert.equal(w.kwant_id, 'k_ann');
  assert.deepEqual(a.auditCardPlace(w), { channelId: null, kwantId: 'k_ann', threadId: 'th1' });
  const inChannel = a.auditWaiting([rebuttal], [approval({ kwant_id: 'k_ann', link: { thread_id: null, channel_id: 'c1', direct_kwant_id: null } })]);
  assert.deepEqual(a.auditCardPlace(inChannel), { channelId: 'c1', kwantId: null, threadId: null });
});

test('audit lines for each status', () => {
  assert.match(a.auditStatusText(audit({ status: 'queued' }), names), /Architect will challenge it/);
  assert.match(a.auditStatusText(audit({ status: 'rebuttal' }), names), /Ann is answering/);
  assert.match(a.auditStatusText(audit({ risks: [risk(), risk({ id: 'r2' })] }), names), /raised 2 risks; Ann answered/);
  assert.match(a.auditStatusText(audit({ status: 'failed', reason: 'the challenger’s turn ended without a challenge' }), names), /Didn’t finish: the challenger/);
  assert.match(a.auditStatusText(audit({ status: 'interrupted' }), names), /restarted/);
  assert.equal(a.triggerLabel('manual'), 'started by you');
});

test('board badge counts only unresolved risks per task', () => {
  const counts = a.openRiskCounts([
    risk(),
    risk({ id: 'r2', status: 'disputed' }),
    risk({ id: 'r3', status: 'resolved' }),
    risk({ id: 'r4', task_id: 't2', status: 'dismissed' }),
    risk({ id: 'r5', task_id: 't3' }),
  ]);
  assert.deepEqual(counts, { t1: 2, t3: 1 });
  const t = {
    id: 't1', space_id: 's1', mission_id: null, mission_title: null, title: 'Write the copy', acceptance: '', state: 'review', priority: 2,
    owner_kwant_id: 'k_ann', owner_name: 'Ann', suggested_kwant_id: null, budget_tokens: 0, spent_tokens: 0, attempts: 1, max_attempts: 3, blocked_reason: null,
    source_kind: 'mission', source_id: 'm1', created_by: 'k_arch', finished_at: null, channel_id: null, direct_kwant_id: null, thread_id: null, depends_on: [], waiting_on: [], claim: null, needs_you: [],
    checks_summary: { status: 'not_checked', passed: 0, failed: 0, total: 0 }, created_at: '2026-10-29T09:00:00Z', updated_at: '2026-10-29T10:00:00Z',
  };
  assert.match(ui.card({ task: t, names, onOpen: noop, openRisks: 2 }), /data-risk-badge="2"[^]*2 open risks/);
  assert.match(ui.card({ task: t, names, onOpen: noop, openRisks: 1 }), /1 open risk</);
  assert.doesNotMatch(ui.card({ task: t, names, onOpen: noop }), /data-risk-badge/);
  // The task panel shows the section from the risks prop.
  const html = ui.detail({ detail: { task: t, checks: [], evidence: [], events: [] }, names, onAction: noop, onApproveCheck: noop, risks: data([audit()]), canResolveRisks: false });
  assert.match(html, /data-risks/);
  assert.doesNotMatch(html, /data-risk-actions/);
});

test('activity lines for audit events', () => {
  const ev = (kind, detail = {}, actor = 'kwant') => m.eventText({ id: 1, task_id: 't1', mission_id: null, kind, actor, actor_name: null, from_state: null, to_state: null, detail, created_at: '' }, names);
  // 14a: who double-checks whose work, and why; an event from before 14a has no owner.
  assert.equal(ev('audit_started', { challenger: 'k_arch' }), 'Architect started double-checking a teammate’s task');
  assert.equal(
    ev('audit_started', { challenger: 'k_arch', owner: 'k_ann', source_kind: 'routine', trigger: 'low_confidence' }),
    'Architect started double-checking Ann’s routine run: nothing else checks it',
  );
  assert.equal(ev('audit_challenge', { risks: 2 }, 'k_arch'), 'Architect raised 2 risks in the audit');
  assert.equal(ev('audit_done', { status: 'done' }), 'The audit finished. It doesn’t change whether the task passed');
  assert.equal(ev('audit_skipped', { reason: 'daily_cap' }), 'No audit: today’s limit was reached');
  const skipped = ev('audit_skipped', { reason: 'routine_recently_checked', last_audit_at: '2026-10-29T10:00:00Z', next_audit_from: '2026-11-05T10:00:00Z' });
  assert.match(skipped, /^No audit: this routine was double-checked on Oct 29, .+; next check from Nov 5, \d.+$/);
  // Same day: "today at", so it isn't any 5:03 PM.
  const now = Date.parse('2026-11-14T17:30:00Z');
  const today = a.routineSkipText('2026-11-14T17:03:00Z', '2026-11-21T17:03:00Z', now);
  assert.match(today, /^this routine was double-checked today at \d{1,2}:03\s?[AP]M; next check from Nov 21, \d{1,2}:03\s?[AP]M$/);
  assert.match(a.routineSkipText('2026-11-14T17:03:00Z', null, now), /^this routine was double-checked today at /);
  // The raw fresh-eyes event stays out of Activity; the audit's own label says it.
  assert.equal(ev('audit_fresh_eyes', { verdict: 'ok' }, 'k_arch'), '');
  assert.equal(ev('audit_skipped', { reason: 'routine_recently_checked' }), 'No audit: this routine was double-checked in the last week');
  assert.equal(ev('risk_resolved', { resolution: 'dismissed' }, 'person'), 'You dismissed a risk');
});

test('settings: off by default, cap clamped, hidden on servers without it', () => {
  assert.deepEqual(a.parseAuditorSettings({ auditor: 'off', auditor_daily_cap: 5 }), { auditor: 'off', auditor_daily_cap: 5 });
  assert.deepEqual(a.parseAuditorSettings({ auditor: 'on', auditor_daily_cap: 99 }), { auditor: 'on', auditor_daily_cap: 50 });
  assert.deepEqual(a.parseAuditorSettings({ auditor: 'on' }), { auditor: 'on', auditor_daily_cap: 5 });
  assert.equal(a.parseAuditorSettings({}), null);
  assert.match(a.SETTING_HINT, /never marks work passed or failed, and each audit uses tokens/);
  assert.match(a.SETTING_HINT, /Routine runs without a check are double-checked on their first run and then at most once a week\./);
  const src = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  assert.match(src, /id="auditor"/);
  assert.match(src, /\.\.\.\(auditor \?\? \{\}\)/, 'saved only when the server has it');
});

test('a 403 on resolve hides the buttons; resolving re-reads the risks', () => {
  const src = readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8');
  const fn = src.slice(src.indexOf('const resolveRisk = async'), src.indexOf('const decideChange ='));
  assert.match(fn, /status === 403\) setCanResolveRisks\(false\)/);
  assert.match(fn, /await loadRisks\(taskId\)/);
});

// ---------- 14a: who is double-checking whose work, and why ----------

test('the who-and-why line: routine run, high priority, hand-off, manual', () => {
  const routine = { challenger: 'Ben', owner: 'Ann', title: 'Routine: Daily digest', sourceKind: 'routine', trigger: 'low_confidence' };
  assert.equal(a.auditWhyLine(routine), 'Ben is double-checking Ann’s routine run “Daily digest”: nothing else checks it.');
  assert.equal(a.auditWhyLine({ ...routine, trigger: 'high_impact' }), 'Ben is double-checking Ann’s routine run “Daily digest”: it’s marked high priority.');
  assert.equal(a.auditWhyLine({ ...routine, sourceKind: 'delegation', title: 'Fix the footer' }), 'Ben is double-checking Ann’s hand-off “Fix the footer”: nothing else checks it.');
  assert.equal(a.auditWhyLine({ ...routine, sourceKind: 'person', title: 'Fix the footer', trigger: 'manual' }), 'Ben is double-checking Ann’s task “Fix the footer”: you asked for it.');
  assert.equal(a.auditWhyLine({ ...routine, trigger: 'something_new' }), 'Ben is double-checking Ann’s routine run “Daily digest”.');
  // Under the bot's own name the subject goes; in the log it is past.
  assert.equal(a.auditWhyLine(routine, { subject: false }), 'Double-checking Ann’s routine run “Daily digest”: nothing else checks it.');
  assert.equal(a.auditWhyLine(routine, { tense: 'started' }), 'Ben started double-checking Ann’s routine run “Daily digest”: nothing else checks it.');
  assert.equal(a.auditWhyLine(routine, { tense: 'done' }), 'Ben double-checked Ann’s routine run “Daily digest”: nothing else checks it.');
  // The owner's answer turn.
  assert.equal(a.auditWhyLine({ ...routine, stage: 'rebuttal' }), 'Ann is answering Ben’s double-check of the routine run “Daily digest”: nothing else checks it.');
  assert.equal(a.auditWhyLine({ ...routine, stage: 'rebuttal' }, { subject: false }), 'Answering Ben’s double-check of the routine run “Daily digest”: nothing else checks it.');
  // A name ending in s.
  assert.equal(a.auditWhyLine({ ...routine, owner: 'Atlas' }), 'Ben is double-checking Atlas’ routine run “Daily digest”: nothing else checks it.');
});

test('the who-and-why line with a missing name or title', () => {
  assert.equal(a.auditWhyLine({ challenger: 'Ben', owner: null, title: 'Routine: Daily digest', sourceKind: 'routine', trigger: 'low_confidence' }), 'Ben is double-checking a teammate’s routine run “Daily digest”: nothing else checks it.');
  assert.equal(a.auditWhyLine({ challenger: '  ', owner: 'Ann', sourceKind: 'routine', trigger: 'high_impact' }), 'A teammate is double-checking Ann’s routine run: it’s marked high priority.');
  assert.equal(a.auditWhyLine({ challenger: 'Ben', owner: undefined, stage: 'rebuttal', trigger: 'low_confidence' }), 'A teammate is answering Ben’s double-check of the task: nothing else checks it.');
  assert.equal(a.auditWork('routine', 'Routine:   '), 'routine run');
});

test('a running audit turn: names looked up, internals never shown', () => {
  const turn = { audit_id: 'a1', task_id: 't1', task_title: 'Routine: Daily digest', source_kind: 'routine', trigger: 'low_confidence', stage: 'challenge', challenger_kwant_id: 'k_arch', owner_kwant_id: 'k_ann' };
  const line = a.auditTurnLine(turn, names);
  assert.equal(line, 'Architect is double-checking Ann’s routine run “Daily digest”: nothing else checks it.');
  assert.doesNotMatch(line, /Swarm|Auditor|audit_|prompt/i);
  assert.equal(a.auditTurnLine({ ...turn, owner_kwant_id: 'k_gone' }, names), 'Architect is double-checking a teammate’s routine run “Daily digest”: nothing else checks it.');
});

test('the short line for compact places: the work first, no period', () => {
  const turn = { audit_id: 'a1', task_id: 't1', task_title: 'Routine: Weekly report', source_kind: 'routine', trigger: 'low_confidence', stage: 'challenge', challenger_kwant_id: 'k_arch', owner_kwant_id: 'k_ann' };
  // A title: just its name.
  assert.equal(a.auditShortLine(turn, names), 'Double-checking “Weekly report”');
  // No title, with an owner.
  assert.equal(a.auditShortLine({ ...turn, task_title: 'Routine: ' }, names), 'Double-checking Ann’s routine run');
  assert.equal(a.auditShortLine({ ...turn, task_title: '', source_kind: 'delegation' }, names), 'Double-checking Ann’s hand-off');
  // No title and no owner name.
  assert.equal(a.auditShortLine({ ...turn, task_title: '', owner_kwant_id: 'k_gone', source_kind: 'person' }, names), 'Double-checking a teammate’s task');
  // The owner's answer.
  assert.equal(a.auditShortLine({ ...turn, stage: 'rebuttal' }, names), 'Answering the double-check of “Weekly report”');
  for (const t of [turn, { ...turn, stage: 'rebuttal' }, { ...turn, task_title: '' }]) assert.doesNotMatch(a.auditShortLine(t, names), /\.$|Swarm|Auditor|audit_/);
});

test('the audit heading on its task follows the audit status', () => {
  const h = (status, sk) => a.auditHeaderLine(audit({ status, trigger: 'low_confidence' }), names, sk);
  assert.equal(h('queued', 'routine'), 'Architect will double-check Ann’s routine run: nothing else checks it.');
  assert.equal(h('challenge', 'routine'), 'Architect is double-checking Ann’s routine run: nothing else checks it.');
  assert.equal(h('rebuttal', 'routine'), 'Architect is double-checking Ann’s routine run: nothing else checks it.');
  assert.equal(h('done', 'routine'), 'Architect double-checked Ann’s routine run: nothing else checks it.');
  assert.equal(h('failed', null), 'Architect started double-checking Ann’s task: nothing else checks it.');
  const html = ui.risks({ data: data([audit({ status: 'challenge', trigger: 'low_confidence' })]), names, sourceKind: 'routine' });
  assert.match(html, /data-audit-who="true">Architect is double-checking Ann’s routine run: nothing else checks it</);
});
