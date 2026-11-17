// 22r: when an approve-by-reaction turn ends, the chat never shows an older state for a frame.
// Mira's clip-v611-react (mac-v0.9.15) showed "TechLead is waiting for your OK · Approve / Change…" again for ~67 ms
// after the 👍, right before the published reply. Cause: TurnComplete's chat load was replaced by turn_ended's,
// came back first, and its settle dropped the working line while the list without the reply was still on screen.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};
const t = await load('../src/threads.ts');
const a = await load('../src/approvalAsk.ts');

const deferred = () => { let resolve; const p = new Promise((r) => (resolve = r)); return { p, resolve }; };
const tick = () => new Promise((r) => setImmediate(r));

const ASK = { id: 'ask', sender_role: 'kwant', text: 'The release notes are ready. Shall I publish them now?' };
const USER = { id: 'u1', sender_role: 'user', text: 'Can you write up the release notes for Trailhead 2.4?' };
const REPLY = { id: 'pub', sender_role: 'kwant', text: 'Published. The 2.4 release notes are live on the changelog page.' };

/**
 * The open chat the way App keeps it: `refreshContext` (ChatLoads.run, only the newest load lands), `settle`
 * (no turn left: the working line goes), and ChatView's Approve row from the list and the live state. A frame
 * is what the browser paints: the state once a task (a load coming back, an event) and all its microtasks are done.
 * App commits the list and the live state with flushSync and settles in the same task, so in-between states never paint.
 */
function scene({ answered = new Set(['ask']) } = {}) {
  const loads = new t.ChatLoads();
  let screen = [{ ...USER }, { ...ASK }];
  let live = true; // the 👍 started the turn
  let stream = REPLY.text; // the working line has streamed the reply
  let turns = { 'turn-1': { turn_id: 'turn-1', context_id: 'c', state: 'running', seq: 1 } };
  const server = [{ ...USER }, { ...ASK }];
  const fetches = [];
  const frames = [];
  let why = 'after 👍';
  const frame = (w) => { why = w; };
  const paint = () => {
    const last = screen[screen.length - 1];
    const saved = screen.some((m) => m.id === 'pub');
    const streamed = live && stream === REPLY.text;
    const replyShown = saved || streamed;
    // 25a: the header's badge before (turns only) and now (the open chat's live card too)
    const oldHeader = t.threadBadge({ id: 'c', delegation: null }, turns)?.kind ?? null;
    const header = t.writingBadge(t.threadBadge({ id: 'c', delegation: null }, turns), live)?.kind ?? null;
    frames.push({ why, ids: screen.map((m) => m.id), live, stream, replyShown, twice: saved && streamed, approve: a.approveRowId(last, null, { busy: live, answered }), oldHeader, header });
  };
  const refreshContext = () =>
    loads.run('c', async (isLatest) => {
      const f = deferred();
      fetches.push(f);
      const snapshot = server.map((m) => ({ ...m })); // fresh copies, like a reload
      await f.p;
      if (!isLatest()) return;
      screen = snapshot;
      frame('load landed');
    });
  // App's settle: another turn in the chat keeps the working line (its stream starts empty); none left drops it.
  const settle = () => {
    stream = '';
    if (Object.keys(turns).length === 0) live = false;
    frame('settle');
  };
  paint();
  // one load comes back: a task; the browser paints once it and its microtasks are done
  const respond = async (i) => { fetches[i].resolve(); await tick(); paint(); };
  return {
    frames, fetches, respond, server, screen: () => screen,
    turnComplete: () => {
      const shown = screen;
      return refreshContext().then(() => { if (t.settlesOnTurnComplete(turns, 'turn-1') || t.landedNewReply(shown, screen)) settle(); });
    },
    turnEnded: () => { turns = t.applyTurnEvent(turns, { type: 'turn_ended', data: { turn_id: 'turn-1' } }); return refreshContext().then(settle); },
  };
}

test('turn end: a load a newer one replaced never ends the turn on the older list (no Approve row after the 👍)', async () => {
  const s = scene();
  const done1 = s.turnComplete(); // the driver is done, the reply isn't saved yet
  s.server.push({ ...REPLY }); // kwnt saves the reply, then sends turn_ended
  const done2 = s.turnEnded();
  // the first load (no reply) comes back first: replaced, so nothing lands and nothing settles
  await s.respond(0);
  assert.equal(s.frames.at(-1).live, true, 'the working line stays while the newest load is out');
  await s.respond(1);
  await Promise.all([done1, done2]);
  assert.deepEqual(s.screen().map((m) => m.id), ['u1', 'ask', 'pub']);
  for (const f of s.frames) assert.equal(f.approve, null, `no Approve row at "${f.why}" (${f.ids.join(',')}, live ${f.live})`);
  // the working line goes only once the reply is on screen, and the reply's words never blank out on the way
  for (const f of s.frames) if (!f.live) assert.ok(f.ids.includes('pub'), `settled on an older list: ${f.ids.join(',')}`);
  for (const f of s.frames) assert.ok(f.replyShown, `the reply blanked at "${f.why}" (${f.ids.join(',')}, live ${f.live})`);
  for (const f of s.frames) assert.ok(!f.twice, `the reply showed twice at "${f.why}"`);
});

test('turn end: TurnComplete\'s load with the saved reply clears the streamed copy at once (never shown twice)', async () => {
  const s = scene();
  s.server.push({ ...REPLY }); // saved before TurnComplete's load reads it
  const done1 = s.turnComplete();
  await s.respond(0);
  await done1;
  for (const f of s.frames) assert.ok(!f.twice, `the reply showed twice at "${f.why}"`);
  for (const f of s.frames) assert.equal(f.approve, null);
  assert.equal(s.frames.at(-1).live, true, 'the turn is still running: its working line stays (empty) until turn_ended');
  const done2 = s.turnEnded();
  await s.respond(1);
  await done2;
  assert.equal(s.frames.at(-1).live, false);
  for (const f of s.frames) assert.ok(f.replyShown && !f.twice && f.approve === null, `bad frame at "${f.why}"`);
});

test('landedNewReply: only a new, saved bot message at the end', () => {
  const before = [USER, ASK];
  assert.equal(t.landedNewReply(before, [USER, ASK]), false);
  assert.equal(t.landedNewReply(before, [USER, ASK, REPLY]), true);
  assert.equal(t.landedNewReply(before, [USER, ASK, { id: 'u2', sender_role: 'user' }]), false);
  assert.equal(t.landedNewReply(before, [USER, ASK, { id: 'temp-1', sender_role: 'kwant' }]), false);
  assert.equal(t.landedNewReply(undefined, [REPLY]), true);
  assert.equal(t.landedNewReply(before, []), false);
});

test('turn end: TurnComplete\'s load landing alone (turn_ended not here yet) keeps the streamed reply up', async () => {
  const s = scene();
  const done1 = s.turnComplete();
  await s.respond(0); // lands as the newest load, without the reply
  await done1;
  const f = s.frames.at(-1);
  assert.equal(f.live, true);
  assert.equal(f.replyShown, true, 'the streamed words stay until the saved reply lands');
  assert.equal(f.approve, null);
  s.server.push({ ...REPLY });
  const done2 = s.turnEnded();
  await s.respond(1);
  await done2;
  assert.deepEqual(s.frames.at(-1).ids, ['u1', 'ask', 'pub']);
  assert.equal(s.frames.at(-1).live, false);
  for (const fr of s.frames) assert.ok(fr.replyShown && fr.approve === null, `frame "${fr.why}"`);
});

test('settlesOnTurnComplete: only once that turn is no longer running (or the event has no turn id)', () => {
  const turns = { 'turn-1': { turn_id: 'turn-1' } };
  assert.equal(t.settlesOnTurnComplete(turns, 'turn-1'), false);
  assert.equal(t.settlesOnTurnComplete({}, 'turn-1'), true, 'turn_ended came first');
  assert.equal(t.settlesOnTurnComplete(turns, 'turn-2'), true);
  assert.equal(t.settlesOnTurnComplete(turns, undefined), true);
});

test('turn end: the same race without the 👍 kept as answered still shows no Approve row (the load fix alone)', async () => {
  const s = scene({ answered: new Set() });
  const done1 = s.turnComplete();
  s.server.push({ ...REPLY });
  const done2 = s.turnEnded();
  await s.respond(0);
  await s.respond(1);
  await Promise.all([done1, done2]);
  assert.deepEqual(s.frames.filter((f) => f.approve).map((f) => f.why), []);
});

test('ChatLoads.run: resolves only once the newest load has landed, in either order', async () => {
  const loads = new t.ChatLoads();
  const order = [];
  const f1 = deferred(), f2 = deferred();
  const p1 = loads.run('c', async (isLatest) => { await f1.p; order.push(`1 latest=${isLatest()}`); }).then(() => order.push('1 resolved'));
  const p2 = loads.run('c', async (isLatest) => { await f2.p; order.push(`2 latest=${isLatest()}`); }).then(() => order.push('2 resolved'));
  f1.resolve();
  await tick();
  assert.deepEqual(order, ['1 latest=false'], 'the replaced load waits for the newer one');
  f2.resolve();
  await Promise.all([p1, p2]);
  assert.deepEqual(order.slice(0, 2), ['1 latest=false', '2 latest=true']);
  assert.deepEqual(order.slice(2).sort(), ['1 resolved', '2 resolved']);
  // another chat's load is not held by this one
  const f3 = deferred();
  let other = false;
  loads.run('c', () => f3.p);
  await loads.run('d', async () => {}).then(() => (other = true));
  assert.equal(other, true);
  f3.resolve();
  // a failed newer load still lets both resolve (refreshContext reports its own errors), so a settle never hangs
  const f4 = deferred();
  const p4 = loads.run('e', async () => {});
  const p5 = loads.run('e', () => f4.p.then(() => { throw new Error('HTTP 500'); }));
  f4.resolve();
  await assert.doesNotReject(p4);
  await assert.doesNotReject(p5);
});

test('approveRowId: a reply answered with a 👍 never gets the Approve row back, even as a fresh copy after a reload', () => {
  const answered = new Set(['ask']);
  assert.equal(a.approveRowId({ ...ASK }, null, { busy: false, answered: new Set() }), 'ask', 'unanswered: the row shows');
  assert.equal(a.approveRowId({ ...ASK }, null, { busy: false, answered }), null, 'answered by id: hidden');
  assert.equal(a.approveRowId({ ...ASK }, null, { busy: true, answered: new Set() }), null, 'a reply running: hidden');
  assert.equal(a.approveRowId({ ...ASK, id: 'temp-1' }, null, { busy: false, answered: new Set() }), null);
  assert.equal(a.approveRowId({ ...USER }, null, { busy: false, answered: new Set() }), null);
  assert.equal(a.approveRowId({ ...ASK }, { question_id: 'q' }, { busy: false, answered: new Set() }), null);
  assert.equal(a.approveRowId({ ...REPLY }, null, { busy: false, answered }), null);
  assert.equal(a.approveRowId(undefined, null, { busy: false, answered }), null);
});

test('App loads the chat through ChatLoads.run, so every settle after refreshContext waits for the newest load', async () => {
  const src = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const body = src.slice(src.indexOf('const refreshContext = '), src.indexOf('const rememberThreads'));
  assert.match(body, /chatLoadsRef\.current\.run\(ctx,/);
  assert.doesNotMatch(body, /chatLoadsRef\.current\.start\(/);
  const tc = src.slice(src.indexOf("case 'TurnComplete': {"), src.indexOf("case 'browser_navigated':"));
  assert.match(tc, /settlesOnTurnComplete\(turnsRef\.current, event\.turn_id\) \|\| landedNewReply\(shown, /);
});

test('25a: through the turn end the header says Working exactly while the body still shows the live card', async () => {
  const s = scene();
  const done1 = s.turnComplete();
  s.server.push({ ...REPLY });
  const done2 = s.turnEnded(); // the turn is gone from `turns` now; the newest load is still out
  await s.respond(0);
  await s.respond(1);
  await Promise.all([done1, done2]);
  // the old header went idle a fetch before the body (Mira's header-before-body-390.png)
  assert.ok(s.frames.some((f) => f.live && f.oldHeader === null), 'the scene reproduces the gap');
  for (const f of s.frames) assert.equal(f.header === 'working', f.live, `header vs body at "${f.why}" (live ${f.live}, header ${f.header})`);
  assert.equal(s.frames.at(-1).header, null, 'and idle together once the reply is in');
});

test('22r N1 (Rigel): clearLive commits the live map with flushSync before it clears the streamed words', async () => {
  const src = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const body = src.slice(src.indexOf('const clearLive = '), src.indexOf('const kwantName = ', src.indexOf('const clearLive = ')));
  const flush = body.indexOf('flushSync(() => setLive(next))');
  const show = body.indexOf('streamStore.show(');
  assert.ok(flush >= 0, 'setLive(next) goes through flushSync');
  assert.ok(show > flush, 'flushSync(() => setLive(next)) comes before streamStore.show');
  assert.doesNotMatch(body.replace('flushSync(() => setLive(next))', ''), /setLive\(/, 'no plain setLive besides it');
});
