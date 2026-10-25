// One-time sign-in links (#kwnt_token=kl1.…): opening one signs in with no paste, and a used or
// expired one says so and points to the other ways in.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { LINK_SPENT, consumeTokenLink, isLinkCode, redeemSignIn } from './src/auth';
`);

/** A server that answers POST /api/auth/session with `status` and `body`, recording what it got. */
function server(status, body = null) {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init, body: JSON.parse(init.body) });
    return new Response(status === 204 ? null : JSON.stringify(body), { status });
  };
  return calls;
}

/** A page opened at `hash`, with replaceState recorded. */
function page(hash) {
  const replaced = [];
  globalThis.window = {
    location: { hash, pathname: '/', search: '' },
    history: { state: null, replaceState: (_s, _t, url) => replaced.push(url) },
  };
  return replaced;
}

test('link codes are told apart from the API token', () => {
  assert.ok(m.isLinkCode('kl1.abcdef'));
  assert.ok(m.isLinkCode('  kl1.abcdef '));
  assert.ok(!m.isLinkCode('abcdefghijklmnopqrstuvwxyz012345'));
});

test('a live link signs in: the code is posted, then cleared from the address bar', async () => {
  const calls = server(204);
  const replaced = page('#kwnt_token=kl1.live');
  const err = await m.consumeTokenLink();
  assert.equal(err, null);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, '/api/auth/session');
  assert.equal(calls[0].init.method, 'POST');
  assert.deepEqual(calls[0].body, { token: 'kl1.live' });
  assert.deepEqual(replaced, ['/'], 'the code leaves the address bar and history');
});

test('a used or expired link says so, not "wrong token"', async () => {
  server(401, { error: 'this sign-in link was already used or has expired', reason: 'link_used_or_expired' });
  page('#kwnt_token=kl1.spent');
  assert.equal(await m.consumeTokenLink(), m.LINK_SPENT);
  assert.match(m.LINK_SPENT, /already used or has expired.*works once, for 15 minutes.*Sign in another way below/);
});

test('a link the server forgot (it restarted) reads as expired too', async () => {
  server(401, { error: 'wrong token', reason: 'wrong_token' });
  assert.equal(await m.redeemSignIn('kl1.forgotten'), 'link_used_or_expired');
  page('#kwnt_token=kl1.forgotten');
  assert.equal(await m.consumeTokenLink(), m.LINK_SPENT);
});

test('a wrong API token stays a wrong token', async () => {
  server(401, { error: 'wrong token', reason: 'wrong_token' });
  assert.equal(await m.redeemSignIn('not-the-token'), 'wrong_token');
  page('#kwnt_token=not-the-token');
  assert.match(await m.consumeTokenLink(), /token wasn't accepted/);
});

test('an old server without a reason still works', async () => {
  server(401, { error: 'wrong token' });
  assert.equal(await m.redeemSignIn('not-the-token'), 'wrong_token');
  server(204);
  assert.equal(await m.redeemSignIn('the-token'), 'ok');
});

test('an empty link and a server error are explained', async () => {
  page('#kwnt_token=');
  assert.match(await m.consumeTokenLink(), /incomplete/);
  server(500, {});
  await assert.rejects(m.redeemSignIn('kl1.x'), /500/);
});
