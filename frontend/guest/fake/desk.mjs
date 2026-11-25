// Fake front desk pocket for the guest page, answering in the shapes of Lin's desk API (pa13/impl.md, "Pocket API v1:
// front desk", which wins over design.md) inside pa1's v1 envelope. Codes starting with `desk-` pick a scenario
// (server.mjs routes them here). `DeskBox` keeps a little state so a walk can join, say, get a reply and rejoin.
// Names, the place and the messages are sample data from the pa13 mockups; the app itself never knows them.

const ago = (min) => new Date(Date.now() - min * 60000).toISOString();
const inMin = (min) => new Date(Date.now() + min * 60000).toISOString();
const day = (d) => new Date(Date.now() + d * 86400000).toISOString().slice(0, 10);

const face = { kit: 'desk', title: 'Sea-view flat', place: 'Piran, Slovenia', host_name: 'Marta', bot: { name: 'Bell', sprite: 'desk' }, keep_days: 7 };
const starters = ['What is the Wi-Fi password?', 'When is check-out?', 'Where can I park?'];
const limits = (left = 20, next_at = null) => ({ per_hour: 20, left_this_hour: left, next_at, max_chars: 1000 });
const me0 = { id: 'g1', name: 'Ana', avatar: 'visor' };

const G = (id, text, min, client_id) => ({ id, from: 'guest', text, at: ago(min), ...(client_id ? { client_id } : {}) });
const B = (id, text, min) => ({ id, from: 'bot', text, at: ago(min) });
const H = (id, text, min) => ({ id, from: 'host', text, at: ago(min) });
const N = (id, note, min) => ({ id, from: 'note', text: '', note, at: ago(min) });

const wifi = [G('m1', 'What is the Wi-Fi password?', 9), B('m2', 'The Wi-Fi is "SeaView-5G" and the password is on the card by the door: piran2026.', 8)];
const late = [...wifi, G('m3', 'Can we check out at 2 pm on Sunday?', 6), B('m4', "I can't promise a late check-out. I've asked Marta, and she'll answer here.", 5), N('m5', 'handoff', 5)];

// pa13b: the studio's week. `start` is the box's wall-clock time ("YYYY-MM-DDTHH:MM"), days counted from today (UTC).
const at = (d, hour, min = 0) => `${day(d)}T${String(hour).padStart(2, '0')}:${String(min).padStart(2, '0')}`;
const HOURS = [9, 10, 11, 14, 15, 16];
const TAKEN = new Set(['d1-14', 'd2-9', 'd2-16', 'd3-9', 'd3-10', 'd3-11', 'd3-14', 'd3-15', 'd3-16', 'd4-11', 'd5-9', 'd5-10', 'd5-11', 'd5-14']);
const week = (take) =>
  [1, 2, 3, 4, 5].flatMap((d) => HOURS.map((hr) => ({ id: `d${d}-${hr}`, start: at(d, hr), minutes: 60, label: null, open: !TAKEN.has(`d${d}-${hr}`) && `d${d}-${hr}` !== take })));
const bface = { kit: 'desk', mode: 'booking', title: 'Haircuts', place: 'Ljubljana', host_name: 'Nika', bot: { name: 'Bell', sprite: 'desk' }, keep_days: 7, what: 'your cut' };
const bstarters = ['Prices', 'How to find you'];
const BN = (id, note, min, slot) => ({ id, from: 'note', text: '', note, at: ago(min), ...(slot ? { slot, start: at(1, Number(slot.split('-')[1])) } : {}) });
const requested = [BN('b1', 'booking_requested', 3, 'd1-10')];
const confirmed = [...requested, BN('b2', 'booking_confirmed', 1, 'd1-10')];
const declined = [...requested, BN('b2', 'booking_declined', 1, 'd1-10'), H('b3', "Sorry, I'm at a training that morning. Thursday works!", 1)];
const offers = [
  G('b1', 'Do you have anything Thursday morning?', 4),
  { ...B('b2', 'Thursday is open at 10 AM and 11 AM. Tap one to ask for it.', 4), offers: ['d2-10', 'd2-11', 'd3-9'] },
];

/** A booking desk seat (pa13b guest view: slots, my_booking, limits.note_max_chars). */
const bseat = (view = {}, { take, deleteOn } = {}) => ({
  me: { ...me0, name: 'Emma' },
  view: {
    ...bface,
    me: { id: me0.id, name: 'Emma', creature: me0.avatar, delete_on: deleteOn ?? (view.messages?.length ? day(7) : null) },
    starters: bstarters,
    messages: [],
    waiting_for_host: false,
    bot_typing: false,
    limits: { ...limits(), note_max_chars: 200 },
    slots: week(take),
    my_booking: null,
    ...view,
  },
});

/** A seat's view: `me` is pa1's seat, the rest is the kit's guest view. */
const seat = (view = {}) => ({
  me: me0,
  view: { ...face, me: { id: me0.id, name: me0.name, creature: me0.avatar, delete_on: view.messages?.length ? day(7) : null }, starters, messages: [], waiting_for_host: false, bot_typing: false, limits: limits(), ...view },
});

export const deskScenarios = {
  // G1 join (and G11 from "I've joined before"); join gives G2, rejoin gives G5.
  'desk-join': { state: 401, join: seat(), rejoin: seat({ messages: [...late, H('m6', 'Hi Ana, 2 pm is fine. Leave the key in the box.', 1)] }) },
  // G1 refused: the desk already has its most guests.
  'desk-full': { state: 401, joinError: { status: 409, error: 'guest_max', message: 'This desk is full.', max_guests: 12 } },
  // G2 first time: empty thread, starters; say gives G3 (Bell typing).
  'desk-first': { state: seat() },
  // G3 the bot is typing.
  'desk-chat': { state: seat({ messages: [G('m1', 'What is the Wi-Fi password?', 1)], bot_typing: true }) },
  // G4 hand-off: Bell asked the host, waiting.
  'desk-handoff': { state: seat({ messages: late, waiting_for_host: true }) },
  // G5 the host answered.
  'desk-host': { state: seat({ messages: [...late, H('m6', 'Hi Ana, 2 pm is fine. Leave the key in the box.', 1)] }) },
  // G6 slow down: every say is the 21st this hour.
  'desk-slow': { state: seat({ messages: wifi, limits: limits(1) }), sayError: { status: 429, error: 'slow_down', message: 'Slow down.', retry_at: inMin(14) } },
  // G7 the daily cap: kept for the host, no bot.
  'desk-cap': { state: seat({ messages: [...wifi, G('m3', 'Is there a beach towel?', 2), N('m4', 'daily_cap', 2)], waiting_for_host: true }) },
  // G8 too long: the walk types past 1,000 characters.
  'desk-long': { state: seat({ messages: wifi }) },
  // G9 offline: actions give 503, so the bubble fails and waits for Retry.
  'desk-offline': { state: seat({ messages: wifi }), sayError: { status: 503, error: 'unavailable', message: 'Away.' } },
  // G10 the host is away: the hand-off had no reply.
  'desk-away': { state: seat({ messages: [...late, N('m6', 'host_away', 1)], waiting_for_host: true }) },
  // G11 rejoin with a name and PIN.
  'desk-rejoin': { state: 401, rejoin: seat({ messages: wifi }) },
  // G12 ended.
  'desk-ended': { state: 401, ended: true },

  // ---- pa13b Booking Desk (pa13b-design.md K1-K11, Vega's "For Vega" field names). Sample studio and times. ----
  // K1 join with "See open times", then K2 and the K3 sheet; a request gives K4.
  'desk-book-pick': { state: 401, booking: true, join: bseat() },
  // K4 requested: the strip with Cancel, the note, the wait pill.
  'desk-book-requested': { state: bseat({ messages: requested, waiting_for_host: true, my_booking: { slot: 'd1-10', start: at(1, 10), minutes: 60, state: 'requested', at: ago(3) } }, { take: 'd1-10' }), booking: true },
  // K5 confirmed by the host: the ok strip, no Cancel.
  'desk-book-confirmed': { state: bseat({ messages: confirmed, my_booking: { slot: 'd1-10', start: at(1, 10), minutes: 60, state: 'confirmed', at: ago(1) } }, { take: 'd1-10' }), booking: true },
  // K6 declined: the note, the host's own line, then Pick another time.
  'desk-book-declined': { state: bseat({ messages: declined }), booking: true },
  // K7 cancel: a request the walk cancels.
  'desk-book-cancel': { state: bseat({ messages: requested, waiting_for_host: true, my_booking: { slot: 'd1-10', start: at(1, 10), minutes: 60, state: 'requested', at: ago(3) } }, { take: 'd1-10' }), booking: true },
  // K8 nothing open.
  'desk-book-none': { state: bseat({ slots: [] }), booking: true },
  // K10 just taken: the first request is refused with fresh slots, without that time.
  'desk-book-taken': { state: bseat(), booking: true, takenOnce: true },
  // A time that started while the sheet was open: Vega's 410 slot_past (pa13b-impl.md), which must not end the page.
  'desk-book-past': { state: bseat(), booking: true, pastOnce: true },
  // K11 closed, still booked.
  'desk-book-ended-booked': {
    state: bseat({ messages: confirmed, my_booking: { slot: 'd1-10', start: at(1, 10), minutes: 60, state: 'confirmed', at: ago(60) } }, { take: 'd1-10', deleteOn: day(2) }),
    booking: true,
    endedSeat: true,
  },
  // K9 Bell offers two open times as chips (a third id is no longer open, so its chip is dropped).
  'desk-book-offers': { state: bseat({ messages: offers }), booking: true },
};

/** pa1's v1 envelope around a desk view. `me` absent: the face only (info, boot). */
export function deskEnvelope({ code, version, me, view, ended = false }) {
  return {
    api: 'pocket/1',
    version,
    viewer: me ? 'guest' : 'anyone',
    pocket: {
      code, kit: 'desk', title: view?.title ?? face.title, state: ended ? 'ended' : 'open', opens_at: null, closes_at: null,
      ended_at: ended ? ago(60) : null, expires_at: ended ? new Date(Date.now() + 7 * 86400000).toISOString() : null,
      players: null, max_players: null, needs_pin: true, retention: { kind: 'after_last_message', days: 7 },
    },
    ...(me ? { me: { id: me.id, name: me.name, avatar: me.avatar, team: null, notices: [] } } : {}),
    view,
  };
}

const copy = (x) => JSON.parse(JSON.stringify(x));

export class DeskBox {
  constructor(name, { code = name } = {}) {
    this.s = deskScenarios[name];
    this.code = code;
    this.seat = this.s && typeof this.s.state === 'object' ? copy(this.s.state) : null;
    this.version = 1;
    this.n = 100;
  }
  env(seated = true) {
    if (seated && this.seat) return deskEnvelope({ code: this.code, version: this.version, me: this.seat.me, view: this.seat.view, ended: this.s?.endedSeat });
    return deskEnvelope({ code: this.code, version: this.version, view: { ...(this.s?.booking ? bface : face) }, ended: this.s?.ended || this.s?.endedSeat });
  }
  /** A push from server.mjs: a function edits the seat's view (a reply arrives, typing stops). */
  set(fn) {
    if (typeof fn === 'function' && this.seat) fn(this.seat.view);
    this.version++;
  }
  handle(path, body = {}, { seated = true } = {}) {
    const [what, query] = path.split('?');
    const err = (status, error, extra = {}) => ({ status, body: { error, message: extra.message ?? 'Fake desk box.', ...extra } });
    if (!this.s) return err(404, 'not_found');
    if (this.s.ended && what !== 'info') return err(410, 'ended');
    const mine = seated && this.seat;
    if (what === 'state') {
      if (!mine) return err(401, 'signed_out');
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env() };
    }
    if (what === 'info') return { status: 200, body: this.env(false) };
    if (what === 'join') {
      if (this.s.joinError) return err(this.s.joinError.status, this.s.joinError.error, this.s.joinError);
      if (!/^\d{4}$/.test(body.pin ?? '')) return err(400, 'pin_required', { message: 'Pick a 4-digit PIN.' });
      if (!this.s.join) return err(409, 'name_taken');
      this.seat = copy(this.s.join);
      const name = body.name ?? this.seat.me.name;
      this.seat.me = { ...this.seat.me, name, avatar: body.avatar ?? this.seat.me.avatar };
      this.seat.view.me = { ...this.seat.view.me, name };
      this.version++;
      return { status: 201, body: this.env() };
    }
    if (what === 'rejoin') {
      if (!this.s.rejoin) return err(401, 'wrong_pin', { message: "That name and PIN don't match.", tries_left: 4 });
      if ((body.name ?? '').trim().toLowerCase() !== me0.name.toLowerCase()) return err(404, 'no_such_player');
      this.seat = copy(this.s.rejoin);
      this.version++;
      return { status: 200, body: this.env() };
    }
    if (what === 'action') {
      if (!mine) return err(401, 'signed_out');
      if (this.s.endedSeat) return err(410, 'ended', { my_booking: this.seat.view.my_booking });
      if (this.s.booking && (body.name === 'request' || body.name === 'cancel')) return this.book(body.name, body.body ?? {}, err);
      if (body.name !== 'say') return err(400, 'bad_action', { message: 'Unknown action.' });
      if (this.s.sayError) return err(this.s.sayError.status, this.s.sayError.error, this.s.sayError);
      const b = body.body ?? {};
      const text = typeof b.text === 'string' ? b.text.trim() : '';
      if (!text) return err(400, 'empty');
      if ([...text].length > 1000) return err(413, 'too_long');
      const v = this.seat.view;
      v.messages = [...v.messages, { id: `m${++this.n}`, from: 'guest', text, at: new Date().toISOString(), ...(b.client_id ? { client_id: String(b.client_id) } : {}) }].slice(-80);
      v.bot_typing = !v.waiting_for_host;
      v.me = { ...v.me, delete_on: day(7) };
      v.limits = { ...v.limits, left_this_hour: Math.max(0, v.limits.left_this_hour - 1) };
      this.version++;
      return { status: 200, body: this.env() };
    }
    return err(404, 'not_found');
  }
  /** pa13b request / cancel, in the design's shapes (409 slot_taken carries fresh slots, 409 has_request my_booking). */
  book(name, b, err) {
    const v = this.seat.view;
    const slot = v.slots.find((s) => s.id === b.slot);
    const now = new Date().toISOString();
    if (name === 'request') {
      if (!slot) return err(404, 'no_such_slot', { message: "That time isn't on this page." });
      if (typeof b.note === 'string' && [...b.note].length > (v.limits.note_max_chars ?? 200)) return err(413, 'too_long');
      if (v.my_booking) return err(409, 'has_request', { slot: v.my_booking.slot, my_booking: v.my_booking });
      if (this.s.pastOnce && !this.pastDone) {
        this.pastDone = true;
        return err(410, 'slot_past', { message: 'That time has passed. Pick another time.' });
      }
      if (this.s.takenOnce && !this.tookOnce) {
        this.tookOnce = true;
        slot.open = false;
        this.version++;
        return err(409, 'slot_taken', { message: 'Someone else has that time.', slots: v.slots });
      }
      if (!slot.open) return err(409, 'slot_taken', { slots: v.slots });
      slot.open = false;
      v.my_booking = { slot: slot.id, start: slot.start, minutes: slot.minutes, state: 'requested', at: now };
      v.messages = [...v.messages, { id: `m${++this.n}`, from: 'note', text: '', note: 'booking_requested', slot: slot.id, start: slot.start, at: now }];
      v.waiting_for_host = true;
      v.me = { ...v.me, delete_on: day(7) };
    } else {
      const mb = v.my_booking;
      if (!mb || mb.slot !== b.slot) return err(409, 'no_request');
      if (mb.state === 'confirmed') return err(409, 'is_confirmed');
      v.my_booking = null;
      if (slot) slot.open = true;
      v.messages = [...v.messages, { id: `m${++this.n}`, from: 'note', text: '', note: 'booking_cancelled', slot: mb.slot, start: mb.start, at: now }];
      v.waiting_for_host = false;
    }
    this.version++;
    return { status: 200, body: this.env() };
  }
}

/** Desk scenario names in the order the Chrome walk shoots them. */
export const deskOrder = Object.keys(deskScenarios);
