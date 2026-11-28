// pa12 P2: kwant's ready-made blocks for "Make your own" pocket apps (path (b), pa12/plan.md section 3). A bot
// describes its screens as data (`screens.<surface>.blocks`) and this file draws them on the phone, the big screen and
// the host remote. No bot-written code runs here. Text goes in through text nodes only (dom.ts `h`), nothing sets a
// style attribute, and nothing works out a number: counts, tallies, scores and the timer's end all come from the box.
// The shapes are in autonomy/pocket-apps/pa12/P2/blocks.md, the contract for Lin's validator and Vega's builder.
import { creature, h, Kid } from '../dom';
import { Block, clock, Collection, Field, MadeView, secondsLeft, Surface, Value, who } from './view';

/** Sends one action; resolves to an error sentence to show by the block, or null when the box took it. */
export type Send = (name: string, body: Record<string, unknown>) => Promise<string | null>;

/** What the page keeps between draws: typed drafts, which block is waiting on the box, the last refusal per block. */
export interface Ui {
  drafts: Map<string, string>;
  busy: Set<string>;
  errors: Map<string, string>;
  redraw: () => void;
}
export const newUi = (redraw: () => void): Ui => ({ drafts: new Map(), busy: new Set(), errors: new Map(), redraw });

export interface Ctx {
  v: MadeView;
  surface: Surface;
  send: Send;
  ui: Ui;
  /** This device's clock and the box's lead on it, for the countdown. */
  now: number;
  skew: number;
}

const plural = (n: number, noun: [string, string]): string => `${n.toLocaleString(undefined)} ${n === 1 ? noun[0] : noun[1]}`;
const shown = (x: Value | undefined): string => (x === undefined ? '' : typeof x === 'boolean' ? (x ? 'Yes' : 'No') : String(x));
const can = (c: Ctx, action: string) => c.surface === 'remote' && c.v.can.includes(action);

async function run(c: Ctx, key: string, name: string, body: Record<string, unknown>, after?: () => void): Promise<void> {
  if (c.ui.busy.has(key)) return;
  c.ui.busy.add(key);
  c.ui.errors.delete(key);
  c.ui.redraw();
  const err = await c.send(name, body);
  c.ui.busy.delete(key);
  if (err) c.ui.errors.set(key, err);
  else after?.();
  c.ui.redraw();
}

const btn = (c: Ctx, key: string, label: string, onclick: () => void, cls = 'gbtn', extra: Record<string, string | boolean> = {}) =>
  h('button', { type: 'button', class: cls, disabled: c.ui.busy.has(key), 'aria-busy': c.ui.busy.has(key) ? 'true' : null, onclick, ...extra }, label);
const errLine = (c: Ctx, key: string) => h('p', { class: 'err', role: 'alert', id: `${key}-err` }, c.ui.errors.get(key) ?? '');

function face(c: Ctx, id: string | null | undefined, cls = 'mface'): Kid {
  const p = who(c.v, id);
  return p ? creature(p.avatar, cls) : null;
}
const nameOf = (c: Ctx, id: string | null | undefined): string => (id && id === c.v.me ? 'You' : who(c.v, id)?.name ?? 'Someone');

function card(type: string, title: string | null, ...kids: Kid[]): HTMLElement {
  return h('section', { class: `mb mb-${type}`, 'data-block': type }, title ? h('h2', { class: 'mbh' }, title) : null, ...kids);
}

function say(c: Ctx, b: Extract<Block, { type: 'say' }>): HTMLElement | null {
  const text = b.text ?? c.v.step?.say;
  return text ? h('section', { class: `mb mb-say${b.big ? ' big' : ''}`, 'data-block': 'say' }, h('p', null, text)) : null;
}

function input(c: Ctx, key: string, f: Field, locked: boolean): Kid {
  const id = `${key}-${f.name}`;
  const draft = c.ui.drafts.get(id) ?? '';
  const keep = (e: Event) => c.ui.drafts.set(id, (e.target as HTMLInputElement).value);
  const common = { id, name: f.name, disabled: locked || c.ui.busy.has(key), 'aria-describedby': `${key}-err` };
  if (f.type === 'choice')
    return h('fieldset', { class: 'mchoice' }, h('legend', { class: 'lbl' }, f.label),
      ...(f.of ?? []).map((o, i) => h('label', { class: `mopt${draft === o ? ' on' : ''}` },
        h('input', { type: 'radio', ...common, id: `${id}-${i}`, value: o, checked: draft === o, onchange: (e: Event) => (keep(e), c.ui.redraw()) }), h('span', null, o))));
  if (f.type === 'bool')
    return h('label', { class: 'mopt mbool' }, h('input', { type: 'checkbox', ...common, checked: draft === 'true', onchange: (e: Event) => c.ui.drafts.set(id, String((e.target as HTMLInputElement).checked)) }), h('span', null, f.label));
  return h('div', null, h('label', { class: 'lbl', for: id }, f.label, f.type === 'text' && f.max ? h('small', null, `up to ${f.max}`) : null),
    h('input', { class: 'input', ...common, type: f.type === 'int' ? 'number' : 'text', inputmode: f.type === 'int' ? 'numeric' : null, value: draft, maxlength: f.type === 'text' ? f.max ?? null : null, min: f.min ?? null, max: f.type === 'int' ? f.max ?? null : null, autocomplete: 'off', oninput: keep }));
}

function form(c: Ctx, b: Extract<Block, { type: 'form' }>, i: number): HTMLElement | null {
  const col = c.v.collections[b.into];
  const key = `b${i}`;
  if (c.surface === 'screen') return null;
  if (c.surface === 'remote') {
    // The host doesn't fill guests' forms: the remote shows how many came in and the lock.
    const locked = col?.locked === true;
    return card('form', b.label,
      h('p', { class: 'mcount' }, plural(col?.count ?? 0, b.noun), ' in', locked ? ' · locked' : ''),
      can(c, locked ? 'unlock' : 'lock') ? btn(c, key, locked ? 'Unlock' : 'Lock', () => void run(c, key, locked ? 'unlock' : 'lock', { collection: b.into }), 'gbtn ghost') : null,
      errLine(c, key));
  }
  const locked = !col || col.locked;
  const mine = col?.rows?.filter((r) => r.mine) ?? [];
  const submit = (e: Event) => {
    e.preventDefault();
    const vals: Record<string, Value> = {};
    for (const f of b.fields) {
      const d = c.ui.drafts.get(`${key}-${f.name}`) ?? '';
      if (f.type === 'int') {
        if (d.trim() === '') continue;
        vals[f.name] = Number(d);
      } else if (f.type === 'bool') vals[f.name] = d === 'true';
      else if (d.trim()) vals[f.name] = d.trim();
    }
    if (!Object.keys(vals).length) {
      c.ui.errors.set(key, 'Fill in at least one box first.');
      return c.ui.redraw();
    }
    void run(c, key, b.action, { collection: b.into, values: vals }, () => b.fields.forEach((f) => c.ui.drafts.delete(`${key}-${f.name}`)));
  };
  return card('form', b.label,
    h('form', { class: 'mform', novalidate: true, onsubmit: submit },
      ...b.fields.map((f) => input(c, key, f, locked)),
      h('button', { type: 'submit', class: 'gbtn', disabled: locked || c.ui.busy.has(key), 'aria-busy': c.ui.busy.has(key) ? 'true' : null }, locked ? 'Closed for now' : b.submit)),
    mine.length ? h('p', { class: 'mdone', role: 'status' }, b.done, ' ', h('span', { class: 'soft' }, b.fields.map((f) => shown(mine[mine.length - 1].values[f.name])).filter(Boolean).join(' · '))) : null,
    errLine(c, key));
}

function countOnly(col: Collection | undefined, noun: [string, string]): HTMLElement {
  return h('p', { class: 'mcount' }, plural(col?.count ?? 0, noun), ' in');
}

function list(c: Ctx, b: Extract<Block, { type: 'list' }>, i: number): HTMLElement {
  const col = c.v.collections[b.from];
  const key = `b${i}`;
  const title = b.label;
  if (!col?.rows) return card('list', title, countOnly(col, b.noun));
  if (!col.rows.length) return card('list', title, h('p', { class: 'mempty muted' }, b.empty));
  const more = col.count - col.rows.length;
  return card('list', title,
    h('ul', { class: 'mlist' }, ...col.rows.map((r) =>
      h('li', { class: `mrow${r.mine ? ' me' : ''}` },
        face(c, r.owner),
        h('span', { class: 'mtext' }, h('b', null, shown(r.values[b.show[0]])), ...b.show.slice(1).map((s) => (r.values[s] !== undefined ? h('span', { class: 'muted' }, ' · ', shown(r.values[s])) : null))),
        r.mine && c.surface === 'phone' ? h('span', { class: 'mtag' }, 'You') : null,
        r.mine && b.take_back && c.surface === 'phone' && !col.locked ? btn(c, `${key}-${r.id}`, 'Take back', () => void run(c, `${key}-${r.id}`, 'take_back', { collection: b.from, id: r.id }), 'gbtn ghost msm', { 'aria-label': `Take back ${shown(r.values[b.show[0]])}` }) : null))),
    more > 0 ? h('p', { class: 'small' }, `and ${plural(more, b.noun)} more`) : null,
    errLine(c, key));
}

/** One bar: an SVG rect whose width is this row's share of the ballots. The number beside it is the box's. */
function bar(n: number, total: number): SVGSVGElement {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 100 6');
  s.setAttribute('preserveAspectRatio', 'none');
  s.setAttribute('class', 'mbar');
  s.setAttribute('aria-hidden', 'true');
  const r = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
  r.setAttribute('width', total > 0 ? String((n / total) * 100) : '0');
  r.setAttribute('height', '6');
  r.setAttribute('rx', '3');
  s.append(r);
  return s;
}

function board(c: Ctx, b: Extract<Block, { type: 'board' }>, i: number): HTMLElement {
  const from = c.v.collections[b.from];
  const votes = c.v.collections[b.vote];
  const key = `b${i}`;
  const title = b.label;
  if (!from?.rows) return card('board', title, countOnly(from, b.noun));
  if (!from.rows.length) return card('board', title, h('p', { class: 'mempty muted' }, b.empty));
  const tally = votes?.tally;
  const total = tally ? Object.values(tally).reduce((a, n) => a + n, 0) : 0;
  const myPick = votes?.rows?.find((r) => r.mine)?.values.pick;
  const open = c.surface === 'phone' && !!votes && !votes.locked;
  return card('board', title,
    h('ul', { class: 'mboard', role: open ? 'group' : null, 'aria-label': open ? (title ? `Vote: ${title}` : 'Vote') : null }, ...from.rows.map((r) => {
      const label = shown(r.values[b.show]);
      const n = tally?.[r.id];
      const picked = myPick === r.id;
      const body: Kid[] = [face(c, r.owner), h('span', { class: 'mtext' }, h('b', null, label)), tally ? h('span', { class: 'mn tab', 'aria-label': plural(n ?? 0, ['vote', 'votes']) }, String(n ?? 0)) : null];
      return h('li', { class: `mopt-row${picked ? ' on' : ''}` },
        open
          ? h('button', { type: 'button', class: 'mvote', 'aria-pressed': picked ? 'true' : 'false', disabled: c.ui.busy.has(key), onclick: () => void run(c, key, 'vote', { collection: b.vote, value: r.id }) }, ...body)
          : h('div', { class: 'mvote' }, ...body),
        tally ? bar(n ?? 0, total) : null);
    })),
    // With no tally the counts stay hidden. A phone holds only its own ballot, so it can't say how many are in.
    !tally && votes ? h('p', { class: 'small' }, votes.rows ? 'The counts are hidden for now.' : votes.count === 0 ? 'No votes yet.' : votes.count === 1 ? '1 vote in.' : `${plural(votes.count, ['vote', 'votes'])} in. The counts are hidden for now.`) : null,
    open ? h('p', { class: 'small' }, myPick ? 'Tap another one to change your vote.' : 'Tap one to vote.') : null,
    c.surface === 'phone' && votes?.locked ? h('p', { class: 'small' }, 'Voting is closed.') : null,
    errLine(c, key));
}

function timer(c: Ctx, b: Extract<Block, { type: 'timer' }>, i: number): HTMLElement {
  const t = c.v.timer;
  const key = `b${i}`;
  const left = secondsLeft(t, c.now, c.skew);
  return card('timer', null,
    h('p', { class: 'mlbl' }, b.label),
    h('p', { class: `mclock tab${t?.running ? ' run' : ''}`, role: 'timer', 'aria-live': 'off', 'data-timer': t?.running ? '1' : null }, left === null ? '--:--' : clock(left)),
    !t ? h('p', { class: 'small' }, 'No timer running.') : null,
    can(c, 'timer_start') ? btn(c, key, 'Start', () => void run(c, key, 'timer_start', {})) : null,
    can(c, 'timer_stop') ? btn(c, key, 'Stop', () => void run(c, key, 'timer_stop', {}), 'gbtn ghost') : null,
    errLine(c, key));
}

function scores(c: Ctx, b: Extract<Block, { type: 'scores' }>): HTMLElement {
  const s = c.v.scores;
  if (!s?.length) return card('scores', 'Scores', h('p', { class: 'mempty muted' }, 'No points yet.'));
  return card('scores', 'Scores',
    h('ol', { class: 'mscores' }, ...s.slice(0, b.top).map((r, i) =>
      h('li', { class: r.player === c.v.me ? 'me' : null }, h('span', { class: 'mrank tab' }, String(r.rank ?? i + 1)), face(c, r.player), h('span', { class: 'mtext' }, nameOf(c, r.player)), h('b', { class: 'tab' }, r.points.toLocaleString(undefined))))));
}

function turn(c: Ctx, i: number): HTMLElement {
  const t = c.v.turn;
  const key = `b${i}`;
  const mine = !!t && t.player === c.v.me && c.surface === 'phone';
  return card('turn', null,
    t ? h('div', { class: `mturn${mine ? ' me' : ''}` }, face(c, t.player, 'mface lg'), h('p', { class: 'mbig' }, mine ? 'Your turn' : `${who(c.v, t.player)?.name ?? 'Someone'}'s turn`)) : h('p', { class: 'muted' }, 'Turns start soon.'),
    t?.next ? h('p', { class: 'small' }, 'Then ', nameOf(c, t.next)) : null,
    can(c, 'turn_next') ? btn(c, `${key}n`, 'Next turn', () => void run(c, `${key}n`, 'turn_next', {})) : null,
    can(c, 'turn_skip') ? btn(c, `${key}s`, 'Skip', () => void run(c, `${key}s`, 'turn_skip', {}), 'gbtn ghost') : null,
    errLine(c, `${key}n`),
    errLine(c, `${key}s`));
}

function reveal(c: Ctx, b: Extract<Block, { type: 'reveal' }>, i: number): HTMLElement {
  const items = c.v.revealed[b.secret];
  const key = `b${i}`;
  return card('reveal', b.label,
    items ? h('ol', { class: 'mreveal' }, ...items.map((x) => h('li', null, shown(x[b.show])))) : h('p', { class: 'mhidden muted' }, b.hidden ?? (c.surface === 'remote' ? 'Hidden until you press Reveal.' : 'Revealed at the end.')),
    !items && can(c, 'reveal') ? btn(c, key, 'Reveal', () => void run(c, key, 'reveal', {})) : null,
    errLine(c, key));
}

/** Every block of this surface, in the spec's order. A block that has nothing to say on this surface is left out. */
export function drawBlocks(c: Ctx): HTMLElement[] {
  const out: HTMLElement[] = [];
  c.v.blocks.forEach((b, i) => {
    const el =
      b.type === 'say' ? say(c, b)
      : b.type === 'form' ? form(c, b, i)
      : b.type === 'list' ? list(c, b, i)
      : b.type === 'board' ? board(c, b, i)
      : b.type === 'timer' ? timer(c, b, i)
      : b.type === 'scores' ? scores(c, b)
      : b.type === 'turn' ? turn(c, i)
      : reveal(c, b, i);
    if (el) out.push(el);
  });
  return out;
}

/** The remote's step bar: where the host is in `steps[]`, the step's line, and Next step when the box allows it. */
export function stepBar(c: Ctx): HTMLElement | null {
  const s = c.v.step;
  if (!s) return null;
  return h('section', { class: 'mb mstep', 'data-block': 'step' },
    h('p', { class: 'small' }, `Step ${s.index + 1} of ${s.of}`),
    s.say ? h('p', { class: 'mbig' }, s.say) : null,
    can(c, 'step') ? btn(c, 'step', 'Next step', () => void run(c, 'step', 'step', {})) : null,
    errLine(c, 'step'));
}
