// The message overflow (continue-chat, brief "Frontend (Iris)" 2-5): a quiet ⋯ beside Copy holding
// "Continue in a new chat from here". Enter, Space or a click opens it with the first item focused, the arrow
// keys, Home and End move, Esc or Tab closes it, and Esc gives focus back to ⋯. 44 px targets on phones.

import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { MessageSquarePlus, MoreHorizontal } from 'lucide-react';
import { useEscapeLayer } from '../escape';

export interface MoreItem {
  label: string;
  /** Shown in place of the label while it runs; the item is disabled then. */
  busyLabel?: string;
  busy?: boolean;
  run: () => void;
}

export const MessageMore: React.FC<{
  messageId: string;
  /** Who wrote it, for the button's name: "More for your message", "More for Cody's reply". */
  about: string;
  items: MoreItem[];
  /** 'end' for the person's messages (right side), 'start' for replies. */
  align?: 'start' | 'end';
}> = ({ messageId, about, items, align = 'start' }) => {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = `more-${messageId}`;
  // Opens toward the message's side, nudged once so it stays inside the window (a reply's ⋯ at 390 px).
  const [shift, setShift] = useState(0);
  useLayoutEffect(() => {
    if (!open) {
      setShift(0);
      return;
    }
    const r = menuRef.current?.getBoundingClientRect();
    if (!r || !r.width || typeof window === 'undefined') return;
    const room = window.innerWidth - 8;
    setShift(r.right > room ? Math.max(8 - r.left, room - r.right) : r.left < 8 ? 8 - r.left : 0);
  }, [open]);
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) btnRef.current?.focus();
  };
  useEscapeLayer(wrapRef, open, () => close(true));

  useEffect(() => {
    if (!open) return;
    wrapRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]:not([disabled])')?.focus();
    const away = (e: MouseEvent | TouchEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', away);
    document.addEventListener('touchstart', away);
    return () => {
      document.removeEventListener('mousedown', away);
      document.removeEventListener('touchstart', away);
    };
  }, [open]);

  const onMenuKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Tab') {
      setOpen(false);
      return;
    }
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Home' && e.key !== 'End') return;
    const list = [...(wrapRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])') ?? [])];
    if (!list.length) return;
    e.preventDefault();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? list.length - 1 : (at + (e.key === 'ArrowDown' ? 1 : -1) + list.length) % list.length;
    list[next].focus();
  };

  const onButtonKey = (e: React.KeyboardEvent) => {
    // ArrowDown or ArrowUp on the closed button opens it too, as menus do
    if (!open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault();
      setOpen(true);
    }
  };

  return (
    <span ref={wrapRef} className="relative inline-flex shrink-0" data-message-more="">
      <button
        ref={btnRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={`More for ${about}`}
        title="More"
        data-message-more-button=""
        onClick={() => setOpen((o) => !o)}
        onKeyDown={onButtonKey}
        className={`inline-flex h-11 w-11 sm:h-8 sm:w-8 items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100 transition-colors duration-[120ms] motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 cursor-pointer ${
          open ? 'bg-zinc-800 text-zinc-100' : ''
        }`}
      >
        <MoreHorizontal size={15} aria-hidden="true" />
      </button>
      {open && (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label={`More for ${about}`}
          onKeyDown={onMenuKey}
          data-message-more-menu=""
          style={shift ? { transform: `translateX(${shift}px)` } : undefined}
          className={`absolute bottom-full z-30 mb-1 w-max max-w-[calc(100vw-2rem)] rounded-lg border border-hairline-strong bg-surface-overlay p-1 shadow-xl ${
            align === 'end' ? 'right-0' : 'left-0'
          }`}
        >
          {items.map((it) => (
            <button
              key={it.label}
              type="button"
              role="menuitem"
              disabled={it.busy}
              aria-disabled={it.busy || undefined}
              onClick={() => {
                // Focus goes back to ⋯, so an error line lands next to it; a new chat moves it to the composer after.
                close(true);
                it.run();
              }}
              className="flex w-full items-center gap-2 rounded-md px-2.5 min-h-[44px] sm:min-h-8 text-left text-[13px] sm:text-xs text-zinc-200 hover:bg-zinc-800 focus:bg-zinc-800 focus:outline-none focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-400 disabled:opacity-50 disabled:hover:bg-transparent cursor-pointer disabled:cursor-default"
            >
              <MessageSquarePlus size={14} aria-hidden="true" className="shrink-0 text-zinc-400" />
              <span className="whitespace-normal">{it.busy && it.busyLabel ? it.busyLabel : it.label}</span>
            </button>
          ))}
        </div>
      )}
    </span>
  );
};

/** The line by the message when the request fails; polite, so it's read once. */
export const MessageMoreError: React.FC<{ text: string; align?: 'start' | 'end' }> = ({ text, align = 'start' }) => (
  <p role="status" data-continue-error="" className={`mt-1 max-w-full text-xs leading-snug text-rose-300 break-words ${align === 'end' ? 'text-right' : ''}`}>
    {text}
  </p>
);

/** The quiet first line of a continued chat; the link goes back to the pressed message. */
export const ContinuedLine: React.FC<{ text: string; onOpen: () => void }> = ({ text, onOpen }) => (
  <div data-continued-from="" className="mx-auto mb-2 flex w-full max-w-5xl justify-center px-1">
    <button
      type="button"
      onClick={onOpen}
      className="inline-flex min-h-[44px] sm:min-h-7 max-w-full items-center rounded-md px-2 text-center text-xs leading-snug text-zinc-400 underline decoration-zinc-600 underline-offset-2 hover:text-zinc-200 hover:decoration-zinc-400 transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 break-words whitespace-normal cursor-pointer"
    >
      {text}
    </button>
  </div>
);
