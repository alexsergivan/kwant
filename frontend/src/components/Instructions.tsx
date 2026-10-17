import React, { useId, useState } from 'react';
import { ChevronDown, ChevronRight, Pencil, Pin } from 'lucide-react';
import { MAX_INSTRUCTIONS } from '../continuity';

/** Instructions textarea with a live character counter; turns red past the limit. */
export const InstructionsField: React.FC<{
  value: string;
  onChange: (v: string) => void;
  label: string;
  hint: string;
  rows?: number;
  autoFocus?: boolean;
}> = ({ value, onChange, label, hint, rows = 5, autoFocus }) => {
  const id = useId();
  const over = value.length > MAX_INSTRUCTIONS;
  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <label htmlFor={id} className="text-xs font-medium text-zinc-400">
          {label}
        </label>
        <span id={`${id}-count`} className={`text-[10px] font-mono ${over ? 'text-rose-400' : 'text-zinc-500'}`} aria-live="polite">
          {value.length}/{MAX_INSTRUCTIONS}
        </span>
      </div>
      <textarea
        id={id}
        rows={rows}
        value={value}
        autoFocus={autoFocus}
        onChange={(e) => onChange(e.target.value)}
        aria-describedby={`${id}-hint ${id}-count`}
        aria-invalid={over}
        placeholder="e.g. Answer in British English. Our repo is kwant-rust; never push to main."
        className={`w-full bg-zinc-950 border rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:outline-none transition-colors resize-y ${
          over ? 'border-rose-500/70 focus:border-rose-500' : 'border-zinc-800 focus:border-sky-500'
        }`}
      />
      <p id={`${id}-hint`} className="mt-1 text-[11px] text-zinc-500">
        {over ? `${value.length - MAX_INSTRUCTIONS} characters over the limit.` : hint}
      </p>
    </div>
  );
};

export interface PinnedItem {
  label: string;
  text: string;
}

/** Pinned team/channel instructions at the top of a chat, collapsed to one line until opened. */
export const PinnedInstructions: React.FC<{ items: PinnedItem[]; onEdit?: () => void }> = ({ items, onEdit }) => {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  if (items.length === 0) return null;
  const preview = items.map((i) => i.text.replace(/\s+/g, ' ')).join(' · ');
  return (
    <div className="px-3 sm:px-6 pt-3 flex-shrink-0">
      <div className="rounded-xl border border-amber-500/20 bg-amber-500/5 text-xs">
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            aria-controls={panelId}
            className="flex-1 min-w-0 flex items-center gap-2 px-3 py-2 text-left text-amber-100/90 hover:text-amber-50 rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            {open ? <ChevronDown size={13} className="shrink-0 text-amber-300" /> : <ChevronRight size={13} className="shrink-0 text-amber-300" />}
            <Pin size={12} className="shrink-0 text-amber-300" aria-hidden="true" />
            <span className="font-medium shrink-0">Pinned</span>
            {!open && <span className="truncate text-zinc-400">{preview}</span>}
          </button>
          {onEdit && (
            <button
              type="button"
              onClick={onEdit}
              aria-label="Edit pinned instructions"
              title="Edit pinned instructions"
              className="mr-1.5 p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
            >
              <Pencil size={12} />
            </button>
          )}
        </div>
        {open && (
          <div id={panelId} className="px-3 pb-3 space-y-2 max-h-60 overflow-y-auto">
            {items.map((i) => (
              <div key={i.label}>
                <p className="text-[10px] uppercase tracking-wide text-amber-300/80 font-semibold">{i.label}</p>
                <p className="text-zinc-300 whitespace-pre-wrap break-words">{i.text}</p>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
