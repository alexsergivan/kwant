/** 36j + space-icons-2: the 24 space icons as 9x9 pixel glyphs (Mira's icons.py, copied as is).
 * '#' main tone (the space colour, lifted), 'o' detail (creature ink), '.' empty. Keyed by the stored emoji,
 * which stays the stored value. 📦 (the fallback) has a glyph too; custom emoji and old word values keep the emoji. */
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
  // space-icons-2 (Mira, Nov 25): 16 more, from autonomy/space-icons-2/icons.py
  '💻': {
    name: 'code',
    rows: [
      '.........',
      '.#######.',
      '.#o#ooo#.',
      '.#oo#oo#.',
      '.#o#ooo#.',
      '.#oo##o#.',
      '.#######.',
      '#########',
      '.........',
    ],
  },
  '📈': {
    name: 'chart',
    rows: [
      '.........',
      '......##.',
      '......##.',
      '...##.##.',
      '...##.##.',
      '##.##.##.',
      '##.##.##.',
      '#########',
      '.........',
    ],
  },
  '💰': {
    name: 'money',
    rows: [
      '..#...#..',
      '...###...',
      '....#....',
      '..##o##..',
      '.##oo###.',
      '.###oo##.',
      '.###o###.',
      '.#######.',
      '..#####..',
    ],
  },
  '🛒': {
    name: 'shop',
    rows: [
      '##.......',
      '.#.......',
      '.########',
      '.#o#o#o#.',
      '.#######.',
      '..######.',
      '..#......',
      '..######.',
      '..#...#..',
    ],
  },
  '✉️': {
    name: 'mail',
    rows: [
      '.........',
      '#########',
      '#o#####o#',
      '##o###o##',
      '###o#o###',
      '####o####',
      '#########',
      '#########',
      '.........',
    ],
  },
  '💬': {
    name: 'chat',
    rows: [
      '.........',
      '.#######.',
      '#########',
      '##o#o#o##',
      '#########',
      '.#######.',
      '..##.....',
      '.#.......',
      '.........',
    ],
  },
  '📦': {
    name: 'box',
    rows: [
      '.........',
      '.#######.',
      '####o####',
      '#ooooooo#',
      '.###o###.',
      '.###o###.',
      '.#######.',
      '.#######.',
      '.........',
    ],
  },
  '✏️': {
    name: 'writing',
    rows: [
      '......##.',
      '.....####',
      '....#o##.',
      '...####..',
      '..####...',
      '.####....',
      '.###.....',
      '#........',
      '.........',
    ],
  },
  '📷': {
    name: 'photo',
    rows: [
      '.........',
      '.###.....',
      '#########',
      '###ooo###',
      '##o#ooo##',
      '##ooooo##',
      '###ooo###',
      '#########',
      '.........',
    ],
  },
  '🎵': {
    name: 'music',
    rows: [
      '.........',
      '..#######',
      '..#######',
      '..#.....#',
      '..#.....#',
      '..#.....#',
      '###...###',
      '###...###',
      '.........',
    ],
  },
  '🎓': {
    name: 'school',
    rows: [
      '.........',
      '...###...',
      '#########',
      '...###..#',
      '..#####.#',
      '..#ooo#.#',
      '..#####.#',
      '.......##',
      '.........',
    ],
  },
  '❤️': {
    name: 'health',
    rows: [
      '.........',
      '.##...##.',
      '#########',
      '####o####',
      '###ooo###',
      '.###o###.',
      '..#####..',
      '...###...',
      '....#....',
    ],
  },
  '✈️': {
    name: 'travel',
    rows: [
      '....#....',
      '...#o#...',
      '...###...',
      '.#######.',
      '#########',
      '...###...',
      '...###...',
      '..#####..',
      '...#.#...',
    ],
  },
  '☕': {
    name: 'coffee',
    rows: [
      '..#..#...',
      '...#..#..',
      '.........',
      '#ooooo##.',
      '#######.#',
      '#######.#',
      '.#######.',
      '..####...',
      '.........',
    ],
  },
  '🐾': {
    name: 'pets',
    rows: [
      '..##.##..',
      '..##.##..',
      '##.....##',
      '##.###.##',
      '..#####..',
      '.#######.',
      '.#######.',
      '..##.##..',
      '.........',
    ],
  },
  '⭐': {
    name: 'star',
    rows: [
      '....#....',
      '....#....',
      '...###...',
      '#########',
      '.##o#o##.',
      '..#####..',
      '..##.##..',
      '.##...##.',
      '.#.....#.',
    ],
  },
};

/** A stored icon without spaces or U+FE0F (the emoji variation selector), so ✉️ and ✉ are the same icon. */
export const iconKey = (icon: string): string => (icon || '').trim().replace(/\uFE0F/g, '');

const BY_KEY: Record<string, SpacePixelIcon> = Object.fromEntries(Object.entries(SPACE_PIXEL_ICONS).map(([k, g]) => [iconKey(k), g]));

/** The glyph for a stored icon, or null to draw the emoji. Ignores spaces and U+FE0F. */
export function pixelIconFor(icon: string): SpacePixelIcon | null {
  return BY_KEY[iconKey(icon)] ?? null;
}

/** "Coffee": the picked icon's name for the picker's legend and button titles. */
export const iconTitle = (name: string): string => name.charAt(0).toUpperCase() + name.slice(1);

/** Glyph size for a tile: about half the tile, always a whole multiple of 9 so every pixel stays square. */
export function glyphPx(tile: number): number {
  return 9 * Math.max(1, Math.round((tile * 0.5) / 9));
}
