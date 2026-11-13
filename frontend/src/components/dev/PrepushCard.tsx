// 0.8.0 B3: the pre-push card in the author bot's thread (Mira b3-fresh-eyes.md §2–§4, Juno
// fresh-eyes-audit.md §4). Every state and number comes from the server card; the client only
// picks the words. Amber appears only where the person must act (Needs you, Push anyway). There is
// never an "always allow" or "skip this check": an override covers one push of one commit.

import React, { useEffect, useId, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, ArrowUpFromLine, ChevronDown, ChevronRight, GitBranch } from 'lucide-react';
import { DiffStat, Led } from './parts';
import { DevModeError } from '../../devMode';
import {
  CHECK_NAME,
  STATE_LED,
  STATE_WORD,
  canOverride,
  clearedParts,
  foldLine,
  foldable,
  footer,
  storyOf,
  needsYouLead,
  openCount,
  overriddenLine,
  passedRows,
  phaseOf,
  reasonOk,
  rowBlocks,
  rowFindings,
  statusChip,
  toolLine,
  afterRetryLine,
  pushedLine,
  type Story,
  MIN_REASON,
  MAX_REASON,
  type PrepushCard as Card,
  type PrepushCheck,
  type PrepushFinding,
} from '../../dev/prepush';

const BTN =
  'inline-flex items-center justify-center gap-1.5 rounded-lg px-3 text-xs font-medium transition-colors min-h-11 sm:min-h-[32px] max-sm:w-full outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-900 disabled:cursor-not-allowed disabled:opacity-50';
const PLAIN = `${BTN} border border-hairline-strong bg-zinc-800 text-zinc-200 hover:bg-zinc-700`;
const PRIMARY = `${BTN} text-white`;
const AMBER = `${BTN} bg-amber-600 text-white shadow-sm hover:bg-amber-500`;
const AMBER_OUTLINE = `${BTN} border border-amber-400/60 bg-transparent text-amber-200 hover:bg-amber-400/10`;
const LINK = 'inline-flex items-center gap-0.5 rounded px-1 text-meta font-medium text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-sky-400 min-h-11 sm:min-h-0';

const CHIP: Record<string, string> = { plain: 'kw-chip', ok: 'kw-chip kw-chip-ok', bad: 'kw-chip kw-chip-bad', warn: 'kw-chip kw-chip-warn' };

export interface PrepushCardProps {
  card: Card;
  /** The author bot's name, from the space's bot list. */
  bot: string;
  /** "Ask {bot} to try again" / "Run the checks again": /retry, then tell the bot. Absent: read-only. */
  onRetry?: (c: Card) => Promise<void>;
  /** "Push anyway": /override with the reason, then tell the bot. Absent: read-only. */
  onOverride?: (c: Card, reason: string) => Promise<void>;
  /** The card turned clear while the thread was open: show P2 for about 2 s, then fold to P2b. */
  live?: boolean;
  /** Start with the reason form open (screenshots and tests). */
  startOverride?: boolean;
  /** Start with the folded passing rows open on phone (screenshots and tests). */
  startUnfolded?: boolean;
  /** 0.9.7: after a retry or override, what happens next, following the bot's new push live. */
  story?: Story | null;
  /** 0.9.7: this card is the new push after the person's retry on `after`: one story, linked both ways. */
  after?: Card | null;
  /** Scroll to another pre-push card in this thread. */
  onOpenCard?: (id: string) => void;
  /** Open the approval the push still needs (floor, payment) in this chat. */
  onOpenApproval?: (id: string) => void;
}

const where = (f: PrepushFinding) => (f.file ? `${f.file}${f.line !== null ? `:${f.line}` : ''}` : null);

const Finding: React.FC<{ f: PrepushFinding }> = ({ f }) => (
  <li className="min-w-0 text-meta text-zinc-300 [overflow-wrap:anywhere]">
    <span className="text-rose-300">{f.title}</span>
    {where(f) && <code className="dv-sha ml-1.5 inline-block max-w-full align-middle [overflow-wrap:anywhere]">{where(f)}</code>}
    {f.detail && <span className="mt-0.5 block text-zinc-400">{f.detail}</span>}
  </li>
);

const Row: React.FC<{ k: PrepushCheck; card: Card; hidePhone: boolean }> = ({ k, card, hidePhone }) => {
  const [show, setShow] = useState(false);
  const id = useId();
  const found = k.state === 'hit' ? rowFindings(card, k.key) : [];
  const dim = k.state === 'not_installed';
  // A hit the server doesn't count as a blocker (risk, a low tamper or review finding): the LED stays
  // "found a problem", the words say it doesn't hold the push (Mira Later 4, contract §4.3).
  const soft = k.state === 'hit' && card.status !== 'checking' && !rowBlocks(card, k.key);
  const tone = k.state === 'hit' ? (soft ? 'text-zinc-300' : 'text-rose-300') : dim ? 'text-zinc-500' : 'text-zinc-300';
  const ledClass = k.state === 'running' ? 'dv-led-still mt-[5px]' : k.state === 'not_installed' ? 'dv-led-dashed mt-[5px]' : 'mt-[5px]';
  return (
    <li data-check={k.key} data-state={k.state} className={`border-t border-hairline py-2 first:border-t-0 ${hidePhone ? 'max-sm:hidden' : ''}`}>
      <div className="flex min-w-0 items-start gap-2.5">
        <Led s={STATE_LED[k.state]} className={ledClass} />
        <span className="sr-only">{STATE_WORD[k.state]}:</span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5 sm:flex-row sm:items-start sm:gap-3">
          <div className="min-w-0 sm:w-[168px] sm:shrink-0">
            <p className={`text-[13px] leading-[18px] ${dim ? 'text-zinc-500' : 'text-zinc-100'}`}>{CHECK_NAME[k.key]}</p>
            <p className="text-[11px] leading-[15px] text-zinc-500 [overflow-wrap:anywhere]">{toolLine(k.key, card)}</p>
          </div>
          <p className={`min-w-0 flex-1 text-meta [overflow-wrap:anywhere] ${tone}`}>
            {k.text}
            {soft && <span data-soft className="text-zinc-400"> · doesn't block</span>}
          </p>
        </div>
        {found.length > 0 && (
          <button type="button" className={LINK} aria-expanded={show} aria-controls={id} onClick={() => setShow((s) => !s)}>
            {show ? 'Hide' : 'Show'}
            <span className="sr-only"> {CHECK_NAME[k.key]} findings</span>
          </button>
        )}
      </div>
      {show && found.length > 0 && (
        <ul id={id} role="list" className="mt-1.5 space-y-1 pl-[18px] sm:pl-[198px]">
          {found.map((f, i) => (
            <Finding key={f.id ?? `${f.title}-${i}`} f={f} />
          ))}
        </ul>
      )}
    </li>
  );
};

/** Text with the branch rendered as code (Mira Later 6), at its first appearance. */
const withBranch = (text: string, branch: string): React.ReactNode => {
  const i = text.indexOf(branch);
  if (i < 0) return text;
  return (
    <>
      {text.slice(0, i)}
      <code className="dv-sha dv-sha-wrap">{branch}</code>
      {text.slice(i + branch.length)}
    </>
  );
};

const STORY_LED: Record<Story['step'], 'ok' | 'off' | 'run' | 'bad'> = {
  fixing: 'run',
  checking: 'run',
  pushing: 'run',
  asks: 'run',
  pushed: 'ok',
  blocked: 'bad',
  expired: 'off',
  replaced: 'off',
};

/**
 * 0.9.7: where the push is after the person's press. One live line under the lead; the new card and an
 * approval the floor still needs are one link away, so the person never has to hunt for them.
 */
const StoryLine: React.FC<{ story: Story; card: Card; bot: string; onOpenCard?: (id: string) => void; onOpenApproval?: (id: string) => void }> = ({
  story,
  card,
  bot,
  onOpenCard,
  onOpenApproval,
}) => (
  <div data-story={story.step} className="mt-1.5 flex min-w-0 flex-col gap-1">
    {story.step === 'asks' && story.approval ? (
      // One amber line that says it and links to the approval (no second status line repeating it).
      <p
        role="status"
        data-story-approval={story.approval.id}
        className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-amber-400/40 bg-amber-400/[.06] px-2.5 py-1.5 text-meta text-zinc-200"
      >
        <Led s={STORY_LED.asks} className="shrink-0 dv-led-still" />
        <span data-story-now className="min-w-0 flex-1 [overflow-wrap:anywhere]">{withBranch(story.now, card.branch)}</span>
        {onOpenApproval && (
          <button type="button" className={LINK} onClick={() => onOpenApproval(story.approval!.id)}>
            Open it <ArrowDown size={12} aria-hidden="true" />
          </button>
        )}
      </p>
    ) : (
      <p role="status" className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-meta">
        <Led s={STORY_LED[story.step]} className={`shrink-0 ${story.step === 'pushed' || story.step === 'blocked' ? '' : 'dv-led-still'}`} />
        <span data-story-now className={`min-w-0 [overflow-wrap:anywhere] ${story.step === 'pushed' ? 'font-medium text-emerald-300' : story.step === 'blocked' ? 'text-rose-300' : 'text-zinc-200'}`}>
          {story.step === 'asks' ? `${bot}'s push needs your OK.` : story.now}
        </span>
        {story.next && onOpenCard && story.step !== 'asks' && (
          <button type="button" data-story-next={story.next.id} className={LINK} onClick={() => onOpenCard(story.next!.id)}>
            {story.step === 'blocked' ? 'See what it found' : story.step === 'pushed' ? 'See the push' : 'See the new check'} <ArrowDown size={12} aria-hidden="true" />
          </button>
        )}
      </p>
    )}
  </div>
);

/** A one-line card (P0, retried, cleared and folded). */
const Line: React.FC<{ children: React.ReactNode; led?: 'ok' | 'off'; status?: boolean; extra?: React.ReactNode; phase: string; error?: string }> = ({
  children,
  led = 'off',
  status,
  extra,
  phase,
  error,
}) => (
  <div
    data-prepush-phase={phase}
    role={status ? 'status' : undefined}
    className="flex max-w-[640px] flex-wrap items-center gap-x-2.5 gap-y-1 rounded-xl border border-hairline bg-surface-main px-3 py-2 text-meta text-zinc-400"
  >
    <Led s={led} className="shrink-0" />
    <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">{children}</span>
    {extra}
    {error && (
      // A retry that applied but whose chat message failed lands here, on the retried line (#475 review 2).
      <p role="alert" className="w-full text-meta text-bad [overflow-wrap:anywhere]">
        {error}
      </p>
    )}
  </div>
);

export const PrepushCard: React.FC<PrepushCardProps> = ({
  card,
  bot,
  onRetry,
  onOverride,
  live = false,
  startOverride = false,
  startUnfolded = false,
  story = null,
  after = null,
  onOpenCard,
  onOpenApproval,
}) => {
  const phase = phaseOf(card);
  const [form, setForm] = useState(startOverride && canOverride(card));
  const [reason, setReason] = useState('');
  const [busy, setBusy] = useState<null | 'retry' | 'override'>(null);
  // `keep`: how many status changes the error survives. Only a 409 keeps one, for the refetch it
  // starts; the change after that clears it like any other error (#475 review 1).
  const [error, setErrorState] = useState<{ text: string; keep: number } | null>(null);
  const setError = (text: string | null, keep = 0) => setErrorState(text ? { text, keep } : null);
  const statusNow = useRef(card.status);
  statusNow.current = card.status;
  const [collapsed, setCollapsed] = useState(phase === 'P2' && !live);
  const [unfold, setUnfold] = useState(startUnfolded);
  const cardRef = useRef<HTMLElement>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  // "Push anyway…" that opened the form gets focus back on Cancel or Esc (Ada Later 3).
  const opener = useRef<HTMLButtonElement>(null);
  const refocus = useRef(false);
  // role=alert only on the move into P4 while watched, not on mount or after Cancel (Mira Later 5).
  const [seenPhase, setSeenPhase] = useState(phase);
  const [announceP4, setAnnounceP4] = useState(false);
  if (seenPhase !== phase) {
    // Set during render so the move into P4 renders with role=alert from its first frame.
    setSeenPhase(phase);
    setAnnounceP4(phase === 'P4');
  }
  const hintId = useId();
  const titleId = useId();

  // P2 → P2b: a card that turned clear while watched folds after about 2 s; history starts folded.
  useEffect(() => {
    if (phase !== 'P2') return;
    if (!live) {
      setCollapsed(true);
      return;
    }
    setCollapsed(false);
    const t = window.setTimeout(() => setCollapsed(true), 2000);
    return () => window.clearTimeout(t);
  }, [phase, live]);

  // The server moved the card on: drop a form that no longer applies.
  useEffect(() => {
    if (!canOverride(card)) setForm(false);
    setErrorState((e) => (e && e.keep > 0 ? { ...e, keep: e.keep - 1 } : null));
  }, [card.status, card.id]);

  useEffect(() => {
    if (form) {
      setAnnounceP4(false);
      area.current?.focus({ preventScroll: true });
      cardRef.current?.scrollIntoView?.({ block: 'nearest' });
      return;
    }
    if (refocus.current) {
      refocus.current = false;
      opener.current?.focus({ preventScroll: true });
    }
  }, [form]);

  const openForm = () => {
    setError(null);
    setForm(true);
  };
  const cancelForm = () => {
    refocus.current = true;
    setForm(false);
  };

  if (phase === 'P0') {
    return (
      <Line phase="P0" status>
        Nothing to check: <code className="dv-sha dv-sha-wrap">{card.branch}</code> has no new commits. Nothing was pushed.
      </Line>
    );
  }
  if (phase === 'UN') {
    return (
      <Line phase="UN">
        Pre-push check of <code className="dv-sha dv-sha-wrap">{card.branch}</code>. This version of the app can't show where it is yet. Update the app to see it.
      </Line>
    );
  }
  if (phase === 'SS') {
    return (
      <Line
        phase="SS"
        extra={
          card.superseded_by && onOpenCard ? (
            <button type="button" data-superseded-by={card.superseded_by} className={LINK} onClick={() => onOpenCard(card.superseded_by!)}>
              See the newer push <ArrowDown size={12} aria-hidden="true" />
            </button>
          ) : undefined
        }
      >
        Replaced by a newer push of <code className="dv-sha dv-sha-wrap">{card.branch}</code>. Nothing waits for you here.
      </Line>
    );
  }
  if (phase === 'RT') {
    // 0.9.7: the press is the approval. The card says at once what happens next and follows the new push.
    const lead = story?.lead ?? storyOf(card, [card], bot)?.lead ?? footer(card, bot);
    return (
      <div
        data-prepush-phase="RT"
        className="flex max-w-[640px] flex-col rounded-xl border border-hairline bg-surface-main px-3 py-2 text-meta text-zinc-400"
      >
        <p className="min-w-0 text-zinc-300 [overflow-wrap:anywhere]">{withBranch(lead, card.branch)}</p>
        {story && <StoryLine story={story} card={card} bot={bot} onOpenCard={onOpenCard} onOpenApproval={onOpenApproval} />}
        {error && (
          // A retry that applied but whose chat message failed lands here, on the retried line (#475 review 2).
          <p role="alert" className="mt-1 w-full text-meta text-bad [overflow-wrap:anywhere]">
            {error.text}
          </p>
        )}
      </div>
    );
  }
  if (phase === 'P2' && collapsed) {
    return (
      <Line
        phase="P2b"
        led="ok"
        extra={
          <button type="button" className={LINK} aria-expanded={false} onClick={() => setCollapsed(false)}>
            Show checks <ChevronRight size={12} aria-hidden="true" />
          </button>
        }
      >
        {(() => {
          const p = clearedParts(card);
          const used = card.grant?.used_at ?? null;
          if (used !== null)
            return (
              <span data-pushed>
                <span className="font-medium text-emerald-300">{pushedLine(used)}</span> · <code className="dv-sha dv-sha-wrap">{p.branch}</code>
                {after ? ', with no second OK: you asked for the fix.' : ', with no second OK.'}
              </span>
            );
          return (
            <>
              {p.before}
              <code className="dv-sha dv-sha-wrap">{p.branch}</code>
              {p.after}
            </>
          );
        })()}
      </Line>
    );
  }

  const chip = statusChip(card);
  const fold = foldable(card) && passedRows(card).length > 1;
  const run = async (kind: 'retry' | 'override') => {
    if (busy) return;
    setBusy(kind);
    setError(null);
    const from = card.status;
    try {
      if (kind === 'retry') await onRetry?.(card);
      else {
        await onOverride?.(card, reason);
        setForm(false);
        setReason('');
      }
    } catch (e) {
      // message_not_sent: the card already moved before the chat send, so the next change may clear it.
      // 409: keep it through the refetch, unless the card already moved on its own while we waited.
      const keep = e instanceof DevModeError && e.status === 409 && statusNow.current === from ? 1 : 0;
      setError(e instanceof Error && e.message ? e.message : 'Something went wrong. Try again.', keep);
    } finally {
      setBusy(null);
    }
  };
  const retryLabel = card.status === 'failed' ? 'Run the checks again' : `Ask ${bot} to try again`;
  const ok = reasonOk(reason);

  return (
    <article
      ref={cardRef}
      data-prepush={card.id}
      data-prepush-phase={form ? 'P5' : phase}
      aria-labelledby={titleId}
      className="kw-card w-full max-w-[640px] p-3 sm:p-4"
    >
      {after && (
        <p data-after-retry={after.id} className="-mt-0.5 mb-2 flex min-w-0 flex-wrap items-center gap-x-1.5 text-meta text-zinc-400">
          <span className="min-w-0 [overflow-wrap:anywhere]">{afterRetryLine(card)}</span>
          {onOpenCard && (
            <button type="button" className={LINK} onClick={() => onOpenCard(after.id)}>
              See your request <ArrowUp size={12} aria-hidden="true" />
            </button>
          )}
        </p>
      )}
      <header className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-zinc-800 text-zinc-300" aria-hidden="true">
          <ArrowUpFromLine size={15} />
        </span>
        <div className="min-w-0 flex-1 max-[420px]:min-w-[150px]">
          <h3 id={titleId} className="text-sm font-semibold text-zinc-100 [overflow-wrap:anywhere]">
            Before {bot} pushes
          </h3>
          <p className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-meta text-zinc-400">
            <code className="dv-branch dv-branch-wrap max-w-full">
              <GitBranch size={11} className="i" aria-hidden="true" />
              <span className="h">{card.branch}</span>
            </code>
            {card.diffstat && (
              <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
                <span>
                  {card.diffstat.files} {card.diffstat.files === 1 ? 'file' : 'files'}
                </span>
                <span aria-hidden="true">·</span>
                <DiffStat add={card.diffstat.add} del={card.diffstat.del} />
              </span>
            )}
          </p>
        </div>
        {chip && (
          <span data-prepush-chip aria-live="polite" className={`${CHIP[chip.tone]} shrink-0 max-[420px]:order-last max-[420px]:ml-10`}>
            {chip.led && <Led s={chip.led} className="dv-led-still mx-0.5" />}
            {chip.text}
          </span>
        )}
      </header>

      {card.checks.length > 0 && (
        <ul role="list" aria-label="Checks" className="mt-3 rounded-xl border border-hairline bg-[var(--dv-code-bg)] px-3">
          {fold && (
            <li data-fold className="border-t border-hairline py-2 first:border-t-0 sm:hidden">
              <div className="flex items-center gap-2.5">
                <Led s="ok" className="shrink-0" />
                <span className="min-w-0 flex-1 text-meta text-zinc-300 [overflow-wrap:anywhere]">{foldLine(card)}</span>
                <button type="button" className={LINK} aria-expanded={unfold} onClick={() => setUnfold((u) => !u)}>
                  {unfold ? 'Hide' : 'Show'}
                  <span className="sr-only"> passed checks</span>
                  <ChevronDown size={12} aria-hidden="true" className={unfold ? 'rotate-180' : ''} />
                </button>
              </div>
            </li>
          )}
          {card.checks.map((k) => (
            <Row key={k.key} k={k} card={card} hidePhone={(fold && k.state === 'pass' && !unfold) || k.state === 'not_installed'} />
          ))}
        </ul>
      )}

      {/* Footer per phase */}
      {!form && phase === 'P1' && <p className="mt-3 text-meta text-zinc-400">{footer(card, bot)}</p>}

      {!form && phase === 'P2' && (
        <p className="mt-3 flex items-center gap-2 text-meta text-zinc-300">
          <span className="min-w-0 flex-1">{footer(card, bot)}</span>
          {!live && (
            <button type="button" className={LINK} onClick={() => setCollapsed(true)}>
              Hide checks
            </button>
          )}
        </p>
      )}

      {!form && phase === 'P3' && (
        <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
          <p className="flex min-w-0 flex-1 items-start gap-2 text-meta text-zinc-300">
            <Led s="run" className="dv-led-still mt-[5px] shrink-0" />
            <span className="min-w-0">
              <strong className="font-semibold text-zinc-100">{bot} is fixing {openCount(card) === 1 ? 'it' : 'them'}.</strong> {footer(card, bot)}
            </span>
          </p>
          {onOverride && (
            <button type="button" className={PLAIN} ref={opener} onClick={openForm}>
              Push anyway…
            </button>
          )}
        </div>
      )}

      {!form && phase === 'P7' && (
        <div className="mt-3 flex flex-col gap-2">
          <p className="text-meta text-zinc-300">{footer(card, bot)}</p>
          <div className="flex flex-col gap-2 sm:flex-row">
            {onRetry && (
              <button type="button" className={PLAIN} disabled={busy !== null} onClick={() => void run('retry')}>
                {busy === 'retry' ? 'Asking…' : 'Run the checks again'}
              </button>
            )}
            {onOverride && (
              <button type="button" className={PLAIN} ref={opener} onClick={openForm}>
                Push anyway…
              </button>
            )}
          </div>
        </div>
      )}

      {!form && phase === 'P4' && (
        <div data-needs-you role={announceP4 ? 'alert' : undefined} className="mt-3 rounded-xl border border-amber-400/40 bg-amber-400/[.06] p-3">
          <p className="text-meta text-zinc-200">
            <strong className="font-semibold text-amber-200">{needsYouLead(card, bot)}</strong> {footer(card, bot)}
          </p>
          {(onRetry || onOverride) && (
            <div className="mt-2.5 flex flex-col gap-2 sm:flex-row">
              {onRetry && (
                <button type="button" className={PRIMARY} style={{ background: 'var(--accent-gradient)' }} disabled={busy !== null} onClick={() => void run('retry')}>
                  {busy === 'retry' ? 'Asking…' : retryLabel}
                </button>
              )}
              {onOverride && (
                <button type="button" className={AMBER_OUTLINE} disabled={busy !== null} ref={opener} onClick={openForm}>
                  Push anyway…
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {form && (
        <form
          data-override-form
          className="mt-3 rounded-xl border border-amber-400/40 bg-amber-400/[.06] p-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (ok) void run('override');
          }}
        >
          <p className="text-sm font-semibold text-zinc-100">Push anyway?</p>
          <p className="mt-1 text-meta text-zinc-300">Say why. Your reason is saved on this card, so anyone who looks at this push later sees it.</p>
          <label htmlFor={`${hintId}-r`} className="mt-2.5 block text-meta font-medium text-zinc-200">
            Your reason
          </label>
          <textarea
            id={`${hintId}-r`}
            ref={area}
            required
            rows={3}
            maxLength={MAX_REASON}
            value={reason}
            aria-describedby={hintId}
            placeholder="e.g. the key is a revoked test key"
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.stopPropagation();
                cancelForm();
              }
            }}
            className="mt-1 block w-full resize-y rounded-lg border border-hairline-strong bg-zinc-950/60 px-2.5 py-2 text-base text-zinc-100 placeholder:text-zinc-500 outline-none focus-visible:ring-2 focus-visible:ring-sky-400 sm:text-sm"
          />
          <p id={hintId} className="mt-1 text-[11px] leading-[15px] text-zinc-400">
            Needed, at least {MIN_REASON} characters. This push only; next time the checks run again.
          </p>
          <div className="mt-2.5 flex flex-col gap-2 sm:flex-row">
            <button type="submit" className={AMBER} disabled={!ok || busy !== null}>
              {busy === 'override' ? 'Letting it go…' : 'Push anyway'}
            </button>
            <button type="button" className={PLAIN} disabled={busy !== null} onClick={cancelForm}>
              Cancel
            </button>
          </div>
        </form>
      )}

      {phase === 'P6' && (
        <div className="mt-3 text-meta text-zinc-300">
          <p>{overriddenLine(card)}</p>
          {story && <StoryLine story={story} card={card} bot={bot} onOpenCard={onOpenCard} onOpenApproval={onOpenApproval} />}
          {card.override?.reason && (
            <blockquote className="mt-1.5 border-l-2 border-zinc-600 pl-2 italic text-zinc-300 [overflow-wrap:anywhere]">
              <span className="sr-only">Your reason: </span>
              {card.override.reason}
            </blockquote>
          )}
        </div>
      )}

      {error && (
        <p role="alert" className="mt-2 text-meta text-bad [overflow-wrap:anywhere]">
          {error.text}
        </p>
      )}
    </article>
  );
};
