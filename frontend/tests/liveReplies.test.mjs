import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const t = await load('../src/threads.ts');

// A chat on screen, kept the way App keeps it: TurnComplete loads the chat, turn_ended loads it
// again when reloadOnTurnEnd says so, and a load only lands if ChatLoads says it is the newest.
function chat() {
  const server = [{ id: 'm1', text: 'hi' }];
  const loads = new t.ChatLoads();
  let screen = [];
  const pending = [];
  const reload = () => {
    const n = loads.start('c');
    const snapshot = server.slice();
    const p = { done: () => { if (loads.isLatest('c', n)) screen = snapshot; } };
    pending.push(p);
    return p;
  };
  return { server, reload, screen: () => screen, pending };
}

test('a reply saved after TurnComplete still shows once the turn ends (no reopening)', () => {
  const c = chat();
  // TurnComplete: the driver is done, the reply isn't saved yet
  c.reload().done();
  // kwant saves the reply, then sends turn_ended
  c.server.push({ id: 'm2', text: 'Stub reply' });
  if (t.reloadOnTurnEnd('turn-1')) c.reload().done();
  assert.deepEqual(c.screen().map((m) => m.id), ['m1', 'm2'], 'the bot reply is on screen');
});

test('a slow load that finishes last never hides the reply a newer load showed', () => {
  const c = chat();
  const early = c.reload(); // TurnComplete's load, slow over Docker networking
  c.server.push({ id: 'm2', text: 'Stub reply' });
  const late = c.reload(); // turn_ended's load
  late.done();
  early.done();
  assert.deepEqual(c.screen().map((m) => m.id), ['m1', 'm2']);
});

test('App reloads on turn_ended through reloadOnTurnEnd and guards loads with ChatLoads', async () => {
  const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /reloadOnTurnEnd\(/, 'turn_ended asks reloadOnTurnEnd');
  assert.match(app, /chatLoads(Ref)?\.current\.isLatest\(|chatLoads\.isLatest\(/, 'refreshContext drops stale loads');
  assert.doesNotMatch(app, /completed\.delete\(d\.turn_id\)\) settle\(ctx\)/, 'turn_ended no longer skips the reload');
});
