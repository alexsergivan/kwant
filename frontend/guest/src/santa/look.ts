// Santa's pixel art for the guest page: the real ModernAvatar Santa (frame and mood badges shared with the
// Quizmaster in art.ts, body from santa-art.ts) and the gift box whose lid lifts on "Open it".
import { MOOD_FINISHED, MOOD_THINKING, QM_FRAME } from '../art';
import { h, paint, svg } from '../dom';
import { GIFT_BOX, GIFT_LID, SA_BODY, SA_BODY_SMALL, SA_ICONS, SA_PALETTE, SA_SMALL_TRANSFORM } from '../santa-art';

const NS = 'http://www.w3.org/2000/svg';

export type Mood = 'thinking' | 'finished' | undefined;

/** Santa at 28 36 48 64 96 px, with its mood badge. Decorative next to words that say the same, else labelled. */
export function santa(size: 28 | 36 | 48 | 64 | 96, mood?: Mood): HTMLElement {
  const small = size < 48;
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 26 26');
  s.setAttribute('class', 'px');
  s.setAttribute('aria-hidden', 'true');
  s.append(paint(QM_FRAME));
  const body = paint(small ? SA_BODY_SMALL : SA_BODY, 'qm-body', SA_PALETTE);
  if (small) {
    const wrap = document.createElementNS(NS, 'g');
    wrap.setAttribute('transform', SA_SMALL_TRANSFORM);
    wrap.append(body);
    s.append(wrap);
  } else s.append(body);
  const title = mood === 'thinking' ? 'Santa is thinking' : mood === 'finished' ? 'Santa is happy' : 'Santa';
  const el = h('div', { class: `qm qm${size}`, role: 'img', 'aria-label': title }, s);
  if (mood) el.append(svg('0 0 7 7', mood === 'thinking' ? MOOD_THINKING : MOOD_FINISHED, `mood mood-${mood} px`));
  return el;
}

/** The wrapped gift (G5). `.open` lifts the lid (CSS, static under reduced motion). */
export function giftbox(): SVGSVGElement {
  const s = svg('0 0 20 18', GIFT_BOX, 'giftbox px', 'A wrapped gift', SA_PALETTE);
  s.append(paint(GIFT_LID, 'lid', SA_PALETTE));
  return s;
}

export const saIcon = (name: keyof typeof SA_ICONS, cls = 'pi'): SVGSVGElement => svg(SA_ICONS[name][0], SA_ICONS[name][1], cls);
