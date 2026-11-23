// The host remote (pa4b, pa0a/design.md section 5): the host runs the quiz from their phone. It signs in once with
// its link's single-use token (host/signin {token}, pa1b "Pocket API v1.1: links"), then reads host/state and
// ws?view=host and writes with host/action, host/remove and host/end. Every button follows what the box says is
// allowed now (host.can); removing a player and ending the quiz ask first. Nothing here works out a score: every
// number comes from the box. Plain TS + DOM; no inline style (the pocket CSP), text only through text nodes.
import { Api, Env, Fail, Live, backoff } from '../api';
import { Kid, add, fmt, h, quizmaster, svg } from '../dom';
import { R_ICONS } from './icons';
import { HostPlayer, Pending, RemoteView, nextStep, paused, remoteFrom } from './view';

export interface RemoteEnv extends Env {
  now: () => number;
  setInterval: (f: () => void, ms: number) => number;
  clearInterval: (id: number) => void;
}

type Tab = 'game' | 'teams' | 'players';
/** Full-page screens; 'live' is the remote itself. */
type Screen = 'loading' | 'live' | 'expired' | 'signed-out' | 'screen-only' | 'gone' | 'slow';
type Sheet = { kind: 'remove'; id: string } | { kind: 'end' } | null;
type Target = { team: string } | { player: string };

const TABS: [Tab, string][] = [['game', 'Game'], ['teams', 'Teams'], ['players', 'Players']];
const LETTERS = 'ABCDEFGH';
/** The kit takes at most ±100 per adjust (src/kits/quiz/engine.rs MAX_ADJUST). */
const STEP = 100;
/** Not in the quiz kit or pa1 yet: drawn disabled with the reason (pa4b/impl.md). */
export const LATER = {
  skip: 'Skipping a question needs a quiz kit update',
  voice: 'Voice comes with the Quizmaster voice update',
  rename: 'Renaming comes with a later update',
} as const;

const ic = (name: keyof typeof R_ICONS, cls = 'ri') => svg(R_ICONS[name][0], R_ICONS[name][1], cls);
const clock = (ms: number): string => {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};
const signed = (n: number) => `${n > 0 ? '+' : '−'}${fmt(Math.abs(n))}`;
const plural = (n: number, one: string, many: string) => `${fmt(n)} ${n === 1 ? one : many}`;

export class RemoteApp {
  private api: Api<RemoteView>;
  private live: Live<RemoteView>;
  private view: RemoteView | undefined;
  private screen: Screen = 'loading';
  private tab: Tab = 'game';
  private sheet: Sheet = null;
  private opener: string | null = null;
  private busy = false;
  private isAway = false;
  private away = { secs: 0, tries: 0 };
  private note: { text: string; bad: boolean } | null = null;
  private noteTimer = 0;
  private last: { target: Target; who: string; points: number } | null = null;
  /** Team colours in the order the remote first saw them (design.md "Team colours ... join order"). */
  private colors = new Map<string, number>();
  private at = 0;
  private ticker = 0;
  private clockEl: HTMLElement | null = null;
  private barEl: HTMLElement | null = null;
  private page: HTMLElement;
  private said: HTMLElement;

  constructor(root: HTMLElement, private env: RemoteEnv, readonly code: string, private token?: string) {
    this.api = new Api<RemoteView>(env, code, remoteFrom);
    this.api.statePath = 'host/state';
    this.api.wsPath = 'ws?view=host';
    this.live = new Live<RemoteView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => {
        this.isAway = true;
        this.away = { secs, tries };
        this.render();
      },
      back: () => {
        this.isAway = false;
        this.render();
      },
      gone: (status, error) => this.gone(status, error),
    });
    this.page = h('div', { class: 'g rm' });
    this.said = h('p', { class: 'vh', 'aria-live': 'polite' });
    root.replaceChildren(this.page, this.said);
    root.addEventListener('keydown', (e) => this.key(e as KeyboardEvent));
  }

  /** Signs in with the link's token once (retrying while the box is away), then goes live on the cookie. */
  async start(_boot?: unknown): Promise<void> {
    this.screen = 'loading';
    this.render();
    let tries = 0;
    for (;;) {
      const r = this.token ? await this.api.call('host/signin', { token: this.token }) : await this.api.state();
      if (r.ok && !r.view) {
        // Signed in, but not as a host (a big-screen link): host/state says which, in its own words.
        const s = await this.api.state();
        if (!s.ok) return s.status !== 0 && s.status < 500 ? this.gone(s.status, s.error) : this.gone(401, 'signed_out');
        r.view = s.view;
      }
      if (r.ok) {
        this.token = undefined;
        this.isAway = false;
        this.screen = 'live';
        if (r.view) this.show(r.view);
        else this.render();
        this.live.start();
        return;
      }
      if (r.status !== 0 && r.status < 500) return this.gone(r.status, r.error);
      this.isAway = true;
      this.away = { secs: backoff(tries), tries: ++tries };
      this.render();
      await new Promise<void>((res) => this.env.setTimeout(res, this.away.secs * 1000));
    }
  }

  wake(): void {
    if (this.screen === 'live') this.live.now();
  }

  private gone(status: number, error?: string): void {
    this.live.stop();
    this.stopTicker();
    this.sheet = null;
    if ((status === 410 || error === 'ended') && this.view) {
      this.view = { ...this.view, ended: true, can: [] };
      this.screen = 'live';
    } else
      this.screen =
        error === 'link_used_or_expired' ? 'expired'
        : error === 'screen_only' ? 'screen-only'
        : error === 'slow_down' || status === 429 ? 'slow'
        : status === 404 ? 'gone'
        : 'signed-out';
    this.isAway = false;
    this.render();
  }

  private show(v: RemoteView): void {
    const was = this.view;
    this.view = v;
    this.at = this.env.now();
    for (const t of v.teamBoard) if (!this.colors.has(t.team)) this.colors.set(t.team, this.colors.size);
    for (const p of v.players) if (p.team && !this.colors.has(p.team)) this.colors.set(p.team, this.colors.size);
    if (this.sheet?.kind === 'remove') {
      const id = this.sheet.id;
      if (!v.players.some((p) => p.id === id && !p.removed)) this.sheet = null;
    }
    if (this.sheet?.kind === 'end' && (v.ended || v.phase === 'final')) this.sheet = null;
    if (was && was.phase !== v.phase) this.say(this.phaseWords(v));
    this.render();
  }

  private phaseWords(v: RemoteView): string {
    if (v.ended || v.phase === 'final') return 'The quiz has ended.';
    if (paused(v)) return 'Paused.';
    if (v.phase === 'question') return `Question ${v.question?.number ?? ''} is open.`;
    if (v.phase === 'locked') return "Time's up.";
    if (v.phase === 'reveal') return 'The answer is showing.';
    if (v.phase === 'leaderboard') return 'The scores are showing.';
    return '';
  }

  private say(text: string): void {
    this.said.textContent = text;
  }

  // ---- writes -----------------------------------------------------------------------------------

  /** One host write. Its answer is the host view after it; a refusal is shown in plain words. */
  private async write(path: string, body: unknown): Promise<RemoteView | undefined> {
    if (this.busy || this.isAway || this.screen !== 'live') return undefined;
    this.busy = true;
    this.render();
    const r = await this.api.call(path, body);
    this.busy = false;
    if (r.ok) {
      if (r.view) this.show(r.view);
      else this.render();
      return r.view ?? this.view;
    }
    this.failed(r);
    return undefined;
  }

  private failed(r: Fail): void {
    if (r.status === 401 || r.status === 404 || r.status === 410 || r.error === 'screen_only') return this.gone(r.status, r.error);
    const text =
      r.status === 0 ? "Couldn't reach your quiz box. Try again."
      : r.status === 429 ? (r.message ?? 'Too many tries from here. Wait a moment.')
      : (r.message ?? "That didn't work. Try again.");
    this.flash(text, true);
  }

  private flash(text: string, bad = false): void {
    this.note = { text, bad };
    this.env.clearTimeout(this.noteTimer);
    this.noteTimer = this.env.setTimeout(() => {
      this.note = null;
      this.render();
    }, 5000);
    this.render();
  }

  private act = (name: string, body: Record<string, unknown> = {}) => this.write('host/action', { name, body });

  /** The big button. From an open question, Reveal now closes the answers and then reveals: two `next`s. */
  async next(): Promise<void> {
    const v = this.view;
    const s = v && nextStep(v);
    if (!v || !s || !s.ok) return;
    const fromQuestion = s.action === 'next' && v.phase === 'question';
    const after = await this.act(s.action);
    if (fromQuestion && after?.phase === 'locked' && after.can.includes('next')) await this.act('next');
  }

  async pause(): Promise<void> {
    if (this.view?.can.includes('pause')) await this.act('pause');
  }

  async judge(p: Pending, verdict: 'yes' | 'no'): Promise<void> {
    const who = this.player(p.player)?.name ?? 'that player';
    const after = await this.act('judge', { q: p.q, player: p.player, verdict });
    if (after) this.flash(verdict === 'yes' ? `Accepted ${who}'s answer.` : `Marked ${who}'s answer not right.`);
  }

  async adjust(target: Target, points: number, undo = false): Promise<void> {
    const who = 'team' in target ? target.team : (this.player(target.player)?.name ?? 'that player');
    const after = await this.act('adjust', { ...target, points });
    if (!after) return;
    this.last = undo ? null : { target, who, points };
    if (undo) this.flash(`Undone: ${signed(-points)} to ${who}.`);
    else this.render();
  }

  async undo(): Promise<void> {
    if (this.last) await this.adjust(this.last.target, -this.last.points, true);
  }

  async remove(id: string): Promise<void> {
    const who = this.player(id)?.name ?? 'that player';
    const after = await this.write('host/remove', { player_id: id });
    if (after) {
      this.closeSheet();
      this.flash(`${who} was removed.`);
    }
  }

  async end(): Promise<void> {
    const after = await this.write('host/end', {});
    if (after) {
      this.closeSheet();
      this.say('The quiz has ended. Everyone sees the winners.');
    }
  }

  // ---- sheets and keys --------------------------------------------------------------------------

  openSheet(s: NonNullable<Sheet>, opener: string): void {
    this.sheet = s;
    this.opener = opener;
    this.render();
    (this.page.querySelector('.rsheet [data-first]') as HTMLElement | null)?.focus();
  }

  closeSheet(): void {
    if (!this.sheet) return;
    this.sheet = null;
    this.render();
    if (this.opener) (this.page.querySelector(`[data-k="${this.opener}"]`) as HTMLElement | null)?.focus();
    this.opener = null;
  }

  setTab(t: Tab, focus = false): void {
    this.tab = t;
    this.render();
    if (focus) (this.page.querySelector(`[data-k="tab-${t}"]`) as HTMLElement | null)?.focus();
  }

  private key(e: KeyboardEvent): void {
    if (e.key === 'Escape' && this.sheet) {
      e.preventDefault();
      return this.closeSheet();
    }
    if (this.sheet && e.key === 'Tab') {
      // Keep focus inside the open sheet.
      const items = [...this.page.querySelectorAll<HTMLElement>('.rsheet button:not([disabled])')];
      if (!items.length) return;
      const i = items.indexOf(document.activeElement as HTMLElement);
      const to = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : i === items.length - 1 ? 0 : i + 1;
      e.preventDefault();
      items[to].focus();
      return;
    }
    const t = (e.target as HTMLElement | null)?.getAttribute?.('data-k') ?? '';
    if (t.startsWith('tab-') && (e.key === 'ArrowRight' || e.key === 'ArrowLeft')) {
      e.preventDefault();
      const i = TABS.findIndex(([k]) => `tab-${k}` === t);
      this.setTab(TABS[(i + (e.key === 'ArrowRight' ? 1 : TABS.length - 1)) % TABS.length][0], true);
    }
  }

  // ---- drawing ----------------------------------------------------------------------------------

  private player(id: string): HostPlayer | undefined {
    return this.view?.players.find((p) => p.id === id);
  }

  private color(team?: string): string {
    if (!team) return 'sw';
    const i = this.colors.get(team) ?? 0;
    return `sw t${i % 8}${i >= 8 ? ' stripe' : ''}`;
  }

  /** Redraws everything, keeping focus on the same control (data-k) across the redraw. */
  render(): void {
    const focused = (document.activeElement as HTMLElement | null)?.getAttribute?.('data-k');
    this.stopTicker();
    this.page.setAttribute('data-screen', this.screen);
    this.page.replaceChildren();
    add(this.page, this.draw());
    if (focused) (this.page.querySelector(`[data-k="${focused}"]`) as HTMLElement | null)?.focus();
    this.startTicker();
  }

  private draw(): Kid[] {
    const v = this.view;
    if (this.screen === 'loading') return [this.header(), this.middle(quizmaster(64, 'thinking'), 'Opening the remote…', 'Signing in with your link.', this.awayLine())];
    if (this.screen === 'expired')
      return [this.header(), this.middle(ic('link', 'ri big'), 'This link was already used or is too old', 'Open a new one from the card: press Open the remote again. Each link works once, for 10 minutes.')];
    if (this.screen === 'signed-out')
      return [this.header(), this.middle(ic('link', 'ri big'), 'Open the remote again from the card', 'This phone is signed out of the remote. That happens when the host link changes. The quiz keeps running.')];
    if (this.screen === 'screen-only')
      return [this.header(), this.middle(ic('link', 'ri big'), 'Open the remote again from the card', 'This browser is signed in as a big screen, which can only show the game.')];
    if (this.screen === 'gone') return [this.header(), this.middle(ic('x', 'ri big'), 'This quiz is gone', 'It was deleted, or the link is wrong. Your chat has the card for any quiz that is still on.')];
    if (this.screen === 'slow')
      return [
        this.header(),
        this.middle(ic('warn', 'ri big'), 'Too many tries from here', 'Wait a moment, then try again.', h('button', { type: 'button', class: 'rbtn ghost', 'data-k': 'retry', onclick: () => void this.start() }, 'Try again')),
      ];
    if (!v) return [this.header(), this.middle(quizmaster(64, 'thinking'), 'Opening the remote…', '')];
    const body = this.tab === 'game' ? this.game(v) : this.tab === 'teams' ? this.teamsTab(v) : this.playersTab(v);
    return [
      this.header(v),
      h(
        'main',
        { class: 'rbody', inert: !!this.sheet, 'aria-busy': this.busy ? 'true' : null },
        this.isAway ? this.awayBanner() : null,
        this.tabs(),
        h('div', { role: 'tabpanel', id: `panel-${this.tab}`, 'aria-labelledby': `tab-${this.tab}`, class: 'rpanel' }, body),
        h('p', { class: `rnote${this.note?.bad ? ' bad' : ''}`, role: 'status' }, this.note?.text ?? ''),
      ),
      this.sheet ? this.sheetEl(v, this.sheet) : null,
    ];
  }

  private header(v?: RemoteView): HTMLElement {
    const over = !!v && (v.ended || v.phase === 'final');
    const chip =
      !v ? null
      : over ? h('span', { class: 'rchip' }, 'Ended')
      : paused(v) ? h('span', { class: 'rchip warn' }, ic('pause', 'ri sm'), 'Paused')
      : v.phase === 'lobby' ? h('span', { class: 'rchip' }, 'Lobby')
      : h('span', { class: 'rchip live' }, h('i', { class: 'dot', 'aria-hidden': 'true' }), 'Live');
    return h(
      'header',
      { class: 'rhead', inert: !!this.sheet },
      // Moods (pa0a): the host drives a live quiz, so no badge; thinking only while the box is away.
      quizmaster(48, over ? 'finished' : this.isAway ? 'thinking' : undefined),
      h('div', { class: 'rtitle' }, h('h1', null, v?.title ? `Remote · ${v.title}` : 'Remote'), h('p', null, 'Only you see this')),
      chip,
    );
  }

  private middle(art: Kid, title: string, text: string, ...more: Kid[]): HTMLElement {
    return h('main', { class: 'rbody rmid' }, h('div', { class: 'rart' }, art), h('h2', null, title), text ? h('p', { class: 'soft' }, text) : null, ...more);
  }

  private awayLine(): Kid {
    return this.isAway ? h('p', { class: 'small' }, `Can't reach your quiz box. Trying again in ${this.away.secs} s.`) : null;
  }

  private awayBanner(): HTMLElement {
    return h(
      'div',
      { class: 'raway', role: 'status' },
      ic('warn', 'ri'),
      h('span', null, `Can't reach your quiz box. Trying again in ${this.away.secs} s. Buttons wait until it's back.`),
      h('button', { type: 'button', class: 'rlink', 'data-k': 'try-now', onclick: () => this.live.now() }, 'Try now'),
    );
  }

  private tabs(): HTMLElement {
    return h(
      'div',
      { class: 'rtabs', role: 'tablist', 'aria-label': 'Remote' },
      TABS.map(([k, label]) =>
        h(
          'button',
          {
            type: 'button',
            role: 'tab',
            id: `tab-${k}`,
            'data-k': `tab-${k}`,
            'aria-selected': this.tab === k ? 'true' : 'false',
            'aria-controls': `panel-${k}`,
            tabindex: this.tab === k ? '0' : '-1',
            onclick: () => this.setTab(k),
          },
          // A quiz without teams scores players on this tab.
          k === 'teams' && this.view && !this.view.teams ? 'Scores' : label,
        ),
      ),
    );
  }

  /** A write button: aria-disabled while not allowed or while a write is on its way (focus stays put). */
  private btn(k: string, cls: string, ok: boolean, run: () => void, ...kids: Kid[]): HTMLElement {
    const off = !ok || this.busy || this.isAway;
    return h('button', { type: 'button', class: `rbtn ${cls}`, 'data-k': k, 'aria-disabled': off ? 'true' : null, onclick: () => !off && run() }, ...kids);
  }

  /** Not built yet: disabled, with the reason as its description and tooltip. */
  private later(k: string, cls: string, why: string, ...kids: Kid[]): HTMLElement {
    return h('button', { type: 'button', class: `rbtn ${cls}`, 'data-k': k, 'aria-disabled': 'true', 'data-later': why, title: why, 'aria-description': why }, ...kids);
  }

  // ---- Game tab ---------------------------------------------------------------------------------

  private game(v: RemoteView): Kid[] {
    const over = v.ended || v.phase === 'final';
    const s = nextStep(v);
    const isPaused = paused(v);
    const out: Kid[] = [this.nowBox(v)];
    if (s)
      out.push(
        this.btn('next', 'next', s.ok, () => void this.next(), ic(s.action === 'resume' ? 'play' : s.action === 'start' ? 'play' : 'skip', 'ri lg'), h('span', { class: 'nx' }, h('b', null, s.label), s.sub ? h('small', null, s.sub) : null)),
      );
    if (!over && v.phase !== 'lobby') {
      out.push(
        h(
          'div',
          { class: `rrow${isPaused ? ' two' : ''}` },
          isPaused ? null : this.btn('pause', 'tile', v.can.includes('pause'), () => void this.pause(), ic('pause', 'ri md'), h('span', null, 'Pause')),
          this.later('skip', 'tile', LATER.skip, ic('skip', 'ri md'), h('span', null, 'Skip question')),
          this.later('voice', 'tile', LATER.voice, ic('voice', 'ri md'), h('span', null, 'Voice off')),
        ),
        // Touch has no hover, so the reason for the two dead tiles is said out loud (Mira round 1, fix 1).
        h('p', { class: 'small center rlater', 'data-k': 'later-line' }, 'Skip and Voice come in a later update.'),
      );
    }
    if (!over && v.pending.length && v.can.includes('judge')) out.push(this.nearMiss(v));
    if (!over) out.push(this.btn('end', 'endlink', v.can.includes('end'), () => this.openSheet({ kind: 'end' }, 'end'), ic('flag', 'ri'), 'End the quiz'));
    return out;
  }

  private nowBox(v: RemoteView): HTMLElement {
    if (v.ended || v.phase === 'final') {
      const lead = this.leader(v);
      return h('section', { class: 'rnow mid' }, h('h2', null, 'The quiz has ended'), h('p', { class: 'soft' }, lead ? `${lead.names} ${lead.many ? 'share first' : 'won'} with ${fmt(lead.score)}. Everyone sees the winners, and the results are in your chat.` : 'Everyone sees the final results, and they are in your chat.'));
    }
    if (paused(v))
      return h('section', { class: 'rnow mid' }, h('h2', null, 'Paused'), h('p', { class: 'soft' }, `The timer stopped at ${clock(v.time?.remaining_ms ?? 0)}. Phones and the big screen say "Back in a moment".`));
    if (v.phase === 'lobby') {
      const n = v.players.filter((p) => !p.removed).length;
      return h('section', { class: 'rnow' }, h('p', { class: 'meta' }, 'Lobby'), h('p', { class: 'qt' }, n ? `${plural(n, 'player is', 'players are')} in.` : 'Nobody has joined yet.'), h('p', { class: 'small' }, n ? 'Press Start when everyone is in. Late arrivals can still join.' : 'Players show up here as they scan the code. Start when everyone is in.'));
    }
    const q = v.question;
    const meta = [v.round ? `Round ${v.round.number}` : null, q ? `question ${q.number} of ${q.of}` : null].filter(Boolean).join(' · ');
    const right =
      v.phase === 'question' && v.time ? (this.clockEl = h('span', { class: 'clock tab', 'aria-hidden': 'true' }, clock(this.left())))
      : v.phase === 'locked' ? h('span', { class: 'clock sm' }, "Time's up")
      : v.phase === 'reveal' ? h('span', { class: 'clock sm' }, 'Answer showing')
      : v.phase === 'leaderboard' ? h('span', { class: 'clock sm' }, 'Scores showing')
      : null;
    const letter = v.answer?.position !== undefined ? `${LETTERS[v.answer.position] ?? ''}, ` : '';
    return h(
      'section',
      { class: 'rnow', 'aria-label': 'Now' },
      h('div', { class: 'rmeta' }, h('p', { class: 'meta' }, meta), right),
      q ? h('p', { class: 'qt' }, q.text) : null,
      v.phase === 'question' && v.time ? h('div', { class: 'rbar', 'aria-hidden': 'true' }, (this.barEl = h('i', null))) : null,
      h(
        'p',
        { class: 'rcount' },
        v.answered ? [h('b', null, `${fmt(v.answered.count)} of ${fmt(v.answered.of)}`), ' answered'] : null,
        v.answer ? [v.answered ? ' · ' : '', `answer: ${letter}${v.answer.text}`] : null,
      ),
    );
  }

  private left(): number {
    const t = this.view?.time;
    return t ? Math.max(0, t.remaining_ms - (this.env.now() - this.at)) : 0;
  }

  private startTicker(): void {
    const v = this.view;
    if (!this.clockEl || !v?.time || v.phase !== 'question' || v.time.paused) return;
    const tick = () => {
      const ms = this.left();
      if (this.clockEl) this.clockEl.textContent = clock(ms);
      const total = (v.question?.time_limit_s ?? 0) * 1000;
      this.barEl?.style.setProperty('--f', String(total > 0 ? Math.min(1, ms / total) : 0));
    };
    tick();
    this.ticker = this.env.setInterval(tick, 1000);
  }

  private stopTicker(): void {
    if (this.ticker) this.env.clearInterval(this.ticker);
    this.ticker = 0;
    this.clockEl = null;
    this.barEl = null;
  }

  private nearMiss(v: RemoteView): HTMLElement {
    const p = v.pending[0];
    const who = this.player(p.player);
    const pts = v.question && v.question.q === p.q ? v.question.points : 0;
    const also = p.accept.filter((a) => a !== p.answer);
    return h(
      'section',
      { class: 'rmiss', 'aria-label': 'Needs you' },
      h('p', { class: 'mh' }, ic('warn', 'ri'), `Needs you · ${plural(v.pending.length, 'near miss', 'near misses')}`),
      h('p', { class: 'mw' }, h('b', null, who?.name ?? 'A player'), who?.team ? ` (${who.team})` : '', ' wrote ', h('b', null, `"${p.guess}"`)),
      h('p', { class: 'small' }, `Answer: ${p.answer}${also.length ? ` · also accepted: ${also.join(', ')}` : ''}${v.question && v.question.q !== p.q ? ` · from question ${p.q + 1}` : ''}`),
      h(
        'div',
        { class: 'mrow' },
        this.btn(`yes-${p.q}-${p.player}`, 'primary', true, () => void this.judge(p, 'yes'), ic('check'), pts ? `Accept +${fmt(pts)}` : 'Accept'),
        this.btn(`no-${p.q}-${p.player}`, 'ghost', true, () => void this.judge(p, 'no'), ic('x'), 'Not right'),
      ),
      h('p', { class: 'small' }, "If you don't answer before the reveal, Quizmaster's call stands."),
    );
  }

  private leader(v: RemoteView): { names: string; score: number; many: boolean } | undefined {
    const rows = v.teams && v.teamBoard.length ? v.teamBoard.map((t) => ({ name: t.team, score: t.score })) : v.players.filter((p) => !p.removed).map((p) => ({ name: p.name, score: p.score }));
    if (!rows.length) return undefined;
    const top = Math.max(...rows.map((r) => r.score));
    const first = rows.filter((r) => r.score === top).map((r) => r.name);
    return { names: first.length > 1 ? `${first.slice(0, -1).join(', ')} and ${first[first.length - 1]}` : first[0], score: top, many: first.length > 1 };
  }

  // ---- Teams tab --------------------------------------------------------------------------------

  private teamsTab(v: RemoteView): Kid[] {
    const ok = v.can.includes('adjust');
    const teams = v.teams;
    const rows = teams
      ? v.teamBoard.map((t) => ({ key: t.team, name: t.team, team: t.team, score: t.score, target: { team: t.team } as Target }))
      : v.players.filter((p) => !p.removed).sort((a, b) => b.score - a.score).map((p) => ({ key: p.id, name: p.name, team: p.team, score: p.score, target: { player: p.id } as Target }));
    const out: Kid[] = [h('p', { class: 'rintro' }, `+ and − change a ${teams ? "team's" : "player's"} score by 100. Everyone sees the new score at the next leaderboard.`)];
    if (!rows.length) out.push(h('section', { class: 'rlist empty' }, h('p', { class: 'soft' }, teams ? 'No teams yet. They show here as players join.' : 'No players yet.')));
    else
      out.push(
        h(
          'ul',
          { class: 'rlist', 'aria-label': teams ? 'Teams' : 'Players' },
          rows.map((r) =>
            h(
              'li',
              { class: 'rrowi' },
              h('i', { class: this.color(r.team), 'aria-hidden': 'true' }),
              h('span', { class: 'nm' }, r.name),
              h('span', { class: 'sc tab' }, fmt(r.score)),
              this.btn(`minus-${r.key}`, 'sq', ok, () => void this.adjust(r.target, -STEP), ic('minus'), h('span', { class: 'vh' }, `Take 100 from ${r.name}`)),
              this.btn(`plus-${r.key}`, 'sq', ok, () => void this.adjust(r.target, STEP), ic('plus'), h('span', { class: 'vh' }, `Give 100 to ${r.name}`)),
            ),
          ),
        ),
      );
    if (this.last)
      out.push(
        h(
          'section',
          { class: 'rundo' },
          ic('undo', 'ri'),
          h('span', null, `Last change: ${signed(this.last.points)} to ${this.last.who}. `),
          h('button', { type: 'button', class: 'rlink', 'data-k': 'undo', 'aria-disabled': this.busy || this.isAway || !ok ? 'true' : null, onclick: () => !this.busy && ok && void this.undo() }, 'Undo'),
        ),
      );
    return out;
  }

  // ---- Players tab ------------------------------------------------------------------------------

  private status(v: RemoteView, p: HostPlayer): string {
    if (p.removed) return 'removed';
    if (v.phase !== 'lobby' && v.answeredBy.includes(p.id)) return 'answered';
    if (!p.connected) return 'asleep';
    return '—';
  }

  private playersTab(v: RemoteView): Kid[] {
    if (!v.players.length) return [h('section', { class: 'rlist empty' }, h('p', { class: 'soft' }, 'Nobody has joined yet. Players show here as they scan in.'))];
    const ok = v.can.includes('remove');
    return [
      h(
        'ul',
        { class: 'rlist', 'aria-label': 'Players' },
        v.players.map((p) => {
          const st = this.status(v, p);
          return h(
            'li',
            { class: `rrowi pl${p.removed ? ' gone' : ''}` },
            h('i', { class: this.color(p.team), 'aria-hidden': 'true' }),
            h('span', { class: 'who' }, h('b', null, p.name), p.team ? h('small', null, p.team) : null),
            h('span', { class: `st${st === 'answered' ? ' ok' : ''}` }, st),
            p.removed ? h('span', { class: 'sqgap' }) : this.btn(`more-${p.id}`, 'sq', ok, () => this.openSheet({ kind: 'remove', id: p.id }, `more-${p.id}`), ic('dots'), h('span', { class: 'vh' }, `More for ${p.name}`)),
          );
        }),
      ),
    ];
  }

  // ---- sheets -----------------------------------------------------------------------------------

  private sheetEl(v: RemoteView, s: NonNullable<Sheet>): HTMLElement {
    let inner: Kid[];
    if (s.kind === 'remove') {
      const p = this.player(s.id);
      const name = p?.name ?? 'this player';
      inner = [
        h('div', { class: 'sh-who' }, h('i', { class: this.color(p?.team), 'aria-hidden': 'true' }), h('div', null, h('h2', { id: 'sheet-h' }, name), p?.team ? h('p', { class: 'small' }, p.team) : null)),
        h('p', { class: 'sh-text' }, `Remove ${name}? Their phone shows "You've been removed", they can't rejoin, and their answers stop counting${p?.team ? ` for ${p.team}` : ''}.`),
        h('button', { type: 'button', class: 'rbtn badline', 'data-k': 'sheet-remove', 'data-first': true, 'aria-disabled': this.busy ? 'true' : null, onclick: () => !this.busy && void this.remove(s.id) }, ic('userx'), 'Remove from the quiz'),
        this.later('sheet-rename', 'ghost', LATER.rename, ic('pencil'), 'Rename to "Player 9" instead'),
        h('p', { class: 'small center' }, `${LATER.rename}.`),
        h('button', { type: 'button', class: 'rlink cancel', 'data-k': 'sheet-cancel', onclick: () => this.closeSheet() }, 'Cancel'),
      ];
    } else {
      const lead = this.leader(v);
      inner = [
        h('h2', { id: 'sheet-h' }, 'End the quiz now?'),
        h('p', { class: 'sh-text' }, `Scores so far become the final results${lead ? `: ${lead.names} ${lead.many ? 'share first' : lead.names.includes(' and ') ? 'win' : 'win'} with ${fmt(lead.score)}` : ''}. Everyone sees the winners, the link closes, and the results go to your chat.`),
        h('button', { type: 'button', class: 'rbtn badfill', 'data-k': 'sheet-end', 'data-first': true, 'aria-disabled': this.busy ? 'true' : null, onclick: () => !this.busy && void this.end() }, ic('flag'), 'End and show winners'),
        h('button', { type: 'button', class: 'rbtn ghost', 'data-k': 'sheet-keep', onclick: () => this.closeSheet() }, 'Keep playing'),
      ];
    }
    return h(
      'div',
      { class: 'rshade', onclick: (e: Event) => e.target === e.currentTarget && this.closeSheet() },
      h('div', { class: 'rsheet', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'sheet-h', 'data-sheet': s.kind }, h('i', { class: 'grab', 'aria-hidden': 'true' }), ...inner),
    );
  }
}
