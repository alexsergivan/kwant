// The front desk's guest page (Guest Guide), G1-G12 in pa13/guest-390.html, on Lin's desk API (pa13/impl.md, which
// wins over design.md). Loaded as its own chunk when the pocket's kit is `desk`; the other kits' screens are untouched.
// Plain DOM, no framework; text goes in as text only. Every number shown comes from the server except the character
// count of what the guest is typing. The page only ever holds this guest's own conversation.
import { Api, Env, Fail, Live } from '../api';
import { AVATARS, fmt, h, Kid } from '../dom';
import { clock, day } from '../app';
import { PinInput, Problem } from '../santa/pin';
import { book, joinCopy, Line, longCopy, names, Names, noteCopy, offCopy, sayCopy, slowCopy } from './copy';
import { bell, dIcon, Mood } from './look';
import { clientId, Out, Outbox, Store } from './outbox';
import { closedPick, Pick, picker } from './picker';
import { chipLabel, openSlots, timeLabel, whenLabel } from './slots';
import { bookingFrom, deskFrom, DeskView, Msg, Screen, screenOf, slotsFrom } from './view';
import { bookingWhen, strip } from './strip';

/** design.md "For Iris" 4: without a socket the page asks `state` every 15 s. */
export const POLL_DESK_MS = 15_000;
const INFO_MS = 10_000;
/** How close to the end of the thread still counts as "at the bottom" (px). */
const BOTTOM_PX = 32;

export type DeskEnv = Env & {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
  /** localStorage when the browser allows it: the outbox. */
  store?: Store | null;
};

type JoinDraft = { name: string; pin: string };
type BackDraft = { name: string; pin: string; code: string; newPin: string; reset: boolean };
type Shown = Screen | 'away' | 'removed' | 'missing' | 'loading';

const chars = (s: string): number => [...s].length;
/** "the Sea-view flat", but "The Anchor" stays as it is. */
const theTitle = (t: string): string => (/^the\s/i.test(t) ? t : `the ${t}`);

/** "Today", else "Mon, Nov 23" (the phone's own calendar). */
export function dayLabel(iso: string, now: number): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const t = new Date(now);
  if (d.getFullYear() === t.getFullYear() && d.getMonth() === t.getMonth() && d.getDate() === t.getDate()) return 'Today';
  return d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });
}

const dayKey = (iso: string): string => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
};

export class DeskApp {
  view: DeskView | null = null;
  readonly api: Api<DeskView>;
  readonly outbox: Outbox;
  private live: Live<DeskView>;
  private root: HTMLElement;
  private sig = '';
  private away: { secs: number; tries: number; left: number } | null = null;
  private awayTick = 0;
  private infoTimer = 0;
  private slowTimer = 0;
  private gone: 'removed' | 'missing' | null = null;
  private hadSeat = false;
  private rejoin = false;
  private jd: JoinDraft = { name: '', pin: '' };
  private bd: BackDraft = { name: '', pin: '', code: '', newPin: '', reset: false };
  private problem: Problem | null = null;
  private busy = '';
  /** What the guest is typing: kept across redraws, a refusal and the box going away. */
  draft = '';
  /** 429 slow_down: the box's retry_at. The composer keeps the text, switched off, until then. */
  slowUntil: string | null = null;
  /** A refused message (not 429/413/410), the box's reason in plain words. */
  private sayErr = '';
  /** Messages already drawn: only new ones get the entrance motion. */
  private drawn = new Set<string>();
  private painted = false;
  private stick = true;
  private sending = false;
  /** pa13b: the Pick a time sheet. */
  pick: Pick = closedPick();
  /** pa13b K7: the strip is asking "Cancel {when}?". */
  private asking = false;
  private cancelBusy = false;
  /** pa13b: a refused cancel, in the page's words, under the strip. */
  private stripSay = '';
  /** pa13b K1: "See open times" opens the sheet once the seat is there. */
  private pickAfterJoin = false;
  /** The strip's last drawn state: it crossfades only when that changes, not on every redraw. */
  private stripKey = '';

  constructor(root: HTMLElement, private env: DeskEnv, readonly code: string) {
    this.root = root;
    this.api = new Api<DeskView>(env, code, deskFrom);
    this.outbox = new Outbox(env.store);
    this.live = new Live<DeskView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => this.goAway(secs, tries),
      back: () => this.comeBack(),
      gone: (status, error) => void this.onGone(status, error),
    });
    this.live.pollMs = POLL_DESK_MS;
  }

  /** The page's public facts first, then `state` finds out whether this phone has a seat. */
  async start(boot?: unknown): Promise<void> {
    const b = boot ? this.api.take(boot) : undefined;
    if (b) this.show(b);
    else this.paint();
    if (b?.me) return this.seated();
    const r = await this.api.state();
    if (r.ok && r.view) {
      this.show(r.view);
      if (r.view.me) this.seated();
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
    this.env.clearTimeout(this.slowTimer);
  }

  /** The phone woke or came back online: ask the box again and send what didn't go. */
  wake(): void {
    if (this.gone || !this.view?.me) return;
    this.live.now();
    void this.resend();
  }

  /** A seat: live updates on, and messages kept from an earlier visit go again. */
  private seated(): void {
    this.live.start();
    void this.resend();
  }

  // ---------- incoming ----------

  show(v: DeskView): void {
    if (this.gone) return;
    this.view = v;
    if (v.me) {
      this.hadSeat = true;
      this.rejoin = false;
      this.outbox.open(this.code, v.me.id);
      this.outbox.settle(v.messages);
      // The box's own "no more this hour" (a reload during G6) switches the composer off the same way.
      if (v.limits.left_this_hour === 0 && v.limits.next_at && !this.slowUntil) this.slow(v.limits.next_at);
    }
    if (v.my_booking?.state !== 'requested') this.asking = false;
    if (v.ended) {
      this.endLive();
      this.pick = closedPick();
    }
    this.paint();
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
    if (status === 410 || error === 'ended') return this.ended();
    if (status === 403 || error === 'removed') this.end('removed');
    else if (status === 404 || error === 'not_found') this.end('missing');
  }

  /** 410 ended (G12): the last view, closed. The delete date stays the server's. */
  private async ended(): Promise<void> {
    this.endLive();
    if (this.view) return this.show({ ...this.view, ended: true });
    // Opened cold (no boot facts): the public facts still name the desk until it goes.
    const i = await this.api.info();
    if (i.ok && i.view) this.show({ ...i.view, ended: true });
    else this.end('missing');
  }

  private endLive(): void {
    this.live.stop();
    this.env.clearTimeout(this.infoTimer);
    this.env.clearInterval(this.awayTick);
    this.away = null;
  }

  private end(why: 'removed' | 'missing'): void {
    this.endLive();
    this.gone = why;
    this.paint();
  }

  /** Before a seat there is no socket: the public facts are read again every 10 s (an End shows up). */
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
    void this.resend();
  }

  tryNow = (): void => {
    this.live.now();
    void this.resend();
  };

  // ---------- sending (the outbox) ----------

  /** Sends what is in the composer (or a starter chip). Over the limit, empty or in a slow-down it does nothing. */
  async send(text = this.draft): Promise<void> {
    const v = this.view;
    const t = text.trim();
    if (!v?.me || v.ended || !t || this.isSlow() || chars(t) > v.limits.max_chars) return;
    if (text === this.draft) this.draft = '';
    this.sayErr = '';
    const o = this.outbox.add(t, clientId(this.env.now()), new Date(this.env.now()).toISOString());
    this.stick = true;
    this.paint();
    await this.post(o);
  }

  /** One message to the box. 200 swaps it; a network error keeps it (G9); 429 gives the text back (G6). */
  private async post(o: Out): Promise<void> {
    this.outbox.mark(o.client_id, 'sending');
    const r = await this.api.act('say', { text: o.text, client_id: o.client_id });
    if (r.ok) {
      if (r.view) {
        this.show(r.view);
        // An old box that doesn't echo client_id: the reply has it now, whatever the id.
        if (this.outbox.get(o.client_id)) {
          this.outbox.drop(o.client_id);
          this.paint();
        }
      } else {
        this.outbox.drop(o.client_id);
        this.paint();
      }
      return;
    }
    if (r.status === 0 || r.status >= 500) {
      this.outbox.mark(o.client_id, 'failed');
      return this.paint();
    }
    this.outbox.drop(o.client_id);
    if (r.status === 410 || r.error === 'ended') return this.ended();
    if (r.status === 401 || r.error === 'signed_out') {
      this.giveBack(o.text);
      return this.onGone(401, 'signed_out');
    }
    if (r.status === 403 || r.status === 404) return this.onGone(r.status, r.error);
    // The box kept nothing: the text goes back into the composer, ready to send.
    this.giveBack(o.text);
    if (r.status === 429 || r.error === 'slow_down') this.slow(this.retryAt(r));
    else if (r.status !== 413 && r.error !== 'too_long') this.sayErr = sayCopy(r);
    this.paint(true);
  }

  /** A message the box didn't keep goes back into the composer, in front of anything typed since. */
  private giveBack(text: string): void {
    const now = this.draft.trim();
    if (now === text) return;
    this.draft = now ? `${text}\n${this.draft}` : text;
  }

  private retryAt(r: Fail): string {
    return r.retry_at ?? this.view?.limits.next_at ?? new Date(this.env.now() + 60_000).toISOString();
  }

  /** G9 Retry, `online`, waking up or a new seat: the kept messages go again, oldest first. */
  async resend(): Promise<void> {
    if (this.sending || !this.view?.me || this.view.ended) return;
    const list = this.outbox.failed();
    if (!list.length) return;
    this.sending = true;
    for (const o of list) {
      if (!this.view?.me || this.view.ended || this.isSlow()) break;
      this.outbox.mark(o.client_id, 'sending');
      this.paint();
      await this.post(o);
      if (this.outbox.get(o.client_id)?.state === 'failed') break;
    }
    this.sending = false;
  }

  private slow(at: string): void {
    const ms = new Date(at).getTime() - this.env.now();
    if (!(ms > 0)) return;
    this.slowUntil = at;
    this.env.clearTimeout(this.slowTimer);
    this.slowTimer = this.env.setTimeout(() => {
      this.slowUntil = null;
      this.paint();
    }, Math.min(ms + 500, 2 ** 31 - 1));
  }

  isSlow(): boolean {
    return !!this.slowUntil && new Date(this.slowUntil).getTime() > this.env.now();
  }

  // ---------- drawing ----------

  private n(): Names {
    return names(this.view);
  }

  screenNow(): Shown {
    if (this.gone) return this.gone;
    const v = this.view;
    if (!v) return this.away ? 'away' : 'loading';
    const s = screenOf(v, { rejoin: this.rejoin });
    if (this.away && s !== 'chat' && s !== 'ended') return 'away';
    return s;
  }

  /** Draws the screen when something it shows changed. Typing never redraws; focus, caret and the thread's place survive. */
  paint(force = false): void {
    const scr = this.screenNow();
    const sig = JSON.stringify([scr, this.view, this.problem, this.busy, this.bd.reset, this.outbox.items, this.isSlow(), this.slowUntil, this.sayErr, this.away?.tries, { ...this.pick, note: 0 }, this.asking, this.cancelBusy, this.stripSay]);
    if (!force && sig === this.sig) return;
    this.sig = sig;
    const active = document.activeElement as HTMLInputElement | null;
    const k = active?.getAttribute?.('data-k');
    const sel = k && typeof active?.selectionStart === 'number' ? [active.selectionStart, active.selectionEnd ?? active.selectionStart] : null;
    const old = this.root.querySelector<HTMLElement>('.thread');
    const atBottom = !old || old.scrollHeight - old.scrollTop - old.clientHeight <= BOTTOM_PX;
    const top = old?.scrollTop ?? 0;
    const before = new Set(this.drawn);
    const chat = scr === 'chat';
    const sheet = chat && this.view ? this.sheet(this.view) : [];
    const page = h('main', { class: `g dk${chat ? ' gc' : ''} s-${scr}${sheet.length ? ' picking' : ''}`, 'data-screen': scr }, this.bar(scr), this.body(scr), chat ? null : this.madeWith(), sheet);
    // The chat behind the sheet can't be reached while it is open (aria-modal plus inert).
    if (sheet.length) for (const el of page.children) if (!sheet.includes(el as HTMLElement)) el.setAttribute('inert', '');
    this.pick.fresh = false;
    this.root.replaceChildren(page);
    if (k) {
      const el = this.root.querySelector<HTMLInputElement>(`[data-k="${k}"]`);
      if (el && !el.disabled) {
        el.focus({ preventScroll: true });
        if (sel) el.setSelectionRange?.(sel[0], sel[1]);
      }
    }
    const thread = this.root.querySelector<HTMLElement>('.thread');
    if (thread) {
      const fresh = [...this.drawn].some((id) => !before.has(id));
      if (this.stick || atBottom || !this.painted) thread.scrollTop = thread.scrollHeight;
      else {
        thread.scrollTop = top;
        if (fresh) this.root.querySelector('[data-newpill]')?.removeAttribute('hidden');
      }
      this.stick = false;
      thread.addEventListener('scroll', () => {
        if (thread.scrollHeight - thread.scrollTop - thread.clientHeight <= BOTTOM_PX) this.root.querySelector('[data-newpill]')?.setAttribute('hidden', '');
      });
      this.painted = true;
    }
  }

  private body(scr: Shown): HTMLElement {
    const v = this.view;
    const n = this.n();
    switch (scr) {
      case 'loading':
        return this.message(bell(96, 'thinking'), 'Opening…', '');
      case 'removed':
        return this.message(bell(96), "You've been removed from this chat", `${n.Host} took you out of it. For anything else, contact ${n.host} the way you booked.`);
      case 'missing':
        return this.message(bell(96), 'Nothing here', "This link doesn't open a pocket app any more. Check the link with the host.");
      case 'away':
        return this.awayScreen();
      case 'join':
        return this.join(v!);
      case 'rejoin':
        return this.back(v!);
      case 'chat':
        return this.chat(v!);
      case 'ended':
        return this.closed(v!);
    }
  }

  private bar(scr: Shown): HTMLElement {
    const v = this.view;
    const right = !v ? '' : scr === 'chat' ? (v.me?.name ?? '') : scr === 'join' || scr === 'rejoin' || scr === 'away' ? v.place : '';
    return h('header', { class: 'bar' }, bell(28, scr === 'chat' ? this.mood(v!) : undefined), h('b', null, v?.title ?? ''), h('span', { class: 'sp' }), h('span', { class: 'muted tab', 'data-bar': '' }, right));
  }

  /** G10: Bell wears its failed badge while its last word is "can't answer right now". */
  private mood(v: DeskView): Mood {
    if (v.typing) return 'thinking';
    const last = [...v.messages].reverse().find((m) => m.from !== 'guest');
    return last?.note === 'host_away' && v.waiting ? 'failed' : undefined;
  }

  private madeWith(): HTMLElement {
    return h('footer', { class: 'madewith' }, 'made with ', h('a', { href: 'https://kwant.si', rel: 'noopener noreferrer', target: '_blank' }, 'kwant'));
  }

  private message(art: HTMLElement, title: string, lead: string, ...more: Kid[]): HTMLElement {
    return h('div', { class: 'body mid' }, h('div', { class: 'qmwrap' }, art), h('h2', null, title), lead ? h('p', { class: 'lead' }, lead) : null, ...more);
  }

  private keep(text: string): HTMLElement {
    return h('div', { class: 'keep', 'data-keep': '' }, dIcon('lock'), h('span', null, text));
  }

  /** "Messages are deleted 7 days after your last one." The number is the box's keep_days. */
  private keepTail(v: DeskView, your = false): string {
    const d = v.keep_days;
    const what = your ? 'Your messages' : 'Messages';
    return d === undefined ? `${what} are deleted after your last one.` : `${what} are deleted ${fmt(d)} day${d === 1 ? '' : 's'} after your last one.`;
  }

  private hero(v: DeskView): HTMLElement {
    const n = this.n();
    const sub = [v.place, n.anon ? '' : `hosted by ${n.host}`].filter(Boolean).join(' · ');
    return h('div', { class: 'hero' }, bell(64), h('div', null, h('h1', { id: 'title' }, v.title), sub ? h('div', { class: 'soft sub' }, sub) : null));
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

  private bold(line: Line): Kid[] {
    return [h('b', null, line[0]), ` ${line[1]}`];
  }

  // G1 Join
  private join(v: DeskView): HTMLElement {
    const d = this.jd;
    const n = this.n();
    const pinOn = v.needs_pin;
    const busy = this.busy === 'join';
    const bk = v.booking;
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-join': '', 'aria-busy': busy ? 'true' : null }, dIcon(bk ? 'cal' : 'chat'), busy ? 'Opening…' : bk ? book.join_button : 'Start chatting') as HTMLButtonElement;
    const ready = () => (btn.disabled = busy || !d.name.trim() || (pinOn && d.pin.length !== 4));
    const name = this.nameInput('d-name', d.name, (s) => {
      d.name = s;
      ready();
    }, 'name-why');
    const pin = pinOn
      ? new PinInput('d-pin', 'Pick a 4-digit PIN', (p) => {
          d.pin = p;
          ready();
        }, d.pin, this.problem?.field === 'pin', 'pin-why')
      : null;
    ready();
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      this.keep(bk ? book.keep_booking(n, v.keep_days) : `${n.Host} and ${n.hosts} bot, ${n.bot}, read this chat. ${n.bot} answers first. ${this.keepTail(v)}`),
      this.hero(v),
      h('div', { class: 'say' }, h('div', { class: 'b' }, bk ? book.join_say(n, v.what) : `Ask me anything about your stay: Wi-Fi, check-out, parking. I answer from ${n.hosts} house guide and ask ${n.host} when I can't.`)),
      h('div', null, h('label', { class: 'lbl', for: 'd-name' }, 'Your first name ', h('small', { id: 'name-why' }, `${n.host} sees it`)), name),
      pin ? h('div', null, h('label', { class: 'lbl', for: 'd-pin' }, 'Pick a 4-digit PIN ', h('small', { id: 'pin-why' }, 'to come back on another phone')), pin.el) : null,
      this.problemBox(),
      btn,
      h(
        'div',
        { class: 'ginfos' },
        h('p', { class: 'ginfo' }, dIcon('bot'), h('span', null, `${n.bot} is a bot. To write a reply it sends your message to ${n.hosts} AI service. Nothing else about you leaves ${n.hosts} computer.`)),
        h('p', { class: 'ginfo' }, dIcon('person'), h('span', null, bk ? book.join_nobook(n) : `${n.bot} can't say yes to anything: a late check-out or an extra night is ${n.hosts} call.`)),
      ),
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
    this.root.querySelector<HTMLElement>('#d-bname')?.focus();
  }

  private leaveRejoin(): void {
    this.rejoin = false;
    this.problem = null;
    this.paint();
    this.root.querySelector<HTMLElement>('#d-name')?.focus();
  }

  async submitJoin(): Promise<void> {
    const d = this.jd;
    const v = this.view;
    if (!v || this.busy) return;
    const name = d.name.trim();
    if (!name) return this.fail({ title: 'Type your first name.', field: 'name' });
    if (v.needs_pin && d.pin.length !== 4) return this.fail({ title: 'Pick a 4-digit PIN.', field: 'pin' });
    this.busy = 'join';
    this.problem = null;
    this.pickAfterJoin = v.booking;
    this.paint();
    const r = await this.api.join(v.needs_pin ? { name, avatar: AVATARS[0], pin: d.pin } : { name, avatar: AVATARS[0] });
    this.busy = '';
    if (r.ok) {
      this.env.clearTimeout(this.infoTimer);
      this.jd = { name: '', pin: '' };
      this.stick = true;
      if (r.view) this.show(r.view);
      this.seated();
      if (this.pickAfterJoin && this.view?.me) this.openPick('', '');
      this.pickAfterJoin = false;
      return;
    }
    this.pickAfterJoin = false;
    if (r.status === 410 || r.error === 'ended') return this.ended();
    if (r.status === 404) return void this.onGone(404, 'not_found');
    if (r.error === 'pin_required') d.pin = '';
    this.fail(joinCopy(r, this.n(), true));
  }

  private fail(p: Problem): void {
    this.problem = p;
    this.paint(true);
    const focus = p.field === 'name' ? '#d-name, #d-bname' : p.field === 'pin' ? '#d-pin, #d-bpin, #d-newpin' : p.field === 'code' ? '#d-code' : null;
    if (focus) this.root.querySelector<HTMLElement>(focus)?.focus();
  }

  // G11 Rejoin (wrong PIN, lockout and the reset code as pa10a)
  private back(v: DeskView): HTMLElement {
    const d = this.bd;
    const n = this.n();
    const pinOn = v.needs_pin;
    const busy = this.busy === 'rejoin';
    const btn = h('button', { class: 'gbtn', type: 'submit', 'data-rejoin': '', 'aria-busy': busy ? 'true' : null }, busy ? 'Opening…' : 'Open my chat') as HTMLButtonElement;
    const ready = () =>
      (btn.disabled = busy || !d.name.trim() || (pinOn && (d.reset ? d.code.trim().length !== 4 || d.newPin.length !== 4 : d.pin.length !== 4)));
    const name = this.nameInput('d-bname', d.name, (s) => {
      d.name = s;
      ready();
    });
    const warn = this.problem?.field === 'pin';
    let pinPart: Kid = null;
    if (pinOn && d.reset) {
      const bad = this.problem?.field === 'code';
      const code = h('input', {
        id: 'd-code',
        'data-k': 'd-code',
        class: `input code${bad ? ' bad' : ''}`,
        type: 'text',
        maxlength: 4,
        autocomplete: 'off',
        autocapitalize: 'characters',
        spellcheck: 'false',
        'aria-invalid': bad ? 'true' : null,
      }) as HTMLInputElement;
      code.value = d.code;
      code.addEventListener('input', () => {
        d.code = code.value;
        ready();
      });
      const np = new PinInput('d-newpin', 'Pick a new 4-digit PIN', (p) => {
        d.newPin = p;
        ready();
      }, d.newPin, warn);
      pinPart = [
        h('div', null, h('label', { class: 'lbl', for: 'd-code' }, 'Reset code ', h('small', null, `4 letters from ${n.host}`)), code),
        h('div', null, h('label', { class: 'lbl', for: 'd-newpin' }, 'Pick a new 4-digit PIN'), np.el),
      ];
    } else if (pinOn) {
      const pin = new PinInput('d-bpin', 'Your PIN', (p) => {
        d.pin = p;
        ready();
      }, d.pin, warn);
      pinPart = h('div', null, h('label', { class: 'lbl', for: 'd-bpin' }, 'Your PIN'), pin.el);
    }
    ready();
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'back-title' },
      this.keep(`${n.Host} and ${n.hosts} bot, ${n.bot}, read this chat. ${n.bot} answers first. ${this.keepTail(v)}`),
      this.hero(v),
      h(
        'div',
        null,
        h('h2', { id: 'back-title' }, 'Welcome back'),
        h('p', { class: 'soft sub2' }, d.reset ? `Type your first name, the code from ${n.host} and a new PIN.` : pinOn ? 'Your first name and PIN bring your chat back, on any phone.' : 'Your first name brings your chat back, on any phone.'),
      ),
      h('div', null, h('label', { class: 'lbl', for: 'd-bname' }, 'Your first name'), name),
      pinPart,
      this.problemBox(),
      btn,
      !pinOn
        ? null
        : d.reset
          ? h('p', { class: 'center' }, this.linkButton('Use my PIN instead', () => this.setReset(false), 'use-pin'))
          : [h('p', { class: 'small center' }, `Forgot it? Ask ${n.host} for a reset code.`), h('p', { class: 'center' }, this.linkButton('I have a reset code', () => this.setReset(true), 'use-code'))],
      h('p', { class: 'center' }, this.linkButton('New here? Start chatting', () => this.leaveRejoin(), 'to-join')),
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
    this.root.querySelector<HTMLElement>(on ? '#d-code' : '#d-bpin')?.focus();
  }

  async submitRejoin(): Promise<void> {
    const d = this.bd;
    const v = this.view;
    if (!v || this.busy) return;
    const name = d.name.trim();
    if (!name) return this.fail({ title: 'Type your first name.', field: 'name' });
    this.busy = 'rejoin';
    this.problem = null;
    this.paint();
    const body = !v.needs_pin ? { name } : d.reset ? { name, reset_code: d.code.trim().toUpperCase(), new_pin: d.newPin } : { name, pin: d.pin };
    const r = await this.api.call('rejoin', body);
    this.busy = '';
    if (r.ok) {
      this.env.clearTimeout(this.infoTimer);
      this.bd = { name: '', pin: '', code: '', newPin: '', reset: false };
      this.stick = true;
      if (r.view) this.show(r.view);
      return this.seated();
    }
    if (r.status === 410 && r.error !== 'reset_expired') return this.ended();
    if (r.status === 404 && r.error !== 'no_such_player') return void this.onGone(404, 'not_found');
    if (r.error === 'wrong_pin' || r.error === 'locked' || r.error === 'hard_locked') {
      d.pin = '';
      d.newPin = '';
    }
    if (r.error === 'reset_pending') d.reset = true;
    if (r.error === 'no_such_player' || r.status === 404) return this.fail({ title: 'No chat with that name here.', sub: 'Check the spelling, or start a new chat.', field: 'name' });
    this.fail(joinCopy(r, this.n(), false));
  }

  // G2-G10 Chat
  private chat(v: DeskView): HTMLElement {
    const n = this.n();
    const me = v.me!;
    const bk = v.booking;
    const items: Kid[] = [this.keep(bk ? book.keep_booking(n, v.keep_days) : `${n.Host} and ${n.bot} read this chat. ${this.keepTail(v)}`)];
    const now = this.env.now();
    let lastDay = '';
    const divider = (at: string) => {
      const k = dayKey(at);
      if (!k || k === lastDay) return null;
      lastDay = k;
      return h('div', { class: 'gday' }, dayLabel(at, now));
    };
    // Bell's hello is the page's own (the box keeps no greeting): first in the thread, under today's divider when empty.
    const first = v.messages[0]?.at;
    items.push(divider(first ?? new Date(now).toISOString()));
    items.push(this.botBubble(bk ? book.hello(me.name, n) : `Hi ${me.name}! I'm ${n.bot}, ${n.hosts} bot. Ask me about the stay. If I don't know, I'll ask ${n.host}.`, 'hello'));
    for (const m of v.messages) {
      items.push(divider(m.at));
      items.push(this.msg(m, n));
    }
    // K6: after "can't do", with no booking now, the next step is right there.
    const lastBook = [...v.messages].reverse().find((m) => m.note?.startsWith('book'));
    const declined = !!(bk && !v.my_booking && lastBook?.note === 'booking_declined');
    if (declined) {
      const again = h('button', { type: 'button', class: 'gsmall pri again', 'data-k': 'pk-open-again', 'data-pick-again': '' }, dIcon('cal'), book.pick_again);
      again.addEventListener('click', () => this.openPick('', 'pk-open-again'));
      items.push(again);
    }
    for (const o of this.outbox.items) items.push(this.mine(o));
    if (v.typing) items.push(this.typing(n));
    if (v.waiting) items.push(h('p', { class: 'waitpill', role: 'status', 'data-waiting': '' }, dIcon('clock'), `Waiting for ${n.host}`));
    const slow = this.isSlow();
    if (slow) {
      const c = slowCopy(this.slowUntil!);
      items.push(
        h('div', { class: 'gnote warn', role: 'status', 'data-slow': '' }, dIcon('clock'), h('span', null, h('b', null, c.lead), ' You can write again at ', h('b', null, c.time || 'soon'), `. ${c.rest}`)),
      );
    }
    const thread = h('div', { class: 'thread', role: 'log', 'aria-label': `Chat with ${n.bot}`, 'aria-live': 'polite', tabindex: 0 }, items);
    const pill = h('button', { type: 'button', class: 'waitpill newpill', 'data-newpill': '', hidden: true }, 'New message');
    pill.addEventListener('click', () => {
      const t = this.root.querySelector<HTMLElement>('.thread');
      if (t) t.scrollTop = t.scrollHeight;
      pill.setAttribute('hidden', '');
    });
    const empty = !v.messages.some((m) => m.from === 'guest') && !this.outbox.items.length;
    // pa13b K4/K5: while a time is held the mockup shows no starters at all; the strip is the next step. K6: nor while
    // "Pick another time" shows (Mira's design check FIX 1: one picker button).
    const quick = empty && !slow && !(bk && v.my_booking) && !declined ? v.starters : [];
    // pa13b K2: "Pick a time" comes first, and stays while the guest has no booking (with nothing open it opens K8).
    const pickChip = bk && !v.my_booking && !declined;
    let starters: HTMLElement | null = null;
    if (quick.length || pickChip) {
      const chips = quick.map((s) => {
        const b = h('button', { type: 'button', 'data-starter': '' }, s);
        b.addEventListener('click', () => void this.send(s));
        return b;
      });
      if (pickChip) {
        const b = h('button', { type: 'button', class: 'pick', 'data-k': 'pk-open-start', 'data-pick-open': '' }, dIcon('cal'), book.starter_pick);
        b.addEventListener('click', () => this.openPick('', 'pk-open-start'));
        chips.unshift(b);
      }
      starters = h('div', { class: 'gstart', role: 'group', 'aria-label': 'Quick questions' }, chips);
    }
    return h('div', { class: 'chat' }, this.bookingStrip(v, n), this.offbar(), h('div', { class: 'tw' }, thread, pill), starters, this.composer(v, n, slow));
  }

  private offbar(): HTMLElement | null {
    const failed = this.outbox.failed().length > 0;
    if (!failed && !this.away) return null;
    const retry = h('button', { type: 'button', class: 'gretry', 'data-retry': '' }, dIcon('retry'), 'Retry');
    retry.addEventListener('click', this.tryNow);
    const text = failed ? this.bold(offCopy) : [h('b', null, offCopy[0]), ' ', h('span', { 'data-away': '' }, this.awayLine())];
    return h('div', { class: 'offbar', role: 'status', 'data-offline': '' }, dIcon('nowifi'), h('span', { class: 'grow' }, text), retry);
  }

  private botBubble(text: Kid, id: string, at?: string): HTMLElement {
    const n = this.n();
    return this.bubble('', id, bell(28), [h('b', null, n.bot), h('span', { class: 'bottag' }, 'bot'), at ? ` · ${clock(at)}` : null], text);
  }

  private bubble(kind: string, id: string, av: Kid, who: Kid[], text: Kid): HTMLElement {
    const isNew = this.painted && !this.drawn.has(id);
    this.drawn.add(id);
    return h('div', { class: `gm${kind ? ` ${kind}` : ''}${isNew ? ' new' : ''}`, 'data-id': id }, h('span', { class: 'av' }, av), h('div', { class: 'col' }, h('div', { class: 'who' }, who), h('div', { class: 'b' }, text)));
  }

  private msg(m: Msg, n: Names): HTMLElement {
    if (m.from === 'note') {
      const c = noteCopy(m.note!, n, this.noteWhen(m));
      const isNew = this.painted && !this.drawn.has(m.id);
      this.drawn.add(m.id);
      return h('div', { class: `gnote${c.ok ? ' ok' : ''}${isNew ? ' new' : ''}`, role: 'status', 'data-note': m.note!, 'data-id': m.id }, dIcon(c.icon), h('span', null, this.bold(c.line)));
    }
    if (m.from === 'guest') return this.mine({ client_id: m.client_id ?? m.id, text: m.text, at: m.at, state: 'sending' }, m);
    if (m.from === 'host') {
      const tile = h('span', { class: 'hostav', 'aria-hidden': 'true' }, n.anon ? dIcon('bell') : [...n.host][0].toUpperCase());
      return this.bubble('host', m.id, tile, [h('b', null, n.Host), h('span', { class: 'hosttag' }, 'host'), ` · ${clock(m.at)}`], m.text);
    }
    return this.botBubble(m.offers ? [m.text, this.offers(m)] : m.text, m.id, m.at);
  }

  /** A booking note's time: its own start, else its slot's. Without either the note says it without a time. */
  private noteWhen(m: Msg): string {
    if (!m.start && !m.slot) return '';
    const s = this.view?.slots.find((x) => x.id === m.slot);
    return whenLabel({ id: m.slot ?? '', start: m.start ?? s?.start ?? '', label: s?.label });
  }

  /** pa13b K9: Bell's offered times as chips; a chip whose time isn't open any more is left out. */
  private offers(m: Msg): HTMLElement | null {
    const v = this.view;
    if (!v?.booking || v.my_booking) return null;
    const open = openSlots(v.slots, this.env.now());
    const list = (m.offers ?? []).map((id) => open.find((s) => s.id === id)).filter((s) => !!s);
    if (!list.length) return null;
    return h(
      'div',
      { class: 'offer', role: 'group', 'aria-label': book.offers_label(this.n()) },
      list.map((s) => {
        const k = `offer-${m.id}-${s!.id}`;
        const b = h('button', { type: 'button', 'data-k': k, 'data-offer': s!.id }, chipLabel(s!));
        b.addEventListener('click', () => this.openPick(s!.id, k));
        return b;
      }),
    );
  }

  /** The guest's own bubble: the box's (`sent`), or one in the outbox (sending, or failed with Try again). */
  private mine(o: Out, sent?: Msg): HTMLElement {
    const id = sent?.id ?? `out:${o.client_id}`;
    // The swap: the box's copy takes the outbox bubble's place without moving in again.
    const was = this.drawn.has(`out:${o.client_id}`);
    const isNew = this.painted && !this.drawn.has(id) && !was;
    this.drawn.add(id);
    let tail: Kid;
    if (sent) tail = h('span', { class: 't' }, clock(sent.at));
    else if (o.state === 'sending') tail = h('span', { class: 't' }, 'Sending…');
    else {
      const b = h('button', { type: 'button', class: 'gretry', 'data-try-one': '' }, dIcon('retry'), 'Not sent · Try again');
      b.addEventListener('click', this.tryNow);
      tail = b;
    }
    const state = sent ? '' : ` ${o.state}`;
    return h('div', { class: `gm me${state}${isNew ? ' new' : ''}`, 'data-id': id }, h('div', { class: 'col' }, h('div', { class: 'b' }, h('span', { class: 'vh' }, 'You: '), sent?.text ?? o.text), tail));
  }

  private typing(n: Names): HTMLElement {
    return h(
      'div',
      { class: 'gm', 'data-typing': '' },
      h('span', { class: 'av' }, bell(28, 'thinking')),
      h('div', { class: 'col' }, h('div', { class: 'who' }, h('b', null, n.bot), h('span', { class: 'bottag' }, 'bot')), h('div', { class: 'b' }, h('span', { class: 'typing', role: 'status', 'aria-label': `${n.bot} is writing` }, h('i'), h('i'), h('i')))),
    );
  }

  private composer(v: DeskView, n: Names, slow: boolean): HTMLElement {
    const max = v.limits.max_chars;
    const input = h('textarea', {
      id: 'd-say',
      'data-k': 'd-say',
      class: `in${slow ? ' off' : ''}`,
      rows: 1,
      placeholder: `Message ${n.bot}`,
      'aria-label': `Message to ${n.bot}`,
      'aria-describedby': 'say-hint',
      enterkeyhint: 'send',
      autocomplete: 'off',
      disabled: slow,
    }) as HTMLTextAreaElement;
    input.value = this.draft;
    const send = h('button', { type: 'submit', class: 'send', 'aria-label': 'Send', 'data-send': '' }, dIcon('send')) as HTMLButtonElement;
    const hint = h('div', { class: 'hint', id: 'say-hint', 'aria-live': 'polite' });
    const update = () => {
      const len = chars(this.draft);
      const over = len > max;
      input.classList.toggle('over', over);
      input.setAttribute('aria-invalid', over ? 'true' : 'false');
      send.disabled = slow || over || !this.draft.trim();
      send.classList.toggle('off', send.disabled);
      if (slow) hint.replaceChildren(h('span', null, slowCopy(this.slowUntil!).hint));
      else if (over) hint.replaceChildren(h('span', null, this.bold(longCopy(max))), h('span', { class: 'over' }, `${fmt(len)} / ${fmt(max)}`));
      else if (this.sayErr) hint.replaceChildren(h('span', { role: 'alert' }, this.sayErr));
      else hint.replaceChildren();
      hint.hidden = !hint.childNodes.length;
    };
    input.addEventListener('input', () => {
      this.draft = input.value;
      if (this.sayErr) this.sayErr = '';
      update();
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.send();
      }
    });
    update();
    const form = h('form', { class: 'gcomp', novalidate: true, 'data-composer': '' }, hint, h('div', { class: 'row' }, input, send));
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void this.send();
    });
    return form;
  }

  // G12 Ended (pa13b K11 with a booking)
  private closed(v: DeskView): HTMLElement {
    const n = this.n();
    const del = v.me?.delete_on;
    if (v.booking) {
      const b = v.my_booking?.state === 'confirmed' ? v.my_booking : null;
      return h(
        'div',
        { class: 'body mid', 'data-ended': '' },
        h('div', { class: 'qmwrap' }, bell(96, 'finished')),
        h('h2', null, book.ended_title(n)),
        b ? strip(b, bookingWhen(b, v.slots), n, { asking: false, busy: false, ended: true }, { cancel: () => {}, keep: () => {}, confirm: () => {} }) : null,
        h('p', { class: 'lead' }, b ? book.ended_change(n) : book.ended_none(n)),
        this.keep(b && del ? book.ended_keep(day(del)) : del ? `${this.keepTail(v, true).slice(0, -1)}, on ${day(del)}.` : this.keepTail(v, true)),
      );
    }
    const keep = del ? `${this.keepTail(v, true).slice(0, -1)}, on ${day(del)}.` : this.keepTail(v, true);
    return h(
      'div',
      { class: 'body mid', 'data-ended': '' },
      h('div', { class: 'qmwrap' }, bell(96, 'finished')),
      h('h2', null, n.anon ? 'This chat is closed' : `${n.Host} closed this chat`),
      h('p', { class: 'lead' }, `Thanks for staying at ${theTitle(v.title)}! For anything else, contact ${n.host} the way you booked.`),
      this.keep(keep),
    );
  }

  // ---------- pa13b booking ----------

  private bookingStrip(v: DeskView, n: Names): Kid {
    const b = v.my_booking;
    if (!v.booking || !b) return null;
    const el = strip(b, bookingWhen(b, v.slots), n, { asking: this.asking, busy: this.cancelBusy }, {
      cancel: () => {
        this.asking = true;
        this.stripSay = '';
        this.paint();
        this.root.querySelector<HTMLElement>('[data-cancel-keep]')?.focus();
      },
      keep: () => {
        this.asking = false;
        this.paint();
        this.root.querySelector<HTMLElement>('[data-cancel]')?.focus();
      },
      confirm: () => void this.cancelRequest(),
    });
    const key = `${b.slot}|${b.state}|${this.asking}`;
    if (this.painted && key !== this.stripKey) el.classList.add('new');
    this.stripKey = key;
    return [el, this.stripSay ? h('p', { class: 'bksay', role: 'alert', 'data-strip-say': '' }, this.stripSay) : null];
  }

  private sheet(v: DeskView): HTMLElement[] {
    const p = this.pick;
    return picker(v, p, this.n(), this.env.now(), {
      close: () => this.closePick(),
      day: (key) => {
        p.day = key;
        p.notice = null;
        this.paint();
        this.root.querySelector<HTMLElement>(`[data-day="${key}"]`)?.focus();
      },
      pick: (id) => {
        p.slot = id;
        p.notice = null;
        this.paint();
        this.root.querySelector<HTMLElement>(`[data-slot="${id}"]`)?.focus();
      },
      note: (text) => (p.note = text),
      ask: () => void this.request(),
      write: () => {
        this.closePick();
        this.root.querySelector<HTMLElement>('#d-say')?.focus();
      },
    });
  }

  /** Opens the sheet, with `slot` picked (an offer chip). `from`: the opener's data-k, for focus on close. */
  openPick(slot: string, from: string): void {
    const v = this.view;
    if (!v?.booking || !v.me || v.ended) return;
    const s = v.slots.find((x) => x.id === slot);
    this.pick = { ...closedPick(), open: true, fresh: true, slot: s ? slot : '', day: s ? s.start.slice(0, 10) : '', from };
    this.paint(true);
    this.root.querySelector<HTMLElement>('#pk-title')?.focus();
  }

  closePick(): void {
    if (!this.pick.open) return;
    const from = this.pick.from;
    this.pick = closedPick();
    this.paint(true);
    const back = (from && this.root.querySelector<HTMLElement>(`[data-k="${from}"]`)) || this.root.querySelector<HTMLElement>('[data-cancel]') || this.root.querySelector<HTMLElement>('#d-say');
    back?.focus();
  }

  private async refresh(): Promise<void> {
    const r = await this.api.state();
    if (r.ok && r.view) this.show(r.view);
  }

  /** K3 "Ask for {when}": one request. Refusals stay in the sheet, in the page's words (K10 and the copy table). */
  async request(): Promise<void> {
    const v = this.view;
    const p = this.pick;
    const slot = v?.slots.find((s) => s.id === p.slot && s.open);
    if (!v || !slot || p.busy || [...p.note].length > v.limits.note_max_chars) return;
    p.busy = true;
    p.notice = null;
    this.paint();
    const note = p.note.trim();
    const r = await this.api.act('request', note ? { slot: slot.id, note } : { slot: slot.id });
    p.busy = false;
    if (r.ok) {
      const from = p.from;
      this.pick = closedPick();
      this.stick = true;
      if (r.view) this.show(r.view);
      else await this.refresh();
      this.paint(true);
      // The opener (a starter or an offer) goes once there is a request: focus the strip's Cancel instead.
      ((from && this.root.querySelector<HTMLElement>(`[data-k="${from}"]`)) || this.root.querySelector<HTMLElement>('[data-cancel]') || this.root.querySelector<HTMLElement>('#d-say'))?.focus();
      return;
    }
    const e = r.error;
    const say = (text: string) => (p.notice = { kind: 'say', text });
    if (e === 'slot_taken') {
      // K10: the fresh list from the body (design), else ask the box; nothing stays picked.
      p.slot = '';
      p.notice = { kind: 'taken', time: timeLabel(slot) };
      if (Array.isArray(r.body?.slots) && this.view) this.view = { ...this.view, slots: slotsFrom(r.body!.slots) };
      else await this.refresh();
    } else if (e === 'has_request') {
      const b = bookingFrom(r.body?.my_booking);
      say(book.has_request(b ? bookingWhen(b, v.slots) : ''));
      if (b && this.view) this.view = { ...this.view, my_booking: b };
      await this.refresh();
    } else if (e === 'slot_past') {
      p.slot = '';
      say(book.slot_past);
      await this.refresh();
    } else if (e === 'no_such_slot' || r.status === 404) {
      p.slot = '';
      say(book.no_slot(this.n()));
      await this.refresh();
    } else if (e === 'too_long' || r.status === 413) say(book.note_long(v.limits.note_max_chars));
    else if (e === 'slow_down' || r.status === 429) say(book.slow(r.retry_at ? clock(r.retry_at) : ''));
    else if (e === 'ended' || r.status === 410) {
      const b = bookingFrom(r.body?.my_booking);
      if (this.view) this.view = { ...this.view, my_booking: b ?? this.view.my_booking };
      return this.ended();
    } else if (r.status === 401 || e === 'signed_out') return this.onGone(401, 'signed_out');
    else if (r.status === 403) return this.onGone(403, e);
    else say(book.failed);
    this.paint(true);
    this.root.querySelector<HTMLElement>('[data-taken], [data-pick-say]')?.focus?.();
  }

  /** K7 "Cancel my request": frees a requested time. A booked one is never offered (409 is_confirmed refreshes). */
  async cancelRequest(): Promise<void> {
    const b = this.view?.my_booking;
    if (!b || b.state !== 'requested' || this.cancelBusy) return;
    this.cancelBusy = true;
    this.stripSay = '';
    this.paint();
    const r = await this.api.act('cancel', { slot: b.slot });
    this.cancelBusy = false;
    this.asking = false;
    if (r.ok) {
      this.stick = true;
      if (r.view) this.show(r.view);
      else await this.refresh();
      this.paint(true);
      (this.root.querySelector<HTMLElement>('[data-pick-open]') || this.root.querySelector<HTMLElement>('#d-say'))?.focus();
      return;
    }
    if (r.error === 'ended' || r.status === 410) return this.ended();
    if (r.status === 401 || r.error === 'signed_out') return this.onGone(401, 'signed_out');
    if (r.error === 'is_confirmed' || r.error === 'no_request') await this.refresh();
    else this.stripSay = book.failed;
    this.paint(true);
  }

  private awayLine(): string {
    const a = this.away;
    if (!a) return 'Trying again by itself.';
    return `Trying again in ${a.left} s.`;
  }

  private awayScreen(): HTMLElement {
    const n = this.n();
    const btn = h('button', { type: 'button', class: 'gbtn ghost', 'data-try': '' }, 'Try now');
    btn.addEventListener('click', this.tryNow);
    return h(
      'div',
      { class: 'body mid', role: 'status' },
      h('div', { class: 'qmwrap' }, bell(96, 'thinking')),
      h('h2', null, `${n.hosts.charAt(0).toUpperCase()}${n.hosts.slice(1)} computer is asleep`),
      h('p', { class: 'lead' }, `This page lives on ${n.hosts} computer, and it's off or asleep right now. The page tries again by itself.`),
      h('p', { class: 'small', 'data-away': '' }, this.awayLine()),
      btn,
    );
  }
}
