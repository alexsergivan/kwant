// 0.9.0 V1: a placeholder for Iris's <ReviewItemView item entry onDone>, so the Inbox panes, the deep link
// and the crumb work before it lands. It shows the header, the verdict and anything held, and an Approve
// that follows D3 (disabled while anything is held). Send back and Roll back come with Iris's view.
import React from 'react';
import { CheckCircle2, Hand, Loader2, MinusCircle, XCircle } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ReviewItem, VERDICT_WORD, approveState } from '../api/inbox';

// Same shapes as Iris's reviewItem.ts (PR #503), so swapping ReviewItemSlot is one line.
export type ReviewEntry = 'inbox' | 'board' | 'dev';
export interface ReviewDone {
  kind: 'approved' | 'sent_back' | 'rolled_back' | 'stale';
  text: string;
  task_id: string;
  undo_checkpoint_id?: string | null;
  copy_task_id?: string | null;
  /** stale (409): what changed since the verdict. */
  paths?: string[];
}

export interface ReviewItemViewProps {
  item: ReviewItem;
  entry: ReviewEntry;
  onDone: (r: ReviewDone) => void;
  /** Stub only: the store's approve, so the flow can be tried end to end. */
  onApprove?: (item: ReviewItem) => void;
  busy?: boolean;
}

export const VERDICT_STYLE = {
  pass: { icon: CheckCircle2, text: 'text-teal-300', box: 'border-teal-400/25 bg-teal-400/5' },
  fail: { icon: XCircle, text: 'text-rose-300', box: 'border-rose-400/30 bg-rose-400/[.06]' },
  needs_you: { icon: Hand, text: 'text-amber-300', box: 'border-amber-500/30 bg-amber-950/25' },
  none: { icon: MinusCircle, text: 'text-zinc-400', box: 'border-zinc-700 bg-white/[.02]' },
} as const;

export const ReviewItemStub: React.FC<ReviewItemViewProps> = ({ item, onApprove, busy = false }) => {
  const v = VERDICT_STYLE[item.verdict.result];
  const Icon = v.icon;
  const ap = approveState(item);
  return (
    <div className="space-y-4" data-review-stub={item.id}>
      <header className="flex items-start gap-3">
        <ModernAvatar role={item.kwant_name} name={item.kwant_name} size={34} kwantId={item.kwant_id} />
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold text-zinc-100 break-words">{item.title}</h2>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {item.round.n > 1 && <span className="px-1.5 rounded-md border border-violet-400/40 text-[11px] text-violet-200">Round {item.round.n}</span>}
            {item.risk.level && item.risk.level !== 'medium' && (
              <span className={`px-1.5 rounded-md border text-[11px] ${item.risk.level === 'high' ? 'border-amber-500/40 text-amber-200' : 'border-zinc-600 text-zinc-300'}`}>
                {item.risk.level === 'high' ? 'High risk' : 'Low risk'}
              </span>
            )}
          </div>
          <p className="mt-1 text-xs text-zinc-400">
            {[item.project, `round ${item.round.n} of ${item.round.max}`].filter(Boolean).join(' · ')}
          </p>
        </div>
      </header>
      <section className={`rounded-xl border p-3 ${v.box}`} aria-label="Verdict">
        <p className={`flex items-center gap-1.5 text-sm font-semibold ${v.text}`}>
          <Icon size={16} aria-hidden="true" />
          {VERDICT_WORD[item.verdict.result]}
          {item.verdict.confidence && <span className="font-normal text-zinc-400">· {item.verdict.confidence} confidence</span>}
        </p>
        <p className="mt-1 text-sm text-zinc-200">{item.verdict.line}</p>
      </section>
      {item.held_actions.length > 0 && (
        <section className="rounded-xl border border-amber-500/30 bg-amber-950/20 p-3 space-y-2" aria-label="Held">
          {item.held_actions.map((h) => (
            <div key={h.approval_id}>
              <p className="flex items-center gap-1.5 text-xs font-semibold text-amber-200">
                <Hand size={13} aria-hidden="true" /> Held: {item.kwant_name} wants to run this, exactly:
              </p>
              <pre className="mt-1 whitespace-pre-wrap font-mono text-xs text-zinc-200">{h.args_display}</pre>
            </div>
          ))}
        </section>
      )}
      <div className="space-y-1.5">
        <button
          type="button"
          aria-disabled={!!ap.blocked || busy}
          onClick={() => !ap.blocked && !busy && onApprove?.(item)}
          className={`kw-focus inline-flex items-center gap-1.5 min-h-[44px] lg:min-h-[34px] px-4 rounded-lg text-sm font-semibold bg-violet-500/80 text-white ${ap.blocked || busy ? 'opacity-[.42] cursor-not-allowed' : 'hover:bg-violet-500'}`}
          data-stub-approve
        >
          {busy ? <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <CheckCircle2 size={14} aria-hidden="true" />}
          {busy ? 'Approving…' : ap.label}
        </button>
        {ap.blocked && (
          <p className="flex items-center gap-1 text-[11.5px] text-amber-200">
            <Hand size={12} aria-hidden="true" /> {ap.blocked}
          </p>
        )}
      </div>
      <p className="rounded-lg border border-dashed border-zinc-700 p-3 text-xs text-zinc-500" data-stub-note>
        Send back, Roll back and the details (Couldn’t check, Decisions, What done looks like, Evidence) arrive with the full item view.
      </p>
    </div>
  );
};

/** The one item view every entry point mounts. Swap this for Iris's ReviewItemView when it lands. */
export const ReviewItemSlot = ReviewItemStub;
