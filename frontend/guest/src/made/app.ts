// pa12 P2: the page for a `made` pocket on the three surfaces. The phone joins with a first name and reads its own
// `state`; the big screen signs in with its link's token and reads `host/state?view=screen`; the host remote signs in
// with its token and reads `host/state`, writing with `host/action`. All three draw the spec's ready-made blocks for
// their surface (blocks.ts) from the view the box filtered for them. No bot-written code, no frame (that is P1).
import { Api, Env, Live, backoff } from '../api';
import { creature, h } from '../dom';
import { drawBlocks, newUi, stepBar, Ui } from './blocks';
import { clock as clockText, madeFrom, MadeView, secondsLeft, Surface } from './view';

export type MadeEnv = Env & { now: () => number; setInterval: (f: () => void, ms: number) => number; clearInterval: (id: number) => void };
type Screen = 'loading' | 'join' | 'live' | 'away' | 'ended' | 'signed-out' | 'gone';

const LABEL: Record<Surface, string> = { phone: 'Pocket app', screen: 'Big screen', remote: 'Host remote' };

export class MadeApp {
  private api: Api<MadeView>;
  private live: Live<MadeView>;
  view: MadeView | undefined;
  private screen: Screen = 'loading';
  private ui: Ui;
  private skew = 0;
  private tick = 0;
  private away = { secs: 0, tries: 0 };
  private joinErr = '';
  private name = '';
  private busy = false;
  private main: HTMLElement;

  constructor(root: HTMLElement, private env: MadeEnv, readonly code: string, readonly surface: Surface = 'phone', private token?: string) {
    this.api = new Api<MadeView>(env, code, madeFrom);
    if (surface === 'screen') {
      this.api.statePath = 'host/state?view=screen';
      this.api.wsPath = 'ws?view=screen';
    } else if (surface === 'remote') {
      this.api.statePath = 'host/state';
      this.api.wsPath = 'ws?view=host';
    }
    this.live = new Live<MadeView>(env, this.api, {
      view: (v) => this.show(v),
      away: (secs, tries) => {
        this.away = { secs, tries };
        this.go('away');
      },
      back: () => this.view && this.show(this.view),
      gone: (status, error) => this.gone(status, error),
    });
    this.ui = newUi(() => this.render());
    this.main = h('main', { class: `g made m-${surface}`, 'data-surface': surface, 'aria-label': LABEL[surface] });
    root.replaceChildren(this.main);
    if (surface === 'screen') document.documentElement.classList.add('mtvpage');
  }

  async start(_boot?: unknown): Promise<void> {
    this.go('loading');
    let tries = 0;
    for (;;) {
      const r = this.token ? await this.api.call('host/signin', { token: this.token }) : await this.api.state();
      if (r.ok) {
        this.token = undefined;
        if (r.view) this.show(r.view);
        else {
          const s = await this.api.state();
          if (!s.ok) return this.gone(s.status, s.error);
          if (s.view) this.show(s.view);
        }
        this.live.start();
        return;
      }
      if (r.status === 401 && this.surface === 'phone') {
        // Not seated yet: the public `info` (the kit's lobby: title and words, no rows) names the join screen.
        const i = await this.api.info();
        if (i.ok && i.view) this.view = i.view;
        return this.go('join');
      }
      if (r.status !== 0 && r.status !== 429 && r.status < 500) return this.gone(r.status, r.error);
      this.away = { secs: backoff(tries), tries: ++tries };
      this.go('away');
      await new Promise<void>((res) => this.env.setTimeout(res, this.away.secs * 1000));
    }
  }

  wake(): void {
    if (this.screen === 'live' || this.screen === 'away') this.live.now();
  }

  fit(_w: number, _h: number): void {
    // The big screen scales with the viewport in made.css (vw units); nothing to measure.
  }

  private show(v: MadeView): void {
    this.view = v;
    if (v.now !== null) this.skew = v.now - this.env.now();
    this.screen = v.ended ? 'ended' : 'live';
    this.render();
    // One tick a second only while a timer runs; it rewrites the clock's text and nothing else.
    this.env.clearInterval(this.tick);
    this.tick = v.timer?.running ? this.env.setInterval(() => this.clock(), 1000) : 0;
  }

  private clock(): void {
    const t = this.view?.timer;
    const left = secondsLeft(t ?? null, this.env.now(), this.skew);
    for (const el of this.main.querySelectorAll('[data-timer]')) el.textContent = left === null ? '--:--' : clockText(left);
  }

  private gone(status: number, error?: string): void {
    this.live.stop();
    this.env.clearInterval(this.tick);
    if (status === 401 && this.surface === 'phone') return this.go('join');
    this.go(status === 410 || error === 'ended' ? 'ended' : status === 401 || status === 403 ? 'signed-out' : 'gone');
  }

  private go(s: Screen): void {
    this.screen = s;
    this.render();
  }

  private async join(e: Event): Promise<void> {
    e.preventDefault();
    const name = this.name.trim();
    if (!name) {
      this.joinErr = 'Type a first name to join.';
      return this.render();
    }
    this.busy = true;
    this.joinErr = '';
    this.render();
    const r = await this.api.join({ name, avatar: 'antenna' });
    this.busy = false;
    if (!r.ok) {
      this.joinErr = r.message ?? (r.status === 0 ? "Couldn't reach the box. Try again." : "Couldn't join. Try again.");
      return this.render();
    }
    const s = r.view ? r : await this.api.state();
    if (s.ok && s.view) this.show(s.view);
    this.live.start();
  }

  private send = async (name: string, body: Record<string, unknown>): Promise<string | null> => {
    const r = this.surface === 'remote' ? await this.api.call('host/action', { name, body }) : await this.api.act(name, body);
    if (r.ok) {
      if (r.view) this.show(r.view);
      return null;
    }
    if (r.status === 0) return "Couldn't reach the box. Try again.";
    return r.message ?? 'That didn’t go through. Try again.';
  };

  private bar(): HTMLElement {
    const v = this.view;
    return h('header', { class: 'bar' }, creature('antenna', 'mface'), h('b', null, v?.title ?? LABEL[this.surface]), h('span', { class: 'sp' }), this.surface === 'phone' ? null : h('span', { class: 'muted' }, LABEL[this.surface]));
  }

  private calm(title: string, lead: string, extra?: HTMLElement): HTMLElement {
    return h('div', { class: 'body mid' }, creature('antenna', 'mface xl'), h('h2', null, title), h('p', { class: 'lead' }, lead), extra ?? null);
  }

  render(): void {
    // Keep focus on the same control across a redraw (a push can land while someone types).
    const focus = (document.activeElement as HTMLElement | null)?.id;
    const v = this.view;
    let body: HTMLElement;
    if (this.screen === 'loading') body = this.calm('Opening…', 'One moment.');
    else if (this.screen === 'join')
      body = h('div', { class: 'body' },
        h('h1', null, v?.words.join ?? 'Join in'),
        h('form', { class: 'mform', novalidate: true, onsubmit: (e: Event) => void this.join(e) },
          h('div', null, h('label', { class: 'lbl', for: 'mname' }, 'Your first name'),
            h('input', { id: 'mname', class: 'input', maxlength: 20, autocomplete: 'given-name', value: this.name, 'aria-invalid': this.joinErr ? 'true' : null, 'aria-describedby': 'mname-err', oninput: (e: Event) => (this.name = (e.target as HTMLInputElement).value) })),
          h('button', { type: 'submit', class: 'gbtn', disabled: this.busy, 'aria-busy': this.busy ? 'true' : null }, 'Join')),
        h('p', { class: 'err', id: 'mname-err', role: 'alert' }, this.joinErr));
    else if (this.screen === 'away') body = this.calm('Lost the connection', `Trying again in ${this.away.secs} s.`, h('button', { type: 'button', class: 'gbtn ghost', onclick: () => this.wake() }, 'Try now'));
    else if (this.screen === 'ended') body = this.calm('This has ended', v?.words.done ?? 'Thanks for taking part!');
    else if (this.screen === 'signed-out')
      body = this.calm(this.surface === 'phone' ? 'Signed out' : 'This link was already used', this.surface === 'phone' ? 'Open the link again to come back.' : 'Each link opens one screen. Make a new one on the card in kwant.');
    else if (this.screen === 'gone') body = this.calm('Nothing here', 'Check the link with whoever sent it.');
    else {
      const ctx = { v: v!, surface: this.surface, send: this.send, ui: this.ui, now: this.env.now(), skew: this.skew };
      const blocks = drawBlocks(ctx);
      body = h('div', { class: 'body mblocks' }, this.surface === 'remote' ? stepBar(ctx) : null, ...(blocks.length ? blocks : [h('p', { class: 'muted center' }, 'Nothing to show here yet.')]));
    }
    this.main.dataset.screen = this.screen;
    this.main.replaceChildren(this.bar(), body);
    document.title = v?.title ?? LABEL[this.surface];
    if (focus) document.getElementById(focus)?.focus();
  }
}
