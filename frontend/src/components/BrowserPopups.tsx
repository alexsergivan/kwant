import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { Check, Globe } from 'lucide-react';
import { PageDialog, Rect, SelectRequest, Size, matchOption, nextEnabled, popoverPlacement } from '../browserInput';

const PAGE_STEP = 8;
const TYPEAHEAD_MS = 500;
const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-300';
export const PRIMARY_BUTTON = `bg-[#6c58e0] hover:bg-[#5845bd] text-white font-semibold ${focusRing}`;
export const GHOST_BUTTON = `bg-[rgb(44_37_82/0.8)] hover:bg-[#2c2552] border border-[#3f3868] text-zinc-100 ${focusRing}`;

/**
 * A page's <select>, drawn by us: the screencast can't show the native list. Arrows, Home/End,
 * PageUp/Down and type-ahead move; Enter or a click picks; Escape or a click outside closes.
 * `anchor` is the field's box in the page area; null shows a bottom sheet (phones).
 */
export function SelectPopover({
  request,
  anchor,
  stage,
  onPick,
  onClose,
}: {
  request: SelectRequest;
  anchor: Rect | null;
  stage: Size;
  onPick: (indexes: number[]) => void;
  onClose: () => void;
}) {
  const { options, multiple } = request;
  const first = Math.max(0, options.findIndex((o) => o.selected && !o.disabled));
  const [active, setActive] = useState(options[first]?.disabled ? nextEnabled(options, -1, 1) : first);
  const [chosen, setChosen] = useState<Set<number>>(() => new Set(options.flatMap((o, i) => (o.selected ? [i] : []))));
  const listRef = useRef<HTMLUListElement>(null);
  const typed = useRef({ text: '', at: 0 });
  const id = useId();
  const sheet = !anchor;

  useLayoutEffect(() => {
    listRef.current?.focus({ preventScroll: true });
  }, []);
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const pick = (i: number) => {
    if (options[i]?.disabled) return;
    if (!multiple) return onPick([i]);
    setChosen((s) => {
      const n = new Set(s);
      if (n.has(i)) n.delete(i);
      else n.add(i);
      return n;
    });
  };
  const moveBy = (from: number, n: number) => {
    let a = from;
    for (let k = 0; k < Math.abs(n); k++) a = nextEnabled(options, a, n > 0 ? 1 : -1);
    return a;
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === 'ArrowDown') setActive((a) => nextEnabled(options, a, 1));
    else if (e.key === 'ArrowUp') setActive((a) => nextEnabled(options, a, -1));
    else if (e.key === 'Home') setActive(nextEnabled(options, -1, 1));
    else if (e.key === 'End') setActive(nextEnabled(options, options.length, -1));
    else if (e.key === 'PageDown') setActive((a) => moveBy(a, PAGE_STEP));
    else if (e.key === 'PageUp') setActive((a) => moveBy(a, -PAGE_STEP));
    else if (e.key === 'Enter' || (e.key === ' ' && multiple && !typed.current.text)) {
      if (multiple && e.key === 'Enter') onPick([...chosen].sort((a, b) => a - b));
      else pick(active);
    } else if (e.key === 'Escape' || e.key === 'Tab') onClose();
    else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const now = e.timeStamp;
      const t = typed.current;
      t.text = now - t.at > TYPEAHEAD_MS ? e.key : t.text + e.key;
      t.at = now;
      setActive((a) => matchOption(options, t.text, a));
    } else return;
    e.preventDefault();
  };

  const place = anchor ? popoverPlacement(anchor, stage, options.length * 32 + 8 + 30) : null;

  const list = (
    <ul
      ref={listRef}
      id={id}
      role="listbox"
      aria-multiselectable={multiple || undefined}
      aria-activedescendant={`${id}-${active}`}
      aria-label={request.label || 'Choose an option'}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className={`flex-1 min-h-0 overflow-y-auto focus:outline-none ${sheet ? 'px-2 py-1' : 'p-1'}`}
    >
      {options.map((o, i) => {
        const on = multiple ? chosen.has(i) : o.selected;
        const isActive = i === active && !o.disabled;
        return (
          <li
            key={i}
            id={`${id}-${i}`}
            data-index={i}
            role="option"
            aria-selected={on}
            aria-disabled={o.disabled || undefined}
            onMouseEnter={() => !o.disabled && setActive(i)}
            onClick={() => pick(i)}
            className={`flex items-center gap-2 pl-2 pr-2.5 rounded-md cursor-default ${sheet ? 'h-12 text-base' : 'h-8 text-sm'} ${
              o.disabled ? 'text-[#6f6898]' : 'text-zinc-100'
            } ${isActive ? 'bg-[#313868] shadow-[inset_2px_0_0_#7cc4ff]' : ''}`}
          >
            <span className="w-4 shrink-0 flex items-center justify-center text-sky-200" aria-hidden="true">
              {multiple ? (
                <span className={`w-3.5 h-3.5 rounded-[3px] border flex items-center justify-center ${on ? 'bg-[#6c58e0] border-[#6c58e0] text-white' : 'border-[#5a5383]'}`}>
                  {on && <Check size={11} strokeWidth={3} />}
                </span>
              ) : (
                on && <Check size={14} />
              )}
            </span>
            <span className="truncate">{o.label || ' '}</span>
          </li>
        );
      })}
    </ul>
  );

  if (sheet) {
    return (
      <div
        className="fixed inset-0 z-[80] flex items-end bg-[rgb(5_3_15/0.6)]"
        data-browser-popup
        onPointerDown={(e) => {
          e.stopPropagation();
          if (e.target === e.currentTarget) onClose();
        }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <SheetBody title={request.label || 'Choose an option'} onClose={onClose}>
          {list}
          {multiple && (
            <div className="shrink-0 flex gap-2 px-3 pt-2">
              <button type="button" onClick={onClose} className={`flex-1 h-11 rounded-[10px] text-sm ${GHOST_BUTTON}`}>
                Cancel
              </button>
              <button type="button" onClick={() => onPick([...chosen].sort((a, b) => a - b))} className={`flex-1 h-11 rounded-[10px] text-sm ${PRIMARY_BUTTON}`}>
                Done
              </button>
            </div>
          )}
        </SheetBody>
      </div>
    );
  }

  return (
    <div
      className="absolute inset-0 z-40"
      data-browser-popup
      onPointerDown={(e) => {
        e.stopPropagation();
        if (e.target === e.currentTarget) onClose();
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    >
      <div
        data-select-popover
        className="absolute flex flex-col rounded-[10px] border border-white/[0.14] bg-[#231d4b] shadow-[0_12px_32px_rgb(0_0_0/0.45)] overflow-hidden kw-pop-in"
        style={place ? { left: place.left, top: place.top, width: place.width, maxHeight: place.maxHeight } : undefined}
      >
        {list}
        <div className="shrink-0 hidden lg:flex items-center gap-2 px-2.5 h-[30px] border-t border-white/[0.07] text-[11px] text-zinc-400">
          <span className="flex-1 truncate">
            <Kbd>↑↓</Kbd> move · <Kbd>Enter</Kbd> {multiple ? 'done' : 'pick'} · <Kbd>Esc</Kbd> close
          </span>
          {multiple && (
            <button type="button" onClick={() => onPick([...chosen].sort((a, b) => a - b))} className={`h-6 px-2 rounded-md text-xs ${PRIMARY_BUTTON}`}>
              Done
            </button>
          )}
        </div>
        {multiple && (
          <div className="shrink-0 lg:hidden flex justify-end p-1.5 border-t border-white/[0.07]">
            <button type="button" onClick={() => onPick([...chosen].sort((a, b) => a - b))} className={`h-8 px-3 rounded-[10px] text-sm ${PRIMARY_BUTTON}`}>
              Done
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

const Kbd = ({ children }: { children: React.ReactNode }) => (
  <kbd className="inline-block px-1 rounded bg-[#2c2552] font-sans text-[10px] text-zinc-300">{children}</kbd>
);

/** A bottom sheet with a grabber: swiping it down closes it. */
function SheetBody({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  const start = useRef<number | null>(null);
  const [drag, setDrag] = useState(0);
  return (
    <div
      role="dialog"
      aria-label={title}
      data-select-sheet
      className="kw-sheet w-full max-h-[60vh] flex flex-col rounded-t-2xl bg-[#1a1538] border-t border-white/[0.14] shadow-2xl"
      style={{ transform: drag ? `translateY(${drag}px)` : undefined }}
    >
      <div
        className="shrink-0 pt-2 pb-1 touch-none"
        onPointerDown={(e) => {
          start.current = e.clientY;
          e.currentTarget.setPointerCapture?.(e.pointerId);
        }}
        onPointerMove={(e) => start.current !== null && setDrag(Math.max(0, e.clientY - start.current))}
        onPointerUp={() => {
          if (drag > 60) onClose();
          start.current = null;
          setDrag(0);
        }}
        onPointerCancel={() => {
          start.current = null;
          setDrag(0);
        }}
      >
        <div className="mx-auto w-9 h-1 rounded-full bg-[#3f3868]" aria-hidden="true" />
        <p className="px-4 pt-2.5 pb-1 text-[13px] text-[#a39dcc] truncate">{title}</p>
      </div>
      {children}
    </div>
  );
}

/** A page's alert, confirm, prompt or "leave this page?", answered here; the page waits on it. */
export function PageDialogModal({ dialog, host, onAnswer }: { dialog: PageDialog; host: string; onAnswer: (accept: boolean, text?: string) => void }) {
  const [text, setText] = useState(dialog.default_prompt);
  const okRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const messageId = useId();
  const cancellable = dialog.kind !== 'alert';
  const leave = dialog.kind === 'beforeunload';

  useLayoutEffect(() => {
    if (dialog.kind === 'prompt') inputRef.current?.select();
    else okRef.current?.focus({ preventScroll: true });
  }, [dialog.kind]);

  const button = 'h-8 [@media(pointer:coarse)]:h-11 px-3 rounded-[10px] text-[13px] max-sm:flex-1';

  return (
    <div
      className="absolute inset-0 z-50 flex justify-center items-start px-4 sm:px-0 pt-24 bg-[rgb(10_7_25/0.6)]"
      data-browser-popup
      onPointerDown={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    >
      <form
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={dialog.message ? messageId : undefined}
        data-escape-layer
        data-page-dialog
        onSubmit={(e) => {
          e.preventDefault();
          onAnswer(true, dialog.kind === 'prompt' ? text : undefined);
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === 'Escape') {
            e.preventDefault();
            onAnswer(!cancellable);
          }
        }}
        className="w-full sm:w-[420px] rounded-[14px] border border-white/[0.14] bg-[#1a1538] p-4 shadow-[0_24px_64px_rgb(0_0_0/0.55)] text-zinc-100 kw-pop-in"
      >
        <p id={titleId} className="flex items-center gap-1.5 text-xs text-[#a39dcc] min-w-0">
          <Globe size={14} className="shrink-0" aria-hidden="true" />
          <span className="truncate">
            <span className="font-semibold text-zinc-100">{host}</span> {leave ? 'asks if you want to leave' : 'asks'}
          </span>
        </p>
        {dialog.message && (
          <p id={messageId} className="mt-3 text-[15px] leading-[22px] whitespace-pre-wrap break-words max-h-60 overflow-y-auto">
            {dialog.message}
          </p>
        )}
        {leave && !dialog.message && <p className="mt-3 text-[15px] leading-[22px]">Changes you made may not be saved.</p>}
        {dialog.kind === 'prompt' && (
          <input
            ref={inputRef}
            value={text}
            onChange={(e) => setText(e.target.value)}
            aria-label="Answer"
            className="mt-3 w-full h-9 px-3 rounded-[10px] bg-[#120e26] border border-white/[0.14] text-sm focus:outline-none focus:ring-2 focus:ring-sky-300"
          />
        )}
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <p className="basis-full sm:basis-auto sm:flex-1 text-xs text-zinc-400">The page is paused until you answer.</p>
          {cancellable && (
            <button type="button" onClick={() => onAnswer(false)} className={`${button} ${GHOST_BUTTON}`}>
              {leave ? 'Stay' : 'Cancel'}
            </button>
          )}
          <button ref={okRef} type="submit" className={`${button} ${PRIMARY_BUTTON}`}>
            {leave ? 'Leave' : 'OK'}
          </button>
        </div>
      </form>
    </div>
  );
}
