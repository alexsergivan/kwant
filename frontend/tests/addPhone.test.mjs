// "Add a phone or browser" in Settings → Signed-in devices: a one-time link and its QR code,
// drawn in this browser, with a countdown, Hide and New code; hidden in local/no-token mode.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { AddDeviceView } from './src/components/DeviceSignIn';
  export * from './src/deviceSignIn';
  const noop = () => {};
  export const view = (state, now, copied = null) =>
    renderToStaticMarkup(React.createElement(AddDeviceView, { state, now, copied, onShow: noop, onHide: noop, onCopy: noop }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;|’/g, "'").replace(/&amp;/g, '&').replace(/\s+/g, ' ');
const buttons = (html) => [...html.matchAll(/<button[^>]*>(.*?)<\/button>/g)].map((b) => text(b[1]).trim());
const NOW = Date.parse('2026-10-25T12:00:00Z');
const URL_ = 'https://kwant.example/#kwnt_token=kl1.secretcode';
const showing = { kind: 'showing', link: { url: URL_, expiresAt: NOW + 15 * 60_000 } };

function fake(...answers) {
  const calls = [];
  const send = async (url, init = {}) => {
    calls.push({ url, init });
    const [status, body] = answers.shift() ?? [500, null];
    return new Response(body === null ? null : JSON.stringify(body), { status });
  };
  return { calls, send };
}

// ---------------------------------------------------------------- minting the link

test('a link is minted with POST /api/signin/links; url null is built from this page', async () => {
  const { calls, send } = fake([200, { code: 'kl1.abc', url: null, expires_at: '2026-10-25T12:15:00Z', expires_in: 900 }]);
  const link = await m.createSigninLink('https://box.example', NOW, send);
  assert.equal(calls[0].url, '/api/signin/links');
  assert.equal(calls[0].init.method, 'POST');
  assert.equal(link.url, 'https://box.example/#kwnt_token=kl1.abc');
  assert.equal(link.expiresAt, NOW + 900_000);
});

test('the server-given url wins when kwnt knows its public address', async () => {
  const { send } = fake([200, { code: 'kl1.abc', url: 'https://public.example/#kwnt_token=kl1.abc', expires_in: 900 }]);
  assert.equal((await m.createSigninLink('http://10.0.0.2:8899', NOW, send)).url, 'https://public.example/#kwnt_token=kl1.abc');
});

test('a refused or empty answer is an error, never a half link', async () => {
  await assert.rejects(m.createSigninLink('https://x', NOW, fake([403, { error: 'person only' }]).send), /403/);
  await assert.rejects(m.createSigninLink('https://x', NOW, fake([200, { url: null }]).send), /no sign-in code/);
});

// ---------------------------------------------------------------- the states

test('before showing: just the button, no QR and no code on the page', () => {
  const html = m.view({ kind: 'idle' }, NOW);
  assert.deepEqual(buttons(html), ['Add a phone or browser']);
  assert.ok(!html.includes('<svg role'), 'no QR');
  assert.ok(!html.includes('kwnt_token'));
});

test('showing: the QR, the copyable link, the warning, 15:00 left, New code and Hide', () => {
  const html = m.view(showing, NOW);
  const t = text(html);
  assert.match(html, /aria-label="QR code for the one-time sign-in link"/);
  assert.match(html, /<path d="M\d/);
  assert.match(html, new RegExp(`value="${URL_.replace(/[.]/g, '\\.')}"`));
  assert.match(t, /Anyone who scans this can sign in once\. Don't share or screenshot it\./);
  assert.match(t, /Works once · 15:00 left/);
  assert.deepEqual(buttons(html), ['Copy link', 'New code', 'Hide']);
});

test('the countdown runs down with the clock', () => {
  assert.match(text(m.view(showing, NOW + (14 * 60 + 1) * 1000)), /0:59 left/);
});

test('copy feedback: Copied, or a hint when the clipboard refused', () => {
  assert.ok(buttons(m.view(showing, NOW, 'yes')).includes('Copied'));
  assert.match(text(m.view(showing, NOW, 'failed')), /Couldn't copy\. Select the link and copy it yourself\./);
});

test('expired: the code and QR are gone; New code and Hide remain', () => {
  const html = m.view({ kind: 'expired' }, NOW);
  assert.ok(!html.includes('kwnt_token') && !html.includes('<svg role'));
  assert.match(text(html), /The code expired, so it was hidden/);
  assert.deepEqual(buttons(html), ['New code', 'Hide']);
});

test('signed in: "Signed in ✓" with the browser, no QR, and Add another', () => {
  const html = m.view({ kind: 'signed_in', browser: 'Safari on iPhone' }, NOW);
  assert.match(text(html), /Signed in ✓ Safari on iPhone signed in with the code/);
  assert.ok(!html.includes('kwnt_token'));
  assert.deepEqual(buttons(html), ['Add another', 'Done']);
});

test('a failed mint says so and keeps the button', () => {
  const html = m.view({ kind: 'error', message: 'The server answered 403.' }, NOW);
  assert.match(text(html), /Couldn't make a sign-in code: The server answered 403\./);
  assert.deepEqual(buttons(html), ['Add a phone or browser']);
});

// ---------------------------------------------------------------- "Signed in ✓" detection

test('only a new link session counts as the phone that scanned', () => {
  const s = (id, via, current = false) => ({ id, via, current, browser: id, ip: '', created_at: '', last_seen: '' });
  const known = new Set(['a', 'b']);
  assert.equal(m.newLinkSession([s('a', 'link', true), s('b', 'link')], known), null, 'old sessions');
  assert.equal(m.newLinkSession([s('a', 'token', true), s('c', 'device')], known), null, 'device approval is not this code');
  assert.equal(m.newLinkSession([s('a', 'token', true), s('c', 'link')], known)?.id, 'c');
});

// ---------------------------------------------------------------- the QR itself

test('the QR is drawn locally, with a quiet zone, and is the same for the same link', () => {
  const a = m.qrPath(URL_);
  assert.ok(a.size >= 29 + 8 - 4, `size ${a.size}`); // version ≥ 3 for this length, 4-module border
  assert.ok(a.d.startsWith('M4 4h1v1h-1z'), 'finder pattern starts after the border');
  assert.deepEqual(m.qrPath(URL_), a);
  assert.notEqual(m.qrPath(URL_ + 'x').d, a.d);
});

test('no QR service and nothing logged: the component never fetches an image or uses console', () => {
  const src = readFileSync(new URL('../src/components/DeviceSignIn.tsx', import.meta.url), 'utf8');
  assert.ok(!/console\./.test(src));
  assert.ok(!/<img[^>]*qr/i.test(src) && !/api\.qrserver|chart\.googleapis|quickchart/i.test(src));
  assert.ok(!/localStorage|sessionStorage/.test(src.slice(src.indexOf('Add a phone or browser'))));
});

// ---------------------------------------------------------------- local mode

test('hidden on the Mac in local/no-token mode and on servers without device login', () => {
  assert.equal(m.signedInDevicesVisible(false, true), false, 'local / no sign-in');
  assert.equal(m.signedInDevicesVisible(true, false), false, 'older server');
  assert.equal(m.signedInDevicesVisible(true, true), true);
  const settings = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  assert.match(settings, /\{signedInDevicesVisible\(authRequired, deviceLogin\) && <SignedInDevices \/>\}/);
});
