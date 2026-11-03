import React, { useRef } from 'react';

export type StickerTone = 'holo' | 'ink';
export type StickerSize = 'md' | 'sm';

/** Tilt in degrees, kept to the -3..3 the style allows; anything else (or NaN) falls back to `fallback`. */
export const clampTilt = (tilt: number | undefined, fallback = 0): number =>
  typeof tilt === 'number' && Number.isFinite(tilt) ? Math.max(-3, Math.min(3, tilt)) : fallback;

const reducedMotion = () =>
  typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

export interface StickerProps {
  /** "holo": earned things only (verified, picked, goal kept). "ink": a plain plate, e.g. "New". Default "ink". */
  tone?: StickerTone;
  /** "md" 24 px (default) or "sm" 18 px. The class kw-sticker-sm does the same as size="sm". */
  size?: StickerSize;
  /** -3 to 3 degrees. Default: -2 for holo, 2 for ink. */
  tilt?: number;
  /** A small lucide icon (12 px on md, 10 px on sm) shown before the text. */
  icon?: React.ReactNode;
  /** Hover text. */
  title?: string;
  /** Spoken label when the text alone isn't enough. */
  ariaLabel?: string;
  /** Makes it clickable: the sticker sits inside a 44x44 button (.kw-sticker-hit). */
  onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void;
  /** On the outermost element (the hit button when clickable), so positioning classes work either way. */
  className?: string;
  children: React.ReactNode;
}

/**
 * A die-cut sticker (spec.md §1). Holo is earned: use it only for results a check produced, at most one
 * per card. Warnings, errors, needs you and Approvals off stay plain chips (kw-chip-warn / kw-chip-bad).
 */
export const Sticker: React.FC<StickerProps> = ({ tone = 'ink', size, tilt, icon, title, ariaLabel, onClick, className = '', children }) => {
  const ref = useRef<HTMLSpanElement | null>(null);
  const deg = clampTilt(tilt, tone === 'holo' ? -2 : 2);
  const sm = size === 'sm' || /\bkw-sticker-sm\b/.test(className);
  // The sheen follows a mouse or pen through CSS variables (registered with @property, so it glides);
  // nothing re-renders. Touch and reduced motion keep the static 30% / 20%.
  const track = (e: React.PointerEvent<HTMLElement>) => {
    const el = ref.current;
    if (!el || tone !== 'holo' || e.pointerType === 'touch' || reducedMotion()) return;
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const clamp = (v: number) => Math.max(0, Math.min(100, Math.round(v)));
    el.style.setProperty('--mx', `${clamp(((e.clientX - r.left) / r.width) * 100)}%`);
    el.style.setProperty('--my', `${clamp(((e.clientY - r.top) / r.height) * 100)}%`);
  };
  const reset = () => {
    const el = ref.current;
    if (!el) return;
    el.style.removeProperty('--mx');
    el.style.removeProperty('--my');
  };
  const sticker = (outer: boolean) => (
    <span
      ref={ref}
      className={`kw-sticker kw-sticker-${tone} ${sm ? 'kw-sticker-sm' : ''} ${outer ? className.replace(/\bkw-sticker-sm\b/, '') : ''}`.replace(/\s+/g, ' ').trim()}
      style={{ ['--kw-tilt' as string]: `${deg}deg` } as React.CSSProperties}
      title={outer ? title : undefined}
      aria-label={outer ? ariaLabel : undefined}
      data-sticker={tone}
      onPointerMove={outer ? track : undefined}
      onPointerLeave={outer ? reset : undefined}
    >
      {icon}
      <span>{children}</span>
    </span>
  );
  if (!onClick) return sticker(true);
  return (
    <button
      type="button"
      onClick={onClick}
      onPointerMove={track}
      onPointerLeave={reset}
      title={title}
      aria-label={ariaLabel}
      className={`kw-sticker-hit ${className.replace(/\bkw-sticker-sm\b/, '')}`.replace(/\s+/g, ' ').trim()}
    >
      {sticker(false)}
    </button>
  );
};
