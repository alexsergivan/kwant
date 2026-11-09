// 0.7.7 Watch live (W1, W3; design/w0-answers.md): the globe on a sidebar row, Watch on the working
// strip, and the browser panel's switcher between chats whose browser is active.
import React, { useEffect, useId, useRef, useState } from 'react';
import { Check, ChevronDown, Eye, Globe, Hash, Users } from 'lucide-react';
import type { Browsing } from '../watchLive';
import type { SwitchEntry } from '../watchLive';
import { watchLabel } from '../watchLive';
import { ModernAvatar } from './ModernAvatar';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]';

const GLOBE_FADE_MS = 150;
const reducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

/**
 * Keeps the last value for the 150 ms fade-out after it goes (none under reduced motion), so the globe
 * fades out as it fades in (QA F2). `leaving` is set during that time.
 */
export function useFadeOut<T>(value: T | null | undefined, ms = GLOBE_FADE_MS): { value: T | undefined; leaving: boolean } {
  const last = useRef<T | undefined>(value ?? undefined);
  if (value) last.current = value;
  const [gone, setGone] = useState(!value);
  const present = !!value;
  useEffect(() => {
    if (present) {
      setGone(false);
      return;
    }
    if (reducedMotion()) {
      setGone(true);
      return;
    }
    const id = window.setTimeout(() => setGone(true), ms);
    return () => window.clearTimeout(id);
  }, [present, ms]);
  if (present) return { value: value as T, leaving: false };
  return gone ? { value: undefined, leaving: false } : { value: last.current, leaving: true };
}

/**
 * W1: 12 px globe at the end of a row's name line (`ml-auto`, before any count). The box is the icon's
 * 12 px, so the row keeps its height; the hit area overflows it: 24 px on desktop, 44 on a phone.
 * Fades in and out over 150 ms, opacity only. The row decides when it shows (not while it needs you).
 */
export const WatchGlobe: React.FC<{ name: string; host: string; phone?: boolean; leaving?: boolean; className?: string; onWatch: () => void }> = ({ name, host, phone, leaving, className = '', onWatch }) => {
  const label = watchLabel(name, host);
  return (
    <button
      type="button"
      data-watch-globe=""
      data-leaving={leaving || undefined}
      tabIndex={leaving ? -1 : undefined}
      aria-hidden={leaving || undefined}
      // Out of the global 44 px touch minimum: the box stays 12 px so the row keeps its height, and the
      // ::after area overflows to 44 px instead (Mira 1).
      data-compact=""
      onClick={(e) => {
        e.stopPropagation();
        if (!leaving) onWatch();
      }}
      onKeyDown={(e) => e.stopPropagation()}
      aria-label={label}
      title={label}
      className={`${leaving ? 'kw-globe-out pointer-events-none' : 'kw-globe-in'} relative shrink-0 inline-flex h-3 w-3 items-center justify-center rounded-sm text-[#9d8fff] after:absolute after:content-[''] after:rounded-md hover:after:bg-[rgb(157_143_255/.10)] ${phone ? 'after:-inset-4' : 'after:-inset-1.5'} ${FOCUS} ${className}`}
    >
      <Globe size={12} aria-hidden="true" />
    </button>
  );
};

/**
 * Ids that just left `ids`, kept for the globe's 150 ms fade-out (none under reduced motion). The
 * phone's Working group lists only busy bots, so without this a row unmounts the moment its turn
 * ends and its globe can't fade (QA F2 on phone).
 */
export function useLingeringIds(ids: readonly string[], ms = GLOBE_FADE_MS): ReadonlySet<string> {
  const key = ids.join('\n');
  const [st, setSt] = useState<{ key: string; ids: readonly string[]; linger: string[] }>({ key, ids, linger: [] });
  let linger = st.linger;
  if (st.key !== key) {
    // Derived from the previous render, so the row stays in this same commit (no unmount, then remount)
    const now = new Set(ids);
    const gone = reducedMotion() ? [] : st.ids.filter((id) => !now.has(id));
    linger = [...new Set([...st.linger.filter((id) => !now.has(id)), ...gone])];
    setSt({ key, ids, linger });
  }
  const lingerKey = st.linger.join('\n');
  useEffect(() => {
    if (!lingerKey) return;
    const id = window.setTimeout(() => setSt((s) => ({ ...s, linger: [] })), ms);
    return () => window.clearTimeout(id);
  }, [lingerKey, ms]);
  return new Set(linger);
}

/** A row's globe slot: the globe while `browsing` is set, and for its fade-out after. */
export const GlobeSlot: React.FC<{ browsing?: Browsing | null; name: string; phone?: boolean; className?: string; onWatch: (owner: string) => void }> = ({ browsing, name, phone, className, onWatch }) => {
  const { value, leaving } = useFadeOut(browsing);
  if (!value) return null;
  return <WatchGlobe name={name} host={value.host} phone={phone} leaving={leaving} className={className} onWatch={() => onWatch(value.owner)} />;
};

/** W1: "Watch" on the working strip while this chat's browser is active and the panel is closed. */
/** phone: the phone strip's muted 28 px button. line: the desktop status line's (28 px, -mr-2 so the word
 * ends where the composer's Stop square does). Otherwise the 32 px desktop button. */
export const WatchButton: React.FC<{ name: string; phone?: boolean; line?: boolean; onWatch: () => void }> = ({ name, phone, line, onWatch }) => (
  <button
    type="button"
    data-watch-button=""
    onClick={onWatch}
    aria-label={`Watch the page ${name} is on`}
    data-compact={phone ? '' : undefined}
    className={
      phone
        ? `textbtn kws-hit -mr-2 shrink-0 inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 ${FOCUS}`
        : line
          ? `textbtn -mr-2 shrink-0 inline-flex h-7 items-center gap-1 rounded-md px-2 font-medium text-[#b8adff] hover:bg-[rgb(157_143_255/.10)] text-xs ${FOCUS}`
          : `textbtn shrink-0 inline-flex items-center gap-1 rounded-lg px-2 font-medium text-[#b8adff] hover:bg-[rgb(157_143_255/.10)] min-h-8 text-xs ${FOCUS}`
    }
  >
    <Eye size={phone ? 12 : 13} aria-hidden="true" />
    Watch
  </button>
);

const EntryIcon: React.FC<{ e: SwitchEntry; size: number }> = ({ e, size }) =>
  e.kind === 'bot' ? (
    <ModernAvatar role={e.sprite} name={e.label} size={size} />
  ) : (
    <span className="inline-flex items-center justify-center rounded-[8px] bg-zinc-800 text-zinc-300" style={{ width: size, height: size }} aria-hidden="true">
      {e.kind === 'team' ? <Users size={Math.round(size * 0.55)} /> : <Hash size={Math.round(size * 0.55)} />}
    </span>
  );

// "@Rook" as the panel header says it (bots and teams), "#ops" for a channel
const shown = (e: SwitchEntry) => (e.kind === 'channel' ? `#${e.label}` : `@${e.label}`);

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
export const BrowserSwitcherMenu: React.FC<SwitcherProps & { title: string; sub?: React.ReactNode }> = ({ entries, current, disabled, onPick, title, sub }) => {
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
  // The menu hangs from the whole two-line title block (name and sub line), never over the sub line
  return (
    <div className="relative min-w-0" data-switcher-block="">
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
      {sub}
      {open && (
        <div
          ref={menu}
          id={id}
          role="menu"
          aria-label="Browsers in use"
          onKeyDown={onKey}
          className="absolute left-0 top-[calc(100%+4px)] z-40 min-w-[240px] max-w-[320px] rounded-[10px] border border-white/10 bg-[#231d4b] p-1 shadow-xl"
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
                <span className="min-w-0 flex-1 flex items-center gap-3">
                  <span className="min-w-0 flex-1 truncate text-[13px] text-zinc-100">{shown(e)}</span>
                  {/* Sorted first because it needs you: the row says so where the host would be */}
                  {e.needsYou ? (
                    <span data-switch-needs-you="" className="shrink-0 text-[11px] font-medium text-[#fcd34d]">
                      Needs you
                    </span>
                  ) : (
                    <span className="shrink-0 max-w-[120px] truncate font-mono text-[11px] text-zinc-400">{e.host}</span>
                  )}
                </span>
                <Check size={14} className={`shrink-0 ${here ? 'text-[#b8adff]' : 'invisible'}`} aria-hidden="true" />
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
};

/**
 * W3 phone: a sideways row of 32 px avatars (44 px to tap) across the top of the full-screen panel.
 * While Take over holds the browser it says why it can't switch, in words (a tooltip never shows on touch).
 */
export const BrowserSwitcherRow: React.FC<SwitcherProps> = ({ entries, current, disabled, onPick }) => {
  const hintId = useId();
  return (
    <div role="group" aria-label="Browsers in use" data-browser-switcher-row="" className="flex items-center gap-2 px-2 py-1 border-b border-zinc-800 bg-zinc-900/60">
      <div className="kw-scroll-row min-w-0 flex items-center gap-1 overflow-x-auto">
        {entries.map((e) => {
          const here = e.owner === current;
          return (
            <button
              key={e.owner}
              type="button"
              data-switch-owner={e.owner}
              aria-label={`${e.kind === 'bot' ? e.label : shown(e)}, on ${e.host || 'a page'}${e.needsYou ? ', needs you' : ''}`}
              aria-current={here || undefined}
              aria-disabled={disabled || undefined}
              aria-describedby={disabled ? hintId : undefined}
              onClick={() => !disabled && !here && onPick(e.owner)}
              className={`shrink-0 inline-flex items-center justify-center h-11 w-11 rounded-[12px] ${disabled ? 'opacity-60 cursor-not-allowed' : ''} ${FOCUS}`}
            >
              <span className={`relative inline-flex rounded-[10px] ${here ? 'ring-2 ring-[#b8adff] ring-offset-2 ring-offset-zinc-900' : ''}`}>
                <EntryIcon e={e} size={32} />
                {e.needsYou && (
                  <span data-switch-needs-you="" aria-hidden="true" className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-[#fcd34d] ring-2 ring-[#1c1838]" />
                )}
              </span>
            </button>
          );
        })}
      </div>
      {disabled && (
        <span id={hintId} data-switch-hint="" className="ml-auto shrink-0 text-[11px] text-[#9a94c4]">
          Hand back first to switch
        </span>
      )}
    </div>
  );
};

/**
 * W-L4: a `/live` link to a chat whose browser isn't active, to a chat that isn't in this space, or a
 * broken one. One plain page whatever the reason, so it never tells which ids exist; its button opens
 * the chat (the one the link named, else the space's first).
 */
export const LiveMissPage: React.FC<{
  chatLabel: string;
  /** `place`: a team or channel link (`/c/<id>/live`), which no single bot owns. */
  kind?: 'bot' | 'place';
  onOpenChat: () => void;
  onBack?: () => void;
}> = ({ chatLabel, kind = 'bot', onOpenChat, onBack }) => {
  const label = chatLabel.replace(/^[@#]$/, '').trim();
  const btn = useRef<HTMLButtonElement>(null);
  useEffect(() => btn.current?.focus(), []);
  return (
    <main data-live-miss="" className="flex-1 flex flex-col items-center justify-center gap-3 px-6 text-center">
      <span className="inline-flex h-11 w-11 items-center justify-center rounded-full bg-white/[0.05] text-[#9a94c4]" aria-hidden="true">
        <Globe size={20} />
      </span>
      <h1 className="text-[15px] font-semibold text-zinc-100">{kind === 'place' ? 'No bot is browsing here now' : 'This bot isn’t browsing now'}</h1>
      <p className="max-w-[320px] text-[13px] leading-5 text-[#a39dcc]">
        {kind === 'place' ? 'When a bot opens a page here, this link shows it live.' : 'When it opens a page, this link shows it live.'}
      </p>
      <button
        ref={btn}
        type="button"
        onClick={onOpenChat}
        className={`mt-1 inline-flex min-h-9 max-sm:min-h-11 items-center justify-center rounded-lg bg-control px-4 text-[13px] font-semibold text-control-ink hover:brightness-110 ${FOCUS}`}
      >
        {label ? `Open ${label}` : 'Open the chat'}
      </button>
      {onBack && (
        <button type="button" onClick={onBack} className={`textbtn min-h-11 px-3 text-[13px] text-[#a39dcc] hover:text-zinc-100 rounded-lg ${FOCUS}`}>
          All chats
        </button>
      )}
    </main>
  );
};
