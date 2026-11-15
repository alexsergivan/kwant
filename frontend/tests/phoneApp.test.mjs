// 20h: "Use kwant bots on your phone" in Settings › All spaces. Which state this address gets
// (one-time sign-in QR, address QR, loopback, plain http), what each says, and that a loopback or
// plain-http address never gets a QR.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { PhoneAppCardView, usesPhoneLayout } from './src/components/PhoneAppCard';
  import { AddDeviceView } from './src/components/DeviceSignIn';
  export * from './src/pwa';
  export { usesPhoneLayout };
  const noop = () => {};
  export const card = (props) => renderToStaticMarkup(React.createElement(PhoneAppCardView, props));
  // The wired card passes AddDevice and InstallAppButton; here the views stand in for them.
  export const cardWith = (props, addState) =>
    renderToStaticMarkup(React.createElement(PhoneAppCardView, {
      ...props,
      signIn: addState ? React.createElement(AddDeviceView, { state: addState, variant: 'phone', now: 0, onShow: noop, onHide: noop, onCopy: noop }) : null,
      installFooter: React.createElement('button', { type: 'button' }, 'Install on this computer'),
    }));
  export const addDevice = (state, variant) =>
    renderToStaticMarkup(React.createElement(AddDeviceView, { state, variant, now: 0, onShow: noop, onHide: noop, onCopy: noop }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;|’/g, "'").replace(/&amp;/g, '&').replace(/\s+/g, ' ').replace(/ ([,.])/g, '$1');
const kind = (hostname, protocol = 'https:', authRequired = false, deviceLogin = false) => m.phoneAppKind({ hostname, protocol, authRequired, deviceLogin });

// ---------------------------------------------------------------- the origin helper

test('loopback: 127.x, localhost, *.localhost, ::1 and 0.0.0.0 are only this computer', () => {
  for (const h of ['127.0.0.1', '127.1.2.3', 'localhost', 'LOCALHOST', 'kwant.localhost', '::1', '[::1]', '0.0.0.0']) assert.ok(m.isLoopbackHost(h), h);
  for (const h of ['192.168.1.20', '10.0.0.2', 'kwant.jumpin.games', 'localhost.example.com', '128.0.0.1', '127.0.0', 'mylocalhost', '::2'])
    assert.ok(!m.isLoopbackHost(h), h);
});

test('phoneAppKind: loopback first, then plain http, then sign-in or the address', () => {
  assert.equal(kind('127.0.0.1', 'http:'), 'loopback', 'the Mac app This Mac');
  assert.equal(kind('localhost', 'https:', true, true), 'loopback', 'loopback even over https with sign-in');
  assert.equal(kind('[::1]', 'http:'), 'loopback');
  assert.equal(kind('192.168.1.20', 'http:'), 'insecure');
  assert.equal(kind('kwant.example.com', 'http:', true, true), 'insecure', 'sign-in does not rescue plain http');
  assert.equal(kind('kwant.example.com', 'https:', true, true), 'signin');
  assert.equal(kind('kwant.example.com', 'https:', false, false), 'address');
  assert.equal(kind('kwant.example.com', 'https:', true, false), 'address', 'token sign-in without device login');
  assert.equal(kind('kwant.example.com', 'https:', false, true), 'address');
});

test('phones and tablets use the phone layout; a desktop UA only when narrow', () => {
  const ua = (userAgent, maxTouchPoints = 0) => ({ userAgent, maxTouchPoints });
  assert.ok(m.usesPhoneLayout(ua('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)', 5), false));
  assert.ok(m.usesPhoneLayout(ua('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari', 5), false), 'iPadOS says Mac');
  assert.ok(m.usesPhoneLayout(ua('Mozilla/5.0 (Linux; Android 15; SM-X710) Chrome/154'), false), 'Android tablet');
  assert.ok(!m.usesPhoneLayout(ua('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/154'), false));
  assert.ok(m.usesPhoneLayout(ua('Mozilla/5.0 (Windows NT 10.0) Chrome/154'), true));
});

// ---------------------------------------------------------------- the card on a computer

const showing = { kind: 'showing', link: { url: 'https://kwant.example/#kwnt_token=kl1.secret', expiresAt: 15 * 60_000 } };
const base = { layout: 'computer', host: 'kwant.example.com', origin: 'https://kwant.example.com', machine: 'computer' };
const qrs = (html) => (html.match(/role="img"/g) ?? []).length;

test('address: a QR of this address at once, Copy link, the three steps and the guide', () => {
  const html = m.card({ ...base, kind: 'address' });
  const t = text(html);
  assert.match(html, /id="phone-app"/);
  assert.match(html, /data-phone-app="address"/);
  assert.match(t, /Use kwant bots on your phone/);
  assert.match(t, /It opens kwant bots at kwant\.example\.com on the phone\./);
  assert.equal(qrs(html), 1);
  assert.match(html, /aria-label="QR code for this kwant bots address"/);
  assert.match(html, /value="https:\/\/kwant\.example\.com\/"/);
  assert.match(t, /Copy link/);
  assert.match(t, /Add it to the Home Screen/);
  assert.match(t, /iPhone or iPad: in Safari, tap Share › Add to Home Screen\./);
  assert.match(t, /Android: in Chrome, tap › Add to Home screen \(or Install app\)\./);
  assert.match(t, /Turn on notifications/);
  assert.match(t, /Settings › All spaces › Notifications › Notify me on this device/);
  assert.match(html, /href="https:\/\/kwant\.si\/docs\/phone\.html"[^>]*target="_blank"[^>]*rel="noopener noreferrer"/);
  assert.doesNotMatch(t, /Works once|Anyone who scans/, 'the address is not a secret');
  assert.doesNotMatch(t, /access token/);
});

test('address with a token sign-in says to sign in with the access token there', () => {
  assert.match(text(m.card({ ...base, kind: 'address', tokenSignIn: true })), /Sign in there with your access token\./);
});

test('copy: Copied, or the copy-it-yourself line', () => {
  assert.match(text(m.card({ ...base, kind: 'address', copied: 'yes' })), /Copied/);
  assert.match(text(m.card({ ...base, kind: 'address', copied: 'failed' })), /Couldn't copy\. Select the link and copy it yourself\./);
});

test('signin: step 1 is the AddDevice slot, never an address QR', () => {
  const html = m.cardWith({ ...base, kind: 'signin' }, { kind: 'idle' });
  const t = text(html);
  assert.match(t, /It opens kwant bots on the phone and signs it in once\./);
  assert.match(t, /Show QR code/);
  assert.equal(qrs(html), 0, 'no QR until Show QR code mints a link');
  assert.doesNotMatch(html, /data-phone-qr/);
  const shown = m.cardWith({ ...base, kind: 'signin' }, showing);
  assert.equal(qrs(shown), 1, 'one QR: the one-time link, which opens and signs in');
  assert.match(shown, /kwnt_token=kl1\.secret/);
  assert.doesNotMatch(shown, /value="https:\/\/kwant\.example\.com\/"/, 'not the bare address too');
});

test('loopback: no QR anywhere, says this Mac, links to the guide and Telegram', () => {
  const html = m.card({ ...base, kind: 'loopback', host: '127.0.0.1:8799', origin: 'http://127.0.0.1:8799', machine: 'Mac' });
  const t = text(html);
  assert.equal(qrs(html), 0);
  assert.doesNotMatch(html, /127\.0\.0\.1:8799\/"/, 'no address field to copy');
  assert.match(t, /Your phone can't reach this one\. kwant bots runs at 127\.0\.0\.1:8799, an address only this Mac can open\./);
  assert.match(t, /To have it on your phone:/);
  assert.match(html, /href="https:\/\/kwant\.si\/docs\/phone\.html"/);
  assert.match(t, /How to set it up/);
  assert.match(html, /href="https:\/\/kwant\.si\/docs\/telegram\.html"/);
  assert.match(t, /Set up Telegram/);
  assert.match(text(m.card({ ...base, kind: 'loopback', host: 'localhost:5173', machine: 'computer' })), /only this computer can open/);
});

test('insecure: phones need https, the proxy link, no QR', () => {
  const html = m.card({ ...base, kind: 'insecure', host: '192.168.1.20:8799', origin: 'http://192.168.1.20:8799' });
  const t = text(html);
  assert.equal(qrs(html), 0);
  assert.match(t, /Phones only install apps from an https address\. This one is http:\/\/192\.168\.1\.20:8799, so a phone can open it but can't add it/);
  assert.match(html, /href="https:\/\/kwant\.si\/docs\/install\.html#proxy"/);
});

test('the install footer shows only when given, and never on the phone layout', () => {
  assert.doesNotMatch(text(m.card({ ...base, kind: 'address' })), /Install on this computer/);
  for (const kind of ['address', 'signin', 'loopback', 'insecure']) assert.match(text(m.cardWith({ ...base, kind })), /Install on this computer/, kind);
  assert.doesNotMatch(text(m.cardWith({ ...base, layout: 'phone', kind: 'address', phoneCase: 'ios' })), /Install on this computer/);
});

// ---------------------------------------------------------------- the card on a phone

const phone = { ...base, layout: 'phone' };

test('phone: installs this phone, never a QR', () => {
  for (const phoneCase of ['prompt', 'ios', 'android', 'insecure']) {
    const html = m.card({ ...phone, kind: 'signin', phoneCase });
    assert.equal(qrs(html), 0, phoneCase);
    assert.match(text(html), /Add kwant bots to your Home Screen/, phoneCase);
  }
});

test('phone: Install button, iPhone Share steps, Android menu steps, or https first', () => {
  const prompt = text(m.card({ ...phone, kind: 'address', phoneCase: 'prompt' }));
  assert.match(prompt, /Install/);
  assert.match(prompt, /Then open it from your Home Screen and turn on Notify me on this device, under Notifications below\./);
  const ios = text(m.card({ ...phone, kind: 'address', phoneCase: 'ios' }));
  assert.match(ios, /Tap Share/);
  assert.match(ios, /In Safari it's at the bottom of the screen; on iPad, at the top\./);
  assert.match(ios, /Add to Home Screen, then Add/);
  assert.match(ios, /No Add to Home Screen\? Open this page in Safari\./);
  const android = text(m.card({ ...phone, kind: 'address', phoneCase: 'android' }));
  assert.match(android, /in the browser's menu/);
  assert.match(android, /Tap Add to Home screen \(or Install app\)/);
  const http = text(m.card({ ...phone, kind: 'insecure', origin: 'http://192.168.1.20:8799', phoneCase: 'insecure' }));
  assert.match(http, /Phones only install apps from an https address\. This one is http:\/\/192\.168\.1\.20:8799\./);
  assert.doesNotMatch(http, /Tap /);
});

// ---------------------------------------------------------------- AddDevice


test('Add a phone or browser now says to add it to the Home Screen under the QR', () => {
  assert.match(text(m.addDevice(showing, 'settings')), /On a phone, then add it to the Home Screen: Share › Add to Home Screen on iPhone, ⋮ › Add to Home screen on Android\./);
  assert.doesNotMatch(text(m.addDevice(showing, 'phone')), /On a phone, then add it/, 'the phone card has step 2 for that');
});

test('the phone variant: Show QR code as the primary button, and the next step once signed in', () => {
  const idle = m.addDevice({ kind: 'idle' }, 'phone');
  assert.match(text(idle), /Show QR code/);
  assert.match(idle, /bg-sky-600/);
  assert.match(text(m.addDevice({ kind: 'idle' }, 'settings')), /Add a phone or browser/);
  assert.match(text(m.addDevice({ kind: 'signed_in', browser: 'Safari on iPhone' }, 'phone')), /Next, on the phone: add it to the Home Screen \(step 2\)\./);
  assert.doesNotMatch(text(m.addDevice({ kind: 'signed_in', browser: 'x' }, 'settings')), /Next, on the phone/);
  assert.match(m.addDevice(showing, 'phone'), /id="add-device-link-phone"/);
  assert.match(m.addDevice(showing, 'settings'), /id="add-device-link-settings"/, 'two AddDevice on one page get their own ids');
});
