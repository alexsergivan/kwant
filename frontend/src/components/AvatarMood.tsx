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

/** Pixel marks in the avatar's own 26x26 grid, drawn at its top-right corner. */
const px = (x: number, y: number, w: number, h: number, fill: string, key: string) => (
  <rect key={key} x={x} y={y} width={w} height={h} fill={fill} />
);

function marks(mood: Mood): React.ReactNode[] {
  switch (mood) {
    case 'thinking':
      return [
        px(14, 0, 11, 5, INK, 'p'),
        ...[0, 1, 2].map((i) => <rect key={`d${i}`} className={`kw-mood-dot kw-mood-dot-${i}`} x={15.5 + i * 3.2} y={1.5} width={2} height={2} fill="#e1d6ff" />),
      ];
    case 'finished':
      return [
        px(19, 0, 2, 2, '#ffe066', 's1'), px(17, 2, 2, 2, '#ffe066', 's2'), px(19, 2, 2, 2, '#ffffff', 's3'),
        px(21, 2, 2, 2, '#ffe066', 's4'), px(19, 4, 2, 2, '#ffe066', 's5'),
      ];
    case 'failed':
      return [
        px(16, 0, 9, 8, INK, 'p'),
        px(17, 1, 2, 2, '#ff5d6c', 'x1'), px(21, 1, 2, 2, '#ff5d6c', 'x2'), px(19, 3, 2, 2, '#ff5d6c', 'x3'),
        px(17, 5, 2, 2, '#ff5d6c', 'x4'), px(21, 5, 2, 2, '#ff5d6c', 'x5'),
      ];
    case 'needs_you':
      // A raised hand: fingers, palm and thumb.
      return [
        px(16, 0, 9, 9, INK, 'p'),
        px(17, 1, 1, 3, '#ffb020', 'f1'), px(19, 0, 1, 4, '#ffb020', 'f2'), px(21, 1, 1, 3, '#ffb020', 'f3'), px(23, 2, 1, 2, '#ffb020', 'f4'),
        px(17, 4, 7, 3, '#ffb020', 'h'), px(18, 7, 5, 1, '#ffb020', 'h2'), px(16, 5, 1, 2, '#ffb020', 't'),
      ];
    case 'resting':
      // A pixel "z".
      return [px(16, 0, 9, 8, INK, 'p'), px(17, 1, 6, 1, '#b8adff', 'z1'), px(21, 2, 1, 1, '#b8adff', 'z2'), px(20, 3, 1, 1, '#b8adff', 'z3'), px(19, 4, 1, 1, '#b8adff', 'z4'), px(17, 5, 6, 1, '#b8adff', 'z5')];
    default:
      return [];
  }
}

export interface AvatarMoodProps {
  mood: Mood;
  /** Pixel size of the avatar this sits on. */
  size: number;
}

/**
 * The mood overlay for a pixel avatar: a small pixel mark at its top-right. "working" and "idle" draw
 * nothing here (the avatar's own bob and ring cover working). Decorative: the text for it lives on the
 * avatar's title and screen-reader label (moodLabel). ModernAvatar renders it for you when it is
 * given a kwantId; use this directly only on a custom avatar.
 */
export const AvatarMood: React.FC<AvatarMoodProps> = ({ mood, size }) => {
  const drawn = marks(mood);
  if (!drawn.length) return null;
  return (
    <svg
      viewBox="0 0 26 26"
      width={size}
      height={size}
      className={`kw-mood kw-mood-${mood} pointer-events-none absolute inset-0`}
      style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
      aria-hidden="true"
      data-mood={mood}
    >
      {drawn}
    </svg>
  );
};
