// The big screen's view (pa5b): pa1's v1 state envelope with `viewer: "screen"` around the quiz kit's
// `view(Viewer::BigScreen)` (src/kits/quiz/view.rs QuizView). Values are copied or left out; nothing here works out a
// score, a rank, a count or a percentage. Fields the kit doesn't send yet (a lobby row's `avatar`, a team's `color`, a
// board row's `move`, a reveal's per-option `pcts`) are read when present and otherwise not drawn (pa5b/impl.md).

export type ScreenPhase = 'lobby' | 'question' | 'locked' | 'reveal' | 'leaderboard' | 'final';

export interface LobbyRow {
  name: string;
  team?: string;
  avatar?: string;
}

export interface Row {
  name: string;
  score: number;
  rank: number;
  tied: boolean;
  /** Teams: how many players. */
  members?: number;
  /** Solo: the player's team, if any. */
  team?: string;
  /** Places moved since the last board (not sent by the kit yet). */
  move?: number;
  /** Team colour index (not sent by the kit yet; else join order). */
  color?: number;
}

export interface ScreenView {
  version?: number;
  ended: boolean;
  title: string;
  teams: boolean;
  phase: ScreenPhase;
  players?: number;
  line?: string;
  round?: { number: number; of: number; title: string; intro?: string };
  question?: { number: number; of: number; kind: string; text: string; options?: string[]; picture?: { url: string; alt?: string; credit?: string }; time_limit_s: number };
  time?: { remaining_ms: number; paused: boolean };
  answered?: { count: number; of: number; names?: string[] };
  reveal?: { answer: string; position?: number; source_url: string; source_quote: string; line?: string; picks?: number[]; right?: number; pcts?: number[] };
  board?: Row[];
  lobby?: LobbyRow[];
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);
const nums = (x: unknown): number[] | undefined => (Array.isArray(x) && x.every((n) => num(n) !== undefined) ? (x as number[]) : undefined);
const strs = (x: unknown): string[] | undefined => (Array.isArray(x) && x.every((n) => typeof n === 'string') ? (x as string[]) : undefined);
const PHASES: ScreenPhase[] = ['lobby', 'question', 'locked', 'reveal', 'leaderboard', 'final'];

function row(x: unknown, team: boolean): Row | undefined {
  const r = obj(x);
  const name = str(team ? r?.team : r?.name);
  const score = num(r?.score);
  const rank = num(r?.rank);
  if (!name || score === undefined || rank === undefined) return undefined;
  return { name, score, rank, tied: r?.tied === true, members: num(r?.members), team: team ? undefined : str(r?.team), move: num(r?.move), color: num(r?.color) };
}

/** One v1 screen state (from host/signin, host/state?view=screen or a ws frame). Undefined when it isn't one. */
export function screenFrom(json: unknown): { view: ScreenView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  const v = obj(j?.view);
  if (!j || !pocket || !v) return undefined;
  const phase = PHASES.find((p) => p === v.phase);
  if (!phase) return undefined;
  const teams = v.teams === true;
  const round = obj(v.round);
  const q = obj(v.question);
  const time = obj(v.time);
  const answered = obj(v.answered);
  const rv = obj(v.reveal);
  const boardSrc = teams && Array.isArray(v.team_board) ? v.team_board : v.leaderboard;
  const fromTeams = teams && Array.isArray(v.team_board);
  const view: ScreenView = {
    ended: str(pocket.state) === 'ended',
    title: str(v.title) ?? str(pocket.title) ?? '',
    teams,
    phase,
    players: num(pocket.players),
    line: str(v.line),
    round: round && num(round.number) !== undefined && num(round.of) !== undefined
      ? { number: num(round.number)!, of: num(round.of)!, title: str(round.title) ?? '', intro: str(round.intro) }
      : undefined,
    question: q && str(q.text) !== undefined && num(q.number) !== undefined
      ? {
          number: num(q.number)!,
          of: num(q.of) ?? 0,
          kind: str(q.kind) ?? 'text',
          text: str(q.text)!,
          options: strs(q.options),
          picture: obj(q.picture) && str(obj(q.picture)!.url) ? { url: str(obj(q.picture)!.url)!, alt: str(obj(q.picture)!.alt), credit: str(obj(q.picture)!.credit) } : undefined,
          time_limit_s: num(q.time_limit_s) ?? 0,
        }
      : undefined,
    time: time && num(time.remaining_ms) !== undefined ? { remaining_ms: num(time.remaining_ms)!, paused: time.paused === true } : undefined,
    answered: answered && num(answered.count) !== undefined && num(answered.of) !== undefined
      ? { count: num(answered.count)!, of: num(answered.of)!, names: strs(answered.names) }
      : undefined,
    reveal: rv && str(rv.answer) !== undefined
      ? {
          answer: str(rv.answer)!,
          position: num(rv.position),
          source_url: str(rv.source_url) ?? '',
          source_quote: str(rv.source_quote) ?? '',
          line: str(rv.line),
          picks: nums(rv.picks),
          right: num(rv.right),
          pcts: nums(rv.pcts),
        }
      : undefined,
    board: Array.isArray(boardSrc) ? boardSrc.map((x) => row(x, fromTeams)).filter((r): r is Row => !!r) : undefined,
    lobby: Array.isArray(v.lobby)
      ? v.lobby.map((x) => obj(x)).filter((x): x is Obj => !!x && !!str(x.name)).map((x) => ({ name: str(x.name)!, team: str(x.team), avatar: str(x.avatar) }))
      : undefined,
  };
  return { view, version: num(j.version) };
}
