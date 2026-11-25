// stuck-getting-started (0.9.51): a team chat said "Quant is getting started · 1:25:45" with Stop while
// nothing ran. The app had missed Quant's turn_ended. It now reads the running turns again on every
// connect, when the window comes back and after Stop, and a chat no listed turn runs in stops showing
// a reply. Notes: autonomy/stuck-getting-started/impl.md.
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

const turn = (turn_id, context_id, state = 'running') => ({ turn_id, kwant_id: 'quant', context_id, thread_id: null, state, since: '2026-11-24T19:04:00Z' });

test('a chat whose turn the server no longer lists stops showing a reply', () => {
  // The app still shows Quant "getting started" in the team chat; the server lists only another chat's turn
  const live = { 'team-binan': 'Quant', 'dm-ben': 'Ben' };
  const listed = t.hydrateTurns([turn('t2', 'dm-ben')]);
  assert.deepEqual(t.unlistedLive(live, listed), ['team-binan']);
});

test('a chat with a listed turn, running or queued, keeps its reply', () => {
  const live = { 'team-binan': 'Quant', 'dm-ben': 'Ben' };
  const listed = t.hydrateTurns([turn('t1', 'team-binan', 'queued'), turn('t2', 'dm-ben')]);
  assert.deepEqual(t.unlistedLive(live, listed), []);
  assert.deepEqual(t.unlistedLive({}, listed), []);
  assert.deepEqual(t.unlistedLive(live, t.hydrateTurns([])), ['team-binan', 'dm-ben']);
});

test('coming back to the window re-reads at most every few seconds, and never while hidden', () => {
  const now = 1_000_000;
  assert.equal(t.rereadDue(0, now, false), true);
  assert.equal(t.rereadDue(now - 1_000, now, false), false, 'focus and visibilitychange fire together');
  assert.equal(t.rereadDue(now - t.REREAD_GAP_MS, now, false), true);
  assert.equal(t.rereadDue(0, now, true), false);
});

test('the app re-reads the running turns on focus, on a shown tab and after Stop', async () => {
  const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /window\.addEventListener\('focus', onBack\)/);
  assert.match(app, /document\.addEventListener\('visibilitychange', onBack\)/);
  assert.match(app, /refreshContext\(ctx\)\s*\.then\(\(\) => rereadTurns\(true\)\)/);
  // A live turn event during the read wins over the list
  assert.match(app, /if \(fresh && turnEventsRef\.current !== seen\) return;/);
});
