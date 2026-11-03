import React, { useEffect, useRef, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { AvatarMood } from './AvatarMood';
import { PILL_AFTER_HOVER_MS, PILL_LEAVE_MS, PILL_MS, pillMotion, pillText, type Nudge } from '../attention';

export interface NeedsYouPillProps {
  items: Nudge[];
  /** Bumped when more arrive: the 5 s start again and the text swaps in. */
  rev: number;
  /** The avatar sprite for a bot, when the roster has it. */
  spriteOf: (kwantId: string) => string | undefined;
  /** One item opens its chat; several open the Inbox. */
  onOpen: (items: Nudge[]) => void;
  onDismiss: () => void;
}

const AVATAR = 32;

const lightScheme = () => {
  try {
    return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-color-scheme: light)').matches;
  } catch {
    return false;
  }
};

/**
 * The in-app "needs you" pill (0.6.9, Mira's mac-nudge/design/pill): drops from the top of the app for
 * 5 s when a bot needs the person in a chat they aren't looking at. Hover or keyboard focus holds it
 * (2 s more after the pointer leaves), Esc dismisses it, a click goes there. It never decides
 * anything: there is no Allow or Deny here.
 */
export const NeedsYouPill: React.FC<NeedsYouPillProps> = ({ items, rev, spriteOf, onOpen, onDismiss }) => {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [hoveredOnce, setHoveredOnce] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const [motion] = useState(pillMotion);
  const [theme] = useState(() => (lightScheme() ? 'light' : 'dark'));
  const showing = items.length > 0;
  // The rev the current pill opened with: a later rev means more arrived and the text swaps in.
  const firstRev = useRef(rev);
  const wasShowing = useRef(false);
  if (showing && !wasShowing.current) firstRev.current = rev;
  wasShowing.current = showing;

  // A new pill starts fresh; more arriving swaps the text in place.
  useEffect(() => {
    if (!showing) {
      setHovered(false);
      setFocused(false);
      setHoveredOnce(false);
      setLeaving(false);
    }
  }, [showing]);

  const leave = () => setLeaving(true);
  useEffect(() => {
    if (!leaving) return;
    const t = setTimeout(onDismiss, PILL_LEAVE_MS);
    return () => clearTimeout(t);
  }, [leaving, onDismiss]);

  useEffect(() => {
    if (!showing || hovered || focused || leaving) return;
    const t = setTimeout(leave, hoveredOnce ? PILL_AFTER_HOVER_MS : PILL_MS);
    return () => clearTimeout(t);
  }, [showing, hovered, focused, leaving, hoveredOnce, rev]);

  const text = pillText(items);
  const lead = items[0];
  const many = items.length > 1;
  const swapping = showing && rev !== firstRev.current;

  return (
    <div className="kw-nudge-region" role="status" aria-live="polite" aria-atomic="true">
      {showing && lead && (
        <button
          type="button"
          className={`kw-nudge${leaving ? ' is-leaving' : ''}${swapping ? ' is-swapping' : ''}`}
          key={firstRev.current}
          data-motion={motion}
          data-theme={theme}
          data-count={items.length}
          onClick={() => onOpen(items)}
          onMouseEnter={() => setHovered(true)}
          onMouseLeave={() => {
            setHovered(false);
            setHoveredOnce(true);
          }}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              leave();
            }
          }}
        >
          <span className="kw-nudge__av" key={`av${rev}`} aria-hidden="true">
            {many ? (
              <span className="kw-nudge__count">{items.length > 9 ? '9+' : items.length}</span>
            ) : (
              <ModernAvatar role={spriteOf(lead.kwantId)} name={lead.kwantName} size={AVATAR} />
            )}
            <AvatarMood mood="needs_you" size={AVATAR} />
          </span>
          <span className="kw-nudge__text" key={`t${rev}`}>
            <span className="kw-nudge__l1">{text.headline}</span>
            {text.detail && (
              <>
                <span className="sr-only">: </span>
                <span className="kw-nudge__l2">{text.detail}</span>
              </>
            )}
            <span className="sr-only">. {text.action}. Escape dismisses.</span>
          </span>
          <ChevronRight className="kw-nudge__chev" size={14} strokeWidth={2.25} aria-hidden="true" />
        </button>
      )}
    </div>
  );
};
