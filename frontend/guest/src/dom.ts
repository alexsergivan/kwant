// Tiny DOM helpers. Text always goes in through textContent / text nodes: nothing from the server is ever parsed as
// HTML, and nothing here writes a style attribute (the pocket CSP has no 'unsafe-inline').
import { CREATURES, CREATURE_VIEWBOX, ICONS, MOOD_FINISHED, MOOD_THINKING, PALETTE, QM_BODY, QM_BODY_SMALL, QM_FRAME, QM_SMALL_TRANSFORM, Rects } from './art';

export type Kid = Node | string | number | null | undefined | false | Kid[];
type Props = Record<string, string | number | boolean | EventListener | null | undefined>;

export function h(tag: string, props?: Props | null, ...kids: Kid[]): HTMLElement {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (typeof v === 'function') el.addEventListener(k.slice(2), v);
      else if (k === 'class') el.className = String(v);
      else el.setAttribute(k, v === true ? '' : String(v));
    }
  }
  add(el, kids);
  return el;
}

export function add(el: Element, kids: Kid[]): void {
  for (const k of kids) {
    if (k == null || k === false) continue;
    if (Array.isArray(k)) add(el, k);
    else el.append(typeof k === 'object' ? k : document.createTextNode(String(k)));
  }
}

const NS = 'http://www.w3.org/2000/svg';

export function svg(viewBox: string, rects: Rects, cls: string, label?: string): SVGSVGElement {
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', viewBox);
  s.setAttribute('class', cls);
  s.setAttribute('shape-rendering', 'crispEdges');
  if (label) {
    s.setAttribute('role', 'img');
    s.setAttribute('aria-label', label);
  } else s.setAttribute('aria-hidden', 'true');
  s.append(paint(rects));
  return s;
}

function paint(rects: Rects, cls?: string): SVGGElement {
  const g = document.createElementNS(NS, 'g');
  if (cls) g.setAttribute('class', cls);
  for (const [x, y, w, hh, c] of rects) {
    const r = document.createElementNS(NS, 'rect');
    r.setAttribute('x', String(x));
    r.setAttribute('y', String(y));
    r.setAttribute('width', String(w));
    r.setAttribute('height', String(hh));
    if (c >= 0) r.setAttribute('fill', PALETTE[c]);
    g.append(r);
  }
  return g;
}

export const icon = (name: keyof typeof ICONS, cls = 'pi'): SVGSVGElement => svg(ICONS[name][0], ICONS[name][1], cls);

export const AVATARS = ['antenna', 'ears', 'visor', 'spikes', 'beret', 'goggles', 'bulb', 'horns'];

export function creature(avatar: string, cls: string, label?: string): SVGSVGElement {
  const i = Math.max(0, AVATARS.indexOf(avatar));
  return svg(CREATURE_VIEWBOX, CREATURES[i], `c px ${cls}`, label);
}

export type Mood = 'thinking' | 'finished' | undefined;

/** The Quizmaster: the real ModernAvatar export (quizmaster style, `host` accessory), framed. Sizes: 28 36 48 64 96. */
export function quizmaster(size: 28 | 36 | 48 | 64 | 96, mood?: Mood): HTMLElement {
  const small = size < 48;
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 26 26');
  s.setAttribute('class', 'px');
  s.setAttribute('aria-hidden', 'true');
  s.append(paint(QM_FRAME));
  const body = paint(small ? QM_BODY_SMALL : QM_BODY, 'qm-body');
  if (small) {
    const wrap = document.createElementNS(NS, 'g');
    wrap.setAttribute('transform', QM_SMALL_TRANSFORM);
    wrap.append(body);
    s.append(wrap);
  } else s.append(body);
  const title = mood === 'thinking' ? 'Quizmaster is thinking' : mood === 'finished' ? 'Quizmaster just finished' : 'Quizmaster';
  const el = h('div', { class: `qm qm${size}`, role: 'img', 'aria-label': title }, s);
  if (mood) el.append(svg('0 0 7 7', mood === 'thinking' ? MOOD_THINKING : MOOD_FINISHED, `mood mood-${mood} px`));
  return el;
}

export const fmt = (n: number): string => n.toLocaleString('en-US');
