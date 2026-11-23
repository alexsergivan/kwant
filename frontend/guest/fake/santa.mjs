// Fake Secret Santa pocket for the guest page, answering in the shapes of Lin's santa API (pa10a/server-impl.md,
// "The santa API") inside pa1's v1 envelope. Codes starting with `santa-` pick a scenario (server.mjs routes them here).
// `SantaBox` keeps a little state so a walk can join, open the match, ask, answer and tick the gift.
// Names, wishes and questions are sample data from the pa10a mockups.

const P = (id, name, avatar) => ({ id, name, avatar });
export const people = [P('p1', 'Mia', 'visor'), P('p2', 'Sam', 'antenna'), P('p3', 'Jonas', 'spikes'), P('p4', 'Priya', 'beret'), P('p5', 'Leo', 'goggles')];
const [mia, sam, jonas, priya, leo] = people;

const facts = {
  phase: 'collecting',
  gift_on: '2026-12-18',
  budget: '€25',
  host_name: 'Dana',
  draw_at: '2026-12-01T18:00:00Z',
  reveal: 'scheduled',
  reveal_at: '2026-12-18T18:00:00Z',
  joined: 5,
  expected: 8,
};
const drawn = { ...facts, phase: 'drawn', joined: 8 };
const revealed = { ...facts, phase: 'revealed', joined: 5, reveal: 'shown' };

const asks0 = { sent: [], received: [], left_today: 10, max_chars: 200, open: false };

/** A seat's view (flat, for reading): `me` is pa1's seat, the rest is the kit's guest view. */
const seat = (view, me = mia) => ({ me: { ...me, notices: [] }, view: { people, my_wish: null, in_draw: true, can_open: false, opened: false, match: null, gift_ready: false, notices: [], asks: asks0, my_santa: null, reveal_loop: null, ...view } });

export const secretMatch = { ...jonas, wish: 'A good paperback, any thriller\nWarm socks, size 43\nNothing with nuts, please' };

const askedAt = '2026-12-04T08:12:00Z';
const matchAsks = {
  sent: [{ id: 'a1', q: 'Favourite colour?', a: 'Green, like a forest.', at: '2026-12-03T19:40:00Z', answered_at: askedAt, new_answer: true }],
  received: [{ id: 'r1', q: 'Do you like dark chocolate?', a: null, at: askedAt, answered_at: null, new: true }],
  left_today: 9,
  max_chars: 200,
  open: true,
};

export const santaScenarios = {
  // G1 join (and G3/G4 from "I've joined before"); join gives G2, rejoin gives the opened match.
  'santa-join': { state: 401, info: facts, join: seat({ ...facts, my_wish: null }), rejoin: seat({ ...drawn, opened: true, match: secretMatch, asks: matchAsks }) },
  // G4: every rejoin is a wrong PIN with 3 tries left.
  'santa-wrongpin': { state: 401, info: facts, rejoinError: { status: 401, error: 'wrong_pin', message: "That name and PIN don't match.", tries_left: 3 } },
  // G2 waiting for the draw.
  'santa-waiting': { state: seat({ ...facts, my_wish: 'Warm socks' }) },
  // G5 unwrap; open_match gives G6 with G7 asks.
  'santa-unwrap': { state: seat({ ...drawn, can_open: true }), open: { match: secretMatch, asks: matchAsks } },
  // G5 after a late joiner took the old match (match_changed): closed until "Open it again".
  'santa-changed': { state: seat({ ...drawn, can_open: true, opened: true, notices: [{ kind: 'match_changed', at: askedAt }] }), open: { match: { ...priya, wish: 'Board games' }, asks: { ...asks0, open: true } } },
  // G8 the reveal, with the server's loop.
  'santa-reveal': {
    state: seat({
      ...revealed,
      opened: true,
      match: secretMatch,
      my_santa: priya,
      reveal_loop: [
        { giver: mia, receiver: jonas },
        { giver: jonas, receiver: sam },
        { giver: sam, receiver: leo },
        { giver: leo, receiver: priya },
        { giver: priya, receiver: mia },
      ],
    }),
  },
  // G9 the host's computer is away: state and actions give 503 (the page was served with the public facts).
  'santa-away': { state: 503, info: facts },
  // G10 names already drawn: no seat; join is refused with the server's reason.
  'santa-late': { state: 401, info: drawn, joinError: { status: 409, error: 'not_now', message: 'The names are already drawn. Ask Dana to let you in.' } },
  // G11 ended.
  'santa-ended': { state: 401, info: revealed, ended: true },
};

/** pa1's v1 envelope around a santa view. `me` absent: the public facts only (info, boot). */
export function santaEnvelope(s, { code, version, me, view, ended = false }) {
  return {
    api: 'pocket/1',
    version,
    viewer: me ? 'guest' : 'anyone',
    pocket: {
      code, kit: 'santa', title: 'Office Secret Santa', state: ended ? 'ended' : 'open', opens_at: null, closes_at: null,
      ended_at: ended ? '2026-12-18T18:00:00Z' : null, expires_at: '2026-12-25T18:00:00Z',
      players: view.joined ?? 5, max_players: 60, needs_pin: true, retention: { kind: 'after_date', days: 7 },
    },
    ...(me ? { me: { id: me.id, name: me.name, avatar: me.avatar, team: null, notices: me.notices ?? [] } } : {}),
    view,
  };
}

const factKeys = Object.keys(facts);
const onlyFacts = (v) => Object.fromEntries(Object.entries(v).filter(([k]) => factKeys.includes(k)));
const copy = (x) => JSON.parse(JSON.stringify(x));

export class SantaBox {
  constructor(name, { code = name } = {}) {
    this.s = santaScenarios[name];
    this.code = code;
    this.seat = this.s && typeof this.s.state === 'object' ? copy(this.s.state) : null;
    this.version = 1;
    this.n = 0;
  }
  env(seated = true) {
    if (seated && this.seat) return santaEnvelope(this.s, { code: this.code, version: this.version, me: this.seat.me, view: this.seat.view });
    return santaEnvelope(this.s, { code: this.code, version: this.version, view: onlyFacts(this.s.info ?? this.seat?.view ?? facts), ended: this.s.ended });
  }
  bump() {
    this.version++;
  }
  handle(path, body = {}, { seated = true } = {}) {
    const [what, query] = path.split('?');
    const err = (status, error, extra = {}) => ({ status, body: { error, message: extra.message ?? 'Fake santa box.', ...extra } });
    if (!this.s) return err(404, 'not_found');
    const mine = seated && this.seat;
    if (what === 'state') {
      if (!mine) return typeof this.s.state === 'number' && this.s.state !== 401 ? err(this.s.state, 'unavailable') : err(401, 'signed_out');
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env() };
    }
    if (what === 'info') return { status: 200, body: this.env(false) };
    if (what === 'join') {
      if (this.s.joinError) return err(this.s.joinError.status, this.s.joinError.error, this.s.joinError);
      if (!/^\d{4}$/.test(body.pin ?? '')) return err(400, 'pin_required', { message: 'Pick a 4-digit PIN.' });
      if (!this.s.join) return err(410, 'ended');
      this.seat = copy(this.s.join);
      this.seat.me = { ...this.seat.me, name: body.name ?? this.seat.me.name, avatar: body.avatar ?? this.seat.me.avatar };
      // The joiner is a new person (p6): everyone already listed stays, and joined counts exactly the people drawn.
      this.seat.me.id = 'p6';
      this.seat.view.people = [...this.seat.view.people.filter((p) => p.id !== 'p6'), { id: 'p6', name: this.seat.me.name, avatar: this.seat.me.avatar }];
      this.seat.view.joined = this.seat.view.people.length;
      this.bump();
      return { status: 201, body: this.env() };
    }
    if (what === 'rejoin') {
      if (this.s.rejoinError) return err(this.s.rejoinError.status, this.s.rejoinError.error, this.s.rejoinError);
      if (!this.s.rejoin) return err(401, 'wrong_pin', { message: "That name and PIN don't match.", tries_left: 4 });
      this.seat = copy(this.s.rejoin);
      this.bump();
      return { status: 200, body: this.env() };
    }
    if (what === 'notices/seen') {
      if (!mine) return err(401, 'signed_out');
      this.seat.me.notices = [];
      return { status: 204, body: null };
    }
    if (what === 'action') {
      if (!mine) return typeof this.s.state === 'number' && this.s.state !== 401 ? err(this.s.state, 'unavailable') : err(401, 'signed_out');
      const v = this.seat.view;
      const b = body.body ?? {};
      const notNow = (message) => err(409, 'not_now', { message });
      switch (body.name) {
        case 'open_match':
          if (v.phase === 'collecting' || !v.in_draw) return notNow("The names aren't drawn yet.");
          if (!v.match) Object.assign(v, copy(this.s.open ?? {}));
          Object.assign(v, { opened: true, can_open: false, notices: [] });
          v.asks = { ...v.asks, open: v.phase === 'drawn' };
          break;
        case 'set_wish':
          if (typeof b.text !== 'string' || [...b.text].length > 300) return err(400, 'bad_action', { message: 'Wish lists are up to 300 characters.' });
          v.my_wish = b.text.trim() || null;
          break;
        case 'gift_ready':
          if (!v.match) return notNow('Open your match first.');
          v.gift_ready = b.ready !== false;
          break;
        case 'ask':
          if (!v.asks.open) return notNow("You can't ask right now.");
          if ([...(b.text ?? '')].length > 200) return err(400, 'bad_action', { message: 'Questions are up to 200 characters.' });
          if (!v.asks.left_today) return notNow("That's 10 for today. You can ask again tomorrow.");
          v.asks = { ...v.asks, left_today: v.asks.left_today - 1, sent: [...v.asks.sent, { id: `s${++this.n}`, q: b.text, a: null, at: new Date().toISOString(), answered_at: null, new_answer: false }] };
          break;
        case 'answer': {
          const q = v.asks.received.find((x) => x.id === b.id);
          if (!q) return err(400, 'bad_action', { message: "That isn't your Santa's question." });
          if (q.a) return notNow('Already answered.');
          q.a = b.text;
          q.answered_at = new Date().toISOString();
          v.asks.left_today = Math.max(0, v.asks.left_today - 1);
          break;
        }
        case 'seen_asks':
          v.asks = { ...v.asks, sent: v.asks.sent.map((x) => ({ ...x, new_answer: false })), received: v.asks.received.map((x) => ({ ...x, new: false })) };
          break;
        default:
          return err(400, 'bad_action', { message: 'Unknown action.' });
      }
      this.bump();
      return { status: 200, body: this.env() };
    }
    return err(404, 'not_found');
  }
}

/** Santa scenario names in the order the Chrome walk shoots them. */
export const santaOrder = Object.keys(santaScenarios);
