// The guest side of pa1's guest port, Pocket API v1 (pa1/impl.md): GET /p/<code> serves this page and
// /api/pocket/<code>/* is all it talks to. state (?since=<version> gives 204 when nothing changed; the kp_<code> cookie
// is the seat, 401 signed_out without one), info (public facts, before joining), join {name, avatar, team}, rejoin
// {name} (by name, for "That's me"), action {name, body}, ws (`{type:"state", ...}` pushes, `signed_out`, `gone`, `removed`;
// without WebSocket the page polls state every 2 s). Same origin only: the CSP allows nothing else.
import type { Answer, BoardRow, GuestView, Question, Retention, Reveal, Team } from './view';

export interface Env {
  fetch: typeof fetch;
  WebSocket?: typeof WebSocket;
  setTimeout: (f: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
  location: { pathname: string; protocol: string; host: string };
}

/**
 * A failed call keeps what pa1 and the kits say about it: `error` (code), `message` (plain words to show as is),
 * `tries_left` (wrong PIN), `until` (PIN lock), `retry_at` (a kit's own slow_down, the front desk's 429) and `limit` (the
 * queue's 429 take limit: "phone" or "network", which picks the line).
 */
/** `body`: the whole error JSON, for a kit that reads more of it (pa13b's slot_taken carries fresh slots). */
export type Fail = { ok: false; status: number; error?: string; message?: string; tries_left?: number; until?: string; retry_at?: string; limit?: string; body?: Record<string, unknown> };
export type Reply<V = GuestView> = { ok: true; view?: V } | Fail;
/** Reads one v1 state into the page's own view (the quiz's `fromState`, Santa's `santaFrom`). */
export type Parse<V> = (json: unknown, memo?: QuizMemo) => { view: V; version?: number; missing?: boolean } | undefined;

/**
 * What the quiz phone keeps from the public `info` (the kit's lobby): the teams with their colours and the team size. The
 * engine's guest view names a team but not its colour, so the colour comes from here (the TV and the join screen use the
 * same `lobby_info.teams[].color`).
 */
export interface QuizMemo {
  teams?: Team[];
  team_max?: number;
  rounds?: number;
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);

/** pa1's `retention.describe()`: after_end {days}, after_date {fields, days}, at_end. Anything else: not shown. */
export function retentionFrom(x: unknown): Retention | undefined {
  const r = obj(x);
  const kind = str(r?.kind);
  if (kind === 'at_end') return { kind };
  if ((kind === 'after_end' || kind === 'after_date') && num(r?.days) !== undefined) return { kind, days: num(r?.days) };
  return undefined;
}

const LETTERS = 'ABCDEFGH';
const arr = (x: unknown): Obj[] => (Array.isArray(x) ? x.map(obj).filter((o): o is Obj => !!o) : []);

/** The quiz engine's verdict (src/kits/quiz/view.rs ResultView) as the phone's reveal outcome. */
const OUTCOME: Record<string, Reveal['outcome']> = { correct: 'right', partial: 'near', wrong: 'wrong', no_answer: 'none', waiting_for_judge: 'judging' };

/**
 * The quiz engine's own guest view (src/kits/quiz/view.rs `QuizView`, what `src/pocket/quiz_kit.rs` sends as `view`) as the
 * phone's GuestView. Every value is copied from the engine view; the only conversions are units and names (milliseconds to
 * seconds, an option's position to its letter, the source address to its host). A team's colour comes from `memo` (the
 * public lobby's `teams[].color`); `missing` says a team on screen has none yet, so the page asks `info` again.
 */
export function quizPhone(kv: Obj, memo: QuizMemo = {}): { view: Partial<GuestView>; missing: boolean } {
  let missing = false;
  const colour = (team: string | undefined): number => {
    if (!team) return 0;
    const c = memo.teams?.find((t) => t.name === team)?.color;
    if (c === undefined) missing = true;
    return c ?? 0;
  };
  const engine = str(kv.phase) ?? 'lobby';
  const round = obj(kv.round);
  const q = obj(kv.question);
  const time = obj(kv.time);
  const kme = obj(kv.me);
  const result = obj(kme?.result);
  const myTeam = str(kme?.team);
  const teamsOn = kv.teams === true;
  const options = Array.isArray(q?.options) ? (q.options as unknown[]).map((o) => String(o)) : undefined;
  const kind = str(q?.kind);
  const pic = obj(q?.picture);
  const mine = str(kme?.my_answer);
  const answer: Answer | undefined =
    kme?.answered !== true || mine === undefined
      ? undefined
      : kind === 'choice' && options && options.indexOf(mine) >= 0
        ? { choice: LETTERS[options.indexOf(mine)] }
        : kind === 'number' && Number.isFinite(Number(mine))
          ? { number: Number(mine) }
          : { text: mine };
  const question: Question | undefined = q
    ? {
        id: String(num(q.q) ?? ''),
        round: num(round?.number) ?? 0,
        round_name: str(round?.title) || undefined,
        rounds: num(round?.of),
        n: num(q.number) ?? 0,
        of: num(q.of) ?? 0,
        kind: kind === 'choice' && pic ? 'picture' : kind === 'choice' || kind === 'number' ? kind : 'text',
        text: str(q.text) ?? '',
        options,
        picture: pic ? { url: str(pic.url) ?? '', alt: str(pic.alt), caption: str(pic.credit) } : undefined,
        // Only an open, running question has time left; locked (time's up) and paused show none ticking.
        secs_left: engine === 'question' && time ? Math.ceil((num(time.remaining_ms) ?? 0) / 1000) : 0,
        total_secs: num(q.time_limit_s) ?? 0,
        answer,
        // The page's own number line promises a closest-wins bonus the engine doesn't have: a number counts when it's right.
        hint: kind === 'number' ? 'Send the number. Your first answer counts.' : undefined,
      }
    : undefined;
  if (question && !question.answer) delete question.answer;
  if (question && !question.hint) delete question.hint;
  const rv = obj(kv.reveal);
  let reveal: GuestView['reveal'];
  if (engine === 'reveal' && rv && question) {
    const pos = num(rv.position);
    let source = '';
    try {
      source = new URL(str(rv.source_url) ?? '').hostname.replace(/^www\./, '');
    } catch {
      /* no address */
    }
    source = [source, str(rv.source_quote)].filter(Boolean).join(': ');
    reveal = {
      outcome: OUTCOME[str(result?.verdict) ?? ''] ?? 'none',
      points: num(result?.points) ?? 0,
      yours: mine,
      answer: str(rv.answer) ?? '',
      answer_letter: pos !== undefined ? LETTERS[pos] : undefined,
      source: source || undefined,
      question,
    };
  }
  const board: BoardRow[] = teamsOn
    ? arr(kv.team_board).map((t) => ({ rank: num(t.rank) ?? 0, name: str(t.team) ?? '', color: colour(str(t.team)), score: num(t.score) ?? 0, me: str(t.team) === myTeam }))
    : arr(kv.leaderboard).map((r, i) => ({ rank: num(r.rank) ?? 0, name: str(r.name) ?? '', color: i % 8, score: num(r.score) ?? 0, me: r.me === true }));
  const top = board[0];
  const tiedTop = teamsOn ? arr(kv.team_board)[0]?.tied === true : arr(kv.leaderboard)[0]?.tied === true;
  const phase: GuestView['phase'] =
    engine === 'question' && time?.paused === true ? 'paused' : engine === 'question' || engine === 'locked' ? 'question' : engine === 'reveal' ? 'reveal' : engine === 'leaderboard' ? 'between' : engine === 'final' ? 'ended' : 'lobby';
  const view: Partial<GuestView> = {
    kit: 'quiz',
    noun: 'quiz',
    title: str(kv.title) ?? '',
    phase,
    line: str(kv.line),
    rounds: num(round?.of) ?? memo.rounds,
    team_max: memo.team_max,
    players: engine === 'lobby' ? arr(kv.lobby).map((p) => ({ name: str(p.name) ?? '', avatar: str(p.avatar) ?? '' })) : undefined,
    question: phase === 'question' || phase === 'paused' ? question : undefined,
    reveal,
    between: phase === 'between' ? { after_round: num(round?.number) ?? 0, rounds: num(round?.of) ?? 0, board } : undefined,
    ended: phase === 'ended' ? (top && !tiedTop ? { winner: top.name, points: top.score } : {}) : undefined,
    me: kme
      ? ({ score: num(kme.score) ?? 0, ...(myTeam ? { team_name: myTeam, team_color: colour(myTeam) } : {}) } as GuestView['me'])
      : undefined,
  };
  for (const k of Object.keys(view) as (keyof GuestView)[]) if (view[k] === undefined) delete view[k];
  return { view, missing };
}

/**
 * One v1 state (from state, join, rejoin, action, info, a ws `state` frame or the boot JSON) as the page's GuestView.
 * The kit's `view` gives the game (phase, question, reveal, between, teams...); `pocket` and `me` give the facts pa1 owns
 * and win over the kit for those. Nothing is worked out here: values are copied or left out. The quiz sends its engine's
 * own view (`viewer: "guest"`), which `quizPhone` reads; its public `info` (the lobby, with the teams) passes through.
 */
export function fromState(json: unknown, memo?: QuizMemo): { view: GuestView; version?: number; missing?: boolean } | undefined {
  const j = obj(json);
  if (!j) return undefined;
  const pocket = obj(j.pocket);
  if (!pocket) return undefined;
  const raw = obj(j.view) ?? {};
  const engine = str(pocket.kit) === 'quiz' && typeof raw.viewer === 'string' ? quizPhone(raw, memo) : undefined;
  const kv = (engine?.view ?? raw) as Partial<GuestView>;
  const me = obj(j.me);
  const kme = obj(kv.me) as Partial<NonNullable<GuestView['me']>> | undefined;
  const ended = str(pocket.state) === 'ended';
  const expires = str(pocket.expires_at);
  const most = num(pocket.max_players);
  const players = num(pocket.players);
  // No seat and no room left: the full screen (a comparison only; the page shows pa1's own numbers).
  const full = !me && most !== undefined && players !== undefined && players >= most;
  const view: GuestView = {
    ...kv,
    v: 1,
    kit: str(pocket.kit) ?? kv.kit ?? '',
    title: kv.title ?? str(pocket.title) ?? '',
    // With no seat the kit's phase never decides the screen: an unseated guest is drawn the join screen.
    phase: ended ? 'ended' : full ? 'full' : me ? (kv.phase ?? 'lobby') : 'join',
    max_players: most ?? kv.max_players,
    player_count: kv.player_count ?? players,
    retention: retentionFrom(pocket.retention) ?? kv.retention,
    notice: str(pocket.notice) ?? kv.notice,
    first_names_only: pocket.first_names_only === true || kv.first_names_only === true,
    deletes_on: kv.deletes_on ?? (expires && /^\d{4}-\d{2}-\d{2}/.test(expires) ? expires.slice(0, 10) : undefined),
    me: me
      ? {
          score: 0,
          ...kme,
          name: str(me.name) ?? kme?.name ?? '',
          avatar: str(me.avatar) ?? kme?.avatar ?? '',
          team_name: kme?.team_name ?? str(me.team),
        }
      : undefined,
  };
  if (!me) delete view.me;
  if (!view.notice) delete view.notice;
  if (!view.first_names_only) delete view.first_names_only;
  return { view, version: num(j.version), missing: engine?.missing || undefined };
}

/** The pocket code is the path segment after /p/. */
export const codeFrom = (pathname: string): string => decodeURIComponent(pathname.split('/')[2] || '');

/** Seconds between reconnect tries: 2, 5, 10, 20, then every 20 (design.md 2d). */
export const BACKOFF = [2, 5, 10, 20];
export const backoff = (tries: number): number => BACKOFF[Math.min(tries, BACKOFF.length - 1)];
export const POLL_MS = 2000;

export class Api<V = GuestView> {
  readonly base: string;
  /** The last state version seen, for `state?since=`. */
  version: number | undefined;
  /** The quiz's public facts from the last `info` (team colours); see QuizMemo. */
  memo: QuizMemo = {};
  /** The last quiz state named a team whose colour isn't known yet: ask `info` again. */
  missing = false;
  /** The last state's JSON, so it can be read again once `info` brings the colours (state?since= answers 204 then). */
  private last: unknown;
  /** Where the view comes from: the guest's own `state` and `ws`, or the big screen's `host/state?view=screen` and `ws?view=screen`. */
  statePath = 'state';
  wsPath = 'ws';
  constructor(private env: Env, readonly code: string, private parse: Parse<V> = fromState as unknown as Parse<V>) {
    this.base = `/api/pocket/${encodeURIComponent(code)}`;
  }

  async call(path: string, body?: unknown): Promise<Reply<V>> {
    let res: Response;
    try {
      res = await this.env.fetch(`${this.base}/${path}`, {
        method: body === undefined ? 'GET' : 'POST',
        credentials: 'same-origin',
        cache: 'no-store',
        headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch {
      return { ok: false, status: 0 };
    }
    if (res.status === 204) return { ok: true };
    let json: unknown = {};
    try {
      json = await res.json();
    } catch {
      /* empty or not JSON */
    }
    if (!res.ok) {
      const e = obj(json);
      return { ok: false, status: res.status, error: str(e?.error), message: str(e?.message), tries_left: num(e?.tries_left), until: str(e?.until), retry_at: str(e?.retry_at), limit: str(e?.limit), body: e };
    }
    return { ok: true, view: this.take(json) };
  }

  /**
   * Reads a v1 state and remembers its version. A state older than the last one taken gives no view and leaves the version
   * where it is: a slow HTTP reply (an answer, a join) can land after a newer push, and the page must not go back to it.
   */
  take(json: unknown): V | undefined {
    const s = this.parse(json, this.memo);
    if (s?.version !== undefined && this.version !== undefined && s.version < this.version) return undefined;
    if (s?.version !== undefined) this.version = s.version;
    const v = s?.view as (GuestView & { team_max?: number; rounds?: number }) | undefined;
    if (Array.isArray(v?.teams)) this.memo = { teams: v.teams, team_max: v.team_max, rounds: v.rounds };
    else if (s) {
      this.missing = s.missing === true;
      this.last = json;
    }
    return s?.view;
  }

  /** The last state read again, with what `memo` knows now (the team colours from a later `info`). */
  retake(): V | undefined {
    return this.last === undefined ? undefined : this.parse(this.last, this.memo)?.view;
  }

  /** Full state, or with `changed` only what is newer than the last version (204 gives no view). */
  state = (changed = false) =>
    this.call(changed && this.version !== undefined ? `${this.statePath}${this.statePath.includes('?') ? '&' : '?'}since=${this.version}` : this.statePath);
  info = () => this.call('info');
  rejoin = (name: string) => this.call('rejoin', { name });
  join = (body: { name: string; avatar: string; team?: string; pin?: string }) => this.call('join', body);
  /**
   * The quiz engine's answer (src/kits/quiz/mod.rs): `{q: <question number>, value}`, where value is the option's position
   * for a choice (A is 0), the text for a text question and the number for a number question.
   */
  answer = (q: string, answer: Answer) =>
    this.call('action', { name: 'answer', body: { q: Number(q), value: 'choice' in answer ? LETTERS.indexOf(answer.choice) : 'text' in answer ? answer.text : answer.number } });
  /** A kit action: `{name, body}` (body left out when there is none). */
  act = (name: string, body?: unknown) => this.call('action', body === undefined ? { name } : { name, body });
}

/**
 * Keeps one live link to the box: a WebSocket when the browser has one, else `state` every 2 s. When the box stops
 * answering it calls `away(nextSecs, tries)` and retries on the BACKOFF schedule until `state` answers again.
 */
export class Live<V = GuestView> {
  private ws: WebSocket | null = null;
  private timer = 0;
  private tries = 0;
  private wsFailed = 0;
  private stopped = true;
  isAway = false;
  /** How often `state` is asked without a socket (the front desk asks every 15 s). */
  pollMs = POLL_MS;

  constructor(
    private env: Env,
    private api: Api<V>,
    private on: {
      view: (v: V) => void;
      away: (nextSecs: number, tries: number) => void;
      back: () => void;
      gone: (status: number, error?: string) => void;
    },
  ) {}

  start(): void {
    this.stopped = false;
    this.open();
  }

  stop(): void {
    this.stopped = true;
    this.env.clearTimeout(this.timer);
    this.ws?.close();
    this.ws = null;
  }

  /**
   * "Try now", or the phone woke / came back online: skip the wait and ask `state`. A socket that may have died quietly
   * (Wi-Fi to cellular, sleep) is dropped first; `retry` opens a fresh one once `state` answers.
   */
  now(): void {
    if (this.stopped) return;
    this.env.clearTimeout(this.timer);
    const old = this.ws;
    this.ws = null;
    if (old) {
      old.onclose = null;
      old.onmessage = null;
      old.close();
    }
    void this.retry();
  }

  private open(): void {
    if (this.stopped) return;
    const WS = this.env.WebSocket;
    if (!WS || this.wsFailed >= 2) return this.poll();
    const { protocol, host } = this.env.location;
    let opened = false;
    let ws: WebSocket;
    try {
      ws = new WS(`${protocol === 'https:' ? 'wss' : 'ws'}://${host}${this.api.base}/${this.api.wsPath}`);
    } catch {
      this.wsFailed = 2;
      return this.poll();
    }
    this.ws = ws;
    ws.onopen = () => {
      opened = true;
      this.wsFailed = 0;
    };
    ws.onmessage = (e) => {
      let msg: Obj | undefined;
      try {
        msg = obj(JSON.parse(String(e.data)));
      } catch {
        return; // ignore a bad frame
      }
      if (msg?.type === 'signed_out' || msg?.type === 'gone' || msg?.type === 'removed') {
        this.stop();
        if (msg.type === 'removed') this.on.gone(403, 'removed');
        else this.on.gone(msg.type === 'gone' ? 404 : 401, msg.type === 'gone' ? 'not_found' : 'signed_out');
        return;
      }
      const v = msg?.type === 'state' ? this.api.take(msg) : undefined;
      if (v) this.on.view(v);
    };
    ws.onclose = () => {
      if (this.ws !== ws || this.stopped) return;
      this.ws = null;
      if (!opened) this.wsFailed++;
      // A close can be the box going away or a proxy that drops sockets: `state` tells which.
      void this.retry();
    };
  }

  private poll(): void {
    if (this.stopped) return;
    this.timer = this.env.setTimeout(async () => {
      const r = await this.api.state(true);
      if (this.stopped) return;
      if (r.ok) {
        if (r.view) this.on.view(r.view);
        this.poll();
      } else if (r.status === 0 || r.status >= 500) this.lost();
      else this.on.gone(r.status, r.error);
    }, this.pollMs);
  }

  private async retry(): Promise<void> {
    if (this.stopped) return;
    const r = await this.api.state();
    if (this.stopped) return;
    if (r.ok) {
      this.tries = 0;
      if (this.isAway) {
        this.isAway = false;
        this.on.back();
      }
      if (r.view) this.on.view(r.view);
      this.open();
    } else if (r.status === 0 || r.status >= 500) this.lost();
    else this.on.gone(r.status, r.error);
  }

  private lost(): void {
    const secs = backoff(this.tries);
    this.tries++;
    this.isAway = true;
    this.on.away(secs, this.tries);
    this.timer = this.env.setTimeout(() => void this.retry(), secs * 1000);
  }
}

/**
 * The big screen's one sign-in: trades the `#screen=<token>` from its link for the HttpOnly `kph_<code>` cookie
 * (pa1 `host/signin`). A one-time screen link is spent only as `{token}` (src/pocket/guest.rs host_signin); `{key}` is
 * the host's long key and a link's token sent as `key` gets 401. Shared by the quiz TV, the wall TV and the queue TV.
 */
export const screenSignIn = <V>(api: Api<V>, token: string): Promise<Reply<V>> => api.call('host/signin', { token });
