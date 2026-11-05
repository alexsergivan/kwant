import React, { useEffect, useReducer, useState } from 'react';
import {
  AlertTriangle,
  Check,
  FlaskConical,
  GitPullRequest,
  KeyRound,
  Loader2,
  Lock,
  Package,
  Send,
  ShieldAlert,
  ShieldCheck,
  Target,
  Wrench,
  X,
  Bug,
  Code2,
} from 'lucide-react';
import type { DevFinding, FindingAction, Forge, ReviewCard as Review, ReviewCardProps, Severity, TamperFlag } from '../../dev/types';
import {
  DISMISS_REASONS,
  clock,
  closedDraft,
  dismissReducer,
  dismissedLine,
  findingsSummary,
  FORGE_NAME,
  readOnlyHint,
  readOnlyTip,
  STALE_LINE,
  postCount,
  postLabel,
  postLine,
  postState,
  postedLine,
  riskReasons,
  undoLeft,
} from '../../dev/logic';
import type { PostState } from '../../dev/logic';
import { OutLink, SectionHead, Who, modelLabel, personText } from './parts';

// Severity styling copied from results/Findings.tsx (design §5.6: "reuse as is").
const DOT: Record<Severity, string> = { critical: 'bg-rose-500', high: 'bg-orange-500', medium: 'bg-amber-500', low: 'bg-sky-500' };
const STRIPE: Record<Severity, string> = { critical: 'border-l-rose-500', high: 'border-l-orange-500', medium: 'border-l-amber-500', low: 'border-l-sky-500' };
const SEV_WORD: Record<Severity, string> = { critical: 'Critical', high: 'High', medium: 'Medium', low: 'Low' };
// S4 (WCAG 1.4.1): the severity is a word too, not only the dot's colour.
const SEV_TONE: Record<Severity, string> = { critical: 'text-rose-300', high: 'text-rose-300', medium: 'text-amber-300', low: 'text-teal-300' };

const RISK_BOX = {
  high: { box: 'border-rose-300/35 bg-rose-300/[0.12]', pill: 'bg-rose-300 text-[#120e26]' },
  medium: { box: 'border-amber-300/35 bg-amber-300/[0.12]', pill: 'bg-amber-300 text-[#120e26]' },
  low: { box: 'border-teal-300/35 bg-teal-300/[0.12]', pill: 'bg-teal-300 text-[#120e26]' },
} as const;

const TAMPER_WORD: Record<TamperFlag['kind'], string> = {
  deleted_assert: 'An assert was deleted',
  skip: 'A test is skipped',
  loosened: 'An expectation was loosened',
  snapshot: 'A snapshot was rewritten',
  coverage_drop: 'Coverage dropped',
};

const ref = (f: { file: string | null; line: number | null }) => (f.file ? `${f.file}${f.line !== null ? `:${f.line}` : ''}` : null);

// ---------------------------------------------------------------------------------------------------

const Tamper: React.FC<{ flag: TamperFlag }> = ({ flag }) => (
  <div data-tamper={flag.kind} className="rounded-xl border border-amber-300/35 bg-amber-300/10 p-3">
    <p className="flex items-center gap-1.5 text-[13px] font-semibold text-warn">
      <AlertTriangle size={14} aria-hidden="true" />
      A test was made easier to pass
    </p>
    <p className="mt-1 text-meta text-zinc-200 [overflow-wrap:anywhere]">
      <span className="sr-only">{TAMPER_WORD[flag.kind]}. </span>
      {flag.why}
    </p>
    {flag.excerpt && (
      <figure className="mt-2 overflow-hidden rounded-lg border border-hairline bg-zinc-950/70">
        <figcaption className="border-b border-hairline px-2 py-1 font-mono text-[11px] text-zinc-400 [overflow-wrap:anywhere]">{ref(flag)}</figcaption>
        <pre className="overflow-x-auto py-1 font-mono text-[12px] leading-5" tabIndex={0} aria-label={`Diff excerpt from ${flag.file}`}>
          {flag.excerpt.split('\n').map((l, i) => (
            <div key={i} className={`px-2 ${l.startsWith('-') ? 'bg-rose-500/[0.12] text-rose-200' : l.startsWith('+') ? 'bg-teal-500/10 text-teal-200' : 'text-zinc-300'}`}>
              <span className="sr-only">{l.startsWith('-') ? 'Removed: ' : l.startsWith('+') ? 'Added: ' : ''}</span>
              {l}
            </div>
          ))}
        </pre>
      </figure>
    )}
  </div>
);

const SecTile: React.FC<{ icon: React.ReactNode; name: string; tool: string; state: 'pass' | 'hit' | 'not_run' | 'later'; text: string }> = ({ icon, name, tool, state, text }) => {
  const look =
    state === 'later'
      ? 'border-dashed border-hairline-strong opacity-60'
      : state === 'hit'
        ? 'border-rose-300/50 bg-rose-300/10'
        : 'border-hairline bg-surface-main';
  const tone = state === 'pass' ? 'text-ok' : state === 'hit' ? 'text-bad' : 'text-zinc-400';
  return (
    <li data-sec={tool} data-state={state} className={`min-w-0 rounded-xl border p-3 ${look}`}>
      <p className="flex items-center gap-1.5 text-[13px] font-semibold text-zinc-100">
        <span aria-hidden="true" className="text-zinc-400">
          {icon}
        </span>
        <span className="min-w-0 [overflow-wrap:anywhere]">{name}</span>
      </p>
      <p className="mt-0.5 text-2xs text-zinc-500">{tool}</p>
      <p className={`mt-1 text-meta ${tone}`}>{text}</p>
    </li>
  );
};

const scanText = (s: { state: 'pass' | 'hit' | 'not_run'; count: number }, one: string, many: string, clean: string) =>
  s.state === 'not_run' ? 'Not run' : s.state === 'pass' ? clean : `${s.count} ${s.count === 1 ? one : many} · see finding`;

// ---------------------------------------------------------------------------------------------------

interface FindingRowProps {
  f: DevFinding;
  author: string | null;
  now: number;
  busy: boolean;
  act: (f: DevFinding, a: FindingAction) => Promise<void>;
}

const FindingRow: React.FC<FindingRowProps> = ({ f, author, now, busy, act }) => {
  const [draft, send] = useReducer(dismissReducer, closedDraft);
  const formId = `dismiss-${f.id}`;
  const where = ref(f);

  if (f.state === 'dismissed') {
    return (
      <li data-finding={f.id} data-state="dismissed" className={`rounded-xl border border-zinc-700/60 border-l-[3px] bg-zinc-900/40 px-3 py-2 text-xs ${STRIPE[f.severity]}`}>
        <span className="text-zinc-400 line-through decoration-zinc-600">{f.title}</span>
        <span className="ml-2 text-zinc-400">{dismissedLine(f)}</span>
      </li>
    );
  }

  const left = undoLeft(f, now);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!draft.reason || draft.sending) return;
    send({ type: 'submit' });
    try {
      await act(f, { action: 'dismiss', reason: draft.reason, ...(draft.note.trim() ? { note: draft.note.trim() } : {}) });
      send({ type: 'done' });
    } catch (err) {
      send({ type: 'failed', error: err instanceof Error ? err.message : 'That didn’t go through. Try again.' });
    }
  };

  return (
    <li data-finding={f.id} data-state={f.state} className={`rounded-xl border border-zinc-700/60 border-l-[3px] bg-zinc-900/60 p-3 text-xs shadow-[var(--kws-shadow)] ${STRIPE[f.severity]}`}>
      <div className="flex items-start gap-2">
        <span className={`mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full ${DOT[f.severity]}`} aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <div className="font-semibold leading-snug text-zinc-100 [overflow-wrap:anywhere]">
            <span data-sev-word={f.severity} className={`mr-1.5 text-[10px] font-semibold uppercase tracking-wide ${SEV_TONE[f.severity]}`}>
              {SEV_WORD[f.severity]}
              <span className="sr-only">:</span>
            </span>
            {f.title}
          </div>
          {f.detail && <p className="mt-1 leading-relaxed text-zinc-400 [overflow-wrap:anywhere]">{f.detail}</p>}
          {where && (
            <div className="mt-1.5">
              <code className="rounded bg-zinc-900 px-1.5 py-0.5 font-mono text-[11px] text-sky-300 [overflow-wrap:anywhere]">{where}</code>
            </div>
          )}
          {f.fix && (
            <p className="mt-1.5 leading-relaxed text-zinc-300 [overflow-wrap:anywhere]">
              <span className="font-semibold text-zinc-200">Suggested fix:</span> {f.fix}
            </p>
          )}

          <div className="mt-2 flex flex-wrap items-center gap-2" aria-live="polite">
            {f.state === 'open' && !draft.open && (
              <>
                <button type="button" disabled={busy} onClick={() => void act(f, { action: 'fix' })} className="kw-btn-primary min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
                  <Wrench size={13} aria-hidden="true" />
                  Fix it
                </button>
                <button type="button" disabled={busy} aria-expanded={false} aria-controls={formId} onClick={() => send({ type: 'open' })} className="kw-btn-ghost min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
                  Dismiss
                </button>
              </>
            )}
            {f.state === 'open' && f.send_error && <span className="text-bad">Couldn’t send it: {f.send_error}</span>}
            {f.state === 'sent' && (
              <>
                <span className="inline-flex items-center gap-1 text-ok">
                  <Check size={13} aria-hidden="true" />
                  Sent to {f.sent_to ?? author ?? 'the author'} as a task{clock(f.sent_at) ? ` · ${clock(f.sent_at)}` : ''}
                </span>
                {left > 0 && (
                  <button type="button" disabled={busy} onClick={() => void act(f, { action: 'undo_fix' })} className="kw-btn-ghost min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
                    Undo <span className="text-zinc-400">({Math.ceil(left / 1000)} s)</span>
                  </button>
                )}
              </>
            )}
          </div>

          {draft.open && (
            <form id={formId} onSubmit={submit} aria-label={`Dismiss: ${f.title}`} className="mt-2 rounded-lg border border-hairline bg-surface-main p-2.5">
              <fieldset disabled={draft.sending}>
                <legend className="text-meta text-zinc-200">Why dismiss? Rigel learns from this.</legend>
                <div role="radiogroup" aria-label="Reason" className="mt-2 flex flex-wrap gap-1.5">
                  {DISMISS_REASONS.map((r) => {
                    const on = draft.reason === r.id;
                    return (
                      <button
                        key={r.id}
                        type="button"
                        role="radio"
                        aria-checked={on}
                        onClick={() => send({ type: 'reason', reason: r.id })}
                        className={`kw-focus min-h-[44px] rounded-full border px-3 text-xs sm:min-h-[28px] ${on ? 'border-sky-400 bg-sky-500/15 text-zinc-50' : 'border-hairline text-zinc-300 hover:border-hairline-strong'}`}
                      >
                        {r.label}
                      </button>
                    );
                  })}
                </div>
                <label className="mt-2 block text-2xs text-zinc-400">
                  Note (optional)
                  <textarea
                    value={draft.note}
                    onChange={(e) => send({ type: 'note', note: e.target.value })}
                    rows={2}
                    maxLength={500}
                    className="kw-focus mt-1 block w-full resize-y rounded-lg border border-hairline bg-surface-main px-2 py-1.5 text-xs text-zinc-100"
                  />
                </label>
                {draft.error && (
                  <p role="alert" className="mt-1 text-bad">
                    {draft.error}
                  </p>
                )}
                <div className="mt-2 flex flex-wrap gap-2">
                  <button type="submit" disabled={!draft.reason} aria-busy={draft.sending || undefined} className="kw-btn-primary min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
                    {draft.sending ? 'Dismissing…' : 'Dismiss finding'}
                  </button>
                  <button type="button" onClick={() => send({ type: 'cancel' })} className="kw-btn-ghost min-h-[44px] px-2.5 py-1 text-xs sm:min-h-0">
                    Cancel
                  </button>
                </div>
              </fieldset>
            </form>
          )}
        </div>
      </div>
    </li>
  );
};

// ---------------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------------
// 7. Footer: Post to {Forge} (0.7.1, design §5.7, plan/post-to-github.md + amendment). With no forge there is
// no Post button and no lock line.

const PostFooter: React.FC<{
  id: string;
  state: PostState;
  forge: Forge | null;
  count: number;
  login: string | null;
  prLink: string | null;
  onPost: () => void;
}> = ({
  id,
  state,
  forge,
  count,
  login,
  prLink,
  onPost,
}) => {
  const hintId = `${id}-post-hint`;
  const name = forge ? FORGE_NAME[forge] : null;
  const label = forge ? postLabel(count, forge) : '';
  // One row: the lock line (or the result) on the left, the button on the right. Under 640 px the button
  // wraps under the text and keeps a 44 px touch target.
  const btn = 'kw-btn-ghost min-h-[44px] w-full px-3 py-1 text-xs sm:min-h-0 sm:w-auto';
  let line: React.ReactNode;
  let action: React.ReactNode = null;
  let extra: React.ReactNode = null;
  switch (state.kind) {
    case 'none':
      line = forge && (
        <span className="inline-flex min-w-0 items-start gap-1.5 text-xs text-zinc-400">
          <Lock size={13} aria-hidden="true" className="mt-px shrink-0" />
          {postLine(null)}
        </span>
      );
      break;
    case 'posted':
      line = (
        <span data-post-done className="inline-flex min-w-0 flex-wrap items-center gap-x-1.5 text-xs text-ok">
          <Check size={13} aria-hidden="true" className="shrink-0" />
          {postedLine(state.at)}
          {state.url && (
            <>
              <span aria-hidden="true" className="text-zinc-500">·</span>
              <OutLink href={state.url}>{name ? `View on ${name}` : 'View review'}</OutLink>
            </>
          )}
        </span>
      );
      break;
    case 'stale':
      line = (
        <span data-post-stale className="inline-flex min-w-0 items-start gap-1.5 text-xs text-warn">
          <AlertTriangle size={13} aria-hidden="true" className="mt-px shrink-0" />
          {STALE_LINE}
        </span>
      );
      break;
    case 'failed':
      line = (
        <span data-post-failed className="inline-flex min-w-0 items-start gap-1.5 text-xs text-bad [overflow-wrap:anywhere]">
          <AlertTriangle size={13} aria-hidden="true" className="mt-px shrink-0" />
          Couldn’t post: {state.message}
        </span>
      );
      action = (
        <button type="button" onClick={onPost} className={btn}>
          <Send size={13} aria-hidden="true" />
          Try again
        </button>
      );
      break;
    default:
      line = (
        <span className="inline-flex min-w-0 items-start gap-1.5 text-xs text-zinc-400">
          <Lock size={13} aria-hidden="true" className="mt-px shrink-0" />
          {postLine(login)}
        </span>
      );
      if (state.kind === 'read_only' && forge) {
        // aria-disabled, not disabled: it stays focusable, so keyboard and screen-reader users reach the
        // tooltip's words (also tied on with aria-describedby) and the hint line below.
        action = (
          <button
            type="button"
            aria-disabled="true"
            aria-describedby={hintId}
            title={readOnlyTip(forge)}
            data-post-read-only
            onClick={(e) => e.preventDefault()}
            className={`${btn} cursor-not-allowed opacity-60 hover:bg-white/[0.03] hover:text-zinc-200`}
          >
            <Send size={13} aria-hidden="true" />
            {label}
            <span className="sr-only">. {readOnlyTip(forge)}.</span>
          </button>
        );
        extra = (
          <p id={hintId} className="basis-full text-2xs text-zinc-400">
            {readOnlyHint(forge)}
          </p>
        );
      } else {
        const busy = state.kind === 'posting';
        action = (
          <button type="button" disabled={busy} aria-busy={busy || undefined} onClick={onPost} data-post-button className={`${btn} disabled:cursor-wait disabled:opacity-70`}>
            {busy ? <Loader2 size={13} aria-hidden="true" className="motion-safe:animate-spin" /> : <Send size={13} aria-hidden="true" />}
            {label}
            {busy && <span className="sr-only">. Posting.</span>}
          </button>
        );
      }
  }
  const link = !!(prLink && name && state.kind !== 'posted');
  // Nothing to post and nowhere to open (no forge): no empty footer rule either.
  if (!line && !action && !link) return null;
  return (
    <footer data-review-footer data-post-state={state.kind} className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline pt-3">
      <div className="min-w-0 flex-1 basis-full sm:basis-0">{line}</div>
      {/* F7 (user-flow.md): Post to {Forge} first, then "Open on {Forge}". With no forge, neither. */}
      {action && <div className="basis-full sm:basis-auto">{action}</div>}
      {link && (
        <span data-forge-link className="text-meta">
          <OutLink href={prLink!} className="min-h-[44px] sm:min-h-0">{`Open on ${name}`}</OutLink>
        </span>
      )}
      {extra}
      <span role="status" aria-live="polite" className="sr-only">
        {state.kind === 'posted' ? postedLine(state.at) : state.kind === 'failed' ? `Couldn’t post: ${state.message}` : state.kind === 'stale' ? STALE_LINE : ''}
      </span>
    </footer>
  );
};

/** One PR in review: risk, intent, tests, security and findings (design §5, parts 1–7). */
export const ReviewCard: React.FC<ReviewCardProps> = ({ card: given, onFinding, sample = false, now: fixedNow, onPost, forge = null, forgeLogin = null }) => {
  const [card, setCard] = useState<Review>(given);
  const [busy, setBusy] = useState<string | null>(null);
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => setCard(given), [given]);
  const now = fixedNow ?? tick;

  // Tick once a second only while an Undo is on screen.
  const undoing = fixedNow === undefined && card.findings.some((f) => undoLeft(f, tick) > 0);
  useEffect(() => {
    if (!undoing) return;
    const t = window.setInterval(() => setTick(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [undoing]);

  const act = async (f: DevFinding, a: FindingAction) => {
    if (busy) return;
    setBusy(f.id);
    try {
      setCard(await onFinding(card.id, f.id, a));
      setTick(Date.now());
    } catch (err) {
      if (a.action === 'dismiss') throw err;
      const msg = err instanceof Error ? err.message : 'it didn’t go through';
      setCard((c) => ({ ...c, findings: c.findings.map((x) => (x.id === f.id ? { ...x, send_error: msg } : x)) }));
    } finally {
      setBusy(null);
    }
  };

  const [posting, setPosting] = useState(false);
  const post = async () => {
    if (!onPost || posting || card.posted) return;
    setPosting(true);
    try {
      setCard(await onPost(card.id));
    } catch (err) {
      // The route answers 200 with post_error for the forge's refusals; a throw is the network or a 4xx from kwant.
      const message = err instanceof Error && err.message ? err.message : 'it didn’t go through';
      setCard((c) => ({ ...c, post_error: { kind: 'failed', message } }));
    } finally {
      setPosting(false);
    }
  };

  const author = personText(card.author);
  const authorModel = card.author ? modelLabel(card.author.engine, card.author.model) : null;
  const reviewerModel = card.reviewer ? modelLabel(card.reviewer.engine, card.reviewer.model) : null;
  const headingId = `review-${card.id || card.pr}-h`;
  const tile = card.draft ? 'bg-zinc-500/15 text-zinc-300' : 'bg-teal-400/15 text-teal-300';
  const summary = findingsSummary(card.findings);
  const sec = card.security as Review['security'] & { osv?: unknown; semgrep?: unknown };

  return (
    <article data-dev-card="review" data-pr={card.pr} aria-labelledby={headingId} className="kw-card min-w-0 scroll-mt-3 p-4">
      {/* 1. Header */}
      <header className="flex items-start gap-3">
        <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-xl ${tile}`} aria-hidden="true">
          <GitPullRequest size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <h3 id={headingId} className="min-w-0 rounded-sm text-title font-semibold text-zinc-50 [overflow-wrap:anywhere] focus:outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400">
              #{card.pr} {card.title}
            </h3>
            {card.draft && <span className="kw-chip">Draft</span>}
            {sample && <span className="kw-chip kw-chip-info">Sample PR</span>}
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-meta text-zinc-400">
            {card.branch && (
              <code className="rounded-md border border-hairline bg-surface-main px-1.5 py-0.5 font-mono text-[11px] text-zinc-300 [overflow-wrap:anywhere]">
                {card.branch}
                {card.base && ` → ${card.base}`}
              </code>
            )}
            {author && (
              <span className="inline-flex flex-wrap items-center gap-1">
                by <Who name={author.name} />
                {authorModel && <span>on {authorModel}</span>}
              </span>
            )}
            {card.reviewer && (
              <span className="inline-flex flex-wrap items-center gap-1">
                <span aria-hidden="true">·</span> reviewed by <Who name={card.reviewer.name} />
                {reviewerModel && <span>on {reviewerModel}</span>}
              </span>
            )}
          </div>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {card.reviewer && (card.same_engine ? <span className="kw-chip kw-chip-warn">Same engine as the author</span> : <span className="kw-chip kw-chip-accent">Different engine from the author</span>)}
            {!card.reviewer && <span className="kw-chip kw-chip-warn">{card.note ?? 'No reviewer yet'}</span>}
          </div>
        </div>
      </header>

      {card.status === 'pending' && <p className="mt-3 text-meta text-zinc-400">Reviewing this PR…</p>}
      {card.error && (
        <p className="mt-3 text-meta text-zinc-400">
          The review didn’t finish: <span className="text-zinc-300">{card.error}</span>
        </p>
      )}

      {/* 2. Risk */}
      {card.risk && (
        <div data-risk={card.risk.level} className={`mt-3 flex flex-wrap items-center gap-1.5 rounded-xl border p-2.5 ${RISK_BOX[card.risk.level].box}`}>
          <span className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-semibold ${RISK_BOX[card.risk.level].pill}`}>
            <ShieldAlert size={13} aria-hidden="true" />
            {card.risk.level === 'high' ? 'High risk' : card.risk.level === 'medium' ? 'Medium risk' : 'Low risk'}
          </span>
          {riskReasons(card.risk).map((r) => (
            <span key={r} className="kw-chip">
              {r}
            </span>
          ))}
        </div>
      )}

      {/* 3. Intent */}
      {card.intent && (
        <section className="mt-4" aria-label="Did it do what was asked?">
          <SectionHead icon={<Target size={12} aria-hidden="true" />}>Did it do what was asked?</SectionHead>
          {card.intent.none ? (
            <p className="text-meta text-zinc-400">No task to compare with. Checked against the PR description.</p>
          ) : (
            <p className="rounded-lg border border-hairline bg-surface-main px-3 py-2 text-meta text-zinc-300 [overflow-wrap:anywhere]">
              <span className="font-semibold text-zinc-100">Asked ({card.intent.source}):</span> {card.intent.asked}
            </p>
          )}
          {card.intent.rows.length > 0 && (
            <ul role="list" className="mt-2 space-y-1.5">
              {card.intent.rows.map((r, i) => (
                <li key={i} data-intent={r.state} className="flex items-start gap-2 text-meta text-zinc-200">
                  <span
                    aria-hidden="true"
                    className={`mt-0.5 flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full ${r.state === 'ok' ? 'bg-teal-400/20 text-teal-300' : r.state === 'missing' ? 'bg-rose-400/20 text-rose-300' : 'bg-amber-400/20 text-amber-300'}`}
                  >
                    {r.state === 'ok' ? <Check size={11} /> : r.state === 'missing' ? <X size={11} /> : <AlertTriangle size={10} />}
                  </span>
                  <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">
                    <span className="sr-only">{r.state === 'ok' ? 'Done: ' : r.state === 'missing' ? 'Missing: ' : 'Extra: '}</span>
                    {r.text}
                  </span>
                  {r.state === 'missing' && <span className="kw-chip kw-chip-bad shrink-0">Did less than asked</span>}
                  {r.state === 'extra' && <span className="kw-chip kw-chip-warn shrink-0">Did more than asked</span>}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      {/* 4. Tests */}
      <section className="mt-4" aria-label="Tests">
        <SectionHead icon={<FlaskConical size={12} aria-hidden="true" />}>Tests</SectionHead>
        {card.tamper.flags.length > 0 ? (
          <div className="space-y-2">
            {card.tamper.flags.map((t, i) => (
              <Tamper key={i} flag={t} />
            ))}
          </div>
        ) : (
          <p className="flex items-center gap-1.5 text-meta text-ok">
            <Check size={13} aria-hidden="true" />
            Tests changed with the code. Nothing loosened.
          </p>
        )}
        {card.tamper.unread.length > 0 && (
          <div data-tamper-unread={card.tamper.unread.length} className="mt-2 text-meta text-zinc-400">
            <p className="flex items-center gap-1.5 text-warn">
              <AlertTriangle size={13} aria-hidden="true" />
              Couldn’t read {card.tamper.unread.length} {card.tamper.unread.length === 1 ? 'file' : 'files'}: GitHub sent no diff, so nothing in{' '}
              {card.tamper.unread.length === 1 ? 'it' : 'them'} was checked.
            </p>
            <ul role="list" className="mt-1 space-y-0.5 pl-5">
              {card.tamper.unread.map((p) => (
                <li key={p} className="font-mono text-[11px] text-zinc-300 [overflow-wrap:anywhere]">
                  {p}
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      {/* 5. Security */}
      <section className="mt-4" aria-label="Security checks">
        <SectionHead icon={<ShieldCheck size={12} aria-hidden="true" />}>Security checks</SectionHead>
        <ul role="list" className="grid grid-cols-1 gap-2 min-[420px]:grid-cols-2 lg:grid-cols-4">
          <SecTile icon={<KeyRound size={13} />} name="Secrets" tool="Gitleaks" state={sec.secrets.state} text={scanText(sec.secrets, 'secret', 'secrets', 'None found')} />
          <SecTile icon={<Package size={13} />} name="New packages" tool="Registry check" state={sec.packages.state} text={scanText(sec.packages, 'package', 'packages', 'All real')} />
          <SecTile icon={<Bug size={13} />} name="Dependencies" tool="OSV" state="later" text="in 0.7.x" />
          <SecTile icon={<Code2 size={13} />} name="Code patterns" tool="Semgrep" state="later" text="in 0.7.x" />
        </ul>
      </section>

      {/* 6. Findings */}
      <section className="mt-4" aria-label="Findings">
        <SectionHead extra={summary ? <span className="normal-case tracking-normal text-zinc-500">· {summary}</span> : null}>Findings</SectionHead>
        {card.findings.length === 0 ? (
          <p className="text-meta text-zinc-400">{card.status === 'ready' ? 'No findings.' : 'No findings yet.'}</p>
        ) : (
          <ul role="list" className="space-y-1.5">
            {card.findings.map((f) => (
              <FindingRow key={f.id} f={f} author={author?.name ?? null} now={now} busy={busy !== null} act={act} />
            ))}
          </ul>
        )}
      </section>

      {card.cut && <p className="mt-3 text-2xs text-zinc-400">{card.cut}</p>}

      {/* 7. Footer: Post to {Forge} (design §5.7) */}
      <PostFooter
        id={headingId}
        state={postState(card, { canPost: !!onPost && !sample, posting, forge })}
        forge={card.posted?.forge ?? forge}
        count={postCount(card.findings)}
        login={forgeLogin}
        prLink={card.links.pr}
        onPost={() => void post()}
      />
    </article>
  );
};
