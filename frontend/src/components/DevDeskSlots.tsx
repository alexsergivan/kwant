// Where the Dev desk mounts Iris's renderers (components/dev/, #314). The shell fetches and caches loose
// `ShellCard`s; this file parses each one with her parse* (dev/types.ts) and draws her card, or her plain
// `CardFallback` when a card has a shape her renderer can't draw honestly.

import React, { useMemo } from 'react';
import type { FindingAction, LandedRollup, ShellCard } from '../dev/shellTypes';
import { parseLanded, parseReleaseNotes, parseReview } from '../dev/types';
import type { Forge, LandedCard as Landed, ReviewCard as Review } from '../dev/types';
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
  onOpenReview?: (pr: number, repo?: string | null) => void;
  /** `forge` from GET dev_mode: names the "Open on {Forge}" link. */
  forge?: Forge | null;
  /** 0.7.2: the card's repo chip text, or null with one repo in the space (`cardRepoLabel`). */
  repoLabel?: (repo: string | null, name?: string | null) => string | null;
  /** 0.7.3 (any-git): is this card's repo read with plain git? Then no CI tile and no "Open on {Forge}" link. */
  gitOnly?: (repo: string | null) => boolean;
}

export interface ReviewCardProps {
  card: ShellCard;
  onFinding: (cardId: string, findingId: string, action: FindingAction) => Promise<ShellCard>;
  /** 0.7.1 Post to {Forge} (`POST /api/dev/cards/:id/post`). */
  onPost?: (cardId: string) => Promise<ShellCard>;
  /** `forge` and `forge_login` from GET dev_mode; no forge, no Post button. */
  forge?: Forge | null;
  forgeLogin?: string | null;
  sample?: boolean;
  now?: number;
  /** 0.7.2: the card's repo chip text, or null with one repo in the space (`cardRepoLabel`). */
  repoLabel?: (repo: string | null, name?: string | null) => string | null;
  /** 0.7.3 (any-git): the card's repo is read with plain git: no forge, so no Post button. */
  gitOnly?: boolean;
  /** 0.7.6: the saved Jira site's origins, for "Matches the ticket" links (api-decisions 18). */
  jiraOrigins?: readonly string[];
  /** 0.9.0 G1: the line-notes routes; without it the card has no Changes section. */
  lineNotes?: import('../dev/lineNotesApi').LineNotesApi;
  /** 0.9.0 G1: "Open {bot}'s chat" after a send. */
  onOpenChat?: (botId: string) => void;
  /** Fix it "Send to…" (#609): the card's recipients, and "Open task ›" after a send. */
  recipients?: (cardId: string) => Promise<import('../dev/types').DevRecipients>;
  onOpenTask?: (taskId: string) => void;
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

export const LandedList: React.FC<LandedListProps> = ({ cards, reviews, now, onOpenReview, forge = null, repoLabel, gitOnly }) => {
  const parsed = useMemo(() => parseLandedCards(cards), [cards]);
  const parsedReviews = useMemo(() => parseReviewCards(reviews), [reviews]);
  // Cards Iris's parser refuses (no PR number and no commit, wrong kind) still show, as the plain fallback row.
  const refused = useMemo(() => cards.filter((c) => parseLanded(c) === null), [cards]);
  return (
    <>
      {parsed.length > 0 && <IrisLandedList cards={parsed} reviews={parsedReviews} now={now} onOpenReview={onOpenReview} forge={forge} repoLabel={repoLabel} gitOnly={gitOnly} />}
      {refused.map((c) => (
        <CardFallback key={c.id} raw={c} />
      ))}
    </>
  );
};

export const ReviewCard: React.FC<ReviewCardProps> = ({ card, onFinding, onPost, forge, forgeLogin, sample, now, repoLabel, gitOnly = false, jiraOrigins, lineNotes, onOpenChat, recipients, onOpenTask }) => {
  const parsed = useMemo(() => parseReview(card), [card]);
  if (!parsed) return <CardFallback raw={card} />;
  const send = async (cardId: string, findingId: string, action: FindingAction): Promise<Review> => {
    const next = parseReview(await onFinding(cardId, findingId, action));
    if (!next) throw new Error('The review came back in a shape this app doesn’t know.');
    return next;
  };
  const post = onPost && !gitOnly
    ? async (cardId: string): Promise<Review> => {
        const next = parseReview(await onPost(cardId));
        if (!next) throw new Error('The review came back in a shape this app doesn’t know.');
        return next;
      }
    : undefined;
  return <IrisReviewCard card={parsed} onFinding={send} onPost={post} forge={gitOnly ? null : forge} forgeLogin={forgeLogin} sample={sample} now={now} repoLabel={repoLabel} jiraOrigins={jiraOrigins} lineNotes={lineNotes} onOpenChat={onOpenChat} recipients={recipients} onOpenTask={onOpenTask} />;
};

export const ReleaseNotesSheet: React.FC<ReleaseNotesSheetProps> = ({ card, onSaveToNotion, onDiscard, onClose }) => {
  const parsed = useMemo(() => parseReleaseNotes(card), [card]);
  if (!parsed) return null;
  return <IrisReleaseNotesSheet card={parsed} onSaveToNotion={onSaveToNotion} onDiscard={onDiscard} onClose={onClose} />;
};
