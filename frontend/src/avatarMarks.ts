// small-avatar-badges (autonomy/small-avatar-badges/design.md, Mira, Nov 26 2026): under 36 px no corner mark may touch
// the face. Each mark shrinks to its small size and sits `out` px past the avatar's edge on both axes. 36 px and up: unchanged.
export const SMALL_MARKS = 36;
export type MarkKind = 'mood' | 'status' | 'model' | 'hat';
/** Small size (px) and how far past the avatar's edge (px, both axes) a mark sits, or null at 36 px and up. */
export function smallMark(size: number, kind: MarkKind): { px: number; out: number } | null {
  if (size >= SMALL_MARKS) return null;
  switch (kind) {
    case 'mood': return { px: 10, out: 4 }; // the 5x5 glyph at 2 px cells on its own ink square (no 1-cell pad)
    case 'status': return { px: 8, out: 1 };
    case 'model': return { px: 12, out: size < 28 ? 5 : 4 };
    case 'hat': return { px: 12, out: 4 };
  }
}
