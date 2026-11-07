// 0.7.7 Watch live (W1, W3; design/w0-answers.md): the globe on a sidebar row, Watch on the working
// strip, and the browser panel's switcher between chats whose browser is active.
import React, { useEffect, useId, useRef, useState } from 'react';
import { Check, ChevronDown, Eye, Globe, Hash, Users } from 'lucide-react';
import type { SwitchEntry } from '../watchLive';
import { watchLabel } from '../watchLive';
import { ModernAvatar } from './ModernAvatar';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]';

/**
 * W1: 12 px globe at the end of a row's name line, before any count. Hit area 24 px (44 on a phone).
 * Fades in over 150 ms, opacity only. The row decides when it shows (not while it needs you).
 */
export const WatchGlobe: React.FC<{ name: string; host: string; phone?: boolean; onWatch: () => void }> = ({ name, host, phone, onWatch }) => {
  const label = watchLabel(name, host);
  return (
    <button
      type="button"
      data-watch-globe=""
      onClick={(e) => {
        e.stopPropagation();
        onWatch();
      }}
      onKeyDown={(e) => e.stopPropagation()}
      aria-label={label}
      title={label}
      className={`kw-globe-in shrink-0 inline-flex items-center justify-center rounded-md text-[#9d8fff] hover:bg-[rgb(157_143_255/.10)] ${phone ? 'h-11 w-11 -my-3' : 'h-6 w-6 -my-0.5'} ${FOCUS}`}
    >
      <Globe size={12} aria-hidden="true" />
    </button>
  );
};

/** W1: "Watch" on the working strip while this chat's browser is active and the panel is closed. */
export const WatchButton: React.FC<{ name: string; phone?: boolean; onWatch: () => void }> = ({ name, phone, onWatch }) => (
  <button
    type="button"
    data-watch-button=""
    onClick={onWatch}
    aria-label={`Watch the page ${name} is on`}
    className={`textbtn shrink-0 inline-flex items-center gap-1 rounded-lg px-2 font-medium text-[#b8adff] hover:bg-[rgb(157_143_255/.10)] ${phone ? 'min-h-[44px] text-sm' : 'min-h-8 text-xs'} ${FOCUS}`}
  >
    <Eye size={13} aria-hidden="true" />
    Watch
  </button>
);

const EntryIcon: React.FC<{ e: SwitchEntry; size: number }> = ({ e, size }) =>
  e.kind === 'bot' ? (
    <ModernAvatar role={e.sprite} name={e.label} size={size} />
  ) : (
    <span className="inline-flex items-center justify-center rounded-full bg-zinc-800 text-zinc-300" style={{ width: size, height: size }} aria-hidden="true">
      {e.kind === 'team' ? <Users size={Math.round(size * 0.55)} /> : <Hash size={Math.round(size * 0.55)} />}
    </span>
  );

const shown = (e: SwitchEntry) => (e.kind === 'bot' ? e.label : e.kind === 'team' ? `@${e.label}` : `#${e.label}`);

interface SwitcherProps {
  entries: SwitchEntry[];
  current: string;
  /** During Take over: "Hand back first". */
  disabled?: boolean;
  onPick: (owner: string) => void;
}

/**
 * W3 desktop (Q2 a): with 2+ browsing chats the header's "@Rook" is a menu button. Picking a row opens
 * that chat with the panel open. Arrow keys move between rows, Esc closes and returns focus.
 */
export const BrowserSwitcherMenu: React.FC<SwitcherProps & { title: string }> = ({ entries, current, disabled, onPick, title }) => {
  const [open, setOpen] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    const items = () => Array.from(menu.current?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? []);
    (items().find((el) => el.getAttribute('aria-checked') === 'true') ?? items()[0])?.focus();
    const onDown = (e: MouseEvent) => {
      if (!menu.current?.contains(e.target as Node) && !btn.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);
  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);
  const onKey = (e: React.KeyboardEvent) => {
    const list = Array.from(menu.current?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? []);
    const i = list.indexOf(document.activeElement as HTMLElement);
    if (e.key === 'Escape') {
      e.stopPropagation();
      setOpen(false);
      btn.current?.focus();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = list.length;
      list[(i + (e.key === 'ArrowDown' ? 1 : n - 1)) % n]?.focus();
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      list[e.key === 'Home' ? 0 : list.length - 1]?.focus();
    } else if (e.key === 'Tab') setOpen(false);
  };
  return (
    <div className="relative min-w-0">
      <button
        ref={btn}
        type="button"
        data-browser-switcher=""
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-disabled={disabled || undefined}
        title={disabled ? 'Hand back first' : `Switch browser: ${entries.length} chats are browsing`}
        onClick={() => !disabled && setOpen((o) => !o)}
        className={`flex items-center gap-1 min-w-0 max-w-full rounded-md -mx-1 px-1 text-xs font-semibold text-zinc-100 ${disabled ? 'cursor-not-allowed opacity-60' : 'hover:bg-white/[0.06]'} ${FOCUS}`}
      >
        <span className="truncate">{title}</span>
        <ChevronDown size={12} className="shrink-0 text-zinc-400" aria-hidden="true" />
      </button>
      {open && (
        <div
          ref={menu}
          id={id}
          role="menu"
          aria-label="Browsers in use"
          onKeyDown={onKey}
          className="absolute left-0 top-full mt-1.5 z-40 min-w-[240px] max-w-[320px] rounded-[10px] border border-white/10 bg-[#231d4b] p-1 shadow-xl"
        >
          {entries.map((e) => {
            const here = e.owner === current;
            return (
              <button
                key={e.owner}
                type="button"
                role="menuitemradio"
                aria-checked={here}
                data-switch-owner={e.owner}
                onClick={() => {
                  setOpen(false);
                  if (!here) onPick(e.owner);
                }}
                className={`w-full flex items-center gap-2 min-h-9 px-2 rounded-md text-left hover:bg-white/[0.06] focus-visible:bg-white/[0.08] focus-visible:outline-none`}
              >
                <EntryIcon e={e} size={20} />
                <span className="min-w-0 flex-1 truncate text-[13px] text-zinc-100">{shown(e)}</span>
                {e.needsYou && <span className="sr-only">, needs you</span>}
                <span className="shrink-0 max-w-[120px] truncate font-mono text-[11px] text-zinc-400">{e.host}</span>
                <Check size={14} className={`shrink-0 ${here ? 'text-[#b8adff]' : 'invisible'}`} aria-hidden="true" />
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
};

/** W3 phone: a sideways row of 32 px avatars (44 px to tap) across the top of the full-screen panel. */
export const BrowserSwitcherRow: React.FC<SwitcherProps> = ({ entries, current, disabled, onPick }) => (
  <div role="group" aria-label="Browsers in use" data-browser-switcher-row="" className="kw-scroll-row flex items-center gap-1 overflow-x-auto px-2 py-1 border-b border-zinc-800 bg-zinc-900/60" title={disabled ? 'Hand back first' : undefined}>
    {entries.map((e) => {
      const here = e.owner === current;
      return (
        <button
          key={e.owner}
          type="button"
          data-switch-owner={e.owner}
          aria-label={`${shown(e)}, on ${e.host || 'a page'}`}
          aria-current={here || undefined}
          aria-disabled={disabled || undefined}
          onClick={() => !disabled && !here && onPick(e.owner)}
          className={`shrink-0 inline-flex items-center justify-center h-11 w-11 rounded-full ${disabled ? 'opacity-60 cursor-not-allowed' : ''} ${FOCUS}`}
        >
          <span className={`inline-flex rounded-full ${here ? 'ring-2 ring-[#b8adff] ring-offset-1 ring-offset-zinc-900' : ''}`}>
            <EntryIcon e={e} size={32} />
          </span>
        </button>
      );
    })}
  </div>
);
