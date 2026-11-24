// Fake guest port for the guest page, speaking pa1's Pocket API v1 (pa1/impl.md): the state envelope
// {api, version, viewer, pocket, me, view}, 401 signed_out without a seat, 403 removed, 410 ended, 204 on state?since.
// The scenarios below are written flat for reading; `envelope()` splits each into pa1's pocket/me facts and the kit's
// `view` (the quiz guest view pa5 proposes to pa2, see pa5/impl.md). `FakeBox` is the one box both
// tests/guestPage.test.mjs and guest/fake/server.mjs (Chrome under the CSP, guest/e2e/csp.mjs) talk to.
// Names, teams and questions are sample data from the pa0a mockups.

const base = {
  v: 1,
  kit: 'quiz',
  noun: 'quiz',
  title: 'Friday Pub Quiz',
  host: 'Dana',
  starts_at: '2026-11-27T20:00:00',
  rounds: 4,
  team_max: 4,
  max_players: 60,
  retention: { kind: 'after_end', days: 7 },
};

export const teams = [
  { id: 't1', name: 'Quizzly Bears', color: 0, count: 2, max: 4 },
  { id: 't2', name: 'Fact Hunters', color: 1, count: 3, max: 4 },
  { id: 't3', name: 'Table Four', color: 3, count: 4, max: 4, full: true },
];

const me = { name: 'Mia', avatar: 'visor', team: 't1', team_name: 'Quizzly Bears', team_color: 0, team_count: 3, score: 1240 };

const players = [
  ['Mia', 'visor'], ['Sam', 'antenna'], ['Jonas', 'spikes'], ['Priya', 'beret'],
  ['Leo', 'goggles'], ['Ana', 'bulb'], ['Kai', 'horns'], ['Zoe', 'ears'],
].map(([name, avatar]) => ({ name, avatar }));

const qChoice = {
  id: 'q8', round: 2, round_name: '90s music', n: 8, of: 10, kind: 'choice',
  text: 'In which year did the Spice Girls release their first single, "Wannabe"?',
  options: ['1994', '1996', '1998', '2000'], secs_left: 14, total_secs: 20,
};
const qText = { id: 'q3', round: 1, round_name: 'General knowledge', n: 3, of: 10, kind: 'text', text: 'What is the capital city of Australia?', secs_left: 22, total_secs: 30 };
const qNumber = { id: 'q2', round: 4, round_name: 'Numbers', n: 2, of: 10, kind: 'number', text: 'How many bones are in an adult human body?', number: { start: 212, min: 0, max: 1000 }, secs_left: 9, total_secs: 30 };
const qPicture = {
  id: 'q4', round: 3, round_name: 'Pictures', n: 4, of: 10, kind: 'picture', text: 'Which fruit is this, cut in half?',
  picture: { url: '/api/pocket/CODE/media/kiwi.svg', alt: 'A green fruit cut in half, with a pale centre and small black seeds', caption: 'Picture: drawn by Quizmaster', pixel: true },
  options: ['Lime', 'Kiwi', 'Fig', 'Guava'], secs_left: 17, total_secs: 30,
};

const board = [
  { rank: 1, name: 'Quizzly Bears', color: 0, score: 1960, move: 2, me: true },
  { rank: 2, name: 'Fact Hunters', color: 1, score: 1920, move: -1 },
  { rank: 3, name: 'The Late Answers', color: 2, score: 1710 },
  { rank: 4, name: 'Table Four', color: 3, score: 1480, move: -1 },
  { rank: 5, name: 'Brain Freeze', color: 4, score: 1220 },
];

const lobby = { ...base, phase: 'lobby', me, teams, player_count: 14, team_count: 5, players, preload: ['/api/pocket/CODE/media/kiwi.svg'] };

const pub = { ...base, phase: 'lobby', teams, player_count: 14, team_count: 5 };

/** pa11: the class quiz's line and the join refusal, word for word from src/pocket (quiz_kit.rs CLASS_NOTICE, words.rs). */
export const CLASS_NOTICE = 'First names or nicknames only. Everything about players is deleted when the quiz ends. Nothing is shared.';
export const FIRST_ONLY = 'First name or nickname only, please: no surname, email or phone number.';
const notFirstName = (n = '') => n.trim().split(/\s+/).length > 2 || /[@＠]/.test(n) || (n.match(/\d/g) ?? []).length >= 6;

/**
 * Each scenario: what `state` (with this phone's cookie), `info` (no seat yet), `join`, `rejoin` (by name) and
 * `action` answer. A number is an HTTP error status; `state: 401` means this phone has no seat.
 */
export const scenarios = {
  join: { state: 401, info: pub, join: lobby, rejoin: { ...lobby, phase: 'question', question: qChoice } },
  'join-solo': { state: 401, info: { ...base, phase: 'lobby', teams: null, team_max: undefined, retention: { kind: 'at_end' } }, join: { ...lobby, teams: null, me: { ...me, team: undefined, team_name: undefined } } },
  // pa11 class quiz (Lin's #1143): the class line, first names only, deleted at End. Not in `order`: tests/guestPage
  // and the shots open it by name. A name of 3+ words, an @ or 6+ digits gets 400 name_first_only (the box's rule).
  'join-class': {
    state: 401,
    info: { ...base, title: 'Year 8 Science', host: 'Ms Patel', phase: 'lobby', teams: null, team_max: undefined, player_count: 0, max_players: 70, retention: { kind: 'at_end' }, first_names_only: true, notice: CLASS_NOTICE },
    join: { ...lobby, title: 'Year 8 Science', host: 'Ms Patel', teams: null, retention: { kind: 'at_end' }, first_names_only: true, notice: CLASS_NOTICE, me: { ...me, team: undefined, team_name: undefined } },
  },
  lobby: { state: lobby },
  rejoin: { state: { ...base, phase: 'question', me, question: qChoice } },
  away: { state: 503, info: pub },
  full: { state: 401, info: { ...base, phase: 'lobby', player_count: 60 } },
  ended: { state: 401, info: { ...base, phase: 'ended', ended: { winner: 'Quizzly Bears', points: 3420 }, deletes_on: '2026-12-04' } },
  choice: { state: { ...base, phase: 'question', me, question: qChoice }, action: { ...base, phase: 'question', me, question: { ...qChoice, answer: { choice: 'B' }, waiting: 5, secs_left: 9 } } },
  text: { state: { ...base, phase: 'question', me, question: qText }, action: { ...base, phase: 'question', me, question: { ...qText, answer: { text: 'Canbera' }, waiting: 5 } } },
  number: { state: { ...base, phase: 'question', me, question: qNumber } },
  picture: { state: { ...base, phase: 'question', me, question: qPicture } },
  locked: { state: { ...base, phase: 'question', me, question: { ...qChoice, answer: { choice: 'B' }, waiting: 5, secs_left: 9 } } },
  right: {
    state: {
      ...base, phase: 'reveal', me: { ...me, score: 1360 },
      reveal: { question: qChoice, outcome: 'right', points: 120, base: 100, speed: 20, answer: '1996', answer_letter: 'B', pct: 62, source: 'Wikipedia, "Wannabe (song)": released 7 July 1996 in the UK.', team: { rank: 1, name: 'Quizzly Bears', color: 0, score: 1960 } },
    },
  },
  near: {
    state: {
      ...base, phase: 'reveal', me: { ...me, score: 1340 },
      reveal: { question: qText, outcome: 'near', points: 100, yours: 'Canbera', why: 'One letter off, so it counts.', answer: 'Canberra', source: 'Wikipedia, "Canberra": the capital of Australia since 1913.' },
    },
  },
  overturned: {
    state: {
      ...base, phase: 'reveal', me,
      reveal: { question: qText, outcome: 'overturned', points: -100, yours: 'Canbra', answer: 'Canberra', source: 'Wikipedia, "Canberra": the capital of Australia since 1913.' },
    },
  },
  wrong: {
    state: {
      ...base, phase: 'reveal', me,
      reveal: { question: qNumber, outcome: 'wrong', points: 40, close: true, yours: '212', answer: '206', source: 'Wikipedia, "Human skeleton": an adult has 206 bones.' },
    },
  },
  between: {
    state: {
      ...base, phase: 'between', me: { ...me, score: 1360 },
      between: { after_round: 2, rounds: 4, next_in_secs: 40, line: 'Quizzly Bears jump to first, just 40 ahead. Round 3 is pictures, so phones up and screens bright.', board },
    },
  },
  paused: { state: { ...base, phase: 'paused', me } },
  removed: { state: 403, info: pub },
  missing: { state: 404 },
};

const ERRORS = { 401: 'signed_out', 403: 'removed', 404: 'not_found', 409: 'name_taken', 410: 'ended', 503: 'unavailable' };
export const errorBody = (status, error = ERRORS[status] ?? 'error') => ({ error, message: 'Fake guest port.' });

/** A flat scenario view as pa1's v1 state envelope: pocket and me facts from pa1, the rest as the kit's `view`. */
export function envelope(v, { code = 'CODE', version = 1, withView = true } = {}) {
  const { v: _v, kit, title, max_players, player_count, retention, deletes_on, me, notice, first_names_only, ...kit_view } = v;
  const { name, avatar, team_name, ...kit_me } = me ?? {};
  const ended = v.phase === 'ended';
  const out = {
    api: 'pocket/1',
    version,
    viewer: me ? 'guest' : 'anyone',
    pocket: {
      code, kit, title, state: ended ? 'ended' : 'open', opens_at: null, closes_at: null,
      ended_at: ended ? '2026-11-27T22:10:00Z' : null, expires_at: deletes_on ? `${deletes_on}T22:10:00Z` : null,
      players: player_count ?? 14, max_players, needs_pin: false, retention,
      // pa11: only a class quiz sends these, so every other scenario's envelope stays as it was.
      ...(first_names_only !== undefined ? { first_names_only } : {}),
      ...(notice !== undefined ? { notice } : {}),
    },
  };
  if (me) out.me = { id: 'p1', name, avatar, team: team_name ?? null, notices: [] };
  if (withView) out.view = me ? { ...kit_view, me: kit_me } : kit_view;
  return out;
}

const fill = (v, code) => (v && typeof v === 'object' ? JSON.parse(JSON.stringify(v).replaceAll('/CODE/', `/${code}/`)) : v);

/**
 * One fake pocket. `handle(path, body)` answers like pa1's guest routes and returns {status, body}.
 * `info` carries the kit's join facts as `view` (pa5 asks pa1 for that; until then the page joins without a team list).
 */
export class FakeBox {
  constructor(name, { code = name, overrides = {} } = {}) {
    this.s = { ...scenarios[name], ...overrides };
    this.code = code;
    this.seat = null; // flat view after join or rejoin
    this.version = 1;
    this.joinError = null;
  }
  env(v) {
    return envelope(fill(v, this.code), { code: this.code, version: this.version });
  }
  /** `seated`: this phone has the seat's cookie. A phone without it never sees another phone's seat. */
  current(seated = true) {
    return (seated && this.seat) || this.s.state || 401;
  }
  /** pa1 sends a new state on every change: bump the version. */
  set(v) {
    this.seat = v;
    this.version++;
  }
  handle(path, body = {}, { seated = true } = {}) {
    const [what, query] = path.split('?');
    const err = (status, code) => ({ status, body: errorBody(status, code) });
    if (!this.s.state && !this.s.info) return err(404);
    if (what === 'state') {
      const v = this.current(seated);
      if (typeof v === 'number') return err(v);
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env(v) };
    }
    if (what === 'info') {
      const v = this.s.info ?? this.current(seated);
      if (typeof v === 'number') return err(v === 401 ? 404 : v);
      const { me: _me, ...pub } = v;
      return { status: 200, body: this.env(pub) };
    }
    if (what === 'join') {
      if (this.joinError) return err(this.joinError === 'full' || this.joinError === 'name_taken' || this.joinError === 'not_open_yet' ? 409 : 400, this.joinError);
      if (this.s.info?.first_names_only && notFirstName(body.name)) return { status: 400, body: { error: 'name_first_only', message: FIRST_ONLY } };
      if (!this.s.join) return err(410);
      const v = fill(this.s.join, this.code);
      this.set({ ...v, me: { ...v.me, name: body.name ?? v.me.name, avatar: body.avatar ?? v.me.avatar, team_name: body.team ?? v.me.team_name } });
      return { status: 201, body: this.env(this.seat) };
    }
    if (what === 'rejoin') {
      if (!this.s.rejoin) return err(401, 'signed_out');
      const v = fill(this.s.rejoin, this.code);
      this.set({ ...v, me: { ...v.me, name: body.name ?? v.me.name } });
      return { status: 200, body: this.env(this.seat) };
    }
    if (what === 'action') {
      const now = this.current(seated);
      if (typeof now === 'number') return err(now);
      if (!this.s.action) return { status: 200, body: this.env(now) };
      this.set(fill(this.s.action, this.code));
      return { status: 200, body: this.env(this.seat) };
    }
    return err(404);
  }
}

/** Scenario names in the order the shots and the browser run walk them. */
export const order = ['join', 'join-solo', 'lobby', 'rejoin', 'away', 'full', 'ended', 'choice', 'text', 'number', 'picture', 'locked', 'right', 'near', 'overturned', 'wrong', 'between', 'paused', 'removed', 'missing'];

/** The policy pa1 sends with GET /p/<code>, exactly as in pa1/brief.md. */
export const CSP = "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";

/** A tiny pixel picture for the picture question (a kiwi cut in half). */
export const KIWI_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 20" shape-rendering="crispEdges"><rect width="32" height="20" fill="#120e26"/><rect x="8" y="2" width="16" height="16" fill="#7a5230"/><rect x="9" y="3" width="14" height="14" fill="#8fd14f"/><rect x="13" y="7" width="6" height="6" fill="#f2f7d9"/><rect x="11" y="6" width="1" height="1" fill="#1d1840"/><rect x="20" y="6" width="1" height="1" fill="#1d1840"/><rect x="11" y="13" width="1" height="1" fill="#1d1840"/><rect x="20" y="13" width="1" height="1" fill="#1d1840"/><rect x="15" y="5" width="1" height="1" fill="#1d1840"/><rect x="16" y="14" width="1" height="1" fill="#1d1840"/></svg>`;
