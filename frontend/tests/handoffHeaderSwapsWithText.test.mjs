// 32g (autonomy/handoff-header-swaps-with-text/brief.md; Mira's design check of the 0.9.35 handoff clip): at a handoff
// in a team run, the live card's name and the Handoff band must not name the next speaker (B) while the previous
// speaker's (A's) words are still in the stream. They change in the same render as the text clears, after the reload
// that brings A's saved reply in. A failed reload still swaps; two quick handoffs (A to B to C) end on C without
// wiping C's first words; another chat open meanwhile keeps its own stream.
//
// The test runs App.tsx's own `case 'DiscussionTurnStarted'` block (its source, transpiled) with the real stream store,
// holds refreshContext open, and records what the live card would show at every render: the name ChatView uses
// (discussionState.currentSpeaker while a run is in progress, ChatView.tsx:856 and the band at :2574) and the
// stream text. A setter outside flushSync is one render; everything inside one flushSync is one render. Red at main 3896052e.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { transform } from 'esbuild';
import { loadBundled } from './bundle.mjs';

process.on('unhandledRejection', () => {}); // main's .then() has no rejection handler; the test checks what shows instead

const { createStreamText } = await loadBundled(`export { createStreamText } from './src/streamText';`);

const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
const head = "case 'DiscussionTurnStarted': {";
const start = app.indexOf(head);
assert.ok(start > 0, 'App.tsx handles DiscussionTurnStarted');
const end = app.indexOf('\n            break;\n          }', start);
assert.ok(end > start, 'its block ends with break');
const block = app.slice(start + head.length, end);
const { code: blockJs } = await transform(`function onTurnStarted(event) { switch (0) { default: {${block}\n break; } } }`, { loader: 'ts' });
const DEPS = ['discussionInProgressRef', 'handoffRef', 'setDiscussionState', 'flushSync', 'streamingContextIdRef', 'selectedIdRef',
  'refreshContext', 'streamsRef', 'openCtxRef', 'streamStore', 'markLive'];
const make = new Function(...DEPS, `${blockJs}\nreturn onTurnStarted;`);

/** App's pieces for one team run in chat 'team', A speaking, with a reload per handoff the test resolves or rejects. */
function rig({ open = 'team' } = {}) {
  const s = { ds: { inProgress: true, currentSpeaker: 'Ada', currentTurn: 1, totalTurns: 3, kind: 'handoff' }, live: { team: 'Ada' }, frames: [], loads: [] };
  const store = createStreamText({}, { frame: () => 0, cancelFrame: () => {} });
  const streamsRef = { current: { team: { text: 'Ada: the plan is ready.', thinking: '' } } };
  store.show({ text: 'Ada: the plan is ready.' });
  let batching = 0;
  s.render = () => {
    if (batching) return;
    const name = s.ds.inProgress && s.ds.currentSpeaker ? s.ds.currentSpeaker : s.live.team;
    s.frames.push({ name, text: open === 'team' ? store.get().text : streamsRef.current.team?.text ?? '', band: s.ds.currentSpeaker, turn: s.ds.currentTurn, sidebar: s.live.team });
  };
  store.subscribe(() => s.render());
  const flushSync = (fn) => { batching += 1; try { fn(); } finally { batching -= 1; } s.render(); };
  const setDiscussionState = (u) => { s.ds = typeof u === 'function' ? u(s.ds) : u; s.render(); };
  const markLive = (ctx, name) => { if (!ctx) return; const n = name ?? s.live[ctx] ?? ''; if (s.live[ctx] === n) return; s.live = { ...s.live, [ctx]: n }; s.render(); };
  const refreshContext = () => new Promise((ok, no) => s.loads.push({ ok, no }));
  s.inRun = { current: true };
  s.openCtx = { current: open };
  s.streamsRef = streamsRef;
  s.store = store;
  s.handoff = make(s.inRun, { current: 0 }, setDiscussionState, flushSync, { current: 'team' }, { current: 'team' },
    refreshContext, streamsRef, s.openCtx, store, markLive);
  s.speak = (name, i) => s.handoff({ data: { speaker_name: name, turn_index: i, total_turns: 3, asked_by: 'Ada' } });
  /** A token from the speaker now streaming, as App's TextDelta does (store + streamsRef). */
  s.token = (t) => { const next = (streamsRef.current.team?.text ?? '') + t; streamsRef.current.team = { text: next, thinking: '' }; if (open === 'team') store.show({ text: next }); };
  s.last = () => s.frames[s.frames.length - 1];
  return s;
}
const tick = () => new Promise((r) => setTimeout(r, 0));
const mixed = (s, words, who) => s.frames.filter((f) => f.name === who && f.text.includes(words));

test('32g: while the reload is out, the card still names Ada over Ada\'s words (no frame shows Lea over them)', async () => {
  const s = rig();
  s.speak('Lea', 2);
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Lea'), [], 'no render names Lea over Ada\'s text');
  assert.equal(s.last()?.name ?? 'Ada', 'Ada');
  s.loads[0].ok(); await tick();
  assert.equal(s.last().name, 'Lea', 'then the card names Lea');
  assert.equal(s.last().text, '', 'with the text cleared');
  assert.equal(s.last().band, 'Lea', 'the Handoff band names Lea');
  assert.equal(s.last().turn, 2, 'turn 2 of 3');
  assert.equal(s.last().sidebar, 'Lea', 'the sidebar live row names Lea');
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Lea'), []);
});

test('32g: the name and the cleared text land in one render', async () => {
  const s = rig();
  const before = s.frames.length;
  s.speak('Lea', 2);
  s.loads[0].ok(); await tick();
  const after = s.frames.slice(before);
  const first = after.findIndex((f) => f.name === 'Lea');
  assert.ok(first >= 0, 'Lea shows');
  assert.equal(after[first].text, '', 'the first render naming Lea has no Ada text');
  assert.ok(after.slice(0, first).every((f) => f.name === 'Ada'), 'every earlier render still names Ada');
});

test('32g: a failed reload still moves the card to Lea and clears Ada\'s words', async () => {
  const s = rig();
  s.speak('Lea', 2);
  s.loads[0].no(new Error('HTTP 502')); await tick();
  assert.equal(s.last()?.name, 'Lea');
  assert.equal(s.last()?.text, '');
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Lea'), []);
});

test('32g: two quick handoffs (Ada to Lea to Max) end on Max, and Lea\'s late reload keeps Max\'s first words', async () => {
  const s = rig();
  s.speak('Lea', 2);
  s.speak('Max', 3);
  s.loads[1].ok(); await tick();
  assert.equal(s.last().name, 'Max');
  s.token('Max: on it.');
  s.loads[0].ok(); await tick();
  assert.equal(s.last().name, 'Max', 'still Max');
  assert.equal(s.last().band, 'Max');
  assert.equal(s.last().turn, 3);
  assert.equal(s.store.get().text, 'Max: on it.', 'Max\'s words stay');
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Lea'), []);
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Max'), []);
});

test('32g: two quick handoffs whose reloads land in order still end on Max', async () => {
  const s = rig();
  s.speak('Lea', 2);
  s.speak('Max', 3);
  s.loads[0].ok(); await tick();
  s.loads[1].ok(); await tick();
  assert.equal(s.last().name, 'Max');
  assert.equal(s.last().text, '');
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Lea'), []);
  assert.deepEqual(mixed(s, 'Ada: the plan', 'Max'), []);
});

test('32g (guard, passes on main too): with another chat open, the team chat\'s kept stream clears and its live name moves; the open chat\'s stream is not touched', async () => {
  const s = rig({ open: 'other' });
  s.store.show({ text: 'other chat words' });
  s.speak('Lea', 2);
  s.loads[0].ok(); await tick();
  assert.deepEqual(s.streamsRef.current.team, { text: '', thinking: '' });
  assert.equal(s.live.team, 'Lea');
  assert.equal(s.store.get().text, 'other chat words');
});

test('32g (guard, passes on main too): a run that ended before the reload landed is not brought back to life', async () => {
  const s = rig();
  s.speak('Lea', 2);
  s.inRun.current = false; // DiscussionCompleted / Stop: App sets the ref false and discussionState to { inProgress: false }
  s.ds = { inProgress: false };
  delete s.live.team;
  s.loads[0].ok(); await tick();
  assert.equal(s.ds.inProgress, false, 'the run stays over');
  assert.equal(s.live.team, undefined, 'the chat is not marked live again');
});

test('32g: a new run bumps the handoff count, so a handoff from the run before changes nothing', () => {
  const started = app.slice(app.indexOf("case 'DiscussionStarted': {"), app.indexOf("case 'discussion_turn_started':"));
  assert.match(started, /handoffRef\.current \+= 1/);
  assert.match(app, /const handoffRef = useRef\(0\);/);
});
