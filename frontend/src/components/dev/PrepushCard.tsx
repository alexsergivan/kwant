// 0.8.0 B3: the pre-push card in the author bot's thread (Mira b3-fresh-eyes.md §2–§4, Juno
// fresh-eyes-audit.md §4). Every state and number comes from the server card; the client only
// picks the words. Amber appears only where the person must act (Needs you, Push anyway). There is
// never an "always allow" or "skip this check": an override covers one push of one commit.

import React, { useEffect, useId, useRef, useState } from 'react';
import { ArrowUpFromLine, ChevronDown, ChevronRight, GitBranch } from 'lucide-react';
import { DiffStat, Led } from './parts';
import {
  CHECK_NAME,
  STATE_LED,
  STATE_WORD,
  canOverride,
  clearedLine,
  foldLine,
  foldable,
  footer,
  needsYouLead,
  overriddenLine,
  passedRows,
  phaseOf,
  reasonOk,
  rowFindings,
  statusChip,
  toolLine,
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
  const tone = k.state === 'hit' ? 'text-rose-300' : dim ? 'text-zinc-500' : 'text-zinc-300';
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
          <p className={`min-w-0 flex-1 text-meta [overflow-wrap:anywhere] ${tone}`}>{k.text}</p>
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

/** A one-line card (P0, retried, cleared and folded). */
const Line: React.FC<{ children: React.ReactNode; led?: 'ok' | 'off'; status?: boolean; extra?: React.ReactNode; phase: string }> = ({ children, led = 'off', status, extra, phase }) => (
  <div
    data-prepush-phase={phase}
    role={status ? 'status' : undefined}
    className="flex max-w-[640px] flex-wrap items-center gap-x-2.5 gap-y-1 rounded-xl border border-hairline bg-surface-main px-3 py-2 text-meta text-zinc-400"
  >
    <Led s={led} className="shrink-0" />
    <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">{children}</span>
    {extra}
  </div>
);

export const PrepushCard: React.FC<PrepushCardProps> = ({ card, bot, onRetry, onOverride, live = false, startOverride = false, startUnfolded = false }) => {
  const phase = phaseOf(card);
  const [form, setForm] = useState(startOverride && canOverride(card));
  const [reason, setReason] = useState('');
  const [busy, setBusy] = useState<null | 'retry' | 'override'>(null);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(phase === 'P2' && !live);
  const [unfold, setUnfold] = useState(startUnfolded);
  const cardRef = useRef<HTMLElement>(null);
  const area = useRef<HTMLTextAreaElement>(null);
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
    setError(null);
  }, [card.status, card.id]);

  useEffect(() => {
    if (!form) return;
    area.current?.focus({ preventScroll: true });
    cardRef.current?.scrollIntoView?.({ block: 'nearest' });
  }, [form]);

  if (phase === 'P0') {
    return (
      <Line phase="P0" status>
        Nothing to check: <code className="dv-sha">{card.branch}</code> has no new commits. Nothing was pushed.
      </Line>
    );
  }
  if (phase === 'RT') {
    return (
      <Line phase="RT">
        You asked for a new check of <code className="dv-sha">{card.branch}</code>. {bot}'s next push of this commit is checked again from try 1.
      </Line>
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
        {clearedLine(card)}
      </Line>
    );
  }

  const chip = statusChip(card);
  const fold = foldable(card) && passedRows(card).length > 1;
  const run = async (kind: 'retry' | 'override') => {
    if (busy) return;
    setBusy(kind);
    setError(null);
    try {
      if (kind === 'retry') await onRetry?.(card);
      else {
        await onOverride?.(card, reason);
        setForm(false);
        setReason('');
      }
    } catch (e) {
      setError(e instanceof Error && e.message ? e.message : 'Something went wrong. Try again.');
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
      <header className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-zinc-800 text-zinc-300" aria-hidden="true">
          <ArrowUpFromLine size={15} />
        </span>
        <div className="min-w-0 flex-1">
          <h3 id={titleId} className="text-sm font-semibold text-zinc-100 [overflow-wrap:anywhere]">
            Before {bot} pushes
          </h3>
          <p className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-meta text-zinc-400">
            <code className="dv-branch max-w-full">
              <GitBranch size={11} className="i" aria-hidden="true" />
              <span className="h [overflow-wrap:anywhere]">{card.branch}</span>
            </code>
            {card.diffstat && (
              <span className="inline-flex items-center gap-1.5">
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
            {chip.led && <Led s={chip.led} className="dv-led-still" />}
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
              <strong className="font-semibold text-zinc-100">{bot} is fixing them.</strong> {footer(card, bot)}
            </span>
          </p>
          {onOverride && (
            <button type="button" className={PLAIN} onClick={() => setForm(true)}>
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
              <button type="button" className={PLAIN} onClick={() => setForm(true)}>
                Push anyway…
              </button>
            )}
          </div>
        </div>
      )}

      {!form && phase === 'P4' && (
        <div data-needs-you role="alert" className="mt-3 rounded-xl border border-amber-400/40 bg-amber-400/[.06] p-3">
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
                <button type="button" className={AMBER_OUTLINE} disabled={busy !== null} onClick={() => setForm(true)}>
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
                setForm(false);
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
            <button type="button" className={PLAIN} disabled={busy !== null} onClick={() => setForm(false)}>
              Cancel
            </button>
          </div>
        </form>
      )}

      {phase === 'P6' && (
        <div className="mt-3 text-meta text-zinc-300">
          <p>{overriddenLine(card)}</p>
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
          {error}
        </p>
      )}
    </article>
  );
};
