// The Queue ticket's look: the kit's pixel icons for this page (from Mira's queue-shots/src/gen.py, drawn from rows of
// '#' so the chunk stays small), the customer's creature (picked from the ticket number, so no data is needed) and the
// ticket favicon in three tints, read from the page's own tokens (no colour is written here).
import { AVATARS, creature, svg } from '../dom';
import type { Rects } from '../art';

const ROWS = {
  ticket: ['#######', '##.####', '.#.###.', '.#.###.', '##.####', '#######'],
  lock: ['..###..', '.#...#.', '.#...#.', '#######', '###.###', '###.###', '#######'],
  clock: ['.#####.', '#..#..#', '#..#..#', '#..###.', '#.....#', '.#####.'],
  warn: ['...#...', '..###..', '..#.#..', '.##.##.', '.#####.', '##.#.##', '#######'],
  bell: ['...#...', '..###..', '.#####.', '.#####.', '#######', '.......', '#######'],
  speaker: ['...#...', '..##.#.', '####..#', '####..#', '..##.#.', '...#...'],
  buzz: ['#.###.#', '#.#.#.#', '..###..', '#.###.#', '#.###.#', '..###..'],
  screen: ['#######', '#.....#', '#.....#', '#.....#', '#######', '...#...', '.#####.'],
  tab: ['###....', '#######', '#.....#', '#.....#', '#######'],
  phone: ['.###.', '#...#', '#...#', '#...#', '#...#', '#.#.#', '.###.'],
  pause: ['##..##', '##..##', '##..##', '##..##', '##..##', '##..##'],
  check: ['.......##', '......##.', '.....##..', '##..##...', '.####....', '..##.....'],
  nowifi: ['#..###.', '.#....#', '#.#.#..', '...#...', '..#.#..', '...#..#'],
};

export type QIcon = keyof typeof ROWS;

/** Rows of '#' as rects in currentColor: one rect per run. */
export function rects(rows: readonly string[]): Rects {
  const out: Rects = [];
  rows.forEach((row, y) => {
    for (const m of row.matchAll(/#+/g)) out.push([m.index ?? 0, y, m[0].length, 1, -1]);
  });
  return out;
}

const box = (rows: readonly string[]): string => `0 0 ${Math.max(...rows.map((r) => r.length))} ${rows.length}`;

export const qIcon = (name: QIcon, cls = 'pi'): SVGSVGElement => svg(box(ROWS[name]), rects(ROWS[name]), cls);

/** The customer's creature: one of the guest page's eight, by ticket number. Decorative (the number says who). */
export const ticketCreature = (n: number): SVGSVGElement => creature(AVATARS[n % AVATARS.length], 'tkc');

export type Tint = 'grey' | 'violet' | 'bright';

/** The ticket icon as an SVG data URL in one tint, its colour read from the page's own tokens. */
export function favicon(tint: Tint, read: (token: string) => string): string {
  const token = tint === 'grey' ? '--zinc-400' : tint === 'violet' ? '--sky-500' : '--sky-300';
  const fill = read(token).trim() || 'currentColor';
  const r = rects(ROWS.ticket)
    .map(([x, y, w, hh]) => `<rect x="${x}" y="${y + 0.5}" width="${w}" height="${hh}"/>`)
    .join('');
  // Only our own numbers and a CSS colour go in: nothing from the server.
  return `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 7 7" shape-rendering="crispEdges" fill="${fill}">${r}</svg>`)}`;
}
