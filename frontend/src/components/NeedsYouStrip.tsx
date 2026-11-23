import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowUp, Check, ChevronLeft, ChevronRight, Globe, Hand, Inbox, Loader2, Play, RotateCcw, type LucideIcon } from 'lucide-react';
import type { AwayItem } from '../away';
import { approvalLine } from '../approvals';
import { localClock } from '../clock';
import {
  ACTION_LABEL,
  BUSY_LABEL,
  RESOLVES,
  RUN_AGAIN_SKIPPED,
  actionName,
  askAction,
  askHint,
  askKind,
  bandLine,
  canClear,
  retryRoute,
  failLine,
  stripHeading,
  stripIndex,
  type AskAction,
} from '../askHere';
import { useFocusAfterRender } from '../focusAfterRender';

export interface NeedsYouStripProps {
  /** The open asks in this chat, oldest first (`asksHere`). Nothing shows when empty. */
  asks: AwayItem[];
  /** Runs Retry, Continue, Ask again or Open the browser step. A throw shows its message as the reason. */
  onAction: (item: AwayItem, action: AskAction) => Promise<unknown> | void;
  /**
   * "Done, clear it": closes the item (the person only). Resolves `{ use }` when the server refused it
   * because the ask has its own answer (#700's 409 `use_*`): the strip switches its button to that one.
   */
  onDismiss: (item: AwayItem) => Promise<{ use: AskAction } | void | unknown>;
  /**
   * Go to the question / approval: scroll to its card and ring it, loading history around it when needed.
   * Resolves false when the card can't be found; the strip then offers Open in Inbox. With `quiet`, only
   * rings a card that is already rendered (stepping between asks) and never loads or scrolls far.
   */
  onShow?: (item: AwayItem, opts?: { quiet?: boolean }) => boolean | Promise<boolean>;
  /** Open in Inbox, when the card is gone from the chat. */
  onOpenInbox?: (item: AwayItem) => void;
  /** Bumped when a row was opened from the keyboard: focus moves to the strip, so its ask is read. */
  focusKey?: number;
  /** The chat on screen; announcements only compare asks within one chat. */
  hereKey?: string;
  phone?: boolean;
  /** Tests: the clock's "now", and how long a sent action waits before its button comes back. */
  now?: number;
  sentWaitMs?: number;
}

const ICON: Record<AskAction, LucideIcon> = {
  retry: RotateCcw,
  continue: Play,
  ask_again: RotateCcw,
  answer: ArrowUp,
  decide: ArrowUp,
  browser: Globe,
  run_again: RotateCcw,
};

/** How long a sent Retry / Continue / Ask again waits for the close event before its button comes back. */
export const SENT_WAIT_MS = 5000;
/** At most one spoken update per this many ms (the sidebar's `data-needs-attention-live` pattern). */
const SAY_EVERY_MS = 10_000;

const motionOk = () => typeof window !== 'undefined' && !!window.matchMedia && !window.matchMedia('(prefers-reduced-motion: reduce)').matches;

const focusComposer = () => document.querySelector<HTMLElement>('textarea[data-composer]')?.focus();

/**
 * 22i: the "Needs you" strip above the composer (design.md §2). While an open ask sits in the chat on
 * screen it says the whole ask, what kind it is, what its one button does, and "Done, clear it", and steps
 * through several. It reads the Needs-you list, not the messages, so it shows even when the ask's message
 * isn't loaded, and it goes in the same render as the sidebar row once the list drops the item.
 */
export const NeedsYouStrip: React.FC<NeedsYouStripProps> = ({ asks, onAction, onDismiss, onShow, onOpenInbox, focusKey, hereKey, phone, now, sentWaitMs = SENT_WAIT_MS }) => {
  const [index, setIndex] = useState(0);
  const shownId = useRef<string | null>(null);
  // The action can be sending while "Done, clear it" stays pressable; clearing stops both.
  const [acting, setActing] = useState(false);
  const [clearing, setClearing] = useState(false);
  // A second press before the first re-renders: the state isn't set yet, so refs hold the locks.
  const actLock = useRef(false);
  const clearLock = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  // routine-run-again: Run again says "Started." at once and holds its button until the run takes the item away.
  const [started, setStarted] = useState(false);
  // A Run again the server skipped because a run is still going: said in the hint, and the button comes back.
  const [skipped, setSkipped] = useState(false);
  const [gone, setGone] = useState(false);
  // A dismiss the server refused (409 use_*): the ask's own button instead, and no Done.
  const [switched, setSwitched] = useState<AskAction | null>(null);
  const actionRef = useRef<HTMLButtonElement | null>(null);
  const sentTimer = useRef<number | null>(null);
  const sectionRef = useRef<HTMLElement | null>(null);
  const hadFocus = useRef(false);
  const [narrow, setNarrow] = useState(false);

  // Leaving: the last ask fades for 120 ms (none under reduced motion) instead of vanishing mid-read.
  const [ghost, setGhost] = useState<AwayItem[] | null>(null);
  const prevAsks = useRef<AwayItem[]>(asks);
  const shown = asks.length ? asks : ghost ?? [];
  const leaving = !asks.length && !!ghost;
  useLayoutEffect(() => {
    const before = prevAsks.current;
    prevAsks.current = asks;
    if (asks.length || !before.length) {
      if (asks.length) setGhost(null);
      return;
    }
    // The strip is going: focus that was in it goes to the composer, never to the page body.
    if (hadFocus.current) {
      hadFocus.current = false;
      focusComposer();
    }
    if (!motionOk()) return;
    setGhost(before);
    const t = window.setTimeout(() => setGhost(null), 120);
    return () => window.clearTimeout(t);
  }, [asks]);

  const i = stripIndex(shown, shownId.current, index);
  const item = shown[i];
  useEffect(() => {
    if (i !== index) setIndex(i);
  }, [i, index]);
  // A new ask on screen starts clean: no leftover error, spinner or "Sent" from the one before.
  const itemId = item?.id ?? null;
  useEffect(() => {
    if (shownId.current !== itemId) {
      setError(null);
      setActing(false);
      setClearing(false);
      setSent(false);
      setStarted(false);
      setSkipped(false);
      setGone(false);
      setSwitched(null);
      actLock.current = false;
      clearLock.current = false;
      if (sentTimer.current) window.clearTimeout(sentTimer.current);
    }
    shownId.current = itemId;
  }, [itemId]);
  useEffect(() => () => {
    if (sentTimer.current) window.clearTimeout(sentTimer.current);
  }, []);

  // A keyboard open lands on the strip, so a screen reader reads the ask and Tab reaches its button.
  useEffect(() => {
    if (focusKey) sectionRef.current?.focus();
  }, [focusKey]);

  // Under 640 px of composer the buttons drop below the hint (design.md §2).
  useEffect(() => {
    const el = sectionRef.current;
    if (!el || phone || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(([e]) => setNarrow(e.contentRect.width < 640));
    ro.observe(el);
    return () => ro.disconnect();
  }, [phone, !!item]);

  // One polite line: an ask that arrives in the open chat, the last one closing, and a clear.
  const [said, setSaid] = useState('');
  const saidAt = useRef(0);
  const cleared = useRef(false);
  const seen = useRef<{ here?: string; ids: Set<string> }>({ here: hereKey, ids: new Set(asks.map((a) => a.id)) });
  useEffect(() => {
    const was = seen.current;
    const ids = new Set(asks.map((a) => a.id));
    seen.current = { here: hereKey, ids };
    if (was.here !== hereKey) return;
    const say = (text: string, always = false) => {
      const t = Date.now();
      if (!always && t - saidAt.current < SAY_EVERY_MS) return;
      saidAt.current = t;
      setSaid(text);
    };
    if (cleared.current && ids.size < was.ids.size) {
      cleared.current = false;
      return say('Cleared.', true);
    }
    if (!ids.size && was.ids.size) return say('Nothing needs you here now.');
    const fresh = asks.find((a) => !was.ids.has(a.id));
    if (fresh) {
      const act = askAction(fresh);
      say(`${fresh.kwant_name} needs you: ${act ? ACTION_LABEL[act] : canClear(fresh) ? 'Done, clear it' : `write to ${fresh.kwant_name}`}`);
    }
  }, [asks, hereKey]);
  const live = (
    <span className="sr-only" aria-live="polite" data-needs-you-live>
      {said}
    </span>
  );

  const focusAfterRender = useFocusAfterRender();
  if (!item) return live;
  // A Retry with nowhere to go (a failed hand-off with no board task) isn't offered (Mira's fix 3).
  const base = askAction(item);
  const action = switched ?? (base === 'retry' && !retryRoute(item) ? null : base);
  const clearable = canClear(item) && !switched;
  const n = shown.length;
  const bot = item.kwant_name || 'the bot';
  const heading = stripHeading(shown);
  const headId = `needs-you-head-${item.id}`;
  const manyBots = new Set(shown.map((a) => a.kwant_id)).size > 1 || n > 1;
  const time = localClock(item.timestamp, now);
  const goTo = action === 'answer' || action === 'decide';
  const resolving = !!action && RESOLVES.has(action);

  const step = (d: number) => {
    const next = Math.min(Math.max(0, i + d), n - 1);
    if (next === i) return;
    shownId.current = shown[next].id;
    setIndex(next);
    void onShow?.(shown[next], { quiet: true });
  };
  const clear = async () => {
    if (clearLock.current || leaving) return;
    clearLock.current = true;
    setError(null);
    setClearing(true);
    cleared.current = true;
    try {
      const r = await onDismiss(item);
      const use = r && typeof r === 'object' && 'use' in r ? (r as { use: AskAction }).use : null;
      if (use) {
        // Not an error: this ask is resolved on its own card, so the button goes there.
        cleared.current = false;
        setSwitched(use);
        focusAfterRender(() => actionRef.current);
      }
    } catch (e) {
      cleared.current = false;
      setError(failLine('dismiss', e instanceof Error && e.message ? e.message : null, bot));
    } finally {
      clearLock.current = false;
      setClearing(false);
    }
  };
  const act = async () => {
    if (actLock.current || clearLock.current || leaving || !action) return;
    if (gone) return onOpenInbox?.(item);
    actLock.current = true;
    setError(null);
    setSent(false);
    setSkipped(false);
    setActing(true);
    let waiting = false;
    try {
      if (goTo) {
        const found = onShow ? await onShow(item) : true;
        if (!found) setGone(true);
      } else {
        const r = await onAction(item, action);
        if (action === 'run_again' && r === 'skipped') {
          setSkipped(true);
          setSaid(RUN_AGAIN_SKIPPED);
        } else if (resolving) {
          // Sent: wait for the close event. Still open after 5 s, the button comes back with a note.
          waiting = true;
          if (action === 'run_again') {
            setActing(false);
            setStarted(true);
            setSent(true);
            setSaid('Started.');
          }
          sentTimer.current = window.setTimeout(() => {
            sentTimer.current = null;
            actLock.current = false;
            setActing(false);
            setStarted(false);
            setSent(true);
          }, sentWaitMs);
        }
      }
    } catch (e) {
      setError(failLine(action, e instanceof Error && e.message ? e.message : null, bot));
    } finally {
      if (!waiting) {
        actLock.current = false;
        setActing(false);
      }
    }
  };

  const stacked = phone || narrow;
  const btnH = phone ? 'h-11 text-[14px]' : 'h-8';
  const Icon = gone ? Inbox : started ? Check : action ? ICON[action] : null;
  const busyAct = acting && resolving;
  const label = gone ? 'Open in Inbox' : action ? (busyAct ? BUSY_LABEL[action] ?? ACTION_LABEL[action] : started ? 'Started' : ACTION_LABEL[action]) : '';
  // After a 409 switch: why Done went away (Mira's fix 5).
  const switchedHint =
    switched === 'answer'
      ? 'This one clears when you answer it on its card.'
      : switched === 'decide'
        ? 'This one clears when you Allow or Deny it on its card.'
        : switched === 'browser'
          ? `This one clears when you finish or decline it in ${bot}’s browser.`
          : null;
  const hint = gone ? 'Its card isn’t in this chat anymore.' : sent ? (action === 'run_again' ? 'Started.' : `Sent. This clears once ${bot} answers.`) : skipped ? RUN_AGAIN_SKIPPED : switchedHint ?? askHint(item, action, bot, clearable);
  const stepBtn =
    'kws-hit inline-flex items-center justify-center w-7 h-7 rounded-lg border border-hairline-strong text-zinc-200 hover:bg-white/[0.08] disabled:opacity-40 disabled:cursor-default kw-focus';

  const buttons = (
    <div
      data-strip-buttons
      className={phone ? `mt-2.5 flex gap-2 ${action === 'browser' && !gone ? 'flex-col-reverse' : ''}` : stacked ? 'mt-2 flex justify-end gap-2' : 'flex items-center gap-2 self-center row-start-2 col-start-2'}
    >
      {clearable && (
        <button
          type="button"
          data-strip-dismiss
          disabled={clearing || leaving}
          aria-busy={clearing || undefined}
          aria-label="Done, clear it from Needs attention"
          onClick={() => void clear()}
          className={`kw-btn-ghost ${btnH} ${phone ? 'flex-1' : ''} disabled:opacity-60 disabled:cursor-default`}
        >
          {clearing ? <Loader2 size={13} aria-hidden="true" className="kw-dict-spin" /> : <Check size={13} aria-hidden="true" />}
          {clearing ? 'Clearing…' : 'Done, clear it'}
        </button>
      )}
      {action && (
        <button
          ref={actionRef}
          type="button"
          data-strip-action={gone ? 'inbox' : action}
          disabled={busyAct || started || clearing || leaving}
          aria-busy={busyAct || undefined}
          aria-label={gone ? 'Open in Inbox' : actionName(action, bot)}
          onClick={() => void act()}
          className={`kw-btn-primary ${btnH} ${phone ? 'flex-[1.4]' : ''} whitespace-nowrap`}
        >
          {busyAct ? <Loader2 size={14} aria-hidden="true" className="kw-dict-spin shrink-0" /> : Icon && <Icon size={14} aria-hidden="true" className="shrink-0" />}
          {label}
        </button>
      )}
    </div>
  );

  return (
    <>
      {live}
      <section
        ref={sectionRef}
        tabIndex={-1}
        data-needs-you-strip={item.id}
        data-action={action ?? 'none'}
        data-strip-state={clearing ? 'clearing' : busyAct ? 'busy' : gone ? 'gone' : error ? 'error' : skipped ? 'skipped' : started ? 'started' : sent ? 'sent' : 'idle'}
        aria-labelledby={headId}
        onFocus={() => (hadFocus.current = true)}
        onBlur={(e) => {
          if (e.relatedTarget && !e.currentTarget.contains(e.relatedTarget as Node)) hadFocus.current = false;
        }}
        className={`${leaving ? 'kw-needs-leave' : 'kw-pin-enter'} max-w-4xl mx-auto mb-2 rounded-xl border border-amber-300/35 bg-surface-main px-3 py-2.5 kw-focus ${
          phone ? 'max-h-[45vh]' : 'max-h-[240px]'
        } overflow-y-auto ${stacked ? '' : 'grid grid-cols-[1fr_auto] gap-x-4'}`}
        style={{ backgroundImage: 'linear-gradient(rgb(252 211 77 / 0.06), rgb(252 211 77 / 0.06))' }}
      >
        <div className="flex items-center gap-1.5 min-w-0 col-span-2">
          <Hand size={12} aria-hidden="true" className="shrink-0 text-amber-300" />
          <h2 id={headId} className="text-[11px] font-semibold uppercase tracking-[0.06em] text-amber-300 truncate">
            {heading}
          </h2>
          {n > 1 && (
            <span className="ml-auto flex items-center gap-1.5 shrink-0" data-strip-steps>
              <button type="button" data-compact className={stepBtn} onClick={() => step(-1)} disabled={i === 0} aria-label="Previous ask">
                <ChevronLeft size={14} aria-hidden="true" />
              </button>
              <span className="text-meta tabular-nums text-zinc-300" aria-live="polite" data-strip-count>
                {i + 1} of {n}
              </span>
              <button type="button" data-compact className={stepBtn} onClick={() => step(1)} disabled={i === n - 1} aria-label="Next ask">
                <ChevronRight size={14} aria-hidden="true" />
              </button>
            </span>
          )}
        </div>
        {/* Stepping cross-fades the ask (opacity only); keyed so each ask enters on its own. */}
        <div key={item.id} className={`kw-needs-fade min-w-0 ${stacked ? '' : 'col-start-1 row-start-2'}`}>
          <p className="mt-0.5 text-meta text-zinc-400" data-strip-kind>
            <span className="font-semibold text-amber-200">{askKind(item, action)}</span>
            {manyBots && <> · {bot}</>}
            {time && <> · {time}</>}
          </p>
          {/* The whole line, never cut: it wraps, and the box scrolls past 240 px. */}
          <p data-strip-line className="mt-0.5 text-body font-medium text-zinc-50 break-words">
            {approvalLine(item) ?? bandLine(item, action)}
          </p>
          <p data-strip-hint className={`mt-0.5 text-meta ${gone ? 'text-zinc-300' : 'text-zinc-400'}`}>
            {hint}
          </p>
          {error && (
            <p role="alert" data-strip-error className="mt-1 text-meta text-bad break-words">
              {error}
            </p>
          )}
        </div>
        {clearable || action ? buttons : null}
      </section>
    </>
  );
};
