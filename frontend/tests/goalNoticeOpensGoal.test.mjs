// 34d goal-notice-opens-the-goal (Mira's UX pass at 0.9.40, Minor e): a tap on a goal notice opened the bot's empty
// chat. The notice now names its goal (push `goal_id`, `&goal=<id>` in its address) and the app opens that space's
// Mission board with the goal's drawer: on first load, from the open app (the worker's message), from another space,
// and from the page's own notification. Without a goal id, or with the board off, it opens the bot's chat as before.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const b = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { GoalsStrip } from './src/components/GoalsStrip';
  export * from './src/goals';
  export { targetFromQuery, goalFromQuery, goalOfNotification, notificationPath, notificationTag, showDeviceNotification } from './src/push';
`);

const root = new URL('..', import.meta.url).pathname;
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const settle = async (ms = 0) => {
  if (ms) await new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const plain = (v) => JSON.parse(JSON.stringify(v));
/** openGoal dispatches on the (jsdom) window; Node's own CustomEvent would be refused there. */
const onWindow = (fn) => {
  const nodeCustomEvent = globalThis.CustomEvent;
  globalThis.CustomEvent = window.CustomEvent;
  try {
    return fn();
  } finally {
    globalThis.CustomEvent = nodeCustomEvent;
  }
};

const NOW = Date.parse('2026-11-22T12:00:00Z');
const minsAgo = (m) => new Date(NOW - m * 60_000).toISOString();
const check = { id: 'c1', label: 'Site answers', kind: 'http', spec: { url: 'https://kwant.si' }, required: true, latest: { status: 'failed', verdict: 'not_met', excerpt: 'HTTP 502', observed_at: minsAgo(10) } };
const broke = b.parseGoal({
  id: 'g1', space_id: 's1', title: 'kwant.si stays up', kwant_id: 'k1', status: 'not_met', mode: 'suggest', interval_minutes: 60,
  created_by: 'person', created_at: minsAgo(600), updated_at: minsAgo(10), checks: [check], open_mission: null,
});

// The server's push for a goal notice (src/server/push.rs a_goal_notice_names_its_goal)
const GOAL_PUSH = {
  title: 'kwant.si stays up is failing',
  body: 'Since 18:02 UTC, this isn’t true: Site answers.',
  kind: 'goal_alert',
  space_id: 's1',
  channel_id: null,
  kwant_id: 'k1',
  goal_id: 'g1',
  url: '/?space=s1&kwant=k1&goal=g1',
  tag: 'kwnt-s1-goal_alert-g1',
};

// ---------------------------------------------------------------- the address

test('the push url keeps its goal, and still names the bot', () => {
  assert.deepEqual(b.targetFromQuery('?space=s1&kwant=k1&goal=g1'), { spaceId: 's1', channelId: null, botId: 'k1', goalId: 'g1' });
  assert.deepEqual(b.targetFromQuery('?space=s1&kwant=k1&goal=g%201'), { spaceId: 's1', channelId: null, botId: 'k1', goalId: 'g 1' });
  // As on main without one
  assert.deepEqual(b.targetFromQuery('?space=s1&kwant=k1'), { spaceId: 's1', channelId: null, botId: 'k1' });
  // The worker's own address (/s/<space>/b/<bot>?goal=<id>) and the push url both say which goal
  assert.equal(b.goalFromQuery('?goal=g1'), 'g1');
  assert.equal(b.goalFromQuery('?space=s1&kwant=k1&goal=g1'), 'g1');
  assert.equal(b.goalFromQuery('?goal='), null);
  assert.equal(b.goalFromQuery(''), null);
});

test('a goal notice opens at the goal’s own address, with one tag per goal like the server’s', () => {
  const n = { space_id: 's1', channel_id: null, kwant_id: 'k1', goal_id: 'g 1', kind: 'goal_alert' };
  assert.equal(b.notificationPath(n), '/s/s1/b/k1?goal=g%201', 'the bot stays in the address, so an older page opens its chat');
  assert.equal(b.notificationTag(n), 'kwnt-s1-goal_alert-g 1');
  assert.equal(b.notificationTag({ ...n, goal_id: 'g2' }), 'kwnt-s1-goal_alert-g2');
  // Other notifications as before
  assert.equal(b.notificationPath({ space_id: 's1', channel_id: null, kwant_id: 'k1' }), '/s/s1/b/k1');
  assert.equal(b.notificationTag({ space_id: 's1', kind: 'goal_alert', channel_id: null, kwant_id: 'k1' }), 'kwnt-s1-goal_alert-k1');
  assert.equal(b.notificationPath({ space_id: null, kwant_id: 'k1', goal_id: 'g1' }), '/');
  assert.equal(b.goalOfNotification({ goal_id: 'g1' }), 'g1');
  assert.equal(b.goalOfNotification({ goal_id: '' }), null);
  assert.equal(b.goalOfNotification({}), null);
  assert.equal(b.goalOfNotification(null), null);
});

// ---------------------------------------------------------------- opening it

test('a goal notice opens the Mission board and the goal; without a goal, or with the board off, the chat as before', () => {
  const shown = [];
  const show = (p) => shown.push(p);
  assert.equal(onWindow(() => b.openGoalNotice({ kind: 'goal_alert', goal_id: 'g1' }, true, show)), true);
  assert.deepEqual(shown, ['board']);
  assert.equal(b.takePendingGoal(), 'g1', 'the strip opens it when the board mounts');
  // No goal id (an older server): not handled here, so the bot's chat opens as today
  assert.equal(onWindow(() => b.openGoalNotice({ kind: 'goal_alert' }, true, show)), false);
  // The Mission board is off: nowhere to show the goal, so the chat as today
  assert.equal(onWindow(() => b.openGoalNotice({ kind: 'goal_alert', goal_id: 'g1' }, false, show)), false);
  assert.deepEqual(shown, ['board']);
  assert.equal(b.takePendingGoal(), null, 'nothing left pending');
});

async function mountStrip(api) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const r = b.createRoot(host);
  r.render(b.React.createElement(b.GoalsStrip, { names: { k1: 'Cody' }, now: NOW, api }));
  await settle();
  return { done: () => (r.unmount(), host.remove()) };
}

test('first load on a goal notice’s address: the board opens with that goal’s drawer, once', async () => {
  // What the shell does with ?goal= (below): the goal is pending until the board's strip mounts
  onWindow(() => b.openGoalOnBoard(b.goalFromQuery('?space=s1&kwant=k1&goal=g1')));
  const api = { list: async () => [broke], get: async () => b.parseGoalDetail({ goal: broke, checks: [check], events: [], fix_tasks: [], missions: [] }) };
  const m = await mountStrip(api);
  await settle();
  const drawer = document.querySelector('[role="dialog"]');
  assert.ok(drawer, 'the goal’s drawer is open');
  assert.match(drawer.textContent, /kwant\.si stays up/);
  assert.equal(b.takePendingGoal(), null, 'used once');
  m.done();
});

test('a goal that is gone: the board says so in plain words, no error', async () => {
  onWindow(() => b.openGoalOnBoard('g_gone'));
  const api = { list: async () => [broke], get: async () => { throw new b.GoalError('not found', 404); } };
  const m = await mountStrip(api);
  await settle();
  assert.equal(document.querySelector('[role="dialog"]'), null, 'no drawer for a goal that isn’t there');
  assert.match(document.body.textContent, /That goal was deleted\./);
  m.done();
});

test('the shell: ?goal= opens the space’s board with the goal pending, and leaves the address', () => {
  const shell = src('Shell.tsx');
  const first = shell.match(/\/\/ First load:[\s\S]*?\}, \[reload, open\]\);/)[0];
  // Only in the space the address names, and with the bot's chat as the target (the board off falls back to it)
  assert.match(first, /const goalId = here \? goalFromQuery\(window\.location\.search\) : null;/);
  assert.match(first, /open\(s\.id, fromUrl, true, goalId \? \{ kind: 'board' \} :/);
  assert.match(first, /if \(goalId\) openGoalOnBoard\(goalId\);/);
  assert.match(first, /if \(\(cardId \|\| goalId\) && window\.location\.search\) window\.history\.replaceState\(null, '', window\.location\.pathname\);/);
  // From another space (the worker's message, or the page's own notification): that space's board and the goal
  assert.match(shell, /const goToNotified = useCallback\(/);
  assert.match(shell, /goalOfNotification\(n\)[\s\S]*?open\(sid, spaceTargetOfNotification\(n\), false, \{ kind: 'board' \}\);[\s\S]*?openGoalOnBoard\(goal\);/);
  assert.match(shell, /const go = \(\) => goToNotified\(sid, n\);/);
  assert.match(shell, /goToNotified\(m\.space_id, m\);/);
});

test('the open app: the worker’s message with goal_id opens the board and the goal; without it, the chat', () => {
  const app = src('App.tsx');
  const handler = app.match(/const onMessage = \(e: MessageEvent\) => \{\s*const m = e\.data;\s*if \(!isNotificationOpen\(m\)\) return;[\s\S]*?\n    \};/)[0];
  assert.match(handler, /\.\.\.\(m\.goal_id \? \{ goal_id: m\.goal_id \} : \{\}\)/);
  const opened = app.match(/const openNotified = \([\s\S]*?\n  \};/)[0];
  // After the no-bot places, before the bot's chat; the board only when it is on
  assert.match(opened, /else if \(openGoalNotice\(n, missionBoard, showPage\)\) \{\s*\/\/[^\n]*\n\s*\}/);
  assert.ok(opened.indexOf('openGoalNotice') > opened.indexOf("place === 'home'"));
  assert.ok(opened.indexOf('openGoalNotice') < opened.indexOf('openPlaceRef.current(n)'));
});

// ---------------------------------------------------------------- the page's own notification

test('the page’s own notification carries the goal to the tap', async () => {
  const shown = [];
  const reg = { showNotification: async (title, opts) => shown.push({ title, opts }) };
  const nav = { serviceWorker: { getRegistration: async () => reg } };
  await b.showDeviceNotification({ kind: 'goal_alert', kwant_id: 'k1', kwant_name: 'Cody', channel_id: null, goal_id: 'g1', title: 't', body: 'b', space_id: 's1' }, () => undefined, nav);
  assert.equal(shown[0].opts.tag, 'kwnt-s1-goal_alert-g1');
  assert.deepEqual(shown[0].opts.data, { path: '/s/s1/b/k1?goal=g1', kind: 'goal_alert', space_id: 's1', channel_id: null, kwant_id: 'k1', goal_id: 'g1' });
  // Without one: no goal_id key, as before
  await b.showDeviceNotification({ kind: 'goal_alert', kwant_id: 'k1', kwant_name: 'Cody', channel_id: null, title: 't', body: 'b', space_id: 's1' }, () => undefined, nav);
  assert.deepEqual(shown[1].opts.data, { path: '/s/s1/b/k1', kind: 'goal_alert', space_id: 's1', channel_id: null, kwant_id: 'k1' });
});

// ---------------------------------------------------------------- sw.js

function loadWorker({ clients = [] } = {}) {
  const code = readFileSync(`${root}/sw.js`, 'utf8').replace('__KWNT_SW_VERSION__', 'v').replace('__KWNT_SW_SHELL__', '[]');
  const handlers = {};
  const log = { shown: [], opened: [], posted: [] };
  const list = clients.map((c) => ({
    url: new URL(c.url, 'https://kwant.example').href,
    focused: false,
    visibilityState: 'hidden',
    postMessage: (m) => log.posted.push(m),
    focus: async function () { return this; },
  }));
  const self = {
    location: { origin: 'https://kwant.example' },
    addEventListener: (type, fn) => { handlers[type] = fn; },
    registration: { showNotification: async (title, opts) => { log.shown.push({ title, ...opts }); } },
    clients: { matchAll: async () => list, openWindow: async (url) => { log.opened.push(url); }, claim: async () => undefined },
  };
  vm.runInNewContext(code, { self, URL, caches: {}, fetch: async () => ({ ok: true }) });
  const run = async (type, extra) => {
    let waited;
    handlers[type]({ ...extra, waitUntil: (pr) => { waited = pr; } });
    await waited;
  };
  return {
    log,
    push: (payload) => run('push', { data: { json: () => payload } }),
    click: (data) => run('notificationclick', { notification: { data, close() {} } }),
  };
}

test('sw.js: a goal notice keeps its goal, opens at the goal’s address when closed, and tells the open app', async () => {
  const sw = loadWorker();
  await sw.push(GOAL_PUSH);
  const data = plain(sw.log.shown[0].data);
  assert.equal(sw.log.shown[0].tag, 'kwnt-s1-goal_alert-g1');
  assert.deepEqual(data, { path: '/s/s1/b/k1?goal=g1', kind: 'goal_alert', space_id: 's1', channel_id: null, kwant_id: 'k1', goal_id: 'g1' });
  // kwant closed: it opens on that address (the shell opens the board and the goal)
  await sw.click(data);
  assert.deepEqual(sw.log.opened, ['/s/s1/b/k1?goal=g1']);
  // kwant open: the app is told the goal
  const open = loadWorker({ clients: [{ url: '/s/s1/c/zz' }] });
  await open.click(data);
  assert.deepEqual(plain(open.log.posted), [{ type: 'kwnt-notification-open', kind: 'goal_alert', space_id: 's1', channel_id: null, kwant_id: 'k1', goal_id: 'g1' }]);
});

test('sw.js: without a goal id, a goal notice is exactly as before', async () => {
  const sw = loadWorker({ clients: [{ url: '/s/s1/c/zz' }] });
  const { goal_id, ...old } = GOAL_PUSH;
  await sw.push({ ...old, url: '/?space=s1&kwant=k1', tag: 'kwnt-s1-goal_alert-k1' });
  const data = plain(sw.log.shown[0].data);
  assert.deepEqual(data, { path: '/s/s1/b/k1', kind: 'goal_alert', space_id: 's1', channel_id: null, kwant_id: 'k1' });
  await sw.click(data);
  assert.deepEqual(plain(sw.log.posted), [{ type: 'kwnt-notification-open', kind: 'goal_alert', space_id: 's1', channel_id: null, kwant_id: 'k1' }]);
});
