// The guest side of pa1's guest port, Pocket API v1 (pa1/impl.md): GET /p/<code> serves this page and
// /api/pocket/<code>/* is all it talks to. state (?since=<version> gives 204 when nothing changed; the kp_<code> cookie
// is the seat, 401 signed_out without one), info (public facts, before joining), join {name, avatar, team}, rejoin
// {name} (by name, for "That's me"), action {name, body}, ws (`{type:"state", ...}` pushes, `signed_out`, `gone`, `removed`;
// without WebSocket the page polls state every 2 s). Same origin only: the CSP allows nothing else.
import type { Answer, GuestView, Retention } from './view';

export interface Env {
  fetch: typeof fetch;
  WebSocket?: typeof WebSocket;
  setTimeout: (f: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
  location: { pathname: string; protocol: string; host: string };
}

/**
 * A failed call keeps what pa1 and the kits say about it: `error` (code), `message` (plain words to show as is),
 * `tries_left` (wrong PIN), `until` (PIN lock) and `retry_at` (a kit's own slow_down, the front desk's 429).
 */
export type Fail = { ok: false; status: number; error?: string; message?: string; tries_left?: number; until?: string; retry_at?: string };
export type Reply<V = GuestView> = { ok: true; view?: V } | Fail;
/** Reads one v1 state into the page's own view (the quiz's `fromState`, Santa's `santaFrom`). */
export type Parse<V> = (json: unknown) => { view: V; version?: number } | undefined;

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

/**
 * One v1 state (from state, join, rejoin, action, info, a ws `state` frame or the boot JSON) as the page's GuestView.
 * The kit's `view` gives the game (phase, question, reveal, between, teams...); `pocket` and `me` give the facts pa1 owns
 * and win over the kit for those. Nothing is worked out here: values are copied or left out.
 */
export function fromState(json: unknown): { view: GuestView; version?: number } | undefined {
  const j = obj(json);
  if (!j) return undefined;
  const pocket = obj(j.pocket);
  if (!pocket) return undefined;
  const kv = (obj(j.view) ?? {}) as Partial<GuestView>;
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
  return { view, version: num(j.version) };
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
      return { ok: false, status: res.status, error: str(e?.error), message: str(e?.message), tries_left: num(e?.tries_left), until: str(e?.until), retry_at: str(e?.retry_at) };
    }
    return { ok: true, view: this.take(json) };
  }

  /** Reads a v1 state and remembers its version. */
  take(json: unknown): V | undefined {
    const s = this.parse(json);
    if (s?.version !== undefined) this.version = s.version;
    return s?.view;
  }

  /** Full state, or with `changed` only what is newer than the last version (204 gives no view). */
  state = (changed = false) =>
    this.call(changed && this.version !== undefined ? `${this.statePath}${this.statePath.includes('?') ? '&' : '?'}since=${this.version}` : this.statePath);
  info = () => this.call('info');
  rejoin = (name: string) => this.call('rejoin', { name });
  join = (body: { name: string; avatar: string; team?: string; pin?: string }) => this.call('join', body);
  answer = (q: string, answer: Answer) => this.call('action', { name: 'answer', body: { q, ...answer } });
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
 * (pa1 `host/signin`). Today's body is `{key}`; pa1b's one-time screen links take `{token}` ("Pocket API v1.1: links"),
 * and switching is this one line.
 */
export const screenSignIn = <V>(api: Api<V>, token: string): Promise<Reply<V>> => api.call('host/signin', { key: token });
