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
};

/** pa1's v1 envelope around a desk view. `me` absent: the face only (info, boot). */
export function deskEnvelope({ code, version, me, view, ended = false }) {
  return {
    api: 'pocket/1',
    version,
    viewer: me ? 'guest' : 'anyone',
    pocket: {
      code, kit: 'desk', title: face.title, state: ended ? 'ended' : 'open', opens_at: null, closes_at: null,
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
    if (seated && this.seat) return deskEnvelope({ code: this.code, version: this.version, me: this.seat.me, view: this.seat.view });
    return deskEnvelope({ code: this.code, version: this.version, view: { ...face }, ended: this.s?.ended });
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
}

/** Desk scenario names in the order the Chrome walk shoots them. */
export const deskOrder = Object.keys(deskScenarios);
