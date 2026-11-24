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
  export { validDesk, validDeskItems, validConversation, deskStage, deskMood, deskAnswers, deskKey, withAnswers, DeskCard, DeskNeedsRow, DeskDoneRow, deskDoneLine, deskConversationCache, DESK_YES, DESK_SORRY, DESK_FAILED, DESK_LATER } from './src/components/results/PocketDesk';
  export { DESK_FIXTURES, DESK_CONVERSATIONS, DESK_WAITING, DESK_OPEN } from './src/components/results/fixtures';
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
  assert.match(v.host.textContent, /Sent to Emma · Undo for 5\u00a0s/);
  assert.equal(calls.filter(([meth]) => meth === 'POST').length, 0, 'nothing sent while held');
  await wait(1000);
  await settle();
  const post = calls.find(([meth]) => meth === 'POST');
  assert.deepEqual(post[2], { name: 'reply', body: { guest: 'g-emma', text: m.DESK_YES } });
  assert.match(v.host.textContent, /Sent to Emma/);
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
