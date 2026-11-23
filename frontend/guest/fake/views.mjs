// Fake views for the guest page, shaped like pa1/brief.md (Pocket API v1 as the guest page uses it; see pa5/impl.md).
// Used by tests/guestPage.test.mjs (jsdom), guest/fake/server.mjs and guest/e2e/csp.mjs (Chrome under the CSP).
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

/** Each scenario: what `rejoin`, `state`, `join` and `action` answer. A number is an HTTP error status. */
export const scenarios = {
  join: { rejoin: 401, state: { ...base, phase: 'lobby', teams, player_count: 14, team_count: 5 }, join: lobby },
  'join-solo': { rejoin: 401, state: { ...base, phase: 'lobby', teams: null, team_max: undefined, retention: { kind: 'at_end' } }, join: { ...lobby, teams: null, me: { ...me, team: undefined, team_name: undefined } } },
  lobby: { rejoin: lobby, state: lobby },
  rejoin: { rejoin: { ...base, phase: 'question', me, question: qChoice }, state: { ...base, phase: 'question', me, question: qChoice } },
  away: { rejoin: 503, state: 503 },
  full: { rejoin: 401, state: { ...base, phase: 'full', player_count: 60 } },
  ended: { rejoin: 401, state: { ...base, phase: 'ended', ended: { winner: 'Quizzly Bears', points: 3420 }, deletes_on: '2026-12-04' } },
  choice: { rejoin: { ...base, phase: 'question', me, question: qChoice }, action: { ...base, phase: 'question', me, question: { ...qChoice, answer: { choice: 'B' }, waiting: 5, secs_left: 9 } } },
  text: { rejoin: { ...base, phase: 'question', me, question: qText }, action: { ...base, phase: 'question', me, question: { ...qText, answer: { text: 'Canbera' }, waiting: 5 } } },
  number: { rejoin: { ...base, phase: 'question', me, question: qNumber } },
  picture: { rejoin: { ...base, phase: 'question', me, question: qPicture } },
  locked: { rejoin: { ...base, phase: 'question', me, question: { ...qChoice, answer: { choice: 'B' }, waiting: 5, secs_left: 9 } } },
  right: {
    rejoin: {
      ...base, phase: 'reveal', me: { ...me, score: 1360 },
      reveal: { question: qChoice, outcome: 'right', points: 120, base: 100, speed: 20, answer: '1996', answer_letter: 'B', pct: 62, source: 'Wikipedia, "Wannabe (song)": released 7 July 1996 in the UK.', team: { rank: 1, name: 'Quizzly Bears', color: 0, score: 1960 } },
    },
  },
  near: {
    rejoin: {
      ...base, phase: 'reveal', me: { ...me, score: 1340 },
      reveal: { question: qText, outcome: 'near', points: 100, yours: 'Canbera', why: 'One letter off, so it counts.', answer: 'Canberra', source: 'Wikipedia, "Canberra": the capital of Australia since 1913.' },
    },
  },
  overturned: {
    rejoin: {
      ...base, phase: 'reveal', me,
      reveal: { question: qText, outcome: 'overturned', points: 0, yours: 'Canbra', answer: 'Canberra', source: 'Wikipedia, "Canberra": the capital of Australia since 1913.' },
    },
  },
  wrong: {
    rejoin: {
      ...base, phase: 'reveal', me,
      reveal: { question: qNumber, outcome: 'wrong', points: 40, close: true, yours: '212', answer: '206', source: 'Wikipedia, "Human skeleton": an adult has 206 bones.' },
    },
  },
  between: {
    rejoin: {
      ...base, phase: 'between', me: { ...me, score: 1360 },
      between: { after_round: 2, rounds: 4, next_in_secs: 40, line: 'Quizzly Bears jump to first, just 40 ahead. Round 3 is pictures, so phones up and screens bright.', board },
    },
  },
  paused: { rejoin: { ...base, phase: 'paused', me } },
  removed: { rejoin: { ...base, phase: 'removed' }, state: { ...base, phase: 'removed' } },
};

/** Scenario names in the order the shots and the browser run walk them. */
export const order = ['join', 'join-solo', 'lobby', 'rejoin', 'away', 'full', 'ended', 'choice', 'text', 'number', 'picture', 'locked', 'right', 'near', 'overturned', 'wrong', 'between', 'paused', 'removed'];

/** The policy pa1 sends with GET /p/<code>, exactly as in pa1/brief.md. */
export const CSP = "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";

/** A tiny pixel picture for the picture question (a kiwi cut in half). */
export const KIWI_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 20" shape-rendering="crispEdges"><rect width="32" height="20" fill="#120e26"/><rect x="8" y="2" width="16" height="16" fill="#7a5230"/><rect x="9" y="3" width="14" height="14" fill="#8fd14f"/><rect x="13" y="7" width="6" height="6" fill="#f2f7d9"/><rect x="11" y="6" width="1" height="1" fill="#1d1840"/><rect x="20" y="6" width="1" height="1" fill="#1d1840"/><rect x="11" y="13" width="1" height="1" fill="#1d1840"/><rect x="20" y="13" width="1" height="1" fill="#1d1840"/><rect x="15" y="5" width="1" height="1" fill="#1d1840"/><rect x="16" y="14" width="1" height="1" fill="#1d1840"/></svg>`;
