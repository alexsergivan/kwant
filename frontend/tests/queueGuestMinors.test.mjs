// queue-guest-minors (autonomy/queue-guest-minors/brief.md), from Mira's queue UX pass rows m4 and m6:
// m4 the big screen opened from a spent (or old) #screen= link says the link is one-time and where a new one comes from;
// m6 the phone's hidden turn line is emptied once the turn is over, so a screen reader never lands on a stale "Your turn!".
import './pinLocale.mjs'; // en-US default, whatever the machine's locale
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';
import { QueueBox } from '../guest/fake/queue.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/queue-x', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'SVGElement', 'getComputedStyle', 'matchMedia', 'requestAnimationFrame']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { queueFrom } from './guest/src/queue/view';
  export { QueueApp } from './guest/src/queue/app';
  export { QueueTV, TV_LINK_USED } from './guest/src/queue/tv';
`);

const timers = () => {
  let id = 1;
  return { setTimeout: () => id++, clearTimeout: () => {}, setInterval: () => id++, clearInterval: () => {}, now: () => Date.now() };
};

test('m4: a refused screen link says it was already used, where a new one comes from, and the 10-minute limit', async () => {
  const root = document.getElementById('app');
  const calls = [];
  const fetch = async (url, init) => {
    calls.push(url);
    return { ok: false, status: 401, json: async () => ({ error: 'link_used_or_expired', message: 'This link was already used or is too old. Open a new one from the card.' }) };
  };
  const tv = new m.QueueTV(root, { fetch, WebSocket: undefined, location: { pathname: '/p/queue-x', protocol: 'http:', host: 'localhost' }, ...timers() }, 'queue-x', 'spent-token');
  await tv.start();
  assert.ok(calls.some((u) => u.endsWith('/host/signin')), 'the screen tried its one sign-in');
  assert.equal(root.querySelector('[data-screen]')?.getAttribute('data-screen'), 'signed-out');
  const h1 = root.querySelector('.tvcalm h1');
  assert.equal(h1?.textContent, 'This big-screen link was already used');
  const lines = [...root.querySelectorAll('.tvcalm p.tvsoft')].map((p) => p.textContent);
  assert.deepEqual(lines, [
    'Each link opens one screen. To add another screen, open a new link from the queue card in kwant.',
    "Links also stop working 10 minutes after they're made.",
  ]);
  assert.ok(!/expired|Pocket card/.test(root.textContent), 'the old copy is gone');
  assert.deepEqual([h1.textContent, ...lines], [...m.TV_LINK_USED]);
  tv.stop();
});

test('m6: the hidden turn line says "Your turn!" on the call and is empty once served (or skipped, or left)', () => {
  for (const after of ['served', 'skipped', 'left']) {
    const root = document.getElementById('app');
    root.replaceChildren();
    const app = new m.QueueApp(root, { fetch: async () => ({ ok: false, status: 0, json: async () => ({}) }), WebSocket: undefined, location: { pathname: '/p/q', protocol: 'http:', host: 'localhost' }, buzz: { navigator: {} }, ...timers() }, 'q');
    const view = (scene) => m.queueFrom(new QueueBox(`queue-${scene}--1`).env(true)).view;
    const said = () => root.querySelector('[aria-live]')?.textContent;
    app.show(view('next'));
    assert.equal(said(), '', 'nothing said before the call');
    app.show(view('turn'));
    assert.equal(said(), 'Your turn! Number #27.');
    app.show(view(after));
    assert.equal(said(), '', `cleared after ${after}`);
    app.stop();
  }
});

test('m6: a turn view that repeats (a push, a refresh) keeps the line; the next and turn cues still fire once each', () => {
  const root = document.getElementById('app');
  root.replaceChildren();
  const cues = [];
  const app = new m.QueueApp(root, { fetch: async () => ({ ok: false, status: 0, json: async () => ({}) }), WebSocket: undefined, location: { pathname: '/p/q', protocol: 'http:', host: 'localhost' }, buzz: { navigator: { userAgent: 'Android', vibrate: (p) => cues.push(p) } }, ...timers() }, 'q');
  const view = (scene) => m.queueFrom(new QueueBox(`queue-${scene}--1`).env(true)).view;
  app.show(view('ticket'));
  app.show(view('next'));
  app.show(view('turn'));
  app.show(view('turn'));
  assert.equal(root.querySelector('[aria-live]')?.textContent, 'Your turn! Number #27.');
  assert.equal(cues.length, 2, 'one buzz for next, one for turn');
  app.stop();
});
