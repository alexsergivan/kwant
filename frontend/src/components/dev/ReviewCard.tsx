import React, { useEffect, useRef, useState } from 'react';
import {
  AlertTriangle,
  Check,
  FlaskConical,
  GitPullRequest,
  Loader2,
  ShieldAlert,
  ShieldCheck,
  Target,
  Wrench,
  X,
  GitBranch,
} from 'lucide-react';
import { FreshEyesChip } from './FreshEyesChip';
import { ChangesSection } from './LineNotes';
import { verdictShort } from '../../api/inbox';
import { inboxNav, openReview, reviewForPr, useInbox } from '../../inboxStore';
import type { DevFinding, FindingAction, Forge, ReviewCard as Review, ReviewCardProps, Severity, TamperFlag } from '../../dev/types';
import {
  clock,
  dismissedLine,
  findingsSummary,
  FORGE_NAME,
  readOnlyFix,
  commentsWord,
  readOnlyTip,
  STALE_LINE,
  postCount,
  postLabel,
  postState,
  postedLine,
  riskReasons,
  undoLeft,
  lgtm,
  lgtmLabel,
  readingLine,
  cardForge,
  postForge,
  laterForge,
  prName,
  prRef,
  NO_FORGE_LINE,
} from '../../dev/logic';
import type { LedState, PostState } from '../../dev/logic';
import { Led, OutLink, SectionHead, Who, modelLabel, personText } from './parts';
import { DismissForm, useDismiss } from './DismissForm';
import { TicketMatches, itemFinding, matchFindingIds } from './TicketMatches';

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
      <figure className="mt-2 overflow-hidden rounded-lg border border-hairline bg-[var(--dv-code-bg)]">
        {ref(flag) && (
          <figcaption className="border-b border-hairline px-2 py-1.5">
            <span data-tamper-ref className="dv-sha max-w-full [overflow-wrap:anywhere]">
              {ref(flag)}
            </span>
          </figcaption>
        )}
        <pre className="overflow-x-auto py-1 font-mono text-[12px] leading-5" tabIndex={0} aria-label={`Diff excerpt from ${flag.file}`}>
          {flag.excerpt.split('\n').map((l, i) => (
            <div key={i} className={`px-2 ${l.startsWith('-') ? 'bg-[var(--dv-del-bg)] text-[var(--dv-del)]' : l.startsWith('+') ? 'bg-[var(--dv-add-bg)] text-[var(--dv-add)]' : 'text-zinc-300'}`}>
              <span className="sr-only">{l.startsWith('-') ? 'Removed: ' : l.startsWith('+') ? 'Added: ' : ''}</span>
              {l}
            </div>
          ))}
        </pre>
      </figure>
    )}
  </div>
);

// §6: an LED says the state (ok / bad for a hit / off when it didn't run); the tool sits top right in mono.
const SEC_LED = { pass: 'ok', hit: 'bad', not_run: 'off', later: 'off' } as const satisfies Record<string, LedState>;

const SecTile: React.FC<{ name: string; tool: string; mono: string; state: 'pass' | 'hit' | 'not_run' | 'later'; text: string }> = ({ name, tool, mono, state, text }) => {
  const look =
    state === 'later'
      ? 'border-dashed border-hairline-strong opacity-60'
      : state === 'hit'
        ? 'border-rose-300/50 bg-rose-300/10'
        : 'border-hairline bg-surface-main';
  const tone = state === 'pass' ? 'text-ok' : state === 'hit' ? 'text-bad' : 'text-zinc-400';
  return (
    <li data-sec={tool} data-state={state} className={`min-w-0 rounded-xl border p-3 ${look}`}>
      <p className="flex items-center gap-2 text-[13px] font-semibold text-zinc-100">
        <Led s={SEC_LED[state]} />
        <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">{name}</span>
        <span className="shrink-0 font-mono text-[10.5px] font-medium text-zinc-500">
          <span className="sr-only"> checked by </span>
          {mono}
        </span>
      </p>
      <p className={`mt-1 text-meta ${tone}`}>{text}</p>
    </li>
  );
};

/** 0.8.0 B3: the semgrep tile from the real run. Not installed (or no repo rules) is the dashed
 *  `later` look, "Not installed. Optional."; a PR card that never ran it says so plainly. */
export function semgrepTile(s: Review['security']['semgrep'] | undefined): { state: 'pass' | 'hit' | 'not_run' | 'later'; text: string } {
  if (!s) return { state: 'later', text: 'Runs before a push, not on pull requests' };
  if (s.state === 'pass') return { state: 'pass', text: 'No matches' };
  if (s.state === 'hit') return { state: 'hit', text: `${s.count} ${s.count === 1 ? 'match' : 'matches'} · see finding` };
  if (s.reason && /isn't installed|no \.semgrep/.test(s.reason)) return { state: 'later', text: 'Not installed. Optional.' };
  return { state: 'not_run', text: s.reason ? `Didn't run: ${s.reason}` : "Didn't run" };
}

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
  const dismiss = useDismiss(f, act);
  const { draft, send } = dismiss;
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

          <DismissForm id={formId} title={f.title} d={dismiss} />
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
}> = ({ id, state, forge, count, login, prLink, onPost }) => {
  // Style v2 §7: forge null means no footer at all (nothing to post, nowhere named to open).
  if (!forge) return null;
  const name = FORGE_NAME[forge];
  const label = postLabel(count, forge);
  const lineId = `${id}-post-line`;
  const At = ({ who }: { who: string }) => <span className="font-mono text-zinc-100">@{who}</span>;
  const withLed = (s: LedState, body: React.ReactNode, attr: Record<string, string>) => (
    <span {...attr} className="inline-flex min-w-0 items-start gap-[7px]">
      <Led s={s} className="mt-[5px] shrink-0" />
      <span className="min-w-0 [overflow-wrap:anywhere]">{body}</span>
    </span>
  );
  // Phone: the button gets its own full-width row at 44 px; from 640 px it sits on the right of the line.
  const btn = 'min-h-[44px] w-full justify-center px-3 py-1 text-xs sm:min-h-[32px] sm:w-auto';
  let line: React.ReactNode;
  let action: React.ReactNode = null;
  const lockLine = (who: string | null) => (
    <span>
      Nothing is posted until you press it.
      {who && (
        <>
          {' '}Posts as <At who={who} />.
        </>
      )}
    </span>
  );
  // A disabled Post button (read only, stale): aria-disabled so it stays focusable and its title is reachable.
  const off = (title?: string) => (
    <button
      type="button"
      aria-disabled="true"
      aria-describedby={lineId}
      title={title}
      data-post-disabled
      onClick={(e) => e.preventDefault()}
      className={`kw-btn-ghost ${btn} cursor-not-allowed opacity-60 hover:bg-white/[0.03] hover:text-zinc-200`}
    >
      {label}
    </button>
  );
  switch (state.kind) {
    case 'none':
      line = lockLine(null);
      break;
    case 'posted':
      line = withLed(
        'ok',
        <>
          Posted{clock(state.at) ? ` ${clock(state.at)}` : ''}
          {state.login && (
            <>
              {' '}as <At who={state.login} />
            </>
          )}
          {commentsWord(state.comments) && <> · {commentsWord(state.comments)}</>}
        </>,
        { 'data-post-done': '' },
      );
      if (state.url) action = <OutLink href={state.url} className="min-h-[44px] sm:min-h-0">{`View on ${name}`}</OutLink>;
      break;
    case 'read_only':
      line = withLed(
        'warn',
        <>
          {readOnlyTip(forge)}. Give the token <code className="dv-code">{readOnlyFix(forge)}</code> to post.
        </>,
        { 'data-post-read-only': '' },
      );
      // F1 (QA 0.7.1): read_only is cleared only by a post, so the press must reach the server once the token is fixed.
      action = (
        <button type="button" onClick={onPost} aria-describedby={lineId} data-post-retry className={`kw-btn-ghost ${btn}`}>
          Try again
        </button>
      );
      break;
    case 'stale':
      line = withLed('run', STALE_LINE, { 'data-post-stale': '' });
      action = off();
      break;
    case 'failed':
      line = withLed('bad', <>Couldn’t post: {state.message}</>, { 'data-post-failed': '' });
      action = state.retry ? (
        <button type="button" onClick={onPost} data-post-retry className={`kw-btn-ghost ${btn}`}>
          Try again
        </button>
      ) : null;
      break;
    default: {
      const busy = state.kind === 'posting';
      line = lockLine(login);
      action = (
        <button type="button" disabled={busy} aria-busy={busy || undefined} onClick={onPost} data-post-button className={`kw-btn-primary ${btn} disabled:cursor-wait`}>
          {busy ? (
            <>
              <Loader2 size={12} aria-hidden="true" className="motion-safe:animate-spin" />
              Posting…
            </>
          ) : (
            label
          )}
        </button>
      );
    }
  }
  // 0.7.4: GitLab and Gitea: posting there comes later (G7). The server's words, no button.
  if (state.kind === 'later') {
    line = withLed('off', state.message, { 'data-post-later': '' });
    action = null;
  }
  // F7 (user-flow.md): Post first, then "Open on {Forge}". Not after posting: View on {Forge} replaces it.
  const link = !!prLink && state.kind !== 'posted';
  // F4: no review, no Post footer; the card keeps only its link to the PR.
  if (state.kind === 'no_review')
    return link ? (
      <footer data-review-footer data-post-state="no_review" className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline pt-3 text-[12.5px] text-zinc-400">
        <span data-forge-link className="text-meta">
          <OutLink href={prLink!} className="min-h-[44px] sm:min-h-0">{`Open on ${name}`}</OutLink>
        </span>
      </footer>
    ) : null;
  return (
    <footer data-review-footer data-post-state={state.kind} className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline pt-3 text-[12.5px] text-zinc-400">
      <div id={lineId} className="min-w-0 flex-1 basis-full sm:basis-0">{line}</div>
      {action && <div className="flex basis-full sm:basis-auto">{action}</div>}
      {link && (
        <span data-forge-link className="text-meta">
          <OutLink href={prLink!} className="min-h-[44px] sm:min-h-0">{`Open on ${name}`}</OutLink>
        </span>
      )}
      <span role="status" aria-live="polite" className="sr-only">
        {state.kind === 'posted'
          ? postedLine(state.at, state.login, state.comments)
          : state.kind === 'failed'
            ? `Couldn’t post: ${state.message}`
            : state.kind === 'stale'
              ? STALE_LINE
              : state.kind === 'posting'
                ? 'Posting…'
                : ''}
      </span>
    </footer>
  );
};

/**
 * Cards whose LGTM stamp has already been drawn in this page. §6: the 180 ms stamp-in plays once, the first time
 * a card renders ready, and never again (not on re-render, not on a remount after a poll or a tab switch).
 */
const STAMPED = new Set<string>();

/** Style v2 §6 LGTM stamp: ink, not foil. Drawn only when lgtm(card) holds; static under reduced motion (CSS). */
const Stamp: React.FC<{ id: string; reviewer: string }> = ({ id, reviewer }) => {
  const fresh = useRef(!STAMPED.has(id));
  useEffect(() => {
    STAMPED.add(id);
  }, [id]);
  return (
    <div data-lgtm className="mb-2.5 sm:absolute sm:right-4 sm:top-3.5 sm:mb-0">
      <span
        className="dv-stamp"
        role="img"
        aria-label={lgtmLabel(reviewer)}
        data-stamp-in={fresh.current ? '' : undefined}
        style={fresh.current ? undefined : { animation: 'none' }}
      >
        <b aria-hidden="true">LGTM</b>
        <small aria-hidden="true">{reviewer} · 0 to fix</small>
      </span>
    </div>
  );
};

/** One PR in review: risk, intent, tests, security and findings (design §5, parts 1–7). */
export const ReviewCard: React.FC<ReviewCardProps> = ({ card: given, onFinding, sample = false, now: fixedNow, onPost, forge = null, forgeLogin = null, repoLabel, jiraOrigins = [], lineNotes, onOpenChat }) => {
  const [card, setCard] = useState<Review>(given);
  const [busy, setBusy] = useState<string | null>(null);
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => setCard(given), [given]);
  const now = fixedNow ?? tick;
  // 0.9.0 V1: a PR that belongs to a kwant task in Review is decided in the Inbox; its button goes first.
  useInbox();
  const inboxItem = inboxNav.go && !sample ? reviewForPr(given.links.pr, given.pr) : null;

  // Tick once a second only while an Undo is on screen.
  const undoing =
    fixedNow === undefined &&
    (card.findings.some((f) => undoLeft(f, tick) > 0) || (card.matches?.items ?? []).some((it) => { const f = itemFinding(it); return !!f && undoLeft(f, tick) > 0; }));
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
      setCard((c) => ({
        ...c,
        findings: c.findings.map((x) => (x.id === f.id ? { ...x, send_error: msg } : x)),
        // A matches item's Fix it (api.md 2.11) keeps its error on the item's own fix.
        matches: c.matches && {
          ...c.matches,
          items: c.matches.items.map((it) => (it.id === f.id && it.fix ? { ...it, fix: { ...it.fix, send_error: msg } } : it)),
        },
      }));
    } finally {
      setBusy(null);
    }
  };

  const [posting, setPosting] = useState(false);
  // F4: the server answers 409 when the card has no review; that press is final, no Try again.
  const [noReview, setNoReview] = useState(false);
  const post = async () => {
    if (!onPost || posting || noReview || card.posted || card.no_forge || laterForge(card)) return;
    setPosting(true);
    try {
      setCard(await onPost(card.id));
    } catch (err) {
      // The route answers 200 with post_error for the forge's refusals; a throw is the network or a 4xx from kwant.
      const message = err instanceof Error && err.message ? err.message : 'it didn’t go through';
      if ((err as { status?: unknown } | null)?.status === 409) setNoReview(true);
      setCard((c) => ({ ...c, post_error: { kind: 'failed', message } }));
    } finally {
      setPosting(false);
    }
  };

  const author = personText(card.author);
  const authorModel = card.author ? modelLabel(card.author.engine, card.author.model) : null;
  const reviewerModel = card.reviewer ? modelLabel(card.reviewer.engine, card.reviewer.model) : null;
  const headingId = `review-${card.id || card.pr || card.branch}-h`;
  // 0.7.4: the card's own forge names its MR or PR and the Open link; a branch card (G5) has neither.
  const named = cardForge(card, forge);
  const branchCard = card.no_forge || card.pr === null;
  const tile = card.draft ? 'bg-zinc-500/15 text-zinc-300' : 'bg-teal-400/15 text-teal-300';
  // 0.7.6 J12: a PR that maps to a ticket's brief shows "Matches the ticket" (its own `matches` block,
  // api-decisions 17) instead of the intent section. Its items carry their own `k{n}` id and Fix it state
  // (api.md 2.11); a finding with one of those ids is never listed twice in Findings.
  const ticketMatches = card.matches;
  const onRows = matchFindingIds(ticketMatches);
  const listed = onRows.size ? card.findings.filter((f) => !onRows.has(f.id)) : card.findings;
  const summary = findingsSummary(listed);
  const sec = card.security as Review['security'] & { osv?: unknown };
  const stamp = lgtm(card) && card.reviewer ? card.reviewer.name : null;
  const repoChip = repoLabel ? repoLabel(card.repo, card.repo_name) : null;

  return (
    <article data-dev-card="review" data-repo={card.repo ?? undefined} data-pr={card.pr ?? undefined} data-forge={named ?? undefined} data-no-forge={branchCard || undefined} aria-labelledby={headingId} className="kw-card relative min-w-0 scroll-mt-3 p-4">
      {stamp && <Stamp id={card.id || `pr-${card.pr}`} reviewer={stamp} />}
      {/* 1. Header */}
      <header className={`flex items-start gap-3 ${stamp ? 'sm:pr-32' : ''}`}>
        <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-xl ${tile}`} aria-hidden="true">
          {branchCard ? <GitBranch size={16} /> : <GitPullRequest size={16} />}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <h3 id={headingId} className="min-w-0 rounded-sm text-title font-semibold text-zinc-50 [overflow-wrap:anywhere] focus:outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400">
              {branchCard ? (
                <>
                  <span className="sr-only">Branch </span>
                  <span className="font-mono text-[0.92em] text-zinc-300">{card.branch}</span>
                  <span aria-hidden="true" className="text-zinc-500">
                    {' · '}
                  </span>
                  <span className="sr-only">: </span>
                  {card.title}
                </>
              ) : (
                named === 'gitlab' ? (
                  <>
                    <span className="sr-only">{prName(named, card.pr!)} </span>
                    <span aria-hidden="true">{prRef(named, card.pr!)}</span> {card.title}
                  </>
                ) : (
                  <>
                    {prRef(named, card.pr!)} {card.title}
                  </>
                )
              )}
            </h3>
            {card.draft && <span className="kw-chip">Draft</span>}
            {branchCard && <span className="kw-chip">Branch</span>}
            {sample && <span className="kw-chip kw-chip-info">Sample PR</span>}
            {repoChip && (
              <span data-dev-card-repo className="kw-chip font-mono [overflow-wrap:anywhere]" title={repoChip}>
                <span className="sr-only">Repo {repoChip}</span>
                <span aria-hidden="true">{repoChip.replace(/\/+$/, '').split('/').pop() || repoChip}</span>
              </span>
            )}
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-meta text-zinc-400">
            {card.branch && (
              <code data-branch className="dv-branch">
                <GitBranch size={11} className="i" aria-hidden="true" />
                <span className="h">{card.branch}</span>
                {card.base && (
                  <>
                    <span className="arr" aria-hidden="true">
                      →
                    </span>
                    <span className="sr-only"> into </span>
                    <span className="shrink-0 text-zinc-300">{card.base}</span>
                  </>
                )}
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
          <div className="relative mt-1.5 flex flex-wrap gap-1.5">
            {card.reviewer && (card.same_engine ? <span className="kw-chip kw-chip-warn">Same engine as the author</span> : <span className="kw-chip kw-chip-accent">Different engine from the author</span>)}
            {/* 0.8.0 B3: how the review was run, right after the engine chip (Mira b3 §1). */}
            <FreshEyesChip label={card.fresh_eyes} reviewer={card.reviewer?.name ?? null} author={author?.name ?? null} specRef={ticketMatches?.key ?? null} />
            {!card.reviewer && <span className="kw-chip kw-chip-warn">{card.note ?? 'No reviewer yet'}</span>}
          </div>
        </div>
      </header>

      {/* 0.9.0 G1: the diff, with line notes for the author bot (g1-line-notes.md §1). */}
      {lineNotes && card.changes && (
        <ChangesSection
          cardId={card.id}
          api={lineNotes}
          stat={card.changes}
          notesCount={card.line_notes}
          headSha={card.head_sha}
          prUrl={card.links.pr ?? null}
          onOpenChat={onOpenChat}
        />
      )}

      {card.status === 'pending' && (
        <div data-reading>
          <p role="status" className="mt-3 flex items-center gap-2.5 rounded-[10px] border border-hairline bg-[var(--dv-code-bg)] px-3 py-2.5 font-mono text-[12.5px] leading-[18px] text-zinc-300 [overflow-wrap:anywhere]">
            <Led s="run" />
            <span className="min-w-0">
              {readingLine(card.reviewer?.name)}
              <span className="dv-caret" aria-hidden="true" />
            </span>
          </p>
          {/* Static skeleton: no shimmer, no ETA. */}
          <div aria-hidden="true" className="mt-3 grid gap-2">
            <div className="h-2.5 w-[62%] rounded-full bg-[var(--kw-hairline-strong)]" />
            <div className="h-2.5 w-[44%] rounded-full bg-[var(--kw-hairline-strong)]" />
          </div>
        </div>
      )}
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

      {/* 3. Intent, or Matches the ticket (0.7.6) */}
      {ticketMatches && (
        <TicketMatches matches={ticketMatches} now={now} origins={jiraOrigins} lead={author?.name ?? null} busy={busy !== null} act={act} />
      )}
      {card.intent && !ticketMatches && (
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
          <SecTile name="Secrets" tool="Gitleaks" mono="gitleaks" state={sec.secrets.state} text={scanText(sec.secrets, 'secret', 'secrets', 'None found')} />
          <SecTile name="New packages" tool="Registry check" mono="registry" state={sec.packages.state} text={scanText(sec.packages, 'package', 'packages', 'All real')} />
          <SecTile name="Dependencies" tool="OSV" mono="osv" state="later" text="Not checked in this version" />
          <SecTile name="Code patterns" tool="Semgrep" mono="semgrep" {...semgrepTile(sec.semgrep)} />
        </ul>
      </section>

      {/* 6. Findings */}
      <section className="mt-4" aria-label="Findings">
        <SectionHead extra={summary ? <span className="normal-case tracking-normal text-zinc-500">· {summary}</span> : null}>Findings</SectionHead>
        {listed.length === 0 ? (
          <p className="text-meta text-zinc-400">{card.status === 'ready' ? 'No findings.' : 'No findings yet.'}</p>
        ) : (
          <ul role="list" className="space-y-1.5">
            {listed.map((f) => (
              <FindingRow key={f.id} f={f} author={author?.name ?? null} now={now} busy={busy !== null} act={act} />
            ))}
          </ul>
        )}
      </section>

      {card.cut && <p className="mt-3 text-2xs text-zinc-400">{card.cut}</p>}

      {inboxItem && (
        <div className="mt-4 flex flex-wrap items-center gap-2" data-review-in-inbox={inboxItem.task_id}>
          <button
            type="button"
            onClick={() => openReview(inboxItem.task_id, 'dev')}
            className="kw-focus inline-flex items-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-3 rounded-lg text-meta font-semibold bg-violet-500/80 text-white hover:bg-violet-500"
          >
            <ShieldCheck size={13} aria-hidden="true" /> Review in Inbox
          </button>
          {verdictShort(inboxItem.verdict) && <span className="text-2xs text-zinc-400">{verdictShort(inboxItem.verdict)}</span>}
        </div>
      )}
      {/* 7. Footer: Post to {Forge} (design §5.7) */}
      {branchCard ? (
        <footer data-review-footer data-post-state="no_forge" className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-hairline pt-3 text-[12.5px] text-zinc-400">
          <span data-no-forge-line className="inline-flex min-w-0 flex-1 items-start gap-[7px]">
            <Led s="off" className="mt-[5px] shrink-0" />
            <span className="min-w-0 [overflow-wrap:anywhere]">{NO_FORGE_LINE}</span>
          </span>
          {card.links.commit && (
            <span className="text-meta">
              <OutLink href={card.links.commit} className="min-h-[44px] sm:min-h-0">
                Open commit
              </OutLink>
            </span>
          )}
        </footer>
      ) : (
      <PostFooter
        id={headingId}
        state={postState(card, { canPost: !!onPost && !sample, posting, forge: postForge(card, forge), noReview, later: laterForge(card) })}
        forge={card.posted?.forge ?? (postForge(card, forge) ? named : null)}
        count={postCount(card.findings)}
        login={forgeLogin}
        prLink={card.links.pr}
        onPost={() => void post()}
      />
      )}
    </article>
  );
};
