// The one Dismiss-with-a-reason form (design §5.6, api-decisions 25). Findings and Matches rows both use it:
// `useDismiss` holds the draft (dismissReducer) and posts `{action:"dismiss", reason, note?}` through the review
// card's `act`; `DismissForm` draws the reasons, the optional note and the error. Nothing here is copied per row.

import React, { useReducer } from 'react';
import type { DevFinding, FindingAction } from '../../dev/types';
import { DISMISS_REASONS, closedDraft, dismissReducer } from '../../dev/logic';
import type { DismissDraft, DismissEvent } from '../../dev/logic';

export interface Dismiss {
  draft: DismissDraft;
  send: React.Dispatch<DismissEvent>;
  submit: (e: React.FormEvent) => Promise<void>;
}

/** The draft for one row's Dismiss, and its submit. A failed post keeps the form open with the error. */
export function useDismiss(f: DevFinding | null, act: ((f: DevFinding, a: FindingAction) => Promise<void> | void) | undefined): Dismiss {
  const [draft, send] = useReducer(dismissReducer, closedDraft);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!f || !act || !draft.reason || draft.sending) return;
    send({ type: 'submit' });
    try {
      const note = draft.note.trim();
      await act(f, { action: 'dismiss', reason: draft.reason, ...(note ? { note } : {}) });
      send({ type: 'done' });
    } catch (err) {
      send({ type: 'failed', error: err instanceof Error ? err.message : 'That didn’t go through. Try again.' });
    }
  };
  return { draft, send, submit };
}

export const DismissForm: React.FC<{
  id: string;
  /** What is being dismissed, for the form's accessible name. */
  title: string;
  d: Dismiss;
  submitLabel?: string;
  className?: string;
}> = ({ id, title, d, submitLabel = 'Dismiss finding', className = 'mt-2' }) => {
  const { draft, send, submit } = d;
  if (!draft.open) return null;
  return (
    <form id={id} onSubmit={submit} aria-label={`Dismiss: ${title}`} data-dismiss-form className={`${className} rounded-lg border border-hairline bg-surface-main p-2.5 text-xs`}>
      <fieldset disabled={draft.sending} className="min-w-0">
        <legend className="text-meta text-zinc-200">Why dismiss? Rigel learns from this.</legend>
        <div role="radiogroup" aria-label="Reason" className="mt-2 flex flex-wrap gap-1.5">
          {DISMISS_REASONS.map((r) => {
            const on = draft.reason === r.id;
            return (
              <button
                key={r.id}
                type="button"
                role="radio"
                aria-checked={on}
                data-reason={r.id}
                onClick={() => send({ type: 'reason', reason: r.id })}
                className={`kw-focus min-h-[44px] rounded-full border px-3 text-xs sm:min-h-[28px] ${on ? 'border-sky-400 bg-sky-500/15 text-zinc-50' : 'border-hairline text-zinc-300 hover:border-hairline-strong'}`}
              >
                {r.label}
              </button>
            );
          })}
        </div>
        <label className="mt-2 block text-2xs text-zinc-400">
          Note (optional)
          <textarea
            value={draft.note}
            onChange={(e) => send({ type: 'note', note: e.target.value })}
            rows={2}
            maxLength={500}
            className="kw-focus mt-1 block w-full resize-y rounded-lg border border-hairline bg-surface-main px-2 py-1.5 text-xs text-zinc-100"
          />
        </label>
        {draft.error && (
          <p role="alert" className="mt-1 text-bad">
            {draft.error}
          </p>
        )}
        <div className="mt-2 flex flex-wrap gap-2">
          <button type="submit" disabled={!draft.reason} aria-busy={draft.sending || undefined} className="kw-btn-primary min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
            {draft.sending ? 'Dismissing…' : submitLabel}
          </button>
          <button type="button" onClick={() => send({ type: 'cancel' })} className="kw-btn-ghost min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
            Cancel
          </button>
        </div>
      </fieldset>
    </form>
  );
};
