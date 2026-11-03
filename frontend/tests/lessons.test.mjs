// Self-improving bots, round 1: the Settings switch, the read-only "Lessons from earlier work" group on a
// bot's Memory tab, and the three Activity lines. Built to the contract in self-improve/plan.md.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { LessonsGroup } from './src/components/MemoryPanel';
  export const group = (p) => renderToStaticMarkup(React.createElement(LessonsGroup, p));
`);

const LINE = {
  key: 'check_failed:3fa2c1',
  source: 'check',
  text: '3 days ago, task "Draft the 1.4 release notes": the command check `cargo test --doc` failed after hand-in; fixed by AutoFix, then verified.',
  count: 3,
  last_at: 1762180000000,
  task_id: 't_ab12',
};

test('parseLessons: the contract answer gives its lines in order', () => {
  const second = { ...LINE, key: 'stall:9', source: 'stall', text: 'A turn stalled.', count: 1, task_id: 't_cd34' };
  const got = m.parseLessons({ enabled: true, window_days: 30, lines: [LINE, second] });
  assert.equal(got.length, 2);
  assert.deepEqual(got[0], { key: LINE.key, source: 'check', text: LINE.text, count: 3, last_at: 1762180000000, task_id: 't_ab12' });
  assert.equal(got[1].key, 'stall:9');
});

test('parseLessons: no lines is an empty list; the setting off or a wrong shape hides it', () => {
  assert.deepEqual(m.parseLessons({ enabled: true, window_days: 30, lines: [] }), []);
  assert.equal(m.parseLessons({ enabled: false, window_days: 30, lines: [LINE] }), null, 'off: show nothing');
  assert.equal(m.parseLessons(null), null);
  assert.equal(m.parseLessons({ error: 'not found' }), null);
  assert.equal(m.parseLessons({ enabled: true }), null);
});

test('parseLessons: drops lines without text, defaults count to 1, keeps unknown sources as null', () => {
  const got = m.parseLessons({ lines: [{ key: 'a', text: '   ' }, { key: 'b', text: 'Hello', source: 'weird', count: 'x' }, 7, null, { text: 'No task', count: 0 }] });
  assert.equal(got.length, 2);
  assert.deepEqual(got[0], { key: 'b', source: null, text: 'Hello', count: 1, last_at: null, task_id: null });
  assert.equal(got[1].count, 1);
  assert.equal(got[1].task_id, null);
});

test('missionsApi.lessons: a 404 from an older server hides the group, so does a network error', async () => {
  const real = globalThis.fetch;
  try {
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    assert.equal(await m.missionsApi.lessons('k1'), null);
    globalThis.fetch = async () => {
      throw new TypeError('network');
    };
    assert.equal(await m.missionsApi.lessons('k1'), null);
    let url = '';
    globalThis.fetch = async (u) => {
      url = String(u);
      return new Response(JSON.stringify({ enabled: true, window_days: 30, lines: [LINE] }), { status: 200 });
    };
    const lines = await m.missionsApi.lessons('k 1');
    assert.equal(lines.length, 1);
    assert.equal(url, '/api/kwants/k%201/lessons');
  } finally {
    globalThis.fetch = real;
  }
});

test('parseLessonsSetting: on and off are read; an older server with no field gives null (switch hidden)', () => {
  assert.equal(m.parseLessonsSetting({ lessons: 'on' }), 'on');
  assert.equal(m.parseLessonsSetting({ lessons: 'off' }), 'off');
  assert.equal(m.parseLessonsSetting({}), null);
  assert.equal(m.parseLessonsSetting({ mission_board: true }), null);
  assert.equal(m.parseLessonsSetting({ lessons: 'measure' }), null);
  assert.equal(m.parseLessonsSetting({ lessons: true }), null);
  assert.equal(m.parseLessonsSetting(null), null);
});

const ev = (kind, detail = {}) => ({ id: 1, task_id: 't', kind, actor: 'kwant', actor_name: null, from_state: null, to_state: null, detail, created_at: 0 });

test('eventText: self_checked, lessons_shown and lessons_held_out', () => {
  assert.equal(m.eventText(ev('self_checked', { checks: 2 })), 'It ran its checks before hand-in');
  assert.equal(m.eventText(ev('lessons_shown', { keys: ['a', 'b', 'c'], bytes: 400 })), 'It started with 3 lessons from earlier work');
  assert.equal(m.eventText(ev('lessons_shown', { keys: ['a'], bytes: 90 })), 'It started with 1 lesson from earlier work');
  assert.equal(m.eventText(ev('lessons_shown', {})), 'It started with lessons from earlier work', 'no keys: no made-up number');
  assert.equal(m.eventText(ev('lessons_held_out', { keys: ['a'] })), '');
});

test('isQuietEvent: only the holdout is left out of Activity', () => {
  assert.equal(m.isQuietEvent(ev('lessons_held_out')), true);
  assert.equal(m.isQuietEvent(ev('lessons_shown')), false);
  assert.equal(m.isQuietEvent(ev('self_checked')), false);
});

test('LessonsGroup: text, ×N only above 1, a task link, the caption, and no action buttons beyond the links', () => {
  const html = ui.group({ kwantName: 'Ada', lines: [LINE, { ...LINE, key: 'b', count: 1, text: 'One-off.', task_id: 't_x' }], onOpenTask: () => {} });
  assert.match(html, /Lessons from earlier work/);
  assert.match(html, /Ada sees these before similar work\. They come from the last 30 days and update on their own\./);
  assert.match(html, /×3/);
  assert.equal((html.match(/×/g) ?? []).length, 1, 'count 1 shows no ×N');
  assert.equal((html.match(/>View task</g) ?? []).length, 2);
  assert.equal((html.match(/<button/g) ?? []).length, 2, 'only the task links are buttons');
  assert.doesNotMatch(html, /Delete|Edit|Pin|Dismiss/);
  assert.match(html, /cargo test --doc/);
});

test('LessonsGroup: hidden with no lines, while loading or when off/404 (null); no link without a task or without the board', () => {
  assert.equal(ui.group({ kwantName: 'Ada', lines: [] }), '');
  assert.equal(ui.group({ kwantName: 'Ada', lines: null }), '');
  const noBoard = ui.group({ kwantName: 'Ada', lines: [LINE] });
  assert.doesNotMatch(noBoard, /View task/);
  assert.match(noBoard, /×3/);
  const noTask = ui.group({ kwantName: 'Ada', lines: [{ ...LINE, task_id: null }], onOpenTask: () => {} });
  assert.doesNotMatch(noTask, /View task/);
});
