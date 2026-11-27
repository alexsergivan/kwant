// desk-card-in-chat (autonomy/desk-card-in-chat/brief.md): the card in a bot's chat once the desk is live.
// m3: Stop while a guest waits names them first; the started card keeps a compact QR and link (or says why
// phones can't reach it); a booking guest asking for a listed time is counted once; the live week opens on the
// next booking, also when the live read lands after the first paint. "Now" is Tue Dec 1 2026, 11:40 AM.
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
  export { bookingFeatures, LiveWeek } from './src/components/results/PocketDeskBooking';
  export { stillWaitingLine, DeskCompactJoin, DESK_LATER } from './src/components/results/PocketDesk';
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
const D = m.DESK_FIXTURES;
const guideRaw = (name, over = {}) => {
  const b = structuredClone(D[name].results.blocks[0]);
  b.desk = { ...b.desk, ...over };
  return b;
};
const guideCard = (b) => {
  const v = m.validPocket(b);
  assert.ok(v, 'valid desk');
  return mount(h(m.DeskCard, { block: v, ask: () => {}, openGuest: null }));
};
const act = (v, a) => v.host.querySelector(`[data-action="${a}"]`);
const text = (v) => v.host.textContent;

test('stillWaitingLine: nobody, one by name, several by count', () => {
  const g = (name, waiting) => ({ id: name, name, waiting });
  assert.equal(m.stillWaitingLine([]), null);
  assert.equal(m.stillWaitingLine([g('Emma', false)]), null);
  assert.equal(m.stillWaitingLine([g('Emma', false), g('Yuki', true)]), 'Yuki is still waiting.');
  assert.equal(m.stillWaitingLine([g('Ana', true), g('Yuki', true), g('Tom', true)]), '3 guests are still waiting.');
});

test('m3: Stop while Emma waits asks "Emma is still waiting. Stop anyway?" and Answer Emma opens her chat', async () => {
  stub();
  const b = guideRaw('live');
  const waiting = b.desk.guests.filter((x) => x.waiting);
  assert.equal(waiting.length, 1, 'the live fixture has one guest waiting');
  const who = waiting[0].name;
  const v = await guideCard(b);
  await click(act(v, 'stop'));
  const box = v.host.querySelector('[data-confirm="stop"]');
  assert.equal(box.getAttribute('data-waiting'), 'yes');
  assert.equal(box.getAttribute('aria-label'), `${who} is still waiting. Stop anyway?`);
  assert.match(v.host.querySelector('[data-still-waiting]').textContent, new RegExp(`${who} is still waiting\\. Stop anyway\\?`));
  assert.equal(act(v, 'confirm-stop').textContent, 'Stop anyway');
  assert.equal(act(v, 'answer-first').textContent, `Answer ${who}`);
  await click(act(v, 'answer-first'));
  assert.equal(v.host.querySelector('[data-confirm]'), null, 'the confirm closes');
  assert.ok(act(v, 'back'), 'the waiting guest’s chat is open');
  assert.equal(posts().length, 0, 'nothing was stopped');
});

test('m3: several waiting say the count; nobody waiting keeps the plain confirm', async () => {
  stub();
  const many = guideRaw('cap');
  const n = many.desk.guests.filter((x) => x.waiting).length;
  assert.ok(n > 1);
  const v = await guideCard(many);
  await click(act(v, 'stop'));
  assert.match(text(v), new RegExp(`${n} guests are still waiting\\. Stop anyway\\?`));
  assert.equal(act(v, 'answer-first').textContent, 'Answer them first');
  v.done();
  const calm = guideRaw('live');
  calm.desk.guests = calm.desk.guests.map((x) => ({ ...x, waiting: false, handoff_reason: null }));
  calm.desk.counts = { ...calm.desk.counts, waiting: 0 };
  const w = await guideCard(calm);
  await click(act(w, 'stop'));
  const box = w.host.querySelector('[data-confirm="stop"]');
  assert.equal(box.getAttribute('data-waiting'), null);
  assert.equal(box.getAttribute('aria-label'), 'Stop the desk?');
  assert.equal(act(w, 'confirm-stop').textContent, 'Stop the desk');
  assert.equal(act(w, 'keep').textContent, 'Keep it open');
});

test('live guide card keeps a compact QR and the one link; no PIN anywhere', async () => {
  stub();
  const b = guideRaw('live', { join: { mode: 'anywhere', anywhere_kind: 'funnel', anywhere_url: 'https://frontdesk.example.net/p/abc123', wifi_url: null } });
  const v = await guideCard(b);
  const j = v.host.querySelector('[data-join="compact"]');
  assert.ok(j, 'compact join box');
  assert.equal(j.getAttribute('data-join-mode'), 'anywhere');
  assert.match(j.textContent, /The same link for every guest/);
  assert.ok(j.querySelector('svg, img, canvas'), 'a QR is drawn');
  const row = j.querySelector('[data-join-link]');
  assert.ok(row && row.className.includes('col-span-2') && row.className.includes('sm:col-span-1'), 'the link has its own full-width row on phone');
  assert.ok(j.textContent.includes('https://frontdesk.example.net/p/abc123') || j.querySelector('input')?.value === 'https://frontdesk.example.net/p/abc123', 'the link is shown');
  assert.ok(!/\bpin\b/i.test(text(v)), 'no PIN word on the card');
});

test('a live desk with no reachable address says why, and Share link / Try as a guest say the same', async () => {
  stub();
  const b = guideRaw('live', { join: { mode: 'none', anywhere_kind: null, anywhere_url: null, wifi_url: null, wifi_note: 'this_computer_only' } });
  const v = await guideCard(b);
  const n = v.host.querySelector('[data-join="none"]');
  assert.ok(n);
  assert.match(n.textContent, /Phones can’t reach this desk yet/);
  assert.match(n.textContent, /https address\. Then the link and QR show here\./);
  assert.ok(n.parentElement.className.includes('border-amber-300/35'), 'a warn Callout, not small grey text');
  assert.ok(n.querySelector('svg.text-warn'), 'WifiOff in warn');
  assert.equal(v.host.querySelector('[data-join="compact"]'), null);
  const later = [...v.host.querySelectorAll('[aria-disabled="true"]')].map((e) => e.getAttribute('title'));
  assert.ok(later.length >= 1);
  for (const t of later) assert.equal(t, m.DESK_LATER.noAddress, 'not "the link exists once the desk starts" on a started desk');
});

test('booking card: a guest who asked for a listed time is counted once, not again as a hand-off', async () => {
  stub();
  const b = raw('requests');
  const asking = new Set(b.desk.slots.flatMap((x) => x.requested.map((r) => r.guest)));
  assert.ok(asking.size > 0);
  b.desk.guests = b.desk.guests.map((x) => (asking.has(x.id) ? { ...x, waiting: true, handoff_reason: 'Asked for a time' } : x));
  b.desk.counts = { ...b.desk.counts, waiting: asking.size };
  const v = await mount(h(m.DeskCard, { block: m.validPocket(b), ask: () => {}, openGuest: null }));
  const rows = [...v.host.querySelectorAll('[data-chip="needs-you"]')];
  assert.equal(rows.length, 0, 'no Needs-you guest rows for guests whose request already has its Allow card');
  assert.ok(v.host.querySelector('[data-booking-allow="open"]'), 'their Allow card is there');
  assert.ok(v.host.querySelector('[data-join]'), 'the booking card has the join line too');
});

test('booking chip counts a requesting guest once, the same number as the Requests tile', async () => {
  stub();
  const b = raw('requests');
  const asking = new Set(b.desk.slots.flatMap((x) => x.requested.map((r) => r.guest)));
  const requests = b.desk.slots.reduce((n, x) => n + x.requested.length, 0);
  assert.ok(requests > 0);
  const extra = { ...b.desk.guests[0], id: 'g-luka', name: 'Luka', waiting: true, handoff_reason: 'beard trim' };
  b.desk.guests = [...b.desk.guests.map((x) => (asking.has(x.id) ? { ...x, waiting: true, handoff_reason: 'Asked for a time' } : x)), extra];
  b.desk.counts = { ...b.desk.counts, waiting: asking.size + 1 };
  const v = await mount(h(m.DeskCard, { block: m.validPocket(b), ask: () => {}, openGuest: null }));
  const chips = [...v.host.querySelectorAll('[data-chip="live"]')].map((c) => c.textContent);
  assert.ok(chips.length >= 1);
  for (const c of chips) assert.equal(c, `Live · ${requests + 1} for you`, 'requests plus the one hand-off, not the asker again');
});

const slotAt = (start, over = {}) => ({ id: start, start, minutes: 30, seats: 1, open_seats: 1, label: null, past: false, requested: [], confirmed: [], ...over });
const ana = { guest: 'g-ana', name: 'Ana', at: '2026-12-01T09:00:00Z', by: 'guest' };

test('live week opens on the week of the next booking, not the first open time', async () => {
  const slots = [slotAt('2026-12-02T10:00'), slotAt('2026-12-03T10:00'), slotAt('2026-12-15T10:30', { confirmed: [ana], open_seats: 0 })];
  const v = await mount(h(m.LiveWeek, { slots }));
  assert.match(text(v), /Ana/);
  assert.match(text(v), /Dec 15/);
});

test('live week follows a late live read until the host moves the week', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  mounted.add({ done: () => (root.unmount(), host.remove()) });
  const early = [slotAt('2026-12-02T10:00'), slotAt('2026-12-15T10:30')];
  root.render(h(m.LiveWeek, { slots: early }));
  await settle();
  assert.ok(!/Ana/.test(host.textContent));
  root.render(h(m.LiveWeek, { slots: [early[0], slotAt('2026-12-15T10:30', { confirmed: [ana], open_seats: 0 })] }));
  await settle();
  assert.match(host.textContent, /Ana/, 'the booking that arrived later is in view');
});
