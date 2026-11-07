// 0.7.8 Telegram (design §G): the Connections card, the setup sheet (token, pairing, paired), the
// Allow approvals confirm, Disconnect and Unpair, and the chat marks. Fake tokens and a mock API only:
// nothing here reaches Telegram.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/telegram';
  export { TelegramConnection, SetupSheet, ConnectedCard, CatalogCard, AllowConfirm } from './src/components/TelegramCard';
  export { ViaTelegram, SentToTelegram, ResolvedViaTelegram } from './src/components/TelegramMarks';
`);
const { React } = m;
const h = React.createElement;

const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
let root = null;
const mount = async (el) => {
  const host = document.createElement('ul');
  document.body.appendChild(host);
  root = m.createRoot(host);
  root.render(el);
  await settle();
  return host;
};
afterEach(() => {
  root?.unmount();
  root = null;
  document.body.innerHTML = '';
});
const $ = (sel) => document.querySelector(sel);
const byText = (text, sel = 'button') => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim() === text);
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (input, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const key = async (k) => {
  (document.activeElement ?? document.body).dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};

const T0 = Date.parse('2026-11-07T10:00:00Z');
const BOT = { username: 'shop_helper_bot', name: 'Shop helper' };
const SAM = { name: 'Sam', username: 'sam_example', since: '2026-11-07T10:04:00Z' };
const st = (over = {}) => m.parseStatus({ state: 'off', status: 'listening', ...over });
const saved = (over = {}) => st({ state: 'token_saved', bot: BOT, ...over });
const paired = (over = {}) => st({ state: 'paired', bot: BOT, paired: SAM, ...over });
const QR = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="2" height="2"/></svg>';
const LINK = { link: 'https://t.me/shop_helper_bot?start=FAKEcode123', expires_at: new Date(T0 + 600_000).toISOString(), qr_svg: QR };

/** A mock API: each call is recorded; behaviour per method can be swapped. */
const mockApi = (over = {}) => {
  const calls = [];
  const rec = (name, f) => async (...a) => (calls.push([name, ...a]), f(...a));
  const api = {
    status: rec('status', async () => st()),
    saveToken: rec('saveToken', async () => saved()),
    pair: rec('pair', async () => LINK),
    unpair: rec('unpair', async () => saved({ undo_until: new Date(Date.now() + 10_000).toISOString() })),
    undoUnpair: rec('undoUnpair', async () => paired()),
    patch: rec('patch', async (b) => paired({ allow_approvals: !!b.allow_approvals })),
    test: rec('test', async () => undefined),
    disconnect: rec('disconnect', async () => undefined),
  };
  for (const [k, f] of Object.entries(over)) api[k] = rec(k, f);
  return { api, calls };
};

// ------------------------------------------------------------------ helpers

test('status parses defensively: unknown states are off, paired needs state paired, Allow needs pairing', () => {
  const s = m.parseStatus({ state: 'weird', allow_approvals: true, kinds: { done: true, question: 'yes' } });
  assert.equal(s.state, 'off');
  assert.equal(s.allow_approvals, false);
  assert.equal(s.kinds.done, true);
  assert.equal(s.kinds.question, true);
  assert.equal(m.parseStatus({ state: 'token_saved', bot: BOT, paired: SAM }).paired, null);
  const p = m.parseStatus({ state: 'paired', bot: { username: '@shop_helper_bot', name: 'Shop helper' }, paired: SAM, allow_approvals: true, queued: -3 });
  assert.equal(p.bot.username, 'shop_helper_bot');
  assert.equal(p.allow_approvals, true);
  assert.equal(p.queued, 0);
  assert.equal(m.pairedName(p.paired), 'Sam @sam_example');
  assert.equal(m.pairedName({ name: 'Sam', username: null }), 'Sam');
});

test('countdown is m:ss, never negative, and is spoken only at 5:00 and 1:00', () => {
  assert.equal(m.countdown(LINK.expires_at, T0).text, '10:00');
  assert.equal(m.countdown(LINK.expires_at, T0 + 61_500).text, '8:59');
  assert.equal(m.countdown(LINK.expires_at, T0 + 700_000).secs, 0);
  assert.equal(m.countdown(null, T0).text, '0:00');
  assert.deepEqual([600, 300, 299, 60, 59, 0].filter(m.announceAt), [300, 60]);
});

test('only https t.me links open, and the QR is an image source, never markup', () => {
  assert.equal(m.safeLink(LINK.link), LINK.link);
  assert.equal(m.safeLink('http://t.me/x'), null);
  assert.equal(m.safeLink('https://t.me.evil.example.com/x'), null);
  assert.equal(m.safeLink('javascript:alert(1)'), null);
  assert.equal(m.shortLink(LINK.link), 't.me/shop_helper_bot?start=…');
  assert.match(m.qrSource(QR), /^data:image\/svg\+xml;charset=utf-8,%3Csvg/);
  assert.equal(m.qrSource('<img src=x onerror=alert(1)>'), null);
  assert.equal(m.qrSource(''), null);
});

test('token errors: rejected and can\'t-reach read in our words; others pass the server sentence', () => {
  assert.deepEqual(m.tokenErrorText(new m.TelegramError('x', 400, 'token_rejected')), { text: m.COPY.rejected, net: false });
  assert.deepEqual(m.tokenErrorText(new m.TelegramError('x', 400, 'token_shape')), { text: m.COPY.rejected, net: false });
  assert.deepEqual(m.tokenErrorText(new m.TelegramError('x', 502, 'telegram_unreachable')), { text: m.COPY.unreachable, net: true });
  assert.equal(m.tokenErrorText(new m.TelegramError('Locked.', 423, 'vault_locked')).text, 'Locked.');
});

test('the status line: listening, offline with a queue, throttled, conflict, rejected', () => {
  assert.deepEqual(m.statusLine(paired()), { tone: 'ok', text: 'Listening' });
  assert.match(m.statusLine(paired({ status: 'offline', queued: 3 })).text, /^Paused: this computer is offline\. 3 messages wait and go out/);
  assert.match(m.statusLine(paired({ status: 'offline', queued: 1 })).text, /1 message waits and goes out/);
  assert.equal(m.statusLine(paired({ status: 'throttled' })).tone, 'muted');
  assert.equal(m.statusLine(paired({ error: 'conflict' })).tone, 'warn');
  assert.deepEqual(m.statusLine(st({ state: 'rejected', bot: BOT })), { tone: 'bad', text: 'Telegram rejected the token.' });
});

test('events from another space are ignored', () => {
  assert.equal(m.forThisSpace('spc_a', 'spc_a'), true);
  assert.equal(m.forThisSpace('spc_b', 'spc_a'), false);
  assert.equal(m.forThisSpace(undefined, 'spc_a'), true);
  assert.equal(m.forThisSpace('spc_b', null), true);
});

// ------------------------------------------------------------------ catalogue and setup

test('off: the catalogue card offers "Set up Telegram" with the privacy note', async () => {
  const { api } = mockApi();
  await mount(h(m.TelegramConnection, { api }));
  const card = $('[data-catalog="telegram"]');
  assert.ok(card);
  assert.match(card.textContent, /Message your bots from Telegram/);
  assert.match(card.textContent, /aren't end-to-end encrypted/);
  const btn = byText('Set up Telegram');
  assert.equal(btn.getAttribute('aria-haspopup'), 'dialog');
  await click(btn);
  const sheet = $('[role="dialog"]');
  assert.ok(sheet, 'the sheet opens');
  assert.equal($('[data-tg-sheet]').dataset.tgSheet, 'token');
  // Step 1 is open; step 2 waits.
  assert.ok($('[data-tg-token]'));
  assert.equal($('[data-tg-token]').type, 'password');
  assert.equal($('[data-tg-token]').getAttribute('autocomplete'), 'off');
  assert.equal($('[data-tg-check]').disabled, true, 'Check is off until something is typed');
});

test('a rejected token keeps step 1, says so in our words, and ties the error to the field', async () => {
  const { api } = mockApi({ saveToken: async () => { throw new m.TelegramError('nope', 400, 'token_rejected'); } });
  await mount(h(m.TelegramConnection, { api, initial: st(), initialSheet: { start: 'token' } }));
  await type($('[data-tg-token]'), '123456:FAKE-token-for-tests-only');
  await click($('[data-tg-check]'));
  const err = $('[data-tg-error]');
  assert.equal(err.dataset.tgError, 'bad');
  assert.equal(err.getAttribute('role'), 'alert');
  assert.equal(err.textContent.trim(), m.COPY.rejected);
  const input = $('[data-tg-token]');
  assert.ok(input.getAttribute('aria-describedby').split(' ').includes(err.id));
  assert.equal(input.getAttribute('aria-invalid'), 'true');
  assert.equal(document.activeElement, input, 'focus goes back to the field');
});

test('can\'t reach Telegram is its own line; nothing moves on', async () => {
  const { api } = mockApi({ saveToken: async () => { throw new m.TelegramError('x', 502, 'telegram_unreachable'); } });
  await mount(h(m.TelegramConnection, { api, initial: st(), initialSheet: { start: 'token' } }));
  await type($('[data-tg-token]'), '123456:FAKE-token-for-tests-only');
  await click($('[data-tg-check]'));
  assert.equal($('[data-tg-error]').dataset.tgError, 'net');
  assert.match($('[data-tg-error]').textContent, /Couldn't reach Telegram from this computer/);
  assert.equal($('[data-tg-sheet]').dataset.tgSheet, 'token');
});

test('while checking, the button says so and the field is read-only', async () => {
  let release;
  const { api } = mockApi({ saveToken: () => new Promise((r) => (release = r)) });
  await mount(h(m.TelegramConnection, { api, initial: st(), initialSheet: { start: 'token' } }));
  await type($('[data-tg-token]'), '123456:FAKE-token-for-tests-only');
  await click($('[data-tg-check]'));
  assert.equal($('[data-tg-check]').disabled, true);
  assert.match($('[data-tg-check]').textContent, /Checking/);
  assert.ok($('[data-tg-token]').readOnly || $('[data-tg-token]').disabled);
  release(saved());
  await settle();
  assert.equal($('[data-tg-sheet]').dataset.tgSheet, 'pair');
});

test('a good token goes to pairing: link, QR as an image, countdown, copy', async () => {
  const { api, calls } = mockApi();
  const writes = [];
  Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (t) => writes.push(t) }, configurable: true });
  await mount(h(m.TelegramConnection, { api, initial: st(), initialSheet: { start: 'token', now: () => T0 } }));
  await type($('[data-tg-token]'), '123456:FAKE-token-for-tests-only');
  await click($('[data-tg-check]'));
  assert.deepEqual(calls.find((c) => c[0] === 'saveToken').slice(1, 2), ['123456:FAKE-token-for-tests-only']);
  assert.ok(calls.some((c) => c[0] === 'pair'), 'a link is made on entering step 2');
  assert.equal($('[data-tg-token]'), null, 'the token field is gone once saved');
  const qr = $('[data-tg-qr]');
  assert.equal(qr.getAttribute('role'), 'img');
  assert.equal(qr.getAttribute('aria-label'), 'QR code for the pairing link');
  assert.equal(qr.querySelector('svg'), null, 'the server SVG is never put into the page');
  assert.match(qr.querySelector('img').getAttribute('src'), /^data:image\/svg\+xml/);
  const open = [...document.querySelectorAll('a')].find((a) => /Open in Telegram/.test(a.textContent));
  assert.equal(open.getAttribute('href'), LINK.link);
  assert.match(open.getAttribute('rel') ?? '', /noopener/);
  assert.match($('[data-tg-wait]').textContent, /10:00/);
  assert.ok(!document.body.textContent.includes('FAKEcode123'), 'the start code is never printed');
  await click(byText('Copy link'));
  assert.deepEqual(writes, [LINK.link]);
  assert.match(document.body.textContent, /Link copied\. It works once, for 10 minutes\./);
});

test('the pairing link expires: the QR dims and "Make a new link" makes another', async () => {
  const { api, calls } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: saved(), initialSheet: { start: 'pair', now: () => T0, initialLink: LINK } }));
  window.dispatchEvent(new window.CustomEvent(m.TG_EVENT, { detail: { type: 'telegram.expired', data: {} } }));
  await settle();
  assert.match($('[data-tg-expired]').textContent, /This link expired after 10 minutes\. Nothing was paired\./);
  assert.ok($('[data-tg-qr]').classList.contains('dim'));
  const before = calls.filter((c) => c[0] === 'pair').length;
  await click($('[data-tg-newlink]'));
  assert.equal(calls.filter((c) => c[0] === 'pair').length, before + 1);
  assert.equal($('[data-tg-expired]'), null);
});

test('a countdown that reaches 0:00 expires on its own', async () => {
  const { api } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: saved(), initialSheet: { start: 'pair', now: () => T0 + 600_000, initialLink: LINK } }));
  assert.ok($('[data-tg-expired]'));
});

test('paired: the teal box replaces the QR, takes focus, and keeps approvals in the app', async () => {
  const { api } = mockApi({ status: async () => paired() });
  await mount(h(m.TelegramConnection, { api, initial: saved(), initialSheet: { start: 'pair', now: () => T0, initialLink: LINK } }));
  window.dispatchEvent(new window.CustomEvent(m.TG_EVENT, { detail: { type: 'telegram.paired', data: {} } }));
  await settle();
  const box = $('[data-tg-paired]');
  assert.ok(box);
  assert.equal($('[data-tg-qr]'), null);
  assert.match(box.textContent, /Sam/);
  assert.ok(box.contains(document.activeElement), 'focus moves to the paired heading');
  assert.match(document.body.textContent, /Approvals still happen in the app\./);
  await click(byText('Done'));
  assert.equal($('[role="dialog"]'), null);
});

test('a paired event whose status is not paired leaves the QR up', async () => {
  const { api } = mockApi({ status: async () => saved() });
  await mount(h(m.TelegramConnection, { api, initial: saved(), initialSheet: { start: 'pair', now: () => T0, initialLink: LINK } }));
  window.dispatchEvent(new window.CustomEvent(m.TG_EVENT, { detail: { type: 'telegram.paired', data: {} } }));
  await settle();
  assert.equal($('[data-tg-paired]'), null);
  assert.ok($('[data-tg-qr]'));
});

test('already paired when a link is asked for: straight to the paired box', async () => {
  const { api } = mockApi({ pair: async () => { throw new m.TelegramError('x', 409, 'already_paired'); }, status: async () => paired() });
  await mount(h(m.TelegramConnection, { api, initial: saved(), initialSheet: { start: 'pair', now: () => T0 } }));
  assert.ok($('[data-tg-paired]'));
});

test('Escape closes the sheet when nothing is typed', async () => {
  const { api } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: st(), initialSheet: { start: 'token' } }));
  $('[data-tg-token]').focus();
  await key('Escape');
  assert.equal($('[role="dialog"]'), null);
});

// ------------------------------------------------------------------ connected card

test('connected and paired: bot, paired with, where messages go, privacy, kinds, Allow off', async () => {
  const { api } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: paired({ last_message_at: '2026-11-07T10:06:00Z' }) }));
  const card = $('[data-catalog="telegram"]');
  assert.equal(card.dataset.tgState, 'paired');
  assert.match(card.textContent, /@shop_helper_bot · token in the vault/);
  assert.match(card.textContent, /Sam @sam_example/);
  assert.match(card.textContent, /#general, answered as in the app/);
  assert.equal($('[data-tg-status]').dataset.tgStatus, 'ok');
  assert.match($('[data-tg-status]').textContent, /Listening · last message/);
  const switches = [...document.querySelectorAll('[role="switch"]')];
  assert.equal(switches.length, 6, 'five kinds and Allow approvals');
  for (const s of switches) assert.ok(s.getAttribute('aria-labelledby'), 'each switch is labelled');
  assert.equal($('[data-tg-allow]').dataset.tgAllow, 'off');
  assert.match(card.textContent, /Telegram can't grant anything/);
  assert.equal($('[data-tg-never]'), null);
});

test('not paired yet: Pair opens the sheet at pairing; Allow and the test are off', async () => {
  const { api } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: saved() }));
  assert.match($('[data-catalog="telegram"]').textContent, /Not paired yet/);
  const allow = $('[data-tg-allow] [role="switch"]');
  assert.equal(allow.disabled, true);
  assert.match($('[data-tg-allow]').textContent, /Pair first to change it/);
  assert.equal(byText('Send a test message').disabled, true);
  await click($('[data-tg-pair]'));
  assert.equal($('[data-tg-sheet]').dataset.tgSheet, 'pair');
});

test('a rejected token shows the red line with Fix it, which opens step 1', async () => {
  const { api } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: st({ state: 'rejected', bot: BOT }) }));
  assert.equal($('[data-tg-status]').dataset.tgStatus, 'bad');
  await click(byText('Fix it'));
  assert.equal($('[data-tg-sheet]').dataset.tgSheet, 'token');
});

test('Allow approvals: a confirm with "Keep it off" focused; Escape keeps it off', async () => {
  const { api, calls } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: paired() }));
  await click($('[data-tg-allow] [role="switch"]'));
  const dlg = $('[role="alertdialog"]');
  assert.ok(dlg);
  assert.match(dlg.textContent, /Allow approvals from Telegram\?/);
  assert.match(dlg.textContent, /Only Allow once/);
  assert.equal(document.activeElement.textContent.trim(), 'Keep it off');
  await key('Escape');
  assert.equal($('[role="alertdialog"]'), null);
  assert.ok(!calls.some((c) => c[0] === 'patch'), 'nothing changed');
});

test('Allow approvals on: amber, "On since", and the never-from-Telegram line; off needs no confirm', async () => {
  const { api, calls } = mockApi({ patch: async (b) => paired({ allow_approvals: !!b.allow_approvals, allow_since: '2026-11-07T10:12:00Z' }) });
  await mount(h(m.TelegramConnection, { api, initial: paired() }));
  await click($('[data-tg-allow] [role="switch"]'));
  await click(byText('Allow from Telegram'));
  assert.deepEqual(calls.at(-1), ['patch', { allow_approvals: true }]);
  assert.equal($('[data-tg-allow]').dataset.tgAllow, 'on');
  assert.ok($('[data-tg-allow] [role="switch"]').classList.contains('armed'));
  assert.equal($('[data-tg-allow] [role="switch"]').getAttribute('aria-checked'), 'true');
  assert.match($('[data-tg-never]').textContent, /Never from Telegram: production connections, payment floors, human steps and Take over/);
  assert.match($('[data-tg-allow]').textContent, /On since/);
  await click($('[data-tg-allow] [role="switch"]'));
  assert.equal($('[role="alertdialog"]'), null);
  assert.deepEqual(calls.at(-1), ['patch', { allow_approvals: false }]);
});

test('a kind switch saves just that kind', async () => {
  const { api, calls } = mockApi({ patch: async () => paired({ kinds: { done: true } }) });
  await mount(h(m.TelegramConnection, { api, initial: paired() }));
  const done = [...document.querySelectorAll('.tg-kinds [role="switch"]')].at(-1);
  assert.equal(done.getAttribute('aria-checked'), 'false');
  await click(done);
  assert.deepEqual(calls.at(-1), ['patch', { kinds: { done: true } }]);
  assert.equal([...document.querySelectorAll('.tg-kinds [role="switch"]')].at(-1).getAttribute('aria-checked'), 'true');
});

test('Disconnect is an inline confirm, "Keep it" first; confirming goes back to the catalogue card', async () => {
  const { api, calls } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: paired() }));
  await click($('[data-tg-disconnect-open]'));
  const box = $('[data-tg-disconnect]');
  assert.match(box.textContent, /The token is deleted from the vault and Sam is unpaired/);
  assert.equal(document.activeElement.textContent.trim(), 'Keep it');
  await click(byText('Keep it'));
  assert.equal($('[data-tg-disconnect]'), null);
  await click($('[data-tg-disconnect-open]'));
  await click([...$('[data-tg-disconnect]').querySelectorAll('button')].find((b) => b.textContent.trim() === 'Disconnect'));
  assert.ok(calls.some((c) => c[0] === 'disconnect'));
  assert.ok(byText('Set up Telegram'), 'back to the catalogue card');
});

test('Unpair shows a toast with Undo, and Undo pairs again', async () => {
  const { api, calls } = mockApi();
  await mount(h(m.TelegramConnection, { api, initial: paired() }));
  await click(document.querySelector('[aria-label="Unpair Sam @sam_example"]'));
  assert.match(document.body.textContent, /Unpaired Sam\./);
  assert.match($('[data-catalog="telegram"]').textContent, /Not paired ·/);
  await click(byText('Undo'));
  assert.ok(calls.some((c) => c[0] === 'undoUnpair'));
  assert.match($('[data-catalog="telegram"]').textContent, /Sam @sam_example/);
});

test('a failed action shows the server sentence as an alert', async () => {
  const { api } = mockApi({ test: async () => { throw new m.TelegramError('Telegram is busy. Try again in a minute.', 429, 'throttled'); } });
  await mount(h(m.TelegramConnection, { api, initial: paired() }));
  await click(byText('Send a test message'));
  const alert = $('[data-catalog="telegram"] [role="alert"]');
  assert.equal(alert.textContent.trim(), 'Telegram is busy. Try again in a minute.');
});

// ------------------------------------------------------------------ chat marks

test('chat marks: via Telegram, sent to Telegram too, and a decision made there', async () => {
  assert.equal(m.fromTelegram({ source: 'telegram' }), true);
  assert.equal(m.fromTelegram({ source: 'web' }), false);
  assert.equal(m.sentToTelegram({ sent_to_telegram: true }), true);
  assert.equal(m.sentToTelegram(null), false);
  await mount(h('div', null, h(m.ViaTelegram), h(m.SentToTelegram), h(m.ResolvedViaTelegram, { kind: 'question', lead: 'Drafter asked “Which day?”', outcome: 'You picked Monday', at: '2026-11-07T10:06:00Z' })));
  assert.match($('[data-via-telegram]').textContent, /via Telegram/);
  assert.match($('[data-sent-telegram]').textContent, /sent to Telegram too/);
  const row = $('[data-resolved-via="telegram"]');
  assert.match(row.textContent, /Drafter asked “Which day\?” · You picked Monday/);
  assert.ok(row.querySelector('[data-via-telegram]'));
});
