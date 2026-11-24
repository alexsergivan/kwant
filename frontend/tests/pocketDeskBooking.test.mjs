// Booking Desk Pocket card (pa13b, autonomy/pocket-apps/pa13/pa13b-design.md, pa13b-card-brief.md, pa13b-card-impl.md):
// the validator in booking mode (good input, missing fields fall back, junk rejected), QuickFill (no overlaps, past
// days skipped), the 5 s Undo on a booking's Yes, "answering in either place closes both", the odd-request card and
// the times editor's refusal. "Now" is Tue Dec 1 2026, 11:40 AM in the studio, as in Mira's sample.
process.env.TZ = 'Europe/Berlin';
import assert from 'node:assert/strict';
import test, { afterEach, before } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'HTMLTextAreaElement', 'HTMLInputElement', 'HTMLSelectElement', 'PopStateEvent']) {
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
  export { validDeskItems, deskAnswers, deskKey, DeskCard, deskConversationCache } from './src/components/results/PocketDesk';
  export { bookingFeatures } from './src/components/results/PocketDeskBooking';
  export { bookingClock, bookingKey, applyFill, fillWeek, validBooking, withBookingAnswers, requestsOf, bookingChip, nextSlotFor } from './src/components/results/deskBooking';
  export { BOOKING_FIXTURES, BOOKING_CONVERSATIONS, BOOKING_WAITING, BOOKING_OPEN, BOOKING_TODAY, DESK_FIXTURES } from './src/components/results/fixtures';
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
before(() => m.bookingClock.set(m.BOOKING_TODAY));
afterEach(() => {
  m.deskAnswers.reset();
  for (const v of mounted) v.done();
  mounted.clear();
  m.deskAnswers.reset();
  m.deskConversationCache.clear();
  m.bookingFeatures.setBook?.(true);
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
const F = m.BOOKING_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const withD = (name, over) => {
  const b = raw(name);
  b.desk = { ...b.desk, ...over };
  return b;
};
const bad = (b, why) => assert.equal(m.validPocket(b), null, why);
const calls = [];
const posts = () => calls.filter(([meth]) => meth === 'POST');
function stub({ action = 'ok', status = 200 } = {}) {
  calls.length = 0;
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    calls.push([method, String(url), init.body ? JSON.parse(init.body) : null]);
    const u = String(url);
    const json = (b, s = 200) => new Response(JSON.stringify(b), { status: s, headers: { 'content-type': 'application/json' } });
    if (u.startsWith('/api/waiting')) return json(m.BOOKING_WAITING);
    const view = /view=guest%3A([^&]+)/.exec(u);
    if (view) return json({ pocket: { state: 'live' }, summary: raw('odd').desk, join: null, conversation: m.BOOKING_CONVERSATIONS[view[1]] });
    if (u.endsWith('/action')) {
      if (action !== 'ok') return json(action, status);
      return json({ pocket: { state: 'live' }, summary: raw('requests').desk, join: null });
    }
    if (method === 'GET') return json({ error: 'offline' }, 503);
    return json({});
  };
}
const deskCard = (name, opts = {}) => {
  const b = m.validPocket(raw(name));
  assert.ok(b, name);
  return mount(h(m.DeskCard, { block: b, ask: opts.ask ?? (() => {}), openGuest: opts.open === undefined ? (m.BOOKING_OPEN[name] ?? null) : opts.open }));
};
const act = (v, a) => v.host.querySelector(`[data-action="${a}"]`);
const acts = (v, a) => [...v.host.querySelectorAll(`[data-action="${a}"]`)];
const askedNow = () => ({ ...m.BOOKING_WAITING, items: m.BOOKING_WAITING.items.map((i) => ({ ...i, asked_at: new Date(Date.now() - 60_000).toISOString() })) });
const inbox = (items = m.validDeskItems(askedNow())) => {
  m.inboxStore.patch({ data: { items: [], counts: { waiting_on_you: 0 }, since: new Date().toISOString(), now: new Date().toISOString() }, loading: false, error: null });
  return mount(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: false, deskItems: items }));
};

// --------------------------------------------------------------------------------------------- validator

test('booking mode: every fixture validates as a Booking Desk and keeps no bot id', () => {
  for (const name of Object.keys(F)) {
    const b = m.validPocket(raw(name));
    assert.ok(b, name);
    assert.equal(b.desk.mode, 'booking', name);
    assert.ok(b.desk.booking, name);
    assert.ok(!JSON.stringify(b.desk).includes('kw-never-shown-02'), `${name}: answer_bot id dropped`);
  }
  assert.equal(m.kitLabel('desk', 'booking'), 'Booking Desk');
  assert.equal(m.kitLabel('desk'), 'Guest Guide', 'the guide label is unchanged');
  const s = m.validPocket(raw('requests')).desk.booking;
  assert.equal(s.slots.length, 32);
  assert.equal(s.requests, 2);
  assert.equal(s.booked_upcoming, 3, 'Mia’s 9 AM today is past');
  assert.equal(s.open_left, 29 - 2 - 3, 'Tue 9, 10 and 11 AM are past and hold no open seats');
  // A guide desk stays a guide desk: no mode, no booking part.
  const guide = m.validPocket(structuredClone(m.DESK_FIXTURES.live.results.blocks[0]));
  assert.equal(guide.desk.mode, undefined);
  assert.equal(guide.desk.booking, undefined);
});

test('booking mode: missing fields fall back', () => {
  // mode read from spec.mode; slots from spec.slots; counts worked out from the slots.
  const b = raw('requests');
  const slots = b.desk.slots;
  delete b.desk.mode;
  delete b.desk.slots;
  b.desk.spec = { mode: 'booking', slots };
  const v = m.validPocket(b);
  assert.ok(v);
  assert.equal(v.desk.mode, 'booking');
  assert.equal(v.desk.booking.slots.length, 32);
  // No slots at all: an empty week, not a rejection.
  const e = m.validPocket(withD('setup', { slots: undefined }));
  assert.ok(e);
  assert.deepEqual(e.desk.booking.slots, []);
  // A slot without seats, minutes, label or people gets the defaults.
  const one = m.validBooking({ slots: [{ id: 's1', start: '2026-12-02T10:00' }] });
  assert.ok(one);
  assert.equal(one.slots[0].seats, 1);
  assert.ok(one.slots[0].minutes > 0);
  assert.deepEqual(one.slots[0].requested, []);
  assert.deepEqual(one.slots[0].confirmed, []);
  // Server counts win when given.
  assert.equal(m.validBooking({ slots: [], open_left: 7 }).open_left, 7);
});

test('booking mode: junk is rejected', () => {
  bad(withD('open', { slots: 'many' }), 'slots not a list');
  bad(withD('open', { slots: [{ id: 's1', start: 'Tuesday 10' }] }), 'start not a wall-clock time');
  bad(withD('open', { slots: [{ id: 's1', start: '2026-12-02T10:00+01:00' }] }), 'a zone on start');
  bad(withD('open', { slots: [{ id: 's1', start: '2026-12-02T10:00' }, { id: 's1', start: '2026-12-02T11:00' }] }), 'duplicate ids');
  bad(withD('open', { slots: [{ id: 's1', start: '2026-12-02T10:00', seats: 0 }] }), 'no seats');
  bad(withD('open', { slots: [{ id: 's1', start: '2026-12-02T10:00', seats: 99 }] }), 'too many seats');
  bad(withD('open', { slots: [{ id: 'bad id!', start: '2026-12-02T10:00' }] }), 'slot id');
  bad(withD('open', { slots: [{ id: 's1', start: '2026-12-02T10:00', requested: [{ guest: 'bad id!' }] }] }), 'person id');
  bad(withD('open', { slots: Array.from({ length: 201 }, (_, i) => ({ start: `2026-12-02T10:${String(i % 60).padStart(2, '0')}` })) }), 'over 200 times');
  bad(withD('open', { requests: -1 }), 'negative count');
});

test('Needs-you items: ask booking and ask handoff, booking ids carry the time', () => {
  const items = m.validDeskItems(m.BOOKING_WAITING);
  assert.equal(items.length, 2);
  assert.equal(items[0].ask, 'booking');
  assert.equal(items[0].id, 'desk:studio:g-emma@2026-12-02T10:00');
  assert.equal(items[0].slot_start, '2026-12-02T10:00');
  assert.equal(items[1].ask, 'handoff');
  assert.equal(items[1].ask_slot_start, '2026-12-01T18:00');
  assert.ok(!JSON.stringify(items).includes('kw-never-shown-02'));
});

// --------------------------------------------------------------------------------------------- QuickFill

test('QuickFill: the rule makes the week, skipping past days and today’s past hours', () => {
  const rule = { days: [0, 1, 2, 3, 4], from: '09:00', to: '17:00', minutes: 60, seats: 1 };
  const made = m.fillWeek(rule, '2026-11-30');
  assert.ok(!made.some((s) => s.start.startsWith('2026-11-30')), 'Monday is past');
  assert.ok(!made.some((s) => s.start <= '2026-12-01T11:40'), 'no time before now');
  assert.ok(made.some((s) => s.start === '2026-12-01T12:00'), 'today from noon');
  assert.equal(made.filter((s) => s.start.startsWith('2026-12-02')).length, 8);
  assert.equal(made.length, 5 + 8 * 3);
  // No time runs past `to`.
  assert.ok(m.fillWeek({ ...rule, minutes: 90 }, '2026-12-07').every((s) => s.start.slice(11) <= '15:30'));
  // A bad rule makes nothing.
  assert.deepEqual(m.fillWeek({ ...rule, from: '17:00', to: '09:00' }, '2026-12-07'), []);
});

test('QuickFill: no overlaps with times in use, other days untouched, at most 200', () => {
  const b = m.validPocket(raw('requests')).desk.booking;
  const rule = { days: [2], from: '09:00', to: '12:00', minutes: 45, seats: 1 };
  const r = m.applyFill(b.slots, rule, '2026-11-30');
  const wed = r.slots.filter((s) => s.start.startsWith('2026-12-02'));
  assert.ok(wed.some((s) => s.start === '2026-12-02T10:00' && s.requested.length === 1), 'Emma’s held time stays');
  const mins = (s) => +s.start.slice(11, 13) * 60 + +s.start.slice(14);
  for (const x of wed)
    for (const y of wed) if (x !== y) assert.ok(mins(x) + x.minutes <= mins(y) || mins(y) + y.minutes <= mins(x), `${x.start} overlaps ${y.start}`);
  assert.equal(r.slots.filter((s) => s.start.startsWith('2026-12-03')).length, b.slots.filter((s) => s.start.startsWith('2026-12-03')).length, 'Thursday untouched');
  assert.ok(r.slots.some((s) => s.start === '2026-12-01T09:00'), 'past times kept');
  const big = m.applyFill([], { days: [0, 1, 2, 3, 4, 5, 6], from: '00:00', to: '23:55', minutes: 5, seats: 1 }, '2026-12-07');
  assert.equal(big.slots.length, 200);
  assert.equal(big.capped, true);
});

// --------------------------------------------------------------------------------------------- card

test('each booking state draws its chip, Booking Desk label, and holo never shows', async () => {
  const want = { setup: /Set up · not shared yet/, open: /Open · no requests/, requests: /Live · 2 requests/, odd: /Live · 3 for you/, cap: /Daily limit reached/, ended: /Stopped/ };
  for (const [name, re] of Object.entries(want)) {
    const v = await deskCard(name, { open: null });
    assert.match(v.host.textContent, re, name);
    assert.match(v.host.textContent, /Booking Desk/, name);
    assert.ok(!v.host.querySelector('[class*="holo"]'), `${name}: no holo`);
    assert.equal(v.host.querySelector('[data-mode]')?.getAttribute('data-mode'), 'booking', name);
    v.done();
    mounted.delete(v);
  }
});

test('the who line says how long ago the guest asked, never a bare length', async () => {
  const v = await deskCard('requests', { open: null });
  const cards = [...v.host.querySelectorAll('[data-booking-allow="open"]')].map((c) => c.textContent);
  assert.match(cards[0], /asked 10:32 AM · 1 hour ago/);
  assert.match(cards[1], /asked 11:05 AM · 35 min ago/);
  v.done();
  mounted.delete(v);
});

test('requests: at most 3 cards, oldest first, then N more opens the Inbox', async () => {
  stub();
  const v = await deskCard('manyRequests');
  const cards = [...v.host.querySelectorAll('[data-booking-allow="open"]')];
  assert.equal(cards.length, 3);
  assert.match(cards[0].textContent, /Emma/, 'Emma asked first');
  assert.match(cards[1].textContent, /Tom/);
  const more = act(v, 'more-requests');
  assert.match(more.textContent, /2 more requests/);
  window.history.replaceState(null, '', '/s/home/');
  let path = null;
  window.addEventListener('popstate', () => (path = window.location.pathname + window.location.search), { once: true });
  await click(more);
  assert.ok(path && /inbox/.test(path), `the Inbox opens: ${path}`);
});

test('a booking’s Yes is held 5 s with Undo, then confirm is sent and Booked is a plain ok chip', async () => {
  stub();
  m.deskAnswers.reset(700);
  const v = await deskCard('requests');
  await click(acts(v, 'confirm')[0]);
  const held = v.host.querySelector('[data-booking-allow="holding"]');
  assert.ok(held);
  assert.ok(held.querySelector('.kw-chip-ok[data-chip="booked"]'));
  assert.ok(!held.querySelector('[class*="holo"]'));
  assert.ok(act(v, 'undo'));
  assert.equal(posts().length, 0, 'nothing sent while held');
  await wait(1000);
  await settle();
  assert.deepEqual(posts()[0][2], { name: 'confirm', body: { guest: 'g-emma', slot: 's-1202-10' } });
});

test('the booking Undo timer is 5 s by default, and Undo sends nothing', async () => {
  stub();
  m.deskAnswers.reset();
  const v = await deskCard('requests');
  await click(acts(v, 'confirm')[0]);
  await wait(200);
  await settle();
  assert.equal(posts().length, 0, 'still held after 200 ms');
  assert.equal(m.deskAnswers.get(m.bookingKey('studio', 'g-emma', 's-1202-10')).phase, 'holding');
  m.deskAnswers.reset(600);
  await click(acts(v, 'confirm')[0]);
  await click(act(v, 'undo'));
  await wait(800);
  await settle();
  assert.equal(posts().length, 0);
  assert.equal(v.host.querySelectorAll('[data-booking-allow="open"]').length, 2, 'the request card comes back');
});

test('Sorry frees the time with the host’s line, held with Undo', async () => {
  stub();
  m.deskAnswers.reset(300);
  const v = await deskCard('requests');
  await click(acts(v, 'decline')[0]);
  const box = v.host.querySelector('textarea');
  box.value = 'Wednesday is full, sorry!';
  box.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle();
  await click(act(v, 'decline-send'));
  assert.ok(v.host.querySelector('[data-answer="decline"]'));
  await wait(500);
  await settle();
  assert.equal(posts()[0][2].name, 'decline');
  assert.equal(posts()[0][2].body.guest, 'g-emma');
});

test('withBookingAnswers: a held Yes books at once, a failed one changes nothing', () => {
  const b = m.validPocket(raw('requests')).desk.booking;
  const yes = m.withBookingAnswers('studio', b, (k) => (k === m.bookingKey('studio', 'g-emma', 's-1202-10') ? { phase: 'holding', kind: 'confirm' } : undefined));
  const wed = yes.slots.find((s) => s.id === 's-1202-10');
  assert.equal(wed.requested.length, 0);
  assert.equal(wed.confirmed[0].guest, 'g-emma');
  const failed = m.withBookingAnswers('studio', b, () => ({ phase: 'failed', kind: 'confirm' }));
  assert.equal(failed.slots.find((s) => s.id === 's-1202-10').requested.length, 1);
});

test('answering in the Inbox closes the card’s request, and the other way round', async () => {
  stub();
  m.deskAnswers.reset(1200);
  const box = await inbox();
  const c = await deskCard('requests');
  assert.equal(box.host.querySelectorAll('[data-desk-item]').length, 2);
  assert.match(box.host.textContent, /Emma asks for a time/);
  assert.match(box.host.textContent, /Studio Nika · Booking Desk/);
  await click(box.host.querySelector('[data-ask="booking"] [data-action="confirm"]'));
  assert.ok(c.host.querySelector('[data-booking-allow="holding"]'), 'the card shows the held answer at once');
  assert.ok(box.host.querySelector('[data-action="undo"]'), 'the Inbox keeps Undo in reach');
  await wait(1400);
  await settle();
  assert.equal(box.host.querySelectorAll('[data-ask="booking"]').length, 0, 'sent: the Inbox row goes');
  assert.equal(posts().length, 1, 'one answer, sent once');
  m.deskAnswers.reset(200);
  for (const v of [box, c]) (v.done(), mounted.delete(v));
  // The other way: the card answers, the Inbox row goes.
  const box2 = await inbox();
  const c2 = await deskCard('requests');
  await click(c2.host.querySelector('[data-booking-allow="open"] [data-action="confirm"]'));
  assert.ok(box2.host.querySelector('[data-ask="booking"] [data-booking-allow="holding"]'), 'the Inbox shows it held too');
  await wait(400);
  await settle();
  assert.equal(box2.host.querySelectorAll('[data-ask="booking"]').length, 0);
});

test('the odd request: no plain Yes, Book a time picks an open slot and sends book', async () => {
  stub();
  const v = await deskCard('odd');
  await settle();
  assert.ok(!act(v, 'yes'), 'no plain Yes on an odd request');
  await click(act(v, 'book-for'));
  assert.ok(v.host.querySelector('[data-book-for]'));
  await click(v.host.querySelector('[data-pick]'));
  const book = act(v, 'book');
  assert.match(book.textContent, /^Book Ben for /);
  await click(book);
  const p = posts()[0];
  assert.equal(p[2].name, 'book');
  assert.equal(p[2].body.guest, 'g-ben');
  assert.ok(p[2].body.slot || p[2].body.new_slot);
});

test('Book a time hides when the server doesn’t know the book action', async () => {
  stub({ action: { error: 'bad_action', message: 'unknown action book' }, status: 400 });
  const v = await deskCard('odd');
  await click(act(v, 'book-for'));
  await click(v.host.querySelector('[data-pick]'));
  await click(act(v, 'book'));
  await settle();
  assert.equal(m.bookingFeatures.canBook(), false);
  const v2 = await deskCard('odd');
  assert.ok(!v2.host.querySelector('[data-action="book-for"]'));
});

test('edit times: a lock tap shows the refusal and keeps the editor open; 409 slot_in_use does too', async () => {
  // Vega's 409 body: {error, message, slot, start, name} at the top level.
  stub({ action: { error: 'slot_in_use', message: 'Emma holds it', slot: 's-1202-10', start: '2026-12-02T10:00', name: 'Emma' }, status: 409 });
  const v = await deskCard('requests', { open: null });
  await click(act(v, 'edit-times'));
  assert.ok(act(v, 'save-times'), 'editor open');
  await click(v.host.querySelector('[data-action="locked"]'));
  assert.ok(v.host.querySelector('[data-refusal]'), 'lock tap: refusal line');
  assert.ok(act(v, 'save-times'), 'still editing');
  v.done();
  mounted.delete(v);
  // A fresh editor with no lock tap: only the 409 can draw the refusal.
  const w = await deskCard('requests', { open: null });
  await click(act(w, 'edit-times'));
  assert.ok(!w.host.querySelector('[data-refusal]'));
  await click(act(w, 'save-times'));
  await settle();
  assert.match(w.host.querySelector('[data-refusal]')?.textContent ?? '', /Emma/);
  const ring = w.host.querySelector('[data-slot="2026-12-02T10:00"][data-refused]');
  assert.ok(ring, 'the time gets the warn ring');
  // Design check FIX 1/3: the warn border beats the requested chip's sky border; no ring animation under reduced motion.
  assert.match(ring.className, /!border-\[color-mix\(in_srgb,var\(--kw-warn\)/);
  assert.match(ring.className, /motion-reduce:transition-none/);
  await settle();
  assert.equal(posts()[0][2].name, 'set_slots');
  assert.ok(act(w, 'save-times'), 'editor stays open');
});

test('Vega’s shapes: nameless bookings, 60 seats, 24 h, mode on items, booking{} fallback', () => {
  const v = m.validBooking({
    slots: [
      { id: 's1', start: '2026-12-02T10:00', minutes: 1440, seats: 60, past: false, open_seats: 59, confirmed: [{ guest: null, name: null, at: '2026-11-30T10:00:00Z', by: 'guest' }] },
    ],
  });
  assert.ok(v, 'a nameless booking, 60 seats and a day-long slot are fine');
  assert.equal(v.slots[0].confirmed[0].name, null);
  assert.equal(v.booked_upcoming, 1);
  assert.equal(m.validBooking({ slots: [{ id: 's1', start: '2026-12-02T10:00', requested: [{ guest: null, name: null }] }] }), undefined, 'a request needs its guest');
  assert.equal(m.validBooking({ slots: [{ id: 's1', start: '2026-12-02T10:00', seats: 61 }] }), undefined);
  const [emma, ben] = m.BOOKING_WAITING.items;
  const guideAsk = m.validDeskItems({ items: [{ ...ben, mode: 'guide' }] });
  assert.equal(guideAsk[0].ask, undefined, 'a guide desk item stays a guide item');
  const nested = m.validDeskItems({ items: [{ ...emma, mode: 'booking', slot: undefined, slot_start: undefined, booking: { slot: 's7', start: '2026-12-02T10:00', label: null } }] });
  assert.equal(nested[0].slot, 's7');
  assert.equal(nested[0].slot_start, '2026-12-02T10:00');
  const noQuote = m.validDeskItems({ items: [{ ...emma, mode: 'booking', quote: '' }] });
  assert.equal(noQuote[0].quote, null);
});

test('chip words', () => {
  assert.equal(m.bookingChip(0, 0), 'Open · no requests');
  assert.equal(m.bookingChip(2, 0), 'Live · 2 requests');
  assert.equal(m.bookingChip(2, 1), 'Live · 3 for you');
});
