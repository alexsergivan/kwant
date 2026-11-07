import React from 'react';
import type { DevBotRef } from '../devMode';
import { ModernAvatar } from './ModernAvatar';

/** 18 px bot faces in a row: the lead alone (Settings) or the whole team (the step 2 picker). */
export const TeamFaces: React.FC<{ bots: DevBotRef[]; size?: number; max?: number }> = ({ bots, size = 18, max = 4 }) => (
  <span className="inline-flex items-center gap-1 shrink-0" aria-hidden="true" data-team-faces>
    {bots.slice(0, max).map((b) => (
      <ModernAvatar key={b.id} role={b.avatar} name={b.name} size={size} />
    ))}
  </span>
);
