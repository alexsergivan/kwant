// The host remote's view (pa4b): pa1's v1 state envelope with `viewer: "host"` around the quiz kit's
// `view(Viewer::Host)` (src/kits/quiz/view.rs QuizView + HostView). Values are copied or left out; nothing here works
// out a score, a rank or a count. The one rule the remote adds is which word the big Next button says (nextStep).

export type Phase = 'lobby' | 'question' | 'locked' | 'reveal' | 'leaderboard' | 'final';

export interface HostPlayer {
  id: string;
  name: string;
  team?: string;
  connected: boolean;
  removed: boolean;
  score: number;
}

export interface Pending {
  q: number;
  player: string;
  question: string;
  answer: string;
  accept: string[];
  guess: string;
}

export interface TeamRow {
  team: string;
  members: number;
  score: number;
  rank: number;
}

export interface RemoteView {
  version?: number;
  ended: boolean;
  title: string;
  teams: boolean;
  phase: Phase;
  round?: { number: number; of: number; title: string };
  question?: { q: number; number: number; of: number; text: string; points: number; time_limit_s: number };
  time?: { remaining_ms: number; paused: boolean };
  answered?: { count: number; of: number };
  /** The current question's answer, ahead of the reveal: its words and, for a choice, its letter. */
  answer?: { text: string; position?: number };
  /** Player ids who answered the current question. */
  answeredBy: string[];
  pending: Pending[];
  players: HostPlayer[];
  /** What the box says the host may do right now (start, next, pause, resume, adjust, remove, end, judge). */
  can: string[];
  teamBoard: TeamRow[];
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);
const list = (x: unknown): Obj[] => (Array.isArray(x) ? x.map(obj).filter((o): o is Obj => !!o) : []);
const strs = (x: unknown): string[] => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string') : []);
const PHASES: Phase[] = ['lobby', 'question', 'locked', 'reveal', 'leaderboard', 'final'];

/** One v1 host state (from host/signin, host/state, host/action, host/remove, host/end or a ws frame). */
export function remoteFrom(json: unknown): { view: RemoteView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  const v = obj(j?.view);
  if (!j || !pocket || !v) return undefined;
  // A big-screen session's state is not the host's (pa1b: screens only read view=screen).
  if (j.viewer !== undefined && j.viewer !== 'host') return undefined;
  const phase = PHASES.find((p) => p === v.phase);
  if (!phase) return undefined;
  const host = obj(v.host) ?? {};
  const round = obj(v.round);
  const q = obj(v.question);
  const time = obj(v.time);
  const answered = obj(v.answered);
  const now = obj(host.answer_now);
  const view: RemoteView = {
    ended: str(pocket.state) === 'ended',
    title: str(v.title) ?? str(pocket.title) ?? '',
    teams: v.teams === true,
    phase,
    round: round && num(round.number) !== undefined && num(round.of) !== undefined ? { number: num(round.number)!, of: num(round.of)!, title: str(round.title) ?? '' } : undefined,
    question:
      q && num(q.q) !== undefined && num(q.number) !== undefined && str(q.text) !== undefined
        ? { q: num(q.q)!, number: num(q.number)!, of: num(q.of) ?? 0, text: str(q.text)!, points: num(q.points) ?? 0, time_limit_s: num(q.time_limit_s) ?? 0 }
        : undefined,
    time: time && num(time.remaining_ms) !== undefined ? { remaining_ms: num(time.remaining_ms)!, paused: time.paused === true } : undefined,
    answered: answered && num(answered.count) !== undefined && num(answered.of) !== undefined ? { count: num(answered.count)!, of: num(answered.of)! } : undefined,
    answer: now && str(now.answer) !== undefined ? { text: str(now.answer)!, position: num(now.position) } : undefined,
    answeredBy: list(host.answers).map((a) => str(a.player)).filter((s): s is string => !!s),
    pending: list(host.pending)
      .filter((p) => num(p.q) !== undefined && str(p.player))
      .map((p) => ({ q: num(p.q)!, player: str(p.player)!, question: str(p.question) ?? '', answer: str(p.answer) ?? '', accept: strs(p.accept), guess: str(p.guess) ?? '' })),
    players: list(host.players)
      .filter((p) => str(p.id) && str(p.name) !== undefined)
      .map((p) => ({ id: str(p.id)!, name: str(p.name)!, team: str(p.team), connected: p.connected === true, removed: p.removed === true, score: num(p.score) ?? 0 })),
    can: strs(host.can),
    teamBoard: list(v.team_board)
      .filter((t) => str(t.team) && num(t.score) !== undefined)
      .map((t) => ({ team: str(t.team)!, members: num(t.members) ?? 0, score: num(t.score)!, rank: num(t.rank) ?? 0 })),
  };
  return { view, version: num(j.version) };
}

export const paused = (v: RemoteView): boolean => v.phase === 'question' && !!v.time?.paused;

export interface NextStep {
  /** The kit action the button sends. `next` from an open question also reveals (two steps, see RemoteApp.next). */
  action: 'start' | 'next' | 'resume';
  label: string;
  sub?: string;
  /** The box allows it now (host.can). */
  ok: boolean;
}

/**
 * The big button's words by phase (design.md section 5: Start, Reveal now, Next question, Show scores, Next round).
 * The view has no "a leaderboard comes next" flag, so after a reveal mid-quiz it says Next question and notes that
 * scores come first at a round's end (pa4b/impl.md, ask for Juno).
 */
export function nextStep(v: RemoteView): NextStep | undefined {
  if (v.ended || v.phase === 'final') return undefined;
  const can = (a: string) => v.can.includes(a);
  const n = v.question?.number ?? 0;
  const last = !!v.question && v.question.of > 0 && n >= v.question.of;
  if (paused(v)) return { action: 'resume', label: 'Resume', ok: can('resume') };
  switch (v.phase) {
    case 'lobby':
      return { action: 'start', label: 'Start', sub: 'question 1', ok: can('start') };
    case 'question':
      return { action: 'next', label: 'Reveal now', sub: 'closes answers, shows the answer', ok: can('next') };
    case 'locked':
      return { action: 'next', label: 'Reveal now', sub: 'shows the answer', ok: can('next') };
    case 'reveal':
      return last
        ? { action: 'next', label: 'Show winners', ok: can('next') }
        : { action: 'next', label: 'Next question', sub: `question ${n + 1}, or scores if the round is over`, ok: can('next') };
    case 'leaderboard': {
      // Between rounds the view has no question, only the round that just ended.
      const r = v.round;
      return r && r.number >= r.of
        ? { action: 'next', label: 'Show winners', ok: can('next') }
        : { action: 'next', label: 'Next round', sub: r ? `round ${r.number + 1}` : undefined, ok: can('next') };
    }
  }
  return undefined;
}
