import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

// 33c: the lead's hand-off card (and the add-on card) sometimes showed in the team chat only after a reload. A load of
// the chat started just before the server saved the card and came back just after the card arrived live. It was still
// the newest load, so it replaced the list with one that had no card, and nothing loaded the chat again.

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const t = await load('../src/threads.ts');

// One open chat, kept the way App keeps it: refreshContext runs a load through ChatLoads and, if it is still the
// newest, puts what it got on screen (with the live messages it missed: keepLive); message_added appends the message
// and notes it for the loads still running (noteLive).
function chat(initial) {
  const loads = new t.ChatLoads();
  const server = initial.slice();
  let screen = initial.slice();
  return {
    server,
    screen: () => screen.map((m) => m.id),
    /** Starts a load: it reads the server now and lands when `land()` is called. */
    startLoad() {
      let land;
      const done = new Promise((r) => (land = r));
      const snapshot = server.slice();
      // run's own promise waits for the newest load, so each load here awaits only itself
      let own;
      void loads.run('c', (isLatest, keepLive) => (own = (async () => {
        await done;
        if (!isLatest()) return;
        screen = keepLive ? keepLive(snapshot) : snapshot;
      })()));
      return { land: async () => { land(); await own; } };
    },
    /** The server saves a message and sends message_added. */
    add(m) {
      server.push(m);
      loads.noteLive?.('c', m);
      if (!screen.some((x) => x.id === m.id)) screen = [...screen, m];
    },
  };
}

test('a load that started before the hand-off card arrived live does not drop it when it lands', async () => {
  const c = chat([{ id: 'u1', sender_role: 'user' }, { id: 'k1', sender_role: 'kwant' }]);
  // The Editor's turn ends: the chat loads again (its fetch reads the list without the card)
  const l = c.startLoad();
  // Meanwhile the server saves the card and sends it live
  c.add({ id: 'card', sender_role: 'system', meta: '{"kind":"delegation_started"}' });
  assert.deepEqual(c.screen(), ['u1', 'k1', 'card'], 'the card shows live');
  // The older load lands
  await l.land();
  assert.deepEqual(c.screen(), ['u1', 'k1', 'card'], 'the card is still on screen after the load lands');
});

test('the add-on card survives two loads that both started before it (the newest lands last)', async () => {
  const c = chat([{ id: 'u1' }, { id: 'k1' }, { id: 'card1' }]);
  const a = c.startLoad();
  const b = c.startLoad();
  c.add({ id: 'addon', sender_role: 'system', meta: '{"after_delegation_id":"d1"}' });
  await a.land();
  await b.land();
  assert.deepEqual(c.screen(), ['u1', 'k1', 'card1', 'addon']);
});

test('a load that also has the live message shows it once', async () => {
  const c = chat([{ id: 'u1' }]);
  const l = c.startLoad();
  c.add({ id: 'k1', sender_role: 'kwant' });
  // This load read the list after the server saved k1
  c.server.length = 0;
  c.server.push({ id: 'u1' }, { id: 'k1' });
  const l2 = c.startLoad();
  await l.land();
  await l2.land();
  assert.deepEqual(c.screen(), ['u1', 'k1'], 'no second copy');
});

test('keepLive adds only messages that came live after that load started, in the order they came, never twice', () => {
  const loads = new t.ChatLoads();
  assert.equal(typeof loads.noteLive, 'function', 'ChatLoads.noteLive');
  assert.equal(typeof loads.keepLive, 'function', 'ChatLoads.keepLive');
  let n1 = 0;
  let n2 = 0;
  let land1;
  let land2;
  loads.run('c', () => { n1 = 1; return new Promise((r) => (land1 = r)); });
  loads.noteLive('c', { id: 'a' });
  loads.run('c', () => { n2 = 2; return new Promise((r) => (land2 = r)); });
  loads.noteLive('c', { id: 'b' });
  loads.noteLive('c', { id: 'c' });
  // Load 1 started before all three; load 2 only before b and c (it read a already)
  assert.deepEqual(loads.keepLive('c', n1, [{ id: 'x' }, { id: 'b' }]).map((m) => m.id), ['x', 'b', 'a', 'c']);
  assert.deepEqual(loads.keepLive('c', n2, [{ id: 'x' }, { id: 'a' }]).map((m) => m.id), ['x', 'a', 'b', 'c']);
  // Another chat's live messages never join this one
  loads.noteLive('other', { id: 'z' });
  assert.ok(!loads.keepLive('c', n2, []).some((m) => m.id === 'z'));
  land1();
  land2();
});

test('once the newest load lands, the live messages it covered are forgotten', async () => {
  const loads = new t.ChatLoads();
  let land;
  const p = loads.run('c', () => new Promise((r) => (land = r)));
  loads.noteLive('c', { id: 'a' });
  const kept = loads.keepLive('c', 1, [{ id: 'x' }]).map((m) => m.id);
  assert.deepEqual(kept, ['x', 'a']);
  land();
  await p;
  // A later load started after `a` came, so its own list has it; nothing is added from before
  let land2;
  const p2 = loads.run('c', () => new Promise((r) => (land2 = r)));
  assert.deepEqual(loads.keepLive('c', 2, [{ id: 'x' }]).map((m) => m.id), ['x']);
  land2();
  await p2;
  // With no load running, a live message is not kept for later
  loads.noteLive('c', { id: 'late' });
  assert.deepEqual(loads.keepLive('c', 3, []).map((m) => m.id), []);
});

test('App notes each live message and a landing load keeps the ones it missed', async () => {
  const src = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const refresh = src.slice(src.indexOf('const refreshContext ='), src.indexOf('const rememberThreads ='));
  assert.match(refresh, /run\(ctx, async \(isLatest, keepLive\)[^]*keepLive\(data\)/, 'refreshContext keeps live messages its load missed');
  const added = src.slice(src.indexOf("case 'MessageAdded':"), src.indexOf("case 'decide_updated':"));
  assert.match(added, /chatLoadsRef\.current\.noteLive\(ctx, m\)/, 'message_added notes the message for loads in flight');
});
