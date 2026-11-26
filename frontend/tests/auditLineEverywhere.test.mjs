// 16b (autonomy/audit-line-everywhere/brief.md): an audit reads the same everywhere, and Activity has
// no raw event names.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/missions'; export { auditTurnLine, auditShortLine } from './src/auditor'; export { stripSentence } from './src/workingStrip';`);
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
  const esc = (x) => x.replace(/"/g, '&quot;');
  assert.ok(html.includes(`data-scene-said="">${esc(line)}.</p>`), 'the polite region says the audit line (36d: no fixed aria-label)');
  // 36d design check fix 3: the bot's name is bright, as in A and B.
  assert.ok(html.includes(`data-scene-line=""><span class="font-medium text-zinc-50">Writer</span>${esc(line.slice('Writer'.length))}</p>`), 'line 1 is the audit line');
  assert.ok(html.includes('data-audit=""'));
  assert.doesNotMatch(html, /is working on it/);
  // 36d: "Open the task" sits in line 2, after the clock (design C).
  assert.match(html, /data-scene-meta=""><span class="tabular-nums"[^>]*>Working · [\d:]+<\/span><span class="text-zinc-600" aria-hidden="true">·<\/span><button type="button" class="kw-wa-open" data-open-audited-task="">Open the task<\/button>/);
  assert.match(html, /<svg class="kw-wa-scene"/, 'the scene (avatar, animation) stays');
});

test('an audit turn with no board: the line, no link', () => {
  const html = ui.scene({ name: 'Writer', audit: { line } });
  assert.doesNotMatch(html, /Open the task/);
});

test('a plain turn: the live bubble is unchanged', () => {
  const html = ui.scene({ name: 'Writer' });
  assert.match(html, /data-scene-line=""><span class="font-medium text-zinc-50">Writer<\/span> is getting started<\/p>/);
  assert.match(html, /data-scene-said="">Writer is getting started\.<\/p>/);
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

test('36d: while an audit turn works the strip stays away on desktop and phone; the scene carries the line and Open the task', () => {
  for (const phone of [false, true]) {
    assert.equal(ui.strip({ name: 'Writer', phone, audit: { line, onOpenTask: open } }), '', `phone=${phone}: no strip while working`);
    assert.equal(ui.strip({ name: 'Writer', phone }), '');
  }
  const ph = ui.scene({ name: 'Writer', phone: true, audit: { line, onOpenTask: open } });
  assert.match(ph, /data-open-audited-task="">Open the task<\/button>/);
  assert.match(ph, /data-working-elapsed=""/);
});

test('the phone scene says the short line; the region keeps the full line, and desktop shows the full line', () => {
  const short = m.auditShortLine(audit, names);
  assert.match(short, /^Double-checking “Weekly report”/);
  const esc = (x) => x.replace(/"/g, '&quot;');
  const phone = ui.scene({ name: 'Writer', phone: true, audit: { line, short, onOpenTask: open } });
  assert.ok(phone.includes(`data-scene-line="">${esc(short)}</p>`), 'phone text is the short line');
  assert.ok(phone.includes(`data-scene-said="">${esc(line)}.</p>`), 'the region says the full line');
  const desk = ui.scene({ name: 'Writer', audit: { line, short, onOpenTask: open } });
  assert.ok(desk.includes(`data-scene-line=""><span class="font-medium text-zinc-50">Writer</span>${esc(line.slice('Writer'.length))}</p>`), 'desktop text stays the full line');
  assert.match(read('src/App.tsx'), /short: auditShortLine\(auditHere, kwantNames\)/);
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
  assert.equal(r('kwant_task'), 'Not checked: it’s the app’s own task');
  assert.equal(r('round_limit'), 'Not checked: it was already checked the most times allowed');
  assert.equal(r('no_checkpoint'), 'Not checked: there was no checkpoint to re-run it from');
  assert.equal(r('nothing_to_reproduce'), 'Not checked: there was nothing to re-run');
  assert.equal(r('new_reason'), 'Not checked: the checker skipped it');
});

test('tier_stepped_down names the tier it drops to, or says one level when the tier is unknown', () => {
  const r = (d) => m.eventText(ev('tier_stepped_down', d));
  assert.equal(r({ from_tier: 'act', to_tier: 'sandbox' }), 'You rolled it back, so its owner drops to Sandbox on this kind of work');
  assert.equal(r({ from_tier: 'sandbox', to_tier: 'observe' }), 'You rolled it back, so its owner drops to Observe on this kind of work');
  assert.equal(r({ to_tier: 'mystery' }), 'You rolled it back, so its owner drops one level on this kind of work');
  assert.equal(r({}), 'You rolled it back, so its owner drops one level on this kind of work');
});

test('checkpoint_pruned is quiet', () => {
  assert.equal(m.isQuietEvent({ kind: 'checkpoint_pruned' }), true);
  assert.equal(m.isQuietEvent({ kind: 'lessons_held_out' }), true);
  assert.equal(m.isQuietEvent({ kind: 'audit_fresh_eyes' }), true);
});

// --- 3. The guard: every task event kind the server emits. The server owns the list (TASK_EVENT_KINDS in
// src/db/event_kinds.rs, 17d); tests/task_event_kinds_test.rs keeps this checked-in copy equal to it and fails
// when an emitter writes a kind that isn't on it.
const SERVER_KINDS = JSON.parse(read('src/generated/taskEventKinds.json'));

// Server kinds that still have no plain words. Each one is shown as "Kwant: <raw kind>" until eventText words it;
// then remove it here, and the guard below covers it. Empty since 19c; keep it so the next new kind lands here.
const NEEDS_COPY = [];

test('lead_review_asked: kwant asked the lead, by name when known', () => {
  const names = { lead1: 'Ada' };
  assert.equal(m.eventText(ev('lead_review_asked', { delegation_id: 'd1', lead_kwant_id: 'lead1' }), names), 'Kwant asked Ada to review the hand-off');
  assert.equal(m.eventText(ev('lead_review_asked', { lead_kwant_id: 'gone' }), names), 'Kwant asked the lead to review the hand-off');
  assert.equal(m.eventText(ev('lead_review_asked', {})), 'Kwant asked the lead to review the hand-off');
  assert.equal(m.eventText({ ...ev('lead_review_asked'), detail: null }), 'Kwant asked the lead to review the hand-off');
});

test('lead_review_stopped: the person stopped the summary, so the review was skipped (32o)', () => {
  assert.equal(m.eventText(ev('lead_review_stopped', { delegation_id: 'd1', lead_kwant_id: 'lead1' }), { lead1: 'Ada' }), 'Review skipped: you stopped the summary');
  assert.equal(m.eventText({ ...ev('lead_review_stopped'), detail: null }), 'Review skipped: you stopped the summary');
  assert.equal(m.isQuietEvent({ kind: 'lead_review_stopped' }), false);
});

test('spec: the bot wrote down what done means, with a count of points', () => {
  const bot = (detail) => ({ ...ev('spec', detail), actor: 'k1', actor_name: 'Lin' });
  assert.equal(m.eventText(bot({ lines: ['a', 'b', 'c'] })), 'Lin wrote down what done means (3 points)');
  assert.equal(m.eventText(bot({ lines: ['a'] })), 'Lin wrote down what done means (1 point)');
  assert.equal(m.eventText(bot({ lines: [] })), 'Lin wrote down what done means');
  assert.equal(m.eventText(bot({ lines: 'not a list' })), 'Lin wrote down what done means');
  assert.equal(m.eventText({ ...bot({}), detail: null }), 'Lin wrote down what done means');
  assert.equal(m.isQuietEvent({ kind: 'spec' }), false);
});

test('the server list is the checked-in JSON, and the kinds waiting for copy are on it and still raw', () => {
  assert.ok(Array.isArray(SERVER_KINDS) && SERVER_KINDS.length > 50, 'the list loaded');
  assert.deepEqual([...SERVER_KINDS].sort(), SERVER_KINDS, 'sorted');
  for (const kind of NEEDS_COPY) {
    assert.ok(SERVER_KINDS.includes(kind), `${kind} is a server kind`);
    const text = m.eventText(ev(kind, {}));
    assert.ok(new RegExp(`^Kwant: ${kind.replace(/_/g, ' ')}$`, 'i').test(text), `${kind} has words now (${JSON.stringify(text)}): remove it from NEEDS_COPY`);
  }
});

test('guard: no kind the server emits renders as "Kwant: <raw kind>"', () => {
  const raw = [];
  for (const kind of SERVER_KINDS) {
    if (NEEDS_COPY.includes(kind)) continue;
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
