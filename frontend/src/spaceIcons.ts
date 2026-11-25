/** 36j: the 8 space icons as 9x9 pixel glyphs (Mira, ux-a11y-1124/space-icons/icons.py, copied as is).
 * '#' main tone (the space colour, lifted), 'o' detail (creature ink), '.' empty. Keyed by the stored emoji,
 * which stays the stored value: 📦, custom emoji and old word values keep drawing as the emoji. */
export interface SpacePixelIcon {
  name: string;
  rows: readonly string[];
}

export const SPACE_PIXEL_ICONS: Record<string, SpacePixelIcon> = {
  '💼': {
    name: 'work',
    rows: [
      '.........',
      '...###...',
      '...#.#...',
      '#########',
      '#########',
      '####o####',
      '#########',
      '#########',
      '.........',
    ],
  },
  '🌿': {
    name: 'nature',
    rows: [
      '##.....##',
      '###...###',
      '#o##.##o#',
      '.###.###.',
      '...###...',
      '....#....',
      '....#....',
      '..#####..',
      '.........',
    ],
  },
  '📚': {
    name: 'books',
    rows: [
      '.........',
      '##.....##',
      '####.####',
      '#oo#.#oo#',
      '####.####',
      '#oo#.#oo#',
      '####.####',
      '#########',
      '...#.#...',
    ],
  },
  '🎮': {
    name: 'games',
    rows: [
      '.........',
      '.........',
      '.#######.',
      '##o###o##',
      '#ooo#o#o#',
      '##o###o##',
      '####.####',
      '.##...##.',
      '.........',
    ],
  },
  '🏠': {
    name: 'home',
    rows: [
      '....#....',
      '...###...',
      '..#####..',
      '.#######.',
      '#########',
      '.#o###o#.',
      '.###.###.',
      '.###.###.',
      '.........',
    ],
  },
  '🚀': {
    name: 'rocket',
    rows: [
      '....#....',
      '...###...',
      '...###...',
      '...#o#...',
      '...###...',
      '..#####..',
      '.##.#.##.',
      '.#..#..#.',
      '....#....',
    ],
  },
  '🎨': {
    name: 'art',
    rows: [
      '.........',
      '..#####..',
      '.#o##o##.',
      '#########',
      '#o###..##',
      '#####..##',
      '.#######.',
      '..#####..',
      '.........',
    ],
  },
  '🧪': {
    name: 'lab',
    rows: [
      '..#####..',
      '...#.#...',
      '...#.#...',
      '..#...#..',
      '.#.....#.',
      '#########',
      '##o###o##',
      '#########',
      '.#######.',
    ],
  },
};

/** The glyph for a stored icon, or null to draw the emoji. Ignores spaces and U+FE0F (the emoji variation selector). */
export function pixelIconFor(icon: string): SpacePixelIcon | null {
  const key = (icon || '').trim().replace(/\uFE0F/g, '');
  return SPACE_PIXEL_ICONS[key] ?? null;
}

/** Glyph size for a tile: about half the tile, always a whole multiple of 9 so every pixel stays square. */
export function glyphPx(tile: number): number {
  return 9 * Math.max(1, Math.round((tile * 0.5) / 9));
}
