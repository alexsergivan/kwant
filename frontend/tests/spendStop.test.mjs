// Today's spending stop (0.7.4): the two daily limits in Settings with today's use beside them, the
// chat notice and the Needs-you item with Continue today, a tap on its notification, and the skips it
// leaves in routine and mission history. Every number shown is the server's.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: 'https://kwant.example/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'localStorage', 'sessionStorage']) {
  Object.defineProperty(globalThis, k, { value: k === 'window' ? dom.window : dom.window[k], configurable: true, writable: true });
}
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  import React from 'react';
  export * from './src/spendStop';
  export { targetFromQuery, opensSpendStop, placeOfNotification, placeOfQuery } from './src/push';
  export { notifyKindsFor, notifyOnToSave, shouldShowOsNotification } from './src/continuity';
  export { eventText } from './src/missions';
  export { SpendStoppedNotice, SpendStopNeedsYou, SpendStopFields, forgetToday } from './src/components/SpendStop';
  export { React };
  export { createRoot } from 'react-dom/client';
`);

const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const flush = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
  for (let i = 0; i < 6; i++) await flush();
};

/** A fetch that answers by url ([status, body]) and records the calls. */
function fake(routes) {
  const calls = [];
  const send = async (url, init = {}) => {
    calls.push({ url, method: init.method ?? 'GET' });
    const r = routes[`${init.method ?? 'GET'} ${url}`];
    if (!r) throw new Error(`unexpected ${url}`);
    const [status, body] = typeof r === 'function' ? r() : r;
    return { ok: status >= 200 && status < 300, status, json: async () => body };
  };
  return { send, calls };
}

const TODAY = { day: '2026-11-07', timezone: 'UTC', today_tokens: 17420, today_usd: 0.84, stop_tokens: 10000, stop_usd: 0, continued: false, stopped: true, over: 'tokens', day_ends_at: '2026-11-08T00:00:00+00:00' };
const META = { kind: 'spend_stopped', day: '2026-11-07', over: 'tokens', used_tokens: 17420, stop_tokens: 10000, used_usd: 0.84, stop_usd: 0, day_ends_at: '2026-11-08T00:00:00+00:00' };

async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  return { host, unmount: () => (root.unmount(), host.remove()) };
}

// ---------------------------------------------------------------- the settings fields

test('the limits read the server’s settings; empty means off', () => {
  assert.equal(m.stopFieldsFromSettings({ notify_on: [] }), null, 'a server without the stop shows no fields');
  assert.deepEqual(m.stopFieldsFromSettings({ daily_spend_stop_tokens: 0, daily_spend_stop_usd: 0 }), { tokens: '', usd: '' });
  assert.deepEqual(m.stopFieldsFromSettings({ daily_spend_stop_tokens: 200000, daily_spend_stop_usd: 2.5 }), { tokens: '200000', usd: '2.5' });
});

test('tokens accept 500k, 2M and commas; dollars accept $ and cents; errors are plain', () => {
  assert.equal(m.stopTokensFrom(''), 0);
  assert.equal(m.stopTokensFrom('500k'), 500000);
  assert.equal(m.stopTokensFrom('1.5M'), 1500000);
  assert.equal(m.stopTokensFrom('200,000 tokens'), 200000);
  assert.equal(m.stopTokensFrom('lots'), null);
  assert.equal(m.stopTokensFrom('20000000000'), null);
  assert.match(m.stopTokensError('lots'), /like 500k or 2M/);
  assert.match(m.stopTokensError('20000000000'), /10 billion/);
  assert.equal(m.stopTokensError('2M'), null);

  assert.equal(m.stopUsdFrom(''), 0);
  assert.equal(m.stopUsdFrom('$5'), 5);
  assert.equal(m.stopUsdFrom('2.50'), 2.5);
  assert.equal(m.stopUsdFrom('2.505'), null);
  assert.equal(m.stopUsdFrom('-1'), null);
  assert.match(m.stopUsdError('abc'), /like 5 or 2\.50/);
  assert.match(m.stopUsdError('200000'), /\$100,000/);

  assert.deepEqual(m.stopBody({ tokens: '500k', usd: '' }), { daily_spend_stop_tokens: 500000, daily_spend_stop_usd: 0 });
  assert.equal(m.stopBody({ tokens: 'x', usd: '1' }), null);
});

test('today’s line is the server’s figures, unrounded beyond cents', () => {
  const u = m.parseUsageToday(TODAY);
  assert.equal(m.todayLine(u), 'Today: 17,420 tokens · $0.84');
  assert.equal(m.todayLine({ today_tokens: 1, today_usd: 0 }), 'Today: 1 token · $0.00');
  assert.match(m.todayState(u), /won’t start new work today unless you continue/);
  assert.match(m.todayState({ ...u, stopped: false, continued: true }), /Continued for today/);
  assert.equal(m.todayState({ ...u, stopped: false, over: null }), null);
  assert.equal(m.parseUsageToday({ error: 'x' }), null);
});

test('Settings shows the two fields beside today’s use, and re-reads after a save', async () => {
  m.forgetToday();
  const f = fake({ 'GET /api/usage/today': [200, TODAY] });
  let value = { tokens: '10000', usd: '' };
  const el = (key) => m.React.createElement(m.SpendStopFields, { value, onChange: (v) => (value = v), open: true, send: f.send, savedKey: key });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el(0));
  await settle();
  assert.ok(host.querySelector('#spend-stop-tokens[data-switch="spend-stop"]'));
  assert.equal(host.querySelector('label[for="spend-stop-tokens"]')?.textContent.trim(), 'Tokens per day');
  assert.ok(host.querySelector('#spend-stop-usd'));
  assert.match(host.querySelector('[data-spend-today]').textContent, /Today: 17,420 tokens · \$0\.84/);
  assert.doesNotMatch(host.textContent, /\bMCP\b|token budget|endpoint/i);
  root.render(el(1));
  await settle();
  assert.equal(f.calls.filter((c) => c.url === '/api/usage/today').length, 2);
  root.unmount();
  host.remove();
});

// ---------------------------------------------------------------- Continue today

test('Continue today posts once; 403 and 400 read plainly', async () => {
  const ok = fake({ 'POST /api/usage/today/continue': [200, { ...TODAY, stopped: false, continued: true }] });
  const u = await m.spendStopApi(ok.send).continueToday();
  assert.equal(u.continued, true);
  assert.deepEqual(ok.calls, [{ url: '/api/usage/today/continue', method: 'POST' }]);

  const no = fake({ 'POST /api/usage/today/continue': [403, { error: 'person only' }] });
  await assert.rejects(m.spendStopApi(no.send).continueToday(), /Only you can continue today/);
  const bad = fake({ 'POST /api/usage/today/continue': [400, { error: 'No spending stop is on today.' }] });
  await assert.rejects(m.spendStopApi(bad.send).continueToday(), /No spending stop is on today\./);
  await assert.rejects(m.spendStopApi(async () => { throw new Error('down'); }).continueToday(), /Couldn’t reach the server/);
});

test('the notice meta and the Needs-you item parse; the detail is the server’s numbers', () => {
  assert.equal(m.parseSpendStopped(JSON.stringify({ kind: 'budget_paused' })), null);
  assert.equal(m.parseSpendStopped('not json'), null);
  const meta = m.parseSpendStopped(JSON.stringify(META));
  assert.equal(m.stoppedDetail(meta), '17.4k of 10k fresh tokens today');
  assert.equal(m.stoppedDetail({ ...meta, over: 'usd', stop_usd: 0.5 }), '$0.84 of $0.50 today');
  const it = m.spendStoppedItem({ items: [{ kind: 'approval' }, { ...META, id: 'spend_stopped:2026-11-07', text: 'Today’s spending limit is reached.' }] });
  assert.equal(it.id, 'spend_stopped:2026-11-07');
  assert.equal(it.text, 'Today’s spending limit is reached.');
  assert.equal(m.spendStoppedItem({ items: [] }), null);
});

test('the chat notice: Continue today, then Continued for today', async () => {
  m.forgetToday();
  const f = fake({
    'GET /api/usage/today': [200, TODAY],
    'POST /api/usage/today/continue': [200, { ...TODAY, stopped: false, continued: true }],
  });
  const { host, unmount } = await mount(m.React.createElement(m.SpendStoppedNotice, { messageId: 'm1', text: 'Today’s spending limit is reached.', meta: META, send: f.send, onOpenSettings: () => {} }));
  const btn = host.querySelector('[data-spend-continue]');
  assert.equal(btn.textContent.trim(), 'Continue today');
  assert.ok(host.querySelector('[data-spend-settings]'));
  btn.click();
  btn.click();
  await settle();
  assert.equal(host.querySelector('[data-spend-continue]').textContent.trim(), 'Continued for today');
  assert.equal(host.querySelector('[data-spend-continue]').disabled, true);
  assert.equal(f.calls.filter((c) => c.method === 'POST').length, 1, 'a double click posts once');
  unmount();
});

test('an earlier day’s notice, or one already continued, reads so on load', async () => {
  m.forgetToday();
  const cont = fake({ 'GET /api/usage/today': [200, { ...TODAY, stopped: false, continued: true }] });
  let r = await mount(m.React.createElement(m.SpendStoppedNotice, { messageId: 'm2', text: 'x', meta: META, send: cont.send }));
  assert.equal(r.host.querySelector('[data-spend-continue]').textContent.trim(), 'Continued for today');
  r.unmount();
  m.forgetToday();
  const later = fake({ 'GET /api/usage/today': [200, { ...TODAY, day: '2026-11-08', stopped: false, over: null }] });
  r = await mount(m.React.createElement(m.SpendStoppedNotice, { messageId: 'm3', text: 'x', meta: META, send: later.send }));
  assert.equal(r.host.querySelector('[data-spend-continue]'), null);
  r.unmount();
});

test('Needs you shows the stop with Continue today, focused after a notification tap', async () => {
  m.forgetToday();
  const f = fake({
    'GET /api/waiting': [200, { items: [{ ...META, id: 'spend_stopped:2026-11-07', text: 'Today’s spending limit is reached.' }] }],
    'POST /api/usage/today/continue': [200, { ...TODAY, stopped: false, continued: true }],
  });
  m.requestSpendStopFocus();
  const { host, unmount } = await mount(m.React.createElement(m.SpendStopNeedsYou, { send: f.send, onOpenSettings: () => {} }));
  const box = host.querySelector('[data-needs-spend-stop]');
  assert.ok(box);
  assert.match(box.textContent, /Needs you/);
  const btn = box.querySelector('[data-spend-continue]');
  assert.equal(document.activeElement, btn, 'Continue today has focus');
  assert.equal(m.takeSpendStopFocus(), false, 'the request is used once');
  btn.click();
  await settle();
  assert.equal(box.querySelector('[data-spend-continue]').textContent.trim(), 'Continued for today');
  unmount();

  const none = fake({ 'GET /api/waiting': [200, { items: [] }] });
  const empty = await mount(m.React.createElement(m.SpendStopNeedsYou, { send: none.send }));
  assert.equal(empty.host.querySelector('[data-needs-spend-stop]'), null);
  empty.unmount();
});

// ---------------------------------------------------------------- a tap on the notification

test('cold open: ?notify=spend_stopped opens the stop; any other notify= opens home', () => {
  assert.deepEqual(m.targetFromQuery('?notify=spend_stopped'), { spaceId: null, channelId: null, botId: null, notify: 'spend_stopped' });
  assert.equal(m.placeOfQuery('spend_stopped'), 'spend_stop');
  for (const kind of ['app_update', 'loose_end', 'mission_finished', 'something_new']) {
    assert.deepEqual(m.targetFromQuery(`?notify=${kind}`), { spaceId: null, channelId: null, botId: null, notify: kind });
    assert.equal(m.placeOfQuery(kind), 'home');
  }
  // The shell strips the query either way, so a reload doesn't open it again.
  const shell = src('Shell.tsx');
  assert.match(shell, /if \(q\?\.notify\) \{[\s\S]*?placeOfQuery\(q\.notify\) === 'spend_stop'[\s\S]*?requestSpendStopFocus\(\)[\s\S]*?replaceState\(null, '', window\.location\.pathname\)[\s\S]*?return;/);
});

test('?kwant= and ?channel= still open as on main', () => {
  assert.deepEqual(m.targetFromQuery('?space=sp%201&channel=c1'), { spaceId: 'sp 1', channelId: 'c1', botId: null });
  assert.deepEqual(m.targetFromQuery('?space=s&kwant=k1'), { spaceId: 's', channelId: null, botId: 'k1' });
  assert.deepEqual(m.targetFromQuery('?kwant=k1'), { spaceId: null, channelId: null, botId: 'k1' });
  assert.deepEqual(m.targetFromQuery('?channel=c1&kwant=k1'), { spaceId: null, channelId: 'c1', botId: null });
  assert.equal(m.targetFromQuery(''), null);
  assert.equal(m.targetFromQuery('?notify='), null);
  assert.equal(m.placeOfNotification({ kind: 'turn_finished', kwant_id: 'k1', channel_id: null }), null);
  assert.equal(m.placeOfNotification({ kind: 'turn_finished', kwant_id: null, channel_id: 'c1' }), null);
});

test('with the app open: the worker’s message for a stop opens Needs you; other no-bot kinds open home', () => {
  const open = { type: 'kwnt-notification-open', space_id: null, channel_id: null, kwant_id: null };
  assert.equal(m.placeOfNotification({ ...open, kind: 'spend_stopped' }), 'spend_stop');
  assert.equal(m.opensSpendStop({ kind: 'spend_stopped' }), true);
  assert.equal(m.placeOfNotification({ ...open, kind: 'app_update' }), 'home');
  assert.equal(m.placeOfNotification({ ...open, kind: 'app_update', kwant_id: '' }), 'home');
  // The App branches on it before the old kwant_id check, and the page's own notification goes the same way.
  const app = src('App.tsx');
  assert.match(app, /const place = placeOfNotification\(m\);\s*if \(place === 'spend_stop'\) return openSpendStopRef\.current\(\);[\s\S]*?if \(place === 'home'\) return openHomeRef\.current\(\);\s*if \(!m\.kwant_id\) return;/);
  assert.match(app, /const place = placeOfNotification\(n\);\s*if \(place === 'spend_stop'\) openSpendStop\(\);\s*else if \(place === 'home'\) openHome\(\);/);
  assert.match(app, /top=\{<SpendStopNeedsYou refreshKey=\{spendStopSeq\}/);
});

test('the stop’s notification is a kind to tick, only where the server has the stop, and shows on this device', () => {
  const ids = (on) => m.notifyKindsFor(false, false, false, false, on).map((k) => k.id);
  assert.ok(!ids(false).includes('spend_stopped'));
  assert.ok(ids(true).includes('spend_stopped'));
  assert.deepEqual(m.notifyOnToSave(['turn_finished', 'spend_stopped'], false, false, false, false, true), ['turn_finished', 'spend_stopped']);
  assert.deepEqual(m.notifyOnToSave(['turn_finished', 'spend_stopped'], false, false, false, false, false), ['turn_finished']);
  assert.equal(m.shouldShowOsNotification({ enabled: true, permission: 'granted', hidden: true, kind: 'spend_stopped' }), true);
});

// ---------------------------------------------------------------- skips in history

test('a routine run the stop skipped reads plainly, not as an error', () => {
  const err = "spend_stopped: today's spending stop is reached (used 17k of 10k tokens); it lifts at 2026-11-07T00:00:00+00:00 or when the person continues today";
  assert.deepEqual(m.routineRunNote({ status: 'skipped', error: err }), { text: 'Skipped: today’s spending stop was on.', quiet: true });
  assert.deepEqual(m.routineRunNote({ status: 'error', error: 'boom' }), { text: 'boom', quiet: false });
  assert.equal(m.routineRunNote({ status: 'ok', error: null }), null);
});

test('mission task lines for the stop', () => {
  const ev = (kind, detail, to_state = null) => ({ id: 1, task_id: 't', mission_id: null, kind, actor: 'kwant', actor_name: null, from_state: null, to_state, detail, created_at: '2026-11-07T00:00:00Z' });
  assert.equal(m.eventText(ev('skipped', { reason: 'spend_stopped', why: 'x' })), 'Skipped for now: today’s spending stop is on');
  assert.equal(m.eventText(ev('released', { reason: 'skipped: spend_stopped' }, 'ready')), 'Back to Ready: today’s spending stop is on, so it didn’t start');
  assert.match(m.eventText(ev('autofix_held', { reason: 'spend_stopped', autofix_id: 'a1' })), /^AutoFix waits/);
  assert.match(m.eventText(ev('state', { reason: "skipped: today's spending stop holds new turns" }, 'cancelled')), /^Cancelled: today’s spending stop/);
  assert.equal(m.spendStopEventText(ev('skipped', { reason: 'other' })), null);
});
