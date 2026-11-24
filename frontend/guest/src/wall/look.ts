// Snap, the Wall's character (design.md section 5): the real ModernAvatar export (snap style, pink, camera overlay),
// framed like every kwant bot. Frame and mood badges are shared with the Quizmaster (art.ts); only the body is Snap's.
import { MOOD_FINISHED, MOOD_THINKING, QM_FRAME } from '../art';
import { creature, AVATARS, h, paint, svg } from '../dom';
import { SN_BODY, SN_BODY_SMALL, SN_PALETTE, SN_SMALL_TRANSFORM, WALL_ICONS } from './snap-art';

const NS = 'http://www.w3.org/2000/svg';

/** idle: ready. thinking: the box is away. finished: closed, or the best photos are picked. */
export type Mood = 'thinking' | 'finished' | undefined;

/** Snap at 28 36 48 64 96 160 px, with its mood badge. Labelled, unless the words beside it say the same. */
export function snap(size: 28 | 36 | 48 | 64 | 96 | 160, mood?: Mood, decorative = false): HTMLElement {
  const small = size < 48;
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 26 26');
  s.setAttribute('class', 'px');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('shape-rendering', 'crispEdges');
  s.append(paint(QM_FRAME));
  const body = paint(small ? SN_BODY_SMALL : SN_BODY, 'qm-body', SN_PALETTE);
  if (small) {
    const wrap = document.createElementNS(NS, 'g');
    wrap.setAttribute('transform', SN_SMALL_TRANSFORM);
    wrap.append(body);
    s.append(wrap);
  } else s.append(body);
  const title = mood === 'thinking' ? 'Snap is waiting' : mood === 'finished' ? 'Snap is happy' : 'Snap';
  const el = h('div', decorative ? { class: `qm qm${size} snap`, 'aria-hidden': 'true' } : { class: `qm qm${size} snap`, role: 'img', 'aria-label': title }, s);
  if (mood) el.append(svg('0 0 7 7', mood === 'thinking' ? MOOD_THINKING : MOOD_FINISHED, `mood mood-${mood} px`));
  return el;
}

export const wIcon = (name: keyof typeof WALL_ICONS, cls = 'pi'): SVGSVGElement => svg(WALL_ICONS[name][0], WALL_ICONS[name][1], cls);

/** A guest's creature from the server's `creature` (0-7). */
export const guestCreature = (n: number, cls: string, label?: string): SVGSVGElement => creature(AVATARS[n % AVATARS.length], cls, label);
