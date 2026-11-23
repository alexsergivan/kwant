// The host remote's fake box (pa4b): pa1b's host/signin {token}, host/state, ws?view=host, host/action, host/remove and
// host/end for codes starting with `remote-`, in the real shapes (pa1 v1 envelope, viewer "host", around
// src/kits/quiz/view.rs QuizView + HostView). A small copy of the quiz engine's host rules: `next` in a question only
// locks it, another `next` reveals; after the last question of a round comes the leaderboard; adjust takes 1 to 100
// either way (MAX_ADJUST); host_can as in src/kits/quiz/view.rs. The code after `remote-` picks the scene:
// lobby, empty (nobody joined), question, locked, reveal, leaderboard, paused, miss (a near miss waits), last (the
// last reveal), final, solo, away (the box never answers) and expired (the link was used already).
// Every write is recorded in `writes` as [path, body] so a test can check what was sent.

const TEAMS = ['Quizzly Bears', 'Fact Hunters', 'The Late Answers', 'Table Four', 'Brain Freeze', 'Know-It-Owls'];
const NAMES = ['Mia', 'Sam', 'Jonas', 'Priya', 'Leo', 'Ana', 'Kai', 'Zoe', 'Omar', 'Lena', 'Tom', 'Ivy', 'Ben', 'Noor', 'Eli', 'Rosa', 'Finn', 'Maya', 'Theo', 'Nia', 'Hugo', 'Lou', 'Ada'];
const ROUNDS = [
  { title: 'Warm-up', n: 3 },
  { title: '90s music', n: 3 },
  { title: 'Pictures', n: 2 },
  { title: 'Final round', n: 2 },
];
const QS = [
  ['What is the capital of Australia?', 'Canberra'],
  ['How many legs does a spider have?', '8'],
  ['Which planet is known as the red planet?', 'Mars'],
  ['In which year did the Spice Girls release their first single, "Wannabe"?', '1996', ['1994', '1996', '1998', '2000']],
  ['Which band sang "Wonderwall"?', 'Oasis'],
  ['Who sang "...Baby One More Time"?', 'Britney Spears'],
  ['Which city is in this picture?', 'Lisbon'],
  ['Which animal is this?', 'Axolotl'],
  ['What is the chemical symbol for gold?', 'Au'],
  ['How many players are on a football team on the pitch?', '11'],
];
const SCORES = [1960, 1920, 1710, 1480, 1220, 1090];

function players(n, teams = true) {
  return NAMES.slice(0, n).map((name, i) => ({
    id: `p${i + 1}`, name, team: teams ? TEAMS[i % 6] : null, connected: i !== 19 && i !== 21, removed: false,
    score: teams ? 0 : 900 - i * 40, avatar: null, joined_at: null,
  }));
}

export class RemoteBox {
  constructor(code) {
    this.code = code;
    this.scene = code.slice('remote-'.length);
    this.version = 1;
    this.writes = [];
    this.signins = [];
    this.used = new Set();
    this.title = 'Friday Pub Quiz';
    this.teams = this.scene !== 'solo';
    this.players = players(this.scene === 'empty' ? 0 : this.scene === 'lobby' ? 14 : 23, this.teams);
    this.adjust = new Map(); // team or player id -> points the host added
    this.base = new Map(TEAMS.map((t, i) => [t, SCORES[i]]));
    this.q = 3; // flat 0-based question index
    this.phase = 'question';
    this.paused = false;
    this.remaining = 14_000;
    this.answers = new Set(this.players.slice(0, 18).map((p) => p.id));
    this.pending = [];
    this.ended = false;
    const s = this.scene;
    if (s === 'lobby' || s === 'empty') this.phase = 'lobby';
    if (s === 'locked') this.phase = 'locked';
    if (s === 'reveal') this.phase = 'reveal';
    if (s === 'leaderboard') (this.phase = 'leaderboard'), (this.q = 5);
    if (s === 'paused') (this.paused = true), (this.remaining = 12_000);
    if (s === 'last') (this.phase = 'reveal'), (this.q = 9);
    if (s === 'final') (this.phase = 'final'), (this.q = 9);
    if (s === 'solo') (this.phase = 'question'), (this.q = 4);
    if (s === 'miss') this.pending = [{ q: 3, player: 'p4', question: QS[3][0], answer: '1996', accept: ['1996', "'96"], guess: 'nineteen ninety six' }];
  }
  /** Round index and first flat index of the round for flat question q. */
  roundOf(q) {
    let first = 0;
    for (let r = 0; r < ROUNDS.length; r++) {
      if (q < first + ROUNDS[r].n) return { r, first };
      first += ROUNDS[r].n;
    }
    return { r: ROUNDS.length - 1, first: first - ROUNDS.at(-1).n };
  }
  can() {
    if (this.ended || this.phase === 'final') return [];
    const c = [];
    if (this.phase === 'lobby') c.push('start');
    else if (this.phase === 'question') c.push('next', this.paused ? 'resume' : 'pause');
    else c.push('next');
    c.push('adjust', 'remove', 'end');
    if (this.phase !== 'lobby') c.push('judge');
    return c;
  }
  teamBoard() {
    if (!this.teams) return [];
    const rows = TEAMS.map((team) => ({
      team, members: this.players.filter((p) => p.team === team && !p.removed).length, score: (this.base.get(team) ?? 0) + (this.adjust.get(team) ?? 0),
    })).filter((t) => t.members > 0 || this.phase !== 'lobby');
    rows.sort((a, b) => b.score - a.score);
    return rows.map((t, i) => ({ ...t, rank: i + 1, tied: false }));
  }
  view() {
    const live = this.players.filter((p) => !p.removed);
    const inQ = this.phase !== 'lobby' && this.phase !== 'final' && this.phase !== 'leaderboard';
    const [text, answer, options] = QS[this.q];
    const { r } = this.roundOf(this.q);
    const playersOut = this.players.map((p) => ({ ...p, score: this.teams ? (this.base.get(p.team) ?? 0) / 5 : p.score + (this.adjust.get(p.id) ?? 0) }));
    return {
      kit: 'quiz', viewer: 'host', title: this.title, theme: null, teams: this.teams, line: null,
      round: this.phase === 'lobby' ? null : { number: r + 1, of: ROUNDS.length, title: ROUNDS[r].title, intro: null },
      question: inQ ? { q: this.q, number: this.q + 1, of: QS.length, kind: options ? 'choice' : 'text', text, options: options ?? null, picture: null, points: 100, time_limit_s: 30 } : null,
      time: this.phase === 'question' ? { deadline: null, remaining_ms: this.remaining, paused: this.paused } : null,
      answered: inQ ? { count: this.answers.size, of: live.length, names: [] } : null,
      reveal: null,
      leaderboard: [],
      team_board: this.teamBoard(),
      host: {
        answer_now: inQ ? { q: this.q, answer, position: options ? options.indexOf(answer) : null } : null,
        accept: [],
        answers: inQ ? [...this.answers].map((id) => ({ player: id, name: this.players.find((p) => p.id === id)?.name, answer: answer, verdict: 'yes', points: 100 })) : [],
        pending: inQ ? this.pending : [],
        players: playersOut,
        can: this.can(),
      },
      phase: this.phase,
    };
  }
  env() {
    return {
      api: 'pocket/1', version: this.version, viewer: 'host',
      pocket: { code: this.code, kit: 'quiz', title: this.title, state: this.ended ? 'ended' : 'open', opens_at: null, closes_at: null, ended_at: null, expires_at: null, players: this.players.length, max_players: 60, needs_pin: false },
      me: null,
      view: this.view(),
    };
  }
  bump() {
    this.version++;
    return { status: 200, body: this.env() };
  }
  /** pa1b: one host action. Errors are the engine's own codes, in plain words. */
  action(name, body = {}) {
    const err = (error, message, status = 409) => ({ status, body: { error, message } });
    if (!this.can().includes(name)) return err('not_now', "That can't be done right now.");
    switch (name) {
      case 'start':
        if (!this.players.some((p) => !p.removed)) return err('nobody', 'Nobody has joined yet.');
        this.phase = 'question';
        this.q = 0;
        this.answers = new Set();
        this.remaining = 30_000;
        break;
      case 'pause':
        this.paused = true;
        break;
      case 'resume':
        this.paused = false;
        break;
      case 'next': {
        if (this.phase === 'question') (this.phase = 'locked'), (this.paused = false);
        else if (this.phase === 'locked') (this.phase = 'reveal'), (this.pending = []);
        else if (this.phase === 'reveal') {
          const { r, first } = this.roundOf(this.q);
          if (this.q >= QS.length - 1) this.phase = 'final';
          else if (this.q === first + ROUNDS[r].n - 1) this.phase = 'leaderboard';
          else (this.q++), (this.phase = 'question'), (this.answers = new Set()), (this.remaining = 30_000);
        } else if (this.phase === 'leaderboard') (this.q++), (this.phase = 'question'), (this.answers = new Set()), (this.remaining = 30_000);
        break;
      }
      case 'adjust': {
        const pts = body.points;
        if (!Number.isInteger(pts) || pts === 0 || Math.abs(pts) > 100) return err('bad_points', 'Points must be between 1 and 100, up or down.', 400);
        const key = body.team ?? body.player;
        if (body.team && !TEAMS.includes(body.team)) return err('no_team', 'No team by that name.', 400);
        if (body.player && !this.players.some((p) => p.id === body.player)) return err('no_player', 'No player by that id.', 400);
        if (!key) return err('bad_body', 'Name a team or a player.', 400);
        this.adjust.set(key, (this.adjust.get(key) ?? 0) + pts);
        break;
      }
      case 'judge': {
        const i = this.pending.findIndex((p) => p.q === body.q && p.player === body.player);
        if (i < 0) return err('not_pending', 'That answer was already judged.');
        if (!['yes', 'no', 'partial'].includes(body.verdict)) return err('bad_verdict', 'Say yes, no or partial.', 400);
        const [p] = this.pending.splice(i, 1);
        if (body.verdict === 'yes') {
          const pl = this.players.find((x) => x.id === p.player);
          if (pl?.team) this.base.set(pl.team, (this.base.get(pl.team) ?? 0) + 100);
        }
        break;
      }
      default:
        return err('unknown', 'Unknown action.', 400);
    }
    return this.bump();
  }
  /** `signed`: the request carries this pocket's kph_ cookie (the sign-in set it). */
  handle(path, body = {}, { signed = false, method = 'GET' } = {}) {
    const [what, query] = path.split('?');
    const err = (status, error, message = 'Fake box.') => ({ status, body: { error, message } });
    if (this.scene === 'away') return err(503, 'away');
    if (what === 'host/signin') {
      this.signins.push(body);
      if (method !== 'POST') return err(405, 'method');
      const token = body.token;
      if (!token) return err(400, 'no_key', 'Send the link token.');
      if (this.scene === 'expired' || token === 'used' || this.used.has(token)) return err(401, 'link_used_or_expired', 'This link was already used or is too old. Open a new one from the card.');
      if (token === 'screen') return { status: 200, body: { ...this.env(), viewer: 'screen' }, cookie: 'screen' };
      this.used.add(token);
      return { status: 200, body: this.env(), cookie: 'host' };
    }
    if (!signed) return err(401, 'signed_out', 'Open the remote again from the card.');
    if (signed === 'screen') return err(403, 'screen_only', 'This browser is a big screen.');
    if (what === 'host/state') {
      const since = Number(new URLSearchParams(query).get('since'));
      if (since && since === this.version) return { status: 204, body: null };
      return { status: 200, body: this.env() };
    }
    if (method !== 'POST') return err(405, 'method');
    this.writes.push([what, body]);
    if (this.ended) return err(410, 'ended', 'This quiz has ended.');
    if (what === 'host/action') return this.action(body.name, body.body ?? {});
    if (what === 'host/remove') {
      const p = this.players.find((x) => x.id === body.player_id);
      if (!p || p.removed) return err(404, 'no_player', 'That player is not in the quiz.');
      p.removed = true;
      this.answers.delete(p.id);
      return this.bump();
    }
    if (what === 'host/end') {
      this.ended = true;
      this.phase = 'final';
      return this.bump();
    }
    return err(404, 'not_found');
  }
}
