// Secret Santa's guest page, G1-G11 in pa10a/guest-390.html, on Lin's santa API (pa10a/server-impl.md). Loaded as
// its own chunk when the pocket's kit is `santa`; the quiz screens in ../app.ts are untouched. Plain DOM, no
// framework; text goes in as text only. Every number shown comes from the server except "Days left" (from gift_on, on
// this device, as the brief asks) and the character count of what the guest is typing.
import { Api, Env, Fail, Live } from '../api';
import { AVATARS, creature, fmt, h, icon, Kid } from '../dom';
import { CREATURE_NAMES } from '../art';
import { clock, day } from '../app';
import { giftbox, saIcon, santa } from './look';
import { cap, PinInput, pinCopy, Problem } from './pin';
import { Ask, santaFrom, SantaView, Screen, screenOf } from './view';

const TOAST_MS = 4000;
const INFO_MS = 10_000;
const CHIPS = ['Shoe size?', 'Favourite colour?', "Anything you don't like?"];
const WISH_MAX = 300;

export interface Store {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

export type SantaEnv = Env & {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
  /** localStorage when the browser allows it: remembers that this device unwrapped the gift. */
  store?: Store | null;
};

/** "Dec 18" (a calendar day, read in UTC like ../app.ts `day`). */
export function shortDay(iso: string): string {
  const d = new Date(`${iso.slice(0, 10)}T12:00:00Z`);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
}

/** Whole days from this phone's today to the gift day (0 on the day, negative after). */
export function daysLeft(giftOn: string, now: number): number | undefined {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(giftOn);
  if (!m) return undefined;
  const t = new Date(now);
  const today = Date.UTC(t.getFullYear(), t.getMonth(), t.getDate());
  return Math.round((Date.UTC(+m[1], +m[2] - 1, +m[3]) - today) / 86_400_000);
}

/** "today 8:12 AM" for this phone's today, else "Dec 8". */
export function when(iso: string | undefined, now: number): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const t = new Date(now);
  if (d.getFullYear() === t.getFullYear() && d.getMonth() === t.getMonth() && d.getDate() === t.getDate()) return `today ${clock(iso)}`;
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

const poss = (name: string): string => `${name}'s`;
const chars = (s: string): number => [...s].length;

type JoinDraft = { name: string; avatar: string; pin: string; wish: string };
type BackDraft = { name: string; pin: string; code: string; newPin: string; reset: boolean };

export class SantaApp {
  view: SantaView | null = null;
  readonly api: Api<SantaView>;
  private live: Live<SantaView>;
  private root: HTMLElement;
  private sig = '';
  private away: { secs: number; tries: number; left: number } | null = null;
  private awayTick = 0;
  private infoTimer = 0;
  private toastTimer = 0;
  private toast: HTMLElement | null = null;
  private gone: 'removed' | 'missing' | null = null;
  private hadSeat = false;
  // Screens the guest picked
  private rejoin = false;
  private joinLate = false;
  private lateNote = '';
  private wishOpen = false;
  /** Just unwrapped (when): the match card pops in, also if a reply redraws it within the pop. */
  private poppedAt = -1;
  private opening = false;
  // What the guest typed: kept across redraws, the box going away and coming back.
  private jd: JoinDraft = { name: '', avatar: AVATARS[0], pin: '', wish: '' };
  private bd: BackDraft = { name: '', pin: '', code: '', newPin: '', reset: false };
  private problem: Problem | null = null;
  private busy = '';
  private wishDraft: string | null = null;
  private askDraft = '';
  private replies = new Map<string, string>();
  /** Errors under the ask, reply and wish forms (the server's message, as is). */
  private formErr = new Map<string, string>();
  /** Questions shown as New on this page load: they stay marked until reload after `seen_asks` clears them. */
  private fresh = new Set<string>();
  private seenSent = '';
  private noticesSeen = false;

  constructor(root: HTMLElement, private env: SantaEnv, readonly code: string) {
    this.root = root;
    this.api = new Api<SantaView>(env, code, santaFrom);
    this.live = new Live<SantaView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => this.goAway(secs, tries),
      back: () => this.comeBack(),
      gone: (status, error) => void this.onGone(status, error),
    });
  }

  /** Same boot as the quiz: the page's public facts first, then `state` finds out whether this phone has a seat. */
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

  /** Keeps a view without drawing it. */
  private take(v: SantaView): void {
    this.view = v;
    for (const a of v.asks.received) if (a.new) this.fresh.add(a.id);
    for (const a of v.asks.sent) if (a.new) this.fresh.add(a.id);
  }

  show(v: SantaView): void {
    if (this.gone) return;
    this.take(v);
    if (v.me) {
      this.hadSeat = true;
      this.rejoin = false;
      this.joinLate = false;
    }
    this.paint();
    this.afterShow(v);
  }

  /** Things a screen does once it is drawn: open a box this device unwrapped before, mark questions seen, clear notices. */
  private afterShow(v: SantaView): void {
    if (!v.me || this.away) return;
    const scr = this.screenNow();
    if (scr === 'unwrap' && !v.match && v.can_open && !v.match_changed && this.unwrapped()) void this.open(false);
    const unseen = [...v.asks.received, ...v.asks.sent].filter((a) => a.new).map((a) => a.id).join(',');
    if (scr === 'match' && unseen && unseen !== this.seenSent) {
      this.seenSent = unseen;
      void this.api.act('seen_asks').then((r) => r.ok && r.view && this.show(r.view));
    }
    if (v.me.notices.length && !this.noticesSeen) {
      this.noticesSeen = true;
      const n = v.me.notices.find((x) => x.kind === 'pin_reset') ?? v.me.notices[0];
      const host = this.host();
      this.showToast(
        n.kind === 'pin_reset'
          ? `${cap(host)} reset your PIN${n.at ? ` on ${day(n.at)}` : ''}. ${cap(host)} still can't see your match.`
          : `Someone tried to open your page with a wrong PIN${n.at ? ` on ${day(n.at)}` : ''}. Your match is still safe.`,
      );
      void this.api.call('notices/seen', {});
    }
  }

  private async onGone(status: number, error?: string): Promise<void> {
    if (status === 401 || error === 'signed_out') {
      // The seat is gone from this phone (a PIN reset ends old sign-ins): back to the public facts, then name and PIN.
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
      this.env.clearTimeout(this.infoTimer);
      this.away = null;
      if (this.view) this.show({ ...this.view, ended: true, me: undefined });
      else this.end('missing');
      return;
    }
    if (status === 403 || error === 'removed') this.end('removed');
    else if (status === 404 || error === 'not_found') this.end('missing');
  }

  private end(why: 'removed' | 'missing'): void {
    this.env.clearTimeout(this.infoTimer);
    this.live.stop();
    this.away = null;
    this.gone = why;
    this.paint();
  }

  /** Before a seat there is no socket: the facts (joined count, phase) are read again every 10 s. */
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

  // ---------- device memory ----------

  private flagKey(): string {
    return `kp-santa-open:${this.code}:${this.view?.me?.id ?? ''}`;
  }

  private unwrapped(): boolean {
    try {
      return this.env.store?.getItem(this.flagKey()) === '1';
    } catch {
      return false;
    }
  }

  private remember(): void {
    try {
      this.env.store?.setItem(this.flagKey(), '1');
    } catch {
      /* private mode: the box shows again next time */
    }
  }

  // ---------- drawing ----------

  private host(): string {
    return this.view?.facts.host_name ?? 'the host';
  }

  screenNow(): Screen | 'away' | 'removed' | 'missing' | 'loading' {
    if (this.gone) return this.gone;
    if (this.away) return 'away';
    const v = this.view;
    if (!v) return 'loading';
    return screenOf(v, { rejoin: this.rejoin, joinLate: this.joinLate, unwrapped: this.unwrapped() });
  }

  /** Draws the screen when something it shows changed. Typing never redraws; focus and caret survive a redraw. */
  paint(force = false): void {
    const scr = this.screenNow();
    const sig = JSON.stringify([scr, this.view, this.problem, this.busy, this.wishOpen, this.lateNote, this.bd.reset, [...this.formErr], this.popping(), this.opening, this.away?.tries]);
    if (!force && sig === this.sig) return;
    this.sig = sig;
    const active = document.activeElement as HTMLInputElement | null;
    const k = active?.getAttribute?.('data-k');
    const sel = k && typeof active?.selectionStart === 'number' ? [active.selectionStart, active.selectionEnd ?? active.selectionStart] : null;
    const scrollY = typeof window !== 'undefined' ? window.scrollY : 0;
    const page = h('main', { class: `g sa s-${scr}`, 'data-screen': scr }, this.bar(scr), this.body(scr), this.madeWith());
    this.root.replaceChildren(page);
    if (k) {
      const el = this.root.querySelector<HTMLInputElement>(`[data-k="${k}"]`);
      if (el) {
        el.focus({ preventScroll: true });
        if (sel) el.setSelectionRange?.(sel[0], sel[1]);
      }
    } else this.root.querySelector<HTMLElement>('[data-autofocus]')?.focus({ preventScroll: true });
    if (typeof window !== 'undefined' && scrollY && window.scrollTo) window.scrollTo(0, scrollY);
  }

  private popping(): boolean {
    return this.poppedAt >= 0 && this.env.now() - this.poppedAt < 300;
  }

  private body(scr: ReturnType<SantaApp['screenNow']>): HTMLElement {
    const v = this.view;
    switch (scr) {
      case 'loading':
        return this.message(santa(96, 'thinking'), 'Opening…', '');
      case 'removed':
        return this.message(santa(96), "You've been removed from this Secret Santa", `${cap(this.host())} took you out of it. If that's a mistake, have a word with ${this.host()}.`);
      case 'missing':
        return this.message(santa(96), 'Nothing here', "This link doesn't open a pocket app any more. Check the link with the host.");
      case 'away':
        return this.awayScreen();
      case 'join':
        return this.join(v!);
      case 'rejoin':
        return this.back(v!);
      case 'late':
        return this.late(v!);
      case 'waiting':
        return this.waiting(v!);
      case 'unwrap':
        return this.unwrap(v!);
      case 'match':
        return this.match(v!);
      case 'reveal':
        return this.reveal(v!);
      case 'ended':
        return this.ended(v!);
    }
  }

  private bar(scr: string): HTMLElement {
    const v = this.view;
    let right = '';
    if (v && (scr === 'match' || scr === 'unwrap') && v.me) right = v.me.name;
    else if (v && scr === 'reveal') right = 'Reveal day';
    else if (v && !['ended', 'loading', 'removed', 'missing'].includes(scr)) {
      right = [v.facts.budget, v.facts.gift_on ? shortDay(v.facts.gift_on) : ''].filter(Boolean).join(' · ');
    }
    return h('header', { class: 'bar' }, santa(28), h('b', null, v?.title ?? ''), h('span', { class: 'sp' }), h('span', { class: 'muted tab', 'data-bar': '' }, right));
  }

  private madeWith(): HTMLElement {
    return h('footer', { class: 'madewith' }, 'made with ', h('a', { href: 'https://kwant.si', rel: 'noopener noreferrer', target: '_blank' }, 'kwant'));
  }

  private message(art: HTMLElement, title: string, lead: string, small?: string, ...more: Kid[]): HTMLElement {
    return h(
      'div',
      { class: 'body mid' },
      h('div', { class: 'qmwrap' }, art),
      h('h2', null, title),
      lead ? h('p', { class: 'lead' }, lead) : null,
      small ? h('p', { class: 'small' }, small) : null,
      ...more,
    );
  }

  /** The keep line: the deletion date from `pocket.expires_at`, before anything to type. */
  private keep(v: SantaView, full = true): HTMLElement | null {
    const at = v.expires_at;
    if (!at) return null;
    const host = this.host();
    const after = v.facts.reveal === 'scheduled' || v.facts.reveal === 'shown' ? 'the reveal' : 'the gift day';
    const why = v.keep_days !== undefined ? ` (${v.keep_days} day${v.keep_days === 1 ? '' : 's'} after ${after})` : '';
    const text = full ? `Kept on ${poss(host)} computer until ${day(at)}${why}, then deleted.` : `Everything here is deleted on ${day(at)}.`;
    return h('div', { class: 'keep', 'data-keep': '' }, icon('lock', 'pi'), h('span', null, text));
  }

  private hero(title: string, sub: string): HTMLElement {
    return h('div', { class: 'hero' }, santa(48), h('div', null, h('h1', { id: 'title' }, title), h('div', { class: 'soft sub' }, sub)));
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

  private nameInput(id: string, value: string, on: (s: string) => void, hint?: string): HTMLInputElement {
    const el = h('input', {
      id,
      'data-k': id,
      class: `input${this.problem?.field === 'name' ? ' bad' : ''}`,
      type: 'text',
      maxlength: 20,
      autocomplete: 'nickname',
      autocapitalize: 'words',
      spellcheck: 'false',
      enterkeyhint: 'next',
      'aria-describedby': hint ?? null,
      'aria-invalid': this.problem?.field === 'name' ? 'true' : null,
    }) as HTMLInputElement;
    el.value = value;
    el.addEventListener('input', () => on(el.value));
    return el;
  }

  // G1 Join
  private join(v: SantaView): HTMLElement {
    const d = this.jd;
    const f = v.facts;
    const host = f.host_name;
    const sub = [host ? `Hosted by ${host}` : 'Hosted with Santa', f.budget ? `${f.budget} each` : '', f.gift_on ? `gifts on ${day(f.gift_on)}` : ''].filter(Boolean).join(' · ');
    const joined = f.joined !== undefined && f.joined > 0 ? (f.expected ? `${fmt(f.joined)} of ${fmt(f.expected)} joined` : `${fmt(f.joined)} joined`) : '';
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-join': '', 'aria-busy': this.busy === 'join' ? 'true' : null }, this.busy === 'join' ? 'Joining…' : 'Join the draw') as HTMLButtonElement;
    const ready = () => (btn.disabled = this.busy === 'join' || !d.name.trim() || d.pin.length !== 4);
    const name = this.nameInput('s-name', d.name, (s) => {
      d.name = s;
      ready();
    });
    const crow = h('div', { class: 'crow', role: 'radiogroup', 'aria-labelledby': 'lbl-creature' });
    AVATARS.forEach((a, i) => {
      const input = h('input', { type: 'radio', name: 'creature', value: a, class: 'vh', 'aria-label': CREATURE_NAMES[i] }) as HTMLInputElement;
      input.checked = d.avatar === a;
      const tile = h('label', { class: `creature${input.checked ? ' on' : ''}` }, input, creature(a, 'cr'), h('span', { class: 'tick' }, icon('check', 'pi')));
      input.addEventListener('change', () => {
        d.avatar = a;
        crow.querySelectorAll('.creature').forEach((c) => c.classList.toggle('on', c === tile));
      });
      crow.append(tile);
    });
    const pin = new PinInput('s-pin', 'Pick a 4-digit PIN', (p) => {
      d.pin = p;
      ready();
    }, d.pin, this.problem?.field === 'pin', 'pin-why');
    const wish = h('textarea', { id: 's-wish', 'data-k': 's-wish', class: 'wish', rows: 3, maxlength: WISH_MAX, 'aria-describedby': 'wish-why' }) as HTMLTextAreaElement;
    wish.value = d.wish;
    wish.addEventListener('input', () => (d.wish = wish.value));
    ready();
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      this.keep(v),
      this.hero(v.title, sub),
      h('div', null, h('label', { class: 'lbl', for: 's-name' }, 'Your name ', joined ? h('small', null, joined) : null), name),
      h('div', null, h('div', { class: 'lbl', id: 'lbl-creature' }, 'Your creature ', h('small', null, 'the group sees it')), crow),
      h(
        'div',
        null,
        h('label', { class: 'lbl', for: 's-pin' }, 'Pick a 4-digit PIN'),
        pin.el,
        h('p', { class: 'soft fine', id: 'pin-why' }, "So only you can see who you're buying for. Not the host, not anyone. You'll need your name and PIN to come back."),
      ),
      h(
        'div',
        null,
        h('label', { class: 'lbl', for: 's-wish' }, 'Wish list ', h('small', null, 'optional')),
        wish,
        h('p', { class: 'small fine', id: 'wish-why' }, 'Only your Santa sees it. You can change it until the gift day.'),
      ),
      this.problemBox(),
      btn,
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
    this.root.querySelector<HTMLElement>('#s-bname')?.focus();
  }

  async submitJoin(): Promise<void> {
    const d = this.jd;
    const v = this.view;
    if (!v || this.busy) return;
    const name = d.name.trim();
    if (!name) return this.fail({ title: 'Type your name first.', field: 'name' });
    if (d.pin.length !== 4) return this.fail({ title: 'Pick a 4-digit PIN.', field: 'pin' });
    this.busy = 'join';
    this.problem = null;
    this.paint();
    const r = await this.api.join({ name, avatar: d.avatar, pin: d.pin });
    this.busy = '';
    if (r.ok) {
      this.env.clearTimeout(this.infoTimer);
      d.pin = '';
      if (r.view) this.show(r.view);
      this.live.start();
      // The wish list goes right after the seat; a refusal keeps the text in the editor.
      const wish = d.wish.trim();
      if (wish) {
        const w = await this.api.act('set_wish', { text: wish });
        if (w.ok) {
          d.wish = '';
          if (w.view) this.show(w.view);
        } else {
          this.wishOpen = true;
          this.wishDraft = d.wish;
          this.formErr.set('wish', this.formMessage(w));
          this.paint();
        }
      }
      return;
    }
    if (r.error === 'not_now') {
      // Joining after the draw (G10), with the server's own reason.
      this.joinLate = false;
      this.lateNote = r.message ?? '';
      const i = await this.api.info();
      if (i.ok && i.view) this.view = i.view;
      this.paint();
      return;
    }
    if (r.status === 410 || r.error === 'ended') return void this.onGone(410, 'ended');
    if (r.status === 404) return void this.onGone(404, 'not_found');
    if (r.error === 'not_open_yet') return this.fail({ title: r.message ?? "This Secret Santa isn't open yet. Try again in a minute." });
    if (r.error === 'full') return this.fail({ title: r.message ?? 'This Secret Santa is full.' });
    if (r.error === 'pin_required' || r.error === 'bad_avatar') d.pin = '';
    this.fail(pinCopy(r, this.host(), true));
  }

  private fail(p: Problem): void {
    this.problem = p;
    this.paint(true);
    const focus = p.field === 'name' ? '#s-name, #s-bname' : p.field === 'pin' ? '#s-pin, #s-bpin, #s-newpin' : p.field === 'code' ? '#s-code' : null;
    if (focus) this.root.querySelector<HTMLElement>(focus)?.focus();
  }

  // G3 Rejoin, G4 Wrong PIN
  private back(v: SantaView): HTMLElement {
    const d = this.bd;
    const host = this.host();
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-rejoin': '', 'aria-busy': this.busy === 'rejoin' ? 'true' : null }, this.busy === 'rejoin' ? 'Opening…' : 'Open my page') as HTMLButtonElement;
    const ready = () =>
      (btn.disabled = this.busy === 'rejoin' || !d.name.trim() || (d.reset ? d.code.trim().length !== 4 || d.newPin.length !== 4 : d.pin.length !== 4));
    const name = this.nameInput('s-bname', d.name, (s) => {
      d.name = s;
      ready();
    });
    const warn = this.problem?.field === 'pin';
    let pinPart: Kid;
    if (d.reset) {
      const code = h('input', {
        id: 's-code',
        'data-k': 's-code',
        class: `input code${this.problem?.field === 'code' ? ' bad' : ''}`,
        type: 'text',
        maxlength: 4,
        autocomplete: 'off',
        autocapitalize: 'characters',
        spellcheck: 'false',
        'aria-invalid': this.problem?.field === 'code' ? 'true' : null,
      }) as HTMLInputElement;
      code.value = d.code;
      code.addEventListener('input', () => {
        d.code = code.value;
        ready();
      });
      const np = new PinInput('s-newpin', 'Pick a new 4-digit PIN', (p) => {
        d.newPin = p;
        ready();
      }, d.newPin, warn);
      pinPart = [
        h('div', null, h('label', { class: 'lbl', for: 's-code' }, 'Reset code ', h('small', null, `4 letters from ${host}`)), code),
        h('div', null, h('label', { class: 'lbl', for: 's-newpin' }, 'Pick a new 4-digit PIN'), np.el),
      ];
    } else {
      const pin = new PinInput('s-bpin', 'Your PIN', (p) => {
        d.pin = p;
        ready();
      }, d.pin, warn);
      pinPart = h('div', null, h('label', { class: 'lbl', for: 's-bpin' }, 'Your PIN'), pin.el);
    }
    ready();
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      this.hero('Welcome back', d.reset ? `Type your name, the code from ${host} and a new PIN.` : 'Type your name and PIN to see your match.'),
      h('div', null, h('label', { class: 'lbl', for: 's-bname' }, 'Your name'), name),
      pinPart,
      this.problemBox(),
      btn,
      d.reset
        ? h('p', { class: 'center' }, this.linkButton('Use my PIN instead', () => this.setReset(false), 'use-pin'))
        : [
            h('p', { class: 'soft center' }, `Forgot your PIN? Ask ${host} for a reset code. ${cap(host)} still won't see your match.`),
            h('p', { class: 'center' }, this.linkButton('I have a reset code', () => this.setReset(true), 'use-code')),
          ],
      v.facts.phase === 'collecting' && !v.ended ? h('p', { class: 'center' }, this.linkButton('New here? Join the draw', () => this.leaveRejoin(), 'to-join')) : null,
      this.keep(v),
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
    this.root.querySelector<HTMLElement>(on ? '#s-code' : '#s-bpin')?.focus();
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
    if (!name) return this.fail({ title: 'Type your name first.', field: 'name' });
    this.busy = 'rejoin';
    this.problem = null;
    this.paint();
    const body = d.reset ? { name, reset_code: d.code.trim().toUpperCase(), new_pin: d.newPin } : { name, pin: d.pin };
    const r = await this.api.call('rejoin', body);
    this.busy = '';
    if (r.ok) {
      this.env.clearTimeout(this.infoTimer);
      this.bd = { name: '', pin: '', code: '', newPin: '', reset: false };
      this.noticesSeen = false;
      if (r.view) this.show(r.view);
      this.live.start();
      return;
    }
    if (r.status === 410 && r.error !== 'reset_expired') return void this.onGone(410, 'ended');
    if (r.status === 404) return void this.onGone(404, 'not_found');
    // A wrong PIN empties the boxes so the next try starts clean.
    if (r.error === 'wrong_pin' || r.error === 'locked' || r.error === 'hard_locked') {
      d.pin = '';
      d.newPin = '';
    }
    if (r.error === 'reset_pending') d.reset = true;
    this.fail(pinCopy(r, this.host(), false));
  }

  // G10 Late
  private late(v: SantaView): HTMLElement {
    const host = this.host();
    const open = h('button', { type: 'button', class: 'gbtn ghost', 'data-to-rejoin': '' }, "I've joined before: open my page");
    open.addEventListener('click', () => this.goRejoin());
    return this.message(
      santa(96),
      'The names are already drawn',
      this.lateNote || `The names were drawn already. If you should be in it, ask ${host} to add you.`,
      undefined,
      open,
      h('p', { class: 'center' }, this.linkButton(`${cap(host)} added me: join now`, () => this.goJoinLate(), 'join-late')),
      this.keep(v),
    );
  }

  private goJoinLate(): void {
    this.joinLate = true;
    this.lateNote = '';
    this.problem = null;
    this.paint();
  }

  // G2 Waiting for the draw
  private waiting(v: SantaView): HTMLElement {
    const me = v.me!;
    const f = v.facts;
    const host = this.host();
    const late = f.phase !== 'collecting';
    const progress =
      !late && f.joined !== undefined && f.expected
        ? [
            h('div', { class: 'soft' }, 'Waiting for the draw: ', h('b', { class: 'tab' }, `${fmt(f.joined)} of ${fmt(f.expected)}`), ' joined'),
            h(
              'div',
              { class: 'timer', role: 'progressbar', 'aria-valuenow': f.joined, 'aria-valuemin': 0, 'aria-valuemax': f.expected, 'aria-label': `${fmt(f.joined)} of ${fmt(f.expected)} joined` },
              h('i', { class: `w${Math.min(20, Math.round((f.joined / f.expected) * 20))}` }),
            ),
          ]
        : h('div', { class: 'soft' }, late ? `The names are already drawn. ${cap(host)} is fitting you in.` : f.joined !== undefined ? ['Waiting for the draw: ', h('b', { class: 'tab' }, fmt(f.joined)), ' joined'] : 'Waiting for the draw');
    const at = f.draw_at ? `${day(f.draw_at)} at ${clock(f.draw_at)}` : '';
    const line = late
      ? "As soon as you're in the loop, this page shows who you're buying for."
      : f.expected && at
        ? `I'll draw names when everyone's in, or on ${at}. Then this page shows who you're buying for.`
        : f.expected
          ? "I'll draw names when everyone's in. Then this page shows who you're buying for."
          : at
            ? `I'll draw names on ${at}. Then this page shows who you're buying for.`
            : `I'll draw names when ${host} says go. Then this page shows who you're buying for.`;
    const soon = !late && f.expected ? Math.max(0, Math.min(60, f.expected - v.people.length)) : 0;
    const grid = h(
      'div',
      { class: 'who-grid' },
      v.people.map((p) => h('div', null, creature(p.avatar, 'sm'), h('div', { class: 'nm' }, p.name))),
      Array.from({ length: soon }, () => h('div', { 'aria-hidden': 'true' }, h('div', { class: 'empty' }, '?'), h('div', { class: 'muted' }, 'soon'))),
    );
    return h(
      'div',
      { class: 'body' },
      h('div', { class: 'panel center you' }, creature(me.avatar, 'big', `${me.name}'s creature`), h('h2', null, `You're in, ${me.name}!`), progress),
      h('div', { class: 'say' }, santa(36), h('div', { class: 'b' }, line)),
      h('div', null, h('div', { class: 'lbl' }, "Who's in"), grid),
      this.wishEditor(v),
      h('div', { class: 'notice plain small' }, 'Tip: add this page to your home screen. The link stays the same until the gift day.'),
      this.keep(v),
    );
  }

  /** "Edit my wish list" (G2, and under the match): opens a box with Save. Allowed until the gift day. */
  private wishEditor(v: SantaView): HTMLElement {
    const label = v.my_wish ? 'Edit my wish list' : 'Add a wish list';
    if (!this.wishOpen) {
      const b = h('button', { type: 'button', class: 'gbtn ghost', 'data-edit-wish': '' }, label);
      b.addEventListener('click', () => {
        this.wishOpen = true;
        this.wishDraft = v.my_wish ?? '';
        this.paint();
        this.root.querySelector<HTMLElement>('#s-mywish')?.focus();
      });
      return b;
    }
    const ta = h('textarea', { id: 's-mywish', 'data-k': 's-mywish', class: 'wish', rows: 4, maxlength: WISH_MAX, 'aria-describedby': 'mywish-why mywish-err' }) as HTMLTextAreaElement;
    ta.value = this.wishDraft ?? v.my_wish ?? '';
    ta.addEventListener('input', () => (this.wishDraft = ta.value));
    const save = h('button', { type: 'submit', class: 'gbtn', 'aria-busy': this.busy === 'wish' ? 'true' : null }, this.busy === 'wish' ? 'Saving…' : 'Save wish list') as HTMLButtonElement;
    save.disabled = this.busy === 'wish';
    const cancel = h('button', { type: 'button', class: 'gbtn ghost' }, 'Cancel');
    cancel.addEventListener('click', () => {
      this.wishOpen = false;
      this.wishDraft = null;
      this.formErr.delete('wish');
      this.paint();
    });
    const form = h(
      'form',
      { class: 'panel stack', 'data-wish-form': '' },
      h('label', { class: 'lbl', for: 's-mywish' }, 'My wish list ', h('small', null, 'only your Santa sees it')),
      ta,
      h('p', { class: 'err', id: 'mywish-err', role: 'alert' }, this.formErr.get('wish') ?? ''),
      save,
      cancel,
      h('p', { class: 'small', id: 'mywish-why' }, 'You can change it until the gift day. Links are removed.'),
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void this.saveWish();
    });
    return form;
  }

  private async saveWish(): Promise<void> {
    if (this.busy) return;
    this.busy = 'wish';
    this.formErr.delete('wish');
    this.paint();
    const r = await this.api.act('set_wish', { text: (this.wishDraft ?? '').trim() });
    this.busy = '';
    if (r.ok) {
      this.wishOpen = false;
      this.wishDraft = null;
      if (r.view) this.show(r.view);
      else this.paint();
      this.showToast('Wish list saved. Only your Santa sees it.');
      return;
    }
    this.formErr.set('wish', this.formMessage(r));
    this.paint();
  }

  private formMessage(r: Fail): string {
    if (r.message) return r.message;
    if (r.status === 0 || r.status >= 500) return `Not sent: ${poss(this.host())} computer is away. Your words are kept here; try again in a moment.`;
    return "That didn't work. Try again.";
  }

  // G5 Unwrap
  private unwrap(v: SantaView): HTMLElement {
    const me = v.me!;
    const changed = v.match_changed;
    const btn = h('button', { type: 'button', class: 'gbtn', 'data-open': '', 'aria-busy': this.opening ? 'true' : null }, this.opening ? 'Opening…' : changed ? 'Open it again' : 'Open it') as HTMLButtonElement;
    btn.disabled = this.opening;
    btn.addEventListener('click', () => void this.open(true));
    return h(
      'div',
      { class: 'body' },
      changed
        ? h('div', { class: 'notice plain', role: 'status', 'data-changed': '' }, icon('info', 'pi'), h('span', null, h('b', null, 'Your match changed.'), ' ', h('span', { class: 'soft' }, "Someone joined late, so you're buying for someone new.")))
        : null,
      h('div', { class: 'say' }, santa(36), h('div', { class: 'b' }, changed ? 'Your new match is inside. Only you can see it.' : `Names are drawn, ${me.name}! Your match is inside. Only you can see it.`)),
      h('div', { class: 'center' }, giftbox()),
      btn,
      h('p', { class: 'soft center' }, "Check nobody's peeking at your screen first."),
      this.formErr.get('open') ? h('p', { class: 'err', role: 'alert' }, this.formErr.get('open')!) : null,
    );
  }

  /** "Open it": asks the box for the match (once), lifts the lid, then shows the match and remembers this device. */
  async open(tapped: boolean): Promise<void> {
    const v = this.view;
    if (!v?.me || this.opening) return;
    this.formErr.delete('open');
    let match = v.match_changed ? null : v.match;
    if (!match) {
      this.opening = true;
      this.paint();
      const r = await this.api.act('open_match');
      this.opening = false;
      if (!r.ok) {
        this.formErr.set('open', this.formMessage(r));
        this.paint(true);
        return;
      }
      if (r.view) this.take(r.view);
      match = this.view?.match ?? null;
      if (!match) return void this.paint(true);
      this.paint();
    }
    const box = this.root.querySelector('.giftbox');
    box?.classList.add('open');
    const reduced = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    const done = () => {
      this.remember();
      this.poppedAt = tapped ? this.env.now() : -1;
      this.paint(true);
      this.root.querySelector<HTMLElement>('.matchcard')?.focus({ preventScroll: true });
      if (this.view) this.afterShow(this.view);
    };
    if (reduced || !box || !tapped) done();
    else this.env.setTimeout(done, 200);
  }

  // G6 Your match, G7 Santa asks
  private match(v: SantaView): HTMLElement {
    const m = v.match!;
    const f = v.facts;
    const left = f.gift_on ? daysLeft(f.gift_on, this.env.now()) : undefined;
    const facts = [
      f.budget ? this.fact('Budget', f.budget) : null,
      f.gift_on ? this.fact('Gift day', day(f.gift_on)) : null,
      left !== undefined && left >= 0 ? this.fact('Days left', left === 0 ? 'Today' : fmt(left)) : null,
    ].filter(Boolean) as HTMLElement[];
    const wishes = (m.wish ?? '').split(/\n+/).map((s) => s.trim()).filter(Boolean);
    const ready = h('button', { type: 'button', class: `gbtn ghost${v.gift_ready ? ' done' : ''}`, 'aria-pressed': v.gift_ready ? 'true' : 'false', 'data-gift': '' }, icon(v.gift_ready ? 'check' : 'tick', 'pi'), v.gift_ready ? 'Gift ready' : "I've got the gift") as HTMLButtonElement;
    ready.disabled = this.busy === 'gift';
    ready.addEventListener('click', () => void this.giftReady(!v.gift_ready));
    const newIn = v.asks.received.some((a) => !a.a && (a.new || this.fresh.has(a.id)));
    let jump: HTMLElement | null = null;
    if (newIn) {
      jump = h('button', { type: 'button', class: 'notice plain jump', 'data-jump': '' }, santa(28), h('span', null, h('b', null, 'Your Santa asked you something.'), ' ', h('span', { class: 'soft' }, 'Scroll down to answer.')));
      jump.addEventListener('click', () => {
        const t = this.root.querySelector<HTMLElement>('#asks-in');
        t?.scrollIntoView?.({ block: 'start' });
        this.root.querySelector<HTMLElement>('#asks-in [data-k^="reply-"]')?.focus({ preventScroll: true });
      });
    }
    return h(
      'div',
      { class: 'body' },
      h(
        'div',
        { class: `matchcard${this.popping() ? ' pop' : ''}`, tabindex: -1, role: 'group', 'aria-label': `You're buying for ${m.name}` },
        h('span', { class: 'twinkle t1' }, icon('star', 'pi')),
        h('span', { class: 'twinkle t2' }, icon('star', 'pi')),
        h('div', { class: 'eyebrow' }, saIcon('gift'), "You're buying for"),
        h('div', null, creature(m.avatar, 'big')),
        h('div', { class: 'nm' }, m.name),
      ),
      facts.length ? h('div', { class: 'facts' }, facts) : null,
      h(
        'div',
        { class: 'panel' },
        h('div', { class: 'lbl' }, `${poss(m.name)} wish list`),
        wishes.length
          ? h('ul', { class: 'wl' }, wishes.map((w) => h('li', null, saIcon('gift'), h('span', null, w))))
          : h('p', { class: 'soft' }, v.asks.open ? `No wish list yet. You can ask ${m.name} below.` : 'No wish list.'),
      ),
      ready,
      h('p', { class: 'small center' }, `${m.name} won't know. It only tells ${this.host()} how many gifts are ready.`),
      this.formErr.get('gift') ? h('p', { class: 'err', role: 'alert' }, this.formErr.get('gift')!) : null,
      jump,
      this.asks(v),
      f.phase === 'drawn' ? this.wishEditor(v) : null,
    );
  }

  private fact(label: string, value: string): HTMLElement {
    return h('div', { class: 'fact' }, h('small', null, label), h('b', null, value));
  }

  private async giftReady(on: boolean): Promise<void> {
    if (this.busy) return;
    this.busy = 'gift';
    this.formErr.delete('gift');
    this.paint();
    const r = await this.api.act('gift_ready', { ready: on });
    this.busy = '';
    if (r.ok && r.view) return this.show(r.view);
    if (!r.ok) this.formErr.set('gift', this.formMessage(r));
    this.paint();
  }

  private newSticker(): HTMLElement {
    return h('span', { class: 'kw-sticker kw-sticker-ink kw-sticker-sm', 'data-sticker': 'ink' }, h('span', null, 'New'));
  }

  private asks(v: SantaView): HTMLElement {
    const m = v.match!;
    const a = v.asks;
    const now = this.env.now();
    const kids: Kid[] = [];
    if (a.open) {
      const max = a.max_chars;
      const input = h('input', {
        id: 's-ask',
        'data-k': 's-ask',
        class: 'input',
        type: 'text',
        maxlength: max,
        placeholder: 'Type a question…',
        enterkeyhint: 'send',
        autocomplete: 'off',
        'aria-label': `Ask ${m.name} something`,
        'aria-describedby': 'ask-count ask-err',
      }) as HTMLInputElement;
      input.value = this.askDraft;
      const out = a.left_today === 0;
      const send = h('button', { type: 'submit', class: 'send', 'aria-label': 'Send question', 'data-send-ask': '' }, saIcon('send')) as HTMLButtonElement;
      const count = h('p', { class: 'small', id: 'ask-count', 'aria-live': 'polite' });
      const sync = () => {
        send.disabled = out || this.busy === 'ask' || !this.askDraft.trim() || chars(this.askDraft) > max;
        const n = chars(this.askDraft);
        count.textContent = out
          ? 'No questions left today. You can ask again tomorrow.'
          : `${fmt(n)} of ${fmt(max)}${a.left_today !== undefined ? ` · ${fmt(a.left_today)} question${a.left_today === 1 ? '' : 's'} left today` : ''}`;
      };
      input.addEventListener('input', () => {
        this.askDraft = input.value;
        sync();
      });
      sync();
      input.disabled = out;
      const chips = h(
        'div',
        { class: 'tchips' },
        CHIPS.map((c) => {
          const b = h('button', { type: 'button', 'data-chip': '' }, c) as HTMLButtonElement;
          b.disabled = out;
          b.addEventListener('click', () => {
            this.askDraft = c;
            input.value = c;
            sync();
            input.focus();
          });
          return b;
        }),
      );
      const form = h('form', { class: 'askrow', 'data-ask-form': '' }, input, send);
      form.addEventListener('submit', (e) => {
        e.preventDefault();
        void this.sendAsk();
      });
      kids.push(
        h('h3', { class: 'sec-h', id: 'asks-out' }, `Ask ${m.name} something`),
        h('p', { class: 'soft' }, `${m.name} sees "Santa asks", never your name.`),
        chips,
        form,
        count,
        h('p', { class: 'err', id: 'ask-err', role: 'alert' }, this.formErr.get('ask') ?? ''),
      );
    }
    if (a.sent.length) {
      kids.push(
        h(
          'div',
          { class: 'qa', 'data-sent': '' },
          a.sent.map((q) =>
            h(
              'div',
              { class: `q${q.a && (q.new || this.fresh.has(q.id)) ? ' new' : ''}` },
              h('div', { class: 'w' }, saIcon('gift'), `You asked${q.at ? ` · ${when(q.at, now)}` : ''}`, q.a && (q.new || this.fresh.has(q.id)) ? this.newSticker() : null),
              h('div', { class: 'qt' }, q.q),
              q.a ? h('div', { class: 'a' }, h('b', null, `${m.name}:`), ' ', q.a) : h('div', { class: 'a wait' }, `Waiting for ${poss(m.name)} answer`),
            ),
          ),
        ),
      );
    }
    const anyNew = a.received.some((q) => q.new || this.fresh.has(q.id));
    if (a.received.length || a.open) {
      kids.push(h('h3', { class: 'sec-h', id: 'asks-in', tabindex: -1 }, 'Your Santa asked you', anyNew ? this.newSticker() : null));
      kids.push(
        a.received.length
          ? h('div', { class: 'qa', 'data-received': '' }, a.received.map((q) => this.received(q, a.open, now)))
          : h('p', { class: 'soft', 'data-received': '' }, 'Nothing yet. Questions from your Santa show up here, without a name.'),
      );
    }
    if (kids.length && v.expires_at) kids.push(h('p', { class: 'small' }, `Questions and answers go through a word filter, and links are removed. All of it is deleted on ${day(v.expires_at)}.`));
    return h('section', { class: 'stack', 'aria-label': 'Santa asks' }, kids);
  }

  private received(q: Ask, open: boolean, now: number): HTMLElement {
    const isNew = q.new || this.fresh.has(q.id);
    const kids: Kid[] = [h('div', { class: 'w' }, saIcon('gift'), `Santa asks${q.at ? ` · ${when(q.at, now)}` : ''}`), h('div', { class: 'qt' }, q.q)];
    if (q.a) kids.push(h('div', { class: 'a' }, h('b', null, 'You:'), ' ', q.a));
    else if (open) {
      const key = `reply-${q.id}`;
      const input = h('input', { id: key, 'data-k': key, class: 'input', type: 'text', maxlength: 200, autocomplete: 'off', enterkeyhint: 'send', 'aria-label': `Answer: ${q.q}`, 'aria-describedby': `${key}-err` }) as HTMLInputElement;
      input.value = this.replies.get(q.id) ?? '';
      const send = h('button', { type: 'submit', class: 'send', 'aria-label': 'Send answer' }, saIcon('send')) as HTMLButtonElement;
      const sync = () => (send.disabled = this.busy === key || !(this.replies.get(q.id) ?? '').trim());
      input.addEventListener('input', () => {
        this.replies.set(q.id, input.value);
        sync();
      });
      sync();
      const form = h('form', { class: 'reply' }, input, send);
      form.addEventListener('submit', (e) => {
        e.preventDefault();
        void this.sendAnswer(q.id);
      });
      kids.push(form, h('p', { class: 'err', id: `${key}-err`, role: 'alert' }, this.formErr.get(key) ?? ''));
    } else kids.push(h('div', { class: 'a wait' }, 'Not answered'));
    return h('div', { class: `q${isNew && !q.a ? ' new' : ''}`, 'data-ask': q.id }, kids);
  }

  private async sendAsk(): Promise<void> {
    const text = this.askDraft.trim();
    if (!text || this.busy) return;
    this.busy = 'ask';
    this.formErr.delete('ask');
    this.paint();
    const r = await this.api.act('ask', { text });
    this.busy = '';
    if (r.ok) {
      this.askDraft = '';
      if (r.view) this.show(r.view);
      else this.paint(true);
      return;
    }
    this.formErr.set('ask', this.formMessage(r));
    this.paint(true);
  }

  private async sendAnswer(id: string): Promise<void> {
    const text = (this.replies.get(id) ?? '').trim();
    const key = `reply-${id}`;
    if (!text || this.busy) return;
    this.busy = key;
    this.formErr.delete(key);
    this.paint();
    const r = await this.api.act('answer', { id, text });
    this.busy = '';
    if (r.ok) {
      this.replies.delete(id);
      if (r.view) this.show(r.view);
      else this.paint(true);
      return;
    }
    this.formErr.set(key, this.formMessage(r));
    this.paint(true);
  }

  // G8 Reveal
  private reveal(v: SantaView): HTMLElement {
    const me = v.me!;
    const s = v.my_santa;
    const loop = v.reveal_loop!;
    const person = (p: { name: string; avatar: string }) => h('span', { class: 'p' }, creature(p.avatar, 'xs'), h('span', { class: 'pn' }, p.name));
    return h(
      'div',
      { class: 'body' },
      h('div', { class: 'say' }, santa(36, 'finished'), h('div', { class: 'b' }, "Reveal time! Here's who was whose Santa. Thanks for keeping the secret.")),
      s
        ? h(
            'div',
            { class: 'matchcard', role: 'group', 'aria-label': `Your Santa was ${s.name}` },
            h('span', { class: 'twinkle t1' }, icon('star', 'pi')),
            h('span', { class: 'twinkle t2' }, icon('star', 'pi')),
            h('div', { class: 'eyebrow' }, 'Your Santa was'),
            h('div', null, creature(s.avatar, 'big')),
            h('div', { class: 'nm' }, s.name),
          )
        : null,
      h(
        'div',
        null,
        h('div', { class: 'lbl', id: 'loop-lbl' }, 'Who gave to whom'),
        h(
          'ol',
          { class: 'chain', 'aria-labelledby': 'loop-lbl' },
          loop.map((l) =>
            h(
              'li',
              { class: `lk${l.giver.id === me.id || l.receiver.id === me.id ? ' me' : ''}`, 'aria-label': `${l.giver.name} gave to ${l.receiver.name}` },
              person(l.giver),
              h('span', { class: 'ar', 'aria-hidden': 'true' }, saIcon('arrow')),
              person(l.receiver),
            ),
          ),
        ),
      ),
      this.keep(v, false),
    );
  }

  // G9 Away
  private awayLine(): string {
    const a = this.away!;
    return `Trying again in ${a.left} s · tried ${a.tries} time${a.tries === 1 ? '' : 's'}`;
  }

  private awayScreen(): HTMLElement {
    const host = this.host();
    const btn = h('button', { type: 'button', class: 'gbtn ghost', 'data-try': '' }, 'Try now');
    btn.addEventListener('click', this.tryNow);
    const what = this.view?.me ? 'Your match and answers are safe. ' : '';
    return h(
      'div',
      { class: 'body mid', role: 'status' },
      h('div', { class: 'qmwrap' }, santa(96, 'thinking')),
      h('h2', null, `${cap(poss(host))} computer is asleep`),
      h('p', { class: 'lead' }, `This page lives on ${poss(host)} computer, and it's off or asleep right now. ${what}The page tries again by itself.`),
      h('p', { class: 'small', 'data-away': '' }, this.awayLine()),
      btn,
    );
  }

  // G11 Ended
  private ended(v: SantaView): HTMLElement {
    const n = v.players ?? (v.people.length || undefined);
    const lead = `${n ? `${fmt(n)} ${n === 1 ? 'person' : 'people'}. ` : ''}Thanks for keeping the secret!`;
    const small = v.expires_at ? `Names, wish lists and questions are deleted on ${day(v.expires_at)}.` : undefined;
    // A late visit after the reveal, while the pocket is still open: name and PIN still show the reveal.
    let back: HTMLElement | null = null;
    if (!v.ended && !v.me) {
      back = h('button', { type: 'button', class: 'gbtn ghost', 'data-to-rejoin': '' }, "I've joined before: open my page");
      back.addEventListener('click', () => this.goRejoin());
    }
    return this.message(santa(96, 'finished'), 'This Secret Santa has ended', lead, small, back);
  }
}

