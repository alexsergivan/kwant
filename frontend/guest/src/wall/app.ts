// The Wall's guest page, G1-G8 in pa14/guest-390.html, on Juno's wall API (src/pocket/wall.rs, guest.rs). Loaded as its
// own chunk when the pocket's kit is `wall`; the quiz and Santa screens are untouched. Plain DOM, no framework; text
// goes in as text only, photos as <img> from the box (or blob: thumbnails of the guest's own picks). Numbers shown come
// from the server (limits) or count what this page itself holds: the rows it is sending and the tiles it draws.
import { Api, Env, Fail, Live } from '../api';
import { AVATARS, h, icon, Kid } from '../dom';
import { day } from '../app';
import { PinInput, pinCopy, Problem } from '../santa/pin';
import { snap, wIcon } from './look';
import { Row, UploadQueue, xhrSend } from './queue';
import { cap, hasBest, MinePhoto, PAGE, Pick, Refusal, Screen, screenOf, wallFrom, wallPage, WallPhoto, WallView } from './view';

const TOAST_MS = 4000;
const INFO_MS = 10_000;
const ACCEPT = 'image/jpeg,image/png,image/webp';

export type WallEnv = Env & {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
  XHR?: typeof XMLHttpRequest;
  /** URL.createObjectURL / revokeObjectURL, for thumbnails of the guest's own picks (img-src blob: in the CSP). */
  blobs?: { create: (b: Blob) => string; revoke: (u: string) => void };
};

const poss = (name: string): string => `${name}'s`;
const photos = (n: number): string => `${n} photo${n === 1 ? '' : 's'}`;

type Shown = Screen | 'away' | 'removed' | 'missing' | 'loading';
type PageNote = Extract<Refusal, { kind: 'page' }> & { full: boolean };

export class WallApp {
  view: WallView | null = null;
  readonly api: Api<WallView>;
  readonly queue: UploadQueue<File>;
  private live: Live<WallView>;
  private root: HTMLElement;
  private said: HTMLElement;
  private sig = '';
  private away: { secs: number; tries: number; left: number } | null = null;
  private awayTick = 0;
  private infoTimer = 0;
  private toastTimer = 0;
  private toast: HTMLElement | null = null;
  private gone: 'removed' | 'missing' | null = null;
  private hadSeat = false;
  private rejoin = false;
  private pick: Pick = 'auto';
  private jd = { name: '', pin: '' };
  private bd = { name: '', pin: '', code: '', newPin: '', reset: false };
  private problem: Problem | null = null;
  private busy = '';
  /** Photos picked on G1 before the guest has a seat: they go once the join works. */
  private early: File[] = [];
  private note: PageNote | null = null;
  /** How many went up in the batch now sending, and the last finished batch's count for the thank-you (G3). */
  private sentNow = 0;
  private thanks = 0;
  private closedUnsent = 0;
  /** Wall photo ids drawn so far (newer ones wait behind the pill) and how many may show. */
  private shown: string[] = [];
  private limit = PAGE;
  private noticesSeen = false;
  private imgs = new Map<string, HTMLImageElement>();
  private thumbs = new Map<number, { url: string; img: HTMLImageElement; bad: boolean }>();

  constructor(root: HTMLElement, private env: WallEnv, readonly code: string) {
    this.root = root;
    this.api = new Api<WallView>(env, code, wallFrom);
    this.live = new Live<WallView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => this.goAway(secs, tries),
      back: () => this.comeBack(),
      gone: (status, error) => void this.onGone(status, error),
    });
    const XHR = env.XHR ?? (typeof XMLHttpRequest !== 'undefined' ? XMLHttpRequest : undefined);
    this.queue = new UploadQueue<File>(`${this.api.base}/photo`, {
      send: XHR ? xhrSend<File>(XHR) : async () => ({ status: 0, json: null }),
      setTimeout: env.setTimeout,
      clearTimeout: env.clearTimeout,
      change: () => this.rowsChanged(),
      progress: (r) => this.moveBar(r),
      view: (json) => {
        this.sentNow++;
        const v = this.api.take(json);
        if (v) this.show(v);
      },
      page: (r) => {
        this.note = { ...r, full: !/most each guest/.test(r.title) };
        this.paint();
        this.root.querySelector<HTMLElement>('[data-note]')?.focus();
      },
      closed: () => {
        this.closedUnsent = this.queue.unsent.length;
        void this.api.state().then((r) => (r.ok && r.view ? this.show(r.view) : !r.ok && this.onGone(r.status, r.error)));
      },
      probe: async () => {
        const r = await this.api.info();
        return r.ok || (r.status > 0 && r.status < 500);
      },
      away: () => this.live.now(),
      facts: () => ({ host: this.view?.host_name, capMb: this.view?.limits.photo_cap_mb, perGuest: this.view?.limits.per_guest_max, captionMax: this.view?.limits.caption_max ?? 80 }),
    });
    this.said = h('p', { class: 'vh', 'aria-live': 'polite', 'data-said': '' });
  }

  async start(boot?: unknown): Promise<void> {
    const b = boot ? this.api.take(boot) : undefined;
    if (b) this.show(b);
    else this.paint();
    if (b?.me) return this.live.start();
    const r = await this.api.state();
    if (r.ok && r.view) {
      this.show(r.view);
      if (r.view.me) this.live.start();
    } else if (!r.ok && (r.status === 0 || r.status >= 500)) this.live.start();
    else if (!r.ok && r.status === 401) {
      if (!b) {
        const i = await this.api.info();
        if (i.ok && i.view) this.show(i.view);
        else if (!i.ok && i.status > 0 && i.status < 500) return this.onGone(i.status, i.error);
      }
      this.watchInfo();
    } else if (!r.ok) await this.onGone(r.status, r.error);
  }

  stop(): void {
    this.live.stop();
    this.queue.stop();
    this.env.clearInterval(this.awayTick);
    this.env.clearTimeout(this.infoTimer);
    this.env.clearTimeout(this.toastTimer);
  }

  wake(): void {
    if (this.gone || !this.view?.me) return;
    this.live.now();
  }

  tryNow = (): void => this.live.now();

  // ---------- incoming ----------

  show(v: WallView): void {
    if (this.gone) return;
    this.view = v;
    if (v.me) {
      this.hadSeat = true;
      this.rejoin = false;
    }
    // The box answers again: rows waiting for it go.
    if (this.queue.paused === 'away') this.queue.resume();
    this.paint();
    this.afterShow(v);
  }

  private afterShow(v: WallView): void {
    if (!v.me || this.away || this.noticesSeen || !v.me.notices.length) return;
    this.noticesSeen = true;
    const n = v.me.notices.find((x) => x.kind === 'pin_reset') ?? v.me.notices[0];
    const host = this.host();
    this.showToast(
      n.kind === 'pin_reset'
        ? `${cap(host)} reset your PIN${n.at ? ` on ${day(n.at)}` : ''}.`
        : `Someone tried to open your photos with a wrong PIN${n.at ? ` on ${day(n.at)}` : ''}. Your photos are safe.`,
    );
    void this.api.call('notices/seen', {});
  }

  private async onGone(status: number, error?: string): Promise<void> {
    if (status === 401 || error === 'signed_out') {
      this.live.stop();
      this.away = null;
      this.env.clearInterval(this.awayTick);
      if (this.hadSeat) this.rejoin = true;
      const i = await this.api.info();
      if (i.ok && i.view) {
        this.show(i.view);
        this.watchInfo();
      } else if (!i.ok && (i.status === 0 || i.status >= 500)) this.live.start();
      else if (!i.ok && i.status !== 401) await this.onGone(i.status, i.error);
      return;
    }
    if (status === 410 || error === 'ended') {
      this.live.stop();
      this.queue.stop();
      this.env.clearTimeout(this.infoTimer);
      this.away = null;
      if (this.view) this.show({ ...this.view, ended: true, open: false });
      else this.end('missing');
      return;
    }
    if (status === 403 || error === 'removed') this.end('removed');
    else if (status === 404 || error === 'not_found') this.end('missing');
  }

  private end(why: 'removed' | 'missing'): void {
    this.env.clearTimeout(this.infoTimer);
    this.live.stop();
    this.queue.stop();
    this.away = null;
    this.gone = why;
    this.paint();
  }

  private watchInfo(): void {
    this.env.clearTimeout(this.infoTimer);
    this.infoTimer = this.env.setTimeout(async () => {
      if (this.view?.me || this.gone) return;
      const i = await this.api.info();
      if (this.view?.me || this.gone) return;
      if (i.ok && i.view) this.show(i.view);
      else if (!i.ok && i.status > 0 && i.status < 500) return void this.onGone(i.status, i.error);
      if (!this.view?.ended) this.watchInfo();
    }, INFO_MS);
  }

  private goAway(secs: number, tries: number): void {
    this.away = { secs, tries, left: secs };
    this.env.clearInterval(this.awayTick);
    this.awayTick = this.env.setInterval(() => {
      if (!this.away) return;
      this.away.left = Math.max(0, this.away.left - 1);
      const t = this.root.querySelector('[data-away]');
      if (t) t.textContent = this.awayLine();
    }, 1000);
    this.paint();
  }

  private comeBack(): void {
    this.away = null;
    this.env.clearInterval(this.awayTick);
    if (this.queue.paused === 'away') this.queue.resume();
    this.paint();
  }

  private showToast(text: string): void {
    this.toast?.remove();
    const t = h('div', { class: 'toast', role: 'status' }, icon('info', 'pi'), h('span', null, text));
    this.toast = t;
    document.body.append(t);
    this.env.clearTimeout(this.toastTimer);
    this.toastTimer = this.env.setTimeout(() => {
      t.classList.add('out');
      this.env.setTimeout(() => t.remove(), 200);
      if (this.toast === t) this.toast = null;
    }, TOAST_MS);
  }

  // ---------- the upload rows ----------

  private rowsChanged(): void {
    const q = this.queue;
    // The batch is done once the sent rows are gone (settle): the thank-you counts what this page sent.
    if (!q.working && this.sentNow && !q.rows.some((r) => r.state === 'sent')) {
      this.thanks = this.sentNow;
      this.sentNow = 0;
    }
    for (const [key, t] of this.thumbs) {
      if (!q.rows.some((r) => r.key === key)) {
        this.env.blobs?.revoke(t.url);
        this.thumbs.delete(key);
      }
    }
    const sending = q.rows.filter((r) => r.state !== 'refused');
    const sent = sending.filter((r) => r.state === 'sent').length;
    if (sending.length) this.say(`${sent} of ${photos(sending.length)} sent`);
    this.paint();
  }

  private say(text: string): void {
    if (this.said.textContent !== text) this.said.textContent = text;
  }

  /** Moves one row's bar without drawing the page again. */
  private moveBar(r: Row<File>): void {
    const bar = this.root.querySelector<HTMLElement>(`[data-row="${r.key}"] [role="progressbar"]`);
    if (!bar) return;
    bar.setAttribute('aria-valuenow', String(r.loaded));
    bar.setAttribute('aria-valuemax', String(r.total || 1));
    (bar.firstElementChild as HTMLElement | null)?.style.setProperty('--p', String(r.total ? Math.min(1, r.loaded / r.total) : 0));
  }

  addFiles(list: FileList | File[] | null): void {
    const files = list ? [...list] : [];
    if (!files.length) return;
    this.thanks = 0;
    if (!this.view?.me) {
      this.early.push(...files);
      const d = this.jd;
      if (d.name.trim() && d.pin.length === 4) void this.submitJoin();
      else {
        this.paint(true);
        this.fail(!d.name.trim() ? { title: `Type your first name to send ${photos(this.early.length)}.`, field: 'name' } : { title: 'Pick a 4-digit PIN to send them.', field: 'pin' });
      }
      return;
    }
    this.queue.add(files);
  }

  private fileInput(id: string): HTMLInputElement {
    const el = h('input', { id, 'data-k': id, class: 'vh', type: 'file', accept: ACCEPT, multiple: true }) as HTMLInputElement;
    el.addEventListener('change', () => {
      const files = el.files ? [...el.files] : [];
      el.value = '';
      this.addFiles(files);
    });
    return el;
  }

  /** The big dashed "Add photos" (G1), or the sticky one under the wall (G3): a label for the hidden file input. */
  private picker(id: string, sticky: boolean): HTMLElement {
    if (sticky) return h('label', { class: 'gbtn addbar', for: id, 'data-add': '' }, wIcon('camera'), 'Add photos', this.fileInput(id));
    return h('label', { class: 'pickbtn', for: id, 'data-add': '' }, wIcon('camera', 'pi big'), h('span', null, 'Add photos'), h('small', null, 'Pick as many as you like'), this.fileInput(id));
  }

  // ---------- drawing ----------

  private host(): string {
    return this.view?.host_name ?? 'the host';
  }

  screenNow(): Shown {
    if (this.gone) return this.gone;
    if (this.away) return 'away';
    const v = this.view;
    if (!v) return 'loading';
    return screenOf(v, { rejoin: this.rejoin, pick: this.pick });
  }

  /** Draws the screen when something it shows changed. Typing and progress never redraw; focus and scroll survive. */
  paint(force = false): void {
    const scr = this.screenNow();
    const rows = this.queue.rows.map((r) => [r.key, r.state, r.why?.title]);
    const sig = JSON.stringify([scr, this.view, this.problem, this.busy, rows, this.note, this.shown, this.limit, this.thanks, this.early.length, this.bd.reset, this.away?.tries, this.closedUnsent]);
    if (!force && sig === this.sig) return;
    this.sig = sig;
    const active = document.activeElement as HTMLInputElement | null;
    const k = active?.getAttribute?.('data-k');
    const sel = k && active?.type === 'text' && typeof active?.selectionStart === 'number' ? [active.selectionStart, active.selectionEnd ?? active.selectionStart] : null;
    const scrollY = typeof window !== 'undefined' ? window.scrollY : 0;
    const page = h('main', { class: `g wall s-${scr}`, 'data-screen': scr }, this.bar(scr), this.body(scr), this.madeWith());
    this.root.replaceChildren(page, this.said);
    if (k) {
      const el = this.root.querySelector<HTMLInputElement>(`[data-k="${k}"]`);
      if (el) {
        el.focus({ preventScroll: true });
        if (sel) el.setSelectionRange?.(sel[0], sel[1]);
      }
    }
    if (typeof window !== 'undefined' && scrollY && window.scrollTo) window.scrollTo(0, scrollY);
  }

  private body(scr: Shown): HTMLElement {
    const v = this.view;
    switch (scr) {
      case 'loading':
        return this.message(snap(96, 'thinking'), 'Opening…', '');
      case 'removed':
        return this.message(snap(96), "You've been taken off this wall", `${cap(this.host())} took you off it. If that's a mistake, have a word with ${this.host()}.`);
      case 'missing':
        return this.message(snap(96), 'Nothing here', "This link doesn't open a pocket app any more. Check the link with the host.");
      case 'away':
        return this.awayScreen();
      case 'join':
        return this.join(v!);
      case 'rejoin':
        return this.back(v!);
      case 'wall':
        return this.wall(v!);
      case 'best':
        return this.best(v!);
      case 'closed':
        return this.closed(v!);
    }
  }

  private bar(scr: Shown): HTMLElement {
    const v = this.view;
    let right = '';
    if (v?.me && scr !== 'removed' && scr !== 'missing') right = v.me.name;
    else if (v?.when && (scr === 'join' || scr === 'rejoin')) right = day(v.when);
    return h('header', { class: 'bar' }, snap(28, undefined, true), h('b', null, v?.title ?? ''), h('span', { class: 'sp' }), h('span', { class: 'muted' }, right));
  }

  private madeWith(): HTMLElement {
    return h('footer', { class: 'madewith' }, 'made with ', h('a', { href: 'https://kwant.si', rel: 'noopener noreferrer', target: '_blank' }, 'kwant'));
  }

  private message(art: HTMLElement, title: string, lead: string, small?: string, ...more: Kid[]): HTMLElement {
    return h('div', { class: 'body mid' }, h('div', { class: 'qmwrap' }, art), h('h2', null, title), lead ? h('p', { class: 'lead' }, lead) : null, small ? h('p', { class: 'small' }, small) : null, ...more);
  }

  /** The keep line (G1, G2): who gets the photos, and that location and camera details are removed. */
  private keep(): HTMLElement {
    const host = this.view?.host_name;
    return h('div', { class: 'keep', 'data-keep': '' }, icon('lock', 'pi'), h('span', null, `Photos go to ${host ?? 'the host'}, who keeps them. Location and camera details are removed.`));
  }

  private problemBox(): HTMLElement | null {
    const p = this.problem;
    if (!p) return null;
    return h('div', { class: 'notice plain', role: 'alert', 'data-problem': '' }, h('span', null, h('b', null, p.title), p.sub ? [h('br'), h('span', { class: 'soft' }, p.sub)] : null));
  }

  private linkButton(text: string, on: () => void, data: string): HTMLElement {
    const b = h('button', { type: 'button', class: 'linkbtn', [`data-${data}`]: '' }, text);
    b.addEventListener('click', on);
    return b;
  }

  private button(text: Kid, on: () => void, data: string, cls = 'gbtn ghost'): HTMLButtonElement {
    const b = h('button', { type: 'button', class: cls, [`data-${data}`]: '' }, text) as HTMLButtonElement;
    b.addEventListener('click', on);
    return b;
  }

  private nameInput(id: string, value: string, on: (s: string) => void, hint?: string): HTMLInputElement {
    const bad = this.problem?.field === 'name';
    const el = h('input', {
      id,
      'data-k': id,
      class: `input${bad ? ' bad' : ''}`,
      type: 'text',
      maxlength: 20,
      autocomplete: 'given-name',
      autocapitalize: 'words',
      spellcheck: 'false',
      enterkeyhint: 'next',
      'aria-describedby': hint ?? null,
      'aria-invalid': bad ? 'true' : null,
    }) as HTMLInputElement;
    el.value = value;
    el.addEventListener('input', () => on(el.value));
    return el;
  }

  /**
   * Once the flagged field reads right, drop the problem in place: the alert and the field's invalid marks go, with
   * no repaint, so focus and typing are untouched.
   */
  private settle(field: Problem['field'], ok: boolean): void {
    if (!ok || !this.problem || this.problem.field !== field) return;
    this.problem = null;
    this.root.querySelector('[data-problem]')?.remove();
    for (const el of this.root.querySelectorAll<HTMLElement>('[aria-invalid]')) {
      el.removeAttribute('aria-invalid');
      el.classList.remove('bad');
      el.closest('.pinwrap')?.querySelectorAll('.pin.warn').forEach((b) => b.classList.remove('warn'));
    }
  }

  private fail(p: Problem): void {
    this.problem = p;
    this.paint(true);
    const focus = p.field === 'name' ? '#w-name, #w-bname' : p.field === 'pin' ? '#w-pin, #w-bpin, #w-newpin' : p.field === 'code' ? '#w-code' : null;
    if (focus) this.root.querySelector<HTMLElement>(focus)?.focus();
  }

  // G1 Join
  private join(v: WallView): HTMLElement {
    const d = this.jd;
    const host = v.host_name;
    const sub = [v.when ? day(v.when) : '', host ? `hosted by ${host}` : ''].filter(Boolean).join(' · ');
    const name = this.nameInput('w-name', d.name, (s) => ((d.name = s), this.settle('name', !!s.trim())), 'name-why');
    const pin = new PinInput('w-pin', 'Pick a 4-digit PIN', (p) => ((d.pin = p), this.settle('pin', p.length === 4)), d.pin, this.problem?.field === 'pin', 'pin-why');
    const lim = v.limits;
    const each = [lim.photo_cap_mb ? `up to ${lim.photo_cap_mb} MB each` : '', lim.per_guest_max ? `${lim.per_guest_max} each` : ''].filter(Boolean).join(', ');
    const n = this.early.length;
    const go =
      this.busy === 'join'
        ? h('button', { class: 'gbtn', type: 'submit', disabled: true, 'aria-busy': 'true' }, 'Joining…')
        : n
          ? h('button', { class: 'gbtn', type: 'submit', 'data-send-early': '' }, `Send ${photos(n)}`)
          : h('button', { class: 'gbtn ghost', type: 'submit', 'data-look': '' }, 'Just look at the wall');
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      this.keep(),
      h('div', { class: 'hero' }, snap(64), h('div', null, h('h1', { id: 'title' }, v.title), sub ? h('div', { class: 'soft sub' }, sub) : null)),
      h('div', null, h('label', { class: 'lbl', for: 'w-name' }, 'Your first name ', h('small', { id: 'name-why' }, 'shown under your photos')), name),
      h('div', null, h('label', { class: 'lbl', for: 'w-pin' }, 'Pick a 4-digit PIN ', h('small', { id: 'pin-why' }, 'to add more tomorrow')), pin.el),
      this.picker('w-files', false),
      n ? h('p', { class: 'soft fine', 'data-early': '' }, `${photos(n)} picked. They send once you join.`) : null,
      h(
        'div',
        { class: 'limits' },
        h('div', null, wIcon('photos'), h('span', null, each ? `Photos only: ${each}.` : 'Photos only.')),
        h('div', null, wIcon('eye'), h('span', null, `They show on the TV and here once ${host ?? 'the host'} says yes.`)),
      ),
      this.problemBox(),
      go,
      h('p', { class: 'center' }, this.linkButton("I've joined before", () => this.goRejoin(), 'to-rejoin')),
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void this.submitJoin();
    });
    return form;
  }

  private goRejoin(): void {
    this.rejoin = true;
    this.problem = null;
    if (!this.bd.name) this.bd.name = this.jd.name;
    this.paint();
    this.root.querySelector<HTMLElement>('#w-bname')?.focus();
  }

  async submitJoin(): Promise<void> {
    const d = this.jd;
    if (!this.view || this.busy) return;
    const name = d.name.trim();
    if (!name) return this.fail({ title: 'Type your first name first.', field: 'name' });
    if (d.pin.length !== 4) return this.fail({ title: 'Pick a 4-digit PIN.', field: 'pin' });
    this.busy = 'join';
    this.problem = null;
    this.paint();
    // The creature comes from the server (a hash of the guest); pa1's join still wants an avatar.
    const r = await this.api.join({ name, avatar: AVATARS[0], pin: d.pin });
    this.busy = '';
    if (r.ok) return this.seated(r.view, () => (d.pin = ''));
    if (r.status === 410 || r.error === 'ended') return void this.onGone(410, 'ended');
    if (r.status === 404) return void this.onGone(404, 'not_found');
    if (r.error === 'closed') return this.refreshInfo();
    if (r.error === 'pin_required') d.pin = '';
    this.fail(pinCopy(r, this.host(), true));
  }

  private seated(v: WallView | undefined, clear: () => void): void {
    this.env.clearTimeout(this.infoTimer);
    clear();
    this.noticesSeen = false;
    this.problem = null;
    if (v) this.show(v);
    this.live.start();
    const files = this.early;
    this.early = [];
    if (files.length && this.view?.open) this.queue.add(files);
    else this.paint(true);
  }

  private async refreshInfo(): Promise<void> {
    const i = await this.api.info();
    if (i.ok && i.view) this.show(i.view);
    else this.paint(true);
  }

  // G5 Welcome back (and a wrong PIN, or a reset code)
  private back(v: WallView): HTMLElement {
    const d = this.bd;
    const host = this.host();
    const warn = this.problem?.field === 'pin';
    let pinPart: Kid;
    if (d.reset) {
      const code = h('input', {
        id: 'w-code',
        'data-k': 'w-code',
        class: `input code${this.problem?.field === 'code' ? ' bad' : ''}`,
        type: 'text',
        maxlength: 4,
        autocomplete: 'off',
        autocapitalize: 'characters',
        spellcheck: 'false',
        'aria-invalid': this.problem?.field === 'code' ? 'true' : null,
      }) as HTMLInputElement;
      code.value = d.code;
      const codeOk = (): boolean => d.code.trim().length === 4 && d.newPin.length === 4;
      code.addEventListener('input', () => ((d.code = code.value), this.settle('code', codeOk())));
      const np = new PinInput('w-newpin', 'Pick a new 4-digit PIN', (p) => ((d.newPin = p), this.settle('code', codeOk())), d.newPin, warn);
      pinPart = [
        h('div', null, h('label', { class: 'lbl', for: 'w-code' }, 'Reset code ', h('small', null, `4 letters from ${host}`)), code),
        h('div', null, h('label', { class: 'lbl', for: 'w-newpin' }, 'Pick a new 4-digit PIN'), np.el),
      ];
    } else {
      const pin = new PinInput('w-bpin', 'Your PIN', (p) => ((d.pin = p), this.settle('pin', p.length === 4)), d.pin, warn);
      pinPart = h('div', null, h('label', { class: 'lbl', for: 'w-bpin' }, 'Your PIN'), pin.el);
    }
    const name = this.nameInput('w-bname', d.name, (s) => ((d.name = s), this.settle('name', !!s.trim())));
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-rejoin': '', disabled: this.busy === 'rejoin', 'aria-busy': this.busy === 'rejoin' ? 'true' : null }, this.busy === 'rejoin' ? 'Opening…' : 'Open my photos');
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      h('div', { class: 'hero' }, snap(64), h('div', null, h('h1', { id: 'title' }, 'Welcome back'), h('div', { class: 'soft sub' }, d.reset ? `Type your first name, the code from ${host} and a new PIN.` : 'Your name and PIN from last night bring your photos back.'))),
      h('div', null, h('label', { class: 'lbl', for: 'w-bname' }, 'Your first name'), name),
      pinPart,
      this.problemBox(),
      btn,
      d.reset
        ? h('p', { class: 'center' }, this.linkButton('Use my PIN instead', () => this.setReset(false), 'use-pin'))
        : [h('p', { class: 'soft center' }, `Forgot it? Ask ${host} for a reset code.`), h('p', { class: 'center' }, this.linkButton('I have a reset code', () => this.setReset(true), 'use-code'))],
      v.open && !v.ended ? h('p', { class: 'center' }, this.linkButton('New here? Add your photos', () => this.leaveRejoin(), 'to-join')) : null,
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void this.submitRejoin();
    });
    return form;
  }

  private setReset(on: boolean): void {
    this.bd.reset = on;
    this.problem = null;
    this.paint();
    this.root.querySelector<HTMLElement>(on ? '#w-code' : '#w-bpin')?.focus();
  }

  private leaveRejoin(): void {
    this.rejoin = false;
    this.problem = null;
    this.paint();
  }

  async submitRejoin(): Promise<void> {
    const d = this.bd;
    if (!this.view || this.busy) return;
    const name = d.name.trim();
    if (!name) return this.fail({ title: 'Type your first name first.', field: 'name' });
    if (d.reset ? d.code.trim().length !== 4 || d.newPin.length !== 4 : d.pin.length !== 4) return this.fail(d.reset ? { title: 'Type the 4-letter code and a new 4-digit PIN.', field: 'code' } : { title: 'Type your 4-digit PIN.', field: 'pin' });
    this.busy = 'rejoin';
    this.problem = null;
    this.paint();
    const body = d.reset ? { name, reset_code: d.code.trim().toUpperCase(), new_pin: d.newPin } : { name, pin: d.pin };
    const r = await this.api.call('rejoin', body);
    this.busy = '';
    if (r.ok) return this.seated(r.view, () => (this.bd = { name: '', pin: '', code: '', newPin: '', reset: false }));
    if (r.status === 410 && r.error !== 'reset_expired') return void this.onGone(410, 'ended');
    if (r.status === 404) return void this.onGone(404, 'not_found');
    if (r.error === 'wrong_pin' || r.error === 'locked' || r.error === 'hard_locked') {
      d.pin = '';
      d.newPin = '';
    }
    if (r.error === 'reset_pending') d.reset = true;
    this.fail(pinCopy(r as Fail, this.host(), false));
  }

  // ---------- photos ----------

  /** One photo from the box, kept across redraws so it never loads twice. `rotate` is applied here (the box strips EXIF). */
  private photo(id: string, rotate: number, alt: string): HTMLImageElement {
    let img = this.imgs.get(id);
    if (!img) {
      img = h('img', { src: `${this.api.base}/photo/${encodeURIComponent(id)}`, loading: 'lazy', decoding: 'async', alt: '', class: rotate ? `r${rotate}` : null }) as HTMLImageElement;
      this.imgs.set(id, img);
    }
    img.alt = alt;
    return img;
  }

  /** Every 5th tile spans both columns at 4:3 (design.md G3), counted from the top of the list. */
  private tile(p: WallPhoto, mine: boolean, i: number): HTMLElement {
    const wide = i % 5 === 0;
    const who = mine ? 'You' : p.by_name;
    const alt = p.caption ? `${p.caption}, by ${who === 'You' ? 'you' : who}` : `Photo by ${who === 'You' ? 'you' : who}`;
    return h(
      'figure',
      { class: `ph${wide ? ' wide' : ''}`, 'data-photo': p.id },
      this.photo(p.id, p.rotate, alt),
      h('figcaption', { class: 'cap', 'aria-hidden': 'true' }, p.caption ? h('span', null, p.caption) : null, h('small', null, who)),
    );
  }

  private pendingTile(p: MinePhoto): HTMLElement {
    const host = this.host();
    const alt = `Your photo${p.caption ? `, ${p.caption}` : ''}, waiting for ${host}`;
    return h(
      'figure',
      { class: 'ph pend', 'data-mine': p.id },
      this.photo(p.id, p.rotate, alt),
      h('span', { class: 'tag', 'aria-hidden': 'true' }, wIcon('clock'), `Waiting for ${host}`),
      h('figcaption', { class: 'cap', 'aria-hidden': 'true' }, p.caption ? h('span', null, p.caption) : null, p.caption_removed ? h('small', null, `${cap(host)} took the caption off`) : null, h('small', null, 'You')),
    );
  }

  /** The guest's own pick as a thumbnail: an object URL of the file, made once per row. */
  private thumb(r: Row<File>): HTMLElement {
    let t = this.thumbs.get(r.key);
    if (!t && this.env.blobs && r.state !== 'refused') {
      const url = this.env.blobs.create(r.file);
      const img = h('img', { src: url, alt: '', decoding: 'async' }) as HTMLImageElement;
      const made = { url, img, bad: false };
      // A file this browser can't draw (a renamed HEIC, a broken file) shows the plain placeholder, not a broken image.
      img.addEventListener('error', () => {
        made.bad = true;
        img.parentElement?.replaceWith(this.noThumb());
      });
      t = made;
      this.thumbs.set(r.key, t);
    }
    return t && !t.bad ? h('span', { class: 'ph', 'aria-hidden': 'true' }, t.img) : this.noThumb();
  }

  private noThumb(): HTMLElement {
    return h('span', { class: 'noimg', 'aria-hidden': 'true' }, wIcon('photos'));
  }

  private row(r: Row<File>, i: number): HTMLElement {
    const v = this.view!;
    const host = this.host();
    const label = `Photo ${i + 1}`;
    let st: Kid;
    let extra: Kid = null;
    let cls = 'ur';
    switch (r.state) {
      case 'sent':
        st = [h('b', null, icon('check', 'pi'), ' Sent'), h('span', { class: 'muted' }, `Waiting for ${host}`)];
        break;
      case 'sending': {
        st = h('b', null, 'Sending…');
        const fill = h('i');
        fill.style.setProperty('--p', String(r.total ? Math.min(1, r.loaded / r.total) : 0));
        extra = h('div', { class: 'bar2', role: 'progressbar', 'aria-label': `${label} sending`, 'aria-valuemin': 0, 'aria-valuemax': r.total || 1, 'aria-valuenow': r.loaded }, fill);
        break;
      }
      case 'next':
        st = h('b', null, 'Next in line');
        break;
      case 'held':
        st = [h('b', null, 'Not sent yet'), h('span', { class: 'muted' }, this.note?.full ? 'waiting for room' : 'kept on this page')];
        break;
      case 'dropped': {
        cls = 'ur err';
        st = [h('b', null, "Didn't send"), h('span', { class: 'muted' }, 'signal dropped')];
        const b = this.button([wIcon('retry'), 'Try again'], () => this.queue.retry(r.key), 'retry', 'retry');
        b.setAttribute('aria-label', `Try ${label} again`);
        extra = b;
        break;
      }
      case 'refused':
        cls = 'ur no';
        st = h('b', { class: 'fname' }, r.file.name || label);
        extra = h('p', { class: 'why' }, h('span', null, r.why?.title ?? ''), r.why?.fix ? [h('br'), h('span', { class: 'soft' }, r.why.fix)] : null);
        // A refusal whose words say "Tap Try again" (broken_photo, the fallback) gets the same button as a dropped row.
        if (r.why?.again) {
          const b = this.button([wIcon('retry'), 'Try again'], () => this.queue.retry(r.key), 'retry', 'retry');
          b.setAttribute('aria-label', `Try ${label} again`);
          extra = [extra, b];
        }
        break;
    }
    const canCaption = v.captions && r.state !== 'refused' && (r.state !== 'sent' || !!r.id);
    let caption: Kid = null;
    if (canCaption) {
      const id = `w-cap-${r.key}`;
      const input = h('input', { id, 'data-k': id, class: 'capin', type: 'text', maxlength: v.limits.caption_max, placeholder: 'Add a caption (optional)', 'aria-label': `Caption for ${label}`, enterkeyhint: 'done', disabled: r.state === 'sending' }) as HTMLInputElement;
      input.value = r.caption;
      input.addEventListener('input', () => {
        if (r.state === 'sent') r.caption = input.value;
        else this.queue.setCaption(r.key, input.value);
      });
      // A sent photo that is still waiting takes a new caption through the kit's caption action.
      if (r.state === 'sent') input.addEventListener('change', () => void this.recaption(r));
      caption = input;
    }
    return h('li', { class: cls, 'data-row': r.key, 'data-state': r.state }, this.thumb(r), h('div', { class: 'urb' }, h('div', { class: 'st' }, st), extra, caption));
  }

  private async recaption(r: Row<File>): Promise<void> {
    if (!r.id) return;
    const caption = [...r.caption.trim()].slice(0, this.view?.limits.caption_max ?? 80).join('');
    const res = await this.api.act('caption', { id: r.id, caption });
    if (res.ok && res.view) this.show(res.view);
    else if (!res.ok) this.showToast(res.message ?? "That caption didn't save. Try again.");
  }

  /** G2 Sending and G4 Can't go up: the rows this page holds, above the wall. */
  private uploads(v: WallView): Kid {
    const rows = this.queue.rows;
    if (!rows.length && !this.note) return null;
    const refused = rows.filter((r) => r.state === 'refused');
    const going = rows.filter((r) => r.state !== 'refused');
    const sent = going.filter((r) => r.state === 'sent').length;
    const out: Kid[] = [];
    if (this.note) {
      const n = this.note;
      out.push(
        h(
          'div',
          { class: 'notice plain', role: 'alert', tabindex: -1, 'data-note': '' },
          wIcon('photos'),
          h('span', null, h('b', null, n.title), n.sub ? [h('br'), h('span', { class: 'soft' }, n.sub)] : null, n.full ? h('span', { class: 'acts' }, this.button('Try again', () => this.retryHeld(), 'retry-held', 'gbtn ghost sm')) : null),
        ),
      );
    }
    if (refused.length) {
      out.push(
        h(
          'div',
          { 'data-refused': '' },
          h('h2', null, refused.length === 1 ? "This one can't go up" : `${refused.length} of these can't go up`),
          going.length ? h('p', { class: 'soft sub' }, going.length === 1 ? 'The other one is sending.' : `The other ${going.length} are sending.`) : null,
        ),
        h('ul', { class: 'upl', 'aria-label': "Can't go up" }, refused.map((r) => this.row(r, rows.indexOf(r)))),
        h('p', { class: 'soft fine' }, 'iPhone photos are changed to JPEG on your phone before they’re sent.'),
        this.button('Back to the wall', () => this.clearRefused(), 'clear-refused'),
      );
    }
    if (going.length) {
      out.push(
        h('div', { 'data-sending': '' }, h('h2', null, `Sending ${photos(going.length)}`), h('p', { class: 'soft sub' }, `${sent} sent · keep this page open until they’re done.`)),
        h('ul', { class: 'upl', 'aria-label': 'Your photos sending' }, going.map((r) => this.row(r, rows.indexOf(r)))),
      );
      if (v.open && !this.note) out.push(h('label', { class: 'gbtn ghost', for: 'w-more', 'data-add-more': '' }, wIcon('plus'), 'Add more', this.fileInput('w-more')));
    }
    return h('section', { class: 'sending', 'aria-label': 'Your uploads' }, out);
  }

  private retryHeld(): void {
    this.note = null;
    this.queue.resume();
  }

  private clearRefused(): void {
    for (const r of this.queue.rows.filter((x) => x.state === 'refused')) this.queue.remove(r.key);
    if (this.note && !this.note.full) {
      for (const r of this.queue.rows.filter((x) => x.state === 'held')) this.queue.remove(r.key);
      this.note = null;
    }
    this.paint();
    this.root.querySelector<HTMLElement>('[data-add], h2')?.focus();
  }

  /** The approved wall, newest first, 12 at a time, new ones behind the pill. */
  private grid(v: WallView, list: WallPhoto[], paged: boolean): Kid[] {
    const mine = new Set(v.mine.map((p) => p.id));
    if (!paged) return [h('div', { class: 'gwall', 'data-wall': '' }, list.map((p, i) => this.tile(p, mine.has(p.id), i)))];
    const page = wallPage(list, this.shown, this.limit);
    this.shown = page.photos.map((p) => p.id);
    const out: Kid[] = [];
    if (page.fresh.length) {
      out.push(
        this.button([wIcon('up'), page.fresh.length === 1 ? '1 new photo' : `${page.fresh.length} new photos`], () => {
          this.shown = [];
          this.limit = PAGE;
          this.paint();
          this.root.querySelector<HTMLElement>('[data-wall-top]')?.focus();
        }, 'newpill', 'newpill'),
      );
    }
    out.push(h('div', { class: 'gwall', 'data-wall': '' }, page.photos.map((p, i) => this.tile(p, mine.has(p.id), i))));
    if (page.more) out.push(this.button('Show more photos', () => ((this.limit += PAGE), this.paint()), 'more'));
    return out;
  }

  // G2, G3, G4: the guest's rows, their own waiting photos, then the wall
  private wall(v: WallView): HTMLElement {
    const host = this.host();
    const pending = v.mine.filter((p) => p.status === 'pending');
    const kids: Kid[] = [];
    const sending = this.uploads(v);
    if (sending) kids.push(this.keep(), sending);
    else if (this.thanks && v.me) {
      kids.push(
        h('div', { class: 'say', role: 'status', 'data-thanks': '' }, snap(36, 'finished'), h('div', { class: 'b' }, `Thanks, ${v.me.name}! Your ${photos(this.thanks)} ${this.thanks === 1 ? 'is' : 'are'} with ${host}. They show here and on the TV once ${host} says yes.`)),
      );
    }
    if (this.pick === 'all' && !v.open) kids.push(h('p', null, this.linkButton('Back', () => this.setPick('auto'), 'back')));
    if (pending.length) {
      kids.push(h('section', { 'aria-labelledby': 'yours' }, h('h2', { class: 'lbl', id: 'yours' }, 'Yours ', h('small', null, `${pending.length} waiting · only you see these`)), h('div', { class: 'gwall' }, pending.map((p) => this.pendingTile(p)))));
    }
    if (v.wall.length) {
      kids.push(h('section', { 'aria-labelledby': 'thewall', class: 'wallsec' }, h('h2', { class: 'lbl', id: 'thewall', tabindex: -1, 'data-wall-top': '' }, 'The wall'), this.grid(v, v.wall, true)));
    } else if (!sending) {
      kids.push(
        h('div', { class: 'empty', 'data-empty': '' }, snap(96, undefined, true), h('h2', null, 'No photos on the wall yet'), h('p', { class: 'soft' }, `They show here once ${host} says yes.${v.open ? ' Yours could be the first.' : ''}`)),
      );
    }
    if (v.open && !this.queue.rows.length) kids.push(h('div', { class: 'stick' }, this.picker('w-add', true)));
    return h('div', { class: 'body' }, kids);
  }

  private setPick(p: Pick): void {
    this.pick = p;
    this.shown = [];
    this.limit = PAGE;
    this.paint();
    this.root.querySelector<HTMLElement>('h1, h2')?.focus?.();
  }

  // G6 The best
  private best(v: WallView): HTMLElement {
    const host = this.host();
    const list = v.wall.filter((p) => p.best);
    const mineN = list.filter((p) => v.mine.some((m) => m.id === p.id)).length;
    return h(
      'div',
      { class: 'body' },
      h(
        'div',
        { class: 'hero' },
        snap(64, 'finished'),
        h('div', null, h('h1', { id: 'title', tabindex: -1 }, `The best ${list.length}`), h('div', { class: 'soft sub' }, `Picked by ${host}.${mineN ? ` ${mineN === 1 ? '1 of yours is' : `${mineN} of yours are`} in.` : ''}`)),
        // Holo only when a check earned it: Snap's pick (pa14b, `best_by: "snap"`). A host's own pick gets no sticker.
        v.best_by === 'snap' ? h('span', { class: 'kw-sticker kw-sticker-holo', 'data-sticker': 'holo' }, wIcon('eye'), h('span', null, 'Picked')) : null,
      ),
      this.grid(v, list, false),
      this.button('See all the photos', () => this.setPick('all'), 'see-all'),
      v.deletes_on ? h('p', { class: 'small center' }, `This page closes on ${day(v.deletes_on)}. ${cap(host)} keeps the photos after that.`) : null,
    );
  }

  // G7 Closed
  private closed(v: WallView): HTMLElement {
    const host = this.host();
    const more: Kid[] = [];
    if (v.me && hasBest(v)) more.push(this.button(`See the best ${v.wall.filter((p) => p.best).length}`, () => this.setPick('best'), 'see-best', 'gbtn'));
    if (v.me && v.wall.length) more.push(this.button('See all the photos', () => this.setPick('all'), 'see-all'));
    if (!v.me && !v.ended) more.push(this.button("I've joined before: open my photos", () => this.goRejoin(), 'to-rejoin'));
    const unsent = this.closedUnsent ? `${photos(this.closedUnsent)} didn't go up before it closed. ` : '';
    const lead = v.me ? `${unsent}The photos went to ${host}. Thanks for adding yours!` : `The photos went to ${host}.`;
    const small = v.deletes_on ? `This page closes on ${day(v.deletes_on)}.${v.me ? " Your PIN won't be needed after that." : ''}` : undefined;
    return this.message(snap(96, 'finished'), 'The wall is closed', lead, small, ...more);
  }

  // G8 Away
  private awayLine(): string {
    const a = this.away!;
    return `Trying again in ${a.left} s · tried ${a.tries} time${a.tries === 1 ? '' : 's'}`;
  }

  private awayScreen(): HTMLElement {
    const host = this.host();
    const unsent = this.queue.unsent;
    const lead = unsent.length
      ? `Your ${unsent.length === 1 ? 'unsent photo is' : `${unsent.length} unsent photos are`} still on this page. Keep it open and ${unsent.length === 1 ? 'it' : 'they'}'ll send on ${unsent.length === 1 ? 'its' : 'their'} own when it's back.`
      : `This page lives on ${poss(host)} computer, and it's off or asleep right now. The page tries again by itself.`;
    const rows = this.queue.rows;
    return h(
      'div',
      { class: 'body' },
      h('div', { class: 'mid', role: 'status' }, h('div', { class: 'qmwrap' }, snap(96, 'thinking')), h('h2', null, `${cap(poss(host))} computer is resting`), h('p', { class: 'lead' }, lead), h('p', { class: 'small', 'data-away': '' }, this.awayLine())),
      unsent.length ? h('ul', { class: 'upl', 'aria-label': 'Your unsent photos' }, unsent.map((r) => this.row(r, rows.indexOf(r)))) : null,
      this.button('Try now', this.tryNow, 'try'),
    );
  }
}

