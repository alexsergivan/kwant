// 16b (autonomy/audit-line-everywhere/brief.md): an audit reads the same everywhere, and Activity has
// no raw event names.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/missions'; export { auditTurnLine } from './src/auditor'; export { stripSentence } from './src/workingStrip';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { WorkingAvatar } from './src/components/WorkingAvatar';
  import { WorkingStrip } from './src/components/WorkingStrip';
  export const scene = (props) => renderToStaticMarkup(React.createElement(WorkingAvatar, props));
  export const strip = (props) => renderToStaticMarkup(React.createElement(WorkingStrip, { active: true, title: 'Writer', ...props }));
`);
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');

const names = { w: 'Writer', r: 'Researcher' };
const audit = { audit_id: 'a1', task_id: 't1', task_title: 'Routine: Weekly report', source_kind: 'routine', trigger: 'unchecked', stage: 'challenge', challenger_kwant_id: 'w', owner_kwant_id: 'r' };
const line = m.auditTurnLine(audit, names);
const open = () => {};

// --- 1. The live line

test('an audit turn: the live bubble says the audit line and links to the task, not "is working on it"', () => {
  assert.match(line, /^Writer is double-checking Researcher’s routine run/);
  const html = ui.scene({ name: 'Writer', audit: { line, onOpenTask: open } });
  assert.ok(html.includes(`aria-label="${line.replace(/"/g, '&quot;')}"`), 'the status label is the audit line');
  assert.ok(html.includes('data-audit=""'));
  assert.doesNotMatch(html, /is working on it/);
  assert.match(html, /<button type="button" class="kw-wa-open" data-open-audited-task="">Open the task<\/button>/);
  assert.match(html, /<svg class="kw-wa-scene"/, 'the scene (avatar, animation) stays');
});

test('an audit turn with no board: the line, no link', () => {
  const html = ui.scene({ name: 'Writer', audit: { line } });
  assert.doesNotMatch(html, /Open the task/);
});

test('a plain turn: the live bubble is unchanged', () => {
  const html = ui.scene({ name: 'Writer' });
  assert.match(html, /<span class="kw-wa-name">Writer<\/span> is working on it/);
  assert.match(html, /aria-label="Writer is working on the answer"/);
  assert.doesNotMatch(html, /data-audit|Open the task/);
});

test('the strip: the audit line replaces "getting started" / "working on it", never a step, the reply or a wait', () => {
  assert.deepEqual(m.stripSentence('Writer', {}, line), { audit: true, text: line });
  assert.deepEqual(m.stripSentence('Writer', { live: { steps: [{ id: 's', kind: 'tool', status: 'done', title: 'x', started_at: '2026-11-15T03:00:00Z' }] } }, line), { audit: true, text: line });
  assert.deepEqual(m.stripSentence('Writer', { writing: true }, line), { audit: false, text: 'Writer is writing the reply' });
  assert.deepEqual(m.stripSentence('Writer', { queued: true }, line), { audit: false, text: 'Writer is waiting in line' });
  assert.deepEqual(m.stripSentence('Writer', { awaitingAnswer: true }, line), { audit: false, text: 'Writer is waiting for your answer' });
  assert.deepEqual(m.stripSentence('Writer', {}, null), { audit: false, text: 'Writer is getting started' });
});

test('the strip on desktop and phone: the audit line, with "Open the task" on the phone (its live bubble is hidden)', () => {
  const desk = ui.strip({ name: 'Writer', phone: false, audit: { line, onOpenTask: open } });
  assert.ok(desk.includes('data-strip-audit=""'));
  assert.doesNotMatch(desk, /is getting started/);
  const phone = ui.strip({ name: 'Writer', phone: true, audit: { line, onOpenTask: open } });
  assert.match(phone, /data-open-audited-task=""[^>]*>Open the task<\/button>/);
  assert.match(phone, /data-working-elapsed=""/);
  assert.match(phone, /hidden min-\[400px\]:inline/, 'below 400 px the clock gives way to the link');
  const plain = ui.strip({ name: 'Writer', phone: true });
  assert.match(plain, /<span class="font-medium text-zinc-300">Writer<\/span> is getting started/);
  assert.doesNotMatch(plain, /Open the task|data-strip-audit/);
});

test('App finds the open chat’s running audit turn and opens its task on the board', () => {
  const app = read('src/App.tsx');
  assert.match(app, /t\.context_id === openCtx && t\.state === 'running' && t\.audit/);
  assert.match(app, /line: auditTurnLine\(auditHere, kwantNames\)/);
  assert.match(app, /setPendingTaskId\(auditHere\.task_id\);\s*showPage\('board'\);/);
  assert.match(app, /liveAudit=\{liveAudit\}/);
  const chat = read('src/components/ChatView.tsx');
  assert.equal((chat.match(/audit=\{liveAudit\}/g) ?? []).length, 2, 'the bubble and the strip');
});

// --- 2. Activity in plain words

const ev = (kind, detail = {}, extra = {}) => ({ id: 1, task_id: 't', kind, actor: 'kwant', actor_name: null, detail, created_at: '2026-11-15T03:00:00Z', ...extra });

test('checkpoint', () => {
  assert.equal(m.eventText(ev('checkpoint', { checkpoint_id: 'c', kind: 'after_run', files: 12, bytes: 40960, skipped: 0 })), 'Kwant saved a checkpoint after the run (12 files, 40 KB)');
  assert.equal(m.eventText(ev('checkpoint', { kind: 'before_run', files: 1, bytes: 10, skipped: 2 })), 'Kwant saved a checkpoint before the run (1 file, 10 B, 2 files left out)');
  assert.equal(m.eventText(ev('checkpoint', { kind: 'manual' }, { actor: 'person' })), 'You saved a checkpoint');
});

test('checkpoint_failed', () => {
  assert.equal(m.eventText(ev('checkpoint_failed', { kind: 'before_run', error: 'the folder is gone' })), 'Kwant couldn’t save a checkpoint before the run: the folder is gone');
  assert.equal(m.eventText(ev('checkpoint_failed', {})), 'Kwant couldn’t save a checkpoint');
});

test('verifier_started', () => {
  assert.equal(m.eventText(ev('verifier_started', { check_id: 'k', round: 1, risk_tier: 'low' })), 'The checker started');
  assert.equal(m.eventText(ev('verifier_started', { round: 2 })), 'The checker started round 2');
});

test('verifier_turn', () => {
  assert.equal(m.eventText(ev('verifier_turn', { kwant_id: 'w', difference: 'model', pin: 'grok' }), names), 'Writer took on the check, on a different AI model');
  assert.equal(m.eventText(ev('verifier_turn', { kwant_id: 'w', difference: 'tier' }), names), 'Writer took on the check, on a different model size');
  assert.equal(m.eventText(ev('verifier_turn', { kwant_id: 'gone', difference: 'none' }), names), 'A teammate took on the check');
});

test('verifier_done', () => {
  assert.equal(m.eventText(ev('verifier_done', { verdict: 'pass', next: 'counted', findings: 0, couldnt_verify: [] })), 'The checker passed it');
  assert.equal(m.eventText(ev('verifier_done', { verdict: 'fail', next: 'sent_back', findings: 2, couldnt_verify: [] })), 'The checker found 2 problems, so it went back for a fix');
  assert.equal(m.eventText(ev('verifier_done', { verdict: 'fail', next: 'escalated', findings: 1, couldnt_verify: [] })), 'The checker found 1 problem, so it waits for you');
  assert.equal(m.eventText(ev('verifier_done', { verdict: 'needs_you', next: 'held_for_you', findings: 0, couldnt_verify: ['a', 'b'] })), 'The checker needs you to look at it. It couldn’t check 2 steps');
});

test('verifier_skipped: every reason the server gives', () => {
  const r = (reason) => m.eventText(ev('verifier_skipped', { reason }));
  assert.equal(r('off'), 'Not checked: the checker is off');
  assert.equal(r('kwant_task'), 'Not checked: kwant checked this one itself');
  assert.equal(r('round_limit'), 'Not checked: it was already checked the most times allowed');
  assert.equal(r('no_checkpoint'), 'Not checked: there was no checkpoint to re-run it from');
  assert.equal(r('nothing_to_reproduce'), 'Not checked: there was nothing to re-run');
  assert.equal(r('new_reason'), 'Not checked: the checker skipped it');
});

test('checkpoint_pruned is quiet', () => {
  assert.equal(m.isQuietEvent({ kind: 'checkpoint_pruned' }), true);
  assert.equal(m.isQuietEvent({ kind: 'lessons_held_out' }), true);
  assert.equal(m.isQuietEvent({ kind: 'audit_fresh_eyes' }), true);
});

// --- 3. The guard: every task event kind the server emits (grep of src/**/*.rs, Nov 15 2026: insert_event,
// log_task_event, task_event, auditor/verifier event(), TaskChange kinds and literal INSERTs). Add a kind here
// when the server starts emitting one.
const SERVER_KINDS = [
  'audit_challenge', 'audit_done', 'audit_fresh_eyes', 'audit_rebuttal', 'audit_skipped', 'audit_started',
  'autofix_finished', 'autofix_held', 'autofix_scheduled', 'autofix_started',
  'check_added', 'check_result', 'check_started', 'check_wait_more',
  'checkpoint', 'checkpoint_failed', 'checkpoint_pruned',
  'claimed', 'class_changed', 'contract_updated', 'created', 'edited', 'evidence', 'failure',
  'flag_resolved', 'flagged', 'forked', 'handoff_ended', 'held', 'lease_expired',
  'lessons_held_out', 'lessons_shown', 'note', 'released', 'restored', 'review', 'risk_resolved',
  'self_checked', 'skill_drafted_from', 'skipped', 'state', 'tier_stepped_down',
  'verdict_overturned', 'verifier_done', 'verifier_skipped', 'verifier_started', 'verifier_turn', 'watch',
  'workflow_change_applied', 'workflow_change_decided', 'workflow_change_proposed',
];

test('guard: no kind the server emits renders as "Kwant: <raw kind>"', () => {
  const raw = [];
  for (const kind of SERVER_KINDS) {
    const e = ev(kind, {}, { to_state: 'ready' });
    if (m.isQuietEvent(e)) continue;
    const text = m.eventText(e);
    const humanized = kind.replace(/_/g, ' ');
    if (!text || new RegExp(`^Kwant: ${humanized}$`, 'i').test(text)) raw.push(`${kind} -> ${JSON.stringify(text)}`);
  }
  assert.deepEqual(raw, []);
  // The fallback itself still works for a kind nobody knows yet.
  assert.equal(m.eventText(ev('brand_new_kind')), 'Kwant: Brand new kind');
});

// --- 4. The Live now chip

test('Live now chip: full width up to 320 px on a phone, 240 px from sm', () => {
  const board = read('src/components/MissionBoardView.tsx');
  assert.match(board, /flex items-center gap-2 w-full max-w-\[320px\] sm:w-\[240px\] min-h-\[44px\]/);
  assert.doesNotMatch(board, /w-\[220px\] sm:w-\[240px\]/);
});
