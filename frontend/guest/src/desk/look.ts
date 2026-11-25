// The front desk's pixel art: Bell, the real ModernAvatar in its desk style (frame and the thinking/finished badges
// shared with the Quizmaster in art.ts, body and the failed badge from bell-art.ts), and the kit's pixel icons.
import { MOOD_FINISHED, MOOD_THINKING, QM_FRAME } from '../art';
import { h, paint, svg } from '../dom';
import { BL_BODY, BL_BODY_SMALL, BL_PALETTE, BL_SMALL_TRANSFORM, DESK_ICONS, MOOD_FAILED } from './bell-art';

const NS = 'http://www.w3.org/2000/svg';

export type Mood = 'thinking' | 'finished' | 'failed' | undefined;

/**
 * Bell at 28 64 96 px, with its mood badge. `label` names it for screen readers; without one it is decorative
 * (always next to words that say the same).
 */
export function bell(size: 28 | 64 | 96, mood?: Mood, label?: string): HTMLElement {
  const small = size < 48;
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 26 26');
  s.setAttribute('class', 'px');
  s.setAttribute('aria-hidden', 'true');
  s.append(paint(QM_FRAME));
  const body = paint(small ? BL_BODY_SMALL : BL_BODY, 'qm-body', BL_PALETTE);
  if (small) {
    const wrap = document.createElementNS(NS, 'g');
    wrap.setAttribute('transform', BL_SMALL_TRANSFORM);
    wrap.append(body);
    s.append(wrap);
  } else s.append(body);
  const el = h('div', label ? { class: `qm qm${size}`, role: 'img', 'aria-label': label } : { class: `qm qm${size}`, 'aria-hidden': 'true' }, s);
  if (mood) {
    const rects = mood === 'thinking' ? MOOD_THINKING : mood === 'finished' ? MOOD_FINISHED : MOOD_FAILED;
    el.append(svg('0 0 7 7', rects, `mood mood-${mood} px`, undefined, mood === 'failed' ? BL_PALETTE : undefined));
  }
  return el;
}

export type DeskIcon = keyof typeof DESK_ICONS;
export const dIcon = (name: DeskIcon, cls = 'pi'): SVGSVGElement => svg(DESK_ICONS[name][0], DESK_ICONS[name][1], cls);
