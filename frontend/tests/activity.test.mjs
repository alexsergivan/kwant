import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const a = await loadBundled(`export * from './src/activity';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { LiveActivityList, SavedActivityList } from './src/components/ActivityList';
  export const live = (props) => renderToStaticMarkup(React.createElement(LiveActivityList, props));
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const step = (over = {}) => ({ id: 's1', kind: 'tool', status: 'running', title: 'Running `cargo test`', started_at: '2026-10-22T10:00:00Z', ...over });

test('steps parse; unknown kinds draw as notes; junk is dropped', () => {
  assert.equal(a.parseStep(step()).kind, 'tool');
  assert.equal(a.parseStep(step({ kind: 'mystery', status: 'odd' })).kind, 'note');
  assert.equal(a.parseStep(step({ status: 'odd' })).status, 'ok');
  assert.equal(a.parseStep({ id: 1, title: 'x' }), null);
  assert.equal(a.parseStep(null), null);
});

test('meta.activity parses from a string or object', () => {
  const meta = { activity: { steps: [step({ status: 'ok' }), { bad: true }], dropped: 3 } };
  const got = a.parseActivityMeta(JSON.stringify(meta));
  assert.equal(got.steps.length, 1);
  assert.equal(got.dropped, 3);
  assert.equal(a.parseActivityMeta({ kind: 'ask_user' }), null);
  assert.equal(a.parseActivityMeta({ activity: { steps: [] } }), null);
  assert.equal(a.parseActivityMeta('nope'), null);
});

test('applyActivity replaces by id, appends new ones, never reopens a settled step, resets per turn', () => {
  let l = a.applyActivity(undefined, 't1', step());
  l = a.applyActivity(l, 't1', step({ id: 's2', title: 'Reading' }));
  assert.equal(l.steps.length, 2);
  l = a.applyActivity(l, 't1', step({ status: 'ok', duration_ms: 1200 }));
  assert.equal(l.steps[0].status, 'ok');
  assert.equal(a.applyActivity(l, 't1', step()).steps[0].status, 'ok', 'late running event loses');
  assert.equal(a.applyActivity(l, 't2', step({ id: 'x' })).steps.length, 1);
});

test('thinking chunks go to the running thinking step only', () => {
  let l = a.applyActivity(undefined, 't1', step({ id: 'th', kind: 'thinking', title: 'Thinking' }));
  l = a.appendThinking(l, 't1', 'First, ');
  l = a.appendThinking(l, 't1', 'check the tests.');
  assert.equal(l.thinking.th, 'First, check the tests.');
  assert.equal(a.appendThinking(l, 't2', 'x'), null, 'other turn');
  const done = a.applyActivity(l, 't1', step({ id: 'th', kind: 'thinking', status: 'done', text: 'Saved text' }));
  assert.equal(a.appendThinking(done, 't1', '⏹️ Cancelled by user'), null, 'a status line is not thinking');
  assert.equal(a.thinkingText(done.steps[0], done), 'Saved text');
  assert.equal(a.thinkingText(l.steps[0], l), 'First, check the tests.');
});

test('durations and the summary line', () => {
  assert.equal(a.formatDuration(840), '840 ms');
  assert.equal(a.formatDuration(8400), '8.4s');
  assert.equal(a.formatDuration(41000), '41s');
  assert.equal(a.formatDuration(125000), '2m 05s');
  const steps = [
    step({ id: 'a', status: 'ok', started_at: '2026-10-22T10:00:00Z', ended_at: '2026-10-22T10:00:10Z' }),
    step({ id: 'b', status: 'failed', started_at: '2026-10-22T10:00:10Z', ended_at: '2026-10-22T10:00:41Z' }),
    step({ id: 'n', kind: 'note', status: 'ok', title: 'This engine doesn’t share its thinking.' }),
  ];
  assert.equal(a.activitySummary({ steps, dropped: 10 }), '12 steps · 41s · 1 failed');
  assert.equal(a.activitySummary({ steps: [steps[0]], dropped: 0 }), '1 step · 10s');
  assert.equal(a.plainTitle('Running `ls`'), 'Running ls');
  assert.equal(a.statusLabel(step({ kind: 'approval', status: 'declined' })), 'Denied');
});

test('live list: steps, mono command, waits announced, reduced motion', () => {
  const l = {
    turnId: 't1',
    thinking: { th: 'Looking at the failing test' },
    steps: [
      step({ id: 'th', kind: 'thinking', title: 'Thinking' }),
      step({ id: 'ap', kind: 'approval', status: 'waiting', title: 'Approve `rm -rf build`' }),
    ],
  };
  const html = ui.live({ live: l, idBase: 'x' });
  assert.match(html, /Thinking…/);
  assert.match(html, /Looking at the failing test/, 'running thought is open, not cut to one line');
  assert.match(html, /<code[^>]*>rm -rf build<\/code>/);
  assert.match(html, /role="status"[^>]*>Approve rm -rf build</);
  assert.match(html, /motion-reduce:animate-none/);
  assert.match(html, /aria-label="Waiting for you"/);
  assert.equal(ui.live({ live: { turnId: 't', steps: [], thinking: {} }, idBase: 'x' }), '');
});

test('saved list starts collapsed to the summary; the engine note stays visible', () => {
  const activity = { steps: [step({ status: 'ok', duration_ms: 900 }), step({ id: 'n', kind: 'note', status: 'ok', title: 'This engine doesn’t share its thinking.' })], dropped: 0 };
  const html = ui.saved({ activity, idBase: 'm1' });
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /1 step/);
  assert.match(html, /doesn’t share its thinking/);
  assert.doesNotMatch(html, /cargo test/, 'steps hidden until opened');
  assert.equal(a.hasThinkingText(activity), false);
  assert.equal(a.hasThinkingText({ steps: [step({ kind: 'thinking', status: 'done', text: 'hm' })], dropped: 0 }), true);
});
