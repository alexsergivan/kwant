// Where the Dev desk mounts Iris's renderers (components/dev/{Rollup,LandedList,ReviewCard,ReleaseNotesSheet}).
// Until hers land on main these are thin placeholders with the props in iris-dev-cards `dev/types.ts`
// (11c6d23), so the shell, its fetching and its live updates can be built and tested now. Swapping
// them is this one file: import hers and parse each card with her parse* first.

import React from 'react';
import { GitMerge, GitPullRequest, Loader2, X } from 'lucide-react';
import type { FindingAction, LandedRollup, ShellCard } from '../dev/shellTypes';
import { withDayLabels } from '../devMode';

export interface RollupProps {
  rollup: LandedRollup;
  last_tag: string | null;
  last_merge: { pr: number; merged_at: string | null } | null;
  onDraftNotes: () => void;
  onMarkSeen: () => void;
  drafting?: boolean;
}

export interface LandedListProps {
  cards: ShellCard[];
  reviews: ShellCard[];
  now?: number;
}

export interface ReviewCardProps {
  card: ShellCard;
  onFinding: (cardId: string, findingId: string, action: FindingAction) => Promise<ShellCard>;
  sample?: boolean;
  now?: number;
}

export interface ReleaseNotesSheetProps {
  card: ShellCard;
  onSaveToNotion?: (markdown: string) => Promise<void>;
  onDiscard: () => void;
  onClose: () => void;
}

const cardBox = 'rounded-2xl border border-hairline bg-surface-raised px-4 py-3';
const ghost =
  'inline-flex items-center gap-1.5 rounded-xl border border-hairline-strong px-3 py-1.5 text-meta font-medium text-zinc-200 hover:bg-zinc-800 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

const text = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);

export const Rollup: React.FC<RollupProps> = ({ rollup, last_tag, last_merge, onMarkSeen, onDraftNotes, drafting }) => (
  <section className={cardBox} data-dev-slot="rollup" aria-label="Since you last looked">
    <p className="text-body font-semibold text-zinc-50">
      {rollup.count ? `${rollup.count} landed since you last looked` : 'Nothing new since you last looked'}
    </p>
    <p className="mt-0.5 text-meta text-zinc-400">
      {rollup.areas.length ? rollup.areas.map((a) => `${a.area} ${a.count}`).join(' · ') : last_merge ? `Last merge: #${last_merge.pr}` : 'No merges yet.'}
    </p>
    <div className="mt-3 flex flex-wrap gap-2">
      <button type="button" className={ghost} onClick={onMarkSeen} disabled={!rollup.count}>
        Mark all seen
      </button>
      <button type="button" className={ghost} onClick={onDraftNotes} disabled={drafting}>
        {drafting && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
        Draft release notes{last_tag ? ` since ${last_tag}` : ''}
      </button>
    </div>
  </section>
);

export const LandedList: React.FC<LandedListProps> = ({ cards, now }) => (
  <div className="flex flex-col gap-2" data-dev-slot="landed-list">
    {withDayLabels(cards, now === undefined ? new Date() : new Date(now)).map(({ card, dayLabel }) => (
      <React.Fragment key={card.id}>
        {dayLabel && <h3 className="pt-2 text-2xs font-semibold uppercase tracking-wider text-zinc-500">{dayLabel}</h3>}
        <article className={cardBox} data-dev-slot="landed" data-status={card.status}>
          <p className="flex items-start gap-2 text-body font-medium text-zinc-100">
            <GitMerge size={14} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
            <span className="min-w-0 break-words">{text(card.headline) ?? text(card.title) ?? `Merge ${card.id}`}</span>
          </p>
          <p className="mt-1 text-meta text-zinc-400">
            {[card.pr ? `#${card.pr}` : null, text(card.area), card.status !== 'ready' ? card.status : null].filter(Boolean).join(' · ')}
          </p>
        </article>
      </React.Fragment>
    ))}
  </div>
);

export const ReviewCard: React.FC<ReviewCardProps> = ({ card }) => (
  <article className={cardBox} data-dev-slot="review" data-status={card.status}>
    <p className="flex items-start gap-2 text-body font-medium text-zinc-100">
      <GitPullRequest size={14} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
      <span className="min-w-0 break-words">
        {card.pr ? `#${card.pr} ` : ''}
        {text(card.title) ?? 'Pull request'}
      </span>
    </p>
    <p className="mt-1 text-meta text-zinc-400">{card.status === 'pending' ? 'Reviewing…' : card.status}</p>
  </article>
);

export const ReleaseNotesSheet: React.FC<ReleaseNotesSheetProps> = ({ card, onClose, onDiscard }) => (
  <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4" data-dev-slot="release-notes">
    <div role="dialog" aria-modal="true" aria-label="Release notes draft" className="w-full max-w-[540px] rounded-3xl border border-hairline-strong bg-surface-raised p-6">
      <div className="flex items-start justify-between gap-3">
        <h2 className="text-lg font-semibold text-zinc-50">Release notes draft</h2>
        <button type="button" onClick={onClose} aria-label="Close" className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100">
          <X size={16} aria-hidden="true" />
        </button>
      </div>
      {card.status === 'pending' ? (
        <p className="mt-3 flex items-center gap-2 text-meta text-zinc-400">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Writing the draft…
        </p>
      ) : (
        <pre className="mt-3 max-h-[50vh] overflow-auto whitespace-pre-wrap text-meta text-zinc-300">{text(card.markdown) ?? ''}</pre>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <button type="button" className={ghost} onClick={onDiscard}>
          Discard
        </button>
      </div>
    </div>
  </div>
);
