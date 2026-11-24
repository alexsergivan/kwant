// pa13 front desk guest chat (autonomy/pocket-apps/pa13/guest-brief.md): the view mapping from Lin's desk API
// (pa13/impl.md), the copy for notes and refusals, the outbox (client_id swap, network retry, 429 keeps the text,
// 410 gives G12) and the chat flows against the fake desk box (guest/fake/desk.mjs). The browser half (G1-G12 in
// Chrome under pa1's CSP, shots) is guest/e2e/csp.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { DeskBox, deskScenarios, deskEnvelope } from '../guest/fake/desk.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', { url: 'http://localhost/p/desk-chat', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'SVGElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { deskFrom, screenOf, MAX_CHARS } from './guest/src/desk/view';
  export { names, noteCopy, slowCopy, longCopy, joinCopy, sayCopy } from './guest/src/desk/copy';
  export { Outbox, clientId } from './guest/src/desk/outbox';
  export { DeskApp, dayLabel } from './guest/src/desk/app';
`);
const settle = () => new Promise((r) => setTimeout(r, 0));
const flush = async () => {
  for (let i = 0; i < 8; i++) await settle();
};

const me = { id: 'g1', name: 'Ana', avatar: 'visor' };
const face = { kit: 'desk', title: 'Sea-view flat', place: 'Piran', host_name: 'Marta', bot: { name: 'Bell', sprite: 'desk' }, keep_days: 7 };
const view = (over = {}) => ({ ...face, me: { id: 'g1', name: 'Ana', creature: 'visor', delete_on: '2026-12-01' }, starters: [], messages: [], waiting_for_host: false, bot_typing: false, limits: { per_hour: 20, left_this_hour: 20, next_at: null, max_chars: 1000 }, ...over });
const env1 = (v, o = {}) => deskEnvelope({ code: 't', version: 4, me: o.me === null ? undefined : me, view: v, ended: o.ended });

// ---------- view mapping ----------

test('view: own thread with each kind of sender; junk and unknown notes dropped', () => {
  const r = m.deskFrom(
    env1(
      view({
        messages: [
          { id: 'm1', from: 'guest', text: 'Wi-Fi?', at: '2026-11-24T08:00:00Z', client_id: 'c1' },
          { id: 'm2', from: 'bot', text: 'piran2026', at: '2026-11-24T08:01:00Z', client_id: 'cX' },
          { id: 'm3', from: 'host', text: 'Hi', at: '2026-11-24T08:02:00Z' },
          { id: 'm4', from: 'note', text: '', note: 'handoff', at: '2026-11-24T08:03:00Z' },
          { id: 'm5', from: 'note', text: '<b>x</b>', note: 'party', at: '2026-11-24T08:03:00Z' },
          { id: 'm6', from: 'other_guest', text: 'Hi', at: '2026-11-24T08:03:00Z' },
          { id: 'm7', from: 'bot', text: '   ', at: '2026-11-24T08:03:00Z' },
          { from: 'bot', text: 'no id', at: '2026-11-24T08:03:00Z' },
          null,
          'x',
        ],
        starters: ['a', '', 7, ' b ', 'c', 'd'],
        waiting_for_host: true,
        bot_typing: true,
      }),
    ),
  );
  assert.deepEqual(r.view.messages.map((x) => x.id), ['m1', 'm2', 'm3', 'm4']);
  assert.equal(r.view.messages[0].client_id, 'c1');
  assert.equal(r.view.messages[1].client_id, undefined, 'only a guest message keeps its client_id');
  assert.equal(r.view.messages[3].note, 'handoff');
  assert.deepEqual(r.view.starters, ['a', 'b', 'c']);
  assert.equal(r.view.waiting, true);
  assert.equal(r.view.typing, true);
  assert.equal(r.view.me.delete_on, '2026-12-01');
  assert.equal(r.view.limits.max_chars, 1000);
  assert.equal(r.version, 4);
});

test('view: the screen for each case', () => {
  const at = (env, o = { rejoin: false }) => m.screenOf(m.deskFrom(env).view, o);
  assert.equal(at(env1(face, { me: null })), 'join');
  assert.equal(at(env1(face, { me: null }), { rejoin: true }), 'rejoin');
  assert.equal(at(env1(view())), 'chat');
  assert.equal(at(env1(view(), { ended: true })), 'ended');
  assert.equal(at(env1(face, { me: null, ended: true })), 'ended');
  // A kit `me` alone is no seat: pa1's cookie player is.
  assert.equal(at(env1(view(), { me: null })), 'join');
  assert.equal(m.deskFrom(null), undefined);
  assert.equal(m.deskFrom({ view: {} }), undefined);
  const bad = m.deskFrom(env1(view({ limits: { max_chars: -1 }, host_name: '  ', bot: { name: 7 } })));
  assert.equal(bad.view.limits.max_chars, m.MAX_CHARS);
  assert.equal(bad.view.host_name, '');
  assert.equal(bad.view.bot, '');
  assert.equal(m.deskFrom(env1(view({ me: { id: 'g1', name: 'Ana', delete_on: 'soon' } }))).view.me.delete_on, undefined);
});

// ---------- copy ----------

test('copy: names fall back to plain words; notes, slow, long and refusals', () => {
  const n = m.names({ host_name: 'Marta', bot: 'Bell' });
  assert.deepEqual([n.host, n.Host, n.hosts, n.bot, n.anon], ['Marta', 'Marta', "Marta's", 'Bell', false]);
  const a = m.names({ host_name: '', bot: '' });
  assert.deepEqual([a.host, a.Host, a.hosts, a.bot, a.anon], ['the host', 'The host', "the host's", 'The bot', true]);
  assert.deepEqual(m.noteCopy('handoff', n).line, ['Marta will reply here.', "You can close this page; your chat stays, and it's here when you come back."]);
  assert.equal(m.noteCopy('daily_cap', n).icon, 'moon');
  assert.match(m.noteCopy('host_away', a).line.join(' '), /^The bot can't answer right now\. The host has your message/);
  assert.equal(m.slowCopy('nonsense').hint, 'You can send it soon');
  assert.deepEqual(m.longCopy(1000), ["That's a long one.", 'Messages can be up to 1,000 characters. Try two.']);
  assert.deepEqual(m.joinCopy({ ok: false, status: 409, error: 'guest_max', max_guests: 12 }, n, true), { title: 'This guide is full right now.', sub: 'Ask Marta to make room.' });
  assert.ok(m.sayCopy({ ok: false, status: 400, error: 'empty' }));
});

test('no hard-coded sample names, emoji or innerHTML in the desk source', () => {
  for (const f of ['app', 'view', 'copy', 'outbox', 'look', 'desk']) {
    // Code only: comments may name the mockup's sample host and bot.
    const src = readFileSync(new URL(`../guest/src/desk/${f}.ts`, import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '').replace(/\s\/\/ .*$/gm, '');
    assert.doesNotMatch(src, /Marta|\bBell\b|innerHTML|outerHTML|insertAdjacentHTML/, f);
    assert.doesNotMatch(src, /\p{Extended_Pictographic}/u, `${f}: emoji`);
  }
  const css = readFileSync(new URL('../guest/src/desk/desk.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /#[0-9a-f]{3,8}\b/i, 'desk.css uses tokens, no hex');
});

// ---------- outbox ----------

test('outbox: ids are unique, kept per guest, reloaded as failed, settled by client_id', () => {
  assert.notEqual(m.clientId(1, () => 0.1), m.clientId(1, () => 0.2));
  const s = new Map();
  const store = { getItem: (k) => s.get(k) ?? null, setItem: (k, v) => s.set(k, v) };
  const o = new m.Outbox(store);
  o.open('t', 'g1');
  o.add('one', 'c1', '2026-11-24T08:00:00Z');
  o.add('two', 'c2', '2026-11-24T08:00:01Z');
  const o2 = new m.Outbox(store);
  o2.open('t', 'g1');
  assert.deepEqual(o2.failed().map((x) => x.client_id), ['c1', 'c2']);
  o2.settle([{ id: 'm1', from: 'guest', text: 'one', at: 'x', client_id: 'c1' }]);
  assert.deepEqual(o2.items.map((x) => x.client_id), ['c2']);
  const other = new m.Outbox(store);
  other.open('t', 'g2');
  assert.equal(other.items.length, 0, 'another guest on this phone sees none of it');
});

// ---------- flows against the fake box ----------

function harness(name, { store = new Map() } = {}) {
  const calls = [];
  let now = Date.now();
  let timers = [];
  let nextId = 1;
  const box = new DeskBox(name, { code: 't' });
  box.up = true;
  const fetch = async (url, init = {}) => {
    const path = url.split('/api/pocket/t/')[1] ?? '';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ path, body });
    if (!box.up) throw new TypeError('Failed to fetch');
    const r = box.handle(path, body ?? {});
    return { ok: r.status < 400, status: r.status, json: async () => (r.body === null ? Promise.reject(new SyntaxError('empty')) : r.body) };
  };
  const env = {
    fetch,
    WebSocket: undefined,
    setTimeout: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f });
      return nextId++;
    },
    clearTimeout: (id) => (timers = timers.filter((t) => t.id !== id)),
    setInterval: (f, ms) => {
      timers.push({ id: nextId, at: now + ms, f, every: ms });
      return nextId++;
    },
    clearInterval: (id) => (timers = timers.filter((t) => t.id !== id)),
    now: () => now,
    location: { pathname: '/p/t', protocol: 'http:', host: 'localhost' },
    store: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) },
  };
  const root = document.getElementById('app');
  root.replaceChildren();
  const app = new m.DeskApp(root, env, 't');
  const advance = async (ms) => {
    const end = now + ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const t = timers[0];
      if (!t || t.at > end) break;
      now = t.at;
      if (t.every) t.at += t.every;
      else timers.shift();
      t.f();
      await flush();
    }
    now = end;
    await flush();
  };
  const $ = (s) => root.querySelector(s);
  return { app, root, box, calls, store, advance, $, screen: () => $('[data-screen]')?.getAttribute('data-screen'), text: () => root.textContent.replace(/\s+/g, ' '), says: () => calls.filter((c) => c.path === 'action') };
}
async function open(name, opts) {
  const t = harness(name, opts);
  await t.app.start(null);
  await flush();
  return t;
}
const type = async (el, v) => {
  assert.ok(el, 'element to type in');
  el.value = v;
  el.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await flush();
};
const submit = async (form) => {
  assert.ok(form, 'form');
  form.dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await flush();
};
const ownBubbles = (t) => t.root.querySelectorAll('.gm.me').length;

test('G2 to G3: a starter sends with a client_id; the kept message replaces the outbox bubble', async () => {
  const t = await open('desk-first');
  assert.equal(t.screen(), 'chat');
  assert.match(t.text(), /Hi Ana! I'm Bell, Marta's bot\./);
  const chip = t.$('[data-starter]');
  assert.ok(chip);
  chip.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await flush();
  const [say] = t.says();
  assert.equal(say.body.name, 'say');
  assert.equal(say.body.body.text, 'What is the Wi-Fi password?');
  assert.match(say.body.body.client_id, /^c[0-9a-z]+$/);
  assert.equal(ownBubbles(t), 1);
  assert.ok(t.$('[data-typing]'), 'G3 typing');
  assert.equal(t.$('[data-starter]'), null, 'starters go after the first message');
  assert.equal(t.$('.gm.me.sending'), null);
  t.app.stop();
});

test('G4/G5/G7/G10: notes and the waiting pill, never the reason or other guests', async () => {
  for (const [code, note, words] of [
    ['desk-handoff', 'handoff', /Marta will reply here\./],
    ['desk-cap', 'daily_cap', /Bell has answered all it can for today\./],
    ['desk-away', 'host_away', /Bell can't answer right now\./],
  ]) {
    const t = await open(code);
    assert.ok(t.$(`[data-note="${note}"]`), code);
    assert.match(t.text(), words);
    assert.match(t.$('[data-waiting]').textContent, /Waiting for Marta/);
    assert.doesNotMatch(t.text(), /handoff_reason|answer_bot|guests/);
    t.app.stop();
  }
  const h = await open('desk-host');
  assert.ok(h.$('.hosttag'));
  assert.equal(h.$('[data-waiting]'), null);
  h.app.stop();
});

test('G6: 429 slow_down keeps the text in the composer and turns send off until retry_at', async () => {
  const t = await open('desk-slow');
  await type(t.$('#d-say'), 'Is there a hair dryer?');
  await submit(t.$('[data-composer]'));
  assert.equal(t.$('#d-say').value, 'Is there a hair dryer?');
  assert.ok(t.$('[data-slow]'));
  assert.equal(ownBubbles(t), 1, 'nothing was kept: only the earlier Wi-Fi question');
  assert.ok(t.$('[data-send]').disabled);
  await submit(t.$('[data-composer]'));
  assert.equal(t.says().length, 1, 'no second try while slowed down');
  t.app.stop();
});

test('G8: over max_chars the field is invalid, send is off and nothing is sent', async () => {
  const t = await open('desk-long');
  await type(t.$('#d-say'), 'x'.repeat(1001));
  assert.equal(t.$('#d-say').getAttribute('aria-invalid'), 'true');
  assert.ok(t.$('[data-send]').disabled);
  await submit(t.$('[data-composer]'));
  assert.equal(t.says().length, 0);
  assert.match(t.text(), /That's a long one\./);
  t.app.stop();
});

test('G9: a network failure keeps the bubble as Not sent; Retry sends the same client_id once', async () => {
  const t = await open('desk-chat');
  t.box.s = { ...t.box.s };
  t.box.up = false;
  await type(t.$('#d-say'), 'Bakery?');
  await submit(t.$('[data-composer]'));
  assert.ok(t.$('.gm.me.failed [data-try-one]'));
  assert.ok(t.$('[data-offline]'));
  const first = t.says()[0].body.body.client_id;
  assert.ok([...t.store.keys()].some((k) => k.startsWith('kp-desk-out:t:g1')), 'saved on this phone');
  t.box.up = true;
  t.$('[data-retry]').dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await flush();
  const tries = t.says();
  assert.equal(tries.at(-1).body.body.client_id, first);
  assert.equal(ownBubbles(t), 2, 'the kept Wi-Fi question plus Bakery, no duplicate');
  assert.equal(t.$('[data-offline]'), null);
  t.app.stop();
});

test('G9: a message saved on this phone goes again on the next visit', async () => {
  const store = new Map();
  const a = await open('desk-chat', { store });
  a.box.up = false;
  await type(a.$('#d-say'), 'Towels?');
  await submit(a.$('[data-composer]'));
  a.app.stop();
  const b = harness('desk-chat', { store });
  await b.app.start(null);
  await flush();
  const said = b.says().map((c) => c.body.body.text);
  assert.deepEqual(said, ['Towels?']);
  b.app.stop();
});

test('G12: 410 on send closes the chat and keeps the delete date words', async () => {
  const t = await open('desk-chat');
  t.box.s = { ...t.box.s, ended: true };
  await type(t.$('#d-say'), 'Hello?');
  await submit(t.$('[data-composer]'));
  assert.equal(t.screen(), 'ended');
  assert.match(t.text(), /Marta closed this chat/);
  assert.match(t.text(), /Thanks for staying at the Sea-view flat!/);
  t.app.stop();
});

test('G12 cold: an ended desk opened with no boot facts still shows the closed card', async () => {
  const t = await open('desk-ended');
  assert.equal(t.screen(), 'ended');
  assert.ok(t.$('[data-ended]'));
});

test('dayLabel: today, then a date', () => {
  const now = Date.parse('2026-11-24T12:00:00');
  assert.equal(m.dayLabel('2026-11-24T08:00:00', now), 'Today');
  assert.equal(m.dayLabel('2026-11-23T08:00:00', now), 'Mon, Nov 23');
  assert.match(m.dayLabel('2026-11-20T08:00:00', now), /Nov/);
});
