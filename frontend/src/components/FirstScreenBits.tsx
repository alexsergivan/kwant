import React from 'react';
import { spaceSublineParts, sublineText, type SpaceApprovals } from '../firstScreen';
import type { SpaceCounts } from '../spaces';

/**
 * The space header's sub-line (first-screen A): "3 bots · Dev mode · Approvals off · 1 account". The status
 * words are words, not controls; their settings live in Space settings › General.
 */
export function SpaceSubline({ counts, devOn, approvals }: { counts: SpaceCounts | undefined; devOn: boolean; approvals: SpaceApprovals }) {
  const parts = spaceSublineParts(counts, { devOn, approvals });
  if (!parts.length) return null;
  return (
    <span className="block text-meta text-zinc-400 truncate" title={sublineText(parts)} data-space-subline>
      {parts.map((p, i) => (
        <React.Fragment key={i}>
          {i > 0 && ' · '}
          {p.tone ? (
            <span className={p.tone === 'dev' ? 'text-sky-300' : 'text-warn'} data-subline-status={p.tone}>
              {p.text}
            </span>
          ) : (
            p.text
          )}
        </React.Fragment>
      ))}
    </span>
  );
}
