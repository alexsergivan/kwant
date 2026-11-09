import React, { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Archive, ArchiveRestore, ArrowLeft, ArrowRightLeft, Check, ChevronDown, HelpCircle, Loader2, MessageSquare, Plus, RotateCw, Square } from 'lucide-react';
import { Badge, BadgeKind, Thread, TurnMap, sortThreads, threadBadge } from '../threads';

const BADGE_STYLE: Record<BadgeKind, string> = {
  working: 'text-sky-300 bg-sky-500/10 border-sky-500/30',
  waiting: 'text-amber-300 bg-amber-500/10 border-amber-500/30',
  // Grey like the in-chat "In line" line: waiting its turn isn't waiting on you (0.7.7 final check, F).
  queued: 'text-zinc-300 bg-zinc-800/60 border-zinc-600',
  done: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/30',
  failed: 'text-rose-300 bg-rose-500/10 border-rose-500/30',
  interrupted: 'text-amber-300 bg-amber-500/10 border-amber-500/30',
  cancelled: 'text-zinc-400 bg-zinc-800/60 border-zinc-700',
};

export const StatusBadge: React.FC<{ badge: Badge; className?: string }> = ({ badge, className = '' }) => (
  <span
    className={`inline-flex items-center gap-1 px-1.5 py-px rounded-full border text-[10px] font-medium whitespace-nowrap ${BADGE_STYLE[badge.kind]} ${className}`}
  >
    {badge.kind === 'working' && <Loader2 size={9} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
    {badge.kind === 'waiting' && <HelpCircle size={9} aria-hidden="true" />}
    {badge.label}
  </span>
);

export interface ThreadBarProps {
  kwantName: string;
  /** undefined while loading. */
  threads: Thread[] | undefined;
  error: string | null;
  openThreadId: string | null;
  turns: TurnMap;
  /** Turns paused on a question to the person. */
  awaiting?: ReadonlySet<string>;
  /** Badge for the direct chat itself (working / queued). */
  directBadge: Badge | null;
  showArchived: boolean;
  onToggleArchived: () => void;
  onOpenDirect: () => void;
  onOpenThread: (t: Thread) => void;
  onNewThread: () => Promise<string | void>;
  onRetry: () => void;
}

const chip =
  'flex items-center gap-1.5 shrink-0 max-w-[15rem] px-2.5 py-1 rounded-lg border text-xs transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const chipIdle = 'border-zinc-800 bg-zinc-900/60 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/70';
const chipOpen = 'border-zinc-600 bg-zinc-800 text-zinc-100';

/**
 * An agent's conversations: the direct chat first, then delegated and user threads, each with its
 * status. Scrolls sideways, so it fits narrow screens.
 */
export const ThreadBar: React.FC<ThreadBarProps> = ({
  kwantName,
  threads,
  error,
  openThreadId,
  turns,
  awaiting,
  directBadge,
  showArchived,
  onToggleArchived,
  onOpenDirect,
  onOpenThread,
  onNewThread,
  onRetry,
}) => {
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const visible = threads ? sortThreads(threads, showArchived) : [];
  const archivedCount = threads?.filter((t) => t.archived).length ?? 0;

  const create = async () => {
    setCreating(true);
    setCreateError(null);
    const err = await onNewThread();
    if (err) setCreateError(err);
    setCreating(false);
  };

  return (
    <nav aria-label={`Conversations with ${kwantName}`} className="border-b border-zinc-800/80 bg-zinc-950/60 px-3 sm:px-6 py-2">
      <ul className="flex items-center gap-1.5 overflow-x-auto pb-0.5 -mb-0.5" role="list">
        <li>
          <button
            type="button"
            onClick={onOpenDirect}
            aria-current={!openThreadId ? 'page' : undefined}
            className={`${chip} ${!openThreadId ? chipOpen : chipIdle}`}
          >
            <MessageSquare size={12} aria-hidden="true" className="shrink-0" />
            <span className="truncate">Direct chat</span>
            {directBadge && <StatusBadge badge={directBadge} />}
          </button>
        </li>

        {threads === undefined && !error && (
          <li className="flex items-center gap-1.5 px-2 text-[11px] text-zinc-500" aria-live="polite">
            <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            Loading conversations…
          </li>
        )}

        {error && (
          <li className="flex items-center gap-1.5 px-1 text-[11px] text-rose-300" role="alert">
            <span>Couldn't load conversations.</span>
            <button
              type="button"
              onClick={onRetry}
              className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-500"
            >
              <RotateCw size={10} aria-hidden="true" /> Retry
            </button>
          </li>
        )}

        {visible.map((t) => {
          const badge = threadBadge(t, turns, awaiting);
          const isOpen = openThreadId === t.id;
          const delegated = t.kind === 'delegation';
          return (
            <li key={t.id}>
              <button
                type="button"
                onClick={() => onOpenThread(t)}
                aria-current={isOpen ? 'page' : undefined}
                title={t.title}
                className={`${chip} ${isOpen ? chipOpen : chipIdle} ${t.archived ? 'opacity-60' : ''}`}
              >
                {delegated ? (
                  <ArrowRightLeft size={12} aria-hidden="true" className="shrink-0 text-amber-300/90" />
                ) : (
                  <MessageSquare size={12} aria-hidden="true" className="shrink-0" />
                )}
                <span className="truncate">
                  <span className="sr-only">{delegated ? 'Delegated: ' : 'Conversation: '}</span>
                  {t.title}
                  {t.archived && <span className="sr-only"> (archived)</span>}
                </span>
                {badge && <StatusBadge badge={badge} />}
              </button>
            </li>
          );
        })}

        {threads !== undefined && visible.length === 0 && !error && (
          <li className="hidden sm:block px-1 text-[11px] text-zinc-500">No other conversations yet.</li>
        )}

        <li>
          <button
            type="button"
            onClick={create}
            disabled={creating}
            className={`${chip} border-dashed border-zinc-700 text-sky-300 hover:bg-sky-500/10 disabled:opacity-50`}
            title={`Start a separate conversation with ${kwantName}`}
          >
            <Plus size={12} aria-hidden="true" />
            <span>{creating ? 'Starting…' : 'New'}</span>
          </button>
        </li>

        {archivedCount > 0 && (
          <li>
            <button
              type="button"
              onClick={onToggleArchived}
              aria-pressed={showArchived}
              className={`${chip} ${chipIdle}`}
            >
              <Archive size={12} aria-hidden="true" />
              <span>{showArchived ? 'Hide archived' : `Archived (${archivedCount})`}</span>
            </button>
          </li>
        )}
      </ul>
      {createError && (
        <p role="alert" className="mt-1 text-[11px] text-rose-300">
          {createError}
        </p>
      )}
    </nav>
  );
};

/** The status dot on the phone header's second line: violet while working, amber while it waits on you, grey while queued (never holo). */
export function statusDot(badge: Badge | null): { tone: 'working' | 'waiting' | 'queued'; label: string } | null {
  if (!badge) return null;
  if (badge.kind === 'working') return { tone: 'working', label: badge.label };
  if (badge.kind === 'waiting') return { tone: 'waiting', label: badge.label };
  if (badge.kind === 'queued') return { tone: 'queued', label: badge.label };
  return null;
}

const sheetRow =
  'w-full min-h-[52px] flex items-center gap-3 px-4 text-left text-[15px] leading-5 text-zinc-100 hover:bg-white/[0.06] active:bg-white/[0.06] focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[#b8adff]';

/**
 * Phone (spec A2): the bot's conversations folded into the header. Line 2 shows the open one
 * ("Direct chat ▾" or the thread's title) with a status dot; tapping it opens the same list as the
 * desktop thread bar in a bottom sheet, "+ New thread" first.
 */
export const ThreadPicker: React.FC<ThreadBarProps> = ({
  kwantName,
  threads,
  error,
  openThreadId,
  turns,
  awaiting,
  directBadge,
  showArchived,
  onToggleArchived,
  onOpenDirect,
  onOpenThread,
  onNewThread,
  onRetry,
}) => {
  const [open, setOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const visible = threads ? sortThreads(threads, showArchived) : [];
  const archivedCount = threads?.filter((t) => t.archived).length ?? 0;
  const current = openThreadId ? threads?.find((t) => t.id === openThreadId) : undefined;
  const label = openThreadId ? current?.title ?? 'Conversation' : 'Direct chat';
  const badge = current ? threadBadge(current, turns, awaiting) : openThreadId ? null : directBadge;
  const dot = statusDot(badge);

  const close = () => {
    setOpen(false);
    triggerRef.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    sheetRef.current?.querySelector<HTMLElement>('button')?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener('keydown', onKey, true);
    return () => document.removeEventListener('keydown', onKey, true);
  }, [open]);

  const pick = (fn: () => void) => {
    fn();
    setOpen(false);
  };
  const create = async () => {
    setCreating(true);
    setCreateError(null);
    const err = await onNewThread();
    setCreating(false);
    if (err) setCreateError(err);
    else setOpen(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        data-compact
        data-thread-picker
        onClick={() => {
          setCreateError(null);
          setOpen(true);
        }}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`${label}${dot ? `, ${dot.label}` : ''}. Switch conversation with ${kwantName}`}
        // The line is 16 px; the ::after reaches up over the name and down to the header's edge for a thumb-sized target
        className="relative flex items-center gap-1.5 min-w-0 max-w-full text-[13px] leading-4 text-[#a39dcc] hover:text-zinc-100 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff] after:absolute after:-inset-x-1 after:-top-6 after:-bottom-3 after:content-['']"
      >
        {dot && (
          <span
            data-status-dot={dot.tone}
            aria-hidden="true"
            className={`shrink-0 w-1.5 h-1.5 rounded-full ${dot.tone === 'working' ? 'bg-[#9d8fff]' : dot.tone === 'queued' ? 'bg-zinc-500' : 'bg-[#fcd34d]'}`}
          />
        )}
        <span className="truncate">{label}</span>
        <ChevronDown size={12} aria-hidden="true" className="shrink-0" />
      </button>
      {open &&
        createPortal(
          <div className="fixed inset-0 z-50 flex flex-col justify-end" data-thread-sheet>
            <button type="button" aria-label="Close" data-compact tabIndex={-1} onClick={close} className="absolute inset-0 bg-black/50 cursor-default" />
            <div
              ref={sheetRef}
              role="dialog"
              aria-modal="true"
              aria-label={`Conversations with ${kwantName}`}
              className="kw-sheet relative max-h-[70vh] flex flex-col rounded-t-2xl bg-[#1a1538] border-t border-white/10 pb-[env(safe-area-inset-bottom)] shadow-2xl"
            >
              <div className="shrink-0 flex justify-center pt-2 pb-1" aria-hidden="true">
                <span className="w-9 h-1 rounded-sm bg-[#3f3868]" />
              </div>
              <ul className="overflow-y-auto py-1" role="list">
                <li>
                  <button type="button" onClick={create} disabled={creating} className={`${sheetRow} text-[#b8adff] disabled:opacity-60`}>
                    {creating ? <Loader2 size={16} aria-hidden="true" className="animate-spin motion-reduce:animate-none" /> : <Plus size={16} aria-hidden="true" />}
                    <span>{creating ? 'Starting…' : 'New thread'}</span>
                  </button>
                  {createError && (
                    <p role="alert" className="px-4 pb-2 text-[13px] text-rose-300">
                      {createError}
                    </p>
                  )}
                </li>
                <li>
                  <button type="button" onClick={() => pick(onOpenDirect)} aria-current={!openThreadId ? 'page' : undefined} className={sheetRow}>
                    <MessageSquare size={16} aria-hidden="true" className="shrink-0 text-[#a39dcc]" />
                    <span className="min-w-0 flex-1 truncate">Direct chat</span>
                    {directBadge && <StatusBadge badge={directBadge} />}
                    {!openThreadId && <Check size={16} aria-hidden="true" className="shrink-0 text-[#b8adff]" />}
                  </button>
                </li>
                {threads === undefined && !error && (
                  <li className="flex items-center gap-2 px-4 min-h-[52px] text-[13px] text-[#a39dcc]" aria-live="polite">
                    <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
                    Loading conversations…
                  </li>
                )}
                {error && (
                  <li className="flex items-center gap-3 px-4 min-h-[52px] text-[13px] text-rose-300" role="alert">
                    <span className="flex-1">Couldn't load conversations.</span>
                    <button type="button" onClick={onRetry} className="inline-flex items-center gap-1 px-3 rounded-lg border border-rose-500/30 hover:bg-rose-500/10">
                      <RotateCw size={12} aria-hidden="true" /> Retry
                    </button>
                  </li>
                )}
                {visible.map((t) => {
                  const b = threadBadge(t, turns, awaiting);
                  const isOpen = openThreadId === t.id;
                  const delegated = t.kind === 'delegation';
                  return (
                    <li key={t.id}>
                      <button
                        type="button"
                        onClick={() => pick(() => onOpenThread(t))}
                        aria-current={isOpen ? 'page' : undefined}
                        className={`${sheetRow} ${t.archived ? 'opacity-60' : ''}`}
                      >
                        {delegated ? (
                          <ArrowRightLeft size={16} aria-hidden="true" className="shrink-0 text-amber-300/90" />
                        ) : (
                          <MessageSquare size={16} aria-hidden="true" className="shrink-0 text-[#a39dcc]" />
                        )}
                        <span className="min-w-0 flex-1 truncate">
                          <span className="sr-only">{delegated ? 'Delegated: ' : 'Conversation: '}</span>
                          {t.title}
                          {t.archived && <span className="sr-only"> (archived)</span>}
                        </span>
                        {b && <StatusBadge badge={b} />}
                        {isOpen && <Check size={16} aria-hidden="true" className="shrink-0 text-[#b8adff]" />}
                      </button>
                    </li>
                  );
                })}
                {archivedCount > 0 && (
                  <li>
                    <button type="button" onClick={onToggleArchived} aria-pressed={showArchived} className={`${sheetRow} text-[#a39dcc]`}>
                      <Archive size={16} aria-hidden="true" />
                      <span>{showArchived ? 'Hide archived' : `Archived (${archivedCount})`}</span>
                    </button>
                  </li>
                )}
              </ul>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
};

export interface ThreadHeaderProps {
  thread: Thread;
  badge: Badge | null;
  /** Name of the agent that delegated it, for delegation threads. */
  delegatedBy?: string;
  onBack: () => void;
  onStop: () => Promise<string | void>;
  onArchive: (archived: boolean) => Promise<string | void>;
}

/** Above an open thread: what it is, its status, Stop while it runs, and Archive. */
export const ThreadHeader: React.FC<ThreadHeaderProps> = ({ thread, badge, delegatedBy, onBack, onStop, onArchive }) => {
  const [busy, setBusy] = useState<'stop' | 'archive' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const running = badge?.kind === 'working' || badge?.kind === 'waiting' || badge?.kind === 'queued';

  const act = async (kind: 'stop' | 'archive', fn: () => Promise<string | void>) => {
    setBusy(kind);
    setError(null);
    const err = await fn();
    if (err) setError(err);
    setBusy(null);
  };

  return (
    <div className="px-3 sm:px-6 py-2 border-b border-zinc-800/80 bg-zinc-900/20 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs">
      <button
        type="button"
        onClick={onBack}
        className="inline-flex items-center gap-1 text-zinc-400 hover:text-zinc-100 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        <ArrowLeft size={13} aria-hidden="true" />
        <span>Direct chat</span>
      </button>
      <div className="flex items-center gap-2 min-w-0 flex-1">
        <h3 className="min-w-0 truncate text-zinc-200 font-medium" title={thread.title}>
          {thread.title}
        </h3>
        <span className="text-zinc-500 shrink-0 hidden sm:inline">
          {thread.kind === 'delegation' ? `Delegated by ${delegatedBy ?? 'another bot'}` : 'Separate conversation'}
        </span>
        {badge && <StatusBadge badge={badge} />}
        {thread.archived && <span className="text-[10px] text-zinc-500">Archived</span>}
      </div>
      <div className="flex items-center gap-1.5">
        {running && (
          <button
            type="button"
            onClick={() => act('stop', onStop)}
            disabled={busy !== null}
            className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg font-medium text-rose-300 bg-rose-500/10 border border-rose-500/30 hover:bg-rose-500/20 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-500"
          >
            <Square size={10} fill="currentColor" aria-hidden="true" />
            {busy === 'stop' ? 'Stopping…' : 'Stop'}
          </button>
        )}
        <button
          type="button"
          onClick={() => act('archive', () => onArchive(!thread.archived))}
          disabled={busy !== null}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg font-medium text-zinc-300 bg-zinc-800/80 border border-zinc-700/60 hover:bg-zinc-700/80 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          {thread.archived ? <ArchiveRestore size={12} aria-hidden="true" /> : <Archive size={12} aria-hidden="true" />}
          {thread.archived ? 'Unarchive' : 'Archive'}
        </button>
      </div>
      {error && (
        <p role="alert" className="basis-full text-[11px] text-rose-300">
          {error}
        </p>
      )}
    </div>
  );
};
