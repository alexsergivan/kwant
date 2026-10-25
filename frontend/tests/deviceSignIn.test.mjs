// Signing in from another device: the new browser shows a short code and waits; a signed-in
// device approves or denies it; Settings lists signed-in browsers with Sign out.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { DeviceRequestCard, DeviceWaitView, SessionsList } from './src/components/DeviceSignIn';
  import { TokenForm } from './src/components/AuthGate';
  export * from './src/deviceSignIn';
  const noop = () => {};
  export const wait = (state, now) => renderToStaticMarkup(React.createElement(DeviceWaitView, { state, now, onStart: noop, onCancel: noop }));
  export const card = (props) => renderToStaticMarkup(React.createElement(DeviceRequestCard, { more: 0, busy: null, error: null, onApprove: noop, onDeny: noop, onLater: noop, ...props }));
  export const sessions = (state, now, revoking = null) => renderToStaticMarkup(React.createElement(SessionsList, { state, now, revoking, onRevoke: noop, onRetry: noop }));
  export const form = (deviceLogin) => renderToStaticMarkup(React.createElement(TokenForm, { notice: null, onSignedIn: noop, deviceLogin }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;|’/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&').replace(/\s+/g, ' ');
const buttons = (html) => [...html.matchAll(/<button[^>]*>(.*?)<\/button>/g)].map((b) => text(b[1]).trim());
const NOW = Date.parse('2026-10-25T12:00:00Z');
const request = { device_code: 'kd1.secret', user_code: 'BCDF-2345', expires_in: 600, interval: 2 };

/** A fetch that answers each call from `answers` in turn ([status, body]) and records the calls. */
function fake(...answers) {
  const calls = [];
  const send = async (url, init = {}) => {
    calls.push({ url, init });
    const [status, body, headers] = answers.shift() ?? [500, null];
    return new Response(body === null ? null : JSON.stringify(body), { status, headers });
  };
  return { calls, send };
}

// ---------------------------------------------------------------- the sign-in card

test('with device login, "Sign in from another device" leads the card; the link and the token follow', () => {
  const html = m.form(true);
  const t = text(html);
  assert.equal(buttons(html)[0], 'Sign in from another device');
  assert.ok(t.indexOf('Sign in from another device') < t.indexOf('Or open the one-time sign-in link'));
  assert.ok(t.indexOf('kwnt signin-link') < t.indexOf('Other setups'));
  assert.match(html.slice(html.indexOf('<details')), /Paste a sign-in link or the access token/);
});

test('an older server without device login: no device button, the link leads', () => {
  const t = text(m.form(false));
  assert.ok(!t.includes('Sign in from another device'));
  assert.match(t, /Open the one-time sign-in link/);
});

// ---------------------------------------------------------------- the new browser's states

test('waiting: the code, what to do with it, the time left and Cancel', () => {
  const html = m.wait({ kind: 'waiting', request, expiresAt: NOW + 9 * 60_000 + 41_000 }, NOW);
  const t = text(html);
  assert.match(t, /where you're already signed in to kwant bots, open it and approve this code/);
  assert.match(t, /BCDF-2345/);
  assert.match(t, /Waiting for approval · 9:41 left/);
  assert.match(html, /role="status"/);
  assert.match(html, /aria-live="polite"/);
  assert.deepEqual(buttons(html), ['Cancel']);
  assert.ok(!html.includes('kd1.secret'), 'the device secret is never shown');
});

test('approved: signing in', () => {
  assert.match(text(m.wait({ kind: 'approved' }, NOW)), /Approved\. Signing you in/);
});

test('denied, expired, too many and errors say what happened and offer a new code', () => {
  const cases = [
    [{ kind: 'denied' }, /denied on your other device.*still signed out/],
    [{ kind: 'expired' }, /expired before it was approved\. Codes work for 10 minutes/],
    [{ kind: 'too_many', retryAfter: 45 }, /Too many sign-in requests right now\. Try again in 1 min/],
    [{ kind: 'error', message: 'Could not reach the server.' }, /Could not reach the server/],
  ];
  for (const [state, re] of cases) {
    const html = m.wait(state, NOW);
    assert.match(text(html), re, state.kind);
    assert.match(html, /role="alert"/, state.kind);
    assert.deepEqual(buttons(html), ['Get a new code'], state.kind);
  }
});

test('idle and starting', () => {
  assert.deepEqual(buttons(m.wait({ kind: 'idle' }, NOW)), ['Sign in from another device']);
  assert.match(m.wait({ kind: 'starting' }, NOW), /disabled=""/);
});

test('countdown never goes below zero', () => {
  assert.equal(m.countdown(NOW + 61_000, NOW), '1:01');
  assert.equal(m.countdown(NOW - 5000, NOW), '0:00');
});

// ---------------------------------------------------------------- the API, new browser side

test('start: POST /api/auth/device; 429 gives a wait', async () => {
  const ok = fake([200, request]);
  assert.deepEqual(await m.startDeviceRequest(ok.send), { kind: 'started', request });
  assert.equal(ok.calls[0].url, '/api/auth/device');
  assert.equal(ok.calls[0].init.method, 'POST');
  const busy = fake([429, { error: 'too many', retry_after: 30 }]);
  assert.deepEqual(await m.startDeviceRequest(busy.send), { kind: 'too_many', retryAfter: 30 });
  await assert.rejects(m.startDeviceRequest(fake([500, {}]).send), /500/);
});

test('poll: pending, approved (once), denied (403) and expired (410)', async () => {
  const f = fake([200, { status: 'pending' }], [200, { status: 'approved' }], [403, { status: 'denied' }], [410, { status: 'expired' }]);
  assert.equal(await m.pollDeviceRequest('kd1.secret', f.send), 'pending');
  assert.equal(await m.pollDeviceRequest('kd1.secret', f.send), 'approved');
  assert.equal(await m.pollDeviceRequest('kd1.secret', f.send), 'denied');
  assert.equal(await m.pollDeviceRequest('kd1.secret', f.send), 'expired');
  assert.equal(f.calls[0].url, '/api/auth/device/poll');
  assert.deepEqual(JSON.parse(f.calls[0].init.body), { device_code: 'kd1.secret' });
});

// ---------------------------------------------------------------- the signed-in device

const pending = { user_code: 'BCDF-2345', browser: 'Safari on iPhone', ip: '203.0.113.7', created_at: '2026-10-25T11:58:00Z', expires_at: '2026-10-25T12:08:00Z' };

test('the approval card: code, browser, IP, when, the warning, and Approve held until the code matches', () => {
  const html = m.card({ request: pending, now: NOW });
  const t = text(html);
  assert.match(html, /role="alertdialog"/);
  assert.match(html, /aria-modal="true"/);
  assert.match(t, /New device wants access/);
  assert.match(t, /BCDF-2345/);
  assert.match(t, /Safari on iPhone/);
  assert.match(t, /IP 203\.0\.113\.7 · asked 2 min ago/);
  assert.match(t, /Approve only if this exact code is on a screen you're holding right now/);
  assert.match(t, /If someone asked you to approve a code, deny it/);
  assert.match(t, /The code matches the one on my other device/);
  assert.match(html, /type="checkbox"/);
  const approve = html.match(/<button[^>]*>(?:(?!<\/button>).)*Approve<\/button>/)[0];
  assert.match(approve, /disabled=""/, 'Approve waits for the checkbox');
  const deny = html.match(/<button[^>]*>(?:(?!<\/button>).)*Deny<\/button>/)[0];
  assert.ok(!/disabled=""/.test(deny), 'Deny is always available');
  assert.match(html, /aria-label="Decide later"/);
});

test('the approval card: more waiting, busy and errors', () => {
  assert.match(text(m.card({ request: pending, now: NOW, more: 2 })), /2 more requests are waiting/);
  assert.match(text(m.card({ request: pending, now: NOW, more: 1 })), /1 more request is waiting/);
  const busy = m.card({ request: pending, now: NOW, busy: 'deny' });
  assert.deepEqual(buttons(busy).filter(Boolean), ['Deny', 'Approve'], 'labels stay while busy');
  assert.match(busy.match(/<button[^>]*>(?:(?!<\/button>).)*Deny<\/button>/)[0], /disabled=""/);
  assert.match(text(m.card({ request: pending, now: NOW, error: 'Could not reach the server.' })), /Could not reach the server/);
});

test('the signed-in side: list quietly, approve and deny by user code, 404 is "gone"', async () => {
  const f = fake([200, [pending]], [204, null], [204, null], [404, { error: 'no waiting sign-in request' }]);
  assert.deepEqual(await m.listDeviceRequests(f.send), [pending]);
  assert.equal(f.calls[0].url, '/api/signin/requests');
  assert.equal(f.calls[0].init.kwntQuiet, true, 'a background check never opens the sign-in screen');
  assert.equal(await m.decideDeviceRequest('BCDF-2345', true, f.send), 'ok');
  assert.equal(f.calls[1].url, '/api/signin/requests/approve');
  assert.deepEqual(JSON.parse(f.calls[1].init.body), { user_code: 'BCDF-2345' });
  assert.equal(await m.decideDeviceRequest('BCDF-2345', false, f.send), 'ok');
  assert.equal(f.calls[2].url, '/api/signin/requests/deny');
  assert.equal(await m.decideDeviceRequest('BCDF-2345', true, f.send), 'gone');
});

test('a browser that may not see requests (401/403) or an older server (404) gets null, not an error', async () => {
  for (const status of [401, 403, 404]) assert.equal(await m.listDeviceRequests(fake([status, {}]).send), null, String(status));
});

// ---------------------------------------------------------------- Settings: signed-in devices

const sessionRows = [
  { id: 's1', browser: 'Chrome on Mac', ip: '198.51.100.2', via: 'link', created_at: '2026-10-20T10:00:00Z', last_seen: '2026-10-25T11:59:30Z', current: true },
  { id: 's2', browser: 'Safari on iPhone', ip: '203.0.113.7', via: 'device', created_at: '2026-10-24T10:00:00Z', last_seen: '2026-10-24T09:00:00Z', current: false },
  { id: 's3', browser: 'Firefox on Linux', ip: '192.0.2.9', via: 'token', created_at: '2026-10-01T10:00:00Z', last_seen: '2026-10-25T09:00:00Z', current: false },
];

test('sessions: each browser with how it signed in, IP and last activity; this device has no Sign out', () => {
  const html = m.sessions({ kind: 'ready', sessions: sessionRows }, NOW);
  const t = text(html);
  assert.match(t, /Chrome on Mac This device Signed in with a link · IP 198\.51\.100\.2 · active just now/);
  assert.match(t, /Safari on iPhone Approved from another device · IP 203\.0\.113\.7 · active 1 day ago/);
  assert.match(t, /Firefox on Linux Signed in with the token · IP 192\.0\.2\.9 · active 3 h ago/);
  assert.deepEqual(buttons(html), ['Sign out', 'Sign out']);
  assert.match(html, /aria-label="Sign out Safari on iPhone"/);
});

test('sessions: loading, empty, error with Try again, and a revoke in progress', () => {
  assert.match(m.sessions({ kind: 'loading' }, NOW), /role="status"/);
  assert.match(text(m.sessions({ kind: 'ready', sessions: [] }, NOW)), /No browsers are signed in with a session yet/);
  const err = m.sessions({ kind: 'error', message: 'The server answered 500.' }, NOW);
  assert.match(text(err), /Couldn't load signed-in devices: The server answered 500\./);
  assert.deepEqual(buttons(err), ['Try again']);
  const revoking = m.sessions({ kind: 'ready', sessions: sessionRows }, NOW, 's2');
  assert.match(revoking.match(/<button[^>]*aria-label="Sign out Safari on iPhone"[^>]*>/)[0], /disabled=""/);
});

test('sessions API: list uncached, revoke by id, an already-gone session is fine', async () => {
  const f = fake([200, sessionRows], [204, null], [404, {}], [500, {}]);
  assert.deepEqual(await m.listSessions(f.send), sessionRows);
  assert.equal(f.calls[0].url, '/api/signin/sessions');
  assert.equal(f.calls[0].init.cache, 'no-store');
  await m.revokeSession('s2', f.send);
  assert.equal(f.calls[1].url, '/api/signin/sessions/s2');
  assert.equal(f.calls[1].init.method, 'DELETE');
  await m.revokeSession('s2', f.send);
  await assert.rejects(m.revokeSession('s2', f.send), /500/);
});

test('words for how a session was made, and how long ago', () => {
  assert.equal(m.viaLabel('link'), 'Signed in with a link');
  assert.equal(m.viaLabel('device'), 'Approved from another device');
  assert.equal(m.viaLabel('token'), 'Signed in with the token');
  assert.equal(m.ago('2026-10-25T11:59:50Z', NOW), 'just now');
  assert.equal(m.ago('2026-10-25T11:55:00Z', NOW), '5 min ago');
  assert.equal(m.ago('2026-10-22T12:00:00Z', NOW), '3 days ago');
  assert.equal(m.ago('nonsense', NOW), '');
});
