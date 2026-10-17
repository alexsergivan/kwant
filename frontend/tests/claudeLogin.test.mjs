import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/claudeLogin';
  export { EnginesPanel } from './src/components/EnginesPanel';
  export { ModelPicker } from './src/components/ModelPicker';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const h = m.createElement;

// Shaped like SessionView in src/server/engine_login.rs.
const session = (over = {}) => ({
  session_id: 'lgn_1',
  state: 'waiting_for_code',
  detail: null,
  url: 'https://claude.com/cai/oauth/authorize?code=true&client_id=x',
  expires_at: '2026-10-17T21:10:00Z',
  ...over,
});

/** Swaps in a fake fetch; returns the calls it saw. */
function mockFetch(answer) {
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body ?? null });
    const [status, body] = answer(url, init);
    return new Response(body === undefined ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  };
  return calls;
}

test('start, poll, cancel and logout hit the engine sign-in routes', async () => {
  const calls = mockFetch((url, init) => {
    if (init.method === 'DELETE') return [204, undefined];
    if (url.endsWith('/logout')) return [200, { engines: [] }];
    return [200, session()];
  });
  await m.claudeLoginApi.start();
  await m.claudeLoginApi.get('lgn_1');
  await m.claudeLoginApi.cancel('lgn_1');
  assert.deepEqual(await m.claudeLoginApi.logout(), { engines: [] });
  assert.deepEqual(
    calls.map((c) => `${c.method} ${c.url}`),
    ['POST /api/engines/claude/login', 'GET /api/engines/claude/login/lgn_1', 'DELETE /api/engines/claude/login/lgn_1', 'POST /api/engines/claude/logout'],
  );
});

test('the code goes only in the POST body, never in the URL', async () => {
  const calls = mockFetch(() => [200, session({ state: 'done', url: null })]);
  const s = await m.claudeLoginApi.submitCode('lgn_1', 'abc123#state-xyz');
  assert.equal(s.state, 'done');
  assert.equal(calls[0].method, 'POST');
  assert.equal(calls[0].url, '/api/engines/claude/login/lgn_1/code');
  assert.ok(!calls[0].url.includes('abc123'));
  assert.deepEqual(JSON.parse(calls[0].body), { code: 'abc123#state-xyz' });
});

test('a 409 carries the running sign-in, so the UI can pick it up', async () => {
  mockFetch(() => [409, { error: 'a Claude sign-in is already running', session_id: 'lgn_busy' }]);
  await assert.rejects(m.claudeLoginApi.start(), (e) => {
    assert.ok(e instanceof m.ClaudeLoginError);
    assert.equal(e.status, 409);
    assert.equal(e.sessionId, 'lgn_busy');
    return true;
  });
});

test('server errors keep their message; a dead server says so', async () => {
  mockFetch(() => [400, { error: 'That doesn’t look like a code.' }]);
  await assert.rejects(m.claudeLoginApi.submitCode('lgn_1', 'x'), { message: 'That doesn’t look like a code.', status: 400 });
  mockFetch(() => [502, null]);
  await assert.rejects(m.claudeLoginApi.logout(), { message: 'HTTP 502' });
  globalThis.fetch = async () => {
    throw new TypeError('fetch failed');
  };
  await assert.rejects(m.claudeLoginApi.start(), { message: 'Couldn’t reach the server.', status: 0 });
});

test('states: which are still running, and what a failure says', () => {
  assert.deepEqual(
    ['starting', 'waiting_for_code', 'verifying', 'done', 'failed', 'expired', 'cancelled'].filter(m.isLoginActive),
    ['starting', 'waiting_for_code', 'verifying'],
  );
  assert.equal(m.loginFailure(session({ state: 'failed', detail: "Claude didn't accept that code. Start sign-in again." })), "Claude didn't accept that code. Start sign-in again.");
  assert.equal(m.loginFailure(session({ state: 'failed' })), 'Sign-in failed. Start it again.');
  assert.equal(m.loginFailure(session({ state: 'expired' })), 'Sign-in timed out. Start it again.');
  assert.equal(m.loginFailure(session({ state: 'cancelled' })), null);
  assert.equal(m.loginFailure(session({ state: 'done' })), null);
});

test('only https links on Anthropic hosts are opened', () => {
  assert.ok(m.isClaudeSignInUrl('https://claude.com/cai/oauth/authorize?x=1'));
  assert.ok(m.isClaudeSignInUrl('https://claude.ai/oauth/authorize'));
  assert.ok(m.isClaudeSignInUrl('https://platform.claude.com/oauth/code/callback'));
  assert.ok(!m.isClaudeSignInUrl('http://claude.ai/oauth/authorize'));
  assert.ok(!m.isClaudeSignInUrl('https://claude.ai.evil.example/oauth'));
  assert.ok(!m.isClaudeSignInUrl('https://notclaude.ai/'));
  assert.ok(!m.isClaudeSignInUrl('javascript:alert(1)'));
  assert.ok(!m.isClaudeSignInUrl(null));
});

// ---- Rendering ----

const engine = (id, over = {}) => ({
  id,
  name: { claude: 'Claude Code CLI', anthropic_api: 'Anthropic API key', codex: 'Codex CLI' }[id],
  kind: id.endsWith('_api') ? 'api' : 'cli',
  status: 'ready',
  detail: null,
  version: null,
  models: [],
  checked_at: '2026-10-17T19:00:00Z',
  enabled: true,
  ...over,
});
const api = (engines) => ({ engines, error: null, rechecking: false, recheck: async () => {}, reload: async () => {}, setEnabled: async () => {}, apply: () => {} });
const panel = (engines) => m.renderToStaticMarkup(h(m.EnginesPanel, { api: api(engines), now: Date.parse('2026-10-17T19:01:00Z') }));
const signedOut = engine('claude', { status: 'signed_out', detail: 'Not signed in: run `claude login`' });

test('claudeNeedsSignIn: only an installed, switched-on, signed-out Claude CLI', () => {
  assert.ok(m.claudeNeedsSignIn([signedOut]));
  assert.ok(!m.claudeNeedsSignIn([engine('claude')]));
  assert.ok(!m.claudeNeedsSignIn([{ ...signedOut, enabled: false }]));
  assert.ok(!m.claudeNeedsSignIn([engine('claude', { status: 'not_installed' })]));
  assert.ok(!m.claudeNeedsSignIn([engine('anthropic_api', { status: 'signed_out' })]));
  assert.ok(!m.claudeNeedsSignIn(null));
});

test('a signed-out Claude row offers "Sign in with Claude"; the summary points at it', () => {
  const html = panel([signedOut, engine('codex', { status: 'not_installed' })]);
  assert.match(html, /Sign in with Claude<\/button>/);
  assert.match(html, /Opens claude.ai in a new tab/);
  assert.match(html, /No engine can run right now\. Sign in with Claude below, or add an account\./);
  assert.doesNotMatch(html, />Sign out</);
  // The button replaces "run `claude login`".
  assert.doesNotMatch(html, /claude login/);
});

test('a ready Claude row offers Sign out instead', () => {
  const html = panel([engine('claude')]);
  assert.match(html, /Sign out<\/button>/);
  assert.doesNotMatch(html, /Sign in with Claude/);
});

test('no account actions on a switched-off, missing or other engine', () => {
  for (const list of [[{ ...signedOut, enabled: false }], [engine('claude', { status: 'not_installed' })], [engine('codex', { status: 'signed_out' })], [engine('anthropic_api')]]) {
    const html = panel(list);
    assert.doesNotMatch(html, /Sign in with Claude<\/button>/);
    assert.doesNotMatch(html, />Sign out</);
  }
});

const picker = (value, engines, extra = {}) =>
  m.renderToStaticMarkup(h(m.ModelPicker, { value, onChange: () => {}, engines, onOpenSettings: () => {}, onSignInClaude: () => {}, ...extra }));

test('a kwant pinned to Claude links to the sign-in when Claude is signed out', () => {
  const html = picker('claude/opus', [signedOut, engine('codex')]);
  assert.match(html, /Claude can&#x27;t run right now/);
  assert.match(html, /Sign in with Claude/);
  assert.doesNotMatch(html, /Open Settings/);
});

test('"No engine can run" links to the sign-in when Claude is the fix', () => {
  const html = picker('auto', [signedOut]);
  assert.match(html, /No engine can run right now/);
  assert.match(html, /Claude Code is installed but signed out\./);
  assert.match(html, /Sign in with Claude/);
});

test('other reasons keep "Open Settings"', () => {
  // Pinned to Codex, which is signed out: the Claude link would be wrong.
  assert.match(picker('codex', [signedOut, engine('codex', { status: 'signed_out' })]), /Open Settings/);
  // Claude missing rather than signed out.
  const missing = picker('claude', [engine('claude', { status: 'not_installed', detail: 'Claude Code isn’t installed' })]);
  assert.match(missing, /Open Settings/);
  assert.doesNotMatch(missing, /Sign in with Claude/);
  // No onSignInClaude from the caller: fall back to Settings.
  assert.match(picker('claude', [signedOut], { onSignInClaude: undefined }), /Open Settings/);
});
