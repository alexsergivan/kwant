// Tally's pixel art for the guest page: the real ModernAvatar Tally (frame and mood badges shared with the
// Quizmaster in art.ts, body from tally-art.ts, which tools/extract_tally.py makes from Mira's pa10b export).
import { MOOD_FINISHED, MOOD_THINKING, QM_FRAME } from '../art';
import { h, paint, svg } from '../dom';
import { GA_ICONS, TA_BODY, TA_BODY_SMALL, TA_PALETTE, TA_SMALL_TRANSFORM } from '../tally-art';

const NS = 'http://www.w3.org/2000/svg';

export type Mood = 'thinking' | 'finished' | undefined;

/** Tally at 28 36 48 64 96 px, with its mood badge. Decorative next to words that say the same, else labelled. */
export function tally(size: 28 | 36 | 48 | 64 | 96, mood?: Mood): HTMLElement {
  const small = size < 48;
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 26 26');
  s.setAttribute('class', 'px');
  s.setAttribute('aria-hidden', 'true');
  s.append(paint(QM_FRAME));
  const body = paint(small ? TA_BODY_SMALL : TA_BODY, 'qm-body', TA_PALETTE);
  if (small) {
    const wrap = document.createElementNS(NS, 'g');
    wrap.setAttribute('transform', TA_SMALL_TRANSFORM);
    wrap.append(body);
    s.append(wrap);
  } else s.append(body);
  const title = mood === 'thinking' ? 'Tally is thinking' : mood === 'finished' ? 'Tally is happy' : 'Tally';
  const el = h('div', { class: `qm qm${size}`, role: 'img', 'aria-label': title }, s);
  if (mood) el.append(svg('0 0 7 7', mood === 'thinking' ? MOOD_THINKING : MOOD_FINISHED, `mood mood-${mood} px`));
  return el;
}

export const gaIcon = (name: keyof typeof GA_ICONS, cls = 'pi'): SVGSVGElement => svg(GA_ICONS[name][0], GA_ICONS[name][1], cls);
