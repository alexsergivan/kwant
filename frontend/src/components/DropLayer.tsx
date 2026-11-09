// 0.8.0 drop files into a chat (Mira's design, release-080/drag-drop/design.md §2). One layer over the
// ChatView column: veil, a frame with pixel corners, and a card saying who gets the files and what
// they are. DropPill is the one line shown over a page that isn't a chat. Both are aria-hidden: the
// chat's own live region speaks for them.
import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { AlertTriangle, Ban, File as FileIcon, FileText, Film, Image as ImageIcon, MessageSquare, Users } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import type { DropCardModel, Heading, KindPart } from '../dragPreview';
import { PILL_TEXT, leaveMs, modalOpen, windowDragWatch } from '../dragPreview';
import { LIMITS_LINE } from '../attachments';

export type DropPhase = 'armed' | 'over' | 'refused' | 'landing' | 'leaving';

export interface DropBot {
  id?: string;
  name: string;
  avatar?: string;
}

const Corner: React.FC<{ pos: 'tl' | 'tr' | 'bl' | 'br' }> = ({ pos }) => (
  <svg className={`kw-dd-corner ${pos}`} width="18" height="18" viewBox="0 0 6 6" shapeRendering="crispEdges" aria-hidden="true">
    <rect x="0" y="0" width="6" height="1" />
    <rect x="0" y="0" width="1" height="6" />
    <rect x="1" y="1" width="1" height="1" />
  </svg>
);

const KindIcon: React.FC<{ kind: KindPart['kind'] }> = ({ kind }) => {
  const Icon = kind === 'image' ? ImageIcon : kind === 'video' ? Film : kind === 'pdf' || kind === 'text' ? FileText : FileIcon;
  return <Icon size={12} className="text-zinc-400" aria-hidden="true" />;
};

const HeadingLine: React.FC<{ h: Heading; refused: boolean }> = ({ h, refused }) => (
  <h3 className="kw-dd-heading" data-drop-heading>
    {h.pre}
    {h.name && <em className={refused ? 'not-italic text-zinc-100' : 'not-italic text-sky-200'}>{h.name}</em>}
    {h.post}
  </h3>
);

export interface DropLayerProps {
  phase: DropPhase;
  card: DropCardModel;
  /** One bot (a bot chat or an @mention) or a room's bots. */
  single: boolean;
  bots: DropBot[];
  /** Room left under the card for the composer: its height + 24 px. */
  bottomPad: number;
  /** Space taken on the right by a side pane, which is not a target. */
  rightGap?: number;
  /** In B, C and D the layer takes the drop, so links and code below never get it. */
  onDragOver?: React.DragEventHandler;
  onDrop?: React.DragEventHandler;
}

export const DropLayer: React.FC<DropLayerProps> = ({ phase, card, single, bots, bottomPad, rightGap = 0, onDragOver, onDrop }) => {
  const refused = phase === 'refused' || (card.state === 'refused' && phase !== 'armed');
  const heading = phase === 'armed' ? card.armedHeading : card.heading;
  const shown = bots.slice(0, 4);
  const more = bots.length - shown.length;
  const takesDrop = phase === 'over' || phase === 'refused';
  return (
    <div
      className="kw-dd"
      data-drop-layer
      data-phase={phase}
      data-refused={refused ? 'true' : undefined}
      aria-hidden="true"
      style={{ right: rightGap, pointerEvents: takesDrop ? 'auto' : 'none' }}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className="kw-dd-veil" />
      <div className="kw-dd-frame">
        <Corner pos="tl" />
        <Corner pos="tr" />
        <Corner pos="bl" />
        <Corner pos="br" />
      </div>
      <div className="kw-dd-place" style={{ paddingBottom: bottomPad }}>
        <div className="kw-dd-card" data-drop-card>
          <div className="kw-dd-who">
            {single ? (
              <span className="kw-dd-one">
                <ModernAvatar role={bots[0]?.avatar || bots[0]?.name || ''} name={bots[0]?.name || ''} size={64} kwantId={bots[0]?.id} className="rounded-2xl" />
              </span>
            ) : (
              <span className="kw-dd-many">
                {shown.map((b) => (
                  <ModernAvatar key={b.id ?? b.name} role={b.avatar || b.name} name={b.name} size={44} kwantId={b.id} className="rounded-full" />
                ))}
                {more > 0 && <span className="kw-dd-more">+{more}</span>}
                {/* A team whose bots aren't loaded: a plain group circle, never a made-up bot face (#478 M4). */}
                {!bots.length && (
                  <span className="kw-dd-more" data-drop-team-icon>
                    <Users size={18} className="text-zinc-300" aria-hidden="true" />
                  </span>
                )}
              </span>
            )}
            {card.count !== null && !refused && (
              <span key={card.count} className="kw-dd-count" data-drop-count>
                {card.count}
              </span>
            )}
          </div>
          <HeadingLine h={heading} refused={refused} />
          {card.refusal ? (
            <>
              <p className="kw-dd-refuse" data-drop-refusal>
                <Ban size={13} aria-hidden="true" className="shrink-0 mt-[2px]" />
                <span>{card.refusal.line1}</span>
              </p>
              {card.refusal.line2 && <p className="kw-dd-refuse2">{card.refusal.line2}</p>}
            </>
          ) : (
            <>
              {card.what && (
                <p className="kw-dd-what" data-drop-summary>
                  {card.count} {card.count === 1 ? 'file' : 'files'} ·{' '}
                  {card.kinds.map((k, i) => (
                    <React.Fragment key={k.kind}>
                      {i > 0 && ', '}
                      <span className="kw-dd-kind">
                        <KindIcon kind={k.kind} />
                        {k.label}
                      </span>
                    </React.Fragment>
                  ))}
                </p>
              )}
              <p className="kw-dd-lim" data-drop-limits>{LIMITS_LINE}</p>
              {card.wont.length > 0 && (
                <div className="kw-dd-wont" data-drop-warning>
                  {card.wont.map((w) => (
                    <div key={w}>
                      <AlertTriangle size={13} aria-hidden="true" className="shrink-0 mt-[2px] text-amber-300" />
                      <span>{w}</span>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
};

/** State F: over a page with no chat. Bottom-centre of the main pane, 28 px up. */
export const DropPill: React.FC<{ leaving?: boolean }> = ({ leaving }) => (
  <div className="kw-dd-pill" data-drop-pill data-phase={leaving ? 'leaving' : 'shown'} aria-hidden="true">
    <MessageSquare size={15} className="text-zinc-400" aria-hidden="true" />
    {PILL_TEXT}
  </div>
);

/**
 * Shows DropPill while a file drag is over the window and no chat is open to take it (design §2 F),
 * and says it once for screen readers. The window guard already makes such a drop do nothing.
 */
export const DropPillHost: React.FC = () => {
  const watch = windowDragWatch();
  const drag = useSyncExternalStore(watch.subscribe, watch.get, watch.get);
  const want = drag.active && drag.chats === 0 && !(typeof document !== 'undefined' && modalOpen(document));
  const [leaving, setLeaving] = useState(false);
  const was = useRef(false);
  useEffect(() => {
    if (want) {
      was.current = true;
      setLeaving(false);
      return;
    }
    if (!was.current) return;
    was.current = false;
    setLeaving(true);
    const t = setTimeout(() => setLeaving(false), leaveMs());
    return () => clearTimeout(t);
  }, [want]);
  return (
    <>
      {(want || leaving) && <DropPill leaving={!want} />}
      <span role="status" aria-live="polite" className="sr-only" data-drop-pill-announce>
        {want ? PILL_TEXT + '.' : ''}
      </span>
    </>
  );
};
