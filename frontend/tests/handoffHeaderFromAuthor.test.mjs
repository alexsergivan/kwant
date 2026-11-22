// 32s (autonomy/handoff-header-from-author/brief.md; Mira's design check of Kit's 0.9.37 clips, site-clips-0937/design-check.md:14
// and :51): when a bot's reply @mentions a teammate, the server opens the handoff chain only after that reply is done
// (routes.rs DiscussionStarted { kind: "handoff", speakers: [QA] }, then DiscussionTurnStarted { QA, turn 2, asked_by Designer }).
// The Designer's finished words are still the live stream then, so the live card and the Handoff band must keep the
// Designer's name ("Designer is replying · 1 of 2") until 32g's swap in DiscussionTurnStarted moves name and text together.
// A `discussion` start and a handoff with no live text behave as before.
//
// Runs App.tsx's own `case 'DiscussionStarted'` and `case 'DiscussionTurnStarted'` blocks (source, transpiled) with the real
// stream store, holds refreshContext open, and records what the live card shows at every render (ChatView's liveName rule,
// band name and turn, sidebar live name, stream text). Settles on rendered state, never on fixed ticks. Red at main 97423d86.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { transform } from 'esbuild';
import { loadBundled } from './bundle.mjs';

const { createStreamText } = await loadBundled(`export { createStreamText } from './src/streamText';`);

const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
async function blockOf(head) {
  const start = app.indexOf(head);
  assert.ok(start > 0, `App.tsx has ${head}`);
  const end = app.indexOf('\n            break;\n          }', start);
  assert.ok(end > start, 'its block ends with break');
  const { code } = await transform(`function on(event) { switch (0) { default: {${app.slice(start + head.length, end)}\n break; } } }`, { loader: 'ts' });
  return code;
}
const DEPS = ['discussionInProgressRef', 'handoffRef', 'setDiscussionState', 'flushSync', 'streamingContextIdRef', 'selectedIdRef',
  'refreshContext', 'streamsRef', 'openCtxRef', 'streamStore', 'markLive', 'liveRef', 'kwantsRef', 'channelsRef', 'teamChannelOf', 'playSfx'];
const makeStarted = new Function(...DEPS, `${await blockOf("case 'DiscussionStarted': {")}\nreturn on;`);
const makeTurn = new Function(...DEPS, `${await blockOf("case 'DiscussionTurnStarted': {")}\nreturn on;`);

const WORDS = 'Designer: the cards fit at 390. @QA could you check this on a 360 px screen?';

/** One team chat 'team' with the Designer's finished reply still live (or not), and a reload per handoff the test resolves. */
function rig({ words = WORDS, author = 'Designer' } = {}) {
  const s = { ds: { inProgress: false }, live: author ? { team: author } : {}, frames: [], loads: [] };
  const store = createStreamText({}, { frame: () => 0, cancelFrame: () => {} });
  const streamsRef = { current: words ? { team: { text: words, thinking: '' } } : {} };
  if (words) store.show({ text: words });
  let batching = 0;
  s.render = () => {
    if (batching) return;
    const name = s.ds.inProgress && s.ds.currentSpeaker ? s.ds.currentSpeaker : s.live.team;
    s.frames.push({ name, text: store.get().text, band: s.ds.inProgress ? s.ds.currentSpeaker : undefined, turn: s.ds.currentTurn,
      total: s.ds.totalTurns, kind: s.ds.kind, askedBy: s.ds.askedBy, sidebar: s.live.team });
  };
  store.subscribe(() => s.render());
  const flushSync = (fn) => { batching += 1; try { fn(); } finally { batching -= 1; } s.render(); };
  const setDiscussionState = (u) => { s.ds = typeof u === 'function' ? u(s.ds) : u; s.render(); };
  const liveRef = { get current() { return s.live; }, set current(v) { s.live = v; } };
  const markLive = (ctx, name) => { if (!ctx) return; const n = name ?? s.live[ctx] ?? ''; if (s.live[ctx] === n) return; s.live = { ...s.live, [ctx]: n }; s.render(); };
  const refreshContext = () => new Promise((ok, no) => s.loads.push({ ok, no }));
  s.inRun = { current: false };
  s.streamsRef = streamsRef;
  s.store = store;
  const deps = [s.inRun, { current: 0 }, setDiscussionState, flushSync, { current: null }, { current: 'team' },
    refreshContext, streamsRef, { current: 'team' }, store, markLive, liveRef,
    { current: [{ id: 'k-qa', name: 'QA' }, { id: 'k-des', name: 'Designer' }, { id: 'k-lea', name: 'Lea' }] },
    { current: [{ id: 'team', name: 'Clip team' }] }, () => ({ id: 'team' }), () => {}];
  const started = makeStarted(...deps);
  const turn = makeTurn(...deps);
  s.start = (kind = 'handoff', speakers = ['k-qa'], total = 2) =>
    started({ data: { team_id: 'team', team_name: 'Clip team', speakers, total_turns: total, kind } });
  s.speak = (name, i, askedBy = 'Designer', total = 2) => turn({ data: { speaker_name: name, turn_index: i, total_turns: total, asked_by: askedBy } });
  s.last = () => s.frames[s.frames.length - 1];
  return s;
}
const tick = () => new Promise((r) => setTimeout(r, 0));
const over = (s, who) => s.frames.filter((f) => f.text.includes('the cards fit') && (f.name !== 'Designer' || (f.band && f.band !== 'Designer')) && (f.name === who || f.band === who));

test('32s: a reply-mention handoff start keeps the Designer\'s name over the Designer\'s words ("Designer is replying · 1 of 2")', () => {
  const s = rig();
  s.start();
  assert.deepEqual(over(s, 'QA'), [], 'no render names QA over the Designer\'s words');
  const f = s.last();
  assert.equal(f.name, 'Designer', 'live card names the Designer');
  assert.equal(f.band, 'Designer', 'the band reads "Designer is replying"');
  assert.equal(f.turn, 1, '1 of');
  assert.equal(f.total, 2, '... 2');
  assert.equal(f.kind, 'handoff');
  assert.equal(f.sidebar, 'Designer', 'the sidebar live row keeps the Designer');
  assert.equal(s.ds.inProgress, true, 'the run is in progress');
  assert.equal(s.ds.contextId, 'team');
  assert.equal(s.ds.teamName, 'Clip team');
  assert.equal(s.inRun.current, true);
});

test('32s: the band never reads "QA ... 1 of 2"', async () => {
  const s = rig();
  s.start();
  s.speak('QA', 2);
  s.loads[0].ok(); await tick();
  assert.deepEqual(s.frames.filter((f) => f.band === 'QA' && f.turn === 1), []);
});

test('32s: with the reload held open, nothing names QA over the Designer\'s words; then QA, turn 2, asked by the Designer, in one render with the text cleared', async () => {
  const s = rig();
  s.start();
  s.speak('QA', 2);
  assert.deepEqual(over(s, 'QA'), [], 'while the reload is out');
  assert.equal(s.last().name, 'Designer');
  const before = s.frames.length;
  s.loads[0].ok(); await tick();
  const after = s.frames.slice(before);
  const first = after.findIndex((f) => f.name === 'QA');
  assert.ok(first >= 0, 'QA shows once the reload lands');
  assert.equal(after[first].text, '', 'the first render naming QA has no Designer text');
  assert.equal(after[first].band, 'QA');
  assert.equal(after[first].turn, 2);
  assert.equal(after[first].askedBy, 'Designer');
  assert.equal(s.last().sidebar, 'QA');
  assert.deepEqual(over(s, 'QA'), []);
});

test('32s: a failed reload still moves the card to QA and clears the Designer\'s words', async () => {
  const s = rig();
  s.start();
  s.speak('QA', 2);
  s.loads[0].no(new Error('HTTP 502')); await tick();
  assert.equal(s.last().name, 'QA');
  assert.equal(s.last().text, '');
  assert.deepEqual(over(s, 'QA'), []);
});

test('32s (guard, passes on main too): a handoff with no live text names the next speaker at once, as before', () => {
  const s = rig({ words: '', author: null });
  s.start();
  assert.equal(s.last().name, 'QA');
  assert.equal(s.last().band, 'QA');
  assert.equal(s.last().turn, 1);
  assert.equal(s.live.team, 'QA');
});

test('32s (guard, passes on main too): a discussion start names its first speaker at once, as before', () => {
  const s = rig();
  s.start('discussion', ['k-lea', 'k-qa'], 4);
  assert.equal(s.last().name, 'Lea');
  assert.equal(s.last().band, 'Lea');
  assert.equal(s.last().kind, 'discussion');
  assert.equal(s.live.team, 'Lea');
});
