// Saved passwords after the audit (2026-10-20): the list carries no passwords, Reveal fetches one
// for 30 seconds, a caught login waits for Save / Don't save, person-only 401s ask for the token,
// and bot settings say so when tools run without asking.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedPasswordsPanel, SavedLoginRow, PendingLoginPrompt, PasswordsProblemNotice } from './src/components/SavedPasswordsPanel';
  import { PersonTokenDialog } from './src/components/AuthGate';
  import { KwantToolsPicker } from './src/components/KwantToolsPicker';
  export * from './src/savedPasswords';
  export * as auth from './src/auth';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  const state = (p) => ({ logins: [], pending: null, loading: false, problem: null, reload() {}, ...p });
  export const panel = (s, p) => r(SavedPasswordsPanel, { sandboxId: 'k1', state: state(s), pageUrl: '', ...p });
  export const row = (p) => r(SavedLoginRow, { sandboxId: 'k1', onThisPage: false, onDeleted() {}, ...p });
  export const pendingPrompt = (pending) => r(PendingLoginPrompt, { sandboxId: 'k1', pending, onDone() {} });
  export const problem = (problem) => r(PasswordsProblemNotice, { problem, onRetry() {} });
  export const tokenDialog = () => r(PersonTokenDialog, { onClose() {}, onSignedIn() {} });
  export const tools = (p) => r(KwantToolsPicker, { model: 'claude:sonnet', selected: [], onChange() {}, ...p });
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((x) => text(x[1]));
const login = (p) => ({ id: 'c1', origin: 'https://example.com', domain: 'example.com', username: 'alice', created_at: '2026-10-20T07:00:00Z', last_used: null, ...p });

// ---------------------------------------------------------------- the model

test('the list keeps only metadata, even if a server still sends a password', () => {
  const l = m.parsePasswordsList({
    passwords: [{ id: 'c1', origin: 'https://example.com', domain: 'example.com', username: 'alice', password: 'DUMMY-SECRET', created_at: 'x' }, { nope: 1 }],
    pending: { origin: 'https://example.com', username: 'alice', captured_at: 't', password: 'DUMMY-SECRET' },
  });
  assert.equal(l.passwords.length, 1);
  assert.equal(JSON.stringify(l).includes('DUMMY-SECRET'), false);
  assert.deepEqual(l.pending, { origin: 'https://example.com', username: 'alice', captured_at: 't' });
  assert.deepEqual(m.parsePasswordsList({ passwords: [] , pending: null }), { passwords: [], pending: null });
  assert.deepEqual(m.parsePasswordsList(null), { passwords: [], pending: null });
});

test('"this page" means the exact origin: scheme, host and port', () => {
  const l = { origin: 'https://example.com' };
  assert.equal(m.loginMatchesPage(l, 'https://example.com/login?x=1'), true);
  assert.equal(m.loginMatchesPage(l, 'http://example.com/login'), false, 'http is another origin');
  assert.equal(m.loginMatchesPage(l, 'https://evil.example.com/'), false, 'subdomains are other origins');
  assert.equal(m.loginMatchesPage(l, 'https://example.com:8443/'), false, 'another port');
  assert.equal(m.loginMatchesPage(l, 'https://example.com.evil.test/'), false);
  assert.equal(m.loginMatchesPage(l, 'about:blank'), false);
  assert.equal(m.loginMatchesPage(l, ''), false);
});

test('the Site field becomes an origin; a bare host means https', () => {
  assert.equal(m.originFromInput('example.com'), 'https://example.com');
  assert.equal(m.originFromInput(' https://example.com/login '), 'https://example.com');
  assert.equal(m.originFromInput('http://localhost:3000/x'), 'http://localhost:3000');
  assert.equal(m.originFromInput('ftp://example.com'), null);
  assert.equal(m.originFromInput('javascript://alert(1)'), null);
  assert.equal(m.originFromInput('   '), null);
  assert.equal(m.originLabel('https://example.com'), 'example.com');
  assert.equal(m.originLabel('http://example.com:8080'), 'http://example.com:8080');
});

test('server answers map to the panel states, including locked for a store that unlocks', () => {
  assert.equal(m.problemFor(401, { person_only: true }), 'needs-token');
  assert.equal(m.problemFor(423, { locked: true }), 'locked');
  assert.equal(m.problemFor(503, {}), 'unavailable');
  assert.equal(m.problemFor(500, {}), 'error');
  assert.equal(m.REVEAL_MS, 30_000);
});

test('the API: list is quiet and uncached, reveal is a POST, save sends the origin', async () => {
  const calls = [];
  const prev = globalThis.fetch;
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url, ...init });
    if (url.endsWith('/reveal')) return new Response(JSON.stringify({ password: 'DUMMY-1' }), { status: 200 });
    if (url.endsWith('/passwords') && (init.method ?? 'GET') === 'GET') return new Response(JSON.stringify({ passwords: [login()], pending: null }), { status: 200 });
    return new Response(JSON.stringify({ status: 'saved' }), { status: 200 });
  };
  try {
    await m.passwordsApi.list('k 1');
    assert.equal(calls[0].url, '/api/sandbox/k%201/passwords');
    assert.equal(calls[0].kwntQuiet, true, 'a background load shows its own token state');
    assert.equal(calls[0].cache, 'no-store');
    assert.equal(await m.passwordsApi.reveal('k1', 'c1'), 'DUMMY-1');
    assert.equal(calls[1].method, 'POST');
    assert.equal(calls[1].url, '/api/sandbox/k1/passwords/c1/reveal');
    // Quiet, because with Bitwarden a wrong master password is also a 401; a person-only 401 asks for the token itself.
    assert.equal(calls[1].kwntQuiet, true, 'a wrong master password must not open the app-wide gate');
    assert.equal(calls[1].kwntAskToken, true, 'an explicit action still asks for the token');
    await m.passwordsApi.add('k1', 'https://example.com', 'alice', 'DUMMY-2');
    assert.deepEqual(JSON.parse(calls[2].body), { origin: 'https://example.com', username: 'alice', password: 'DUMMY-2' });
    await m.passwordsApi.savePending('k1');
    await m.passwordsApi.discardPending('k1');
    await m.passwordsApi.remove('k1', 'c1');
    assert.deepEqual(calls.slice(3).map((c) => `${c.method} ${c.url}`), [
      'POST /api/sandbox/k1/passwords/pending',
      'DELETE /api/sandbox/k1/passwords/pending',
      'DELETE /api/sandbox/k1/passwords/c1',
    ]);
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'x', person_only: true }), { status: 401 });
    await assert.rejects(m.passwordsApi.reveal('k1', 'c1'), (e) => e.problem === 'needs-token');
    globalThis.fetch = async () => new Response(JSON.stringify({ locked: true }), { status: 423 });
    await assert.rejects(m.passwordsApi.list('k1'), (e) => e.problem === 'locked');
  } finally {
    globalThis.fetch = prev;
  }
});

// ---------------------------------------------------------------- the list

test('a row shows site and username, the password only as dots with a Reveal button', () => {
  const html = m.row({ login: login() });
  const t = text(html);
  assert.match(t, /example\.com/);
  assert.match(t, /alice/);
  assert.match(html, /••••/);
  assert.ok(buttons(html).includes('Reveal'));
  assert.ok(!html.includes('Copy password'), 'nothing to copy before Reveal');
  assert.ok(!html.includes('data-revealed'));
});

test('a revealed row shows the password, a Hide button and when it hides again', () => {
  const html = m.row({ login: login(), initialRevealed: 'DUMMY-3' });
  assert.match(html, /data-revealed[^>]*>DUMMY-3</);
  assert.ok(buttons(html).includes('Hide'));
  assert.match(text(html), /Hides again in 30 seconds/);
  assert.match(html, /aria-label="Copy password"/);
});

test('Delete asks first, with Keep focused', () => {
  const html = m.row({ login: login(), initialConfirmDelete: true });
  assert.match(text(html), /Delete this login\? The bots can't sign in to example\.com/);
  const b = buttons(html);
  assert.ok(b.includes('Keep') && b.includes('Delete'));
  assert.match(html, /<button[^>]*autofocus=""[^>]*>Keep/i);
  assert.ok(!text(m.row({ login: login() })).includes('Delete this login'));
});

test('the exact-origin row is marked "This page" in words, not colour alone', () => {
  assert.match(text(m.row({ login: login(), onThisPage: true })), /This page/);
  assert.ok(!text(m.row({ login: login(), onThisPage: false })).includes('This page'));
});

test('the panel lists logins and marks only the exact origin', () => {
  const html = m.panel({ logins: [login(), login({ id: 'c2', origin: 'http://example.com', username: 'bob' })] }, { pageUrl: 'https://example.com/login' });
  assert.equal((html.match(/data-login-row=/g) || []).length, 2);
  assert.equal((text(html).match(/This page/g) || []).length, 1);
  assert.match(text(html), /http:\/\/example\.com/);
});

test('empty, loading and problem states', () => {
  assert.match(text(m.panel({})), /No saved logins yet/);
  assert.match(text(m.panel({ loading: true })), /Loading saved logins/);
  const tok = m.panel({ problem: 'needs-token' });
  assert.match(text(tok), /need your access token/);
  assert.ok(buttons(tok).includes('Enter access token'));
  assert.ok(!buttons(tok).some((b) => b.includes('Add')), 'no add form while the list is unreachable');
  assert.match(text(m.problem('locked')), /locked/i);
  assert.ok(buttons(m.problem('locked')).includes('Try again'));
});

test('the add form takes a site and says the exact origin it fills on', () => {
  const html = m.panel({}, { initialAdding: true });
  assert.match(text(html), /Site/);
  assert.match(html, /type="password"/);
  assert.match(html, /autoComplete="new-password"|autocomplete="new-password"/);
  assert.ok(buttons(html).includes('Save login'));
});

// ---------------------------------------------------------------- the waiting capture

test('a caught login asks "Save password for <site> as <user>?" with Save and Don\'t save', () => {
  const html = m.pendingPrompt({ origin: 'https://example.com', username: 'alice', captured_at: null });
  assert.match(text(html), /Save password for example\.com as alice \?|Save password for example\.com as alice\?/);
  assert.deepEqual(buttons(html), ["Don't save", 'Save']);
  assert.match(text(html), /Nothing is saved until you choose Save/);
  assert.match(text(m.pendingPrompt({ origin: 'http://example.com:8080', username: '', captured_at: null })), /Save password for http:\/\/example\.com:8080 \?|Save password for http:\/\/example\.com:8080\?/);
});

// ---------------------------------------------------------------- the token screen

test('a person-only 401 asks for the token; any other 401 goes to the gate; quiet loads ask nothing', async () => {
  assert.equal(m.auth.isPersonOnlyBody({ person_only: true }), true);
  assert.equal(m.auth.isPersonOnlyBody({ error: 'API token required' }), false);
  assert.equal(m.auth.isPersonOnlyBody(null), false);

  const seen = [];
  const win = new EventTarget();
  win.location = { origin: 'http://127.0.0.1:18982', hash: '', pathname: '/', search: '' };
  let answer = { status: 401, body: { person_only: true } };
  win.fetch = async () => new Response(JSON.stringify(answer.body), { status: answer.status });
  for (const e of [m.auth.PERSON_TOKEN_EVENT, m.auth.UNAUTHORIZED_EVENT]) win.addEventListener(e, () => seen.push(e));
  const prevWindow = globalThis.window;
  globalThis.window = win;
  try {
    m.auth.installUnauthorizedListener();
    const res = await win.fetch('/api/sandbox/k1/passwords/c1/reveal', { method: 'POST' });
    assert.deepEqual(await res.json(), { person_only: true }, 'the caller still reads the body');
    assert.deepEqual(seen, [m.auth.PERSON_TOKEN_EVENT]);
    await win.fetch('/api/sandbox/k1/passwords', { kwntQuiet: true });
    assert.equal(seen.length, 1, 'quiet: no prompt');
    answer = { status: 401, body: { error: 'API token required' } };
    await win.fetch('/api/kwants');
    assert.deepEqual(seen, [m.auth.PERSON_TOKEN_EVENT, m.auth.UNAUTHORIZED_EVENT]);
  } finally {
    globalThis.window = prevWindow;
  }
});

test('the token dialog says why it is asked from another computer and can be dismissed', () => {
  const html = m.tokenDialog();
  assert.match(html, /role="dialog"/);
  assert.match(html, /aria-modal="true"/);
  const t = text(html);
  assert.match(t, /Confirm it's you/);
  assert.match(t, /deleting bots or teams need you signed in from another computer, so a bot can't do them for you/);
  assert.ok(!t.includes('even on this computer'), 'this computer never asks');
  assert.match(t, /Open the one-time sign-in link/);
  assert.match(t, /Other setups/);
  assert.match(t, /kwnt token --raw/);
  assert.ok(buttons(html).includes('Not now'));
  assert.ok(buttons(html).includes('Continue'));
});

// ---------------------------------------------------------------- the hint

test('bot settings: the tools hint says "runs without asking" when approvals are off', () => {
  assert.match(text(m.tools({})), /asks you before each use/);
  const off = text(m.tools({ approvalsOff: true }));
  assert.match(off, /runs them without asking \(approvals off\)/);
  assert.ok(!off.includes('asks you before each use'));
});
