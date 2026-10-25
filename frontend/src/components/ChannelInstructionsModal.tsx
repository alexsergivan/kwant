import React, { useEffect, useRef, useState } from 'react';
import { Pin, X } from 'lucide-react';
import { MAX_INSTRUCTIONS } from '../continuity';
import { playSfx } from '../audio/chiptune';
import { InstructionsField } from './Instructions';

/** Edits one channel's pinned instructions. */
export const ChannelInstructionsModal: React.FC<{
  channelLabel: string | null;
  initial: string;
  onClose: () => void;
  onSave: (text: string) => Promise<string | void>;
}> = ({ channelLabel, initial, onClose, onSave }) => {
  const [text, setText] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    setText(initial);
    setError(null);
  }, [initial, channelLabel]);

  useEffect(() => {
    if (!channelLabel) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeRef.current();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [channelLabel]);

  if (!channelLabel) return null;
  const over = text.length > MAX_INSTRUCTIONS;

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (over) return;
    setSaving(true);
    setError(null);
    const err = await onSave(text.trim());
    setSaving(false);
    if (err) setError(err);
    else {
      playSfx('coin');
      onClose();
    }
  };

  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-2 sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form
        role="dialog"
        aria-modal="true"
        aria-labelledby="chan-instr-title"
        onSubmit={submit}
        className="kw-dialog-panel bg-zinc-900 border border-zinc-800 rounded-2xl max-w-lg w-full max-h-[94vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-4"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 pb-3">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-amber-500/10 border border-amber-500/20 flex items-center justify-center text-amber-300">
              <Pin size={14} />
            </div>
            <h2 id="chan-instr-title" className="text-sm font-semibold">
              Instructions for {channelLabel}
            </h2>
          </div>
          <button type="button" onClick={onClose} aria-label="Close" className="p-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors">
            <X size={17} />
          </button>
        </div>
        <InstructionsField
          value={text}
          onChange={setText}
          autoFocus
          label="Channel instructions"
          hint="Every bot reads these before each turn in this channel. Leave empty to unpin."
          rows={7}
        />
        {error && (
          <p className="text-[11px] text-rose-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex items-center justify-end gap-2.5 pt-2 border-t border-zinc-800">
          <button type="button" onClick={onClose} className="px-3.5 py-2 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors">
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving || over}
            className="px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-50"
          >
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </form>
    </div>
  );
};
