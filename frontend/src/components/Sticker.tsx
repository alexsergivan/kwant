import React, { useRef } from 'react';

export type StickerTone = 'holo' | 'ink';

/** Tilt in degrees, kept to the -3..3 the style allows; anything else (or NaN) falls back to `fallback`. */
export const clampTilt = (tilt: number | undefined, fallback = 0): number =>
  typeof tilt === 'number' && Number.isFinite(tilt) ? Math.max(-3, Math.min(3, tilt)) : fallback;

export interface StickerProps {
  /** "holo": earned things only (verified, picked, goal kept). "ink": a plain plate, e.g. "New". Default "ink". */
  tone?: StickerTone;
  /** -3 to 3 degrees. Default: -2 for holo, 2 for ink. */
  tilt?: number;
  /** A small lucide icon (size 12) shown before the text. */
  icon?: React.ReactNode;
  /** Hover text. */
  title?: string;
  /** Spoken label when the text alone isn't enough. */
  ariaLabel?: string;
  /** Makes the sticker a button. */
  onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void;
  className?: string;
  children: React.ReactNode;
}

/**
 * A die-cut sticker. Holo is earned: use it only for results a check produced. At most one per card.
 * Warnings, errors and "needs you" stay plain (kw-chip), never a sticker.
 */
export const Sticker: React.FC<StickerProps> = ({ tone = 'ink', tilt, icon, title, ariaLabel, onClick, className = '', children }) => {
  const ref = useRef<HTMLElement | null>(null);
  const deg = clampTilt(tilt, tone === 'holo' ? -2 : 2);
  // The sheen follows the pointer through CSS variables; nothing re-renders.
  const track = (e: React.PointerEvent<HTMLElement>) => {
    const el = ref.current;
    if (!el || tone !== 'holo' || e.pointerType === 'touch') return;
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) return;
    el.style.setProperty('--mx', `${Math.round(((e.clientX - r.left) / r.width) * 100)}%`);
    el.style.setProperty('--my', `${Math.round(((e.clientY - r.top) / r.height) * 100)}%`);
  };
  const props = {
    ref: ref as React.RefObject<never>,
    className: `kw-sticker kw-sticker-${tone} ${className}`.trim(),
    style: { ['--kw-tilt' as string]: `${deg}deg` } as React.CSSProperties,
    title,
    'aria-label': ariaLabel,
    'data-sticker': tone,
    onPointerMove: track,
  };
  const body = (
    <>
      {icon}
      <span>{children}</span>
    </>
  );
  return onClick ? (
    <button type="button" onClick={onClick} {...props}>{body}</button>
  ) : (
    <span {...props}>{body}</span>
  );
};
