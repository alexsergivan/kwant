// The guest side of pa1's guest port: GET /p/<code> serves this page, /api/pocket/<code>/* is all it talks to.
// state (this guest's view), join, rejoin (guest-key cookie, HttpOnly), action (a named action + JSON body), ws (state
// pushes; without WebSocket the page polls state every 2 s). Same origin only: the CSP allows nothing else.
import type { Answer, GuestView } from './view';

export interface Env {
  fetch: typeof fetch;
  WebSocket?: typeof WebSocket;
  setTimeout: (f: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
  location: { pathname: string; protocol: string; host: string };
}

export type Reply = { ok: true; view?: GuestView } | { ok: false; status: number; error?: string };

/** The pocket code is the path segment after /p/. */
export const codeFrom = (pathname: string): string => decodeURIComponent(pathname.split('/')[2] || '');

/** Seconds between reconnect tries: 2, 5, 10, 20, then every 20 (design.md 2d). */
export const BACKOFF = [2, 5, 10, 20];
export const backoff = (tries: number): number => BACKOFF[Math.min(tries, BACKOFF.length - 1)];
export const POLL_MS = 2000;

export class Api {
  readonly base: string;
  constructor(private env: Env, readonly code: string) {
    this.base = `/api/pocket/${encodeURIComponent(code)}`;
  }

  async call(path: string, body?: unknown): Promise<Reply> {
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
    let json: { view?: GuestView; error?: string } & Partial<GuestView> = {};
    try {
      json = await res.json();
    } catch {
      /* empty or not JSON */
    }
    if (!res.ok) return { ok: false, status: res.status, error: json.error };
    const view = json.view ?? (json.phase ? (json as GuestView) : undefined);
    return { ok: true, view };
  }

  state = () => this.call('state');
  rejoin = () => this.call('rejoin', {});
  join = (body: { name: string; avatar: string; team?: string; new_team?: string }) => this.call('join', body);
  answer = (q: string, answer: Answer) => this.call('action', { name: 'answer', q, body: answer });
}

/**
 * Keeps one live link to the box: a WebSocket when the browser has one, else `state` every 2 s. When the box stops
 * answering it calls `away(nextSecs, tries)` and retries on the BACKOFF schedule until `state` answers again.
 */
export class Live {
  private ws: WebSocket | null = null;
  private timer = 0;
  private tries = 0;
  private wsFailed = 0;
  private stopped = false;
  isAway = false;

  constructor(
    private env: Env,
    private api: Api,
    private on: {
      view: (v: GuestView) => void;
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

  /** "Try now": skip the wait. */
  now(): void {
    this.env.clearTimeout(this.timer);
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
      ws = new WS(`${protocol === 'https:' ? 'wss' : 'ws'}://${host}${this.api.base}/ws`);
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
      try {
        const msg = JSON.parse(String(e.data));
        const v: GuestView | undefined = msg.view ?? (msg.phase ? msg : undefined);
        if (v) this.on.view(v);
      } catch {
        /* ignore a bad frame */
      }
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
      const r = await this.api.state();
      if (this.stopped) return;
      if (r.ok) {
        if (r.view) this.on.view(r.view);
        this.poll();
      } else if (r.status === 0 || r.status >= 500) this.lost();
      else this.on.gone(r.status, r.error);
    }, POLL_MS);
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
