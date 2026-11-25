// Fake gather pocket (pa10b: Sign-up, RSVP and Vote) for the guest page, in the shapes of Lin's gather.rs as of
// Nov 24 (server-impl.md "Pocket API v1: gather" is not written yet; names-so-far, see pa10b/guest-impl.md), inside pa1's
// v1 envelope. Codes starting with `gather-` pick a scenario (server.mjs routes them here). `GatherBox` keeps a little
// state so a walk can answer, take a slot, let it go and vote. Names, slots and options are sample data from
// pa10b/guest-390.html; every count below is typed in, as the real box would send it.

const P = (id, name, avatar) => ({ id, name, avatar });
const mia = P('p1', 'Mia', 'visor');
const sam = P('p2', 'Sam', 'antenna');
const jonas = P('p3', 'Jonas', 'spikes');
const priya = P('p4', 'Priya', 'beret');
const leo = P('p5', 'Leo', 'goggles');
const ana = P('p6', 'Ana', 'ears');
const tom = P('p7', 'Tom', 'horns');

const FOOD = ['vegetarian', 'vegan', 'gluten_free', 'dairy_free', 'nut_allergy', 'halal', 'kosher'];

const rsvpFacts = {
  face: 'rsvp',
  phase: 'open',
  when: 'Sat, Dec 12, 7:30 PM',
  place: 'Mia’s place',
  host_name: 'Dana',
  price: '€15',
  closes_at: '2026-12-10T20:00:00Z',
  event_at: '2026-12-12T19:30:00Z',
  fields: ['name', 'answer', 'plus_ones', 'food_needs'],
  show_names: true,
  joined: 14,
  members: 14,
  food_choices: FOOD,
  max_plus_ones: 5,
  note_max: 40,
};
const rsvpPeople = [
  { ...sam, answer: 'yes' },
  { ...jonas, answer: 'yes' },
  { ...priya, answer: 'yes' },
  { ...leo, answer: 'yes' },
  { ...ana, answer: 'maybe' },
  { ...tom, answer: 'no' },
];
const rsvpCounts = { yes: 9, maybe: 3, no: 2, coming: 12 };

const slot = (n, name, cap, people, waiting_people = []) => ({
  slot: n,
  name,
  cap,
  filled: people.length,
  full: people.length >= cap,
  waiting: waiting_people.length,
  people,
  waiting_people,
});
const signupSlots = () => [
  slot(0, 'Mains', 4, [{ ...sam, note: 'lasagne' }, { ...jonas, note: null }]),
  slot(1, 'Salads', 3, [{ ...priya, note: 'greek salad' }]),
  slot(2, 'Desserts', 2, [{ ...leo, note: 'brownies' }, { ...ana, note: null }]),
  slot(3, 'Drinks', 4, []),
];
const signupFacts = {
  face: 'signup',
  phase: 'open',
  when: 'Sat, Dec 12',
  place: 'Club house',
  host_name: 'Dana',
  closes_at: '2026-12-10T20:00:00Z',
  event_at: '2026-12-12T18:00:00Z',
  fields: ['name', 'slot', 'slot_note'],
  show_names: true,
  joined: 6,
  members: 6,
  waiting_list: true,
  note_max: 40,
};

const options = [
  { option: 0, name: 'The Old Mill' },
  { option: 1, name: 'Casa Verde' },
  { option: 2, name: 'Sushi Bar Ko' },
  { option: 3, name: 'Pizza Nonna' },
];
const voteFacts = (kind, extra = {}) => ({
  face: 'vote',
  phase: 'open',
  host_name: 'Dana',
  closes_at: '2026-12-03T20:00:00Z',
  fields: ['name', 'ballot'],
  show_names: false,
  joined: 11,
  members: 11,
  show_live_count: true,
  ballot: { kind, options, in_play: [0, 1, 2, 3] },
  voted: 11,
  ...extra,
});

const result = {
  outcome: 'winner',
  winner: { option: 0, name: 'The Old Mill', votes: 7 },
  tied: [],
  ballots: 12,
  final_round_active: 11,
  rounds: [
    { round: 1, counts: [{ option: 0, name: 'The Old Mill', votes: 5 }, { option: 1, name: 'Casa Verde', votes: 4 }, { option: 2, name: 'Sushi Bar Ko', votes: 2 }, { option: 3, name: 'Pizza Nonna', votes: 1 }], exhausted: 0, dropped: [{ option: 3, name: 'Pizza Nonna' }] },
    { round: 2, counts: [{ option: 0, name: 'The Old Mill', votes: 5 }, { option: 1, name: 'Casa Verde', votes: 4 }, { option: 2, name: 'Sushi Bar Ko', votes: 3 }], exhausted: 0, dropped: [{ option: 2, name: 'Sushi Bar Ko' }] },
    { round: 3, counts: [{ option: 0, name: 'The Old Mill', votes: 7 }, { option: 1, name: 'Casa Verde', votes: 4 }], exhausted: 1, dropped: [] },
  ],
};

const rsvpView = (me) => ({ ...rsvpFacts, counts: rsvpCounts, people: rsvpPeople, me });
const signupView = (me, slots = signupSlots()) => ({ ...signupFacts, slots, me });
const seat = (view, me = mia) => ({ me: { ...me, notices: [] }, view });
const noAnswer = { answer: null, plus_ones: 0, food: [], food_other: null, answered_at: null, paid: null };

/** Sign-up with Mia waiting 1st on Desserts. */
const waitingSlots = () => {
  const s = signupSlots();
  s[2] = slot(2, 'Desserts', 2, [{ ...leo, note: 'brownies' }, { ...ana, note: null }], [mia, tom]);
  return s;
};
/** Sign-up after Ana let Desserts go and Mia moved up; Tom still waits. */
const movedSlots = () => {
  const s = signupSlots();
  s[2] = slot(2, 'Desserts', 2, [{ ...leo, note: 'brownies' }, { ...mia, note: 'tiramisu' }], [tom]);
  return s;
};

export const gatherScenarios = {
  // G1 RSVP join; answering gives G2. "I've answered before" opens G10 and the rejoin gives G2 with paid.
  'gather-rsvp': { info: rsvpView(undefined), join: seat(rsvpView(noAnswer), P('p8', 'Mia', 'visor')), rejoin: seat(rsvpView({ answer: 'yes', plus_ones: 1, food: ['vegetarian'], food_other: null, answered_at: '2026-12-01T10:00:00Z', paid: { at: '2026-12-04T09:00:00Z' } })) },
  // G2 You're in, ticked as paid by the host.
  'gather-rsvp-in': { state: seat(rsvpView({ answer: 'yes', plus_ones: 1, food: ['vegetarian'], food_other: null, answered_at: '2026-12-01T10:00:00Z', paid: { at: '2026-12-04T09:00:00Z' } })) },
  // G2 without names: the counts only, and no paid line.
  'gather-rsvp-counts': { state: seat({ ...rsvpView({ answer: 'maybe', plus_ones: 0, food: [], food_other: null, answered_at: '2026-12-01T10:00:00Z', paid: null }), show_names: false, people: null }) },
  // G3 Sign-up pick: Desserts is full with a waiting list.
  'gather-signup': { info: signupView(undefined), join: seat(signupView({ slot: null, note: null, waiting: null, moved_up: null }), P('p8', 'Mia', 'visor')) },
  // Sign-up, in a slot.
  'gather-signup-in': { state: seat(signupView({ slot: 3, note: 'lemonade', waiting: null, moved_up: null }, (() => { const s = signupSlots(); s[3] = slot(3, 'Drinks', 4, [{ ...mia, note: 'lemonade' }]); return s; })())) },
  // G4 waiting list, 1st in line for Desserts.
  'gather-waiting': { state: seat(signupView({ slot: null, note: null, waiting: { slot: 2, position: 1 }, moved_up: null }, waitingSlots())) },
  // G5 moved up from the waiting list (ink New, then `seen`).
  'gather-moved': { state: seat(signupView({ slot: 2, note: 'tiramisu', waiting: null, moved_up: { slot: 2, at: '2026-12-05T12:00:00Z', new: true } }, movedSlots())) },
  // G6 one pick.
  'gather-vote': { info: { ...voteFacts('one_pick'), me: undefined }, join: seat({ ...voteFacts('one_pick'), me: { voted: false, ballot: null }, live: null, result: null }, P('p8', 'Mia', 'visor')) },
  // G7 ranked.
  'gather-ranked': { info: { ...voteFacts('ranked'), me: undefined }, join: seat({ ...voteFacts('ranked'), me: { voted: false, ballot: null }, live: null, result: null }, P('p8', 'Mia', 'visor')) },
  // G8 voted, with the live count of 1st choices.
  'gather-voted': {
    state: seat({
      ...voteFacts('ranked'),
      me: { voted: true, ballot: [0, 1, 2] },
      live: [{ option: 0, votes: 5 }, { option: 1, votes: 4 }, { option: 2, votes: 2 }, { option: 3, votes: 0 }],
      result: null,
    }),
  },
  // G9 closed with a winner after three rounds (the one holo).
  'gather-result': { state: seat({ ...voteFacts('ranked', { phase: 'closed', voted: 12 }), me: { voted: true, ballot: [0, 1, 2] }, live: null, result }) },
  // G9 tie, waiting on the host: plain, no holo.
  'gather-tie': { state: seat({ ...voteFacts('one_pick', { phase: 'tie' }), me: { voted: true, ballot: [1] }, live: null, result: { outcome: 'tie', winner: null, tied: [{ option: 0, name: 'The Old Mill', votes: 5 }, { option: 1, name: 'Casa Verde', votes: 5 }], ballots: 10, final_round_active: 10, rounds: [{ round: 1, counts: [{ option: 0, name: 'The Old Mill', votes: 5 }, { option: 1, name: 'Casa Verde', votes: 5 }], exhausted: 0, dropped: [] }] } }) },
  // G10 every rejoin is a wrong PIN with 3 tries left.
  'gather-wrongpin': { info: rsvpView(undefined), rejoinError: { status: 401, error: 'wrong_pin', message: "That name and PIN don't match.", tries_left: 3 } },
  // G11 RSVPs closed, no seat.
  'gather-closed': { info: { ...rsvpView(undefined), phase: 'closed' } },
  // Host asleep: state and actions give 503 (the page was served with the public facts).
  'gather-away': { state: 503, info: rsvpView(undefined) },
};

/** pa1's v1 envelope around a gather view. `me` absent: the public facts only (info, boot). */
export function gatherEnvelope({ code, version, me, view, ended = false }) {
  const title = view.face === 'vote' ? 'Where do we eat?' : view.face === 'signup' ? 'Club potluck' : 'Club winter dinner';
  return {
    api: 'pocket/1',
    version,
    viewer: me ? 'guest' : 'anyone',
    pocket: {
      code, kit: 'gather', title, state: ended ? 'ended' : 'open', opens_at: null, closes_at: view.closes_at ?? null,
      ended_at: ended ? '2026-12-13T00:00:00Z' : null, expires_at: '2026-12-19T19:30:00Z',
      players: view.joined ?? 0, max_players: 200, needs_pin: true, retention: { kind: 'after_date', days: 7 },
    },
    ...(me ? { me: { id: me.id, name: me.name, avatar: me.avatar, team: null, notices: me.notices ?? [] } } : {}),
    view,
  };
}

const copy = (x) => JSON.parse(JSON.stringify(x));
/** The public view: facts and counts, never anyone's own answer. */
const publicView = (v) => {
  const { me, live, ...rest } = copy(v);
  void me;
  void live;
  return rest;
};

export class GatherBox {
  constructor(name, { code = name } = {}) {
    this.s = gatherScenarios[name];
    this.code = code;
    this.seat = this.s && typeof this.s.state === 'object' ? copy(this.s.state) : null;
    this.version = 1;
  }
  env(seated = true) {
    if (seated && this.seat) return gatherEnvelope({ code: this.code, version: this.version, me: this.seat.me, view: this.seat.view });
    return gatherEnvelope({ code: this.code, version: this.version, view: publicView(this.s.info ?? this.seat?.view), ended: this.s.ended });
  }
  handle(path, body = {}, { seated = true } = {}) {
    const [what, query] = path.split('?');
    const err = (status, error, extra = {}) => ({ status, body: { error, message: extra.message ?? 'Fake gather box.', ...extra } });
    if (!this.s) return err(404, 'not_found');
    const mine = seated && this.seat;
    if (what === 'state') {
      if (!mine) return typeof this.s.state === 'number' ? err(this.s.state, 'unavailable') : err(401, 'signed_out');
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env() };
    }
    if (what === 'info') return { status: 200, body: this.env(false) };
    if (what === 'join') {
      if (!/^\d{4}$/.test(body.pin ?? '')) return err(400, 'pin_required', { message: 'Pick a 4-digit PIN.' });
      if (!this.s.join) return err(409, 'not_now', { message: 'Answers are closed.' });
      this.seat = copy(this.s.join);
      this.seat.me = { ...this.seat.me, name: body.name ?? this.seat.me.name, avatar: body.avatar ?? this.seat.me.avatar };
      this.version++;
      return { status: 201, body: this.env() };
    }
    if (what === 'rejoin') {
      if (this.s.rejoinError) return err(this.s.rejoinError.status, this.s.rejoinError.error, this.s.rejoinError);
      if (!this.s.rejoin) return err(401, 'wrong_pin', { message: "That name and PIN don't match.", tries_left: 4 });
      this.seat = copy(this.s.rejoin);
      this.version++;
      return { status: 200, body: this.env() };
    }
    if (what === 'notices/seen') {
      if (!mine) return err(401, 'signed_out');
      this.seat.me.notices = [];
      return { status: 204, body: null };
    }
    if (what === 'action') {
      if (!mine) return typeof this.s.state === 'number' ? err(this.s.state, 'unavailable') : err(401, 'signed_out');
      const v = this.seat.view;
      const b = body.body ?? {};
      const me = this.seat.me;
      const notNow = (message) => err(409, 'not_now', { message });
      if (v.phase !== 'open' && body.name !== 'seen') return notNow(v.face === 'vote' && v.phase === 'tie' ? "Voting has closed: it's a tie, and the host picks." : 'Answers are closed.');
      switch (body.name) {
        case 'answer': {
          if (v.face !== 'rsvp') return err(400, 'bad_action', { message: 'Wrong face.' });
          if (!['yes', 'maybe', 'no'].includes(b.answer)) return err(400, 'bad_action', { message: 'Pick Yes, Maybe or No.' });
          if ('plus_ones' in b && !v.fields.includes('plus_ones')) return err(400, 'bad_action', { message: 'No plus-ones on this one.' });
          if ('food' in b && !v.fields.includes('food_needs')) return err(400, 'bad_action', { message: 'No food needs on this one.' });
          const yes = b.answer === 'yes';
          v.me = { ...v.me, answer: b.answer, plus_ones: yes ? (b.plus_ones ?? 0) : 0, food: yes ? (b.food ?? []) : [], food_other: yes ? (b.food_other ?? null) : null, answered_at: new Date().toISOString() };
          if (v.people) v.people = [...v.people.filter((p) => p.id !== me.id), { id: me.id, name: me.name, avatar: me.avatar, answer: b.answer }];
          break;
        }
        case 'take': {
          const s = v.slots?.find((x) => x.slot === b.slot);
          if (!s) return err(400, 'bad_action', { message: 'Pick a slot.' });
          this.leave(v, me.id);
          if (s.full) {
            if (!v.waiting_list) return notNow(`${s.name} is full.`);
            s.waiting_people = [...(s.waiting_people ?? []), P(me.id, me.name, me.avatar)];
            s.waiting = s.waiting_people.length;
            v.me = { slot: null, note: null, waiting: { slot: s.slot, position: s.waiting }, moved_up: null };
          } else {
            s.people = [...(s.people ?? []), { ...P(me.id, me.name, me.avatar), note: b.note ?? null }];
            s.filled = s.people.length;
            s.full = s.filled >= s.cap;
            v.me = { slot: s.slot, note: b.note ?? null, waiting: null, moved_up: null };
          }
          break;
        }
        case 'leave':
        case 'let_go':
          if (!this.leave(v, me.id)) return notNow("You're not in a slot or on a waiting list.");
          v.me = { slot: null, note: null, waiting: null, moved_up: null };
          break;
        case 'seen':
          if (v.me?.moved_up) v.me.moved_up.new = false;
          break;
        case 'vote': {
          const r = b.ranking;
          if (!Array.isArray(r) || !r.length || (v.ballot.kind === 'one_pick' && r.length !== 1)) return err(400, 'bad_action', { message: 'Pick one option.' });
          if (!v.me?.voted) v.voted = (v.voted ?? 0) + 1;
          v.me = { voted: true, ballot: r };
          v.live = v.show_live_count ? [{ option: 0, votes: 5 }, { option: 1, votes: 4 }, { option: 2, votes: 2 }, { option: 3, votes: 0 }] : null;
          break;
        }
        default:
          return err(400, 'bad_action', { message: 'Unknown action.' });
      }
      this.version++;
      return { status: 200, body: this.env() };
    }
    return err(404, 'not_found');
  }
  /** Takes the guest out of any slot or waiting list; the 1st waiting moves up (the fake sets the counts it typed). */
  leave(v, id) {
    let out = false;
    for (const s of v.slots ?? []) {
      if (s.waiting_people?.some((p) => p.id === id)) {
        s.waiting_people = s.waiting_people.filter((p) => p.id !== id);
        s.waiting = s.waiting_people.length;
        out = true;
      }
      if (s.people?.some((p) => p.id === id)) {
        s.people = s.people.filter((p) => p.id !== id);
        if (s.waiting_people?.length) s.people.push({ ...s.waiting_people.shift(), note: null });
        s.waiting = s.waiting_people?.length ?? 0;
        s.filled = s.people.length;
        s.full = s.filled >= s.cap;
        out = true;
      }
    }
    return out;
  }
}

/** Gather scenario names in the order the Chrome walk shoots them. */
export const gatherOrder = Object.keys(gatherScenarios);
