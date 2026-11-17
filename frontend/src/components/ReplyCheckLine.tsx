import React from 'react';
import { CircleAlert } from 'lucide-react';
import { checkLine, type ReplyOutcome } from '../replyOutcome';

/**
 * 26d: the quiet line under a reply whose step didn't run or failed for good, so a confident reply is never
 * the last word. "Details" opens the reply's Details at that step. Nothing for "failed, then worked".
 */
export const ReplyCheckLine: React.FC<{ outcome: ReplyOutcome | null; onDetails: () => void }> = ({ outcome, onDetails }) => {
  const text = checkLine(outcome);
  if (!text) return null;
  return (
    <p data-reply-check={outcome!.kind} className="mt-1.5 flex max-w-full min-w-0 items-start gap-1.5 text-xs leading-5 text-zinc-400">
      <CircleAlert size={13} aria-hidden="true" className={`mt-[3px] shrink-0 ${outcome!.kind === 'failed' ? 'text-rose-300' : 'text-amber-300'}`} />
      <span className="min-w-0 break-words">
        {text}{' '}
        <button
          type="button"
          onClick={onDetails}
          className="kw-focus kw-hit rounded font-semibold text-violet-300 hover:text-violet-200"
          data-compact
        >
          Details
        </button>
      </span>
    </p>
  );
};
