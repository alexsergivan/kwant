import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { isNewThing, subscribeNew, type NewId } from '../whatsNew';
import { Sticker } from './Sticker';

const FADE_MS = 160;

/**
 * A small ink "New" sticker for something 0.6.9 changed; gone for good once the person has used it.
 * When it goes it fades to 0 over 160 ms, then unmounts (at once under reduced motion).
 */
export const NewTag: React.FC<{ id: NewId; className?: string; style?: React.CSSProperties }> = ({ id, className = '', style }) => {
  const show = useSyncExternalStore(subscribeNew, () => isNewThing(id), () => false);
  const [leaving, setLeaving] = useState(false);
  const [wasShown, setWasShown] = useState(show);
  if (show && !wasShown) setWasShown(true);
  useEffect(() => {
    if (show || !wasShown) return;
    const reduce = typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (reduce) {
      setWasShown(false);
      return;
    }
    setLeaving(true);
    const t = window.setTimeout(() => {
      setLeaving(false);
      setWasShown(false);
    }, FADE_MS);
    return () => window.clearTimeout(t);
  }, [show, wasShown]);
  if (!show && !wasShown) return null;
  return (
    <span className={`kw-new-tag pointer-events-none ${className}`} data-new-tag={id} data-leaving={leaving || !show ? '' : undefined} style={style} aria-hidden={!show || undefined}>
      <Sticker tone="ink" size="sm" tilt={2}>New</Sticker>
    </span>
  );
};
