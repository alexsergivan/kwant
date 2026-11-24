// The Queue ticket's guest page (pa11b), states 1-12 in queue-design.md section 2, on Vega's queue API
// (pa11/queue-impl.md, which wins over the design). Loaded as its own chunk when the pocket's kit is `queue`; the other
// kits' screens are untouched. Plain DOM, no framework; text goes in as text only. The page only ever holds this
// phone's own ticket: the box sends no one else's, and no name is ever drawn.
import { Api, Env, Fail, Live } from '../api';
import { clock } from '../app';
import { h, Kid } from '../dom';
import { Buzz, BuzzWin, useBuzz } from './buzz';
import {
  backLine, barRight, BUZZ_HEAD, BUZZ_HINT, closeQuote, eta, facts, GIVE_UP, GONE, HONEST, HONEST_IOS, hostOf, keepLine, LEAVE,
  leaveConfirm, leftLabel, Line, mss, nameLabel, nextLines, NO_VIBRATE, OFFLINE, ordinal, pausedNote, people, refusalCopy, Refusal,
  skippedLines, tabTitle, turnLine, turnNote, wouldBe,
} from './copy';
import { favicon, qIcon, QIcon, ticketCreature, Tint } from './look';
import { live, Phase, phaseOf, queueFrom, QueueView, Ticket } from './view';

/** Before a ticket there is no socket: the public facts are read again every 10 s (a pause or close shows up). */
const INFO_MS = 10_000;
/** A 429 keeps Take a ticket off this long, then lets the guest try again. */
const SLOW_MS = 60_000;
export const NAME_MAX = 24;

export type QueueEnv = Env & {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
  /** prefers-reduced-motion: the tab title stops swapping. */
  still?: () => boolean;
  /** The window, for vibrate, audio and the wake lock (tests hand in their own). */
  buzz?: BuzzWin;
};

type Shown = Phase | 'loading' | 'away' | 'missing';

export class QueueApp {
  view: QueueView | null = null;
  readonly api: Api<QueueView>;
  readonly buzz: Buzz;
  private live: Live<QueueView>;
  private liveOn = false;
  private root: HTMLElement;
  private said: HTMLElement;
  private sig = '';
  private away: { secs: number; tries: number } | null = null;
  private gone = false;
  private infoTimer = 0;
  private slowTimer = 0;
  private tick = 0;
  private blink = false;
  /** Server clock minus this phone's, from the view's `now`: the countdown runs on the box's time. */
  private offset = 0;
  private last: Phase | undefined;
  /** The reopened page shows "Welcome back" until the ticket moves on. */
  private welcome = false;
  private refusal: { r: Refusal | 'offline'; message?: string } | null = null;
  private busy = '';
  private confirm = false;
  /** The first name being typed (names on only): kept across redraws. */
  name = '';

  constructor(root: HTMLElement, private env: QueueEnv, readonly code: string) {
    this.root = root;
    this.api = new Api<QueueView>(env, code, queueFrom);
    this.live = new Live<QueueView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => {
        this.away = { secs, tries };
        this.paint();
      },
      back: () => {
        this.away = null;
        this.paint();
      },
      gone: (status, error) => void this.onGone(status, error),
    });
    this.buzz = useBuzz(env.buzz ?? ({ navigator: {} } as BuzzWin));
    this.buzz.onChange = () => this.paint();
    this.said = h('p', { class: 'vh', 'aria-live': 'polite' });
  }

  async start(boot?: unknown): Promise<void> {
    const b = boot ? this.api.take(boot) : undefined;
    if (b) this.show(b);
    else this.paint();
    const r = await this.api.state();
    if (r.ok && r.view) {
      this.welcome = live(phaseOf(r.view));
      this.last = undefined;
      this.show(r.view);
      this.goLive();
    } else if (!r.ok && (r.status === 0 || r.status >= 500)) this.goLive();
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
    this.liveOn = false;
    this.env.clearTimeout(this.infoTimer);
    this.env.clearTimeout(this.slowTimer);
    this.env.clearInterval(this.tick);
    this.buzz.release();
  }

  /** The phone woke or came back online: ask the box again. */
  wake(): void {
    if (this.gone) return;
    if (this.liveOn) this.live.now();
    else void this.refresh();
  }

  private goLive(): void {
    if (this.liveOn || this.gone) return;
    this.liveOn = true;
    this.env.clearTimeout(this.infoTimer);
    this.live.start();
  }

  // ---------- incoming ----------

  show(v: QueueView): void {
    if (this.gone) return;
    this.view = v;
    if (v.now) {
      const at = Date.parse(v.now);
      if (!Number.isNaN(at)) this.offset = at - this.env.now();
    }
    const p = phaseOf(v);
    if (this.last !== undefined && p !== this.last) {
      // One cue per state change, from the view, never from a timer.
      if (p === 'next') this.buzz.cue('next');
      if (p === 'turn') {
        this.buzz.cue('turn');
        this.said.textContent = `Your turn! Number #${v.my_ticket!.number}.`;
      }
      this.welcome = false;
    }
    if (p !== this.last) this.confirm = false;
    this.last = p;
    if (!live(p)) this.buzz.release();
    // The view now says what a refusal said (paused, closed, full); only these stay until they run out.
    if (this.refusal && !['slow', 'slow_net', 'name'].includes(this.refusal.r)) this.refusal = null;
    if (live(p) && this.refusal) this.refusal = null;
    if (v.state === 'ended') this.endLive();
    this.paint();
  }

  private async refresh(): Promise<void> {
    const r = this.view?.joined || this.liveOn ? await this.api.state() : await this.api.info();
    if (r.ok && r.view) this.show(r.view);
    else if (!r.ok && r.status > 0 && r.status < 500) await this.onGone(r.status, r.error);
  }

  private async onGone(status: number, error?: string): Promise<void> {
    if (status === 401 || error === 'signed_out') {
      // The finished ticket's 2 minutes are up (or the cookie went): back to the public facts and Take a ticket.
      this.live.stop();
      this.liveOn = false;
      this.away = null;
      const i = await this.api.info();
      if (i.ok && i.view) {
        this.show(i.view);
        this.watchInfo();
      } else if (!i.ok && i.status !== 401 && i.status > 0 && i.status < 500) await this.onGone(i.status, i.error);
      return;
    }
    if (status === 410 || error === 'ended') {
      this.endLive();
      if (this.view) return this.show({ ...this.view, state: 'ended' });
    }
    this.endLive();
    this.gone = true;
    this.paint();
  }

  private endLive(): void {
    this.live.stop();
    this.liveOn = false;
    this.away = null;
    this.env.clearTimeout(this.infoTimer);
  }

  private watchInfo(): void {
    this.env.clearTimeout(this.infoTimer);
    this.infoTimer = this.env.setTimeout(async () => {
      if (this.liveOn || this.gone) return;
      const i = await this.api.info();
      if (this.liveOn || this.gone) return;
      if (i.ok && i.view) this.show({ ...i.view, joined: this.view?.joined ?? false });
      else if (!i.ok && i.status > 0 && i.status < 500) return void this.onGone(i.status, i.error);
      if (this.view?.state !== 'ended') this.watchInfo();
    }, INFO_MS);
  }

  // ---------- actions ----------

  /** Take a ticket: a key first (join {}, no name), then `take` (with the first name only when the host asks for one). */
  take = async (): Promise<void> => {
    const v = this.view;
    if (!v || this.busy || this.blocked()) return;
    this.busy = 'take';
    this.refusal = null;
    this.paint();
    if (!v.joined) {
      const j = await this.api.call('join', {});
      if (!j.ok) return this.refused(j);
    }
    const name = v.ask_name ? this.name.trim() : '';
    const r = await this.api.act('take', name ? { name } : {});
    if (!r.ok) return this.refused(r);
    this.busy = '';
    this.name = '';
    if (r.view) this.show(r.view);
    this.goLive();
  };

  private async refused(r: Fail): Promise<void> {
    this.busy = '';
    if (r.status === 0 || r.status >= 500) this.refusal = { r: 'offline' };
    else if (r.status === 401 || r.status === 403 || r.status === 404 || r.error === 'ended') return this.onGone(r.status, r.error);
    else if (r.status === 429 || r.error === 'slow_down') {
      this.refusal = { r: r.limit === 'network' ? 'slow_net' : 'slow' };
      this.env.clearTimeout(this.slowTimer);
      this.slowTimer = this.env.setTimeout(() => {
        if (this.refusal?.r === 'slow' || this.refusal?.r === 'slow_net') this.refusal = null;
        this.paint();
      }, SLOW_MS);
    } else if (r.error === 'bad_action' || r.status === 400) this.refusal = { r: 'name', message: r.message };
    else if (r.error === 'closed') this.refusal = { r: 'closed' };
    else if (r.error === 'paused') this.refusal = { r: 'paused' };
    else if (r.error === 'full') this.refusal = { r: 'full' };
    // has_ticket: no message, the page shows that ticket (the next state has it).
    this.paint();
    if (this.refusal?.r !== 'offline') {
      const keep = this.refusal;
      await this.refresh();
      if (keep && ['slow', 'slow_net', 'name'].includes(keep.r)) this.refusal = keep;
      this.paint(true);
    }
    if (this.view?.joined) this.goLive();
  }

  /** Leave the queue (or give up a called turn): the inline confirm's Leave. */
  leave = async (): Promise<void> => {
    if (this.busy) return;
    this.busy = 'leave';
    this.paint();
    const r = await this.api.act('leave', {});
    this.busy = '';
    this.confirm = false;
    if (r.ok) {
      if (r.view) this.show(r.view);
      else this.paint();
    } else if (r.status === 0 || r.status >= 500) {
      this.refusal = { r: 'offline' };
      this.paint();
    } else await this.onGone(r.status, r.error);
  };

  private ask = (): void => {
    this.confirm = true;
    this.paint();
    this.root.querySelector<HTMLElement>('[data-stay]')?.focus();
  };

  private stay = (): void => {
    this.confirm = false;
    this.paint();
    this.root.querySelector<HTMLElement>('[data-leave-link]')?.focus();
  };

  /** Take a ticket is off: a refusal, or the view already says paused, closed or full. */
  private blocked(): Refusal | null {
    const v = this.view;
    const rr = this.refusal?.r;
    if (rr && rr !== 'name' && rr !== 'offline') return rr;
    if (!v) return null;
    if (v.state === 'paused') return 'paused';
    if (v.state === 'closed') return 'closed';
    if (v.max_waiting && v.waiting >= v.max_waiting) return 'full';
    return null;
  }

  // ---------- drawing ----------

  screenNow(): Shown {
    if (this.gone) return 'missing';
    const v = this.view;
    if (!v) return this.away ? 'away' : 'loading';
    return phaseOf(v);
  }

  /** Draws when something shown changed. Typing never redraws; focus and caret survive. */
  paint(force = false): void {
    const scr = this.screenNow();
    const sig = JSON.stringify([scr, this.view, this.refusal, this.busy, this.confirm, this.away?.tries, this.welcome, this.buzz.sound, this.buzz.wakeOn]);
    if (force || sig !== this.sig) {
      this.sig = sig;
      const active = document.activeElement as HTMLInputElement | null;
      const k = active?.getAttribute?.('data-k');
      const sel = k && typeof active?.selectionStart === 'number' ? [active.selectionStart, active.selectionEnd ?? active.selectionStart] : null;
      const page = h('main', { class: `g qg s-${scr}`, 'data-screen': scr }, this.bar(), this.body(scr), this.madeWith());
      this.root.replaceChildren(page, this.said);
      if (k) {
        const el = this.root.querySelector<HTMLInputElement>(`[data-k="${k}"]`);
        if (el && !el.disabled) {
          el.focus({ preventScroll: true });
          if (sel) el.setSelectionRange?.(sel[0], sel[1]);
        }
      }
    }
    this.clock(scr);
    this.tab(scr);
  }

  /** While called: the countdown and bar once a second, and the tab title swap (not under reduced motion). */
  private clock(scr: Shown): void {
    this.env.clearInterval(this.tick);
    if (scr !== 'turn') return;
    this.count();
    this.tick = this.env.setInterval(() => {
      this.count();
      this.blink = !this.blink && !this.env.still?.();
      this.tab('turn');
    }, 1000);
  }

  /** Seconds left of the call, on the box's clock. */
  secsLeft(): number {
    const end = Date.parse(this.view?.my_ticket?.call_ends_at ?? '');
    return Number.isNaN(end) ? 0 : Math.max(0, (end - (this.env.now() + this.offset)) / 1000);
  }

  private count(): void {
    const cd = this.root.querySelector<HTMLElement>('[data-cd]');
    if (!cd || !this.view) return;
    const s = this.secsLeft();
    cd.setAttribute('aria-label', leftLabel(s));
    const b = cd.querySelector('b');
    if (b) b.textContent = mss(s);
    const bar = cd.querySelector<HTMLElement>('i');
    bar?.style.setProperty('--p', String(Math.min(1, s / (this.view.call_minutes * 60))));
  }

  private tab(scr: Shown): void {
    const v = this.view;
    if (!v) return;
    const p = phaseOf(v);
    if (scr !== 'turn') this.blink = false;
    document.title = tabTitle(p, v, this.blink && p === 'turn');
    const tint: Tint | null = p === 'turn' ? 'bright' : p === 'next' ? 'violet' : v.my_ticket ? 'grey' : null;
    let link = document.querySelector<HTMLLinkElement>('link[data-q]');
    if (!tint) return void link?.remove();
    if (!link) {
      link = h('link', { rel: 'icon', type: 'image/svg+xml', 'data-q': '' }) as HTMLLinkElement;
      document.head.append(link);
    }
    const cs = getComputedStyle(document.documentElement);
    const href = favicon(tint, (t) => cs.getPropertyValue(t));
    if (link.getAttribute('href') !== href) link.setAttribute('href', href);
  }

  private bar(): HTMLElement {
    const v = this.view;
    return h('header', { class: 'bar' }, h('span', { class: 'qtile', 'aria-hidden': 'true' }, qIcon('ticket')), h('b', null, v?.title ?? ''), h('span', { class: 'sp' }), h('span', { class: 'muted' }, v ? barRight(v) : ''));
  }

  private madeWith(): HTMLElement {
    return h('footer', { class: 'madewith' }, 'made with ', h('a', { href: 'https://kwant.si', rel: 'noopener noreferrer', target: '_blank' }, 'kwant'));
  }

  private bold(l: Line): Kid[] {
    return [h('b', null, l[0]), l[1] ? ` ${l[1]}` : null];
  }

  private note(icon: QIcon, l: Line, cls = '', role: string | null = 'status'): HTMLElement {
    return h('div', { class: `gnote${cls ? ` ${cls}` : ''}`, role }, qIcon(icon), h('span', null, this.bold(l)));
  }

  private mid(icon: QIcon, title: string, lead: string, ...more: Kid[]): HTMLElement {
    return h('div', { class: 'body mid' }, h('div', { class: 'qbigicon', 'aria-hidden': 'true' }, qIcon(icon)), h('h2', null, title), lead ? h('p', { class: 'lead' }, lead) : null, ...more);
  }

  private body(scr: Shown): HTMLElement {
    const v = this.view!;
    switch (scr) {
      case 'loading':
        return this.mid('ticket', 'Opening…', '');
      case 'away': {
        const btn = h('button', { type: 'button', class: 'gbtn ghost', onclick: () => this.wake() }, 'Try now');
        return this.mid('nowifi', 'No connection', OFFLINE[1].replace(/^y/, 'Y'), btn);
      }
      case 'missing':
      case 'ended':
        return this.mid('ticket', GONE, v?.title ? `Thanks for waiting at ${v.title}.` : '');
      case 'take':
        return this.takeScreen(v);
      default:
        return this.ticketScreen(v, scr);
    }
  }

  private offline(): Kid {
    return this.away || this.refusal?.r === 'offline' ? this.note('nowifi', OFFLINE, 'warn') : null;
  }

  private takeBtn(label: string): Kid[] {
    const off = this.blocked();
    const btn = h(
      'button',
      { type: 'button', class: 'gbtn', 'data-take': '', disabled: !!off || !!this.busy, 'aria-busy': this.busy === 'take' ? 'true' : null, onclick: this.take },
      qIcon('ticket'),
      this.busy === 'take' ? 'Taking a ticket…' : label,
    );
    const rr = this.refusal?.r;
    const r: Refusal | null = rr && rr !== 'offline' && rr !== 'name' ? rr : off;
    return [r ? this.note(r === 'paused' ? 'pause' : r === 'closed' ? 'clock' : 'warn', refusalCopy(r, this.view!), r === 'paused' ? 'warn' : '', 'alert') : null, btn];
  }

  private takeScreen(v: QueueView): HTMLElement {
    const off = this.blocked();
    const nameErr = this.refusal?.r === 'name' ? refusalCopy('name', v, this.refusal.message)[0] : '';
    const field = v.ask_name
      ? h(
          'div',
          null,
          h('label', { class: 'lbl', for: 'qname' }, nameLabel(v)),
          h('input', {
            id: 'qname',
            class: 'input',
            'data-k': 'name',
            type: 'text',
            maxlength: NAME_MAX,
            autocomplete: 'given-name',
            enterkeyhint: 'go',
            value: this.name,
            'aria-invalid': nameErr ? 'true' : null,
            'aria-describedby': nameErr ? 'qname-err' : null,
            disabled: !!off,
            oninput: (e: Event) => {
              this.name = (e.target as HTMLInputElement).value;
            },
            onkeydown: (e: Event) => {
              if ((e as KeyboardEvent).key === 'Enter') void this.take();
            },
          }),
          nameErr ? h('p', { class: 'err', id: 'qname-err', role: 'alert' }, nameErr) : null,
        )
      : null;
    const wb = wouldBe(v);
    return h(
      'div',
      { class: 'body' },
      this.keep(keepLine(v)),
      h('div', null, h('h1', null, v.title), v.line ? h('p', { class: 'soft qline' }, v.line) : null),
      h('dl', { class: 'facts' }, facts(v).map(([k, val]) => h('div', { class: 'fact' }, h('dt', null, h('small', null, k)), h('dd', null, h('b', null, val))))),
      this.offline(),
      field,
      this.takeBtn('Take a ticket'),
      off ? null : h('p', { class: 'ginfo qwould' }, qIcon('clock'), h('span', null, "You'd be ", h('b', null, wb[0]), wb[1])),
      h('div', { class: 'honest solo' }, qIcon('phone'), h('span', null, BUZZ_HINT)),
    );
  }

  private keep(text: string): HTMLElement {
    return h('div', { class: 'keep', 'data-keep': '' }, qIcon('lock'), h('span', null, text));
  }

  /** The ticket: stub (creature, number, a side fact), perforation, then the main part. */
  ticket(t: Ticket, kind: 'waiting' | 'next' | 'turn' | 'dim', side: [string, string], ...main: Kid[]): HTMLElement {
    return h(
      'div',
      { class: `tk ${kind}`, role: 'group', 'aria-label': `Your ticket, number ${t.number}`, 'data-ticket': kind },
      h(
        'div',
        { class: 'stub' },
        h('span', { class: 'c' }, ticketCreature(t.number)),
        h('div', { class: 't' }, h('small', null, 'Your ticket'), h('b', null, `#${t.number}`)),
        h('div', { class: 'tside' }, side[0], side[1] ? h('b', null, side[1]) : null),
      ),
      h('div', { class: 'perf', 'aria-hidden': 'true' }),
      h('div', { class: 'tmain' }, ...main),
    );
  }

  private ticketScreen(v: QueueView, p: Phase): HTMLElement {
    const t = v.my_ticket!;
    const serving: [string, string] = ['Now serving', v.now_serving ? `#${v.now_serving}` : '—'];
    const at = (iso?: string): string => (iso ? clock(iso) : '');
    const kids: Kid[] = [this.offline()];
    if (this.welcome && live(p)) kids.push(this.note('check', [backLine(t), ''], 'ok'));
    const paused = v.state === 'paused' && p === 'waiting';
    if (paused) kids.push(this.note('pause', pausedNote(v), 'warn'));
    const dim = this.confirm ? 'dim' : null;
    switch (p) {
      case 'waiting': {
        const e = paused ? null : eta(t, v);
        const since = paused && v.paused_at ? `Paused since ${clock(v.paused_at)}` : paused ? 'Paused' : '';
        kids.push(
          this.ticket(
            t,
            dim ?? 'waiting',
            serving,
            h('div', { class: 'pre' }, paused ? "You're still" : "You're"),
            h('div', { class: 'place' }, t.place ? ordinal(t.place) : '—'),
            t.ahead == null ? null : t.ahead === 0 ? h('div', { class: 'post' }, 'Nobody is ahead of you.') : h('div', { class: 'post' }, h('b', null, people(t.ahead)), ' ahead of you'),
            since ? h('div', { class: 'eta none' }, qIcon('clock'), since) : e ? h('div', { class: 'eta' }, qIcon('clock'), e) : null,
          ),
        );
        if (!this.confirm) kids.push(this.buzzPanel());
        kids.push(this.leaveBit(t, false));
        break;
      }
      case 'next': {
        const [go, again] = nextLines(v);
        kids.push(this.ticket(t, dim ?? 'next', serving, h('div', { class: 'word' }, "You're next"), h('div', { class: 'post' }, go, h('br'), again)));
        if (!this.confirm) kids.push(this.note('phone', this.onLine(), ''));
        kids.push(this.leaveBit(t, false));
        break;
      }
      case 'turn': {
        const s = this.secsLeft();
        kids.push(
          this.ticket(
            t,
            dim ?? 'turn',
            ['Called', at(t.called_at)],
            h('div', { class: 'word' }, 'Your turn!'),
            h('div', { class: 'post' }, turnLine(v)),
            h(
              'div',
              { class: 'cd', role: 'timer', 'aria-label': leftLabel(s), 'data-cd': '' },
              h('div', { class: 'row' }, h('span', null, 'Come within'), h('b', null, mss(s))),
              h('div', { class: 'cdbar', 'aria-hidden': 'true' }, h('i')),
            ),
          ),
        );
        kids.push(this.note('clock', [turnNote(v), ''], '', null));
        kids.push(this.leaveBit(t, true));
        break;
      }
      case 'skipped': {
        const sk = skippedLines(t, v);
        kids.push(this.ticket(t, 'dim', ['Called', at(t.called_at)], h('div', { class: 'word' }, 'You missed your turn'), h('div', { class: 'post' }, sk.lead)));
        kids.push(...this.takeBtn('Take a new ticket'));
        if (!this.blocked()) kids.push(h('p', { class: 'ginfo' }, qIcon('clock'), h('span', null, sk.after)));
        break;
      }
      case 'served':
        kids.push(
          this.ticket(
            t,
            'dim',
            ['Served', at(t.done_at)],
            h('div', { class: 'gstat' }, h('span', { class: 'gchip ok' }, qIcon('check'), t.done_at ? `Served at ${clock(t.done_at)}` : 'Served')),
            h('div', { class: 'word' }, 'Thanks for waiting!'),
            h('div', { class: 'post' }, 'Your ticket is deleted now.'),
          ),
        );
        break;
      case 'left':
        kids.push(this.ticket(t, 'dim', ['Left', at(t.done_at)], h('div', { class: 'word' }, 'You left the queue.')));
        kids.push(...this.takeBtn('Take a new ticket'));
        break;
      case 'closed':
        kids.unshift(h('div', { class: 'gnote', role: 'status' }, qIcon('clock'), h('span', null, h('b', null, closeQuote(v)), h('br'), v.title)));
        kids.push(
          this.ticket(
            t,
            'dim',
            ['Closed', at(t.done_at)],
            h('div', { class: 'post' }, `Number #${t.number} wasn't called before closing.`),
            h('div', { class: 'eta none' }, qIcon('lock'), 'Your ticket is deleted'),
          ),
        );
        break;
    }
    return h('div', { class: 'body' }, ...kids);
  }

  /** "Keep this page open." and what is on, when you're next. */
  private onLine(): Line {
    const b = this.buzz;
    const on = [b.sound ? 'Sound' : '', b.can.vibrate ? 'vibration' : ''].filter(Boolean);
    const rest = on.length === 2 ? 'Sound and vibration are on.' : on.length ? `${on[0].charAt(0).toUpperCase()}${on[0].slice(1)} is on.` : "The tab title says “Your turn” when it's time.";
    return ['Keep this page open.', rest];
  }

  private buzzPanel(): HTMLElement {
    const b = this.buzz;
    const row = (icon: QIcon, label: string, end: Kid): HTMLElement => h('div', { class: 'br' }, qIcon(icon), h('span', null, label), end);
    const on = h('span', { class: 'st ok' }, 'On');
    const vib = b.can.vibrate ? row('buzz', 'Vibrate', on.cloneNode(true) as HTMLElement) : b.can.ios ? row('buzz', 'Vibrate', h('span', { class: 'st no' }, NO_VIBRATE)) : null;
    const chime = b.can.chime
      ? row('speaker', 'Chime', b.sound ? on.cloneNode(true) : h('button', { type: 'button', class: 'gb44', 'data-sound': '', onclick: () => b.turnOnSound() }, 'Turn on sound'))
      : null;
    const scr = b.can.wake
      ? row('screen', 'Keep screen on', b.wakeOn ? on.cloneNode(true) : h('button', { type: 'button', class: 'gb44 ghost', 'data-wake': '', onclick: () => void b.keepScreenOn() }, 'Turn on'))
      : null;
    const tabRow = row('tab', 'The tab title says “Your turn”', on.cloneNode(true));
    const honest = b.can.ios ? HONEST_IOS : HONEST;
    // iPhone: screen and sound first, the vibrate row last (it can't).
    const rows = b.can.ios ? [chime, scr, tabRow, vib] : [vib, chime, scr, tabRow];
    return h(
      'div',
      { class: 'buzz', role: 'group', 'aria-label': "How this page tells you it's your turn" },
      h('div', { class: 'bh' }, qIcon('bell'), BUZZ_HEAD),
      ...rows,
      h('div', { class: 'honest' }, qIcon('phone'), h('span', null, this.bold(honest))),
    );
  }

  /** Leave the queue (a link) or, when called, the give-up button; either opens the inline confirm. */
  private leaveBit(t: Ticket, called: boolean): HTMLElement {
    if (this.confirm) {
      const [lead, rest] = leaveConfirm(t);
      const box = h(
        'div',
        { class: 'gconfirm', role: 'alertdialog', 'aria-labelledby': 'qleave-h', 'aria-describedby': 'qleave-d' },
        h('p', null, h('b', { id: 'qleave-h' }, lead), ' ', h('span', { id: 'qleave-d' }, rest)),
        h(
          'div',
          { class: 'two' },
          h('button', { type: 'button', 'data-stay': '', onclick: this.stay }, 'Stay in line'),
          h('button', { type: 'button', class: 'stop', 'data-leave': '', disabled: this.busy === 'leave', onclick: this.leave }, 'Leave'),
        ),
      );
      box.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') this.stay();
      });
      return box;
    }
    return called
      ? h('button', { type: 'button', class: 'gbtn ghost', 'data-leave-link': '', onclick: this.ask }, GIVE_UP)
      : h('button', { type: 'button', class: 'glink', 'data-leave-link': '', onclick: this.ask }, LEAVE);
  }
}

export { hostOf };
