// No sign-in on the person's own computer (2026-10-22): from loopback the server answers
// person-only routes without a token, so the app must never open "Confirm it's you" by itself.
// Every token prompt is driven by a 401; these pin that a 200 (the loopback answer) asks nothing.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/auth';`);

const json = (status, body) => () => new Response(JSON.stringify(body), { status });

/** A minimal `window` whose fetch answers from `answer(url)`, counting the token prompts. */
function fakeWindow(answer) {
  const w = new EventTarget();
  w.location = { origin: 'http://127.0.0.1:8899', hash: '', pathname: '/', search: '' };
  w.fetch = async (url) => answer(String(url))();
  const asked = { person: 0, all: 0 };
  w.addEventListener(m.PERSON_TOKEN_EVENT, () => asked.person++);
  w.addEventListener(m.UNAUTHORIZED_EVENT, () => asked.all++);
  return { w, asked };
}

test('loopback answers to person-only routes open no token prompt; a person-only 401 does', async () => {
  const { w, asked } = fakeWindow((url) =>
    url.includes('/remote/') ? json(401, { error: 'x', person_only: true }) : json(200, { passwords: [] }),
  );
  globalThis.window = w;
  try {
    m.installUnauthorizedListener();
    for (const url of ['/api/sandbox/k1/passwords', '/api/zombie/cases', '/api/parallel/prices', '/api/approvals/a1/resolve']) {
      const res = await window.fetch(url, { method: url.endsWith('resolve') ? 'POST' : 'GET' });
      assert.equal(res.status, 200);
    }
    assert.deepEqual(asked, { person: 0, all: 0 }, 'nothing asks the person on their own computer');
    await window.fetch('/api/remote/passwords');
    assert.deepEqual(asked, { person: 1, all: 0 }, 'another machine still gets "Confirm it\'s you"');
  } finally {
    delete globalThis.window;
  }
});

test('a person-only action that the server lets through is sent once, with no wait for sign-in', async () => {
  const calls = [];
  const send = async (url) => (calls.push(url), new Response('{}', { status: 200 }));
  const res = await m.personFetch('/api/kwants/k1', { method: 'DELETE' }, new EventTarget(), send);
  assert.equal(res.status, 200);
  assert.deepEqual(calls, ['/api/kwants/k1']);
});
