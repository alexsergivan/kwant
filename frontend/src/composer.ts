// The chat composer on a phone (kwant 0.6.13, spec A1).

/**
 * Keyboard mode: on a phone with a touch screen, while the composer has focus. The header shrinks to
 * 48 px and the thread line, pinned instructions and hint row step aside, so what you answer stays in
 * view above the keyboard. Focus is the signal, not the visual viewport: with
 * interactive-widget=resizes-content (index.html) Android shrinks innerHeight along with the keyboard,
 * so a viewport comparison never fires there. A hardware keyboard compacts it too, which is fine.
 */
export function keyboardMode(s: { phone: boolean; touch: boolean; focused: boolean }): boolean {
  return s.phone && s.touch && s.focused;
}

/**
 * The composer is two rows on a phone (the text full width, the tools under it) while it's in use:
 * focused, or holding text or files. Empty and at rest it's one row, so the chat keeps the room.
 */
export function composerStacked(s: { phone: boolean; focused: boolean; hasContent: boolean }): boolean {
  return s.phone && (s.focused || s.hasContent);
}

/** px-3 on each side of the field. */
export const PLACEHOLDER_PAD = 24;

/**
 * The phone placeholder: "Message {name}" when it fits on one line, else "Message" (the header names
 * the chat). A placeholder never wraps: WebKit ignores text-overflow on a textarea placeholder, so a
 * long one would wrap and be cut mid-glyph. Unknown width (before layout): the short one.
 */
export function phonePlaceholder(name: string, fieldWidth: number, measure: (text: string) => number): string {
  return fitPlaceholder(`Message ${name}`, 'Message', fieldWidth, measure);
}

/** The long placeholder when it fits the field, else the short one. */
export function fitPlaceholder(long: string, short: string, fieldWidth: number, measure: (text: string) => number): string {
  if (!(fieldWidth > PLACEHOLDER_PAD)) return short;
  return measure(long) <= fieldWidth - PLACEHOLDER_PAD ? long : short;
}

let ctx: CanvasRenderingContext2D | null | undefined;
/** Width of a text at 16 px in the given font family (canvas), or an estimate where there's no canvas. */
export function measureText16(text: string, family: string): number {
  if (ctx === undefined) {
    try {
      ctx = typeof document === 'undefined' ? null : document.createElement('canvas').getContext('2d');
    } catch {
      ctx = null;
    }
  }
  if (!ctx) return text.length * 9;
  ctx.font = `16px ${family || 'Inter, system-ui, sans-serif'}`;
  return ctx.measureText(text).width;
}

// ------------------------------------------------------------------ composer-diet (0.9.50, Mira's simplify rank 1)

/** Set at the person's first sent message; after that the keyboard hint line under the field stays away. */
export const HINT_KEY = 'kwnt:composer-hint-done';

/** True once the person has sent a message from this browser (b). Storage that can't be read counts as not yet. */
export function hintLearned(store: Pick<Storage, 'getItem'> | null = safeStore()): boolean {
  try {
    return store?.getItem(HINT_KEY) === '1';
  } catch {
    return false;
  }
}

/** Record the first send (b). Private mode: the hint shows again next time, which is harmless. */
export function markHintLearned(store: Pick<Storage, 'setItem'> | null = safeStore()): void {
  try {
    store?.setItem(HINT_KEY, '1');
  } catch {
    /* ignore */
  }
}

/** The "@ mention · Enter send…" line: never on a touch phone, and only until the first sent message (b). */
export function hintLineShows(s: { phone: boolean; touch: boolean; learned: boolean }): boolean {
  return !(s.phone && s.touch) && !s.learned;
}

/**
 * Suggested chips (c): only in a near-empty chat, off the home screen, when something can run, and never while a
 * reply streams: they'd suggest a new task while one is running.
 */
export function suggestedShows(s: { messages: number; suggestions: number; showHome: boolean; canRun: boolean | null | undefined; streaming: boolean }): boolean {
  return s.messages <= 1 && s.suggestions > 0 && !s.showHome && s.canRun !== false && !s.streaming;
}

/**
 * The Answers chip's words (a): "Answers: <who>" and, in a team chat whose lead is known, "· <lead> leads".
 * `name` is the accessible name, so a screen reader hears who answers and that the button opens the list.
 */
export function answersChip(s: { who: string; lead?: string | null }): { lead: string | null; name: string } {
  const lead = s.lead?.trim() || null;
  return {
    lead,
    name: `Answers: ${s.who}${lead ? `, ${lead} leads` : ''}. Choose who answers`,
  };
}

function safeStore(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}
