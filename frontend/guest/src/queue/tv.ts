// The Queue ticket on the big screen (pa11b, queue-design.md section 4), on the pa5 frame: the same top bar, QR paper and
// foot as the quiz screen. Read-only: it signs in once with its link's token, then only reads host/state?view=screen and
// ws?view=screen. Numbers only, never a name (the box sends none). S1 open, S2 paused, Closed (S2's layout, QR hidden).
// This file reads the screen view itself and shares nothing with the phone's queue chunk, so it never counts toward the
// phone budget.
import { Api, Live, backoff, screenSignIn } from '../api';
import { Kid, add, h } from '../dom';
import { qrPath } from '../screen/qr';
import type { ScreenEnv } from '../screen/app';

export interface QueueTvView {
  state: 'open' | 'paused' | 'closed' | 'ended';
  title: string;
  line: string;
  now_serving: number | null;
  next: number[];
  waiting: number;
  avg_minutes: number | null;
  pause_text: string;
  close_text: string;
  hours_until?: string;
  now?: string;
}

type Shown = 'loading' | 'open' | 'paused' | 'closed' | 'away' | 'ended' | 'signed-out' | 'missing';

const NS = 'http://www.w3.org/2000/svg';
export const SWAP_MS = 180;
const STATES = ['open', 'paused', 'closed', 'ended'];
const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;

const obj = (x: unknown): Record<string, unknown> | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Record<string, unknown>) : undefined);
const str = (x: unknown): string => (typeof x === 'string' ? x.trim() : '');
const whole = (x: unknown): number | null => (typeof x === 'number' && Number.isInteger(x) && x >= 0 ? x : null);

export function queueTvFrom(json: unknown): { view: QueueTvView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  const v = obj(j?.view);
  if (!j || !pocket || !v) return undefined;
  const s = str(v.state);
  const ended = str(pocket.state) === 'ended' || s === 'ended';
  const avg = whole(v.avg_minutes);
  const until = str(v.hours_until);
  const view: QueueTvView = {
    state: ended ? 'ended' : STATES.includes(s) ? (s as QueueTvView['state']) : 'open',
    title: str(v.title) || str(pocket.title),
    line: str(v.line),
    now_serving: whole(v.now_serving) || null,
    next: (Array.isArray(v.next) ? v.next : []).map(whole).filter((n): n is number => !!n).slice(0, 5),
    waiting: whole(v.waiting) ?? 0,
    avg_minutes: avg && avg > 0 ? avg : null,
    pause_text: str(v.pause_text),
    close_text: str(v.close_text),
    hours_until: HHMM.test(until) ? until : undefined,
    now: str(v.now) || undefined,
  };
  const version = typeof j.version === 'number' && Number.isFinite(j.version) ? j.version : undefined;
  return { view, version };
}

/** "13:00" → "1 PM", "09:30" → "9:30 AM" (the screen's own copy of the phone's; no shared module). */
export function untilText(hhmm: string): string {
  const [hh, mm] = hhmm.split(':').map(Number);
  const h12 = hh % 12 || 12;
  return `${h12}${mm ? `:${String(mm).padStart(2, '0')}` : ''} ${hh < 12 ? 'AM' : 'PM'}`;
}

/** The box's `now` as the wall clock reads it here, "10:41". */
function hm(iso?: string): string {
  const d = iso ? new Date(iso) : null;
  return d && !Number.isNaN(d.getTime()) ? `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}` : '';
}

export class QueueTV {
  private api: Api<QueueTvView>;
  private live: Live<QueueTvView>;
  view: QueueTvView | undefined;
  private stage: HTMLElement;
  private said: HTMLElement;
  private away = { secs: 0, tries: 0 };
  /** The number on show, so a new call (and only that) swaps with the 180 ms lift and the chime. */
  private serving: number | null | undefined;
  private fresh = false;
  private swapTimer = 0;
  private audio: AudioContext | undefined;

  constructor(root: HTMLElement, private env: ScreenEnv, readonly code: string, private token?: string) {
    this.api = new Api<QueueTvView>(env, code, queueTvFrom);
    this.api.statePath = 'host/state?view=screen';
    this.api.wsPath = 'ws?view=screen';
    this.live = new Live<QueueTvView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => {
        this.away = { secs, tries };
        this.draw('away');
      },
      back: () => this.view && this.show(this.view),
      gone: (status, error) => this.gone(status, error),
    });
    this.stage = h('div', { class: 'tv qtv', 'data-screen': 'loading' });
    this.said = h('p', { class: 'vh', 'aria-live': 'polite' });
    root.replaceChildren(h('main', { class: 'tvstage', 'aria-label': 'Big screen: queue' }, this.stage), this.said);
    document.documentElement.classList.add('tvpage');
  }

  async start(_boot?: unknown): Promise<void> {
    this.draw('loading');
    let tries = 0;
    for (;;) {
      const r = this.token ? await screenSignIn(this.api, this.token) : await this.api.state();
      if (r.ok) {
        this.token = undefined;
        if (r.view) this.show(r.view);
        this.live.start();
        return;
      }
      if (r.status !== 0 && r.status !== 429 && r.status < 500) return this.gone(r.status, r.error);
      this.away = { secs: backoff(tries), tries: ++tries };
      this.draw('away');
      await new Promise<void>((res) => this.env.setTimeout(res, this.away.secs * 1000));
    }
  }

  wake(): void {
    this.live.now();
  }

  stop(): void {
    this.live.stop();
    this.env.clearTimeout(this.swapTimer);
  }

  fit(w: number, hh: number): void {
    this.stage.style.setProperty('--s', String(Math.min(w / 1920, hh / 1080)));
  }

  private gone(status: number, error?: string): void {
    this.stop();
    if (status === 410 || error === 'ended') return this.draw('ended');
    this.draw(status === 404 ? 'missing' : 'signed-out');
  }

  show(v: QueueTvView): void {
    this.view = v;
    if (v.state === 'ended') return this.gone(410);
    const called = this.serving !== undefined && v.now_serving != null && v.now_serving !== this.serving;
    this.serving = v.now_serving;
    if (called) {
      this.said.textContent = `Now serving ${v.now_serving}`;
      this.chime();
      if (!this.env.still?.()) {
        this.fresh = true;
        this.env.clearTimeout(this.swapTimer);
        this.swapTimer = this.env.setTimeout(() => {
          this.fresh = false;
          this.stage.querySelector('.nowbox b')?.classList.remove('swap');
        }, SWAP_MS);
      }
    }
    this.draw(v.state);
  }

  /** The phone's two-note chime, once, if this browser lets a page play sound without a tap (many TVs do; if not, silent). */
  private chime(): void {
    try {
      const AC = (window as unknown as { AudioContext?: typeof AudioContext }).AudioContext;
      if (!AC) return;
      this.audio ??= new AC();
      const a = this.audio;
      if (a.state !== 'running') return void a.resume?.().catch(() => undefined);
      const t = a.currentTime;
      [880, 1320].forEach((f, i) => {
        const o = a.createOscillator();
        const g = a.createGain();
        o.frequency.value = f;
        g.gain.setValueAtTime(0.0001, t + i * 0.18);
        g.gain.exponentialRampToValueAtTime(0.25, t + i * 0.18 + 0.02);
        g.gain.exponentialRampToValueAtTime(0.0001, t + i * 0.18 + 0.4);
        o.connect(g).connect(a.destination);
        o.start(t + i * 0.18);
        o.stop(t + i * 0.18 + 0.45);
      });
    } catch {
      /* no sound on this screen */
    }
  }

  // ---------- drawing ----------

  private link(): string {
    return `${this.env.origin}/p/${encodeURIComponent(this.code)}`;
  }

  private qr(): SVGSVGElement {
    const link = this.link();
    const { size, d } = qrPath(link);
    const qr = document.createElementNS(NS, 'svg');
    qr.setAttribute('viewBox', `0 0 ${size} ${size}`);
    qr.setAttribute('class', 'qqr');
    qr.setAttribute('shape-rendering', 'crispEdges');
    qr.setAttribute('role', 'img');
    qr.setAttribute('aria-label', `QR code to take a ticket: ${link}`);
    const path = document.createElementNS(NS, 'path');
    path.setAttribute('d', d);
    qr.append(path);
    return qr;
  }

  private top(): HTMLElement {
    const v = this.view;
    const right = [v?.hours_until && v.state !== 'closed' ? `open until ${untilText(v.hours_until)}` : '', hm(v?.now)].filter(Boolean).join(' · ');
    return h('header', { class: 'tvtop' }, h('p', null, h('b', null, v?.title || 'Queue')), right ? h('p', { class: 'tvright' }, right) : null);
  }

  private foot(): HTMLElement {
    return h('footer', { class: 'tvfoot' }, h('p', null, 'No app, no sign-up. Numbers only on this screen.'), h('p', { class: 'made' }, 'made with kwant'));
  }

  private calm(title: string, line: string, extra: Kid[] = []): Kid {
    return [this.view?.title ? this.top() : null, h('section', { class: 'tvcalm' }, h('h1', { class: 'qbig' }, title), h('p', { class: 'tvsoft' }, line), ...extra), h('footer', { class: 'tvfoot' }, null, h('p', { class: 'made' }, 'made with kwant'))];
  }

  private queue(s: 'open' | 'paused' | 'closed'): Kid {
    const v = this.view!;
    const box =
      s === 'open'
        ? h(
            'section',
            { class: 'nowbox', 'aria-label': 'Now serving' },
            h('small', null, 'Now serving'),
            h('b', { class: this.fresh ? 'swap' : null }, v.now_serving != null ? String(v.now_serving) : '—'),
            v.line ? h('em', null, v.line) : null,
          )
        : s === 'paused'
          ? h('section', { class: 'nowbox paused' }, h('small', null, 'Paused'), h('b', null, v.pause_text || "We'll be right back"), h('em', null, 'Everyone keeps their place'))
          : h('section', { class: 'nowbox paused closed' }, h('small', null, 'Closed for today'), h('b', null, v.close_text || "Sorry, we've closed for today. Thanks for waiting, and see you next time!"));
    const next = v.next.slice(0, 3);
    const side =
      s === 'closed'
        ? null
        : h(
            'div',
            { class: 'side2' },
            next.length
              ? h(
                  'section',
                  { class: 'nextn' },
                  h('h2', { class: 'lab' }, s === 'paused' ? "Next when we're back" : 'Next'),
                  h('ol', { class: 'row' }, next.map((n) => h('li', null, String(n)))),
                )
              : null,
            h(
              'div',
              { class: 'joinq' },
              h('span', { class: 'qr' }, this.qr()),
              s === 'paused'
                ? h('div', null, h('b', null, 'See the queue'), h('span', null, 'Scan with your phone.', h('br'), "New tickets open when we're back."))
                : h(
                    'div',
                    null,
                    h('b', null, 'Take a ticket'),
                    h('span', null, 'Scan with your phone', h('br'), `${v.waiting.toLocaleString('en-US')} waiting${v.avg_minutes ? ` · ~${v.avg_minutes} min each` : ''}`),
                  ),
            ),
          );
    return [this.top(), box, side, this.foot()];
  }

  private draw(s: Shown): void {
    this.stage.setAttribute('data-screen', s);
    let body: Kid;
    switch (s) {
      case 'open':
      case 'paused':
      case 'closed':
        body = this.queue(s);
        break;
      case 'away':
        body = this.calm("The host's computer is resting", 'This screen reconnects by itself as soon as the queue is back.', [
          h('p', { class: 'tvsoft' }, `Trying again in ${this.away.secs} s`),
          h('button', { class: 'tvbtn', type: 'button', onclick: () => this.wake() }, 'Try now'),
        ]);
        break;
      case 'ended':
        body = this.calm('This queue has ended', 'Thanks for waiting!');
        break;
      case 'signed-out':
        body = this.calm('This screen link has expired', 'Open the big screen again from the Pocket card in kwant.');
        break;
      case 'missing':
        body = this.calm("This queue isn't here any more", 'The host may have deleted it.');
        break;
      default:
        body = this.calm('Opening the big screen', 'One moment.');
    }
    this.stage.replaceChildren();
    add(this.stage, [body]);
  }
}
