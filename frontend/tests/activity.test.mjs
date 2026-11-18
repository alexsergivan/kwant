import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const a = await loadBundled(`export * from './src/activity';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { LiveActivityList, SavedActivityList, STEPS_ROW_BUTTON, STEPS_ROW_LIST } from './src/components/ActivityList';
  export { STEPS_ROW_BUTTON, STEPS_ROW_LIST };
  export const el = (t, p, c) => React.createElement(t, p, c);
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
  assert.equal(a.activitySummary({ steps, dropped: 10 }), 'Worked 0:41 · 12 steps · 1 failed');
  assert.equal(a.activitySummary({ steps: [steps[0]], dropped: 0 }), 'Worked 0:10 · 1 step');
  assert.equal(a.plainTitle('Running `ls`'), 'Running ls');
  assert.equal(a.statusLabel(step({ kind: 'approval', status: 'declined' })), 'You said no');
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

test('saved list starts collapsed to the summary; the engine note waits inside the steps', () => {
  const activity = { steps: [step({ status: 'ok', duration_ms: 900 }), step({ id: 'n', kind: 'note', status: 'ok', title: 'This engine doesn’t share its thinking.' })], dropped: 0 };
  const html = ui.saved({ activity, idBase: 'm1' });
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /1 step/);
  assert.doesNotMatch(html, /doesn’t share its thinking/, 'the note is a step, not a line under the reply');
  assert.doesNotMatch(html, /cargo test/, 'steps hidden until opened');
  assert.equal(a.hasThinkingText(activity), false);
  assert.equal(a.hasThinkingText({ steps: [step({ kind: 'thinking', status: 'done', text: 'hm' })], dropped: 0 }), true);
});

test('a page opened mid-turn seeds the open step from /api/turns (ticket 4)', async () => {
  const w = await loadBundled(`export * from './src/workingStrip';`);
  const fromServer = { steps: [step({ id: 'a', status: 'ok', title: 'Running `ls`' }), step({ id: 'b', title: 'Using `web_search`' })] };
  // Fresh load: nothing came live yet.
  const seeded = a.seedActivity(undefined, 't1', fromServer);
  assert.deepEqual(seeded.steps.map((s) => s.id), ['a', 'b']);
  assert.equal(w.workingLabel({ live: seeded }), 'searching the web');
  // Reconnect: a step that already settled live stays settled; live order is kept.
  const live = { turnId: 't1', thinking: { x: 'hm' }, steps: [step({ id: 'b', status: 'ok', title: 'Using `web_search`' })] };
  const merged = a.seedActivity(live, 't1', fromServer);
  assert.deepEqual(merged.steps.map((s) => [s.id, s.status]), [['b', 'ok'], ['a', 'ok']]);
  assert.equal(merged.thinking.x, 'hm');
  // `/api/turns` sends no thinking text: a thinking step that ended live keeps the text it had.
  const ended = { turnId: 't1', thinking: {}, steps: [step({ id: 'th', kind: 'thinking', status: 'ok', title: 'Thinking', text: 'saw it' })] };
  const reseeded = a.seedActivity(ended, 't1', { steps: [step({ id: 'th', kind: 'thinking', status: 'ok', title: 'Thinking' })] });
  assert.equal(reseeded.steps[0].text, 'saw it');
  // Nothing to add: no activity, junk, or no steps.
  assert.equal(a.seedActivity(undefined, 't1', undefined), null);
  assert.equal(a.seedActivity(undefined, 't1', { steps: [] }), null);
  assert.equal(a.seedActivity(undefined, 't1', 'nope'), null);
});

test('18f: the live card folds once the reply has words, unless a step waits for the person', () => {
  const ok = [step({ status: 'ok' }), step({ id: 's2', status: 'running' })];
  assert.equal(a.foldLiveSteps({ writing: false, steps: ok }), false, 'tool phase: full card');
  assert.equal(a.foldLiveSteps({ writing: true, steps: ok }), true, 'text streams: one row');
  assert.equal(a.foldLiveSteps({ writing: true, steps: [...ok, step({ id: 'ap', kind: 'approval', status: 'waiting', title: 'Approve' })] }), false, 'an ask never hides');
  assert.equal(a.foldLiveSteps({ writing: true, steps: [] }), true);
});

test('18f: the folded row is the saved row: same button classes, same wrapper, no time, list opens to 22rem', () => {
  const steps = [step({ status: 'ok', duration_ms: 900 }), step({ id: 's2', status: 'ok' }), step({ id: 's3', status: 'running', title: 'Reading `a.ts`' })];
  const html = ui.live({ live: { turnId: 't', steps, thinking: {} }, idBase: 'x', folded: true, lead: ui.el('span', { 'data-chip': '' }, 'chip') });
  const saved = ui.saved({ activity: { steps, dropped: 0 }, idBase: 'm1' });
  const btn = (h) => h.match(/<button[^>]*class="([^"]+)"/)[1];
  assert.equal(btn(html), ui.STEPS_ROW_BUTTON);
  assert.equal(btn(saved), ui.STEPS_ROW_BUTTON);
  assert.match(html, /^<section aria-label="What the bot is doing" data-steps-folded="" class="mt-2 max-w-full">/, 'below the bubble, the landed action row\'s gap');
  assert.match(saved, /^<div class="mb-2 max-w-full">/);
  assert.match(html, />Working · 3 steps</);
  assert.doesNotMatch(html, /\d:\d\d/, 'no time while live');
  assert.match(html, /aria-expanded="false"/);
  assert.doesNotMatch(html, /a\.ts/, 'steps hidden until opened');
  assert.match(html, /<\/button><div class="min-w-0 shrink"><span data-chip="">chip<\/span><\/div>/, 'browsed chip inline after the button');
  assert.match(html, /role="status"/, 'the announcer stays');
  assert.match(ui.STEPS_ROW_LIST, /mt-1\.5/);
  // Folded with one step says "1 step"; not folded is the card as before.
  assert.match(ui.live({ live: { turnId: 't', steps: [steps[0]], thinking: {} }, idBase: 'x', folded: true }), />Working · 1 step</);
  assert.doesNotMatch(ui.live({ live: { turnId: 't', steps, thinking: {} }, idBase: 'x' }), /data-steps-folded/);
  // Phone: the compact row keeps its look; folded it sits below the bubble with the landed chip row's mt-1.
  assert.match(ui.live({ live: { turnId: 't', steps, thinking: {} }, idBase: 'x', compact: true, folded: true }), /^<section aria-label="What the bot is doing" data-steps-compact="" class="mt-1 w-full max-w-2xl/);
  assert.match(ui.live({ live: { turnId: 't', steps, thinking: {} }, idBase: 'x', compact: true }), /^<section aria-label="What the bot is doing" data-steps-compact="" class="mb-2 w-full max-w-2xl/);
});

// ---------------------------------------------------------------- 28a: your no reads as a no, not a failure

const T = '2026-11-17T15:00:00Z';
const tool = (id, status, name, detail) => ({ id, kind: 'tool', status, title: `Running ${name}`, tool: name, detail, started_at: T });
const card = (id, status, name) => ({ id, kind: 'approval', status, title: `Waiting for your approval: ${name}`, tool: name, started_at: T });
const CLI_DENIED = 'The user denied this tool call.';
const SAID_NO = 'You said no; nothing ran';
const saved = (steps) => a.parseActivityMeta({ activity: { steps, dropped: 0 } }).steps;
const byId = (steps) => Object.fromEntries(steps.map((s) => [s.id, `${s.status}|${s.detail ?? ''}`]));

test('28a: a Deny (card declined + tool failed with the CLI denial) reads as declined, in both orders', () => {
  // The CLI's tool step usually starts first, the card's step after it.
  assert.deepEqual(byId(saved([tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash')])), {
    t1: `declined|${SAID_NO}`,
    c1: 'declined|',
  });
  assert.deepEqual(byId(saved([card('c1', 'declined', 'Bash'), tool('t1', 'failed', 'Bash', CLI_DENIED)])), {
    c1: 'declined|',
    t1: `declined|${SAID_NO}`,
  });
  assert.equal(a.DENIED_STEP_DETAIL, SAID_NO);
});

test('28a: without a declined card the denial stays failed (kwnt refused the call itself)', () => {
  assert.equal(saved([tool('t1', 'failed', 'Bash', CLI_DENIED)])[0].status, 'failed');
  // A card for another tool, or one the person allowed, doesn't make it theirs
  assert.equal(saved([card('c1', 'declined', 'Read'), tool('t1', 'failed', 'Bash', CLI_DENIED)])[1].status, 'failed');
  assert.equal(saved([card('c1', 'ok', 'Bash'), tool('t1', 'failed', 'Bash', CLI_DENIED)])[1].status, 'failed');
  // A real failure of the same tool next to a Deny stays failed
  assert.equal(saved([card('c1', 'declined', 'Bash'), tool('t1', 'failed', 'Bash', 'exit 1')])[1].status, 'failed');
});

test('28a: one Deny turns one step grey; two denied Bash calls with one card give one declined, one failed', () => {
  const got = saved([tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash'), tool('t2', 'failed', 'Bash', CLI_DENIED)]);
  assert.deepEqual(got.filter((s) => s.kind === 'tool').map((s) => s.status).sort(), ['declined', 'failed']);
  const two = saved([tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash'), tool('t2', 'failed', 'Bash', CLI_DENIED), card('c2', 'declined', 'Bash')]);
  assert.deepEqual(two.filter((s) => s.kind === 'tool').map((s) => s.status), ['declined', 'declined']);
});

test('28a: the 28b shape reads the same and takes its card; 27b\'s no-card no is unchanged', () => {
  // 28b: the server already ends the tool step declined, so the card is its, not the next failed step's
  const b = saved([tool('t1', 'declined', 'Bash', SAID_NO), card('c1', 'declined', 'Bash'), tool('t2', 'failed', 'Bash', CLI_DENIED)]);
  assert.deepEqual(byId(b), { t1: `declined|${SAID_NO}`, c1: 'declined|', t2: `failed|${CLI_DENIED}` });
  // 27b: declined with no card at all
  const NO_CARD = 'You said no to a call like this earlier in this chat, so it didn’t run.';
  assert.deepEqual(byId(saved([tool('t1', 'declined', 'Bash', NO_CARD)])), { t1: `declined|${NO_CARD}` });
  // Reading twice changes nothing
  const once = saved([tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash')]);
  assert.deepEqual(a.readDenials(once), once);
});

test('28a: live, the card then the failed tool step ends declined (and the other order too)', () => {
  let live = a.applyActivity(undefined, 't', card('c1', 'waiting', 'Bash'));
  live = a.applyActivity(live, 't', tool('t1', 'running', 'Bash'));
  live = a.applyActivity(live, 't', card('c1', 'declined', 'Bash'));
  live = a.applyActivity(live, 't', tool('t1', 'failed', 'Bash', CLI_DENIED));
  assert.deepEqual(byId(live.steps), { c1: 'declined|', t1: `declined|${SAID_NO}` });
  let rev = a.applyActivity(undefined, 't', tool('t1', 'running', 'Bash'));
  rev = a.applyActivity(rev, 't', tool('t1', 'failed', 'Bash', CLI_DENIED));
  assert.equal(rev.steps[0].status, 'failed', 'no card yet: still failed');
  rev = a.applyActivity(rev, 't', card('c1', 'declined', 'Bash'));
  assert.equal(rev.steps[0].status, 'declined');
  // Seeding after a reconnect reads the same
  const seeded = a.seedActivity(undefined, 't', { steps: [tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash')], dropped: 0 });
  assert.equal(seeded.steps[0].status, 'declined');
});

test('28a: the summary says "you said no to 1", never "failed", for a plain Deny', () => {
  const steps = saved([tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash')]);
  const sum = a.activitySummary({ steps, dropped: 0 });
  assert.equal(sum, '2 steps · you said no to 1');
  const two = saved([tool('t1', 'failed', 'Bash', CLI_DENIED), card('c1', 'declined', 'Bash'), tool('t2', 'failed', 'Read', CLI_DENIED), card('c2', 'declined', 'Read')]);
  assert.match(a.activitySummary({ steps: two, dropped: 0 }), /you said no to 2$/);
  // The steps row: neutral icon (no rose)
  const html = ui.saved({ activity: { steps, dropped: 0 }, idBase: 'r1' });
  assert.doesNotMatch(html, /text-rose/);
  assert.match(html, /you said no to 1/);
  // The command row says "You said no" in words (a neutral mark, Mira); 28s: so does the approval row's label, and
  // in Details the card folds into the command row, so the answer shows once
  assert.equal(a.statusLabel(steps[0]), 'You said no');
  assert.equal(a.statusLabel(steps[1]), 'You said no');
  const open = ui.saved({ activity: { steps, dropped: 0 }, idBase: 'r1', defaultOpen: true });
  const marks = [...open.matchAll(/data-step-mark="declined" class="([^"]*)"[^>]*>([^<]*)</g)];
  assert.equal(marks.length, 1, 'one mark: on the command, not the approval row');
  assert.equal(marks[0][2], 'You said no');
  assert.match(marks[0][1], /text-zinc-300 border-zinc-500\/40/);
  // Its detail is a sentence in the sans face, in the grey box
  assert.match(open, /<pre class="[^"]*font-sans text-xs[^"]*text-zinc-300 bg-zinc-950\/80[^"]*">You said no; nothing ran<\/pre>/);
});
