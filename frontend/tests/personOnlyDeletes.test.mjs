// Person-only deletes and undos (2026-10-20): deleting a bot or team and undoing a bot action or
// import need the person's token. A person-only 401 asks "Confirm it's you"; once they sign in the
// call is sent again, so no error shows and they don't press Delete twice. Closing the prompt
// changes nothing and says so.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export * from './src/auth';
  export { decisionError } from './src/botActions';
  export { importErrorText, ImportError } from './src/imports';
`);

const personOnly = () => new Response(JSON.stringify({ error: 'person_only', person_only: true }), { status: 401 });
const ok = () => new Response('{}', { status: 200 });

/** A fake fetch answering from a list, recording each call. */
function sender(answers) {
  const calls = [];
  const send = async (url, init) => {
    calls.push([url, init?.method]);
    return answers.shift()();
  };
  return { send, calls };
}

test('a person-only 401 waits for sign-in, then sends the delete once more', async () => {
  const target = new EventTarget();
  const { send, calls } = sender([personOnly, ok]);
  const p = m.personFetch('/api/kwants/k1', { method: 'DELETE' }, target, send);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls.length, 1, 'no retry before the person signs in');
  target.dispatchEvent(new Event(m.PERSON_SIGNED_IN_EVENT));
  const res = await p;
  assert.equal(res.status, 200);
  assert.deepEqual(calls, [
    ['/api/kwants/k1', 'DELETE'],
    ['/api/kwants/k1', 'DELETE'],
  ]);
});

test('closing "Confirm it’s you" throws PersonCancelledError and sends nothing more', async () => {
  const target = new EventTarget();
  const { send, calls } = sender([personOnly]);
  const p = m.personFetch('/api/teams/t1', { method: 'DELETE' }, target, send);
  await new Promise((r) => setTimeout(r, 0));
  target.dispatchEvent(new Event(m.PERSON_PROMPT_CLOSED_EVENT));
  await assert.rejects(p, (e) => e instanceof m.PersonCancelledError && /Nothing was changed/.test(e.message));
  assert.equal(calls.length, 1);
});

test('other answers pass straight through, including a 401 that is not person-only', async () => {
  for (const [answer, status] of [
    [ok, 200],
    [() => new Response('{"error":"nope"}', { status: 403 }), 403],
    [() => new Response('{"error":"unauthorized"}', { status: 401 }), 401],
    [() => new Response('not json', { status: 401 }), 401],
  ]) {
    const { send, calls } = sender([answer]);
    const res = await m.personFetch('/api/x', { method: 'DELETE' }, new EventTarget(), send);
    assert.equal(res.status, status);
    assert.equal(calls.length, 1);
  }
});

test('the wait stops listening once answered', async () => {
  const target = new EventTarget();
  const p = m.waitForPersonSignIn(target);
  target.dispatchEvent(new Event(m.PERSON_PROMPT_CLOSED_EVENT));
  assert.equal(await p, false);
  // A later sign-in for another action doesn't reach this one.
  const q = m.waitForPersonSignIn(target);
  target.dispatchEvent(new Event(m.PERSON_SIGNED_IN_EVENT));
  assert.equal(await q, true);
});

test('a cancelled undo reads as "nothing changed", not as an HTTP error', () => {
  const cancelled = new m.PersonCancelledError();
  assert.equal(m.decisionError(cancelled, 'undo it'), m.PERSON_CANCELLED_TEXT);
  const t = m.importErrorText(new m.ImportError(m.PERSON_CANCELLED_TEXT, 'person_cancelled', 401));
  assert.equal(t.title, 'Not undone');
  assert.equal(t.detail, m.PERSON_CANCELLED_TEXT);
  assert.doesNotMatch(t.detail, /HTTP/);
});
