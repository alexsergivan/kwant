// Fewer confirms (approval audit, cut 6): deleting a routine, skill or memory, rejecting a proposed
// skill, and deleting a Parallel Universe scenario take one click and offer Undo. The delete is only
// sent once the Undo window closes, so Undo never has to restore anything on the server.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const u = await loadBundled(`export * from './src/undoDelete';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { RoutineRow } from './src/components/RoutinesPanel';
  import { ActiveSkill, PendingSkill } from './src/components/SkillsPanel';
  import { MemoryRow } from './src/components/MemoryPanel';
  import { UndoNotice } from './src/components/UndoNotice';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const routine = r(RoutineRow);
  export const active = r(ActiveSkill);
  export const pending = r(PendingSkill);
  export const memory = r(MemoryRow);
  export const undo = r(UndoNotice);
`);

/** Timers that only move when told to. */
const fakeTimers = () => {
  let next = 1;
  const due = new Map();
  return {
    set: (fn, ms) => (due.set(next, { fn, ms }), next++),
    clear: (h) => due.delete(h),
    fire: () => {
      const all = [...due.values()];
      due.clear();
      all.forEach((t) => t.fn());
    },
    get count() {
      return due.size;
    },
    get lastMs() {
      return [...due.values()].at(-1)?.ms;
    },
  };
};
const settle = () => new Promise((r) => setImmediate(r));

test('a delete waits out the Undo window, then is sent once', async () => {
  const t = fakeTimers();
  const results = [];
  let sent = 0;
  const d = new u.DeferredDelete((err) => results.push(err), u.UNDO_MS, t);
  d.start(async () => sent++);
  assert.equal(d.pending, true);
  assert.equal(t.lastMs, u.UNDO_MS);
  assert.equal(sent, 0, 'nothing is sent while Undo is still possible');
  t.fire();
  await settle();
  assert.equal(sent, 1);
  assert.deepEqual(results, [null]);
  assert.equal(d.pending, false);
  await d.flush();
  assert.equal(sent, 1, 'never sent twice');
});

test('Undo cancels it and nothing reaches the server', async () => {
  const t = fakeTimers();
  let sent = 0;
  const d = new u.DeferredDelete(() => {}, u.UNDO_MS, t);
  d.start(async () => sent++);
  assert.equal(d.undo(), true);
  assert.equal(t.count, 0, 'the timer is gone');
  t.fire();
  await d.flush();
  assert.equal(sent, 0);
  assert.equal(d.undo(), false, 'nothing left to undo');
});

test('leaving the screen sends it at once; a failed delete reports why', async () => {
  const t = fakeTimers();
  const results = [];
  let sent = 0;
  const d = new u.DeferredDelete((err) => results.push(err), u.UNDO_MS, t);
  d.start(async () => sent++);
  await d.flush();
  assert.equal(sent, 1);
  assert.equal(t.count, 0);

  d.start(async () => {
    throw new Error('HTTP 500');
  });
  t.fire();
  await settle();
  assert.deepEqual(results, [null, 'HTTP 500']);
});

test('a second delete inside the window sends the first one, never drops it', async () => {
  // Rigel's repro: delete scenario A, open B within 6s, delete B. A used to be silently dropped.
  const sent = [];
  const timers = { set: (fn) => fn, clear: () => {} };
  const d = new u.DeferredDelete(() => {}, 6000, timers);
  d.start(async () => sent.push('A'));
  d.start(async () => sent.push('B'));
  await d.flush();
  assert.deepEqual(sent, ['A', 'B']);
});

test('the second delete keeps its own Undo window; a failed first one still reports', async () => {
  const t = fakeTimers();
  const results = [];
  const sent = [];
  const d = new u.DeferredDelete((err) => results.push(err), u.UNDO_MS, t);
  d.start(async () => {
    sent.push('A');
    throw new Error('HTTP 500');
  });
  d.start(async () => sent.push('B'));
  await settle();
  assert.deepEqual(sent, ['A'], 'A goes at once, B waits');
  assert.deepEqual(results, ['HTTP 500']);
  assert.equal(d.pending, true, 'B can still be undone');
  assert.equal(t.count, 1, 'only B is on a timer');
  assert.equal(d.undo(), true);
  t.fire();
  await settle();
  assert.deepEqual(sent, ['A'], 'undoing B sends nothing more');
});

test('scenario Undo says its past runs go too; each scenario delete drops only itself', () => {
  const s = readFileSync(join(root, 'src/components/ParallelUniverseView.tsx'), 'utf8');
  assert.match(s, /Deleted “\$\{removing\.title\}” and its past runs\./);
  assert.match(s, /await parallelApi\.remove\(s\.id\);[^]*filter\(\(x\) => x\.id !== s\.id\)/);
});

test('a deleted or rejected skill stays hidden after the window, not back until the list reloads', () => {
  const s = readFileSync(join(root, 'src/components/SkillsPanel.tsx'), 'utf8');
  assert.match(s, /if \(removal\.gone\) return null;/);
  assert.match(s, /if \(rejection\.gone\) return null;/);
});

const routine = {
  id: 'r1', kwant_id: 'k1', channel_id: null, name: 'Morning digest', instructions: 'x',
  schedule: { type: 'daily', time: '09:00', tz: 'UTC' }, enabled: true, next_run_at: null, last_run_at: null, last_status: null, created_at: '2026-10-20T08:00:00Z',
};
const skill = (over = {}) => ({
  id: 's1', kwant_id: 'k1', name: 'weekly-report', description: 'Friday report', body: 'steps', status: 'enabled', source: 'user', proposed_by: null,
  replaces_id: null, created_at: '2026-10-17T10:00:00Z', updated_at: '2026-10-17T10:00:00Z', reviewed_at: null, ...over,
});
const memory = { id: 'm1', text: 'Alex likes short replies', source: 'user', created_at: '2026-10-20T08:00:00Z', updated_at: '2026-10-20T08:00:00Z' };
const noop = () => {};
const none = async () => {};

test('routines, skills and memories show one plain Delete (or Reject), never a second "are you sure?"', () => {
  const r = ui.routine({ routine, placeLabel: 'Direct chat', tick: 0, onEdit: noop, onChanged: noop });
  assert.match(r, /Delete/);
  assert.doesNotMatch(r, /Delete\?/);
  const p = ui.pending({ skill: skill({ status: 'pending', source: 'agent' }) });
  assert.match(p, /Reject/);
  assert.doesNotMatch(p, /Reject it\?/);
  const m = ui.memory({ memory, onSave: none, onDelete: none });
  assert.match(m, /aria-label="Delete memory"/);
  assert.doesNotMatch(m, /Delete\?|Confirm delete/);
});

test('while Undo is possible the item is replaced by what happened and an Undo button', () => {
  const r = ui.routine({ routine, placeLabel: 'Direct chat', tick: 0, onEdit: noop, onChanged: noop, initialPending: true });
  assert.match(r, /Deleted “Morning digest”/);
  assert.match(r, />.*Undo<\/button>/);
  assert.match(r, /^<li/, 'still a list item, so the list stays valid');
  assert.doesNotMatch(r, /Run now/);
  assert.match(ui.active({ skill: skill(), initialPending: true }), /Deleted “weekly-report”[^]*Undo/);
  assert.match(ui.pending({ skill: skill({ status: 'pending' }), initialPending: true }), /Rejected “weekly-report”[^]*Undo/);
  assert.match(ui.memory({ memory, onSave: none, onDelete: none, initialPending: true }), /Memory deleted[^]*Undo/);
  const n = ui.undo({ text: 'Deleted “x”.', onUndo: noop });
  assert.match(n, /role="status"/, 'screen readers hear it');
});

test('the four cheap deletes lost their confirm; the irreversible ones kept theirs', () => {
  const src = (f) => readFileSync(join(root, 'src/components', f), 'utf8');
  for (const f of ['RoutinesPanel.tsx', 'SkillsPanel.tsx', 'MemoryPanel.tsx', 'ParallelUniverseView.tsx']) {
    const s = src(f);
    assert.doesNotMatch(s, /'Delete\?'|>Delete\?<|Delete it\?|Reject it\?|confirmDelete|confirmReject|aria-label="Delete scenario"/, f);
    assert.match(s, /useUndoableDelete/, `${f} uses Undo`);
  }
  // Group A stays as it was
  assert.match(src('DeleteBotDialog.tsx'), /alertdialog|confirm/i);
  assert.match(src('LetterView.tsx'), /confirmDelete/);
  assert.match(src('McpServersModal.tsx'), /confirmDelete/);
  assert.match(src('SavedPasswordsPanel.tsx'), /confirmDelete/);
  assert.match(src('SpaceSettings.tsx'), /setConfirmArchive/);
  assert.match(src('ApprovalsSetting.tsx'), /setConfirming/);
});
