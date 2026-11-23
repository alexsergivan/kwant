// The big screen (pa5b, pa0a/design.md section 4): the pub quiz on a TV, readable from 5 m. Read-only: it signs in once
// with its link's token (host/signin), then only reads host/state?view=screen and ws?view=screen. It never calls a
// host write route. A 1920x1080 stage scaled to the viewport (letterboxed on 4:3 or ultra-wide), nothing under 32 px.
// Every number on it comes from the box; the only maths here is layout (the stage scale, bar widths, the countdown
// ticking down from the box's remaining_ms between two states).
import { Api, Env, Live, backoff, screenSignIn } from '../api';
import { AVATARS, Kid, creature, fmt, h, icon, quizmaster, svg } from '../dom';
import { Rects } from '../art';
import { qrPath } from './qr';
import { ScreenView, screenFrom } from './view';

export interface ScreenEnv extends Env {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
  /** The page's own origin (scheme, host and port), for the join link in the QR. */
  origin: string;
  /** prefers-reduced-motion: reduce. */
  still?: () => boolean;
}

type Shown = 'loading' | 'lobby' | 'question' | 'reveal' | 'leaderboard' | 'winners' | 'paused' | 'away' | 'ended' | 'signed-out' | 'missing';

const NS = 'http://www.w3.org/2000/svg';
const TROPHY: Rects = [[0, 0, 9, 1, -1], [0, 1, 1, 1, -1], [2, 1, 5, 1, -1], [8, 1, 1, 1, -1], [0, 2, 1, 1, -1], [2, 2, 5, 1, -1], [8, 2, 1, 1, -1], [1, 3, 7, 1, -1], [2, 4, 5, 1, -1], [3, 5, 3, 1, -1], [4, 6, 1, 1, -1], [3, 7, 3, 1, -1], [2, 8, 5, 1, -1]];
/** Lobby: up to 18 players at full size, up to 36 smaller; past that the newest 36 (the count line has everyone). */
const LOBBY_BIG = 18;
const LOBBY_MOST = 36;
const BOARD_ROWS = 6;
const LETTERS = 'ABCDEFGH';

const clock = (ms: number): string => {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};
const plural = (n: number, one: string, many: string) => `${fmt(n)} ${n === 1 ? one : many}`;
const hostOf = (url: string): string => {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
};

export class ScreenApp {
  private api: Api<ScreenView>;
  private live: Live<ScreenView>;
  private view: ScreenView | undefined;
  private shown: Shown = 'loading';
  /** Everyone seen in the lobby, in join order: their creature and team for the answered row and the podium. */
  private roster = new Map<string, { avatar?: string; team?: string }>();
  /** Team colours in join order (design.md "Data the app doesn't have yet"), unless the box sends one. */
  private colors = new Map<string, number>();
  private seen = new Set<string>();
  private lastBoard: string[] = [];
  private tick = 0;
  private at = 0;
  private stage: HTMLElement;
  private said: HTMLElement;
  private away = { secs: 0, tries: 0 };

  constructor(root: HTMLElement, private env: ScreenEnv, readonly code: string, private token?: string) {
    this.api = new Api<ScreenView>(env, code, screenFrom);
    this.api.statePath = 'host/state?view=screen';
    this.api.wsPath = 'ws?view=screen';
    this.live = new Live<ScreenView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => {
        this.away = { secs, tries };
        this.draw('away');
      },
      back: () => this.view && this.show(this.view),
      gone: (status, error) => this.gone(status, error),
    });
    this.stage = h('div', { class: 'tv', 'data-screen': 'loading' });
    this.said = h('p', { class: 'vh', 'aria-live': 'polite' });
    root.replaceChildren(h('main', { class: 'tvstage', 'aria-label': 'Big screen' }, this.stage), this.said);
    document.documentElement.classList.add('tvpage');
  }

  /** Signs in with the link's token once (retrying on the phone page's backoff while the box is away), then goes live. */
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
      if (r.status !== 0 && r.status < 500) return this.gone(r.status, r.error);
      this.away = { secs: backoff(tries), tries: ++tries };
      this.draw('away');
      await new Promise<void>((res) => this.env.setTimeout(res, this.away.secs * 1000));
    }
  }

  wake(): void {
    this.live.now();
  }

  /** Fits the 1920x1080 stage into the window: the smaller of the two ratios, so it letterboxes instead of overflowing. */
  fit(w: number, hh: number): void {
    this.stage.style.setProperty('--s', String(Math.min(w / 1920, hh / 1080)));
  }

  private gone(status: number, error?: string): void {
    this.live.stop();
    if (status === 410 || error === 'ended') {
      if (this.view) {
        this.view = { ...this.view, ended: true };
        return this.draw(this.view.board?.length ? 'winners' : 'ended');
      }
      return this.draw('ended');
    }
    this.draw(status === 404 ? 'missing' : 'signed-out');
  }

  private show(v: ScreenView): void {
    this.view = v;
    this.at = this.env.now();
    for (const p of v.lobby ?? []) {
      this.roster.set(p.name, { avatar: p.avatar ?? this.roster.get(p.name)?.avatar, team: p.team });
      if (p.team) this.color(p.team);
    }
    for (const r of v.board ?? []) if (v.teams) this.color(r.name, r.color);
    const paused = v.time?.paused && (v.phase === 'question' || v.phase === 'locked');
    const s: Shown =
      v.phase === 'final' || (v.ended && v.board?.length) ? 'winners'
      : v.ended ? 'ended'
      : paused ? 'paused'
      : v.phase === 'lobby' ? 'lobby'
      : v.phase === 'reveal' ? 'reveal'
      : v.phase === 'leaderboard' ? 'leaderboard'
      : 'question';
    this.draw(s);
  }

  private color(team: string, given?: number): number {
    if (given !== undefined) this.colors.set(team, given % 8);
    if (!this.colors.has(team)) this.colors.set(team, this.colors.size % 8);
    return this.colors.get(team)!;
  }

  private avatarOf(name: string): string {
    const i = [...this.roster.keys()].indexOf(name);
    return this.roster.get(name)?.avatar ?? AVATARS[Math.max(0, i) % AVATARS.length];
  }

  private still(): boolean {
    return this.env.still?.() ?? false;
  }

  private draw(s: Shown): void {
    const was = this.shown;
    this.shown = s;
    this.env.clearInterval(this.tick);
    this.tick = 0;
    const v = this.view;
    let body: Kid;
    switch (s) {
      case 'lobby':
        body = this.lobby(v!);
        break;
      case 'question':
        body = this.question(v!);
        break;
      case 'reveal':
        body = this.reveal(v!);
        break;
      case 'leaderboard':
        body = this.board(v!);
        break;
      case 'winners':
        body = this.winners(v!);
        break;
      case 'paused':
        body = this.calm(v, 'Back in a moment', `The quiz is paused. The timer stopped at ${clock(v?.time?.remaining_ms ?? 0)}.`, 'thinking');
        break;
      case 'away':
        body = this.calm(v, "The host's box is away", 'This screen reconnects by itself as soon as the quiz is back.', 'thinking', [
          h('p', { class: 'tvsoft tab' }, `Trying again in ${this.away.secs} s · tried ${this.away.tries} ${this.away.tries === 1 ? 'time' : 'times'}`),
          h('button', { class: 'tvbtn', type: 'button', onclick: () => this.wake() }, 'Try now'),
        ]);
        break;
      case 'ended':
        body = this.calm(v, 'This quiz has ended', 'Thanks for playing!', 'finished');
        break;
      case 'signed-out':
        body = this.calm(undefined, 'This screen link has expired', 'Open the big screen again from the Pocket card in kwant.', 'thinking');
        break;
      case 'missing':
        body = this.calm(undefined, "This quiz isn't here any more", 'The host may have deleted it.', 'thinking');
        break;
      default:
        body = this.calm(undefined, 'Opening the big screen', 'One moment.', undefined);
    }
    this.stage.setAttribute('data-screen', s);
    this.stage.replaceChildren(...(Array.isArray(body) ? body : [body]).filter((x): x is Node => x instanceof Node));
    if (s === 'leaderboard') this.reorder(was === 'leaderboard');
    if (s !== was) this.say(s, v);
  }

  /** One polite line per screen change for a screen reader; the screens themselves are headed and labelled. */
  private say(s: Shown, v: ScreenView | undefined): void {
    const q = v?.question;
    const line =
      s === 'question' && q ? `Question ${q.number} of ${q.of}` :
      s === 'reveal' && v?.reveal ? `The answer is ${v.reveal.answer}` :
      s === 'leaderboard' ? 'Scores' :
      s === 'winners' ? 'Final scores' :
      s === 'lobby' ? 'Lobby open' : '';
    this.said.textContent = line;
  }

  // ---------- pieces ----------

  private top(left: Kid, right?: Kid): HTMLElement {
    return h('header', { class: 'tvtop' }, h('p', null, left), right ? h('p', { class: 'tvright' }, right) : null);
  }

  private host(line: string | undefined, mood?: 'thinking' | 'finished'): HTMLElement | null {
    if (!line) return null;
    const qm = quizmaster(96, mood);
    qm.classList.add('tvqm');
    return h('div', { class: 'tvhost' }, qm, h('p', { class: 'b' }, line));
  }

  private crew(name: string, cls: string, dim = false): SVGSVGElement {
    const c = creature(this.avatarOf(name), cls);
    if (dim) c.classList.add('wait');
    return c;
  }

  private meta(v: ScreenView): Kid {
    const r = v.round;
    const q = v.question;
    return [
      r ? h('b', null, `Round ${r.number}${r.title ? ` · ${r.title}` : ''}`) : null,
      q ? `${r ? ' · ' : ''}question ${q.number} of ${q.of}` : null,
    ];
  }

  // ---------- screens ----------

  private lobby(v: ScreenView): Kid {
    const link = `${this.env.origin}/p/${encodeURIComponent(this.code)}`;
    const { size, d } = qrPath(link);
    const qr = document.createElementNS(NS, 'svg');
    qr.setAttribute('viewBox', `0 0 ${size} ${size}`);
    qr.setAttribute('class', 'tvqr');
    qr.setAttribute('shape-rendering', 'crispEdges');
    qr.setAttribute('role', 'img');
    qr.setAttribute('aria-label', `QR code to join the quiz: ${link}`);
    const path = document.createElementNS(NS, 'path');
    path.setAttribute('d', d);
    qr.append(path);

    const all = v.lobby ?? [];
    const list = all.slice(-LOBBY_MOST);
    const first = this.seen.size === 0;
    const players = h('ul', { class: `tvplayers${list.length > LOBBY_BIG ? ' many' : ''}`, 'aria-label': 'Players here so far' });
    for (const p of list) {
      const li = h('li', { class: 'player' }, this.crew(p.name, 'pc'), h('span', { class: 'nm' }, p.name));
      if (!first && !this.seen.has(p.name) && !this.still()) li.classList.add('drop-in');
      players.append(li);
    }
    for (const p of all) this.seen.add(p.name);
    const n = v.players ?? all.length;
    return [
      this.top([h('b', null, v.title), ' · hosted with Quizmaster'], ['Join at ', h('b', null, link.replace(/^https?:\/\//, ''))]),
      h('section', { class: 'tvjoin' }, h('div', { class: 'tvqrbox' }, qr), h('h1', { class: 'scan' }, 'Scan to play'), h('p', { class: 'noapp' }, 'No app, no account')),
      h('section', { class: 'tvlobby' }, h('h2', { class: 'tvcount tab' }, n ? plural(n, 'player', 'players') : 'Waiting for the first player'), players),
      h('footer', { class: 'tvfoot' }, this.host(v.line ?? "Grab a seat and pick a team name you'd be happy to hear me read out loud."), h('p', { class: 'made' }, 'made with kwant')),
    ];
  }

  private answers(v: ScreenView, reveal: boolean): HTMLElement | null {
    const q = v.question;
    const r = v.reveal;
    if (!q) return null;
    if (!q.options?.length) {
      return reveal && r
        ? h('div', { class: 'tvsolo right' }, icon('check', 'pi ok'), h('span', null, r.answer))
        : h('div', { class: 'tvsolo' }, q.kind === 'number' ? 'Pick your number on your phone' : 'Type your answer on your phone');
    }
    return h(
      'ol',
      { class: 'tvans', 'aria-label': 'Answers' },
      q.options.map((o, i) => {
        const right = reveal && r?.position === i;
        const pct = reveal ? r?.pcts?.[i] : undefined;
        const picks = reveal ? r?.picks?.[i] : undefined;
        return h(
          'li',
          { class: `ans${reveal ? (right ? ' right' : ' dim') : ''}` },
          h('span', { class: 'k', 'aria-hidden': 'true' }, LETTERS[i] ?? ''),
          h('span', { class: 'o' }, o),
          right ? h('span', { class: 'vh' }, ' (the right answer)') : null,
          pct !== undefined ? h('span', { class: 'pct tab' }, `${pct}%`) : picks !== undefined ? h('span', { class: 'pct tab' }, plural(picks, 'pick', 'picks')) : null,
        );
      }),
    );
  }

  private question(v: ScreenView): Kid {
    const q = v.question;
    const t = v.time;
    const locked = v.phase === 'locked' || !t;
    const count = h('p', { class: 'tvclock tab', 'aria-hidden': 'true' }, locked ? '' : clock(t!.remaining_ms));
    const bar = h('i');
    const timer = h('div', { class: 'tvtimer', 'aria-hidden': 'true' }, bar);
    const total = (q?.time_limit_s ?? 0) * 1000;
    const paint = () => {
      const left = locked ? 0 : Math.max(0, t!.remaining_ms - (this.env.now() - this.at));
      if (!locked) count.textContent = clock(left);
      bar.style.transform = `scaleX(${total > 0 ? Math.min(1, left / total) : 0})`;
      timer.classList.toggle('low', !locked && left < 10_000);
    };
    paint();
    if (!locked) this.tick = this.env.setInterval(paint, 1000);
    const a = v.answered;
    let row: HTMLElement | null = null;
    if (a) {
      const names = a.names ?? [];
      const did = new Set(names);
      const waiting = [...this.roster.keys()].filter((n) => !did.has(n));
      row = h(
        'section',
        { class: 'tvanswered' },
        h('p', { class: 'tvn tab' }, `${fmt(a.count)} of ${fmt(a.of)}`, h('br'), 'answered'),
        h('div', { class: 'who', 'aria-hidden': 'true' }, [...names.map((n) => this.crew(n, 'wc')), ...waiting.map((n) => this.crew(n, 'wc', true))].slice(0, LOBBY_MOST)),
      );
    }
    return [
      this.top(this.meta(v), locked ? "Time's up" : null),
      locked ? null : count,
      h('div', { class: 'tvq' }, timer, h('h1', { class: `qbig${(q?.text.length ?? 0) > 90 ? ' long' : ''}` }, q?.text ?? ''), this.answers(v, false)),
      row,
    ];
  }

  private reveal(v: ScreenView): Kid {
    const r = v.reveal!;
    const host = hostOf(r.source_url);
    const src = [host, r.source_quote].filter(Boolean).join(': ');
    const a = v.answered;
    const line = r.line ?? (r.right !== undefined && a ? `${fmt(r.right)} of ${fmt(a.of)} got it right.` : undefined);
    return [
      this.top(this.meta(v), 'Answer'),
      h(
        'div',
        { class: 'tvq rv' },
        h('h1', { class: 'qbig small' }, v.question?.text ?? ''),
        this.answers(v, true),
        src ? h('p', { class: 'tvsrc' }, icon('check', 'pi ok'), h('span', null, `Source: ${src}`)) : null,
      ),
      h('footer', { class: 'tvfoot' }, this.host(line)),
    ];
  }

  private board(v: ScreenView): Kid {
    const rows = (v.board ?? []).slice(0, BOARD_ROWS);
    const lead = Math.max(1, rows[0]?.score ?? 1);
    return [
      this.top(v.round ? h('b', null, `After round ${v.round.number} of ${v.round.of}`) : h('b', null, 'Scores')),
      h(
        'ol',
        { class: 'lbt', 'aria-label': 'Leaderboard' },
        rows.map((r, i) => {
          const team = v.teams ? r.name : r.team;
          const c = team ? this.color(team, v.teams ? r.color : undefined) : -1;
          const bar = h('i', { class: c >= 0 ? `t${c}` : '' });
          bar.style.transform = `scaleX(${Math.max(0, Math.min(1, r.score / lead))})`;
          return h(
            'li',
            { class: `row${i === 0 ? ' first' : ''}`, 'data-name': r.name },
            h('span', { class: 'n tab' }, String(r.rank)),
            h('span', { class: 'nm' }, v.teams ? h('span', { class: `sw${c >= 0 ? ` t${c}` : ''}`, 'aria-hidden': 'true' }) : null, h('span', { class: 'tx' }, r.name)),
            h('span', { class: 'bar', 'aria-hidden': 'true' }, bar),
            h('span', { class: 's tab' }, fmt(r.score)),
            h('span', { class: `d tab${r.move ? (r.move > 0 ? ' up' : ' down') : ''}` }, r.move ? `${r.move > 0 ? '▲' : '▼'} ${Math.abs(r.move)}` : '', r.move ? h('span', { class: 'vh' }, r.move > 0 ? ' places up' : ' places down') : null),
          );
        }),
      ),
      h('footer', { class: 'tvfoot' }, this.host(v.line)),
    ];
  }

  /** Rows slide from their place on the last board to the new one (200 ms translateY); static under reduced motion. */
  private reorder(again: boolean): void {
    const rows = [...this.stage.querySelectorAll<HTMLElement>('.lbt .row')];
    const names = rows.map((r) => r.getAttribute('data-name') ?? '');
    const before = this.lastBoard;
    this.lastBoard = names;
    if (!again && !before.length) return;
    if (this.still()) return;
    rows.forEach((el, i) => {
      const old = before.indexOf(names[i]);
      if (old < 0 || old === i) return;
      el.style.transform = `translateY(${(old - i) * 100}px)`;
      el.getBoundingClientRect();
      el.classList.add('moving');
      el.style.transform = '';
    });
  }

  private winners(v: ScreenView): Kid {
    const rows = (v.board ?? []).slice(0, 3);
    const tops = rows.filter((r) => r.rank === 1);
    const names = tops.map((r) => r.name);
    const head = names.length > 1 ? `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]} win!` : `${names[0] ?? ''} ${v.teams ? 'win!' : 'wins!'}`;
    const order = [rows[1], rows[0], rows[2]];
    const stars = [0, 1, 2, 3, 4].map((i) => icon('star', `pi tvstar s${i}`));
    const sticker = h('span', { class: 'kw-sticker kw-sticker-holo tvsticker' }, svg('0 0 9 9', TROPHY, 'pi'), h('span', null, 'Winners'));
    return [
      this.top([h('b', null, v.title), ' · final scores'], v.players ? plural(v.players, 'player', 'players') : null),
      h('div', { class: `tvstars${this.still() ? '' : ' twinkle'}`, 'aria-hidden': 'true' }, stars),
      h('div', { class: 'tvwin' }, h('span', { class: 'bigsticker' }, sticker), h('h1', { class: 'winner' }, head)),
      h(
        'ol',
        { class: 'podium', 'aria-label': 'Podium' },
        order.map((r, i) => {
          if (!r) return null;
          const place = [2, 1, 3][i];
          const team = v.teams ? r.name : r.team;
          const c = team ? this.color(team, v.teams ? r.color : undefined) : 7;
          const crew = v.teams ? [...this.roster.entries()].filter(([, p]) => p.team === r.name).map(([n]) => n).slice(0, 4) : [r.name];
          return h(
            'li',
            { class: `pod p${place}`, value: String(r.rank) },
            h('div', { class: 'crew', 'aria-hidden': 'true' }, crew.map((n) => this.crew(n, 'cc'))),
            h('p', { class: 'tn' }, r.name),
            h('p', { class: 'ts tab' }, `${fmt(r.score)} points`),
            h('div', { class: `blk t${c}`, 'aria-hidden': 'true' }, String(r.rank)),
          );
        }),
      ),
    ];
  }

  private calm(v: ScreenView | undefined, title: string, line: string, mood: 'thinking' | 'finished' | undefined, extra: Kid[] = []): Kid {
    const qm = quizmaster(96, mood);
    qm.classList.add('tvqm', 'big');
    return [
      v?.title ? this.top(h('b', null, v.title)) : null,
      h('section', { class: 'tvcalm' }, qm, h('h1', { class: 'qbig' }, title), h('p', { class: 'tvsoft' }, line), ...extra),
      h('footer', { class: 'tvfoot' }, null, h('p', { class: 'made' }, 'made with kwant')),
    ];
  }
}
