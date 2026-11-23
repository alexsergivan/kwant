// The big screen's fake box (pa5b): pa1's host/signin, host/state?view=screen and ws?view=screen for codes starting
// with `screen-`, in the real shapes (pa1 v1 envelope, viewer "screen", around src/kits/quiz/view.rs QuizView for
// Viewer::BigScreen). The code after `screen-` picks the scene: lobby, question, locked, reveal, leaderboard, winners,
// paused, ended, solo, away (the box never answers) and expired (the link was used already).
// Every host write (anything under host/ but signin and state) is refused with 403 screen_only and recorded in
// `writes`, so a test can assert the screen never sent one.

const TEAMS = ['Quizzly Bears', 'Fact Hunters', 'The Late Answers', 'Table Four', 'Brain Freeze', 'Know-It-Owls'];
const NAMES = ['Mia', 'Sam', 'Jonas', 'Priya', 'Leo', 'Ana', 'Kai', 'Zoe', 'Omar', 'Lena', 'Tom', 'Ivy', 'Ben', 'Noor', 'Eli', 'Rosa', 'Finn', 'Maya', 'Theo', 'Nia', 'Hugo', 'Lou', 'Ada'];
const lobby = (n) => NAMES.slice(0, n).map((name, i) => ({ name, team: TEAMS[i % 5] }));
const Q = {
  number: 8, of: 10, kind: 'choice',
  text: 'In which year did the Spice Girls release their first single, "Wannabe"?',
  options: ['1994', '1996', '1998', '2000'], picture: null, points: 100, time_limit_s: 30,
};
const ROUND = { number: 2, of: 4, title: '90s music', intro: null };
const BOARD = [
  ['Quizzly Bears', 5, 1960], ['Fact Hunters', 4, 1920], ['The Late Answers', 4, 1710], ['Table Four', 5, 1480], ['Brain Freeze', 3, 1220], ['Know-It-Owls', 2, 1090],
].map(([team, members, score], i) => ({ team, members, score, rank: i + 1, tied: false }));
const FINAL = [['Quizzly Bears', 3420], ['Fact Hunters', 3210], ['The Late Answers', 2980], ['Table Four', 2650], ['Brain Freeze', 2410], ['Know-It-Owls', 2200]]
  .map(([team, score], i) => ({ team, members: 4, score, rank: i + 1, tied: false }));
const base = { kit: 'quiz', viewer: 'screen', title: 'Friday Pub Quiz', theme: null, teams: true, line: null, round: null, question: null, time: null, answered: null, reveal: null, leaderboard: [], team_board: [], lobby: lobby(23) };

export const scenes = {
  lobby: { ...base, phase: 'lobby', line: "Grab a seat and pick a team name you'd be happy to hear me read out loud.", lobby: lobby(14), players: 14 },
  question: { ...base, phase: 'question', round: ROUND, question: Q, time: { deadline: null, remaining_ms: 14_000, paused: false }, answered: { count: 18, of: 23, names: NAMES.slice(0, 18) } },
  locked: { ...base, phase: 'locked', round: ROUND, question: Q, time: null, answered: { count: 23, of: 23, names: NAMES } },
  reveal: {
    ...base, phase: 'reveal', round: ROUND, question: Q, answered: { count: 23, of: 23, names: NAMES },
    reveal: { q: 8, answer: '1996', position: 1, source_url: 'https://en.wikipedia.org/wiki/Wannabe_(song)', source_quote: 'released 7 July 1996 in the UK.', line: 'Fastest right answer: Mia, Quizzly Bears, in 2.1 seconds.', picks: [2, 14, 5, 2], right: 14 },
  },
  leaderboard: { ...base, phase: 'leaderboard', round: ROUND, team_board: BOARD, line: 'Quizzly Bears jump to first, just 40 ahead. Round 3 is pictures, so phones up and screens bright.' },
  winners: { ...base, phase: 'final', round: { ...ROUND, number: 4 }, team_board: FINAL, line: 'What a night. Same time next Friday?' },
  paused: { ...base, phase: 'question', round: ROUND, question: Q, time: { deadline: null, remaining_ms: 12_000, paused: true }, answered: { count: 9, of: 23, names: NAMES.slice(0, 9) } },
  ended: { ...base, phase: 'final', team_board: FINAL, ended: true },
  solo: {
    ...base, phase: 'leaderboard', teams: false, round: ROUND, line: null,
    leaderboard: NAMES.slice(0, 10).map((name, i) => ({ name, team: null, score: 900 - i * 70, rank: i + 1, tied: false })),
  },
};

export class ScreenBox {
  constructor(code) {
    this.code = code;
    this.scene = code.slice('screen-'.length);
    this.view = scenes[this.scene] ?? (this.scene === 'away' || this.scene === 'expired' ? scenes.lobby : null);
    this.version = 1;
    this.writes = []; // every host write the screen tried (must stay empty)
    this.signins = [];
  }
  env(v = this.view) {
    const { ended, players, ...view } = v;
    return {
      api: 'pocket/1', version: this.version, viewer: 'screen',
      pocket: { code: this.code, kit: 'quiz', title: view.title, state: ended ? 'ended' : 'open', opens_at: null, closes_at: null, ended_at: null, expires_at: null, players: players ?? view.lobby?.length ?? 0, max_players: 60, needs_pin: false },
      me: null,
      view,
    };
  }
  set(v) {
    this.view = v;
    this.version++;
  }
  /** `signed`: the request carries this pocket's kph_ cookie (the sign-in set it). */
  handle(path, body = {}, { signed = false, method = 'GET' } = {}) {
    const [what, query] = path.split('?');
    const err = (status, error, message = 'Fake box.') => ({ status, body: { error, message } });
    if (this.scene === 'away') return err(503, 'away');
    if (!this.view) return err(404, 'not_found');
    if (what === 'host/signin') {
      this.signins.push(body);
      if (method !== 'POST') return err(405, 'method');
      const token = body.key ?? body.token;
      if (this.scene === 'expired' || !token || token === 'used') return err(401, 'link_used_or_expired', 'This link was already used or is too old. Open a new one from the card.');
      return { status: 200, body: this.env(), cookie: true };
    }
    if (what === 'host/state') {
      if (!signed) return err(401, 'signed_out');
      if (new URLSearchParams(query).get('view') !== 'screen') return err(400, 'view');
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env() };
    }
    if (what.startsWith('host/')) {
      this.writes.push(`${method} ${path}`);
      return err(403, 'screen_only', 'This screen can only show the game.');
    }
    return err(404, 'not_found');
  }
}
