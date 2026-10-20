// Saved passwords in Bitwarden (2026-10-20): the store picker and connect form, Locked → Unlock,
// Reveal with the master password, "Move N to Bitwarden", and the plain note about autofill with
// approvals off. The master password is read from its input at submit and never kept.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedPasswordsPanel, SavedLoginRow, PasswordsProblemNotice } from './src/components/SavedPasswordsPanel';
  import { PasswordStoreBar, BitwardenLoginForm, MigrateButton, AutofillGapNote } from './src/components/PasswordStorePanel';
  import { OffConfirm } from './src/components/ApprovalsSetting';
  export * from './src/passwordStore';
  export { passwordsApi, problemFor } from './src/savedPasswords';
  export { OFF_CONSEQUENCES } from './src/approvalMode';
  export * as auth from './src/auth';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  const lists = (p) => ({ logins: [], pending: null, loading: false, problem: null, reload() {}, ...p });
  export const storeState = (status) => ({ status, problem: null, reload() {}, set() {} });
  export const panel = (s, p) => r(SavedPasswordsPanel, { sandboxId: 'k1', state: lists(s), pageUrl: '', ...p });
  export const row = (p) => r(SavedLoginRow, { sandboxId: 'k1', onThisPage: false, onDeleted() {}, ...p });
  export const problem = (problem, p) => r(PasswordsProblemNotice, { problem, onRetry() {}, ...p });
  export const bar = (status, p) => r(PasswordStoreBar, { store: storeState(status), onUnlock() {}, onChanged() {}, ...p });
  export const form = (p) => r(BitwardenLoginForm, { mode: 'connect', status: null, onDone() {}, onCancel() {}, ...p });
  export const migrate = (p) => r(MigrateButton, { count: 0, onDone() {}, ...p });
  export const gap = () => r(AutofillGapNote, {});
  export const offConfirm = () => r(OffConfirm, { title: 'Turn off?', busy: false, onCancel() {}, onConfirm() {} });
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((x) => text(x[1]));
const inputs = (html) => [...html.matchAll(/<input[^>]*>/g)].map((x) => x[0]);
const login = (p) => ({ id: 'c1', origin: 'https://example.com', domain: 'example.com', username: 'alice', created_at: null, last_used: null, ...p });
const bw = (p) => ({ server_url: 'https://vault.bitwarden.com', email: 'dummy@example.com', login: 'password', client_id: null, unlocked: true, unlocked_at: 't', item_count: 4, last_sync: 't', ...p });
const LOCAL = { store: 'local', locked: false, local_count: 3, bitwarden: null };
const UNLOCKED = { store: 'bitwarden', locked: false, local_count: 0, bitwarden: bw() };
const LOCKED = { store: 'bitwarden', locked: true, local_count: 0, bitwarden: bw({ unlocked: false, unlocked_at: null, item_count: null }) };

// ---------------------------------------------------------------- the model

test('store status: local, Bitwarden unlocked, Bitwarden locked (also when only "unlocked" says so)', () => {
  assert.deepEqual(m.parseStoreStatus({ store: 'local', locked: false, local_count: 3, bitwarden: null }), LOCAL);
  const u = m.parseStoreStatus({ store: 'bitwarden', locked: false, local_count: 0, bitwarden: { ...bw(), extra: 'x' } });
  assert.equal(u.locked, false);
  assert.equal(u.bitwarden.email, 'dummy@example.com');
  assert.equal(m.parseStoreStatus({ store: 'bitwarden', locked: false, bitwarden: bw({ unlocked: false }) }).locked, true);
  assert.equal(m.parseStoreStatus({ store: 'bitwarden', locked: true, bitwarden: bw() }).locked, true);
  assert.deepEqual(m.parseStoreStatus(null), { store: 'local', locked: false, local_count: 0, bitwarden: null });
  assert.equal(m.parseStoreStatus({ store: 'local', bitwarden: bw({ login: 'api_key', client_id: 'user.x' }) }).bitwarden.login, 'api_key');
});

test('self-hosted address: https, or http only on this computer; nothing after the address', () => {
  assert.deepEqual(m.checkServerUrl('vault.example.com'), { url: 'https://vault.example.com' });
  assert.deepEqual(m.checkServerUrl(' https://vault.example.com/bw/ '), { url: 'https://vault.example.com/bw' });
  assert.deepEqual(m.checkServerUrl('http://127.0.0.1:18990'), { url: 'http://127.0.0.1:18990' });
  assert.deepEqual(m.checkServerUrl('http://localhost:8080'), { url: 'http://localhost:8080' });
  assert.ok('error' in m.checkServerUrl('http://vault.example.com'), 'plain http off this computer');
  assert.ok('error' in m.checkServerUrl('https://vault.example.com/?x=1'));
  assert.ok('error' in m.checkServerUrl('https://u:p@vault.example.com'));
  assert.ok('error' in m.checkServerUrl(''));
  assert.equal(m.serverLabel('https://vault.bitwarden.eu'), 'bitwarden.eu');
  assert.equal(m.serverLabel('http://127.0.0.1:18990'), '127.0.0.1:18990');
});

test('two-step methods: the ones with a code to type come first; "remember" is dropped', () => {
  assert.deepEqual(m.twoFactorChoices(['7', '0', '5', 1]), [
    { id: 0, name: 'Authenticator app', usable: true },
    { id: 1, name: 'Email', usable: true },
    { id: 7, name: 'Passkey or security key', usable: false },
  ]);
  assert.deepEqual(m.twoFactorChoices(null), []);
});

test('unlock answers: token, wrong password, two-step, new device, other account, bad input, unreachable', () => {
  assert.deepEqual(m.unlockProblemFor(401, { person_only: true }), { kind: 'needs-token' });
  assert.deepEqual(m.unlockProblemFor(401, { wrong_password: true, error: 'x' }), { kind: 'wrong-password' });
  assert.equal(m.unlockProblemFor(401, { needs: 'two_factor', providers: ['0'] }).kind, 'two-factor');
  assert.deepEqual(m.unlockProblemFor(401, { needs: 'new_device_otp' }), { kind: 'new-device-code' });
  assert.deepEqual(m.unlockProblemFor(409, {}), { kind: 'other-account' });
  assert.deepEqual(m.unlockProblemFor(400, { error: 'the Bitwarden server must be an https address' }), { kind: 'invalid', message: 'the Bitwarden server must be an https address' });
  assert.equal(m.unlockProblemText(m.unlockProblemFor(400, { error: 'the master password is missing' })), 'The master password is missing.');
  assert.deepEqual(m.unlockProblemFor(502, {}), { kind: 'unreachable' });
  assert.deepEqual(m.unlockProblemFor(423, { locked: true }), { kind: 'locked' });
  assert.match(m.unlockProblemText({ kind: 'wrong-password' }), /five wrong tries/);
  assert.match(m.unlockProblemText({ kind: 'two-factor', providers: m.twoFactorChoices(['7']) }), /can't|can’t/);
  // The saved-passwords side: a wrong master password on Reveal is not "needs the token".
  assert.equal(m.problemFor(401, { wrong_password: true }), 'wrong-password');
  assert.equal(m.problemFor(401, { person_only: true }), 'needs-token');
});

test('the unlock body leaves out empty fields, and the two-step method only with a code', () => {
  assert.deepEqual(m.unlockBody({ master_password: 'DUMMY-MP', server_url: '', email: ' a@b.c ', client_id: '  ', two_factor_provider: 0 }), { master_password: 'DUMMY-MP', email: 'a@b.c' });
  assert.deepEqual(m.unlockBody({ master_password: 'DUMMY-MP', two_factor_provider: 0, two_factor_token: ' 123456 ' }), { master_password: 'DUMMY-MP', two_factor_token: '123456', two_factor_provider: 0 });
  assert.deepEqual(m.unlockBody({ master_password: ' DUMMY ', client_secret: ' s ' }), { master_password: ' DUMMY ', client_secret: ' s ' }, 'secrets are sent as typed');
});

test('the API: quiet, uncached, the documented routes; a person-only 401 asks for the token, a wrong password doesn\'t', async () => {
  const calls = [];
  const events = [];
  const prevFetch = globalThis.fetch;
  const prevWindow = globalThis.window;
  globalThis.window = new EventTarget();
  window.addEventListener(m.auth.PERSON_TOKEN_EVENT, () => events.push('person'));
  let next = null;
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url, ...init });
    if (next) return next;
    if (url.endsWith('/migrate')) return new Response(JSON.stringify({ moved: 2, failed: 1 }), { status: 200 });
    return new Response(JSON.stringify(UNLOCKED), { status: 200 });
  };
  try {
    assert.deepEqual(await m.storeApi.status(), UNLOCKED);
    await m.storeApi.unlock({ master_password: 'DUMMY-MP', server_url: 'http://127.0.0.1:18990', email: 'dummy@example.com' });
    await m.storeApi.lock();
    await m.storeApi.disconnect('DUMMY-MP');
    assert.deepEqual(await m.storeApi.migrate(), { moved: 2, failed: 1 });
    assert.deepEqual(calls.map((c) => `${c.method} ${c.url}`), [
      'GET /api/passwords/store',
      'POST /api/passwords/bitwarden/unlock',
      'POST /api/passwords/bitwarden/lock',
      'DELETE /api/passwords/bitwarden',
      'POST /api/passwords/migrate',
    ]);
    assert.ok(calls.every((c) => c.kwntQuiet === true && c.cache === 'no-store'), 'quiet: a 401 here is often a wrong master password');
    assert.deepEqual(JSON.parse(calls[1].body), { master_password: 'DUMMY-MP', server_url: 'http://127.0.0.1:18990', email: 'dummy@example.com' });
    assert.deepEqual(JSON.parse(calls[3].body), { master_password: 'DUMMY-MP' });

    next = new Response(JSON.stringify({ error: 'wrong master password', wrong_password: true }), { status: 401 });
    await assert.rejects(m.storeApi.unlock({ master_password: 'nope' }), (e) => e.problem.kind === 'wrong-password');
    assert.deepEqual(events, [], 'a wrong password is not a missing token');
    next = new Response(JSON.stringify({ error: 'x', person_only: true }), { status: 401 });
    await assert.rejects(m.storeApi.status(), (e) => e.problem.kind === 'needs-token');
    assert.deepEqual(events, ['person']);

    // Reveal in Bitwarden mode sends the master password, quietly.
    next = new Response(JSON.stringify({ password: 'DUMMY-PW' }), { status: 200 });
    assert.equal(await m.passwordsApi.reveal('k1', 'c1', 'DUMMY-MP'), 'DUMMY-PW');
    assert.deepEqual(JSON.parse(calls.at(-1).body), { master_password: 'DUMMY-MP' });
    next = new Response(JSON.stringify({ error: 'wrong master password', wrong_password: true }), { status: 401 });
    await assert.rejects(m.passwordsApi.reveal('k1', 'c1', 'nope'), (e) => e.problem === 'wrong-password' && /isn't right|isn’t right/.test(e.message));
    assert.deepEqual(events, ['person'], 'still no token prompt');
  } finally {
    globalThis.fetch = prevFetch;
    globalThis.window = prevWindow;
  }
});

test('migrate wording: moved, and what failed stays here', () => {
  assert.equal(m.migrateText({ moved: 3, failed: 0 }), 'Moved 3 passwords to Bitwarden.');
  assert.equal(m.migrateText({ moved: 1, failed: 2 }), 'Moved 1 password to Bitwarden. 2 couldn’t be moved and stay on this server; try again.');
});

// ---------------------------------------------------------------- the picker and the forms

test('the store bar: this server, Bitwarden unlocked with Lock, Bitwarden locked with Unlock', () => {
  const local = m.bar(LOCAL);
  assert.match(local, /data-store-bar="local"/);
  assert.match(text(local), /Saved on this server/);
  assert.deepEqual(buttons(local).length, 1, 'just the toggle');
  const open = m.bar(UNLOCKED);
  assert.match(text(open), /Bitwarden · dummy@example.com Unlocked/);
  assert.ok(buttons(open).includes('Lock'));
  const locked = m.bar(LOCKED);
  assert.match(locked, /data-store-bar="locked"/);
  assert.match(text(locked), /Locked/);
  assert.ok(buttons(locked).includes('Unlock'));
  assert.equal(m.bar(null), '', 'nothing until the status is known');
});

test('the picker: two choices, said plainly; Bitwarden opens the connect form', () => {
  const html = m.bar(LOCAL, { initialOpen: true });
  assert.match(text(html), /Where passwords are kept, for every bot/);
  assert.match(text(html), /Saved on this server Encrypted here\. A bot with approvals off can read them, so use Bitwarden then\./);
  assert.match(text(html), /Bitwarden In your own Bitwarden account/);
  assert.equal(inputs(html).filter((i) => /type="radio"/.test(i)).length, 2);
  const connecting = m.bar(LOCAL, { initialOpen: true, initialConnecting: true });
  assert.match(connecting, /data-bitwarden-form="connect"/);
  const storeRadios = (h) => inputs(h).filter((i) => /type="radio"/.test(i) && /-store"/.test(i));
  assert.deepEqual(storeRadios(html).map((i) => /checked/.test(i)), [true, false], 'on this server by default');
  assert.deepEqual(storeRadios(connecting).map((i) => /checked/.test(i)), [false, true], 'one choice at a time: Bitwarden while connecting');
});

test('connect form: server (cloud or self-hosted), email, password or API key; secrets are uncontrolled', () => {
  const html = m.form({});
  assert.match(text(html), /Connect Bitwarden/);
  assert.match(html, /<option value="https:\/\/vault.bitwarden.com"[^>]*>bitwarden.com/);
  assert.match(html, /<option value="https:\/\/vault.bitwarden.eu"[^>]*>bitwarden.eu/);
  assert.match(html, /<option value="self"[^>]*>Self-hosted/);
  assert.match(text(html), /Account email/);
  assert.match(text(html), /Sign in with Master password API key \+ master password/);
  const pw = inputs(html).filter((i) => /type="password"/.test(i));
  assert.equal(pw.length, 1, 'just the master password without an API key');
  assert.doesNotMatch(pw[0], /value=/, 'never a controlled value');
  assert.match(pw[0], /autoComplete="off"|autocomplete="off"/);
  assert.match(text(html), /never saved/);
  assert.deepEqual(buttons(html), ['Cancel', 'Connect']);

  const key = m.form({ initialApiKey: true });
  assert.match(text(key), /client_id/);
  assert.equal(inputs(key).filter((i) => /type="password"/.test(i)).length, 2, 'client_secret is a password field too');
  assert.ok(inputs(key).filter((i) => /type="password"/.test(i)).every((i) => !/value=/.test(i)));

  const self = m.form({ initialServer: 'self' });
  assert.match(self, /placeholder="https:\/\/vault.example.com"/);
});

test('unlock form: says which account, asks for the master password (and the client secret for an API key)', () => {
  const html = m.form({ mode: 'unlock', status: LOCKED });
  assert.match(text(html), /Unlock Bitwarden dummy@example.com on bitwarden.com/);
  assert.doesNotMatch(text(html), /Account email|Server/, 'server and email are already set up');
  assert.deepEqual(buttons(html), ['Cancel', 'Unlock']);
  const apiKey = m.form({ mode: 'unlock', status: { ...LOCKED, bitwarden: bw({ login: 'api_key', client_id: 'user.x', unlocked: false }) } });
  assert.match(text(apiKey), /client_secret Asked for at every unlock, and never kept\./);
  assert.equal(inputs(apiKey).filter((i) => /type="password"/.test(i)).length, 2);
});

test('two-step login and new-device code are asked for in the same form', () => {
  const tf = m.form({ mode: 'unlock', status: LOCKED, initialStep: { kind: 'two-factor', providers: m.twoFactorChoices(['0', '1']) } });
  assert.match(tf, /data-step="two-factor"/);
  assert.match(text(tf), /Method/);
  assert.match(tf, /autoComplete="one-time-code"|autocomplete="one-time-code"/);
  assert.deepEqual(buttons(tf), ['Cancel', 'Continue']);
  const only = m.form({ mode: 'unlock', status: LOCKED, initialStep: { kind: 'two-factor', providers: m.twoFactorChoices(['0']) } });
  assert.doesNotMatch(text(only), /Method/, 'no picker for one method');
  const passkey = m.form({ mode: 'unlock', status: LOCKED, initialStep: { kind: 'two-factor', providers: m.twoFactorChoices(['7']) }, initialProblem: { kind: 'two-factor', providers: m.twoFactorChoices(['7']) } });
  assert.match(text(passkey), /Use an API key instead/);
  assert.match(passkey, /<button type="submit" disabled=""/);
  const device = m.form({ mode: 'unlock', status: LOCKED, initialStep: { kind: 'new-device-code' } });
  assert.match(text(device), /Code from Bitwarden's email/);
  const wrong = m.form({ mode: 'unlock', status: LOCKED, initialProblem: { kind: 'wrong-password' } });
  assert.match(wrong, /role="alert"/);
  assert.match(text(wrong), /isn't right/);
});

// ---------------------------------------------------------------- locked, reveal, migrate, disconnect

test('locked: the list says so and offers Unlock Bitwarden', () => {
  const html = m.problem('locked', { onUnlock() {} });
  assert.match(text(html), /Bitwarden is locked\. Unlock it to see saved passwords and fill them in\./);
  assert.deepEqual(buttons(html), ['Unlock Bitwarden']);
  assert.deepEqual(buttons(m.problem('locked')), ['Try again'], 'an older server without a store keeps Try again');
  const p = m.panel({ problem: 'locked' }, { store: m.storeState(LOCKED) });
  assert.match(p, /data-store-bar="locked"/);
  assert.ok(buttons(p).includes('Unlock Bitwarden'));
  assert.doesNotMatch(m.panel({ problem: 'needs-token' }, { store: m.storeState(LOCKED) }), /data-store-bar/, 'no bar until the token is in');
});

test('Reveal with Bitwarden asks for the master password in the row', () => {
  const plain = m.row({ login: login(), askMasterPassword: true });
  assert.doesNotMatch(plain, /data-ask-master/, 'asked only after Reveal');
  const asking = m.row({ login: login(), askMasterPassword: true, initialAsking: true });
  assert.match(asking, /data-ask-master/);
  assert.match(text(asking), /Bitwarden master password, to show it/);
  const field = inputs(asking).find((i) => /type="password"/.test(i));
  assert.ok(field && !/value=/.test(field), 'uncontrolled');
  assert.ok(buttons(asking).includes('Show') && buttons(asking).includes('Cancel'));
  const panel = m.panel({ logins: [login()] }, { store: m.storeState(UNLOCKED) });
  assert.match(text(panel), /Kept in your Bitwarden, locked after every restart\./);
  assert.match(text(m.panel({ logins: [login()] }, { store: m.storeState(LOCAL) })), /Encrypted on the server/);
});

test('"Move N saved passwords to Bitwarden" only when unlocked with some left here; shows the result', () => {
  assert.deepEqual(buttons(m.migrate({ count: 3 })), ['Move 3 saved passwords to Bitwarden']);
  assert.deepEqual(buttons(m.migrate({ count: 1 })), ['Move 1 saved password to Bitwarden']);
  assert.deepEqual(buttons(m.migrate({ count: 0, initialResult: 'Moved 3 passwords to Bitwarden.' })), []);
  assert.match(m.migrate({ count: 0, initialResult: 'Moved 3 passwords to Bitwarden.' }), /role="status"/);
  const open = m.bar({ ...UNLOCKED, local_count: 2 }, { initialOpen: true });
  assert.ok(buttons(open).includes('Move 2 saved passwords to Bitwarden'));
  const locked = m.bar({ ...LOCKED, local_count: 2 }, { initialOpen: true });
  assert.ok(!buttons(locked).some((b) => b.startsWith('Move')));
  assert.match(text(locked), /2 passwords are still saved on this server\. Unlock to move them\./);
});

test('Disconnect asks for the master password and says the items stay in Bitwarden', () => {
  const html = m.bar(UNLOCKED, { initialOpen: true, initialDisconnecting: true });
  assert.match(html, /data-disconnect/);
  assert.match(text(html), /Your items stay in Bitwarden\./);
  assert.ok(buttons(html).includes('Keep Bitwarden') && buttons(html).includes('Disconnect'));
  assert.ok(inputs(html).some((i) => /type="password"/.test(i) && !/value=/.test(i)));
  assert.match(text(m.bar(LOCKED, { initialOpen: true, initialDisconnecting: true })), /Unlock Bitwarden first/);
  assert.match(text(m.bar(UNLOCKED, { initialOpen: true })), /Account dummy@example.com Server bitwarden.com Sign-in Master password Logins 4/);
});

// ---------------------------------------------------------------- the honest note

test('approvals off: one plain sentence about autofill, in the confirm and next to saved passwords', () => {
  const note = 'While a password is being filled into a page, a bot that can run commands can read it.';
  assert.equal(m.AUTOFILL_GAP_NOTE, note);
  assert.ok(m.OFF_CONSEQUENCES.includes(note));
  assert.ok(text(m.offConfirm()).includes(note));
  assert.ok(text(m.panel({ logins: [login()] }, { approvalsOff: true })).includes(note));
  assert.ok(!text(m.panel({ logins: [login()] }, { approvalsOff: false })).includes(note));
  assert.doesNotMatch(m.gap(), /rose|red-|amber/, 'no scare styling');
});

test('no master password in storage, logs or React state', () => {
  for (const f of ['src/passwordStore.ts', 'src/components/PasswordStorePanel.tsx', 'src/components/SavedPasswordsPanel.tsx']) {
    const src = readFileSync(new URL(`../${f}`, import.meta.url), 'utf8');
    assert.doesNotMatch(src, /localStorage|sessionStorage|console\./, f);
    assert.doesNotMatch(src, /const \[\s*\w*(master|secret)\w*\s*,/i, `${f}: no secret in state`);
    assert.doesNotMatch(src, /useState[^;\n]*(master|secret)/i, `${f}: no secret in state`);
  }
  // The check itself catches what it's for.
  assert.match('const [masterPassword, setMasterPassword] = useState(\'\');', /const \[\s*\w*(master|secret)\w*\s*,/i);
});
