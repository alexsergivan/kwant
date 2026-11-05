import React, { useEffect, useId, useRef, useState } from 'react';
import { Plus } from 'lucide-react';
import { badgeText } from '../away';
import { IDEA_MAX } from '../ideas';
import { JarIcon } from './JarIcon';
import { useJarWobble } from '../ideaShell';

/**
 * The sidebar's Idea jar row (spec §2.1): a nav row with the violet ready-takes badge, and a `+` that
 * opens a one-line "Drop an idea…" field under it. Enter drops and keeps the field open for the next
 * idea; Esc, or blur while empty, closes it.
 */
export const IdeaJarRow: React.FC<{
  active: boolean;
  count: number;
  onOpen: () => void;
  /** Drops one idea. Rejects with an Error whose message is shown under the field. */
  onDrop: (text: string) => Promise<unknown>;
  className: string;
}> = ({ active, count, onOpen, onDrop, className }) => {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const plus = useRef<HTMLButtonElement>(null);
  const wobble = useJarWobble();
  const id = useId();
  useEffect(() => {
    if (open) input.current?.focus();
  }, [open]);
  const close = (refocus: boolean) => {
    setOpen(false);
    setText('');
    setError(null);
    if (refocus) plus.current?.focus();
  };
  const tooLong = text.trim().length > IDEA_MAX;
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const t = text.trim();
    if (!t || busy || tooLong) return;
    setBusy(true);
    setError(null);
    try {
      await onDrop(t);
      setText('');
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : 'Couldn’t drop it in the jar. Try again.');
    } finally {
      setBusy(false);
      input.current?.focus();
    }
  };
  // The + shows on hover and focus, and always on touch; the row makes room for it then.
  const reveal = 'group-hover:opacity-100 group-focus-within:opacity-100 [@media(hover:none)]:opacity-100';
  const room = 'group-hover:pr-10 group-focus-within:pr-10 [@media(hover:none)]:pr-10';
  return (
    <div data-idea-jar-row>
      <div className="group relative">
        <button type="button" onClick={onOpen} aria-current={active ? 'page' : undefined} className={`${className} ${open ? 'pr-10' : room}`} data-nav="jar">
          <span key={wobble} className={`inline-flex ${wobble ? 'kw-jar-wobble' : ''}`}>
            <JarIcon size={15} className={active ? 'text-sky-400' : 'text-zinc-500'} />
          </span>
          <span className="truncate flex-1">Idea jar</span>
          {count > 0 && (
            <span className="shrink-0 min-w-[18px] px-1.5 py-px rounded-full bg-violet-500 text-white text-[10px] font-semibold text-center" data-jar-badge>
              {badgeText(count)}
              <span className="sr-only"> {count === 1 ? 'take' : 'takes'} ready</span>
            </span>
          )}
        </button>
        <button
          ref={plus}
          type="button"
          aria-label="Drop an idea in the jar"
          aria-expanded={open}
          aria-controls={open ? `${id}-form` : undefined}
          onClick={() => (open ? close(true) : setOpen(true))}
          className={`absolute right-0 inset-y-0 w-9 flex items-center justify-center rounded-xl text-zinc-400 hover:text-zinc-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 ${open ? 'opacity-100' : `opacity-0 ${reveal}`}`}
          data-jar-add
        >
          <span className="w-6 h-6 rounded-md flex items-center justify-center bg-white/[0.06] border border-white/10">
            <Plus size={14} aria-hidden="true" />
          </span>
        </button>
      </div>
      {open && (
        <form id={`${id}-form`} onSubmit={submit} className="mt-1 mb-1" data-jar-quick-add>
          <label htmlFor={`${id}-in`} className="sr-only">
            Drop an idea in the jar
          </label>
          <div className="flex items-center gap-2 h-9 px-2.5 rounded-xl border border-violet-400/60 bg-[#120e26] focus-within:border-violet-300">
            <input
              ref={input}
              id={`${id}-in`}
              value={text}
              maxLength={IDEA_MAX + 200}
              onChange={(e) => {
                setText(e.target.value);
                if (error) setError(null);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault();
                  close(true);
                }
              }}
              onBlur={(e) => {
                const to = e.relatedTarget as Node | null;
                if (!text.trim() && !busy && !(to && e.currentTarget.form?.contains(to)) && to !== plus.current) close(false);
              }}
              placeholder="Drop an idea…"
              aria-invalid={!!error || tooLong || undefined}
              aria-describedby={error || tooLong ? `${id}-err` : undefined}
              className="min-w-0 flex-1 bg-transparent text-xs text-zinc-100 placeholder:text-[#8d87b8] outline-none"
              data-jar-quick-input
            />
            <kbd aria-hidden="true" className="shrink-0 rounded border border-hairline-strong px-1 text-2xs font-sans text-zinc-400">
              ↵
            </kbd>
          </div>
          {(error || tooLong) && (
            <p id={`${id}-err`} role="alert" className="mt-1 px-1 text-2xs text-rose-300">
              {tooLong ? `${text.trim().length.toLocaleString('en-US')} / ${IDEA_MAX.toLocaleString('en-US')} · too long for the jar` : error}
            </p>
          )}
        </form>
      )}
    </div>
  );
};
