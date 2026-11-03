import React, { useSyncExternalStore } from 'react';
import { isNewThing, subscribeNew, type NewId } from '../whatsNew';
import { Sticker } from './Sticker';

/** A small ink "New" sticker for something 0.6.9 changed; gone for good once the person has used it. */
export const NewTag: React.FC<{ id: NewId; className?: string }> = ({ id, className = '' }) => {
  const show = useSyncExternalStore(subscribeNew, () => isNewThing(id), () => false);
  if (!show) return null;
  return (
    <span className={`kw-new-tag pointer-events-none ${className}`} data-new-tag={id}>
      <Sticker tone="ink" tilt={3} className="kw-sticker-sm">New</Sticker>
    </span>
  );
};
