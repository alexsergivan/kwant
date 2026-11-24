// The Wall on the big screen (pa14, design.md section 3): approved photos on a TV, readable from 5 m. Read-only like
// the quiz screen: it signs in once with its link's token, then only reads host/state?view=screen and ws?view=screen.
// TV1 slideshow (8 s a photo, the next one preloaded), TV2 a new photo jumps the line with a short slide and a
// "New from" pill, TV3 before the first photo (a big QR), TV4 still view (reduced motion: a 3x2 grid, one tile swaps
// every 8 s, no movement). A photo the host hides while it is on stage is cut away at once. Every number comes from the
// box; the only maths here is layout (the stage scale) and which photo is next.
import { Api, Live, backoff, screenSignIn } from '../api';
import { Kid, add, h } from '../dom';
import { qrPath } from '../screen/qr';
import type { ScreenEnv } from '../screen/app';
import { guestCreature, snap, wIcon } from './look';
import { TvView, WallPhoto, tvFrom } from './view';

type Shown = 'loading' | 'slides' | 'waiting' | 'still' | 'away' | 'ended' | 'signed-out' | 'missing';

const NS = 'http://www.w3.org/2000/svg';
export const SLIDE_MS = 8000;
export const PILL_MS = 4000;
export const MOVE_MS = 200;
const GRID = 6;

export class WallTV {
  private api: Api<TvView>;
  private live: Live<TvView>;
  private view: TvView | undefined;
  private shown: Shown = 'loading';
  private stage: HTMLElement;
  private said: HTMLElement;
  private away = { secs: 0, tries: 0 };
  /** Ids already seen, so a new arrival can jump the line (none on the first view: those are the wall so far). */
  private seen: Set<string> | undefined;
  /** New arrivals still to jump the line, oldest first. */
  private jump: string[] = [];
  private cur: string | undefined;
  private leaving: string | undefined;
  private pill: { name: string; until: number } | undefined;
  /** Still view: the six tiles on screen, and which slot swaps next. */
  private tiles: string[] = [];
  private slot = 0;
  private timer = 0;
  private pillTimer = 0;
  private moveTimer = 0;
  private imgs = new Map<string, HTMLImageElement>();

  constructor(root: HTMLElement, private env: ScreenEnv, readonly code: string, private token?: string) {
    this.api = new Api<TvView>(env, code, tvFrom);
    this.api.statePath = 'host/state?view=screen';
    this.api.wsPath = 'ws?view=screen';
    this.live = new Live<TvView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => {
        this.away = { secs, tries };
        this.draw('away');
      },
      back: () => this.view && this.show(this.view),
      gone: (status, error) => this.gone(status, error),
    });
    this.stage = h('div', { class: 'tv wtv', 'data-screen': 'loading' });
    this.said = h('p', { class: 'vh', 'aria-live': 'polite' });
    root.replaceChildren(h('main', { class: 'tvstage', 'aria-label': 'Big screen: photo wall' }, this.stage), this.said);
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
    this.env.clearTimeout(this.timer);
    this.env.clearTimeout(this.pillTimer);
    this.env.clearTimeout(this.moveTimer);
  }

  fit(w: number, hh: number): void {
    this.stage.style.setProperty('--s', String(Math.min(w / 1920, hh / 1080)));
  }

  private still(): boolean {
    return !!this.env.still?.();
  }

  private gone(status: number, error?: string): void {
    this.stop();
    if (status === 410 || error === 'ended') return this.draw('ended');
    this.draw(status === 404 ? 'missing' : 'signed-out');
  }

  private byId(id: string | undefined): WallPhoto | undefined {
    return id ? this.view?.photos.find((p) => p.id === id) : undefined;
  }

  private show(v: TvView): void {
    this.view = v;
    if (v.ended) return this.gone(410);
    const ids = v.photos.map((p) => p.id);
    const first = !this.seen;
    const fresh = first ? [] : ids.filter((id) => !this.seen!.has(id));
    this.seen = new Set([...(this.seen ?? []), ...ids]);
    this.jump = this.jump.filter((id) => ids.includes(id));
    if (!v.photos.length) {
      this.cur = undefined;
      this.tiles = [];
      return this.draw('waiting');
    }
    if (this.still()) return this.showStill(fresh);
    this.jump.push(...fresh);
    if (!this.cur || !ids.includes(this.cur)) {
      // First photo, or the one on stage was hidden: cut to the next one at once, no movement.
      this.cur = this.jump.shift() ?? this.after(this.cur);
      this.leaving = undefined;
      this.arm();
      return this.draw('slides');
    }
    if (this.jump.length) return this.next(true);
    if (this.shown !== 'slides') this.arm();
    this.draw('slides');
  }

  /** The photo after `id` in the box's order (oldest first), looping; the first one when `id` is gone. */
  private after(id: string | undefined): string | undefined {
    const ps = this.view?.photos ?? [];
    if (!ps.length) return undefined;
    const i = id ? ps.findIndex((p) => p.id === id) : -1;
    return ps[(i + 1) % ps.length].id;
  }

  private arm(): void {
    this.env.clearTimeout(this.timer);
    this.timer = this.env.setTimeout(() => (this.still() ? this.swap() : this.next(false)), SLIDE_MS);
  }

  /** TV1/TV2: the next photo; a new arrival jumps the line, slides in and gets the pill for 4 s. */
  private next(arrival: boolean): void {
    const id = this.jump.shift() ?? this.after(this.cur);
    if (!id) return;
    const p = this.byId(id);
    if (arrival && p) {
      this.pill = { name: p.by_name, until: this.env.now() + PILL_MS };
      this.said.textContent = `New photo from ${p.by_name}`;
      this.env.clearTimeout(this.pillTimer);
      this.pillTimer = this.env.setTimeout(() => {
        this.pill = undefined;
        this.draw('slides');
      }, PILL_MS);
    }
    this.leaving = id !== this.cur && !this.still() ? this.cur : undefined;
    this.cur = id;
    this.env.clearTimeout(this.moveTimer);
    if (this.leaving)
      this.moveTimer = this.env.setTimeout(() => {
        this.leaving = undefined;
        this.draw('slides');
      }, MOVE_MS);
    this.arm();
    this.draw('slides');
  }

  /** TV4: keep the six tiles (a hidden one is replaced at once), new arrivals take the next slot. */
  private showStill(fresh: string[]): void {
    const ps = this.view!.photos;
    const ids = ps.map((p) => p.id);
    const pool = () => ids.filter((id) => !this.tiles.includes(id));
    this.tiles = this.tiles.map((t) => (ids.includes(t) ? t : pool()[0] ?? '')).filter(Boolean);
    while (this.tiles.length < Math.min(GRID, ids.length)) this.tiles.push(pool()[0]);
    for (const id of fresh) if (!this.tiles.includes(id)) this.put(id);
    if (this.shown !== 'still') this.arm();
    this.draw('still');
  }

  private put(id: string): void {
    if (this.tiles.length < GRID) this.tiles.push(id);
    else {
      this.tiles[this.slot % GRID] = id;
      this.slot = (this.slot + 1) % GRID;
    }
  }

  /** Every 8 s one tile changes, instantly: the photo after the newest one on screen. */
  private swap(): void {
    const ps = this.view?.photos ?? [];
    if (ps.length > this.tiles.length) {
      let id = this.after(this.tiles[(this.slot + GRID - 1) % GRID] ?? this.tiles[this.tiles.length - 1]);
      for (let k = 0; id && this.tiles.includes(id) && k < ps.length; k++) id = this.after(id);
      if (id && !this.tiles.includes(id)) this.put(id);
    }
    this.arm();
    this.draw('still');
  }

  // ---------- drawing ----------

  private link(): string {
    return `${this.env.origin}/p/${encodeURIComponent(this.code)}`;
  }

  private qr(cls: string): SVGSVGElement {
    const link = this.link();
    const { size, d } = qrPath(link);
    const qr = document.createElementNS(NS, 'svg');
    qr.setAttribute('viewBox', `0 0 ${size} ${size}`);
    qr.setAttribute('class', cls);
    qr.setAttribute('shape-rendering', 'crispEdges');
    qr.setAttribute('role', 'img');
    qr.setAttribute('aria-label', `QR code to add your photos: ${link}`);
    const path = document.createElementNS(NS, 'path');
    path.setAttribute('d', d);
    qr.append(path);
    return qr;
  }

  private img(p: WallPhoto, cls = ''): HTMLImageElement {
    let img = this.imgs.get(p.id);
    if (!img) {
      img = h('img', { src: `${this.api.base}/photo/${encodeURIComponent(p.id)}`, decoding: 'async', alt: '' }) as HTMLImageElement;
      this.imgs.set(p.id, img);
    }
    const c = `${cls}${p.rotate ? ` r${p.rotate}` : ''}`.trim();
    if (c) img.setAttribute('class', c);
    else img.removeAttribute('class');
    return img.cloneNode() as HTMLImageElement;
  }

  /** One ahead, so the next slide is already in the browser's cache. */
  private preload(): void {
    const n = this.byId(this.jump[0] ?? this.after(this.cur));
    if (n && !this.imgs.has(n.id)) this.img(n);
  }

  private top(right: Kid): HTMLElement {
    return h('header', { class: 'tvtop' }, h('p', null, h('b', null, this.view?.title || 'Photo wall'), ' · photo wall'), right != null ? h('p', { class: 'tvright' }, right) : null);
  }

  private counter(extra = ''): Kid {
    const n = this.view?.count;
    return n == null ? (extra ? extra.replace(/^ · /, '') : null) : `${n.toLocaleString('en-US')} ${n === 1 ? 'photo' : 'photos'}${extra}`;
  }

  private corner(): HTMLElement {
    return h('div', { class: 'wqr' }, h('p', null, h('b', null, 'Add your photos'), this.link().replace(/^https?:\/\//, '')), h('span', { class: 'wqrbox' }, this.qr('wqrsvg')));
  }

  private slide(p: WallPhoto, cls: string): HTMLElement {
    const label = p.caption ? `Photo from ${p.by_name}: ${p.caption}` : `Photo from ${p.by_name}`;
    return h('figure', { class: `wslide${cls}`, role: 'img', 'aria-label': label, 'data-id': p.id }, h('div', { class: 'wblur', 'aria-hidden': 'true' }, this.img(p)), h('div', { class: 'wfit' }, this.img(p)));
  }

  private caption(p: WallPhoto): HTMLElement {
    const fresh = this.pill && this.pill.name === p.by_name;
    return h(
      'div',
      { class: 'wcap' },
      guestCreature(p.creature, 'wcr'),
      h('div', null, p.caption ? h('q', null, p.caption) : null, h('small', null, fresh ? `${p.by_name} · just now` : p.by_name)),
    );
  }

  private calm(title: string, line: string, mood: 'thinking' | 'finished' | undefined, extra: Kid[] = []): Kid {
    const s = snap(160, mood);
    s.classList.add('tvqm', 'big');
    return [
      this.view?.title ? this.top(null) : null,
      h('section', { class: 'tvcalm' }, s, h('h1', { class: 'qbig' }, title), h('p', { class: 'tvsoft' }, line), ...extra),
      h('footer', { class: 'tvfoot' }, null, h('p', { class: 'made' }, 'made with kwant')),
    ];
  }

  private draw(s: Shown): void {
    this.shown = s;
    this.stage.setAttribute('data-screen', s);
    let body: Kid;
    switch (s) {
      case 'slides': {
        const p = this.byId(this.cur)!;
        const out = this.byId(this.leaving);
        body = [
          this.top(this.counter()),
          out ? this.slide(out, ' out') : null,
          this.slide(p, out ? ' in' : ''),
          this.pill ? h('p', { class: 'wnew', 'aria-hidden': 'true' }, wIcon('camera'), `New from ${this.pill.name}`) : null,
          this.caption(p),
          this.corner(),
        ];
        this.preload();
        break;
      }
      case 'still': {
        const ps = this.tiles.map((id) => this.byId(id)).filter((p): p is WallPhoto => !!p);
        body = [
          this.top(this.counter(' · still view')),
          h(
            'ul',
            { class: 'wgrid', 'aria-label': 'Photos on the wall' },
            ps.map((p) =>
              h('li', { class: 'wtile', 'data-id': p.id }, this.img(p), h('p', { class: 'wtcap' }, p.caption ?? null, h('small', null, p.by_name))),
            ),
          ),
          h('div', { class: 'wcap' }, h('div', null, h('small', null, 'One photo changes every 8 seconds.'))),
          this.corner(),
        ];
        break;
      }
      case 'waiting':
        body = [
          this.top(this.counter()),
          h('div', { class: 'wbigqr' }, this.qr('wqrsvg')),
          h(
            'section',
            { class: 'wscan' },
            h('h1', null, 'Scan to add your photos'),
            h('p', { class: 'tvsoft' }, 'No app, no account. They show here once the host says yes.'),
            h('p', { class: 'wlink' }, this.link().replace(/^https?:\/\//, '')),
          ),
          h('div', { class: 'tvhost wsnap' }, snap(160, 'thinking'), h('p', { class: 'b' }, "I'll put the first photos up as soon as they're approved.")),
        ];
        break;
      case 'away':
        body = this.calm("The host's computer is resting", 'This screen reconnects by itself as soon as the wall is back.', 'thinking', [
          h('p', { class: 'tvsoft' }, `Trying again in ${this.away.secs} s`),
          h('button', { class: 'tvbtn', type: 'button', onclick: () => this.wake() }, 'Try now'),
        ]);
        break;
      case 'ended':
        body = this.calm('The wall is closed', 'Thanks for adding your photos!', 'finished');
        break;
      case 'signed-out':
        body = this.calm('This screen link has expired', 'Open the big screen again from the Pocket card in kwant.', undefined);
        break;
      case 'missing':
        body = this.calm("This wall isn't here any more", 'The host may have deleted it.', undefined);
        break;
      default:
        body = this.calm('Opening the big screen', 'One moment.', undefined);
    }
    this.stage.replaceChildren();
    add(this.stage, [body]);
  }
}
