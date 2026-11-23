// The guest page: every phone screen in pa0a/design.md sections 2 and 3. Plain DOM, no framework.
import { Api, Env, Live } from './api';
import { AVATARS, add, creature, fmt, h, icon, Kid, quizmaster } from './dom';
import { CREATURE_NAMES } from './art';
import type { Answer, BoardRow, GuestView, Question, Reveal } from './view';

const TOAST_MS = 4000;

/** Plain-words date: "Fri, Dec 4". Dates are calendar days (YYYY-MM-DD), so they are read in UTC. */
export function day(iso: string): string {
  const d = new Date(`${iso.slice(0, 10)}T12:00:00Z`);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric', timeZone: 'UTC' });
}

/** "8:00 PM" in the phone's own time zone. */
export function clock(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
}

const noun = (v: GuestView): string => v.noun || 'quiz';

/** The privacy line, from the kit's retention in `state`. Shown before anyone types. */
export function privacyLine(v: GuestView): string {
  const r = v.retention;
  const n = noun(v);
  if (v.deletes_on) return `Your name and answers are deleted on ${day(v.deletes_on)}.`;
  if (!r) return `Your name and answers are deleted after the ${n}.`;
  if (r.kind === 'at_end') return `Your name and answers are deleted when the ${n} ends.`;
  if (r.kind === 'after_date' && r.date) return `Your name and answers are deleted on ${day(r.date)}.`;
  const d = r.days ?? 7;
  return `Your name and answers are deleted ${d} day${d === 1 ? '' : 's'} after the ${n}.`;
}

export function answerText(q: Question, a: Answer | undefined): string {
  if (!a) return '';
  if ('choice' in a) return (q.options ?? [])[a.choice.charCodeAt(0) - 65] ?? a.choice;
  if ('text' in a) return a.text;
  return fmt(a.number);
}

const LETTERS = 'ABCDEFGH';

type Draft = { name: string; avatar: string; team: string; newTeam: string; making: boolean; error: string; busy: boolean; mine: string };

export class GuestApp {
  view: GuestView | null = null;
  private api: Api;
  private live: Live;
  private root: HTMLElement;
  private key = '';
  private away: { secs: number; tries: number; left: number } | null = null;
  private awayTick = 0;
  private tick = 0;
  private toastTimer = 0;
  private toast: HTMLElement | null = null;
  /** Per question: the answer this phone sent first. First tap counts, later taps are ignored. */
  private sent = new Map<string, Answer>();
  /** Questions whose answer the box has taken (an ok or 409 reply, or a state with `answer`). Until then it is pending. */
  private confirmed = new Set<string>();
  private inflight = new Set<string>();
  /** Questions whose answer failed 6 times in a row: the page says "Couldn't send. Trying again." and keeps trying. */
  private failing = new Set<string>();
  private resendTimer = 0;
  private textDraft = '';
  private numDraft: number | null = null;
  private preloaded = new Set<string>();
  private preloading: HTMLImageElement[] = [];
  private draft: Draft = { name: '', avatar: AVATARS[0], team: '', newTeam: '', making: false, error: '', busy: false, mine: '' };
  private gone: 'removed' | 'missing' | null = null;
  /** Local clock reading when the current timer value arrived. */
  private stamp = 0;
  /** Before joining there is no socket (pa1's ws needs a seat): the join screen re-reads `info` every 10 s. */
  private infoTimer = 0;

  constructor(root: HTMLElement, private env: Env & { now: () => number; setInterval: (f: () => void, ms: number) => number; clearInterval: (id: number) => void }, code: string) {
    this.root = root;
    this.api = new Api(env, code);
    this.live = new Live(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => this.goAway(secs, tries),
      back: () => this.comeBack(),
      gone: (status, error) => void this.onGone(status, error),
    });
  }

  /**
   * Boot (Pocket API v1): the server may put this guest's state in the first HTML; then `state` with the kp_<code>
   * cookie brings back a seat this phone already has (with the "You're back" toast). 401 means no seat yet, so the
   * public `info` draws the join screen; 403 removed, 410 ended and 404 end the page.
   */
  async start(boot?: unknown): Promise<void> {
    const b = boot ? this.api.take(boot) : undefined;
    if (b) {
      // The box put a state in the page: draw it at once. A seated one needs no first round trip.
      this.show(b);
      if (b.me) {
        if (b.phase !== 'lobby' && b.phase !== 'join') this.sayBack(b);
        this.live.start();
        return;
      }
    }
    // The seat cookie is scoped to /api/pocket/<code>, so only `state` can tell whether this phone has one.
    const r = await this.api.state();
    if (r.ok && r.view) {
      this.show(r.view);
      if (r.view.me && r.view.phase !== 'lobby' && r.view.phase !== 'join') this.sayBack(r.view);
      this.live.start();
    } else if (!r.ok && (r.status === 0 || r.status >= 500)) this.live.start();
    else if (!r.ok && r.status === 401 && b) this.watchInfo();
    else if (!r.ok) await this.onGone(r.status, r.error);
  }

  stop(): void {
    this.live.stop();
    this.env.clearInterval(this.tick);
    this.env.clearInterval(this.awayTick);
    this.env.clearTimeout(this.toastTimer);
    this.env.clearTimeout(this.infoTimer);
    this.env.clearTimeout(this.resendTimer);
  }

  tryNow = (): void => this.live.now();

  /** The phone woke, came back online or restored the page: a socket may have died quietly, so ask the box now. */
  wake(): void {
    if (this.gone || !this.view?.me) return;
    this.live.now();
  }

  // ---------- incoming ----------

  show(v: GuestView): void {
    if (this.gone) return;
    this.view = v;
    if (v.phase === 'removed') this.gone = 'removed';
    const q = v.question;
    if (q?.answer) {
      this.sent.set(q.id, q.answer);
      this.confirmed.add(q.id);
      this.failing.delete(q.id);
    }
    this.stamp = this.env.now();
    if (v.phase === 'lobby' || v.phase === 'between') this.preload(v.preload ?? []);
    const key = this.keyOf(v);
    if (key === this.key && !this.away) this.patch(v);
    else {
      this.key = key;
      this.draw();
    }
    this.resend(v);
  }

  /** The box is answering again but this question has no answer from this phone: send the pending one again. */
  private resend(v: GuestView): void {
    const q = v.question;
    if (this.away || v.phase !== 'question' || !q || q.answer) return;
    const a = this.sent.get(q.id);
    if (!a || this.confirmed.has(q.id) || this.inflight.has(q.id)) return;
    this.env.clearTimeout(this.resendTimer);
    void this.send(q, a, this.failing.has(q.id) ? 5 : 0);
  }

  private keyOf(v: GuestView): string {
    const q = v.question;
    const answered = q?.answer ? 'L' : q && this.sent.has(q.id) ? 'a' : '';
    const r = v.reveal;
    return [v.phase, q?.id ?? '', answered, r ? `${r.question.id}${r.outcome}` : '', v.between?.after_round ?? '', v.me ? 'me' : ''].join('|');
  }

  /** A refusal from state, a poll or the socket. 401: no seat on this phone (never joined, or the key stopped working). */
  private async onGone(status: number, error?: string): Promise<void> {
    if (status === 401 || error === 'signed_out') {
      this.live.stop();
      this.away = null;
      const i = await this.api.info();
      if (i.ok && i.view) {
        this.key = '';
        this.show(i.view);
        this.watchInfo();
      } else if (!i.ok && (i.status === 0 || i.status >= 500)) this.live.start();
      else if (!i.ok && i.status !== 401) await this.onGone(i.status, i.error);
      return;
    }
    if (status === 403 || error === 'removed') this.end('removed');
    else if (status === 410 || error === 'ended') this.end('ended');
    else if (status === 404 || error === 'not_found') this.end('missing');
  }

  private watchInfo(): void {
    this.env.clearTimeout(this.infoTimer);
    this.infoTimer = this.env.setTimeout(async () => {
      if (this.view?.me || this.gone) return;
      const i = await this.api.info();
      if (this.view?.me || this.gone) return;
      if (i.ok && i.view) this.show(i.view);
      else if (!i.ok && i.status > 0 && i.status < 500) return void this.onGone(i.status, i.error);
      if (this.view?.phase !== 'ended') this.watchInfo();
    }, 10_000);
  }

  private end(why: 'removed' | 'ended' | 'missing'): void {
    this.env.clearTimeout(this.infoTimer);
    this.live.stop();
    this.away = null;
    if (why === 'ended') {
      if (this.view) this.show({ ...this.view, phase: 'ended' });
      else this.end('missing');
      return;
    }
    this.gone = why;
    this.key = why;
    this.draw();
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
    this.key = 'away';
    this.draw();
  }

  private comeBack(): void {
    this.away = null;
    this.env.clearInterval(this.awayTick);
    this.key = '';
    if (this.view?.me) this.sayBack(this.view);
  }

  private sayBack(v: GuestView): void {
    if (!v.me) return;
    this.showToast(
      h('span', null, h('b', null, `You're back, ${v.me.name}.`), ` Same seat${v.me.team_name ? ', same team' : ''}. Your score is safe: ${fmt(v.me.score)}.`),
    );
  }

  private showToast(body: HTMLElement): void {
    this.toast?.remove();
    const t = h('div', { class: 'toast', role: 'status' }, icon('check', 'pi ok'), body);
    this.toast = t;
    document.body.append(t);
    this.env.clearTimeout(this.toastTimer);
    this.toastTimer = this.env.setTimeout(() => {
      t.classList.add('out');
      this.env.setTimeout(() => t.remove(), 200);
      if (this.toast === t) this.toast = null;
    }, TOAST_MS);
  }

  private preload(urls: string[]): void {
    // One at a time, so a picture never competes with the live game; never during a question.
    const next = urls.find((u) => !this.preloaded.has(u) && u.startsWith('/'));
    if (!next || this.preloading.length) return;
    this.preloaded.add(next);
    const img = new Image();
    this.preloading.push(img);
    const done = () => {
      this.preloading = this.preloading.filter((i) => i !== img);
      if (this.view && (this.view.phase === 'lobby' || this.view.phase === 'between')) this.preload(this.view.preload ?? []);
    };
    img.onload = done;
    img.onerror = done;
    img.src = next;
  }

  // ---------- drawing ----------

  private draw(): void {
    this.env.clearInterval(this.tick);
    const v = this.view;
    const kids: Kid[] = [];
    let scr: string;
    if (this.gone === 'removed') [scr, kids[0]] = ['removed', this.removed()];
    else if (this.gone === 'missing') [scr, kids[0]] = ['missing', this.missing()];
    else if (this.away) [scr, kids[0]] = ['away', this.awayScreen()];
    else if (!v) [scr, kids[0]] = ['loading', this.loading()];
    else {
      scr = !v.me && ['join', 'lobby', 'question'].includes(v.phase) ? 'join' : v.phase === 'join' || (v.phase === 'full' && v.me) ? 'lobby' : v.phase;
      kids[0] = this.screen(v);
    }
    const page = h('main', { class: `g s-${scr}`, 'data-screen': scr }, this.bar(scr), ...kids, this.madeWith(scr));
    this.root.replaceChildren(page);
    const focus = this.root.querySelector<HTMLElement>('[data-autofocus]');
    focus?.focus({ preventScroll: true });
    if (v?.question && (v.phase === 'question') && !this.away) this.runTimer(v.question);
    if (v?.between?.next_in_secs != null && v.phase === 'between' && !this.away) this.runCountdown(v.between.next_in_secs);
  }

  private screen(v: GuestView): HTMLElement {
    switch (v.phase) {
      case 'join':
        return v.me ? this.lobby(v) : this.join(v);
      case 'lobby':
        return v.me ? this.lobby(v) : this.join(v);
      case 'question':
        return v.me ? this.question(v, v.question!) : this.join(v);
      case 'reveal':
        return this.reveal(v, v.reveal!);
      case 'between':
        return this.between(v);
      case 'paused':
        return this.paused(v);
      case 'ended':
        return this.ended(v);
      case 'full':
        return v.me ? this.lobby(v) : this.full(v);
      default:
        return this.loading();
    }
  }

  private bar(scr: string): HTMLElement {
    const v = this.view;
    const right = h('span', { class: 'muted tab', 'data-bar': '' }, this.barRight(scr));
    return h('header', { class: 'bar' }, quizmaster(28), h('b', null, v?.title ?? ''), h('span', { class: 'sp' }), right);
  }

  private barRight(scr: string): string {
    const v = this.view;
    if (!v || scr === 'removed' || scr === 'full' || scr === 'ended' || scr === 'loading') return '';
    if (v.me && scr !== 'lobby' && scr !== 'join') return `${fmt(v.me.score)} pts`;
    if (!v.starts_at) return '';
    const t = clock(v.starts_at);
    return v.me ? `Starts ${t}` : t;
  }

  private madeWith(scr: string): HTMLElement | null {
    if (['question', 'reveal', 'between', 'paused'].includes(scr)) return null;
    return h('footer', { class: 'madewith' }, 'made with ', h('a', { href: 'https://kwant.si', rel: 'noopener noreferrer', target: '_blank' }, 'kwant'));
  }

  private loading(): HTMLElement {
    return h('div', { class: 'body mid' }, h('div', { class: 'qmwrap' }, quizmaster(96, 'thinking')), h('p', { class: 'small', role: 'status' }, 'Opening…'));
  }

  // 2a Join
  private join(v: GuestView): HTMLElement {
    const d = this.draft;
    const name = h('input', {
      id: 'name',
      class: 'input',
      type: 'text',
      maxlength: 20,
      autocomplete: 'nickname',
      autocapitalize: 'words',
      spellcheck: 'false',
      enterkeyhint: 'done',
      'aria-describedby': 'name-hint',
      required: true,
    }) as HTMLInputElement;
    name.value = d.name;
    name.addEventListener('input', () => {
      d.name = name.value;
      if (d.error) this.setError('');
    });

    const creatures = h('div', { class: 'creatures', role: 'radiogroup', 'aria-labelledby': 'lbl-creature' });
    AVATARS.forEach((a, i) => {
      const input = h('input', { type: 'radio', name: 'creature', value: a, class: 'vh', 'aria-label': CREATURE_NAMES[i] }) as HTMLInputElement;
      input.checked = d.avatar === a;
      const tile = h('label', { class: `creature${input.checked ? ' on' : ''}` }, input, creature(a, 'cr'), h('span', { class: 'tick' }, icon('check', 'pi')));
      input.addEventListener('change', () => {
        d.avatar = a;
        creatures.querySelectorAll('.creature').forEach((c) => c.classList.toggle('on', c === tile));
      });
      creatures.append(tile);
    });

    const teams = v.teams === null ? null : this.teamList(v);
    const form = h(
      'form',
      { class: 'body', novalidate: true, 'aria-labelledby': 'title' },
      h('div', { class: 'hero' }, quizmaster(64), h('div', null, h('h1', { id: 'title' }, v.title), h('div', { class: 'soft sub' }, this.subtitle(v)))),
      h('div', null, h('label', { class: 'lbl', for: 'name' }, 'Your name ', h('small', { id: 'name-hint' }, '1-20 letters')), name),
      h('div', null, h('div', { class: 'lbl', id: 'lbl-creature' }, 'Pick your creature'), creatures),
      teams,
      h(
        'div',
        { class: 'stick' },
        h('button', { class: 'gbtn', type: 'submit', 'data-join': '', 'aria-busy': d.busy ? 'true' : null }, d.busy ? 'Joining…' : `Join the ${noun(v)}`),
        h('p', { class: 'err', role: 'alert', 'data-err': '' }, d.error),
        this.mineButton(),
      ),
      h('p', { class: 'small center', 'data-privacy': '' }, privacyLine(v)),
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      void this.submitJoin();
    });
    return form;
  }

  private subtitle(v: GuestView): string {
    const parts = [v.host ? `Hosted by ${v.host}` : 'Hosted with Quizmaster'];
    if (v.rounds) parts.push(`${v.rounds} round${v.rounds === 1 ? '' : 's'}`);
    if (v.teams !== null && v.team_max) parts.push(v.team_max === 1 ? 'solo' : `teams of up to ${v.team_max}`);
    return parts.join(' · ');
  }

  private teamList(v: GuestView): HTMLElement {
    const d = this.draft;
    const list = h('div', { class: 'teams', role: 'radiogroup', 'aria-labelledby': 'lbl-team', 'data-teams': '' });
    const fill = () => {
      list.replaceChildren();
      for (const t of v.teams ?? []) {
        const full = !!t.full || t.count >= t.max;
        if (full && d.team === t.id) d.team = '';
        const input = h('input', { type: 'radio', name: 'team', value: t.id, class: 'vh', disabled: full }) as HTMLInputElement;
        input.checked = d.team === t.id && !d.making;
        const row = h(
          'label',
          { class: `team${input.checked ? ' on' : ''}${full ? ' full' : ''}` },
          input,
          h('span', { class: `sw t${t.color % 8}${t.color >= 8 ? ' stripe' : ''}` }),
          h('span', { class: 'tn' }, t.name),
          h('small', null, full ? 'Full' : `${fmt(t.count)} of ${fmt(t.max)}`),
        );
        input.addEventListener('change', () => {
          d.team = t.id;
          d.making = false;
          fill();
        });
        list.append(row);
      }
      if (d.making) {
        const inp = h('input', {
          class: 'input',
          type: 'text',
          maxlength: 24,
          placeholder: 'New team name',
          'aria-label': 'New team name',
          enterkeyhint: 'done',
          'data-autofocus': '',
        }) as HTMLInputElement;
        inp.value = d.newTeam;
        inp.addEventListener('input', () => (d.newTeam = inp.value));
        list.append(h('div', { class: 'team new on' }, icon('plus'), inp));
      } else {
        const btn = h('button', { type: 'button', class: 'team new' }, icon('plus'), 'Start a new team');
        btn.addEventListener('click', () => {
          d.making = true;
          d.team = '';
          fill();
          list.querySelector<HTMLInputElement>('input.input')?.focus();
        });
        list.append(btn);
      }
    };
    fill();
    (list as HTMLElement & { refill?: () => void }).refill = fill;
    const each = v.team_max ? `up to ${v.team_max} each` : '';
    return h('div', null, h('div', { class: 'lbl', id: 'lbl-team' }, 'Your team ', h('small', null, each)), list);
  }

  /** Shown only after name_taken: rejoins the seat with that name (v1 rejoin by name). */
  private mineButton(): HTMLElement {
    const d = this.draft;
    const b = h('button', { type: 'button', class: 'gbtn ghost', 'data-mine': '', hidden: !d.mine }, "That's me");
    b.addEventListener('click', () => void this.rejoinAs(d.mine));
    return b;
  }

  private setError(msg: string, mine = ''): void {
    this.draft.error = msg;
    this.draft.mine = mine;
    const e = this.root.querySelector('[data-err]');
    if (e) e.textContent = msg;
    const m = this.root.querySelector<HTMLButtonElement>('[data-mine]');
    if (m) m.hidden = !mine;
  }

  async submitJoin(): Promise<void> {
    const d = this.draft;
    const v = this.view;
    if (!v || d.busy) return;
    const name = d.name.trim();
    if (!name) return this.setError('Type your name first.');
    if ([...name].length > 20) return this.setError('Names are 1-20 letters.');
    const teamsOn = v.teams !== null && v.teams !== undefined;
    const newTeam = d.newTeam.trim();
    if (teamsOn && !d.team && !(d.making && newTeam)) return this.setError(d.making ? 'Give your new team a name.' : 'Pick a team, or start a new one.');
    d.busy = true;
    this.setBusy(true);
    // v1 takes the team by name: a team on the list or a new one.
    const teamName = d.making ? newTeam : v.teams?.find((t) => t.id === d.team)?.name;
    const r = await this.api.join({ name, avatar: d.avatar, ...(teamsOn && teamName ? { team: teamName } : {}) });
    d.busy = false;
    if (r.ok) {
      d.error = '';
      this.env.clearTimeout(this.infoTimer);
      if (r.view) this.show(r.view);
      this.live.start();
      return;
    }
    this.setBusy(false);
    const err = r.error;
    if (err === 'full') this.show({ ...v, phase: 'full' });
    else if (err === 'ended' || r.status === 410) void this.onGone(410, 'ended');
    else if (err === 'removed' || r.status === 403) void this.onGone(403, 'removed');
    else if (r.status === 404) void this.onGone(404, 'not_found');
    else if (err === 'team_full') this.setError('That team just filled up. Pick another one.');
    else if (err === 'name_taken') this.setError('Someone here already has that name. Add a letter, or tap "That\'s me" if it\'s you on a new phone.', name);
    else if (err === 'not_open_yet') this.setError(`This ${noun(v)} isn't open yet. Try again in a minute.`);
    else if (err === 'name_too_long') this.setError('Names are 1-20 letters.');
    else if (err === 'name_empty') this.setError('Type your name first.');
    else if (err === 'bad_team') this.setError(d.making ? "That team name can't be used here. Try another one." : 'Pick a team, or start a new one.');
    else if (err === 'slow_down' || r.status === 429) this.setError('Too many tries just now. Wait a few seconds and tap Join again.');
    else if (err === 'pin_required' || err === 'wrong_pin' || r.status === 423) this.setError('This name needs its PIN. Ask the host to help you back in.');
    else if (r.status === 400) this.setError("That name can't be used here. Try another one.");
    else if (r.status === 0 || r.status >= 500) this.setError("Couldn't reach the quiz. Check your signal and tap Join again.");
    else this.setError("That didn't work. Tap Join again.");
  }

  /** "That's me": the name is taken because it is this guest on a new phone. v1 rejoins by name. */
  async rejoinAs(name: string): Promise<void> {
    const d = this.draft;
    if (d.busy) return;
    d.busy = true;
    this.setBusy(true);
    const r = await this.api.rejoin(name);
    d.busy = false;
    if (r.ok) {
      d.error = '';
      this.env.clearTimeout(this.infoTimer);
      if (r.view) {
        this.show(r.view);
        this.sayBack(r.view);
      }
      this.live.start();
      return;
    }
    this.setBusy(false);
    if (r.status === 403) void this.onGone(403, 'removed');
    else if (r.status === 410) void this.onGone(410, 'ended');
    else if (r.error === 'pin_required' || r.error === 'wrong_pin' || r.status === 423) this.setError('This name needs its PIN. Ask the host to help you back in.');
    else if (r.status === 429) this.setError('Too many tries just now. Wait a few seconds and try again.');
    else if (r.status === 0 || r.status >= 500) this.setError("Couldn't reach the quiz. Check your signal and try again.");
    else this.setError("That didn't work. Check the name and try again.");
  }

  private setBusy(busy: boolean): void {
    const b = this.root.querySelector('[data-join]');
    if (!b) return;
    b.textContent = busy ? 'Joining…' : `Join the ${this.view ? noun(this.view) : 'quiz'}`;
    if (busy) b.setAttribute('aria-busy', 'true');
    else b.removeAttribute('aria-busy');
  }

  // 2b Lobby
  private lobby(v: GuestView): HTMLElement {
    const me = v.me!;
    const team = me.team_name
      ? h('div', { class: 'soft teamline' }, h('span', { class: `sw t${(me.team_color ?? 0) % 8}` }), ` ${me.team_name}${me.team_count ? ` · ${fmt(me.team_count)} of ${fmt(v.team_max ?? me.team_count)}` : ''}`)
      : null;
    const here = h('div', { class: 'here', 'data-players': '' });
    this.fillPlayers(here, v);
    const counts = [v.player_count != null ? `${fmt(v.player_count)} player${v.player_count === 1 ? '' : 's'}` : '', v.team_count ? `${fmt(v.team_count)} team${v.team_count === 1 ? '' : 's'}` : '']
      .filter(Boolean)
      .join(' · ');
    return h(
      'div',
      { class: 'body' },
      h('div', { class: 'panel center you' }, creature(me.avatar, 'big', `${me.name}'s creature`), h('h2', null, `You're in, ${me.name}!`), team),
      h('div', { class: 'say' }, quizmaster(36), h('div', { class: 'b' }, v.line || 'Grab a seat. When the TV shows question 1, your answers appear right here.')),
      h('div', null, h('div', { class: 'lbl' }, 'Here so far ', h('small', { 'data-counts': '' }, counts)), here),
      h('div', { class: 'notice plain' }, icon('lock', 'pi'), h('span', null, 'Keep this page open. If your phone sleeps, you come back to the same seat.')),
    );
  }

  private fillPlayers(el: HTMLElement, v: GuestView): void {
    el.replaceChildren(...(v.players ?? []).map((p) => h('div', { class: 'who' }, creature(p.avatar, 'sm'), h('div', { class: 'nm' }, p.name))));
  }

  // 2d Away
  private awayLine(): string {
    const a = this.away!;
    return `Trying again in ${a.left} s · tried ${a.tries} time${a.tries === 1 ? '' : 's'}`;
  }

  private awayScreen(): HTMLElement {
    const me = this.view?.me;
    const btn = h('button', { type: 'button', class: 'gbtn ghost' }, 'Try now');
    btn.addEventListener('click', this.tryNow);
    return h(
      'div',
      { class: 'body mid', role: 'status' },
      h('div', { class: 'qmwrap' }, quizmaster(96, 'thinking')),
      h('h2', null, "The host's box is away"),
      h('p', { class: 'lead' }, me ? ['Your score is safe: ', h('b', { class: 'tab' }, fmt(me.score)), '. '] : null, `This page reconnects by itself as soon as the ${this.view ? noun(this.view) : 'quiz'} is back.`),
      h('p', { class: 'small', 'data-away': '' }, this.awayLine()),
      btn,
    );
  }

  // 2e Full
  private full(v: GuestView): HTMLElement {
    const most = v.max_players ? `${fmt(v.max_players)} players have joined, the most this ${noun(v)} takes. ` : '';
    return this.message(quizmaster(96), `This ${noun(v)} is full`, `${most}Ask the host if a team has room for one more.`);
  }

  // 2f Ended
  private ended(v: GuestView): HTMLElement {
    const e = v.ended;
    const won = e?.winner ? `${e.winner} won${e.points != null ? ` with ${fmt(e.points)} points` : ''}. ` : '';
    const del = v.deletes_on ? `Your name and answers are deleted on ${day(v.deletes_on)}.` : v.retention?.kind === 'at_end' ? 'Your name and answers are deleted now that it has ended.' : privacyLine(v);
    return this.message(quizmaster(96, 'finished'), `This ${noun(v)} has ended`, `${won}Thanks for playing!`, del);
  }

  // 3j Removed
  private removed(): HTMLElement {
    const v = this.view;
    const n = v ? noun(v) : 'quiz';
    return this.message(
      quizmaster(96),
      `You've been removed from this ${n}`,
      `The host took you out of ${v?.title ?? `this ${n}`}. If that's a mistake, have a word with the host.`,
      `Your name and answers are deleted with the ${n}.`,
    );
  }

  // 404, or the socket said the pocket is gone
  private missing(): HTMLElement {
    return this.message(quizmaster(96), 'Nothing here', "This link doesn't open a pocket app any more. Check the link with the host.");
  }

  private paused(v: GuestView): HTMLElement {
    const me = v.me;
    return this.message(quizmaster(96, 'thinking'), 'Back in a moment', `The host paused the ${noun(v)}. ${me ? `Your answers and score are safe: ${fmt(me.score)}.` : ''}`.trim());
  }

  private message(art: HTMLElement, title: string, lead: string, small?: string): HTMLElement {
    return h('div', { class: 'body mid' }, h('div', { class: 'qmwrap' }, art), h('h2', null, title), h('p', { class: 'lead' }, lead), small ? h('p', { class: 'small' }, small) : null);
  }

  // ---------- play ----------

  private meta(q: Question): HTMLElement {
    return h(
      'div',
      { class: 'qmeta' },
      h('span', null, `Round ${q.round}${q.round_name ? ` · ${q.round_name}` : ''}`),
      h('span', null, `Question ${q.n} of ${q.of}`),
    );
  }

  private timer(q: Question): HTMLElement {
    const i = h('i');
    const bar = h('div', { class: 'timer', role: 'timer', 'aria-label': `${q.secs_left} seconds left`, 'data-timer': '' }, i);
    const secs = h('span', { class: 'secs', 'data-secs': '', 'aria-hidden': 'true' }, String(q.secs_left));
    this.paintTimer(bar, secs, q.secs_left, q.total_secs);
    return h('div', { class: 'trow' }, bar, secs);
  }

  private paintTimer(bar: Element, secs: Element, left: number, total: number): void {
    const f = total > 0 ? Math.max(0, Math.min(1, left / total)) : 0;
    // A CSSOM write, which the CSP allows (it bans style attributes in markup, not element.style).
    (bar.firstChild as HTMLElement).style.transform = `scaleX(${f})`;
    bar.classList.toggle('low', left < 10);
    bar.setAttribute('aria-label', `${left} second${left === 1 ? '' : 's'} left`);
    secs.textContent = String(left);
  }

  /** Seconds left on the current question, counted down from when the server's value arrived. */
  secsLeft(q: Question): number {
    return Math.max(0, q.secs_left - Math.floor((this.env.now() - this.stamp) / 1000));
  }

  private runTimer(q: Question): void {
    const step = () => {
      const bar = this.root.querySelector('[data-timer]');
      const secs = this.root.querySelector('[data-secs]');
      const left = this.secsLeft(q);
      if (bar && secs) this.paintTimer(bar, secs, left, q.total_secs);
      if (left <= 0) {
        this.env.clearInterval(this.tick);
        this.root.querySelectorAll<HTMLButtonElement | HTMLInputElement>('[data-answer]').forEach((b) => (b.disabled = true));
        const n = this.root.querySelector('[data-note]');
        if (n && !this.sent.has(q.id)) n.textContent = "Time's up.";
      }
    };
    step();
    this.tick = this.env.setInterval(step, 1000);
  }

  private runCountdown(secs: number): void {
    const el = this.root.querySelector('[data-next]');
    const paint = () => {
      const left = Math.max(0, secs - Math.floor((this.env.now() - this.stamp) / 1000));
      if (el) el.textContent = `Round ${(this.view?.between?.after_round ?? 0) + 1} in ${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
      if (left <= 0) this.env.clearInterval(this.tick);
    };
    paint();
    this.tick = this.env.setInterval(paint, 1000);
  }

  private question(v: GuestView, q: Question): HTMLElement {
    const sent = this.sent.get(q.id);
    if (sent && q.answer) return this.locked(q, sent);
    const body = h('div', { class: 'body' }, this.meta(q), this.timer(q));
    const text = h('p', { class: `qtext${q.kind === 'picture' ? ' sm' : ''}`, id: 'qtext' }, q.text);
    body.append(text);
    if (q.kind === 'picture' && q.picture) body.append(this.picture(q));
    if (q.kind === 'choice' || q.kind === 'picture') body.append(this.choices(q, sent));
    else if (q.kind === 'text') body.append(...this.textAnswer(q, sent));
    else if (q.kind === 'number') body.append(...this.numberAnswer(q, sent));
    void v;
    return body;
  }

  private picture(q: Question): HTMLElement {
    const p = q.picture!;
    const img = h('img', { src: p.url, alt: p.alt || 'The picture for this question', class: p.pixel ? 'pixel' : 'photo', decoding: 'async' });
    return h('figure', { class: 'pic' }, img, p.caption ? h('figcaption', { class: 'cap' }, p.caption) : null);
  }

  private choices(q: Question, sent: Answer | undefined): HTMLElement {
    const opts = q.options ?? [];
    const two = q.kind === 'picture' && opts.every((o) => [...o].length <= 18);
    const picked = sent && 'choice' in sent ? sent.choice : '';
    const wrap = h('div', { class: `answers${two ? ' two' : ''}`, role: 'group', 'aria-labelledby': 'qtext' });
    opts.forEach((o, i) => {
      const k = LETTERS[i];
      const b = h(
        'button',
        { type: 'button', class: `ans${picked === k ? ' pick' : ''}`, 'aria-pressed': picked === k ? 'true' : 'false', 'data-answer': k, disabled: !!picked && picked !== k },
        h('span', { class: 'k', 'aria-hidden': 'true' }, k),
        h('span', { class: 'at' }, o),
      ) as HTMLButtonElement;
      b.setAttribute('aria-label', `${k}: ${o}`);
      b.addEventListener('click', () => this.tapChoice(q, k, wrap));
      wrap.append(b);
    });
    const note = h('p', { class: 'small center', 'data-note': '', role: 'status' }, picked ? this.sendNote(q.id) : 'Tap one. Your first tap counts.');
    return h('div', { class: 'stack' }, wrap, note);
  }

  /** First tap counts: the first answer is marked and sent at once, and every later tap is ignored. */
  tapChoice(q: Question, k: string, wrap: Element): void {
    if (this.sent.has(q.id) || this.secsLeft(q) <= 0) return;
    const a = { choice: k };
    this.sent.set(q.id, a);
    wrap.querySelectorAll<HTMLButtonElement>('[data-answer]').forEach((b) => {
      const on = b.dataset.answer === k;
      b.classList.toggle('pick', on);
      b.setAttribute('aria-pressed', on ? 'true' : 'false');
      if (!on) b.disabled = true;
    });
    const n = this.root.querySelector('[data-note]');
    if (n) n.textContent = this.sendNote(q.id);
    void this.send(q, a);
  }

  private textAnswer(q: Question, sent: Answer | undefined): Node[] {
    const input = h('input', {
      class: 'input',
      type: 'text',
      maxlength: 60,
      autocomplete: 'off',
      spellcheck: 'false',
      enterkeyhint: 'send',
      'aria-labelledby': 'qtext',
      'data-answer': '',
      'data-autofocus': '',
      disabled: !!sent,
    }) as HTMLInputElement;
    input.value = sent && 'text' in sent ? sent.text : this.textDraft;
    input.addEventListener('input', () => (this.textDraft = input.value));
    const btn = h('button', { type: 'submit', class: 'gbtn', 'data-answer': '', 'data-send': '', disabled: !!sent }, sent ? this.sendLabel(q.id) : 'Send answer');
    const form = h(
      'form',
      { class: 'stack' },
      h('div', null, input, h('p', { class: 'small hint' }, q.hint || "Spelling doesn't have to be perfect. Quizmaster settles close ones.")),
      btn,
      h('p', { class: 'small center', 'data-note': '', role: 'status' }, sent ? this.failNote(q.id) : ''),
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      const t = input.value.trim();
      if (!t || this.sent.has(q.id) || this.secsLeft(q) <= 0) return;
      const a = { text: t };
      this.sent.set(q.id, a);
      input.disabled = true;
      (btn as HTMLButtonElement).disabled = true;
      btn.textContent = this.sendLabel(q.id);
      void this.send(q, a);
    });
    return [form];
  }

  private numberAnswer(q: Question, sent: Answer | undefined): Node[] {
    const n = q.number ?? {};
    if (this.numDraft === null || this.key !== this.keyOf(this.view!)) this.numDraft = n.start ?? 0;
    let val = sent && 'number' in sent ? sent.number : this.numDraft!;
    const input = h('input', {
      class: 'input num tab',
      type: 'text',
      inputmode: 'numeric',
      pattern: '-?[0-9]*',
      'aria-labelledby': 'qtext',
      'data-answer': '',
      disabled: !!sent,
    }) as HTMLInputElement;
    input.value = String(val);
    const send = h('button', { type: 'submit', class: 'gbtn', 'data-answer': '', 'data-send': '', disabled: !!sent }, sent ? this.sendLabel(q.id) : `Send ${fmt(val)}`);
    const set = (x: number) => {
      if (n.min != null) x = Math.max(n.min, x);
      if (n.max != null) x = Math.min(n.max, x);
      val = x;
      this.numDraft = x;
      input.value = String(x);
      send.textContent = `Send ${fmt(x)}`;
    };
    input.addEventListener('input', () => {
      const x = parseInt(input.value.replace(/[^0-9-]/g, ''), 10);
      if (!Number.isNaN(x)) {
        val = x;
        this.numDraft = x;
        send.textContent = `Send ${fmt(x)}`;
      }
    });
    const minus = h('button', { type: 'button', class: 'gbtn ghost step', 'aria-label': 'Minus 1', 'data-answer': '', disabled: !!sent }, '−');
    const plus = h('button', { type: 'button', class: 'gbtn ghost step', 'aria-label': 'Plus 1', 'data-answer': '', disabled: !!sent }, '+');
    minus.addEventListener('click', () => set(val - 1));
    plus.addEventListener('click', () => set(val + 1));
    const form = h(
      'form',
      { class: 'stack' },
      h('div', { class: 'numrow' }, minus, input, plus),
      h('p', { class: 'small center' }, q.hint || 'Closest answer wins. Exact gets a 50 point bonus.'),
      send,
      h('p', { class: 'small center', 'data-note': '', role: 'status' }, sent ? this.failNote(q.id) : ''),
    );
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      if (this.sent.has(q.id) || this.secsLeft(q) <= 0) return;
      const a = { number: val };
      this.sent.set(q.id, a);
      form.querySelectorAll<HTMLButtonElement | HTMLInputElement>('[data-answer]').forEach((b) => (b.disabled = true));
      send.textContent = this.sendLabel(q.id);
      void this.send(q, a);
    });
    return [form];
  }

  /** Choice note under a picked answer: pending until the box has it, and honest when it can't be sent. */
  private sendNote(id: string): string {
    if (this.failing.has(id)) return "Couldn't send. Trying again.";
    return this.confirmed.has(id) ? 'Sent. Waiting for the others.' : 'Sending…';
  }

  private sendLabel(id: string): string {
    return this.confirmed.has(id) ? 'Sent' : 'Sending…';
  }

  private failNote(id: string): string {
    return this.failing.has(id) ? "Couldn't send. Trying again." : '';
  }

  /** Repaints the note and send button of the question on screen without redrawing it. */
  private paintSend(id: string): void {
    if (this.view?.question?.id !== id) return;
    const n = this.root.querySelector('[data-note]');
    const b = this.root.querySelector('[data-send]');
    if (b) {
      b.textContent = this.sendLabel(id);
      if (n) n.textContent = this.failNote(id);
    } else if (n && this.sent.has(id)) n.textContent = this.sendNote(id);
  }

  /** The question is still open on this phone, so an answer to it is worth sending. */
  private isOpen(q: Question): boolean {
    return !this.gone && this.view?.phase === 'question' && this.view.question?.id === q.id;
  }

  /**
   * Sends an answer and keeps it pending until the box takes it (ok, or 409: the first one counts). A dropped connection
   * retries every 2 s; after 6 failures the page says so and keeps trying every 5 s while the question is open. A state
   * for this question without `answer` (back from away, a wake, a push) sends it again (resend). A refusal frees the pick.
   */
  private async send(q: Question, a: Answer, attempt = 0): Promise<void> {
    if (this.confirmed.has(q.id) || this.inflight.has(q.id) || !this.isOpen(q)) return;
    this.env.clearTimeout(this.resendTimer);
    this.inflight.add(q.id);
    const r = await this.api.answer(q.id, a);
    this.inflight.delete(q.id);
    if (r.ok || r.status === 409) {
      this.confirmed.add(q.id);
      this.failing.delete(q.id);
      const cur = this.view?.question;
      if (r.ok && r.view) this.show(r.view);
      else if (cur?.id === q.id && this.view?.phase === 'question') this.show({ ...this.view, question: { ...cur, answer: cur.answer ?? a, secs_left: this.secsLeft(cur) } });
      return;
    }
    if (r.status === 0 || r.status >= 500) {
      if (attempt >= 5 && !this.failing.has(q.id)) {
        this.failing.add(q.id);
        this.paintSend(q.id);
      }
      if (this.isOpen(q)) this.resendTimer = this.env.setTimeout(() => void this.send(q, a, attempt + 1), attempt >= 5 ? 5000 : 2000);
      return;
    }
    if (r.status === 401 || r.status === 403 || r.status === 404 || r.status === 410) return void this.onGone(r.status, r.error);
    // Refused (time's up, not a valid answer): it was not taken, so the pick is freed and the screen says what is true.
    this.sent.delete(q.id);
    this.failing.delete(q.id);
    if (this.isOpen(q) && this.view) {
      this.key = '';
      this.show(this.view);
    }
  }

  // 3e Locked in
  private locked(q: Question, a: Answer): HTMLElement {
    const letter = 'choice' in a ? a.choice : '';
    const waiting = q.waiting;
    return h(
      'div',
      { class: 'body' },
      this.meta(q),
      this.timer(q),
      h(
        'div',
        { class: 'panel center locked' },
        h('div', { class: 'ic sky' }, icon('lock', 'pi big')),
        h('h2', null, 'Locked in'),
        h('div', { class: 'ans pick' }, letter ? h('span', { class: 'k' }, letter) : null, h('span', { class: 'at' }, answerText(q, a))),
        h('p', { class: 'soft', 'data-waiting': '' }, this.waitingLine(waiting)),
      ),
    );
  }

  private waitingLine(n: number | undefined): string {
    if (n == null) return 'Eyes on the TV!';
    if (n === 0) return 'Everyone has answered. Eyes on the TV!';
    return `Waiting for ${fmt(n)} more player${n === 1 ? '' : 's'}. Eyes on the TV!`;
  }

  // 3f-3h Reveal
  private reveal(v: GuestView, r: Reveal & { question: Question }): HTMLElement {
    const q = r.question;
    const body = h('div', { class: 'body' }, this.meta(q));
    const sign = (n: number) => (n < 0 ? `−${fmt(-n)}` : `+${fmt(n)}`);
    let panel: HTMLElement;
    if (r.outcome === 'right') {
      panel = h(
        'div',
        { class: 'panel big-result ok' },
        h('div', { class: 'ic ok' }, icon('check', 'pi big')),
        h('div', { class: 'word' }, 'Right!'),
        h('div', { class: 'pts' }, sign(r.points), r.speed ? h('span', { class: 'note' }, ` ${fmt(r.base ?? r.points)} + ${fmt(r.speed)} for speed`) : null),
      );
    } else if (r.outcome === 'near') {
      panel = h(
        'div',
        { class: 'panel big-result near' },
        h('div', { class: 'ic sky' }, icon('star', 'pi big')),
        h('div', { class: 'word' }, 'Close one!'),
        h('p', { class: 'soft said' }, 'You wrote ', h('b', null, `"${r.yours ?? ''}"`), `. ${r.why || 'Close enough, so it counts.'}`),
        h('div', { class: 'pts' }, sign(r.points)),
      );
    } else if (r.outcome === 'overturned') {
      panel = h(
        'div',
        { class: 'panel big-result' },
        h('div', { class: 'word mid' }, 'Not counted'),
        h('p', { class: 'soft said' }, "The host didn't count ", h('b', null, `"${r.yours ?? ''}"`), '.'),
        h('div', { class: 'pts plain' }, sign(r.points)),
      );
    } else {
      panel = h(
        'div',
        { class: 'panel big-result' },
        h('div', { class: 'word mid' }, 'Not this time'),
        h('p', { class: 'soft said' }, r.yours ? ['You said ', h('b', null, r.yours), '. '] : "You didn't answer this one. ", 'The answer is ', h('b', null, r.answer), '.'),
        r.points > 0 ? h('div', { class: 'pts plain' }, sign(r.points), r.close ? h('span', { class: 'note' }, ' for being close') : null) : null,
      );
    }
    body.append(panel);
    if (r.outcome === 'right' || r.outcome === 'near') {
      body.append(
        h(
          'div',
          { class: 'ans right' },
          h('span', { class: 'k' }, r.answer_letter ? r.answer_letter : icon('check', 'pi')),
          h('span', { class: 'at' }, r.answer),
          r.pct != null ? h('span', { class: 'pct' }, `${fmt(r.pct)}% got it`) : null,
        ),
      );
    }
    if (r.source) body.append(h('div', { class: 'source' }, icon('check', 'pi ok'), h('span', null, `Source: ${r.source}`)));
    if (r.team) body.append(h('div', { class: 'lb' }, this.row({ ...r.team, me: true })));
    void v;
    return body;
  }

  private row(r: BoardRow): HTMLElement {
    const move =
      r.move && r.move > 0
        ? h('span', { class: 'up', 'aria-label': `up ${r.move}` }, `▲ ${r.move}`)
        : r.move && r.move < 0
          ? h('span', { class: 'down', 'aria-label': `down ${-r.move}` }, `▼ ${-r.move}`)
          : null;
    return h(
      'div',
      { class: `row${r.me ? ' me' : ''}`, 'aria-current': r.me ? 'true' : null },
      h('span', { class: 'n' }, String(r.rank)),
      h('span', { class: 'nm' }, h('span', { class: `sw t${r.color % 8}` }), r.name),
      h('span', { class: 's' }, fmt(r.score), move ? ' ' : null, move),
    );
  }

  // 3i Between rounds
  private between(v: GuestView): HTMLElement {
    const b = v.between!;
    const mine = b.board.find((r) => r.me);
    const line = !mine || !mine.move ? (mine ? 'Your team: same place as last round' : '') : `Your team: ${mine.move > 0 ? 'up' : 'down'} ${Math.abs(mine.move)} place${Math.abs(mine.move) === 1 ? '' : 's'} this round`;
    const lb = h('div', { class: 'lb', role: 'list' });
    add(lb, b.board.slice(0, 5).map((r) => {
      const el = this.row(r);
      el.setAttribute('role', 'listitem');
      return el;
    }));
    if (mine && !b.board.slice(0, 5).includes(mine)) lb.append(this.row(mine));
    return h(
      'div',
      { class: 'body' },
      h('div', { class: 'qmeta' }, h('span', null, `After round ${b.after_round} of ${b.rounds}`), h('span', { 'data-next': '' }, b.next_in_secs != null ? '' : 'Next round soon')),
      b.line ? h('div', { class: 'say' }, quizmaster(48, 'finished'), h('div', { class: 'b lg' }, b.line)) : null,
      lb,
      line ? h('p', { class: 'small center' }, line) : null,
    );
  }

  // ---------- small in-place updates (keep focus and typing) ----------

  private patch(v: GuestView): void {
    const bar = this.root.querySelector('[data-bar]');
    const scr = this.root.querySelector('[data-screen]')?.getAttribute('data-screen') ?? '';
    if (bar) bar.textContent = this.barRight(scr);
    const teams = this.root.querySelector('[data-teams]') as (HTMLElement & { refill?: () => void }) | null;
    if (teams?.refill) {
      // Re-read the teams from the new view, keeping the guest's choice and typing.
      const fresh = this.teamList(v).querySelector('[data-teams]') as HTMLElement & { refill?: () => void };
      if (!this.draft.making) teams.replaceWith(fresh);
    }
    const players = this.root.querySelector<HTMLElement>('[data-players]');
    if (players) this.fillPlayers(players, v);
    const counts = this.root.querySelector('[data-counts]');
    if (counts && v.player_count != null) counts.textContent = [`${fmt(v.player_count)} player${v.player_count === 1 ? '' : 's'}`, v.team_count ? `${fmt(v.team_count)} team${v.team_count === 1 ? '' : 's'}` : ''].filter(Boolean).join(' · ');
    const waiting = this.root.querySelector('[data-waiting]');
    if (waiting) waiting.textContent = this.waitingLine(v.question?.waiting);
    if (v.question && v.phase === 'question') {
      this.env.clearInterval(this.tick);
      this.runTimer(v.question);
    }
    const priv = this.root.querySelector('[data-privacy]');
    if (priv) priv.textContent = privacyLine(v);
  }
}
