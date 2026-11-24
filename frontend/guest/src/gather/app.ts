// The gather guest page (pa10b: Sign-up, RSVP and Vote, the Club Sub Chaser), G1-G11 in pa10b/guest-390.html, on
// Lin's gather API (pa10b/server-impl.md "Pocket API v1: gather"). Loaded as its own chunk when the pocket's kit is
// `gather`; the quiz and Santa screens are untouched. Plain DOM, no framework; text goes in as text only. Every number
// shown comes from the server; this page only formats it ("1st", "Dec 12").
import { Api, Env, Fail, Live } from '../api';
import { AVATARS, creature, fmt, h, icon, Kid } from '../dom';
import { clock, day } from '../app';
import { cap, PinInput, pinCopy, Problem } from '../santa/pin';
import { gaIcon, tally } from './look';
import { Answer, FOOD_LABELS, gatherFrom, GatherView, move, nth, Screen, screenOf, Slot, Votes, whoSees } from './view';

const TOAST_MS = 4000;
const INFO_MS = 10_000;
const ANSWERS: [Answer, string, string][] = [
  ['yes', 'Yes', "I'm coming"],
  ['maybe', 'Maybe', 'not sure yet'],
  ['no', 'No', "can't make it"],
];
const WORD: Record<Answer, string> = { yes: 'Yes', maybe: 'Maybe', no: 'No' };

export type GatherEnv = Env & {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
};

/** "Dec 4" (a calendar day in UTC, like ../app.ts `day`). */
export function shortDay(iso: string): string {
  const d = new Date(`${iso.slice(0, 10)}T12:00:00Z`);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
}

/** "Thu 8 PM": a moment in this phone's time zone, for the bar. */
export function closesShort(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const wd = d.toLocaleDateString('en-US', { weekday: 'short' });
  return `${wd} ${clock(iso).replace(':00 ', ' ')}`;
}

/** "Thu, Dec 10 at 8:00 PM" in this phone's time zone. */
export function at(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return `${d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' })} at ${clock(iso)}`;
}

const poss = (name: string): string => `${name}'s`;
/** "A", "A and B", "A, B and C". */
const and = (xs: string[]): string => (xs.length < 2 ? (xs[0] ?? '') : `${xs.slice(0, -1).join(', ')} and ${xs[xs.length - 1]}`);
/** A creature for a new guest: the page has no picker (G1), so it follows the name. Not a number anyone sees. */
export const avatarFor = (name: string): string => AVATARS[[...name.trim().toLowerCase()].reduce((a, c) => a + c.codePointAt(0)!, 0) % AVATARS.length];
/** A bar's width in 5% steps (classes, since the page's CSP allows no style attributes). */
const wclass = (n: number, of: number): string => `w${of > 0 ? Math.max(0, Math.min(20, Math.round((n / of) * 20))) : 0}`;

type Draft = {
  name: string;
  pin: string;
  answer?: Answer;
  plus: number;
  food: string[];
  otherOn: boolean;
  other: string;
  slot?: number;
  note: string;
  pick?: number;
  order: number[];
};
type BackDraft = { name: string; pin: string; code: string; newPin: string; reset: boolean };

export class GatherApp {
  view: GatherView | null = null;
  readonly api: Api<GatherView>;
  private live: Live<GatherView>;
  private root: HTMLElement;
  private sig = '';
  private away: { secs: number; tries: number; left: number } | null = null;
  private awayTick = 0;
  private infoTimer = 0;
  private toastTimer = 0;
  private toast: HTMLElement | null = null;
  private gone: 'removed' | 'missing' | null = null;
  private hadSeat = false;
  private rejoin = false;
  private edit = false;
  private letGo = false;
  /** Drafts filled from the server once per seat, so "Change my answer" starts from what was sent. */
  private filled = '';
  private jd: Draft = { name: '', pin: '', plus: 0, food: [], otherOn: false, other: '', note: '', order: [] };
  private bd: BackDraft = { name: '', pin: '', code: '', newPin: '', reset: false };
  private problem: Problem | null = null;
  private busy = '';
  /** The row a rank move just swapped, for the 160 ms slide. */
  private moved: { opt: number; dir: 'up' | 'down' } | null = null;
  /** "Moved up" stays ink New for this page load, after `seen` clears it on the box. */
  private freshMove = false;
  private seenSent = false;
  private noticesSeen = false;

  constructor(root: HTMLElement, private env: GatherEnv, readonly code: string) {
    this.root = root;
    this.api = new Api<GatherView>(env, code, gatherFrom);
    this.live = new Live<GatherView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => this.goAway(secs, tries),
      back: () => this.comeBack(),
      gone: (status, error) => void this.onGone(status, error),
    });
  }

  /** Same boot as Santa: the page's public facts first, then `state` finds out whether this phone has a seat. */
  async start(boot?: unknown): Promise<void> {
    const b = boot ? this.api.take(boot) : undefined;
    if (b) this.show(b);
    else this.paint();
    if (b?.seat) return this.live.start();
    const r = await this.api.state();
    if (r.ok && r.view) {
      this.show(r.view);
      if (r.view.seat) this.live.start();
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
    if (this.gone || !this.view?.seat) return;
    this.live.now();
  }

  tryNow = (): void => this.live.now();

  // ---------- incoming ----------

  show(v: GatherView): void {
    if (this.gone) return;
    this.view = v;
    if (v.seat) {
      this.hadSeat = true;
      this.rejoin = false;
      this.fill(v);
      if (v.mine?.moved_up?.new) this.freshMove = true;
    }
    this.paint();
    this.afterShow(v);
  }

  /** Copies the guest's own record into the form once per seat (what they typed is never overwritten after that). */
  private fill(v: GatherView): void {
    const m = v.mine;
    const key = v.seat!.id;
    if (!m || this.filled === key) return;
    this.filled = key;
    const d = this.jd;
    if (m.answer) d.answer = m.answer;
    d.plus = m.plus_ones ?? 0;
    d.food = m.food.slice();
    d.otherOn = !!m.food_other;
    d.other = m.food_other ?? '';
    const s = m.slot ?? m.waiting?.slot;
    if (s !== undefined) d.slot = s;
    d.note = m.note ?? '';
    if (m.ballot) {
      d.order = m.ballot.slice();
      d.pick = m.ballot[0];
    }
  }

  private afterShow(v: GatherView): void {
    if (!v.seat || this.away) return;
    if (this.screenNow() === 'moved' && v.mine?.moved_up?.new && !this.seenSent) {
      this.seenSent = true;
      void this.api.act('seen');
    }
    if (v.seat.notices.length && !this.noticesSeen) {
      this.noticesSeen = true;
      const n = v.seat.notices.find((x) => x.kind === 'pin_reset') ?? v.seat.notices[0];
      const host = this.host();
      this.showToast(
        n.kind === 'pin_reset'
          ? `${cap(host)} reset your PIN${n.at ? ` on ${day(n.at)}` : ''}.`
          : `Someone tried to open your page with a wrong PIN${n.at ? ` on ${day(n.at)}` : ''}. Your answer is still safe.`,
      );
      void this.api.call('notices/seen', {});
    }
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
      this.env.clearTimeout(this.infoTimer);
      this.away = null;
      if (this.view) this.show({ ...this.view, ended: true, seat: undefined, mine: undefined });
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

  /** Before a seat there is no socket: the facts (counts, slots, phase) are read again every 10 s. */
  private watchInfo(): void {
    this.env.clearTimeout(this.infoTimer);
    this.infoTimer = this.env.setTimeout(async () => {
      if (this.view?.seat || this.gone) return;
      const i = await this.api.info();
      if (this.view?.seat || this.gone) return;
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

  // ---------- drawing ----------

  private host(): string {
    return this.view?.facts.host_name ?? 'the host';
  }

  screenNow(): Screen | 'away' | 'removed' | 'missing' | 'loading' {
    if (this.gone) return this.gone;
    if (this.away) return 'away';
    const v = this.view;
    if (!v) return 'loading';
    return screenOf(v, { rejoin: this.rejoin, edit: this.edit });
  }

  /** Draws the screen when something it shows changed. Typing never redraws; focus and caret survive a redraw. */
  paint(force = false): void {
    const scr = this.screenNow();
    const d = this.jd;
    const sig = JSON.stringify([scr, this.view, this.problem, this.busy, this.bd.reset, this.letGo, this.freshMove, this.away?.tries, d.answer, d.plus, d.food, d.otherOn, d.slot, d.pick, d.order]);
    if (!force && sig === this.sig) return;
    this.sig = sig;
    const active = document.activeElement as HTMLInputElement | null;
    const k = active?.getAttribute?.('data-k');
    const sel = k && typeof active?.selectionStart === 'number' ? [active.selectionStart, active.selectionEnd ?? active.selectionStart] : null;
    const scrollY = typeof window !== 'undefined' ? window.scrollY : 0;
    const face = this.view?.facts.face ?? 'rsvp';
    const page = h('main', { class: `g ga f-${face} s-${scr}`, 'data-screen': scr, 'data-face': face }, this.bar(scr), this.body(scr), this.madeWith());
    this.root.replaceChildren(page);
    if (k) {
      const el = this.root.querySelector<HTMLInputElement>(`[data-k="${k}"]`);
      if (el) {
        el.focus({ preventScroll: true });
        if (sel) el.setSelectionRange?.(sel[0], sel[1]);
      }
    } else this.root.querySelector<HTMLElement>('[data-autofocus]')?.focus({ preventScroll: true });
    if (typeof window !== 'undefined' && scrollY && window.scrollTo) window.scrollTo(0, scrollY);
    this.moved = null;
  }

  private body(scr: ReturnType<GatherApp['screenNow']>): HTMLElement {
    const v = this.view;
    switch (scr) {
      case 'loading':
        return this.message(tally(96, 'thinking'), 'Opening…', '');
      case 'removed':
        return this.message(tally(96), "You've been taken off this list", `${cap(this.host())} took you out of it. If that's a mistake, have a word with ${this.host()}.`);
      case 'missing':
        return this.message(tally(96), 'Nothing here', "This link doesn't open a pocket app any more. Check the link with the host.");
      case 'away':
        return this.awayScreen();
      case 'join':
        return this.form(v!);
      case 'rejoin':
        return this.back(v!);
      case 'in':
        return v!.facts.face === 'signup' ? this.slotIn(v!, false) : this.rsvpIn(v!);
      case 'waiting':
        return this.waiting(v!);
      case 'moved':
        return this.slotIn(v!, true);
      case 'voted':
        return this.voted(v!);
      case 'result':
        return this.result(v!);
      case 'closed':
        return this.closed(v!);
    }
  }

  private bar(scr: string): HTMLElement {
    const v = this.view;
    let right = '';
    if (v && ['join', 'rejoin'].includes(scr) && v.facts.phase === 'open' && v.facts.closes_at) right = `closes ${closesShort(v.facts.closes_at)}`;
    else if (v?.seat && ['in', 'waiting', 'moved', 'voted'].includes(scr)) right = v.seat.name;
    else if (v && scr === 'result') right = 'Voting closed';
    return h('header', { class: 'bar' }, tally(28), h('b', null, v?.title ?? ''), h('span', { class: 'sp' }), h('span', { class: 'muted tab', 'data-bar': '' }, right));
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
  private keep(v: GatherView): HTMLElement | null {
    const when = v.expires_at;
    if (!when) return null;
    const after = v.facts.event_at ? 'the event' : 'it closes';
    const why = v.keep_days !== undefined ? ` (${v.keep_days} day${v.keep_days === 1 ? '' : 's'} after ${after})` : '';
    return h('div', { class: 'keep', 'data-keep': '' }, icon('lock', 'pi'), h('span', null, `Kept on ${poss(this.host())} computer until ${day(when)}${why}, then deleted.`));
  }

  /** The short deletion line for closed screens. */
  private gone_(v: GatherView): string | undefined {
    if (!v.expires_at) return undefined;
    const f = v.facts;
    const what =
      f.face === 'vote' ? 'Ballots are' : f.face === 'signup' ? 'Names and slots are' : f.fields.includes('food_needs') ? 'Answers and food needs are' : 'Answers are';
    return `${what} deleted on ${day(v.expires_at)}.`;
  }

  private hero(v: GatherView, sub: string): HTMLElement {
    return h('div', { class: 'hero' }, tally(64), h('div', null, h('h1', { id: 'title' }, v.title), sub ? h('div', { class: 'soft sub' }, sub) : null));
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

  private ghost(text: string, on: () => void, data: string): HTMLElement {
    const b = h('button', { type: 'button', class: 'gbtn ghost', [`data-${data}`]: '' }, text);
    b.addEventListener('click', on);
    return b;
  }

  private say(mood: 'finished' | undefined, text: string): HTMLElement {
    return h('div', { class: 'say' }, tally(36, mood), h('div', { class: 'b' }, text));
  }

  private nameInput(id: string, value: string, on: (s: string) => void): HTMLInputElement {
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
      'aria-invalid': this.problem?.field === 'name' ? 'true' : null,
    }) as HTMLInputElement;
    el.value = value;
    el.addEventListener('input', () => on(el.value));
    return el;
  }

  private whoSeesLine(v: GatherView): HTMLElement {
    return h('div', { class: 'whosees', 'data-whosees': '' }, gaIcon('eye'), h('span', null, whoSees(v.facts).map(([who, what], i) => [i ? ' ' : '', h('b', null, who), what])));
  }

  /** The fact tiles of an RSVP (G1): the event's day and time, and the price, as the host typed it. */
  private factTiles(v: GatherView): HTMLElement | null {
    const f = v.facts;
    const tiles: [string, string][] = [];
    const ev = f.event_at ? new Date(f.event_at) : null;
    if (ev && !Number.isNaN(ev.getTime())) {
      tiles.push([ev.toLocaleDateString('en-US', { weekday: 'long' }), ev.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })]);
      tiles.push(['Time', clock(f.event_at!)]);
    } else if (f.when) tiles.push(['When', f.when]);
    if (f.price) tiles.push(['Each', f.price]);
    if (!tiles.length) return null;
    return h('div', { class: 'facts' }, tiles.map(([k, val]) => h('div', { class: 'fact' }, h('small', null, k), h('b', null, val))));
  }

  // ---------- G1 / G3 / G6 / G7: the answer form ----------

  private form(v: GatherView): HTMLElement {
    const d = this.jd;
    const f = v.facts;
    const seated = !!v.seat;
    const host = f.host_name;
    let sub: string;
    if (f.face === 'vote') sub = [host ? `hosted by ${host}` : '', f.ballot?.kind === 'ranked' ? 'put them in order' : 'one pick'].filter(Boolean).join(' · ');
    else if (f.face === 'signup') sub = [f.when, f.place].filter(Boolean).join(' · ') || (host ? `hosted by ${host}` : '');
    else sub = [f.place, host ? `hosted by ${host}` : ''].filter(Boolean).join(' · ');
    const label = f.face === 'vote' ? 'Send my vote' : f.face === 'signup' ? this.slotButton(f.slots) : 'Send my answer';
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-send': '', 'aria-busy': this.busy === 'send' ? 'true' : null }, this.busy === 'send' ? 'Sending…' : label) as HTMLButtonElement;
    const ready = () => (btn.disabled = this.busy === 'send' || (!seated && (!d.name.trim() || d.pin.length !== 4)) || !this.chosen(v));
    let nameBlock: Kid = null;
    let pinBlock: Kid = null;
    if (!seated) {
      const n = f.face === 'vote' ? f.voted : f.joined;
      const counted = n !== undefined && n > 0 ? `${fmt(n)} ${f.face === 'vote' ? 'voted' : f.face === 'signup' ? 'signed up' : 'answered'}` : '';
      const name = this.nameInput('g-name', d.name, (s) => {
        d.name = s;
        ready();
      });
      nameBlock = h('div', null, h('label', { class: 'lbl', for: 'g-name' }, 'Your name ', counted ? h('small', null, counted) : null), name);
      const pin = new PinInput('g-pin', 'Pick a 4-digit PIN', (p) => {
        d.pin = p;
        ready();
      }, d.pin, this.problem?.field === 'pin', 'pin-why');
      pinBlock = h(
        'div',
        null,
        h('label', { class: 'lbl', for: 'g-pin' }, 'Pick a 4-digit PIN ', h('small', null, f.face === 'vote' ? 'to change your vote' : 'to change it later')),
        pin.el,
        h('p', { class: 'soft fine', id: 'pin-why' }, "You'll need your name and PIN to come back."),
      );
    }
    const choice = f.face === 'rsvp' ? this.rsvpFields(v, ready) : f.face === 'signup' ? this.slotFields(v, ready) : this.ballotFields(v, ready);
    ready();
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      this.keep(v),
      this.hero(v, sub),
      f.face === 'rsvp' ? this.factTiles(v) : null,
      nameBlock,
      choice,
      this.whoSeesLine(v),
      pinBlock,
      this.problemBox(),
      btn,
      seated
        ? h('p', { class: 'center' }, this.linkButton('Keep my answer as it was', () => this.cancelEdit(), 'cancel'))
        : h('p', { class: 'center' }, this.linkButton("I've joined before", () => this.goRejoin(), 'to-rejoin')),
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void this.submit();
    });
    return form;
  }

  private slotButton(slots: Slot[]): string {
    const s = slots.find((x) => x.slot === this.jd.slot);
    const mine = this.view?.mine;
    if (s && s.full && mine?.slot !== s.slot && mine?.waiting?.slot !== s.slot) return 'Join the waiting list';
    return 'Sign me up';
  }

  /** Whether the guest picked what the face needs. */
  private chosen(v: GatherView): boolean {
    const d = this.jd;
    if (v.facts.face === 'rsvp') return !!d.answer;
    if (v.facts.face === 'signup') return d.slot !== undefined && v.facts.slots.some((s) => s.slot === d.slot);
    return v.facts.ballot?.kind === 'ranked' ? d.order.length > 0 : d.pick !== undefined;
  }

  private radio(group: string, value: string, checked: boolean, label: Kid, cls: string, on: () => void, extra: Record<string, string | null> = {}): HTMLElement {
    const input = h('input', { type: 'radio', name: group, value, class: 'vh', 'data-k': `${group}-${value}`, ...extra }) as HTMLInputElement;
    input.checked = checked;
    input.addEventListener('change', on);
    return h('label', { class: `${cls}${checked ? ' on' : ''}` }, input, label);
  }

  // G1
  private rsvpFields(v: GatherView, ready: () => void): Kid {
    const d = this.jd;
    const f = v.facts;
    const opts = h(
      'div',
      { class: 'opts', role: 'radiogroup', 'aria-labelledby': 'lbl-coming' },
      ANSWERS.map(([a, word, sub]) =>
        this.radio('answer', a, d.answer === a, [h('span', null, word), h('small', null, sub)], 'opt', () => {
          d.answer = a;
          this.problem = null;
          this.paint();
          ready();
        }),
      ),
    );
    const out: Kid[] = [h('div', null, h('div', { class: 'lbl', id: 'lbl-coming' }, 'Coming?'), opts)];
    // Plus-ones and food matter only for people coming (the server keeps them only for a Yes).
    if (d.answer === 'yes' && f.max_plus_ones > 0) {
      const max = f.max_plus_ones;
      const step = (by: number) => () => {
        const n = Math.max(0, Math.min(max, d.plus + by));
        if (n === d.plus) return;
        d.plus = n;
        const val = this.root.querySelector('[data-plus]');
        if (val) val.replaceChildren(String(n), h('small', null, n === 1 ? 'plus-one' : 'plus-ones'));
        this.root.querySelector<HTMLButtonElement>('[data-k="plus-less"]')?.setAttribute('aria-disabled', n === 0 ? 'true' : 'false');
        this.root.querySelector<HTMLButtonElement>('[data-k="plus-more"]')?.setAttribute('aria-disabled', n === max ? 'true' : 'false');
      };
      const less = h('button', { type: 'button', class: 'sb', 'data-k': 'plus-less', 'aria-label': 'One fewer', 'aria-disabled': d.plus === 0 ? 'true' : 'false' }, gaIcon('minus'));
      const more = h('button', { type: 'button', class: 'sb', 'data-k': 'plus-more', 'aria-label': 'One more', 'aria-disabled': d.plus === max ? 'true' : 'false' }, icon('plus', 'pi'));
      less.addEventListener('click', step(-1));
      more.addEventListener('click', step(1));
      out.push(
        h(
          'div',
          null,
          h('div', { class: 'lbl', id: 'lbl-plus' }, 'Bringing someone? ', h('small', null, 'plus-ones')),
          h(
            'div',
            { class: 'stepper', role: 'group', 'aria-labelledby': 'lbl-plus' },
            less,
            h('div', { class: 'v', 'data-plus': '', 'aria-live': 'polite', 'aria-atomic': 'true' }, String(d.plus), h('small', null, d.plus === 1 ? 'plus-one' : 'plus-ones')),
            more,
          ),
        ),
      );
    }
    if (d.answer === 'yes' && f.food_choices.length) {
      const chip = (key: string, text: string, on: boolean, change: (c: boolean) => void, mark: Kid = null) => {
        const input = h('input', { type: 'checkbox', class: 'vh', 'data-k': `food-${key}` }) as HTMLInputElement;
        input.checked = on;
        input.addEventListener('change', () => change(input.checked));
        return h('label', { class: on ? 'on' : '' }, input, on ? icon('tick', 'pi') : mark, text);
      };
      const chips = f.food_choices.map((k) =>
        chip(k, FOOD_LABELS[k], d.food.includes(k), (c) => {
          d.food = c ? [...d.food, k] : d.food.filter((x) => x !== k);
          this.paint();
        }),
      );
      chips.push(
        chip('other', 'Something else', d.otherOn, (c) => {
          d.otherOn = c;
          this.paint();
          if (c) this.root.querySelector<HTMLElement>('#g-other')?.focus();
        }, icon('plus', 'pi')),
      );
      let other: Kid = null;
      if (d.otherOn) {
        const el = h('input', { id: 'g-other', 'data-k': 'g-other', class: 'input', type: 'text', maxlength: f.note_max, 'aria-label': 'Something else: what do you need?', autocomplete: 'off' }) as HTMLInputElement;
        el.value = d.other;
        el.addEventListener('input', () => (d.other = el.value));
        other = h('div', { class: 'other' }, el, h('p', { class: 'small fine' }, `Up to ${f.note_max} letters.`));
      }
      out.push(
        h(
          'div',
          null,
          h('div', { class: 'lbl', id: 'lbl-food' }, 'Food needs ', h('small', null, f.max_plus_ones > 0 ? 'for you and your plus-ones' : 'optional')),
          h('div', { class: 'tchips', role: 'group', 'aria-labelledby': 'lbl-food' }, chips),
          other,
        ),
      );
    }
    return out;
  }

  // G3
  private slotFields(v: GatherView, ready: () => void): Kid {
    const d = this.jd;
    const f = v.facts;
    const mine = v.mine;
    const cards = f.slots.map((s) => {
      const mineHere = mine?.slot === s.slot;
      const names = (s.people ?? []).map((p) => p.name);
      let line: string;
      if (mineHere) line = "You're in this one";
      else if (mine?.waiting?.slot === s.slot) line = `You're ${nth(mine.waiting.position)} in line`;
      else if (s.full && !f.waiting_list) line = ['Full', ...names].join(' · ');
      else if (s.full) line = [s.waiting === 0 ? "Full: you'd be 1st in line" : `Full · ${fmt(s.waiting)} waiting`, ...(names.length ? [names.join(', ')] : [])].join(' · ');
      else line = names.length ? names.join(', ') : s.filled === 0 ? 'Nobody yet' : '';
      const closed = s.full && !f.waiting_list && !mineHere;
      const pips = s.cap <= 12 ? h('span', { class: 'pips', 'aria-hidden': 'true' }, Array.from({ length: s.cap }, (_, i) => h('i', { class: i < s.filled ? 'on' : '' }))) : null;
      const capText = s.full ? 'Full' : `${fmt(s.filled)} of ${fmt(s.cap)}`;
      const el = this.radio(
        'slot',
        String(s.slot),
        d.slot === s.slot,
        [h('b', null, s.name), line ? h('small', null, line) : null, h('span', { class: 'cap', 'aria-label': `${fmt(s.filled)} of ${fmt(s.cap)}` }, capText, pips)],
        `gslot${s.full ? ' full' : ''}`,
        () => {
          d.slot = s.slot;
          this.problem = null;
          this.paint();
          ready();
        },
        closed ? { disabled: 'true' } : {},
      );
      return el;
    });
    const out: Kid[] = [
      h(
        'div',
        null,
        h('div', { class: 'lbl', id: 'lbl-slot' }, 'What will you bring? ', h('small', null, 'pick one')),
        h('div', { class: 'gslots', role: 'radiogroup', 'aria-labelledby': 'lbl-slot' }, cards),
      ),
    ];
    if (f.fields.includes('slot_note')) {
      const el = h('input', { id: 'g-note', 'data-k': 'g-note', class: 'input', type: 'text', maxlength: f.note_max, autocomplete: 'off', 'aria-describedby': 'note-why' }) as HTMLInputElement;
      el.value = d.note;
      el.addEventListener('input', () => (d.note = el.value));
      out.push(h('div', null, h('label', { class: 'lbl', for: 'g-note' }, 'What exactly? ', h('small', null, 'optional')), el, h('p', { class: 'small fine', id: 'note-why' }, `Up to ${f.note_max} letters.`)));
    }
    return out;
  }

  // G6 / G7
  private ballotFields(v: GatherView, ready: () => void): Kid {
    const d = this.jd;
    const b = v.facts.ballot;
    if (!b) return null;
    const opts = b.options.filter((o) => !b.in_play.length || b.in_play.includes(o.option));
    if (b.kind === 'one_pick')
      return h(
        'div',
        null,
        h('div', { class: 'lbl', id: 'lbl-pick' }, 'Your pick'),
        h(
          'div',
          { class: 'answers', role: 'radiogroup', 'aria-labelledby': 'lbl-pick' },
          opts.map((o) =>
            this.radio('pick', String(o.option), d.pick === o.option, [h('span', { class: 'radio', 'aria-hidden': 'true' }), h('span', { class: 'at' }, o.name)], `ans${d.pick === o.option ? ' pick' : ''}`, () => {
              d.pick = o.option;
              this.problem = null;
              this.paint();
              ready();
            }),
          ),
        ),
      );
    // Ranked: the order the guest built, then the options still out, dashed.
    const order = d.order.filter((o) => opts.some((x) => x.option === o));
    const name = (o: number) => opts.find((x) => x.option === o)!.name;
    const rows: Kid[] = order.map((o, i) => {
      const first = i === 0;
      const last = i === order.length - 1;
      const up = h('button', { type: 'button', class: `mv${first ? ' off' : ''}`, 'data-k': `up-${o}`, 'aria-label': `Move ${name(o)} up`, 'aria-disabled': first ? 'true' : 'false' }, gaIcon('up'));
      const down = h('button', { type: 'button', class: 'mv', 'data-k': `down-${o}`, 'aria-label': last ? `Leave ${name(o)} out` : `Move ${name(o)} down` }, gaIcon('down'));
      up.addEventListener('click', () => this.rank(o, 'up'));
      down.addEventListener('click', () => this.rank(o, 'down'));
      const slide = this.moved?.opt === o ? ` slide-${this.moved.dir}` : '';
      return h('li', { class: `rk${slide}`, 'data-opt': String(o) }, h('span', { class: 'no', 'aria-hidden': 'true' }, String(i + 1)), h('span', { class: 'nm' }, h('span', { class: 'vh' }, `${nth(i + 1)}: `), name(o)), up, down);
    });
    const left = opts.filter((o) => !order.includes(o.option));
    const add: Kid[] = left.map((o) => {
      const b2 = h('button', { type: 'button', class: 'rk un', 'data-k': `add-${o.option}` }, h('span', { class: 'no', 'aria-hidden': 'true' }, '+'), h('span', { class: 'nm' }, o.name, h('small', null, `Tap to rank it ${nth(order.length + 1)}${order.length ? ', or leave it out' : ''}`)));
      b2.addEventListener('click', () => {
        d.order = [...order, o.option];
        this.problem = null;
        this.paint();
        ready();
        // Focus follows the option to its new row.
        this.root.querySelector<HTMLElement>(`[data-k="down-${o.option}"]`)?.focus();
      });
      return b2;
    });
    return h(
      'div',
      null,
      h('div', { class: 'lbl', id: 'lbl-rank' }, 'Your order ', h('small', null, 'rank as many as you like')),
      order.length ? h('ol', { class: 'ranks', 'aria-labelledby': 'lbl-rank' }, rows) : null,
      add.length ? h('div', { class: 'ranks more' }, add) : null,
      h('p', { class: 'soft fine' }, 'Your 1st choice counts first. If it drops out, your vote moves to your 2nd, then your 3rd.'),
      h('p', { class: 'vh', 'aria-live': 'polite', 'data-rank-say': '' }),
    );
  }

  /** Moves a ranked option; focus stays on the button that moved it, on the moved row. */
  rank(opt: number, dir: 'up' | 'down'): void {
    const d = this.jd;
    const before = d.order;
    const after = move(before, opt, dir);
    if (after === before) return;
    const out = !after.includes(opt);
    d.order = after;
    this.moved = out ? null : { opt, dir };
    const want = out ? `add-${opt}` : `${dir}-${opt}`;
    this.paint();
    const el = this.root.querySelector<HTMLElement>(`[data-k="${want}"]`);
    // At the top the up button is aria-disabled but still focusable, so focus never jumps away from the row.
    el?.focus();
    const say = this.root.querySelector('[data-rank-say]');
    if (say) say.textContent = out ? `${this.optName(opt)} left out.` : `${this.optName(opt)} is now ${nth(after.indexOf(opt) + 1)}.`;
  }

  private optName(o: number): string {
    return this.view?.facts.ballot?.options.find((x) => x.option === o)?.name ?? '';
  }

  private cancelEdit(): void {
    this.edit = false;
    this.problem = null;
    this.filled = '';
    if (this.view) this.fill(this.view);
    this.paint();
  }

  private goRejoin(): void {
    this.rejoin = true;
    this.problem = null;
    if (!this.bd.name) this.bd.name = this.jd.name;
    this.paint();
    this.root.querySelector<HTMLElement>('#g-bname')?.focus();
  }

  private goEdit(): void {
    this.edit = true;
    this.letGo = false;
    this.problem = null;
    this.paint();
    window.scrollTo?.(0, 0);
  }

  /** What the answer action sends, from the draft; only fields the host chose. */
  private action(v: GatherView): [string, Record<string, unknown>] {
    const d = this.jd;
    const f = v.facts;
    if (f.face === 'rsvp') {
      const body: Record<string, unknown> = { answer: d.answer };
      if (d.answer === 'yes' && f.max_plus_ones > 0) body.plus_ones = d.plus;
      if (d.answer === 'yes' && f.food_choices.length) {
        body.food = d.food.filter((x) => f.food_choices.includes(x));
        if (d.otherOn && d.other.trim()) body.food_other = d.other.trim();
      }
      return ['answer', body];
    }
    if (f.face === 'signup') {
      const body: Record<string, unknown> = { slot: d.slot };
      if (f.fields.includes('slot_note') && d.note.trim()) body.note = d.note.trim();
      return ['take', body];
    }
    return ['vote', { ranking: f.ballot?.kind === 'ranked' ? d.order : [d.pick] }];
  }

  async submit(): Promise<void> {
    const v = this.view;
    const d = this.jd;
    if (!v || this.busy) return;
    if (!v.seat) {
      if (!d.name.trim()) return this.fail({ title: 'Type your name first.', field: 'name' });
      if (d.pin.length !== 4) return this.fail({ title: 'Pick a 4-digit PIN.', field: 'pin' });
    }
    if (!this.chosen(v)) return this.fail({ title: v.facts.face === 'rsvp' ? 'Pick Yes, Maybe or No.' : v.facts.face === 'signup' ? 'Pick a slot.' : 'Pick at least one option.' });
    this.busy = 'send';
    this.problem = null;
    this.paint();
    if (!v.seat) {
      const name = d.name.trim();
      const r = await this.api.join({ name, avatar: avatarFor(name), pin: d.pin });
      if (!r.ok) {
        this.busy = '';
        if (r.status === 410 || r.error === 'ended') return void this.onGone(410, 'ended');
        if (r.status === 404) return void this.onGone(404, 'not_found');
        if (r.error === 'pin_required') d.pin = '';
        return this.fail(r.error === 'full' ? { title: r.message ?? 'This list is full.' } : pinCopy(r, this.host(), true));
      }
      this.env.clearTimeout(this.infoTimer);
      d.pin = '';
      if (r.view) this.view = r.view;
      this.hadSeat = true;
      this.live.start();
    }
    // The answer itself; a refusal keeps the form and everything typed, now with a seat.
    const [name, body] = this.action(this.view!);
    const a = await this.api.act(name, body);
    this.busy = '';
    if (a.ok) {
      this.edit = false;
      this.filled = '';
      if (a.view) this.show(a.view);
      else this.paint();
      window.scrollTo?.(0, 0);
      return;
    }
    if (a.ok === false) {
      if (a.status === 410 || a.error === 'ended') return void this.onGone(410, 'ended');
      this.edit = true;
      this.failAct(a);
    }
  }

  private failAct(a: Fail): void {
    if (a.status === 0 || a.status >= 500) return this.fail({ title: `Couldn't reach ${poss(this.host())} computer.`, sub: 'Check your signal and try again. What you picked is kept.' });
    // "Answers are closed", a full slot, a bad note: the server's own words.
    this.fail({ title: a.message ?? "That didn't work. Try again." });
    if (a.error === 'not_now') void this.api.state().then((r) => r.ok && r.view && this.show(r.view));
  }

  private fail(p: Problem): void {
    this.problem = p;
    this.paint(true);
    const focus = p.field === 'name' ? '#g-name, #g-bname' : p.field === 'pin' ? '#g-pin, #g-bpin, #g-newpin' : p.field === 'code' ? '#g-code' : '[data-problem]';
    const el = this.root.querySelector<HTMLElement>(focus);
    if (el && focus === '[data-problem]') el.setAttribute('tabindex', '-1');
    el?.focus();
  }

  // ---------- G10 Rejoin (pa10a's, with "Open my answer") ----------

  private back(v: GatherView): HTMLElement {
    const d = this.bd;
    const host = this.host();
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-rejoin': '', 'aria-busy': this.busy === 'rejoin' ? 'true' : null }, this.busy === 'rejoin' ? 'Opening…' : 'Open my answer') as HTMLButtonElement;
    const ready = () =>
      (btn.disabled = this.busy === 'rejoin' || !d.name.trim() || (d.reset ? d.code.trim().length !== 4 || d.newPin.length !== 4 : d.pin.length !== 4));
    const name = this.nameInput('g-bname', d.name, (s) => {
      d.name = s;
      ready();
    });
    const warn = this.problem?.field === 'pin';
    let pinPart: Kid;
    if (d.reset) {
      const code = h('input', {
        id: 'g-code',
        'data-k': 'g-code',
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
      const np = new PinInput('g-newpin', 'Pick a new 4-digit PIN', (p) => {
        d.newPin = p;
        ready();
      }, d.newPin, warn);
      pinPart = [
        h('div', null, h('label', { class: 'lbl', for: 'g-code' }, 'Reset code ', h('small', null, `4 letters from ${host}`)), code),
        h('div', null, h('label', { class: 'lbl', for: 'g-newpin' }, 'Pick a new 4-digit PIN'), np.el),
      ];
    } else {
      const pin = new PinInput('g-bpin', 'Your PIN', (p) => {
        d.pin = p;
        ready();
      }, d.pin, warn);
      pinPart = h('div', null, h('label', { class: 'lbl', for: 'g-bpin' }, 'Your PIN'), pin.el);
    }
    ready();
    const what = v.facts.face === 'vote' ? 'change your vote' : 'change your answer';
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      h('div', { class: 'hero' }, tally(64, 'finished'), h('div', null, h('h1', { id: 'title' }, 'Welcome back'), h('div', { class: 'soft sub' }, d.reset ? `Type your name, the code from ${host} and a new PIN.` : `Type your name and PIN to ${what}.`))),
      h('div', null, h('label', { class: 'lbl', for: 'g-bname' }, 'Your name'), name),
      pinPart,
      this.problemBox(),
      btn,
      d.reset
        ? h('p', { class: 'center' }, this.linkButton('Use my PIN instead', () => this.setReset(false), 'use-pin'))
        : [
            h('p', { class: 'soft center fine' }, `Forgot your PIN? Ask ${host} for a reset code.`),
            h('p', { class: 'center' }, this.linkButton('I have a reset code', () => this.setReset(true), 'use-code')),
          ],
      v.facts.phase === 'open' && !v.ended ? h('p', { class: 'center' }, this.linkButton('New here? Answer now', () => this.leaveRejoin(), 'to-join')) : null,
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
    this.root.querySelector<HTMLElement>(on ? '#g-code' : '#g-bpin')?.focus();
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
    if (r.error === 'wrong_pin' || r.error === 'locked' || r.error === 'hard_locked') {
      d.pin = '';
      d.newPin = '';
    }
    if (r.error === 'reset_pending') d.reset = true;
    this.fail(pinCopy(r, this.host(), false));
  }

  // ---------- G2 You're in (RSVP) ----------

  private rsvpIn(v: GatherView): HTMLElement {
    const m = v.mine!;
    const f = v.facts;
    const seat = v.seat!;
    const host = this.host();
    const a = m.answer!;
    const food = [...m.food.map((k) => FOOD_LABELS[k].toLowerCase()), ...(m.food_other ? [m.food_other] : [])];
    const summary = [a === 'yes' && m.plus_ones ? `+${fmt(m.plus_ones)}` : '', ...food].filter(Boolean);
    const title = a === 'yes' ? `You're in, ${seat.name}!` : a === 'maybe' ? `Got it, ${seat.name}` : `Thanks, ${seat.name}`;
    const until = f.closes_at ? ` You can change your answer until ${at(f.closes_at)}.` : '';
    const ev = f.event_at ? `${at(f.event_at)}${f.place ? ` at ${f.place}` : ''}` : f.when ? `${f.when}${f.place ? ` at ${f.place}` : ''}` : '';
    const line =
      a === 'yes'
        ? `${ev ? `See you ${ev}.` : 'See you there.'}${until}`
        : a === 'maybe'
          ? `${cap(host)} sees you as a maybe for now.${until}`
          : `${cap(host)} knows you can't make it.${until}`;
    const yes = (v.people ?? []).filter((p) => p.answer === 'yes');
    const maybe = (v.people ?? []).filter((p) => p.answer === 'maybe');
    let who: Kid = null;
    if (v.people) {
      who = h(
        'div',
        null,
        h('div', { class: 'lbl', id: 'lbl-who' }, "Who's coming ", v.counts ? h('small', null, `${fmt(v.counts.coming)} with plus-ones`) : null),
        yes.length
          ? h('ul', { class: 'who-grid', 'aria-labelledby': 'lbl-who' }, yes.map((p) => h('li', null, creature(p.avatar, 'sm'), h('div', { class: 'nm' }, p.name))))
          : h('p', { class: 'soft fine' }, 'Nobody has said yes yet.'),
        maybe.length ? h('p', { class: 'soft fine' }, `Maybe: ${and(maybe.map((p) => p.name))}`) : null,
      );
    } else if (v.counts) {
      who = h(
        'div',
        null,
        h('div', { class: 'lbl' }, 'So far ', h('small', null, `${fmt(v.counts.coming)} coming with plus-ones`)),
        h('div', { class: 'facts' }, ([['Yes', v.counts.yes], ['Maybe', v.counts.maybe], ['No', v.counts.no]] as [string, number][]).map(([k, n]) => h('div', { class: 'fact' }, h('small', null, k), h('b', null, fmt(n))))),
      );
    }
    return h(
      'div',
      { class: 'body' },
      h(
        'div',
        { class: 'panel center you' },
        creature(seat.avatar, 'big'),
        h('h2', null, title),
        h('div', { class: 'soft sum', 'data-summary': '' }, h('b', null, WORD[a]), summary.length ? ` · ${summary.join(' · ')}` : ''),
      ),
      this.say(a === 'yes' ? 'finished' : undefined, line),
      // Only their own tick, and only once the host ticked it (design rule: paid is a host's tick, not a payment).
      m.paid
        ? h('div', { class: 'notice plain paid', 'data-paid': '' }, icon('tick', 'pi'), h('span', null, `${cap(host)} marked you as `, h('b', null, 'paid'), shortDay(m.paid.at) ? ` · ${shortDay(m.paid.at)}` : '', h('span', { class: 'soft' }, ' · only you see this')))
        : null,
      who,
      this.ghost('Change my answer', () => this.goEdit(), 'change'),
      this.keep(v),
    );
  }

  // ---------- Sign-up: you're in, G5 moved up ----------

  private glist(s: Slot, meId: string): HTMLElement | null {
    if (!s.people) return null;
    const rows: Kid[] = s.people.map((p) =>
      h('li', { class: `gi${p.id === meId ? ' me' : ''}` }, creature(p.avatar, 'xs'), h('span', null, p.name, p.note ? h('small', null, ` · ${p.note}`) : null), h('small', null, p.id === meId ? 'you' : 'in')),
    );
    for (const p of s.waiting_people ?? []) {
      const me = p.id === meId;
      const pos = me ? this.view?.mine?.waiting?.position : undefined;
      rows.push(h('li', { class: `gi wt${me ? ' me' : ''}` }, creature(p.avatar, 'xs'), h('span', null, p.name), h('small', null, me && pos ? `you · waiting ${nth(pos)}` : 'waiting')));
    }
    return h('ul', { class: 'glist', 'aria-label': `${s.name}: who's in` }, rows);
  }

  private letGoBlock(s: Slot, waiting: boolean): Kid {
    const next = !waiting && s.waiting_people?.[0];
    const what = waiting ? `Changed your mind? Leave the ${s.name} waiting list.` : next ? `Can't make it after all? Let the spot go, and ${next.name} moves up.` : "Can't make it after all? Let the spot go.";
    if (!this.letGo) return h('p', { class: 'soft center fine' }, what, ' ', this.linkButton(waiting ? 'Leave the list' : 'Let the spot go', () => this.askLetGo(), 'let-go'));
    const yes = h('button', { type: 'button', class: 'gbtn ghost', 'data-let-go-yes': '', 'aria-busy': this.busy === 'let_go' ? 'true' : null }, waiting ? 'Yes, leave the list' : 'Yes, let it go');
    yes.addEventListener('click', () => void this.doLetGo());
    return h(
      'div',
      { class: 'notice plain confirm', role: 'group', 'aria-label': 'Let the spot go?' },
      h('span', null, waiting ? `You'll leave the ${s.name} waiting list.` : `Your ${s.name} spot goes to ${next ? next.name : 'the next person'}.`),
      yes,
      this.linkButton('Keep it', () => this.keepIt(), 'keep-it'),
    );
  }

  private askLetGo(): void {
    this.letGo = true;
    this.paint();
    this.root.querySelector<HTMLElement>('[data-let-go-yes]')?.focus();
  }

  private keepIt(): void {
    this.letGo = false;
    this.paint();
    this.root.querySelector<HTMLElement>('[data-let-go]')?.focus();
  }

  private async doLetGo(): Promise<void> {
    if (this.busy) return;
    this.busy = 'let_go';
    this.paint();
    const r = await this.api.act(this.view?.mine?.waiting ? 'leave' : 'let_go');
    this.busy = '';
    this.letGo = false;
    if (r.ok) {
      this.jd.slot = undefined;
      this.freshMove = false;
      if (r.view) this.show(r.view);
      else this.paint();
      return;
    }
    this.failAct(r);
  }

  private slotIn(v: GatherView, moved: boolean): HTMLElement {
    const m = v.mine!;
    const seat = v.seat!;
    const s = v.facts.slots.find((x) => x.slot === m.slot)!;
    const until = v.facts.closes_at ? ` You can change it until ${at(v.facts.closes_at)}.` : '';
    const bring = m.note ? ` You're bringing ${m.note}.` : '';
    const line = moved ? `A spot opened in ${s.name}, so it's yours now.${bring}${until}` : `See you ${v.facts.when ?? 'there'}.${bring}${until}`;
    return h(
      'div',
      { class: 'body' },
      h(
        'div',
        { class: 'panel center you' },
        creature(seat.avatar, 'big'),
        // Moving up is ink New, never holo (design rule).
        moved && this.freshMove ? h('span', { class: 'kw-sticker kw-sticker-ink kw-sticker-sm', 'data-sticker': 'ink' }, h('span', null, 'New')) : null,
        h('h2', null, `You're in: ${s.name}`),
        moved ? h('div', { class: 'soft sum' }, 'You moved up from the waiting list') : m.note ? h('div', { class: 'soft sum' }, m.note) : null,
      ),
      this.say('finished', line),
      s.people ? h('div', null, h('div', { class: 'lbl' }, s.name, ' ', h('small', null, `${fmt(s.filled)} of ${fmt(s.cap)}`)), this.glist(s, seat.id)) : null,
      this.ghost('Change my slot', () => this.goEdit(), 'change'),
      this.letGoBlock(s, false),
      this.keep(v),
    );
  }

  // ---------- G4 Waiting list ----------

  private waiting(v: GatherView): HTMLElement {
    const m = v.mine!;
    const w = m.waiting!;
    const s = v.facts.slots.find((x) => x.slot === w.slot)!;
    const room = v.facts.slots.filter((x) => !x.full).map((x) => x.name);
    return h(
      'div',
      { class: 'body' },
      h(
        'div',
        { class: 'panel center' },
        h('div', { class: 'soft sum' }, `You're on the ${s.name} waiting list`),
        h('div', { class: 'bigpos', 'data-position': '' }, nth(w.position)),
        h('div', { class: 'soft sum' }, `in line · ${s.name} is ${fmt(s.filled)} of ${fmt(s.cap)}`),
      ),
      this.say(undefined, `If someone in ${s.name} drops out, you move up on your own and this page tells you. Nothing to do until then.`),
      s.people ? h('div', null, h('div', { class: 'lbl' }, s.name), this.glist(s, v.seat!.id)) : null,
      this.ghost('Pick another slot instead', () => this.goEdit(), 'change'),
      room.length ? h('p', { class: 'soft center fine' }, `${and(room)} still ${room.length === 1 ? 'has' : 'have'} room.`) : null,
      this.letGoBlock(s, true),
      this.keep(v),
    );
  }

  // ---------- G8 Voted ----------

  private bars(list: Votes[], total: number | undefined, lead: number | undefined, label: string): HTMLElement {
    return h(
      'ul',
      { class: 'vbars', 'aria-label': label },
      list.map((x) =>
        h(
          'li',
          { class: `vb${x.option === lead ? ' lead' : ''}` },
          h('span', { class: 'on' }, x.name),
          h('span', { class: 'tr', role: 'img', 'aria-label': total !== undefined ? `${x.name}: ${fmt(x.votes)} of ${fmt(total)}` : `${x.name}: ${fmt(x.votes)}` }, h('i', { class: wclass(x.votes, total ?? 0) })),
          h('span', { class: 'n', 'aria-hidden': 'true' }, fmt(x.votes)),
        ),
      ),
    );
  }

  /** The option with the most votes, when exactly one has the most (a comparison, not a new number). */
  private leader(list: Votes[]): number | undefined {
    const top = Math.max(...list.map((x) => x.votes));
    const at = list.filter((x) => x.votes === top);
    return at.length === 1 && top > 0 ? at[0].option : undefined;
  }

  private voted(v: GatherView): HTMLElement {
    const m = v.mine!;
    const f = v.facts;
    const ranked = f.ballot?.kind === 'ranked';
    const name = (o: number) => f.ballot?.options.find((x) => x.option === o)?.name ?? '';
    const mine = m.ballot ?? [];
    const live = v.live && v.live.length ? v.live : null;
    return h(
      'div',
      { class: 'body' },
      h(
        'div',
        { class: 'panel' },
        h('h2', null, `Your vote is in, ${v.seat!.name}`),
        mine.length ? h('div', { class: 'soft sum', 'data-ballot': '' }, ranked ? mine.map((o, i) => `${i + 1} ${name(o)}`).join(' · ') : name(mine[0])) : null,
      ),
      live
        ? h(
            'div',
            null,
            h('div', { class: 'lbl' }, 'Live count ', h('small', null, [ranked ? '1st choices' : 'votes', f.voted !== undefined ? `${fmt(f.voted)} voted` : ''].filter(Boolean).join(' · '))),
            this.bars(live, f.voted, this.leader(live), ranked ? 'Live count of 1st choices' : 'Live count'),
          )
        : null,
      this.say(
        undefined,
        ranked
          ? 'Ranked votes are counted when voting closes: 1st choices first, then the next choice of anyone whose pick drops out.'
          : 'The count is final when voting closes.',
      ),
      f.closes_at ? h('div', { class: 'closes' }, h('div', { class: 'lbl' }, 'Closes ', h('small', null, at(f.closes_at)))) : null,
      this.ghost('Change my vote', () => this.goEdit(), 'change'),
      this.keep(v),
    );
  }

  // ---------- G9 Vote closed ----------

  private result(v: GatherView): HTMLElement {
    const r = v.result!;
    const host = this.host();
    const last = r.rounds[r.rounds.length - 1];
    const total = r.final_round_active ?? r.ballots;
    let card: Kid;
    let line: string;
    if (r.outcome === 'winner' && r.winner) {
      const of = total !== undefined ? ` of ${fmt(total)}` : '';
      card = h(
        'div',
        { class: 'matchcard', role: 'group', 'aria-label': `The vote picked ${r.winner.name}` },
        h('div', { class: 'eyebrow' }, gaIcon('trophy'), 'The vote picked'),
        h('div', { class: 'nm' }, r.winner.name),
        h('div', { class: 'soft sum' }, `${fmt(r.winner.votes)}${of}${r.rounds.length > 1 ? ' in the final round' : ' votes'}`),
        // The one holo on the guest page: the winner, earned by the count.
        h('div', { class: 'stk' }, h('span', { class: 'kw-sticker kw-sticker-holo', 'data-sticker': 'holo' }, gaIcon('trophy'), h('span', null, 'Picked'))),
      );
      line = `${r.winner.name} it is! ${cap(host)} takes it from here.`;
    } else if (r.outcome === 'picked') {
      card = h(
        'div',
        { class: 'matchcard', role: 'group', 'aria-label': 'Tie' },
        h('div', { class: 'eyebrow' }, 'It was a tie'),
        r.winner ? h('div', { class: 'nm' }, r.winner.name) : null,
        h('div', { class: 'soft sum' }, `${cap(host)} picked${r.tied.length ? ` between ${and(r.tied.map((t) => t.name))}` : ''}.`),
      );
      line = `The vote ended in a tie, so ${host} picked.`;
    } else if (r.outcome === 'tie') {
      card = h('div', { class: 'notice plain', role: 'status' }, h('span', null, h('b', null, `Tie: ${host} picks`), h('br'), h('span', { class: 'soft' }, r.tied.length ? `${and(r.tied.map((t) => t.name))} are level.` : 'The top options are level.')));
      line = `It's a tie. ${cap(host)} picks, or opens the vote again for a day.`;
    } else {
      card = h('div', { class: 'notice plain', role: 'status' }, h('span', null, h('b', null, 'Voting is closed'), h('br'), h('span', { class: 'soft' }, 'Nobody voted.')));
      line = 'Nobody voted this time.';
    }
    const rounds =
      r.rounds.length > 1
        ? h(
            'div',
            { class: 'panel' },
            h('div', { class: 'lbl', id: 'lbl-rounds' }, 'How it was counted'),
            h(
              'ol',
              { class: 'rounds2', 'aria-labelledby': 'lbl-rounds' },
              r.rounds.map((x) => {
                const drop = x.dropped.map((d) => d.name);
                return h(
                  'li',
                  null,
                  `Round ${x.round}: ${x.counts.map((c) => `${c.name} ${fmt(c.votes)}`).join(' · ')}.`,
                  drop.length ? ` ${and(drop)} ${drop.length === 1 ? 'drops' : 'drop'} out.` : '',
                  x.exhausted ? ` ${fmt(x.exhausted)} ${x.exhausted === 1 ? 'ballot had' : 'ballots had'} no choice left.` : '',
                );
              }),
            ),
          )
        : null;
    return h(
      'div',
      { class: 'body' },
      card,
      last && last.counts.length ? h('div', null, h('div', { class: 'lbl' }, r.rounds.length > 1 ? 'Final round' : 'Votes'), this.bars(last.counts, total, r.winner?.option, 'Final round')) : null,
      rounds,
      this.say(r.outcome === 'winner' ? 'finished' : undefined, line),
      v.expires_at ? h('div', { class: 'keep', 'data-keep': '' }, icon('lock', 'pi'), h('span', null, this.gone_(v))) : null,
    );
  }

  // ---------- G11 Closed ----------

  private closed(v: GatherView): HTMLElement {
    const f = v.facts;
    const host = this.host();
    const title = v.ended && f.phase === 'open' ? 'This has ended' : f.face === 'rsvp' ? 'RSVPs are closed' : f.face === 'signup' ? 'Sign-ups are closed' : 'Voting is closed';
    let lead = `Need to change something? Ask ${host}.`;
    if (f.face === 'rsvp' && v.counts) lead = `${fmt(v.counts.coming)} ${v.counts.coming === 1 ? 'is' : 'are'} coming${f.event_at ? ` on ${at(f.event_at)}` : ''}. ${lead}`;
    let back: HTMLElement | null = null;
    if (!v.ended && !v.seat && f.face === 'rsvp') {
      back = this.ghost('See my answer', () => this.goRejoin(), 'to-rejoin');
    }
    const mine = v.mine?.answer ? h('p', { class: 'soft' }, 'Your answer: ', h('b', null, WORD[v.mine.answer])) : null;
    return this.message(tally(96, 'finished'), title, lead, this.gone_(v), mine, back);
  }

  // ---------- Host asleep (pa10a's G9) ----------

  private awayLine(): string {
    const a = this.away!;
    return `Trying again in ${a.left} s · tried ${a.tries} time${a.tries === 1 ? '' : 's'}`;
  }

  private awayScreen(): HTMLElement {
    const host = this.host();
    const btn = h('button', { type: 'button', class: 'gbtn ghost', 'data-try': '' }, 'Try now');
    btn.addEventListener('click', this.tryNow);
    const what = this.view?.seat ? 'Your answer is safe. ' : '';
    return h(
      'div',
      { class: 'body mid', role: 'status' },
      h('div', { class: 'qmwrap' }, tally(96, 'thinking')),
      h('h2', null, `${cap(poss(host))} computer is asleep`),
      h('p', { class: 'lead' }, `This page lives on ${poss(host)} computer, and it's off or asleep right now. ${what}The page tries again by itself.`),
      h('p', { class: 'small', 'data-away': '' }, this.awayLine()),
      btn,
    );
  }
}
