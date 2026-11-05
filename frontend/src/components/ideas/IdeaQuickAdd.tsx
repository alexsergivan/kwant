import { forwardRef, useId, useImperativeHandle, useRef, useState } from 'react';
import { ArrowRight, CornerDownLeft } from 'lucide-react';
import { IDEA_MAX, IdeaError, type Idea, dropIdea } from '../../ideas';
import { Spinner, btn, focusRing } from './parts';

export interface IdeaQuickAddHandle {
  /** Fills the field (the empty state's example chips) without dropping it. */
  fill: (text: string) => void;
  focus: () => void;
}

export interface IdeaQuickAddProps {
  spaceId: string;
  /** hero: the jar view's 48 px field with Drop in. phone: 52 px with an arrow button. sidebar: one line under the row. */
  variant?: 'hero' | 'phone' | 'sidebar';
  onDropped?: (idea: Idea) => void;
  /** Sidebar: Esc, or blur while empty, closes the field. */
  onClose?: () => void;
  autoFocus?: boolean;
}

export const dropErrorText = (e: unknown): string => {
  if (e instanceof IdeaError && e.status === 403) return 'Only you can add ideas.';
  if (e instanceof IdeaError && e.status === 400 && e.message && !/^Something went wrong/.test(e.message)) return e.message;
  return 'Couldn’t drop it in the jar. Try again.';
};

/** "Drop an idea…": Enter drops it, the field stays open and empty for the next one. */
export const IdeaQuickAdd = forwardRef<IdeaQuickAddHandle, IdeaQuickAddProps>(({ spaceId, variant = 'hero', onDropped, onClose, autoFocus = false }, ref) => {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement | null>(null);
  const id = useId();
  useImperativeHandle(ref, () => ({
    fill: (t: string) => {
      setText(t);
      setError(null);
      input.current?.focus();
    },
    focus: () => input.current?.focus(),
  }));
  const len = [...text].length;
  const over = len > IDEA_MAX;
  const empty = !text.trim();
  const submit = async () => {
    if (empty || over || busy) return;
    setBusy(true);
    setError(null);
    try {
      const idea = await dropIdea(spaceId, text.trim());
      setText('');
      onDropped?.(idea);
    } catch (e) {
      setError(dropErrorText(e));
    } finally {
      setBusy(false);
      input.current?.focus();
    }
  };
  const sidebar = variant === 'sidebar';
  const phone = variant === 'phone';
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
      className="min-w-0"
      data-quick-add={variant}
    >
      <label htmlFor={id} className="sr-only">
        Drop an idea in the jar
      </label>
      <div
        className={`flex items-center gap-2 rounded-[14px] border bg-[#0f0b22] focus-within:border-[#7c6cf0] focus-within:ring-1 focus-within:ring-[#7c6cf0] ${
          over ? 'border-rose-300/50' : 'border-white/[0.14]'
        } ${sidebar ? 'min-h-[34px] pl-2.5 pr-1.5 py-1 rounded-[10px]' : phone ? 'min-h-[52px] pl-3.5 pr-1.5 py-1.5' : 'min-h-[48px] pl-3.5 pr-1.5 py-1.5'}`}
      >
        <input
          id={id}
          ref={input}
          value={text}
          autoFocus={autoFocus}
          maxLength={IDEA_MAX * 2}
          placeholder="Drop an idea…"
          onChange={(e) => {
            setText(e.target.value);
            if (error) setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape' && onClose) {
              e.preventDefault();
              onClose();
            }
          }}
          onBlur={() => {
            if (sidebar && !text.trim()) onClose?.();
          }}
          aria-invalid={over || undefined}
          aria-describedby={error ? `${id}-err` : undefined}
          className={`min-w-0 flex-1 bg-transparent text-[#f7f6ff] placeholder:text-[#8d87b8] focus:outline-none ${sidebar ? 'text-[13px]' : phone ? 'text-[15px]' : 'text-[14px]'}`}
          enterKeyHint="send"
        />
        {!empty && (
          <span className={`shrink-0 text-[11px] tabular-nums ${over ? 'text-rose-300' : 'text-[#8d87b8]'}`} data-counter aria-live="off">
            {len.toLocaleString('en-US')} / {IDEA_MAX.toLocaleString('en-US')}
          </span>
        )}
        {sidebar ? (
          <button type="submit" disabled={empty || over || busy} aria-label="Drop in" className={`shrink-0 grid place-items-center w-7 h-7 rounded-md text-[#a39dcc] hover:text-[#f7f6ff] disabled:opacity-60 ${focusRing}`}>
            {busy ? <Spinner size={12} /> : <CornerDownLeft size={12} aria-hidden="true" />}
          </button>
        ) : phone ? (
          <button type="submit" disabled={empty || over || busy} aria-label="Drop in" className={`${btn.primary} !min-h-[40px] w-11 !px-0`}>
            {busy ? <Spinner /> : <ArrowRight size={16} aria-hidden="true" />}
          </button>
        ) : (
          <button type="submit" disabled={empty || over || busy} className={`${btn.primary} !min-h-[36px]`} data-drop-in>
            {busy ? <Spinner /> : <ArrowRight size={14} aria-hidden="true" />}
            Drop in
          </button>
        )}
      </div>
      {over && (
        <p className="mt-1.5 text-[12px] text-rose-300" data-too-long>
          {len.toLocaleString('en-US')} / {IDEA_MAX.toLocaleString('en-US')} · too long for the jar
        </p>
      )}
      {error && (
        <p id={`${id}-err`} role="alert" className="mt-1.5 text-[12px] text-rose-300" data-drop-error>
          {error}
        </p>
      )}
    </form>
  );
});
IdeaQuickAdd.displayName = 'IdeaQuickAdd';
