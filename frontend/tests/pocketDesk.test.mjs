// Front desk (Guest Guide) Pocket card (pa13, autonomy/pocket-apps/pa13/design.md, card-brief.md, impl.md): validDesk
// reads Lin's host summary (missing fields fall back, junk is rejected, answer_bot's id is never kept), the states 1-9
// each draw, a guest's words stay plain text, the Allow card's answers are optimistic with a 5 s Undo and roll back on
// failure, the Inbox shows one pocket_desk item per waiting guest, and answering in either place closes both.
process.env.TZ = 'Europe/Berlin';
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'HTMLTextAreaElement', 'HTMLInputElement']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { validPocket, kitLabel } from './src/components/results/Pocket';
  export { validDesk, validDeskItems, validConversation, deskStage, deskMood, deskAnswers, deskKey, withAnswers, DeskCard, DeskNeedsRow, DeskDoneRow, deskDoneLine, deskConversationCache, DESK_YES, DESK_SORRY, DESK_FAILED, DESK_LATER, deskNextCap } from './src/components/results/PocketDesk';
  export { DESK_FIXTURES, DESK_CONVERSATIONS, DESK_WAITING, DESK_CAP_WAITING, DESK_OPEN } from './src/components/results/fixtures';
  export { InboxNew } from './src/components/InboxNew';
  export { inboxStore } from './src/inboxStore';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  // Reset first, so unmounting doesn't send a held answer into the next test.
  m.deskAnswers.reset();
  for (const v of mounted) v.done();
  mounted.clear();
  m.deskAnswers.reset();
  m.deskConversationCache.clear();
  g.fetch = realFetch;
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const F = m.DESK_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withD = (name, over) => {
  const b = raw(name);
  b.desk = { ...b.desk, ...over };
  return b;
};
const ok = (b) => assert.ok(m.validPocket(b), 'should validate');
const bad = (b, why) => assert.equal(m.validPocket(b), null, why);
const calls = [];
/** fetch stub: GET /api/pockets/seaview?view=guest:x → conversation; actions → body or status. */
function stub({ action = 'ok', waiting = m.DESK_WAITING, read = null } = {}) {
  calls.length = 0;
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    calls.push([method, String(url), init.body ? JSON.parse(init.body) : null]);
    const u = String(url);
    const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'content-type': 'application/json' } });
    if (u.startsWith('/api/waiting')) return json(waiting);
    const view = /view=guest%3A([^&]+)/.exec(u);
    if (view) return json({ pocket: { state: 'live' }, summary: raw('live').desk, join: null, conversation: m.DESK_CONVERSATIONS[view[1]] });
    if (u.endsWith('/action')) {
      if (action === 'fail') return json({ error: 'no_such_player', message: 'No such guest' }, 404);
      const body = JSON.parse(init.body);
      if (body.name === 'set_cap') {
        const c = structuredClone(raw('cap').desk);
        c.counts = { ...c.counts, daily_cap: body.body.cap };
        return json({ pocket: { state: 'live' }, summary: c, join: null });
      }
      const s = structuredClone(raw('live').desk);
      s.guests = s.guests.map((x) => (x.id === 'g-emma' ? { ...x, waiting: false, handoff_reason: null } : x));
      s.counts = { ...s.counts, waiting: 0 };
      return json({ pocket: { state: 'live' }, summary: s, join: null });
    }
    // The card's one read on show: off unless a test asks, so each test keeps its own fixture.
    if (method === 'GET') return read ? json(read) : json({ error: 'offline' }, 503);
    return json({});
  };
}
const card = (name, block = raw(name), open = m.DESK_OPEN[name]) =>
  mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [block] }, messageId: `m-${name}${open ? `-${open}` : ''}` }));
const deskCard = (name, opts = {}) => {
  const b = m.validPocket(raw(name));
  assert.ok(b);
  return mount(h(m.DeskCard, { block: b, ask: opts.ask ?? (() => {}), openGuest: opts.open ?? m.DESK_OPEN[name] ?? null }));
};
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const act = (v, a) => v.host.querySelector(`[data-action="${a}"]`);

// --------------------------------------------------------------------------------------------- validator

test('every desk fixture validates, routes to the desk card and keeps no bot id', () => {
  for (const name of Object.keys(F)) {
    const b = m.validPocket(raw(name));
    assert.ok(b, name);
    assert.equal(b.kit, 'desk');
    assert.ok(b.desk);
    assert.ok(!JSON.stringify(b.desk).includes('kw-never-shown-01'), `${name}: answer_bot id dropped`);
  }
  assert.equal(m.kitLabel('desk'), 'Guest Guide');
});

test('missing desk fields fall back', () => {
  const b = raw('live');
  delete b.desk.counts.daily_cap;
  delete b.desk.counts.messages_today;
  delete b.desk.cost_per_reply_usd;
  delete b.desk.guide_preview;
  delete b.desk.place;
  delete b.desk.title;
  b.desk.guests = b.desk.guests.map(({ unread, msgs, ...gg }) => gg);
  const v = m.validPocket(b);
  assert.ok(v);
  assert.equal(v.desk.counts.daily_cap, 50);
  assert.equal(v.desk.counts.messages_today, 0);
  assert.equal(v.desk.cost_per_reply_usd, null);
  assert.equal(v.desk.title, 'Sea-view flat', 'title falls back to the pocket name');
  assert.equal(v.desk.guests[0].unread, 0);
  // `bot` is accepted where answer_bot is missing.
  const b2 = raw('live');
  delete b2.desk.answer_bot;
  b2.desk.bot = { name: 'Bell', model: null };
  ok(b2);
  // A note with an unknown kind is junk; a guest's words with tags are fine (they render as text).
  assert.ok(m.validConversation({ guest: { id: 'g1', name: 'Ann', waiting: false }, messages: [{ id: 'a', from: 'guest', text: '<b>hi</b>', at: '2026-11-24T10:00:00Z' }] }));
  assert.equal(m.validConversation({ guest: { id: 'g1', name: 'Ann', waiting: false }, messages: [{ id: 'a', from: 'note', text: '', note: 'nope', at: 'x' }] }), null);
});

test('junk desk blocks are rejected', () => {
  bad({ ...raw('live'), desk: null }, 'kit desk without its desk part');
  bad({ ...raw('live'), kit: 'quiz' }, 'a desk on another kit');
  bad({ ...raw('live'), santa: { phase: 'ended' } }, 'desk with santa');
  bad({ ...raw('live'), join: { wifi_url: 'http://192.168.1.2/p/x' } }, 'a top-level join');
  bad({ ...raw('live'), state: 'ready' }, 'desk in a quiz-only state');
  bad(withD('live', { counts: { guests: 'four' } }), 'counts not numbers');
  bad(withD('live', { counts: { guests: 4, daily_cap: 5000 } }), 'cap out of range');
  bad(withD('live', { answer_bot: { id: 'x' } }), 'no bot name');
  bad(withD('live', { guests: [{ id: 'bad id!', name: 'Ann', waiting: false }] }), 'guest id');
  bad(withD('live', { guests: [{ id: 'a', name: 'Ann', waiting: 'yes' }] }), 'waiting not a bool');
  bad(withD('live', { guests: [{ id: 'a', name: 'Ann', waiting: false }, { id: 'a', name: 'Bo', waiting: false }] }), 'duplicate guests');
  bad(withD('live', { guests: [{ id: 'a', name: 'Ann', waiting: false, last: { from: 'robot', text: 'x', at: 'y' } }] }), 'bad last.from');
  bad(withD('live', { join: { mode: 'anywhere', anywhere_url: 'https://x.trycloudflare.com/p/a' } }), 'quick tunnel join');
  bad(withD('live', { cost_per_reply_usd: -1 }), 'negative cost');
  bad(withD('live', { title: '<script>x</script>' }), 'HTML in the title');
});

test('needs_stable_address only from its start errors', () => {
  assert.equal(m.validPocket(raw('address')).desk.needs_address, true);
  assert.equal(m.validPocket(withD('setup', { start_error: 'something_else' })).desk.needs_address, false);
});

test('Needs-you items: one per waiting guest, other kinds skipped, junk dropped', () => {
  const item = m.DESK_WAITING.items[0];
  const items = m.validDeskItems({
    items: [item, { ...item }, { kind: 'question', id: 'q1' }, { ...item, guest_id: 'bad id!' }, { ...item, guest_id: 'g-ben', guest_name: 'Ben', text: '', quote: null }],
  });
  assert.equal(items.length, 2);
  assert.equal(items[0].id, 'desk:seaview:g-emma');
  assert.equal(items[1].text, 'Bell asked you to reply', 'an empty reason falls back');
  assert.ok(!JSON.stringify(items).includes('kw-never-shown-01'));
  assert.deepEqual(m.validDeskItems(null), []);
});

// --------------------------------------------------------------------------------------------- stages

test('stage and mood for every state', () => {
  const st = (name) => {
    const b = m.validPocket(raw(name));
    return [m.deskStage(b, b.desk), m.deskMood(m.deskStage(b, b.desk), b.desk)];
  };
  assert.deepEqual(st('setup'), ['setup', 'thinking']);
  assert.deepEqual(st('address'), ['address', 'thinking']);
  assert.deepEqual(st('ready'), ['ready', 'idle']);
  assert.deepEqual(st('live'), ['live', 'needs_you']);
  assert.deepEqual(st('cap'), ['cap', 'needs_you']);
  assert.deepEqual(st('ended'), ['stopped', 'finished']);
  const b = m.validPocket(withD('live', { guests: [{ id: 'a', name: 'Ann', waiting: true, handoff_reason: 'Bell couldn’t answer this message' }], counts: { guests: 1, waiting: 1 } }));
  assert.equal(m.deskMood('live', b.desk), 'failed');
});

test('each state draws its chip, and holo never shows', async () => {
  stub();
  const chips = { setup: 'setup', address: 'address', ready: 'ready', live: 'live', cap: 'cap', ended: 'stopped' };
  for (const [name, chip] of Object.entries(chips)) {
    const v = await card(name);
    assert.ok(v.host.querySelector(`[data-chip="${chip}"]`), `${name} chip`);
    assert.equal(v.host.querySelectorAll('.kw-sticker-holo, [data-tone="holo"]').length, 0, `${name}: no holo`);
    // The stopped card says when its last chat is deleted instead.
    assert.match(v.host.textContent, name === 'ended' ? /The last chat is deleted on/ : /Guest chats stay on this Mac/, `${name}: keep line`);
    v.done();
  }
});

test('set-up: guide, Bell, stepper 10-200, Start asks with the cap, Paste is LATER', async () => {
  const asked = [];
  const v = await deskCard('setup', { ask: (t) => asked.push(t) });
  assert.match(v.host.textContent, /Wi-Fi: SeaView-5G/);
  assert.match(v.host.textContent, /Answers only from this guide/);
  const up = v.host.querySelector('[data-step="up"]');
  await click(up);
  await click(up);
  assert.equal(v.host.querySelector('[data-stepper] output').textContent, '70');
  assert.match(v.host.textContent, /After 70, guests read “Marta will reply”/);
  await click(act(v, 'start'));
  assert.equal(asked.at(-1), 'Start Sea-view flat with 70 replies a day and give me the link.');
  const paste = button(v, 'Paste from a doc');
  assert.equal(paste.getAttribute('aria-disabled'), 'true');
  assert.ok((paste.getAttribute('title') ?? paste.getAttribute('aria-description') ?? '').includes('later'));
  // Bounds.
  const down = v.host.querySelector('[data-step="down"]');
  for (let i = 0; i < 25; i++) await click(down);
  assert.equal(v.host.querySelector('[data-stepper] output').textContent, '10');
  assert.ok(down.disabled);
});

test('needs your own address: Start says why, How to and Wi-Fi only are there', async () => {
  const v = await deskCard('address');
  assert.equal(act(v, 'start').getAttribute('aria-disabled'), 'true');
  assert.ok(button(v, 'How to'));
  assert.ok(act(v, 'wifi-only'));
});

test('ready: QR, link and Try as a guest; no guest list', async () => {
  stub();
  const v = await deskCard('ready');
  assert.ok(v.host.querySelector('[data-join="full"]'));
  assert.ok(!v.host.querySelector('[data-guests]'));
  assert.equal(act(v, 'try').getAttribute('href'), m.validPocket(raw('ready')).desk.join.anywhere_url);
});

test('live: guests newest first, the waiting one marked, its words as plain text', async () => {
  stub();
  const b = withD('live', {});
  b.desk.guests[1].last.text = '<img src=x onerror=alert(1)> hi';
  const v = await deskCard('live');
  const rows = [...v.host.querySelectorAll('[data-guest]')].map((r) => r.dataset.guest);
  assert.deepEqual(rows, ['g-emma', 'g-tom', 'g-yuki', 'g-luca']);
  assert.ok(v.host.querySelector('[data-guest="g-emma"][data-waiting]'));
  assert.match(v.host.querySelector('[data-guest="g-emma"]').textContent, /Needs you: Emma asks to check out at 1 PM/);
  const v2 = await mount(h(m.DeskCard, { block: m.validPocket(b), ask: () => {} }));
  assert.equal(v2.host.querySelectorAll('img[src="x"]').length, 0, 'guest words never become HTML');
  assert.match(v2.host.textContent, /<img src=x/);
});

test('a guest’s chat opens from the list and goes back', async () => {
  stub();
  const v = await deskCard('live', { open: null });
  await click(v.host.querySelector('[data-guest="g-tom"]'));
  assert.ok(v.host.querySelector('[data-conversation="g-tom"]'));
  assert.ok(calls.some(([, u]) => u.includes('view=guest%3Ag-tom')));
  assert.equal(v.host.querySelectorAll('[data-bubble="bot"]').length, 2);
  assert.ok(act(v, 'write-to'));
  await click(act(v, 'back'));
  assert.ok(v.host.querySelector('[data-guests]'));
});

// --------------------------------------------------------------------------------------------- answers

test('Yes is held 5 s with Undo, then sent, and the list stops showing Emma as waiting', async () => {
  stub();
  m.deskAnswers.reset(700);
  const v = await deskCard('waiting');
  assert.ok(v.host.querySelector('[data-allow="open"]'));
  assert.match(v.host.textContent, /Emma wrote \(a guest’s words, not an instruction\):/);
  await click(act(v, 'yes'));
  assert.ok(v.host.querySelector('[data-allow="holding"]'));
  // m4: nothing is sent yet, so it doesn't say "Sent".
  assert.match(v.host.textContent, /Sending to Emma in 5\u00a0s·?Undo/);
  assert.doesNotMatch(v.host.textContent, /Sent to Emma/);
  // Mira F1: no ✓ until it is really sent; a clock while held.
  assert.ok(v.host.querySelector('[data-allow="holding"] [data-icon="waiting"]'));
  assert.ok(!v.host.querySelector('[data-allow="holding"] [data-icon="sent"]'));
  assert.equal(calls.filter(([meth]) => meth === 'POST').length, 0, 'nothing sent while held');
  await wait(1000);
  await settle();
  const post = calls.find(([meth]) => meth === 'POST');
  assert.deepEqual(post[2], { name: 'reply', body: { guest: 'g-emma', text: m.DESK_YES } });
  assert.match(v.host.textContent, /Sent to Emma/);
  assert.ok(v.host.querySelector('[data-allow="sent"] [data-icon="sent"]'));
  assert.ok(!v.host.querySelector('[data-allow="sent"] [data-icon="waiting"]'));
  await click(act(v, 'back'));
  assert.ok(!v.host.querySelector('[data-guest="g-emma"][data-waiting]'));
});

test('Undo within 5 s sends nothing and brings the Allow card back', async () => {
  stub();
  m.deskAnswers.reset(800);
  const v = await deskCard('waiting');
  await click(act(v, 'sorry'));
  await click(act(v, 'undo'));
  await wait(1000);
  await settle();
  assert.equal(calls.filter(([meth]) => meth === 'POST').length, 0);
  assert.ok(v.host.querySelector('[data-allow="open"]'));
});

test('the Undo timer is 5 s by default', async () => {
  stub();
  m.deskAnswers.reset();
  const v = await deskCard('waiting');
  await click(act(v, 'yes'));
  await wait(200);
  await settle();
  assert.equal(calls.filter(([meth]) => meth === 'POST').length, 0, 'still held after 200 ms');
  assert.equal(m.deskAnswers.get(m.deskKey('seaview', 'g-emma')).phase, 'holding');
});

test('a failed send rolls back: the Allow card returns with the error', async () => {
  stub({ action: 'fail' });
  const v = await deskCard('waiting');
  await click(act(v, 'write'));
  const ta = v.host.querySelector('textarea');
  const set = Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(ta, 'Yes, 12 is fine.');
  ta.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle();
  await click(act(v, 'send'));
  await settle();
  assert.ok(v.host.querySelector('[data-allow="open"]'), 'Allow card back');
  assert.match(v.host.querySelector('[data-failed]').textContent, new RegExp(m.DESK_FAILED));
  assert.deepEqual(calls.find(([meth]) => meth === 'POST')[2], { name: 'reply', body: { guest: 'g-emma', text: 'Yes, 12 is fine.' } });
});

test('No reply needed resolves now and sends no words', async () => {
  stub();
  const v = await deskCard('waiting');
  await click(act(v, 'resolve'));
  await settle();
  assert.deepEqual(calls.find(([meth]) => meth === 'POST')[2], { name: 'resolve', body: { guest: 'g-emma' } });
});

test('leaving the screen sends a held answer at once', async () => {
  stub();
  m.deskAnswers.reset();
  const v = await deskCard('waiting');
  await click(act(v, 'yes'));
  v.done();
  mounted.delete(v);
  await settle();
  assert.equal(calls.filter(([meth]) => meth === 'POST').length, 1);
});

test('withAnswers: an answered guest stops waiting until they ask again', () => {
  const d = m.validPocket(raw('live')).desk;
  m.deskAnswers.reset(10_000);
  m.deskAnswers.hold('seaview', 'g-emma', m.DESK_YES);
  const after = m.withAnswers('seaview', d);
  assert.equal(after.counts.waiting, 0);
  assert.equal(after.guests[0].waiting, false);
  // A later hand-off counts again.
  const later = { ...d, guests: d.guests.map((x) => (x.id === 'g-emma' ? { ...x, last: { ...x.last, at: new Date(Date.now() + 60_000).toISOString() } } : x)) };
  assert.equal(m.withAnswers('seaview', later).counts.waiting, 1);
});

// --------------------------------------------------------------------------------------------- Inbox

const inbox = (items = m.validDeskItems(m.DESK_WAITING)) => {
  m.inboxStore.patch({ data: { items: [], counts: { waiting_on_you: 0 }, since: new Date().toISOString(), now: new Date().toISOString() }, loading: false, error: null });
  return mount(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: false, deskItems: items }));
};

test('Inbox: one Needs-you row per waiting guest, with the Allow card inline and Bell', async () => {
  stub();
  const v = await inbox();
  const rows = v.host.querySelectorAll('[data-desk-item]');
  assert.equal(rows.length, 1);
  assert.match(rows[0].textContent, /Emma needs you/);
  assert.match(rows[0].textContent, /Sea-view flat · Guest Guide/);
  assert.ok(rows[0].querySelector('[data-allow="open"]'));
  assert.match(v.host.querySelector('#inbox-needs').textContent, /· 1/);
  assert.ok(!v.host.querySelector('[data-all-clear]'));
});

test('Inbox: a pocket_desk item in /api/inbox draws once, as the desk row, and /api/waiting is not read', async () => {
  stub();
  const [w] = m.DESK_WAITING.items;
  const item = { ...w, id: `desk:${w.pocket_code}:${w.guest_id}`, title: `${w.guest_name} needs you`, summary: `${w.pocket_title} · Guest Guide`, status: 'waiting', timestamp: w.asked_at, group: 'waiting_on_you', sort_key: [0], unread: true, link: { thread_id: null, channel_id: null, direct_kwant_id: null } };
  m.inboxStore.patch({ data: { items: [item], counts: { waiting_on_you: 1 }, since: new Date().toISOString(), now: new Date().toISOString() }, loading: false, error: null });
  const v = await mount(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: false }));
  assert.equal(v.host.querySelectorAll('[data-desk-item]').length, 1);
  assert.equal((v.host.textContent.match(/Emma needs you/g) ?? []).length, 1, 'no plain row as well');
  assert.match(v.host.querySelector('#inbox-needs').textContent, /· 1/);
  assert.ok(!calls.some(([, u]) => u.startsWith('/api/waiting')));
});

test('answering in the Inbox closes the card’s waiting row, and the other way round', async () => {
  stub();
  m.deskAnswers.reset(1500);
  const box = await inbox();
  const c = await deskCard('live', { open: null });
  assert.ok(c.host.querySelector('[data-guest="g-emma"][data-waiting]'));
  await click(box.host.querySelector('[data-action="yes"]'));
  assert.ok(!c.host.querySelector('[data-guest="g-emma"][data-waiting]'), 'card row closes at once');
  assert.ok(box.host.querySelector('[data-action="undo"]'), 'the Inbox keeps Undo in reach');
  await wait(1700);
  await settle();
  assert.equal(box.host.querySelectorAll('[data-desk-item]').length, 0, 'sent: the Inbox row goes');
  m.deskAnswers.reset(30);
  box.done();
  mounted.delete(box);
  c.done();
  mounted.delete(c);
  // The other way: the card's chat answers, the Inbox row goes.
  const box2 = await inbox();
  const c2 = await deskCard('waiting');
  await click(c2.host.querySelector('[data-action="resolve"]'));
  await settle();
  assert.equal(box2.host.querySelectorAll('[data-desk-item]').length, 0);
});

test('stopped: totals, deletion date, Delete chats now confirms; Done row line', async () => {
  stub();
  const v = await deskCard('ended');
  assert.match(v.host.textContent, /6 guests, Bell answered 41, you answered 3, 58 messages\./);
  assert.match(v.host.textContent, /deleted on Dec 17/);
  await click(act(v, 'delete'));
  assert.ok(v.host.querySelector('[data-confirm="delete"]'));
  const d = m.validPocket(raw('ended')).desk;
  assert.equal(m.deskDoneLine(d), 'Piran · 6 guests, Bell answered 41, you answered 3 · chats deleted Dec 17');
  const row = await mount(h('ul', null, h(m.DeskDoneRow, { d })));
  assert.match(row.host.textContent, /Sea-view flat · Guest Guide stopped/);
});

test('cap reached: the warn callout, only waiting guests, Raise the limit saves through set_cap', async () => {
  stub();
  const asked = [];
  const v = await deskCard('cap', { ask: (t) => asked.push(t) });
  assert.ok(v.host.querySelector('[data-callout="cap"]'));
  assert.equal(v.host.querySelectorAll('[data-guest]').length, 3);
  await click(act(v, 'raise'));
  await click(v.host.querySelector('[data-raise] [data-step="up"]'));
  await click(act(v, 'save-cap'));
  await settle();
  const sent = calls.filter(([meth, u]) => meth === 'POST' && u.endsWith('/action'));
  assert.deepEqual(sent.at(-1)[2], { name: 'set_cap', body: { cap: 60 } });
  assert.equal(asked.length, 0, 'no chat round-trip');
  assert.ok(!v.host.querySelector('[data-raise]'), 'the stepper closes');
  assert.ok(!v.host.querySelector('[data-callout="cap"]'), '50 of 60: not at the limit any more');
  assert.match(v.host.textContent, /50 of 60 replies Bell can write today/);
  assert.match(v.host.querySelector('[data-done="ok"]').textContent, /Bell can write 60 replies a day now\./);
});

test('a failed set_cap keeps the stepper open with the error', async () => {
  stub({ action: 'fail' });
  const v = await deskCard('cap');
  await click(act(v, 'raise'));
  await click(v.host.querySelector('[data-raise] [data-step="up"]'));
  await click(act(v, 'save-cap'));
  await settle();
  assert.ok(v.host.querySelector('[data-raise]'));
  assert.ok(v.host.querySelector('[data-callout="cap"]'));
  assert.match(v.host.querySelector('[data-done="error"]').textContent, /Couldn't change the limit: No such guest Try again\./);
});

// ------------------------------------------------------------------- desk-needs-you-buttons (part A)

const guideItem = (over = {}) => m.validDeskItems({ items: [{ ...structuredClone(m.DESK_WAITING.items[0]), ...over }] });

test('desk-needs-you-buttons: Yes says only “Yes, that’s fine.”, nothing about a last morning', () => {
  assert.equal(m.DESK_YES, 'Yes, that’s fine.');
});

test('desk-needs-you-buttons: Emma’s late check-out (ask “request”) has Yes, and it sends “Yes, that’s fine.”', async () => {
  stub();
  m.deskAnswers.reset(300);
  const items = guideItem({ ask: 'request' });
  assert.equal(items[0].need, 'request');
  assert.equal(items[0].ask, undefined, 'a guide row stays a guide row');
  const v = await inbox(items);
  await click(act(v, 'yes'));
  assert.match(v.host.textContent, /Sending to Emma in 5 s/);
  await wait(500);
  await settle();
  const post = calls.find(([meth]) => meth === 'POST');
  assert.deepEqual(post[2], { name: 'reply', body: { guest: 'g-emma', text: 'Yes, that’s fine.' } });
});

test('desk-needs-you-buttons: a plain hand-off (no ask) has no Yes, only Sorry, Write a reply and No reply needed', async () => {
  stub();
  const v = await inbox(guideItem({ ask: undefined, text: 'Price question', quote: 'How much is the parking?' }));
  assert.ok(v.host.querySelector('[data-allow="open"]'));
  assert.ok(!act(v, 'yes'), 'no Yes on a question');
  for (const a of ['sorry', 'write', 'resolve']) assert.ok(act(v, a), a);
});

test('desk-needs-you-buttons: the daily-cap row (ask “daily_cap”) never has Yes', async () => {
  stub();
  const items = guideItem({ ask: 'daily_cap', text: 'Daily limit reached', quote: 'What time is quiet time?' });
  assert.equal(items[0].need, 'daily_cap');
  const v = await inbox(items);
  assert.ok(!act(v, 'yes'));
  assert.ok(act(v, 'raise-cap'), 'desk-cap-row: the cap row gets its own actions');
});

test('desk-needs-you-buttons: in the card, Yes shows for a request guest only', async () => {
  stub();
  const v = await deskCard('waiting');
  assert.ok(act(v, 'yes'), 'Emma asks for a late check-out');
  const b = raw('waiting');
  b.desk.guests = b.desk.guests.map((x) => (x.id === 'g-emma' ? { ...x, ask: undefined } : x));
  const p = m.validPocket(b);
  const v2 = await mount(h(m.DeskCard, { block: p, ask: () => {}, openGuest: 'g-emma' }));
  assert.ok(v2.host.querySelector('[data-allow="open"]'));
  assert.ok(!act(v2, 'yes'), 'no ask on the wire: no Yes');
});

// ------------------------------------------------------------------- desk-cap-row (part B)

const capItems = (over = {}) => m.validDeskItems({ items: [{ ...structuredClone(m.DESK_CAP_WAITING.items[0]), ...over }] });
/** The desk's summary on GET, with this cap (Mira's example: 30 → 60). */
const capRead = (cap) => {
  const s = structuredClone(raw('cap').desk);
  s.counts = { ...s.counts, daily_cap: cap, replies_today: cap };
  return { pocket: { state: 'live' }, summary: s, join: null };
};
const capInbox = (items, onOpen = () => {}) => {
  m.inboxStore.patch({ data: { items: [], counts: { waiting_on_you: 0 }, since: new Date().toISOString(), now: new Date().toISOString() }, loading: false, error: null });
  return mount(h(m.InboxNew, { onOpen, noLoad: true, wide: false, deskItems: items }));
};

test('desk-cap-row: Yuki’s row reads “Daily limit reached · 13 messages wait” with Raise the limit to 60, Write a reply and No reply needed', async () => {
  stub({ read: capRead(30) });
  const items = capItems();
  assert.equal(items[0].waiting, 13);
  assert.ok(!JSON.stringify(items).includes('kw-never-shown-01'), 'the row data keeps no bot id');
  const v = await capInbox(items);
  const row = v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]');
  assert.ok(row);
  assert.equal(row.querySelector('[data-cap-title]').textContent.replace(/\u00a0/g, ' '), 'Daily limit reached · 13 messages wait');
  assert.equal(act(v, 'raise-cap').textContent.trim(), 'Raise the limit to 60');
  assert.equal(act(v, 'write').textContent.trim(), 'Write a reply');
  assert.equal(act(v, 'resolve').textContent.trim(), 'No reply needed');
  assert.deepEqual([...row.querySelectorAll('[data-allow-actions] button')].map((b) => b.dataset.action), ['raise-cap', 'write', 'resolve'], 'Raise, Write a reply, No reply needed, in that order');
  assert.equal(row.querySelector('[data-allow="cap"] p:last-child').textContent, 'Each answer from Bell uses your plan, so a higher limit can use more of it. Your reply goes to Yuki as you, marked “host”.');
});

test('desk-cap-row: Raise the limit sends set_cap with 60, then says “Limit raised to 60”', async () => {
  stub({ read: capRead(30) });
  const v = await capInbox(capItems());
  await click(act(v, 'raise-cap'));
  await settle();
  const post = calls.find(([meth]) => meth === 'POST');
  assert.equal(post[1], '/api/pockets/seaview/action');
  assert.deepEqual(post[2], { name: 'set_cap', body: { cap: 60 } });
  assert.equal(v.host.querySelector('[data-cap-done="ok"]').textContent, 'Limit raised to 60. Bell answers new messages again; Yuki’s 13 still need you.');
  assert.ok(!act(v, 'raise-cap'), 'raised once; the row leaves when the server stops listing it');
  assert.ok(v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]'), 'the row stays until the server drops it');
});

test('desk-cap-row: the next step doubles the current cap; the row’s own cap wins; no cap known → 60; at 200 no Raise', async () => {
  assert.equal(m.deskNextCap(30), 60);
  assert.equal(m.deskNextCap(50), 100);
  assert.equal(m.deskNextCap(150), 200);
  assert.equal(m.deskNextCap(null), 60);
  stub({ read: capRead(50) });
  let v = await capInbox(capItems());
  assert.equal(act(v, 'raise-cap').textContent.trim(), 'Raise the limit to 100');
  v.done();
  m.deskAnswers.reset(); // the desk's cap is shared per desk: start each case clean
  stub();
  v = await capInbox(capItems({ daily_cap: 30 }));
  assert.equal(act(v, 'raise-cap').textContent.trim(), 'Raise the limit to 60', 'a cap on the row needs no read');
  assert.ok(!calls.some(([meth, u]) => meth === 'GET' && u.startsWith('/api/pockets/')));
  v.done();
  m.deskAnswers.reset();
  stub(); // the read fails (503)
  v = await capInbox(capItems());
  assert.equal(act(v, 'raise-cap').textContent.trim(), 'Raise the limit to 60');
  v.done();
  m.deskAnswers.reset();
  stub({ read: capRead(200) });
  v = await capInbox(capItems());
  assert.ok(!act(v, 'raise-cap'));
  assert.equal(v.host.querySelector('[data-allow="cap"] p:last-child').textContent, 'The limit is already 200, the most a desk can have. Bell answers again tomorrow. Your reply goes to Yuki as you, marked “host”.');
  assert.ok(act(v, 'write') && act(v, 'resolve'), 'at 200 the host can still reply or close');
});

const typeReply = async (v, text) => {
  await click(act(v, 'write'));
  const ta = v.host.querySelector('textarea');
  assert.equal(ta.getAttribute('placeholder'), 'Write to Yuki');
  assert.equal(v.host.querySelector(`label[for="${ta.id}"]`).textContent, 'Your reply to Yuki');
  const set = Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(ta, text);
  ta.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle();
  return ta;
};

test('desk-cap-row: “Write a reply” posts reply with the text to Yuki, and the sent row leaves', async () => {
  stub({ read: capRead(30) });
  const v = await capInbox(capItems());
  await typeReply(v, 'The hair dryer is in the hall cupboard. Quiet time is 10 PM.');
  await click(act(v, 'send'));
  await settle();
  const post = calls.find(([meth]) => meth === 'POST');
  assert.equal(post[1], '/api/pockets/seaview/action');
  assert.deepEqual(post[2], { name: 'reply', body: { guest: 'g-yuki', text: 'The hair dryer is in the hall cupboard. Quiet time is 10 PM.' } });
  assert.ok(!v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]'), 'sent: the row goes, as other rows do');
});

test('desk-cap-row: Esc closes the reply box; a failed reply keeps the row with the failed line', async () => {
  stub({ read: capRead(30), action: 'fail' });
  const v = await capInbox(capItems());
  const ta = await typeReply(v, 'Hall cupboard.');
  ta.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.ok(!v.host.querySelector('textarea') && act(v, 'raise-cap'), 'Esc: back to the buttons');
  await typeReply(v, 'Hall cupboard.');
  v.host.querySelector('textarea').dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await settle();
  assert.ok(v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"] [data-failed]'));
  assert.ok(act(v, 'write'));
});

test('desk-cap-row: “No reply needed” posts resolve and sends no words', async () => {
  stub({ read: capRead(30) });
  const v = await capInbox(capItems());
  await click(act(v, 'resolve'));
  await settle();
  assert.deepEqual(calls.find(([meth]) => meth === 'POST')[2], { name: 'resolve', body: { guest: 'g-yuki' } });
  assert.ok(!v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]'));
});

test('desk-cap-row: the cap row has no “Answer in”, Yes, Sorry or Book, even with a chat link and an opener', async () => {
  stub({ read: capRead(30) });
  const opened = [];
  const v = await capInbox(capItems({ actions: ['book', 'reply', 'resolve'] }), (i) => opened.push(i));
  for (const a of ['open-chat', 'yes', 'sorry', 'book']) assert.ok(!act(v, a), `no ${a}`);
  assert.ok(!/Answer in|Sorry|Yes, that|Book a time/.test(v.host.textContent));
  assert.equal(opened.length, 0);
});

test('desk-cap-row (Rigel P1): a Booking Desk’s cap row (mode “booking”) gets the cap card, not the booking row', async () => {
  stub({ read: capRead(30) });
  const items = capItems({ mode: 'booking' });
  assert.equal(items[0].ask, undefined, 'not a booking item');
  assert.equal(items[0].need, 'daily_cap');
  assert.equal(items[0].waiting, 13);
  const v = await capInbox(items);
  const row = v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]');
  assert.ok(row.querySelector('[data-allow="cap"]'), 'the cap card');
  assert.equal(row.querySelector('[data-cap-title]').textContent.replace(/ /g, ' '), 'Daily limit reached · 13 messages wait');
  assert.equal(act(v, 'raise-cap').textContent.trim(), 'Raise the limit to 60');
  assert.ok(!act(v, 'sorry'));
  assert.match(row.textContent, /Sea-view flat · Booking Desk ·/);
  assert.equal(m.validDeskItems({ items: [{ ...structuredClone(m.DESK_CAP_WAITING.items[0]), mode: 'booking', ask: 'handoff', waiting: 4 }] })[0].ask, 'handoff', 'a booking hand-off is still a booking item');
});

test('desk-cap-row (Mira m2, Rigel P11): two cap rows of one desk share one read and one cap, and update together', async () => {
  stub({ read: capRead(30) });
  const items = m.validDeskItems({
    items: [structuredClone(m.DESK_CAP_WAITING.items[0]), { ...structuredClone(m.DESK_CAP_WAITING.items[0]), id: 'desk:seaview:g-kai', guest_id: 'g-kai', guest_name: 'Kai', waiting: 1 }],
  });
  const v = await capInbox(items);
  assert.equal(calls.filter(([meth, u]) => meth === 'GET' && u.startsWith('/api/pockets/seaview')).length, 1, 'one read per desk');
  const yuki = v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]');
  const kai = v.host.querySelector('[data-desk-item="desk:seaview:g-kai"]');
  assert.equal(kai.querySelector('[data-action="raise-cap"]').textContent.trim(), 'Raise the limit to 60');
  await click(yuki.querySelector('[data-action="raise-cap"]'));
  await settle();
  assert.equal(kai.querySelector('[data-action="raise-cap"]').textContent.trim(), 'Raise the limit to 120', 'Kai’s row follows the new cap');
  await click(kai.querySelector('[data-action="raise-cap"]'));
  await settle();
  assert.deepEqual(calls.filter(([meth]) => meth === 'POST').map((c) => c[2].body.cap), [60, 120]);
  assert.equal(kai.querySelector('[data-cap-done="ok"]').textContent, 'Limit raised to 120. Bell answers new messages again; Kai’s message still needs you.');
});

test('desk-cap-row (Mira F2): the success line with no count says the waiting messages still need you', async () => {
  stub({ read: capRead(30) });
  const v = await capInbox(capItems({ waiting: undefined }));
  await click(act(v, 'raise-cap'));
  await settle();
  assert.equal(v.host.querySelector('[data-cap-done="ok"]').textContent, 'Limit raised to 60. Bell answers new messages again; Yuki’s waiting messages still need you.');
});

test('desk-cap-row: waiting 1 → “1 message waits”; no waiting on the wire → no count', async () => {
  stub({ read: capRead(30) });
  let v = await capInbox(capItems({ waiting: 1 }));
  assert.equal(v.host.querySelector('[data-cap-title]').textContent.replace(/\u00a0/g, ' '), 'Daily limit reached · 1 message waits');
  v.done();
  const items = capItems({ waiting: undefined });
  assert.equal(items[0].waiting, undefined);
  v = await capInbox(items);
  assert.equal(v.host.querySelector('[data-cap-title]').textContent.replace(/\u00a0/g, ' '), 'Daily limit reached');
  assert.ok(!/wait/.test(v.host.querySelector('[data-cap-title]').textContent));
  v.done();
  for (const junk of [0, -2, 1.5, '13', null]) assert.equal(capItems({ waiting: junk })[0].waiting, undefined, `junk ${junk}`);
  assert.equal(m.validDeskItems({ items: [{ ...m.DESK_WAITING.items[0], waiting: 4 }] })[0].waiting, undefined, 'a count only on a cap row');
});

test('desk-cap-row: a failed set_cap keeps the row and says why in the server’s words', async () => {
  stub({ read: capRead(30), action: 'fail' });
  const v = await capInbox(capItems());
  await click(act(v, 'raise-cap'));
  await settle();
  assert.ok(v.host.querySelector('[data-desk-item="desk:seaview:g-yuki"]'));
  const err = v.host.querySelector('[data-cap-done="error"]');
  assert.equal(err.getAttribute('role'), 'alert');
  assert.equal(err.textContent, 'Couldn’t raise the limit: No such guest');
  assert.equal(act(v, 'raise-cap').textContent.trim(), 'Raise the limit to 60', 'Raise stays, so the host can try again');
});
