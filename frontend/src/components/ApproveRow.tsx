// reply-chrome part 2 (autonomy/reply-chrome/design.md): Approve / Change… under the newest bot reply when it
// ends by asking for the person's OK and no question card is open. Approve sends an ordinary reply to that
// message; Change… starts a reply and leaves the words to the person. No server action.

import React from 'react';
import { Check, PenLine } from 'lucide-react';
import { waitingCaption } from '../approvalAsk';

const ring = 'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-surface-main';
const size = 'h-11 sm:h-8 w-full sm:w-auto px-3.5 rounded-[10px] text-[13px] font-semibold disabled:opacity-60 disabled:cursor-not-allowed';

export const ApproveRow: React.FC<{
  name: string;
  busy?: boolean;
  onApprove: () => void;
  onChange: () => void;
}> = ({ name, busy = false, onApprove, onChange }) => {
  const caption = waitingCaption(name);
  return (
    <div role="group" aria-label={caption} data-approve-row="" className="mt-2.5 grid w-full max-w-full grid-cols-2 gap-2 sm:flex sm:w-auto sm:items-center">
      <p className="col-span-2 text-xs text-zinc-300 sm:mr-1">
        <b className="font-semibold text-zinc-100">{name}</b> is waiting for your OK
      </p>
      <button
        type="button"
        onClick={onApprove}
        disabled={busy}
        data-approve=""
        className={`inline-flex items-center justify-center gap-1.5 bg-accent-gradient text-white shadow-card transition-[filter] duration-[120ms] hover:brightness-110 motion-reduce:transition-none ${size} ${ring}`}
      >
        <Check size={14} aria-hidden="true" />
        Approve
      </button>
      <button
        type="button"
        onClick={onChange}
        disabled={busy}
        data-change=""
        className={`inline-flex items-center justify-center gap-1.5 border border-hairline-strong bg-transparent text-zinc-100 transition-colors duration-[120ms] hover:bg-zinc-800 motion-reduce:transition-none ${size} ${ring}`}
      >
        <PenLine size={14} aria-hidden="true" />
        Change…
      </button>
    </div>
  );
};
