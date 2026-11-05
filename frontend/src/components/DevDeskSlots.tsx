// Where the Dev desk mounts Iris's renderers (components/dev/, #314). The shell fetches and caches loose
// `ShellCard`s; this file parses each one with her parse* (dev/types.ts) and draws her card, or her plain
// `CardFallback` when a card has a shape her renderer can't draw honestly.

import React, { useMemo } from 'react';
import type { FindingAction, LandedRollup, ShellCard } from '../dev/shellTypes';
import { parseLanded, parseReleaseNotes, parseReview } from '../dev/types';
import type { LandedCard as Landed, ReviewCard as Review } from '../dev/types';
import { needsYouAreas } from '../dev/logic';
import { CardFallback } from './dev/parts';
import { LandedList as IrisLandedList } from './dev/LandedCard';
import { ReviewCard as IrisReviewCard } from './dev/ReviewCard';
import { Rollup as IrisRollup } from './dev/Rollup';
import { ReleaseNotesSheet as IrisReleaseNotesSheet } from './dev/ReleaseNotesSheet';

export interface RollupProps {
  rollup: LandedRollup;
  last_tag: string | null;
  last_merge: { pr: number; merged_at: string | null } | null;
  /** The landed cards on screen, for the tiles' warn border. */
  cards?: ShellCard[];
  branch?: string;
  onDraftNotes: () => void;
  onMarkSeen: () => void;
  drafting?: boolean;
  marking?: boolean;
}

export interface LandedListProps {
  cards: ShellCard[];
  reviews: ShellCard[];
  now?: number;
  onOpenReview?: (pr: number) => void;
}

export interface ReviewCardProps {
  card: ShellCard;
  onFinding: (cardId: string, findingId: string, action: FindingAction) => Promise<ShellCard>;
  /** 0.7.1 Post to GitHub (`POST /api/dev/cards/:id/post`). */
  onPost?: (cardId: string) => Promise<ShellCard>;
  githubLogin?: string | null;
  sample?: boolean;
  now?: number;
}

export interface ReleaseNotesSheetProps {
  card: ShellCard;
  onSaveToNotion?: (markdown: string) => Promise<void>;
  onDiscard: () => void;
  onClose: () => void;
}

const isSome = <T,>(v: T | null): v is T => v !== null;
export const parseLandedCards = (cards: ShellCard[]): Landed[] => cards.map(parseLanded).filter(isSome);
export const parseReviewCards = (cards: ShellCard[]): Review[] => cards.map(parseReview).filter(isSome);

export const Rollup: React.FC<RollupProps> = ({ cards = [], ...rest }) => {
  const needs = useMemo(() => needsYouAreas(parseLandedCards(cards)), [cards]);
  return <IrisRollup {...rest} needsYouAreas={needs} />;
};

export const LandedList: React.FC<LandedListProps> = ({ cards, reviews, now, onOpenReview }) => {
  const parsed = useMemo(() => parseLandedCards(cards), [cards]);
  const parsedReviews = useMemo(() => parseReviewCards(reviews), [reviews]);
  // Cards Iris's parser refuses (no PR number and no commit, wrong kind) still show, as the plain fallback row.
  const refused = useMemo(() => cards.filter((c) => parseLanded(c) === null), [cards]);
  return (
    <>
      {parsed.length > 0 && <IrisLandedList cards={parsed} reviews={parsedReviews} now={now} onOpenReview={onOpenReview} />}
      {refused.map((c) => (
        <CardFallback key={c.id} raw={c} />
      ))}
    </>
  );
};

export const ReviewCard: React.FC<ReviewCardProps> = ({ card, onFinding, onPost, githubLogin, sample, now }) => {
  const parsed = useMemo(() => parseReview(card), [card]);
  if (!parsed) return <CardFallback raw={card} />;
  const send = async (cardId: string, findingId: string, action: FindingAction): Promise<Review> => {
    const next = parseReview(await onFinding(cardId, findingId, action));
    if (!next) throw new Error('The review came back in a shape this app doesn’t know.');
    return next;
  };
  const post = onPost
    ? async (cardId: string): Promise<Review> => {
        const next = parseReview(await onPost(cardId));
        if (!next) throw new Error('The review came back in a shape this app doesn’t know.');
        return next;
      }
    : undefined;
  return <IrisReviewCard card={parsed} onFinding={send} onPost={post} githubLogin={githubLogin} sample={sample} now={now} />;
};

export const ReleaseNotesSheet: React.FC<ReleaseNotesSheetProps> = ({ card, onSaveToNotion, onDiscard, onClose }) => {
  const parsed = useMemo(() => parseReleaseNotes(card), [card]);
  if (!parsed) return null;
  return <IrisReleaseNotesSheet card={parsed} onSaveToNotion={onSaveToNotion} onDiscard={onDiscard} onClose={onClose} />;
};
