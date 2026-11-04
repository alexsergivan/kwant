import React, { useCallback, useSyncExternalStore } from 'react';
import { moodOf, noticesOf, subscribeMood, type Mood } from '../kwantMood';

const noop = () => undefined;

/** A bot's mood now, re-rendering only when it (or its own turns) change. "idle" means nothing to show. */
export function useKwantMood(kwantId: string | undefined): Mood {
  const subscribe = useCallback((on: () => void) => (kwantId ? subscribeMood(kwantId, on) : noop), [kwantId]);
  return useSyncExternalStore(subscribe, () => moodOf(kwantId), () => moodOf(kwantId));
}

/** How many times the person has messaged this bot; the avatar blinks once per change. */
export function useKwantNotices(kwantId: string | undefined): number {
  const subscribe = useCallback((on: () => void) => (kwantId ? subscribeMood(kwantId, on) : noop), [kwantId]);
  return useSyncExternalStore(subscribe, () => noticesOf(kwantId), () => noticesOf(kwantId));
}

const INK = '#1b1033';

type Cell = [number, number];
const row = (y: number): Cell[] => [0, 1, 2, 3, 4].map((x) => [x, y] as Cell);

/**
 * Glyphs on their own 5x5 grid (spec.md §2), drawn on a 7x7 ink plate at whole-pixel cells, so they
 * stay crisp at 24-28 px. Coordinates are (x, y) in the 5x5 grid.
 */
export const MOOD_GLYPHS: Record<'thinking' | 'failed' | 'needs_you' | 'resting', { color: string; cells: Cell[] }> = {
  thinking: { color: '#e1d6ff', cells: [[0, 2], [2, 2], [4, 2]] },
  failed: { color: '#ff5d6c', cells: [[0, 0], [4, 0], [1, 1], [3, 1], [2, 2], [1, 3], [3, 3], [0, 4], [4, 4]] },
  // "!" rather than a raised hand: at 96 px the hand read as a crown (Ada's decision).
  needs_you: { color: '#ffb020', cells: [[2, 0], [2, 1], [2, 2], [2, 4]] },
  resting: { color: '#b8adff', cells: [...row(0), [3, 1], [2, 2], [1, 3], ...row(4)] },
};
/** Just finished: a sparkle that grows over three frames (0 / 60 / 120 ms), centre white. */
const SPARK = '#ffe066';
const F2: Cell[] = [[2, 1], [1, 2], [3, 2], [2, 3]];
const F3: Cell[] = [[2, 0], [0, 2], [4, 2], [2, 4]];

const cell = ([x, y]: Cell, fill: string, key: string, className?: string) => (
  <rect key={key} className={className} x={x + 1} y={y + 1} width={1} height={1} fill={fill} />
);

function glyph(mood: Mood): React.ReactNode[] {
  switch (mood) {
    case 'thinking':
      return MOOD_GLYPHS.thinking.cells.map((c, i) => cell(c, MOOD_GLYPHS.thinking.color, `d${i}`, `kw-mood-dot kw-mood-dot-${i}`));
    case 'finished':
      return [
        ...F3.map((c, i) => cell(c, SPARK, `f3${i}`, 'kw-mood-f3')),
        ...F2.map((c, i) => cell(c, SPARK, `f2${i}`, 'kw-mood-f2')),
        cell([2, 2], '#ffffff', 'f1'),
      ];
    case 'failed':
    case 'needs_you':
    case 'resting':
      return MOOD_GLYPHS[mood].cells.map((c, i) => cell(c, MOOD_GLYPHS[mood].color, `g${i}`));
    default:
      return [];
  }
}

/** Badge geometry for an avatar size: 2 px cells up to 40 px, 4 px from 64 px (96 px Bot home). */
export function moodBadge(size: number): { cellPx: number; px: number; offset: number; radius: number; cut: number } {
  const big = size >= 64;
  const cellPx = big ? 4 : 2;
  return { cellPx, px: cellPx * 7, offset: big ? -6 : -4, radius: big ? 4 : 2, cut: big ? 2 : 1 };
}

export interface AvatarMoodProps {
  mood: Mood;
  /** Pixel size of the avatar this sits on. */
  size: number;
}

/**
 * The mood badge for a pixel avatar: a 7x7 ink plate at its top-right with a 5x5 pixel glyph, cut out
 * of the surface by a 1-2 px ring (--kw-avatar-cut, default the sidebar #0e0a21). "working" and "idle"
 * draw nothing here (the avatar's own bob and violet ring cover working). Decorative: the words live on
 * the avatar's title and screen-reader label (moodLabel). ModernAvatar renders it when given a kwantId;
 * use this directly only on a custom avatar.
 */
export const AvatarMood: React.FC<AvatarMoodProps> = ({ mood, size }) => {
  const drawn = glyph(mood);
  if (!drawn.length) return null;
  const b = moodBadge(size);
  return (
    <svg
      viewBox="0 0 7 7"
      width={b.px}
      height={b.px}
      className={`kw-mood kw-mood-${mood} pointer-events-none absolute`}
      style={{
        top: b.offset,
        right: b.offset,
        borderRadius: b.radius,
        boxShadow: `0 0 0 ${b.cut}px var(--kw-avatar-cut, #0e0a21)`,
        shapeRendering: 'crispEdges',
        imageRendering: 'pixelated',
      }}
      aria-hidden="true"
      data-mood={mood}
    >
      <rect x={0} y={0} width={7} height={7} fill={INK} />
      {drawn}
    </svg>
  );
};
