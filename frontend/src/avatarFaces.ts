// Living avatars (autonomy/living-avatars/design.md, Mira, Nov 26 2026): the face table, ported as-is from
// design/tools/faces.ts. Cells are [x, y, w, h, colour role] on the avatar's 26-grid. Colour roles resolve per
// creature: ink #1b1033, white #f5f2ff, body/shade/light from the palette, blush #ff6b81 (the hexes ModernAvatar
// already draws). A face replaces the 7 face cells (e1-e4, m1-m3) and nothing else on the creature.
import type { Creature } from './components/ModernAvatar';

export type Role = 'ink' | 'white' | 'body' | 'shade' | 'light' | 'blush';
export type Cell = [number, number, number, number, Role];
export type Eyes = 'open' | 'closed' | 'glanceL' | 'glanceR' | 'up' | 'focus' | 'happy' | 'half' | 'sad' | 'wide';
export type Mouth = 'smile' | 'grin' | 'flat' | 'focus' | 'frown' | 'o' | 'none';
export type MoodName = 'idle' | 'thinking' | 'working' | 'finished' | 'failed' | 'needs_you' | 'resting';

export const EYES: Record<Eyes, Cell[]> = {
  open: [[10, 10, 2, 3, 'ink'], [10, 10, 1, 1, 'white'], [14, 10, 2, 3, 'ink'], [14, 10, 1, 1, 'white']],
  closed: [[10, 12, 2, 1, 'shade'], [14, 12, 2, 1, 'shade']],
  glanceL: [[9, 10, 2, 3, 'ink'], [9, 10, 1, 1, 'white'], [13, 10, 2, 3, 'ink'], [13, 10, 1, 1, 'white']],
  glanceR: [[11, 10, 2, 3, 'ink'], [11, 10, 1, 1, 'white'], [15, 10, 2, 3, 'ink'], [15, 10, 1, 1, 'white']],
  up: [[11, 9, 2, 3, 'ink'], [11, 9, 1, 1, 'white'], [15, 9, 2, 3, 'ink'], [15, 9, 1, 1, 'white']],
  focus: [[10, 11, 2, 2, 'ink'], [10, 11, 1, 1, 'white'], [14, 11, 2, 2, 'ink'], [14, 11, 1, 1, 'white'], [10, 10, 2, 1, 'shade'], [14, 10, 2, 1, 'shade']],
  happy: [[9, 11, 1, 1, 'ink'], [10, 10, 1, 1, 'ink'], [11, 11, 1, 1, 'ink'], [14, 11, 1, 1, 'ink'], [15, 10, 1, 1, 'ink'], [16, 11, 1, 1, 'ink']],
  half: [[10, 10, 2, 1, 'shade'], [10, 11, 2, 2, 'ink'], [14, 10, 2, 1, 'shade'], [14, 11, 2, 2, 'ink']],
  sad: [[10, 11, 2, 2, 'ink'], [11, 11, 1, 1, 'white'], [14, 11, 2, 2, 'ink'], [14, 11, 1, 1, 'white'], [10, 9, 1, 1, 'shade'], [11, 8, 1, 1, 'shade'], [14, 8, 1, 1, 'shade'], [15, 9, 1, 1, 'shade']],
  wide: [[10, 9, 2, 4, 'ink'], [10, 9, 1, 2, 'white'], [14, 9, 2, 4, 'ink'], [14, 9, 1, 2, 'white']],
};

/** Glasses (x9/x16 rims, y9 top) and the headband (y8-9) sit on cells some eye frames use: these frames stay inside x10-15, y10-12. */
export const LENS_SAFE: Partial<Record<Eyes, Cell[]>> = {
  glanceL: EYES.open,
  glanceR: [[10, 10, 2, 3, 'ink'], [11, 10, 1, 1, 'white'], [14, 10, 2, 3, 'ink'], [15, 10, 1, 1, 'white']],
  up: [[10, 10, 2, 2, 'ink'], [11, 10, 1, 1, 'white'], [14, 10, 2, 2, 'ink'], [15, 10, 1, 1, 'white']],
  happy: [[10, 11, 1, 1, 'ink'], [11, 10, 1, 1, 'ink'], [14, 10, 1, 1, 'ink'], [15, 11, 1, 1, 'ink']],
  wide: [[10, 10, 2, 3, 'ink'], [10, 10, 1, 2, 'white'], [14, 10, 2, 3, 'ink'], [14, 10, 1, 2, 'white']],
};
export const lensSafeFor = (c: Creature, e: Eyes) => (c.accessory === 'glasses' || (c.accessory === 'headband' && e !== 'glanceL' && e !== 'glanceR') ? LENS_SAFE[e] : undefined);

export const MOUTHS: Record<Mouth, Cell[]> = {
  smile: [[11, 14, 1, 1, 'ink'], [14, 14, 1, 1, 'ink'], [12, 15, 2, 1, 'ink']],
  grin: [[11, 14, 4, 1, 'ink'], [12, 15, 2, 1, 'blush']],
  flat: [[12, 15, 2, 1, 'ink']],
  focus: [[12, 15, 2, 1, 'ink'], [14, 15, 1, 1, 'blush']],
  frown: [[12, 14, 2, 1, 'ink'], [11, 15, 1, 1, 'ink'], [14, 15, 1, 1, 'ink']],
  o: [[12, 14, 2, 2, 'ink']],
  none: [],
};

export const MOOD_FACE: Record<MoodName, { eyes: Eyes; mouth: Mouth }> = {
  idle: { eyes: 'open', mouth: 'smile' },
  thinking: { eyes: 'up', mouth: 'flat' },
  working: { eyes: 'focus', mouth: 'focus' },
  finished: { eyes: 'happy', mouth: 'grin' },
  failed: { eyes: 'sad', mouth: 'frown' },
  needs_you: { eyes: 'wide', mouth: 'o' },
  resting: { eyes: 'half', mouth: 'flat' },
};

/** Accessory twitches: the rest cells to hide and the cells drawn instead, for one twitch frame. */
const KNIT_RED = '#e5484d';
export function twitchFor(c: Creature): { hide: Array<[number, number, number, number]>; add: Array<[number, number, number, number, string]> } | null {
  switch (c.accessory) {
    case 'antenna': return { hide: [[15, 3, 2, 2], [15, 3, 1, 1]], add: [[16, 3, 2, 2, c.accent], [16, 3, 1, 1, '#ffffff']] };
    case 'ears': return { hide: [[8, 5, 2, 3], [8, 6, 1, 1]], add: [[8, 6, 2, 2, c.body], [8, 7, 1, 1, '#ff6b81'], [7, 6, 1, 1, c.body]] };
    case 'sprout': return { hide: [[14, 4, 2, 1], [11, 4, 2, 1]], add: [[14, 3, 2, 1, '#3ccf6e'], [11, 5, 2, 1, '#1f8f4c']] };
    case 'bulb': return { hide: [], add: [[12, 2, 2, 1, '#ffffff'], [11, 3, 4, 2, '#fff8d6'], [12, 3, 2, 1, '#ffffff'], [10, 2, 1, 1, c.accent], [16, 2, 1, 1, c.accent]] };
    case 'knithat': return { hide: [[4, 5, 2, 2], [4, 6, 1, 1], [5, 4, 2, 1]], add: [[5, 4, 2, 1, KNIT_RED], [4, 4, 1, 1, KNIT_RED], [3, 5, 2, 2, '#ffffff'], [3, 6, 1, 1, '#d2cbec']] };
    case 'host': return { hide: [[10, 2, 6, 3], [15, 2, 1, 3], [11, 2, 1, 2], [10, 5, 6, 1]], add: [[10, 1, 6, 3, c.light], [15, 1, 1, 3, '#c9b8f7'], [11, 1, 1, 2, '#ffffff'], [10, 4, 6, 1, c.accent]] };
    case 'camera': return { hide: [[7, 17, 1, 1], [5, 14, 1, 1]], add: [[7, 17, 1, 1, '#ffffff'], [6, 16, 1, 1, '#ffffff'], [5, 14, 1, 1, '#ffffff'], [4, 12, 1, 1, '#fff1b8'], [3, 13, 1, 1, '#fff1b8'], [5, 13, 1, 1, '#fff1b8'], [4, 14, 1, 1, '#fff1b8']] };
    case 'bell': return { hide: [[17, 12, 1, 1]], add: [[17, 13, 1, 1, '#5a3d0c'], [20, 11, 1, 1, '#fff1b8'], [21, 10, 1, 1, '#fff1b8'], [19, 10, 1, 1, '#fff1b8']] };
    default: return null;
  }
}

const ROLE_FILL = (c: Creature, r: Role): string =>
  ({ ink: '#1b1033', white: '#f5f2ff', body: c.body, shade: c.shade, light: c.light, blush: '#ff6b81' })[r];

/** The eye cells this creature draws for an eye frame (lens-safe for glasses and the headband). */
export const eyeCells = (c: Creature, e: Eyes): Cell[] => lensSafeFor(c, e) ?? EYES[e];
/** The mouth cells; House has a door for a mouth, so it never draws one. */
export const mouthCells = (c: Creature, m: Mouth): Cell[] => (c.accessory === 'house' ? [] : MOUTHS[m]);
/** A cell as a fill colour for this creature. */
export const cellFill = (c: Creature, r: Role): string => ROLE_FILL(c, r);
/** A mood name, or idle for anything else (the face table has no "loading"). */
export const faceMood = (mood: string | undefined): MoodName => (mood && mood in MOOD_FACE ? (mood as MoodName) : 'idle');

/** Living avatars: the id of each sender's newest reply (bots and other non-person senders), by position in the chat. */
export function newestReplyIds(messages: Array<{ id: string; sender_role: string; sender_id?: string | null }>): Set<string> {
  const seen = new Set<string>();
  const out = new Set<string>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.sender_role === 'user') continue;
    const who = m.sender_id ?? '';
    if (seen.has(who)) continue;
    seen.add(who);
    out.add(m.id);
  }
  return out;
}
