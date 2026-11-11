// H3 (Mira's h4-why-status.md "H3 copy"): the line under the person's own message when the server
// held it instead of starting a turn. N3 (Rigel's qa-499.md): after the stall check gave up, it offers
// Try again, or the person would think the app is ignoring them.
import React, { useState } from 'react';
import { Clock } from 'lucide-react';

export interface HeldLineProps {
  text: string;
  /** Shown only when the bot gave up (`stalled_gave_up`): releases the held message now. */
  onTryAgain?: () => Promise<string | void>;
}

export const HeldLine: React.FC<HeldLineProps> = ({ text, onTryAgain }) => {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const tryAgain = async () => {
    if (!onTryAgain || busy) return;
    setBusy(true);
    setError(null);
    const err = await onTryAgain().catch(() => 'Couldn’t try again. Check your connection, then press it again.');
    setBusy(false);
    if (err) setError(err);
  };
  return (
    <div data-held-line="" className="mt-1.5 flex max-w-full flex-wrap items-center justify-end gap-x-2 gap-y-1 text-right text-[12.5px] text-zinc-300">
      <span className="min-w-0" role="status">
        <Clock size={13} className="mr-1.5 inline-block -mt-0.5 align-middle text-zinc-400" aria-hidden="true" />
        {text}
      </span>
      {onTryAgain && (
        <button
          type="button"
          data-held-try-again=""
          onClick={tryAgain}
          disabled={busy}
          aria-busy={busy || undefined}
          className="min-h-[32px] rounded-lg border border-zinc-600 px-2.5 py-1 text-[12px] font-medium text-zinc-100 hover:border-sky-500/60 hover:text-sky-100 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-300"
        >
          {busy ? 'Trying…' : 'Try again'}
        </button>
      )}
      {error && (
        <span data-held-error="" role="alert" className="basis-full text-[12px] text-rose-300">
          {error}
        </span>
      )}
    </div>
  );
};
