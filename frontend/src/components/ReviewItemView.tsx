// 0.9.0 V1: one item view for a review, used by the Inbox pane, the phone page, the Mission board and the
// Dev desk (Mira, release-090/designs/v1-review-inbox.md §3-§9 and "ENG (Iris)"; Ada's D2, D3, D5).
// Verdict and buttons first; Couldn't check open when it has anything; Decisions, What done looks like and
// Evidence folded. Approve never runs a held action (D3): the held action keeps its own Deny / Allow once,
// and Approve waits for it. Data from the server is shown as text only, never as HTML.

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronLeft,
  CornerUpLeft,
  EyeOff,
  ExternalLink,
  Eye,
  FolderOpen,
  GitBranch,
  GitPullRequest,
  Hand,
  ListChecks,
  Loader2,
  MessageSquare,
  MinusCircle,
  RotateCcw,
  ShieldCheck,
  SquareKanban,
  Terminal,
  X,
  XCircle,
  type LucideIcon,
} from 'lucide-react';
import { type InboxItem, type ReviewItem, type HeldAction, type InboxCriterion, InboxError, approveState, inboxApi } from '../api/inbox';
import { AwayError } from '../away';
import { type Checkpoint, type RestorePreview, MissionError, RESTORE_ACTION_LABELS, conflictText, missionsApi, newIdempotencyKey, restoreErrorText } from '../missions';
import { usePhoneLayout } from '../usePhoneLayout';
import { ImageViewer } from './Attachments';
import { ModernAvatar } from './ModernAvatar';
import {
  CRITERION_WORD,
  GATE_TEXT,
  NOT_UNDONE_UNKNOWN,
  RISK_WORD,
  type ReviewDone,
  type ReviewEntry,
  changePrefill,
  checkerMeta,
  clock,
  confidenceText,
  criteriaSummaryText,
  decisionsSummaryText,
  evidenceSummaryText,
  lastRound,
  nextCriterionIndex,
  notUndoneTitle,
  rollBackLabel,
  rolledBackForkText,
  rolledBackText,
  sendBackSteps,
  stepDownText,
  tailLines,
  validReviewItem,
  verdictWord,
} from '../reviewItem';

export type { ReviewDone, ReviewEntry } from '../reviewItem';

/** The calls the view makes; tests and screenshots pass a fake. */
export interface ReviewItemApi {
  verify: (taskId: string, verdictId?: string | null) => Promise<unknown>;
  reject: (taskId: string, reason: string) => Promise<unknown>;
  runChecks: (taskId: string) => Promise<unknown>;
  resolveHeld: (approvalId: string, decision: 'allow_once' | 'deny') => Promise<unknown>;
  checkpoints: (taskId: string) => Promise<Checkpoint[]>;
  restorePreview: (taskId: string, checkpointId: string) => Promise<RestorePreview>;
  restore: typeof missionsApi.restore;
  forkFromCheckpoint: typeof missionsApi.forkFromCheckpoint;
}

export const defaultReviewApi: ReviewItemApi = {
  verify: inboxApi.verify,
  reject: inboxApi.reject,
  runChecks: inboxApi.runChecks,
  resolveHeld: inboxApi.resolveHeld,
  checkpoints: missionsApi.checkpoints,
  restorePreview: missionsApi.restorePreview,
  restore: missionsApi.restore,
  forkFromCheckpoint: missionsApi.forkFromCheckpoint,
};

export interface ReviewItemViewProps {
  item: ReviewItem | InboxItem;
  entry: ReviewEntry;
  onDone: (result: ReviewDone) => void;
  api?: ReviewItemApi;
  /** "‹ Back to Mission board" / "‹ Back to Dev desk" above the header (desktop), when given. */
  onBack?: () => void;
  onOpenChat?: () => void;
  onOpenBoard?: () => void;
  /** After a roll back after approval: open the copy. */
  onOpenTask?: (taskId: string) => void;
  /** The phone page draws its own top bar with the title; the view then starts at the chips. */
  hideTitle?: boolean;
}

const BACK_LABEL: Record<ReviewEntry, string | null> = { inbox: null, board: 'Back to Mission board', dev: 'Back to Dev desk' };

const btnBase =
  'inline-flex items-center justify-center gap-1.5 rounded-xl px-3.5 min-h-[36px] max-md:min-h-[44px] text-meta font-semibold transition-colors kw-focus disabled:cursor-not-allowed';
const btnApprove = `${btnBase} bg-accent-gradient text-white aria-disabled:opacity-[.42] aria-disabled:cursor-not-allowed`;
const btnQuiet = `${btnBase} border border-hairline-strong bg-white/[0.03] text-zinc-200 hover:bg-white/[0.08] disabled:opacity-50`;
const btnRose = `${btnBase} border border-rose-400/40 bg-rose-500/[0.06] text-rose-200 hover:bg-rose-500/[0.12] disabled:opacity-50`;
const btnRoseFill = `${btnBase} bg-rose-600 text-white hover:bg-rose-500 disabled:opacity-50`;
const btnAllow = `${btnBase} bg-[#059669] text-white hover:bg-emerald-500 disabled:opacity-50`;
const spin = 'motion-safe:animate-spin';

// ---- Folds: open or closed per section for the session (Mira §3.5), not per item ----

type FoldKey = 'decisions' | 'criteria' | 'evidence';
const FOLD_KEY = 'kw-review-folds';
function readFolds(): Record<FoldKey, boolean> {
  try {
    const v = JSON.parse(sessionStorage.getItem(FOLD_KEY) ?? '{}');
    return { decisions: v.decisions === true, criteria: v.criteria === true, evidence: v.evidence === true };
  } catch {
    return { decisions: false, criteria: false, evidence: false };
  }
}
function writeFolds(f: Record<FoldKey, boolean>) {
  try {
    sessionStorage.setItem(FOLD_KEY, JSON.stringify(f));
  } catch {
    // private mode: the fold just forgets
  }
}

type HeldState = { state: 'open' | 'allowed' | 'denied' | 'expired'; at?: string | null; busy?: boolean; error?: string | null; ran?: string | null };
const HELD_STATE = { pending: 'open', allowed: 'allowed', denied: 'denied', expired: 'expired' } as const;
/** The server's own status for a held action; resolved ones stay listed with their result line. */
const heldFrom = (h: HeldAction): HeldState => ({ state: HELD_STATE[h.status] ?? 'open', at: h.resolved_at, ran: h.result_line });

const typingIn = (t: EventTarget | null) => {
  const el = t as HTMLElement | null;
  if (!el || !el.tagName) return false;
  return el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable;
};

/** The plain card for an item the view can't draw (not a review, or a field missing): title and summary as text. */
const FallbackCard: React.FC<{ item: InboxItem }> = ({ item }) => (
  <section role="region" aria-label={item.title || 'Inbox item'} className="kw-card p-4 space-y-1" data-review-fallback>
    <h2 className="text-[16px] font-semibold text-zinc-100 break-words">{item.title}</h2>
    {item.summary && <p className="text-body text-zinc-300 whitespace-pre-wrap break-words">{item.summary}</p>}
  </section>
);

export const ReviewItemView: React.FC<ReviewItemViewProps> = (props) => {
  if (!validReviewItem(props.item)) return <FallbackCard item={props.item} />;
  // A new item starts fresh: no leftover panel, notice or held state from the last one.
  return <ReviewBody key={props.item.id} {...props} item={props.item} />;
};

export default ReviewItemView;

const ReviewBody: React.FC<ReviewItemViewProps & { item: ReviewItem }> = ({
  item,
  entry,
  onDone,
  api = defaultReviewApi,
  onBack,
  onOpenChat,
  onOpenBoard,
  onOpenTask,
  hideTitle = false,
}) => {
  const phone = usePhoneLayout();
  const uid = useId();
  const titleId = `${uid}-title`;
  const [held, setHeld] = useState<Record<string, HeldState>>(() => Object.fromEntries(item.held_actions.map((h) => [h.approval_id, heldFrom(h)])));
  const openHeld = item.held_actions.filter((h) => held[h.approval_id]?.state === 'open');
  const blocked = openHeld.length > 0 ? approveState({ held_actions: openHeld }).blocked : null;
  const [busy, setBusy] = useState<null | 'approve' | 'send' | 'check'>(null);
  const [notice, setNotice] = useState<{ text: string; tone: 'ok' | 'bad' | 'info'; undo?: string | null; copy?: string | null } | null>(null);
  const [done, setDone] = useState(false);
  const [sendOpen, setSendOpen] = useState(false);
  const [note, setNote] = useState('');
  const [sendErr, setSendErr] = useState<string | null>(null);
  const [rollOpen, setRollOpen] = useState<null | { undoFrom?: string }>(null);
  const [folds, setFolds] = useState(readFolds);
  const firstDenyRef = useRef<HTMLButtonElement>(null);
  const sendBtnRef = useRef<HTMLButtonElement>(null);
  const rollBtnRef = useRef<HTMLButtonElement>(null);
  const approveRef = useRef<HTMLButtonElement>(null);
  const inFlight = useRef(false);

  const toggleFold = (k: FoldKey) =>
    setFolds((f) => {
      const n = { ...f, [k]: !f[k] };
      writeFolds(n);
      return n;
    });

  const finish = useCallback(
    (r: ReviewDone, tone: 'ok' | 'bad' | 'info', extra: { undo?: string | null; copy?: string | null } = {}) => {
      setDone(r.kind !== 'stale');
      setNotice({ text: r.text, tone, ...extra });
      onDone(r);
    },
    [onDone],
  );

  // ---- Held actions: today's resolve call, one at a time ----
  const resolveHeld = async (h: HeldAction, decision: 'allow_once' | 'deny'): Promise<boolean> => {
    const id = h.approval_id;
    setHeld((s) => ({ ...s, [id]: { ...s[id], busy: true, error: null } }));
    try {
      const r = await api.resolveHeld(id, decision);
      const o = r && typeof r === 'object' ? (r as Record<string, unknown>) : {};
      const ran = typeof o.result_line === 'string' ? o.result_line : typeof o.result === 'string' ? o.result : null;
      setHeld((s) => ({ ...s, [id]: { state: decision === 'deny' ? 'denied' : 'allowed', at: new Date().toISOString(), ran } }));
      return true;
    } catch (e) {
      const status = e instanceof AwayError || e instanceof InboxError ? e.status : 0;
      if (status === 404 || status === 409) {
        setHeld((s) => ({ ...s, [id]: { state: 'expired' } }));
        return true;
      }
      setHeld((s) => ({ ...s, [id]: { ...s[id], busy: false, error: e instanceof Error ? e.message : 'Something went wrong.' } }));
      return false;
    }
  };
  /** Send back and Roll back deny every open held action first; one failing stops the whole step. */
  const denyAllOpen = async (): Promise<string | null> => {
    for (const h of openHeld) {
      const ok = await resolveHeld(h, 'deny');
      if (!ok) return `Couldn’t deny the held action, so nothing was sent. Try again.`;
    }
    return null;
  };

  // ---- Approve ----
  const approve = async () => {
    if (inFlight.current || done) return;
    if (blocked) {
      firstDenyRef.current?.focus();
      return;
    }
    inFlight.current = true;
    setBusy('approve');
    setNotice(null);
    try {
      await api.verify(item.task_id, item.verdict.id);
      finish({ kind: 'approved', text: `Approved: “${item.title}”.`, task_id: item.task_id }, 'ok');
    } catch (e) {
      if (e instanceof InboxError && e.stale) {
        const what = e.paths.length ? `${e.paths.join(', ')} changed after its check` : 'The work changed after its check';
        finish({ kind: 'stale', text: `Couldn’t approve: ${item.title}. ${what}. Nothing was approved.`, task_id: item.task_id, paths: e.paths }, 'bad');
      } else {
        const why = e instanceof Error ? e.message.replace(/\.$/, '') : 'Something went wrong';
        setNotice({ text: `Couldn’t approve “${item.title}”. ${why}. Nothing was approved.`, tone: 'bad' });
      }
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  };

  // ---- Check again (stale) ----
  const checkAgain = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy('check');
    setNotice(null);
    try {
      await api.runChecks(item.task_id);
      setNotice({ text: 'Checking again. When the checks pass it goes back to To review.', tone: 'info' });
    } catch (e) {
      setNotice({ text: `Couldn’t run the checks. ${e instanceof Error ? e.message : ''}`.trim(), tone: 'bad' });
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  };

  // ---- Send back ----
  const openSend = (prefill?: string) => {
    if (done) return;
    if (prefill !== undefined) setNote(prefill);
    setSendErr(null);
    setSendOpen(true);
  };
  const closeSend = () => {
    setSendOpen(false);
    setSendErr(null);
    requestAnimationFrame(() => sendBtnRef.current?.focus());
  };
  const sendBack = async () => {
    const reason = note.trim();
    if (!reason || inFlight.current) return;
    inFlight.current = true;
    setBusy('send');
    setSendErr(null);
    try {
      const err = await denyAllOpen();
      if (err) {
        setSendErr(err);
        return;
      }
      await api.reject(item.task_id, reason);
      setSendOpen(false);
      finish({ kind: 'sent_back', text: `Sent back to ${item.kwant_name}. Round ${item.round.n + 1} starts now.`, task_id: item.task_id }, 'ok');
    } catch (e) {
      setSendErr(`Couldn’t send it back. ${e instanceof Error ? e.message : ''}`.trim());
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  };

  // ---- Keyboard: A approves (never a held action), S opens Send back, Esc closes a panel ----
  const keyState = useRef({ approve, openSend, sendOpen, rollOpen, done });
  keyState.current = { approve, openSend, sendOpen, rollOpen, done };
  useEffect(() => {
    if (phone) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || typingIn(e.target)) return;
      const s = keyState.current;
      if (s.rollOpen) return; // the dialog owns the keys
      if (e.key === 'Escape' && s.sendOpen) {
        e.preventDefault();
        closeSend();
        return;
      }
      if (s.done || s.sendOpen) return;
      if (e.key === 'a' || e.key === 'A') {
        e.preventDefault();
        void s.approve();
      } else if (e.key === 's' || e.key === 'S') {
        e.preventDefault();
        s.openSend();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [phone]);

  const v = item.verdict;
  const tone = VERDICT_TONE[v.result];
  const VIcon = tone.icon;
  const meta = checkerMeta(item);
  const conf = confidenceText(v);
  const since = clock(item.in_review_since);
  const back = BACK_LABEL[entry];
  const lockAll = done || busy !== null;
  const rollbackShown = item.rollback_ok;

  return (
    <section role="region" aria-labelledby={titleId} className="space-y-4 min-w-0" data-review-item={item.id} data-entry={entry}>
      {/* Header */}
      {back && onBack && !phone && (
        <button type="button" onClick={onBack} className="inline-flex items-center gap-1 text-meta text-zinc-400 hover:text-zinc-200 rounded kw-focus" data-review-back>
          <ChevronLeft size={14} aria-hidden="true" /> {back}
        </button>
      )}
      <header className="flex items-start gap-3 min-w-0">
        {!hideTitle && <ModernAvatar role={item.kwant_name} name={item.kwant_name} size={34} kwantId={item.kwant_id} />}
        <div className="min-w-0 flex-1 space-y-1">
          <h2 id={titleId} className={hideTitle ? 'sr-only' : 'text-[16px] leading-snug font-[650] text-zinc-50 break-words'}>
            {item.title}
          </h2>
          <div className="flex flex-wrap items-center gap-1.5 text-meta text-zinc-400">
            {item.round.n > 1 && (
              <span className="kw-chip border-violet-400/40 bg-violet-500/15 text-violet-200 !py-0" data-round-chip>
                Round {item.round.n}
              </span>
            )}
            {item.risk.level && (
              <span className={`kw-chip !py-0 ${item.risk.level === 'high' ? 'kw-chip-warn' : ''}`} data-risk={item.risk.level}>
                {RISK_WORD[item.risk.level]}
              </span>
            )}
            <span className="min-w-0 break-words">
              {[item.project, since ? `in Review since ${since}` : null, `round ${item.round.n} of ${item.round.max}`].filter(Boolean).join(' · ')}
            </span>
          </div>
        </div>
        {!phone && (onOpenChat || onOpenBoard) && (
          <div className="flex shrink-0 gap-2">
            {onOpenChat && (
              <button type="button" onClick={onOpenChat} className={btnQuiet}>
                <MessageSquare size={14} aria-hidden="true" /> Open chat
              </button>
            )}
            {onOpenBoard && (
              <button type="button" onClick={onOpenBoard} className={btnQuiet}>
                <SquareKanban size={14} aria-hidden="true" /> Mission board
              </button>
            )}
          </div>
        )}
      </header>

      {/* Verdict */}
      <div className={`rounded-2xl border px-4 py-3 space-y-1 ${tone.box}`} data-verdict={v.result}>
        <p className="text-[15px] font-semibold flex flex-wrap items-center gap-x-1.5">
          <VIcon size={17} className={tone.text} aria-hidden="true" />
          <span className={tone.text}>{verdictWord(v.result)}</span>
          {conf && <span className="font-normal text-zinc-400">· {conf}</span>}
        </p>
        <p className="text-body text-zinc-100 break-words">{v.line}</p>
        {meta && (
          <p className="flex items-start gap-1.5 text-meta text-zinc-400" data-checker-meta>
            <Eye size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
            <span className="min-w-0">{meta}</span>
          </p>
        )}
      </div>

      {/* Stale and approve_error, from the server */}
      {item.stale && (
        <div className="rounded-xl border border-rose-400/30 bg-rose-500/[0.06] px-3 py-2 space-y-2" data-stale>
          <p className="text-body text-rose-200 break-words">
            <AlertTriangle size={14} className="inline -mt-0.5 mr-1" aria-hidden="true" />
            Couldn’t approve: {item.title}. {item.stale.paths.length ? `${item.stale.paths.join(', ')} changed after its check.` : 'The work changed after its check.'}
          </p>
          <button type="button" onClick={() => void checkAgain()} disabled={lockAll} className={btnQuiet} data-check-again>
            {busy === 'check' ? <Loader2 size={14} className={spin} aria-hidden="true" /> : <RotateCcw size={14} aria-hidden="true" />} Check again
          </button>
        </div>
      )}
      {item.approve_error && (
        <p className="rounded-xl border border-rose-400/30 bg-rose-500/[0.06] px-3 py-2 text-body text-rose-200 break-words" data-approve-error={item.approve_error.code}>
          <AlertTriangle size={14} className="inline -mt-0.5 mr-1" aria-hidden="true" />
          {item.approve_error.message}
        </p>
      )}

      {/* Actions: held, then Approve · Send back · Roll back */}
      <div className="kw-card p-3 sm:p-4 space-y-3" data-actions>
        {item.held_actions.length > 0 && (
          <ol className="space-y-2" aria-label="Held actions">
            {item.held_actions.map((h, i) => (
              <HeldBox
                key={h.approval_id}
                h={h}
                n={item.held_actions.length > 1 ? i + 1 : null}
                bot={item.kwant_name}
                st={held[h.approval_id] ?? heldFrom(h)}
                phone={phone}
                lock={done || busy === 'send'}
                denyRef={openHeld[0]?.approval_id === h.approval_id ? firstDenyRef : undefined}
                onResolve={(d) => void resolveHeld(h, d)}
              />
            ))}
          </ol>
        )}
        <div className={phone ? 'grid grid-cols-2 gap-2' : 'flex flex-wrap items-center gap-2'}>
          <button
            ref={approveRef}
            type="button"
            onClick={() => void approve()}
            aria-disabled={!!blocked || lockAll || undefined}
            aria-keyshortcuts={phone ? undefined : 'A'}
            aria-describedby={blocked ? `${uid}-blocked` : undefined}
            className={`${btnApprove} ${phone ? 'col-span-2' : ''}`}
            data-approve
          >
            {busy === 'approve' ? <Loader2 size={14} className={spin} aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
            {busy === 'approve' ? 'Approving…' : 'Approve'}
          </button>
          {phone && blocked && <BlockedLine id={`${uid}-blocked`} text={blocked} className="col-span-2" />}
          <button
            ref={sendBtnRef}
            type="button"
            onClick={() => openSend()}
            disabled={lockAll}
            aria-expanded={sendOpen}
            aria-keyshortcuts={phone ? undefined : 'S'}
            className={btnQuiet}
            data-send-back
          >
            <CornerUpLeft size={14} aria-hidden="true" /> Send back
          </button>
          {rollbackShown && (
            <button ref={rollBtnRef} type="button" onClick={() => setRollOpen({})} disabled={lockAll} aria-haspopup="dialog" className={btnRose} data-roll-back>
              <RotateCcw size={14} aria-hidden="true" /> Roll back
            </button>
          )}
          {!phone && lastRound(item.round) && item.round.n > 1 && (
            <span className="text-meta text-zinc-400 max-w-[14rem]" data-last-round>
              Last round. Sending back again asks you how to go on.
            </span>
          )}
          {!phone && (
            <span className="ml-auto hidden lg:flex items-center gap-1" aria-hidden="true">
              {['A', 'S', 'J', 'K'].map((k) => (
                <kbd key={k} className="rounded border border-hairline-strong px-1 text-[10px] leading-4 text-zinc-400">
                  {k}
                </kbd>
              ))}
            </span>
          )}
        </div>
        {!phone && blocked && <BlockedLine id={`${uid}-blocked`} text={blocked} />}
        {phone && lastRound(item.round) && item.round.n > 1 && <p className="text-meta text-zinc-400">Last round. Sending back again asks you how to go on.</p>}
      </div>

      {/* Status line: approved, sent back, rolled back, or an action error */}
      <div role="status" aria-live="polite" className="empty:hidden" data-review-status>
        {notice && (
          <p
            className={`rounded-xl border px-3 py-2 text-body break-words ${
              notice.tone === 'bad' ? 'border-rose-400/30 bg-rose-500/[0.06] text-rose-200' : notice.tone === 'ok' ? 'border-teal-400/25 bg-teal-400/[0.05] text-zinc-100' : 'border-hairline-strong bg-white/[0.03] text-zinc-200'
            }`}
            data-notice={notice.tone}
          >
            {notice.tone === 'bad' ? <AlertTriangle size={14} className="inline -mt-0.5 mr-1" aria-hidden="true" /> : <CheckCircle2 size={14} className="inline -mt-0.5 mr-1 text-ok" aria-hidden="true" />}
            {notice.text}
            {notice.undo && (
              <>
                {' '}
                <button type="button" onClick={() => setRollOpen({ undoFrom: notice.undo ?? undefined })} className="text-violet-300 underline-offset-2 hover:underline rounded kw-focus kw-hit" data-undo-rollback>
                  Undo the roll back
                </button>
              </>
            )}
            {notice.copy && onOpenTask && (
              <>
                {' '}
                <button type="button" onClick={() => onOpenTask(notice.copy!)} className="text-violet-300 underline-offset-2 hover:underline rounded kw-focus kw-hit" data-open-copy>
                  Open the copy
                </button>
              </>
            )}
          </p>
        )}
      </div>

      {/* Send back: inline panel on desktop, sheet on phone */}
      {sendOpen &&
        (phone ? (
          <Sheet title={`Send back to ${item.kwant_name}`} onClose={closeSend}>
            <SendBackForm item={item} note={note} setNote={setNote} err={sendErr} busy={busy === 'send'} openHeld={openHeld.length} onSend={() => void sendBack()} onCancel={closeSend} phone />
          </Sheet>
        ) : (
          <div className="rounded-2xl border border-violet-400/40 bg-surface-raised p-4 space-y-3 kw-fade-in" data-send-panel>
            <h3 className="flex items-center gap-2 text-body font-semibold text-zinc-100">
              <CornerUpLeft size={15} aria-hidden="true" /> Send back to {item.kwant_name}
            </h3>
            <SendBackForm item={item} note={note} setNote={setNote} err={sendErr} busy={busy === 'send'} openHeld={openHeld.length} onSend={() => void sendBack()} onCancel={closeSend} />
          </div>
        ))}

      {/* Round 2: what changed since round 1 */}
      {item.previous_round && <RoundBox item={item} />}

      {/* Couldn't check: open whenever it isn't empty */}
      {item.couldnt_verify.length > 0 && (
        <section aria-labelledby={`${uid}-cc`} className="space-y-2" data-couldnt-check>
          <h3 id={`${uid}-cc`} className="flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wide text-zinc-300">
            <EyeOff size={14} aria-hidden="true" /> Couldn’t check <span className="font-normal text-zinc-500">{item.couldnt_verify.length}</span>
          </h3>
          <ul className="rounded-xl border border-dashed border-hairline-strong px-4 py-2.5 space-y-1 list-disc pl-8 text-body">
            {item.couldnt_verify.map((c, i) => (
              <li key={i} className="break-words text-zinc-100">
                {c.what}
                {c.what && !/[.!?]$/.test(c.what) ? '.' : ''} <span className="text-zinc-400">{c.why}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Folds */}
      <div className="space-y-2">
        {item.decisions.total > 0 && (
          <Fold id={`${uid}-dec`} icon={GitBranch} label="Decisions" summary={decisionsSummaryText(item.decisions)} open={folds.decisions} onToggle={() => toggleFold('decisions')}>
            <ul className="space-y-2">
              {item.decisions.top.map((d) => (
                <li key={d.index} className="rounded-xl border border-hairline bg-white/[0.02] px-3 py-2 flex items-start gap-3" data-decision={d.index}>
                  <div className="min-w-0 flex-1 text-body">
                    <p className="font-semibold text-zinc-100 break-words">{d.question}</p>
                    <p className="text-zinc-300 break-words">
                      Picked: <span className="font-semibold text-zinc-100">{d.picked}</span>
                    </p>
                    {d.why && <p className="text-meta text-zinc-400 break-words">{d.why}</p>}
                  </div>
                  <button type="button" onClick={() => openSend(changePrefill(d.question))} disabled={lockAll} className={btnQuiet} data-decision-change>
                    Change
                  </button>
                </li>
              ))}
            </ul>
          </Fold>
        )}
        <Fold
          id={`${uid}-crit`}
          icon={ListChecks}
          label="What done looks like"
          summary={criteriaSummaryText(item.criteria_summary)}
          summaryBad={item.criteria_summary.not_met > 0}
          open={folds.criteria}
          onToggle={() => toggleFold('criteria')}
        >
          {item.criteria.length ? (
            <ul className="rounded-xl border border-hairline divide-y divide-hairline">
              {item.criteria.map((c) => (
                <CriterionRow key={c.index} c={c} />
              ))}
            </ul>
          ) : (
            <p className="text-meta text-zinc-400">No criteria were set for this task.</p>
          )}
        </Fold>
        <Fold id={`${uid}-ev`} icon={Terminal} label="Evidence" summary={evidenceSummaryText(item.evidence)} open={folds.evidence} onToggle={() => toggleFold('evidence')}>
          <Evidence item={item} />
        </Fold>
      </div>

      {rollOpen && (
        <RollBackDialog
          item={item}
          api={api}
          phone={phone}
          undoFrom={rollOpen.undoFrom}
          openHeld={openHeld.length}
          denyAllOpen={denyAllOpen}
          onClose={() => {
            setRollOpen(null);
            requestAnimationFrame(() => rollBtnRef.current?.focus());
          }}
          onDone={(r, extra) => {
            setRollOpen(null);
            finish(r, 'ok', extra);
          }}
        />
      )}
    </section>
  );
};

const VERDICT_TONE = {
  pass: { icon: CheckCircle2, text: 'text-ok', box: 'border-teal-400/25 bg-teal-400/[0.05]' },
  fail: { icon: XCircle, text: 'text-bad', box: 'border-rose-400/30 bg-rose-500/[0.06]' },
  needs_you: { icon: Hand, text: 'text-warn', box: 'border-amber-500/[0.32] bg-amber-950/[0.24]' },
  none: { icon: MinusCircle, text: 'text-zinc-400', box: 'border-hairline-strong bg-white/[0.02]' },
} as const;

const BlockedLine: React.FC<{ id: string; text: string; className?: string }> = ({ id, text, className = '' }) => (
  <p id={id} className={`flex items-center gap-1.5 text-[11.5px] text-warn ${className}`} data-approve-blocked>
    <Hand size={13} aria-hidden="true" /> {text}
  </p>
);

const HeldBox: React.FC<{
  h: HeldAction;
  n: number | null;
  bot: string;
  st: HeldState;
  phone: boolean;
  lock: boolean;
  denyRef?: React.Ref<HTMLButtonElement>;
  onResolve: (d: 'allow_once' | 'deny') => void;
}> = ({ h, n, bot, st, phone, lock, denyRef, onResolve }) => {
  const at = clock(st.at);
  const box =
    st.state === 'allowed'
      ? 'border-teal-400/30 bg-teal-400/[0.05]'
      : st.state === 'open'
        ? 'border-amber-500/[0.32] bg-amber-950/[0.24]'
        : 'border-hairline-strong bg-white/[0.02]';
  const head =
    st.state === 'allowed' ? (
      <span className="text-ok">
        <Check size={13} className="inline -mt-0.5 mr-1" aria-hidden="true" />
        You allowed this{at ? ` at ${at}` : ''}. It ran:
      </span>
    ) : st.state === 'denied' ? (
      <span className="text-zinc-300">You denied this{at ? ` at ${at}` : ''}. It didn’t run.</span>
    ) : st.state === 'expired' ? (
      <span className="text-zinc-300">{st.ran || `This request expired. ${bot} will ask again if it still needs it.`}</span>
    ) : (
      <span className="text-warn">
        <Hand size={13} className="inline -mt-0.5 mr-1" aria-hidden="true" />
        Held: {bot} wants to run this, exactly:
      </span>
    );
  return (
    <li className={`rounded-xl border px-3 py-2.5 space-y-2 ${box}`} data-held={h.approval_id} data-held-state={st.state}>
      <div className="flex items-start justify-between gap-3 text-meta font-semibold">
        <p className="min-w-0">
          {n !== null && <span className="text-zinc-400 mr-1">{n}.</span>}
          {head}
        </p>
        {!phone && <span className="shrink-0 font-normal text-zinc-400">{GATE_TEXT[h.gate]}</span>}
      </div>
      {st.state !== 'expired' && (
        <pre className="font-mono text-[12.5px] leading-5 text-zinc-100 whitespace-pre-wrap break-words" data-held-args>
          {h.args_display}
        </pre>
      )}
      {st.state === 'allowed' && st.ran && <p className="text-meta text-zinc-300">{st.ran}</p>}
      {st.error && <p className="text-meta text-rose-200">{st.error}</p>}
      {st.state === 'open' && (
        <div className={phone ? 'grid grid-cols-2 gap-2' : 'flex gap-2'}>
          <button ref={denyRef} type="button" onClick={() => onResolve('deny')} disabled={st.busy || lock} className={btnQuiet} data-held-deny>
            <X size={14} aria-hidden="true" /> Deny
          </button>
          <button type="button" onClick={() => onResolve('allow_once')} disabled={st.busy || lock} className={btnAllow} data-held-allow>
            <Check size={14} aria-hidden="true" /> Allow once
          </button>
        </div>
      )}
    </li>
  );
};

const SendBackForm: React.FC<{
  item: ReviewItem;
  note: string;
  setNote: (s: string) => void;
  err: string | null;
  busy: boolean;
  openHeld: number;
  onSend: () => void;
  onCancel: () => void;
  phone?: boolean;
}> = ({ item, note, setNote, err, busy, openHeld, onSend, onCancel, phone = false }) => {
  const ta = useRef<HTMLTextAreaElement>(null);
  const id = useId();
  useEffect(() => {
    const el = ta.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  const empty = !note.trim();
  return (
    <div className="space-y-3" data-send-form>
      <label htmlFor={`${id}-note`} className="block text-meta text-zinc-300">
        What still needs doing?
      </label>
      <textarea
        id={`${id}-note`}
        ref={ta}
        value={note}
        onChange={(e) => setNote(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            if (!empty && !busy) onSend();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        rows={3}
        className="block w-full min-h-[76px] rounded-xl border border-violet-400/40 bg-surface-main px-3 py-2 text-body text-zinc-100 placeholder:text-zinc-500 kw-focus"
        data-send-note
      />
      {!empty && (
        <div className="flex flex-col sm:flex-row sm:items-start gap-1.5 sm:gap-3" data-becomes>
          <span className="shrink-0 text-meta text-zinc-400 sm:pt-2">Becomes criterion {nextCriterionIndex(item.criteria)}:</span>
          <p className="min-w-0 flex-1 rounded-xl border border-dashed border-violet-400/40 px-3 py-2 text-body text-zinc-100 break-words whitespace-pre-wrap">
            {note.trim()}{' '}
            <span className="kw-chip border-violet-400/40 bg-violet-500/15 text-violet-200 !py-0 align-middle">Your note</span>
          </p>
        </div>
      )}
      <ul className="list-disc pl-5 space-y-0.5 text-meta text-zinc-400">
        {sendBackSteps(item, openHeld).map((s) => (
          <li key={s}>{s}</li>
        ))}
      </ul>
      {err && (
        <p className="text-meta text-rose-200" role="alert">
          {err}
        </p>
      )}
      <div className={phone ? 'grid grid-cols-2 gap-2' : 'flex flex-wrap items-center gap-2'}>
        <button type="button" onClick={onSend} disabled={empty || busy} className={`${btnBase} bg-accent-gradient text-white disabled:opacity-50`} data-send-submit>
          {busy ? <Loader2 size={14} className={spin} aria-hidden="true" /> : <CornerUpLeft size={14} aria-hidden="true" />} Send back
        </button>
        <button type="button" onClick={onCancel} className={btnQuiet} data-send-cancel>
          Cancel
        </button>
        {!phone && (
          <span className="ml-auto text-[11px] text-zinc-500" aria-hidden="true">
            <kbd className="rounded border border-hairline-strong px-1">⌘ Enter</kbd> send · <kbd className="rounded border border-hairline-strong px-1">Esc</kbd> cancel
          </span>
        )}
      </div>
    </div>
  );
};

const RoundBox: React.FC<{ item: ReviewItem }> = ({ item }) => {
  const p = item.previous_round!;
  const at = clock(p.sent_back_at);
  const mine = [...item.criteria].reverse().find((c) => c.source === 'person_note' && c.round_added === item.round.n) ?? null;
  const word = (s: InboxCriterion['status']) => (
    <strong className={s === 'met' ? 'text-ok' : s === 'not_met' ? 'text-bad' : 'text-zinc-300'}>{CRITERION_WORD[s]}</strong>
  );
  return (
    <section className="rounded-2xl border border-violet-400/40 bg-surface-raised px-4 py-3 space-y-2" aria-label={`Round ${item.round.n}, what changed since round ${item.round.n - 1}`} data-round-box>
      <h3 className="flex items-center gap-1.5 text-body font-semibold text-zinc-100">
        <RotateCcw size={14} aria-hidden="true" /> Round {item.round.n} · what changed since round {item.round.n - 1}
      </h3>
      <p className="text-meta text-zinc-400">You sent it back{at ? ` at ${at}` : ''}:</p>
      <blockquote className="border-l-2 border-violet-400/60 bg-white/[0.03] px-3 py-2 text-body text-zinc-100 break-words whitespace-pre-wrap">{p.note}</blockquote>
      <ul className="list-disc pl-5 space-y-0.5 text-meta text-zinc-300">
        {mine && (
          <li>
            Your note is now criterion {mine.index}, and it’s {mine.status === 'met' ? word('met') : mine.status === 'not_met' ? word('not_met') : word(mine.status)}.
          </li>
        )}
        {p.flips.map((f) => (
          <li key={f.index}>
            Criterion {f.index} went {word(f.from)} → {word(f.to)}.
          </li>
        ))}
        {(p.diff.add > 0 || p.diff.del > 0 || p.diff.files > 0) && (
          <li>
            <span className="text-ok">+{p.diff.add}</span> <span className="text-bad">−{p.diff.del}</span> in {p.diff.files === 1 ? '1 file' : `${p.diff.files} files`} since round {item.round.n - 1}
            {item.evidence.pr && (
              <>
                {' · '}
                <a href={item.evidence.pr.url} target="_blank" rel="noopener noreferrer" className="text-violet-300 hover:underline rounded kw-focus">
                  see the changes
                </a>
              </>
            )}
          </li>
        )}
        {p.bot_summary && (
          <li className="break-words">
            {item.kwant_name}: {p.bot_summary}
          </li>
        )}
      </ul>
    </section>
  );
};

const Fold: React.FC<{
  id: string;
  icon: LucideIcon;
  label: string;
  summary: string;
  summaryBad?: boolean;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}> = ({ id, icon: Icon, label, summary, summaryBad = false, open, onToggle, children }) => (
  <div className="rounded-xl border border-hairline bg-surface-raised" data-fold={label}>
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      aria-controls={`${id}-body`}
      className="flex w-full items-center gap-2.5 min-h-[40px] max-md:min-h-[44px] px-3 text-left rounded-xl kw-focus"
    >
      <Icon size={15} className="shrink-0 text-zinc-300" aria-hidden="true" />
      <span className="shrink-0 text-body font-semibold text-zinc-100">{label}</span>
      <span className={`min-w-0 flex-1 truncate text-meta ${summaryBad ? 'text-bad' : 'text-zinc-400'}`}>{summary}</span>
      <ChevronDown size={16} className={`shrink-0 text-zinc-400 ${open ? 'rotate-180' : ''}`} aria-hidden="true" />
    </button>
    <div id={`${id}-body`} hidden={!open} className={open ? 'px-3 pb-3 kw-fade-in' : undefined}>
      {open && children}
    </div>
  </div>
);

const CRIT_ICON = {
  met: { icon: Check, cls: 'text-ok' },
  not_met: { icon: X, cls: 'text-bad' },
  not_checked: { icon: MinusCircle, cls: 'text-zinc-400' },
  claim_only: { icon: MinusCircle, cls: 'text-zinc-400' },
} as const;

const CriterionRow: React.FC<{ c: InboxCriterion }> = ({ c }) => {
  const s = CRIT_ICON[c.status];
  const Icon = s.icon;
  return (
    <li className="flex items-start gap-2.5 px-3 py-2" data-criterion={c.index} data-status={c.status}>
      <Icon size={14} className={`mt-1 shrink-0 ${s.cls}`} aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="text-body text-zinc-100 break-words">
          {c.text}{' '}
          {c.source === 'person_note' && (
            <span className="kw-chip border-violet-400/40 bg-violet-500/15 text-violet-200 !py-0 align-middle text-[11px]">Your note · round {c.round_added}</span>
          )}
          {c.source === 'must_not_change' && <span className="kw-chip !py-0 align-middle text-[11px]">Must not change</span>}
        </p>
        {c.evidence ? (
          <p className="text-meta text-zinc-400 break-words">
            Evidence: <span className="text-zinc-200">{c.evidence.label}</span>
          </p>
        ) : null}
        {c.note && <p className="text-meta text-zinc-400 break-words">{c.note}</p>}
      </div>
      <span className={`shrink-0 text-meta font-semibold ${s.cls}`}>{CRITERION_WORD[c.status]}</span>
    </li>
  );
};

const Evidence: React.FC<{ item: ReviewItem }> = ({ item }) => {
  const e = item.evidence;
  const [viewing, setViewing] = useState<number | null>(null);
  const thumbs = useRef<(HTMLButtonElement | null)[]>([]);
  if (!e.checks.length && !e.screenshots.length && !e.pr) return <p className="text-meta text-zinc-400">No evidence was attached.</p>;
  const shot = viewing !== null ? e.screenshots[viewing] : null;
  return (
    <div className="space-y-3">
      {e.checks.map((c, i) => (
        <CheckOutput key={`${c.name}-${i}`} c={c} />
      ))}
      {e.screenshots.length > 0 && (
        <ul className="flex flex-wrap gap-2" aria-label="Screenshots">
          {e.screenshots.map((s, i) => (
            <li key={`${s.url}-${i}`}>
              <button
                ref={(el) => (thumbs.current[i] = el)}
                type="button"
                onClick={() => setViewing(i)}
                className="block overflow-hidden rounded-lg border border-hairline-strong bg-black/30 kw-focus"
                aria-label={`Open screenshot ${s.caption || i + 1}`}
                data-shot
              >
                <img src={s.url} alt="" width={120} height={72} className="block h-[72px] w-[120px] object-cover" loading="lazy" />
              </button>
              {s.caption && <p className="mt-0.5 w-[120px] truncate text-[11px] text-zinc-400">{s.caption}</p>}
            </li>
          ))}
        </ul>
      )}
      {shot && (
        <ImageViewer
          file={{ id: `review-shot-${viewing}`, name: shot.caption || 'Screenshot', mime: 'image/png', size: 0 }}
          src={shot.url}
          sharedBy={item.kwant_name}
          onClose={() => {
            const i = viewing;
            setViewing(null);
            if (i !== null) thumbs.current[i]?.focus();
          }}
        />
      )}
      {e.pr && (
        <a
          href={e.pr.url}
          target="_blank"
          rel="noopener noreferrer"
          className="flex items-center gap-3 rounded-xl border border-hairline bg-white/[0.02] px-3 py-2 hover:bg-white/[0.05] kw-focus"
          data-pr-card
        >
          <GitPullRequest size={16} className="shrink-0 text-violet-300" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            <span className="block text-body font-semibold text-zinc-100 truncate">
              {e.pr.repo} #{e.pr.number}
            </span>
            <span className="block text-meta text-zinc-400">
              <span className="text-ok">+{e.pr.add}</span> <span className="text-bad">−{e.pr.del}</span> · {e.pr.files === 1 ? '1 file' : `${e.pr.files} files`}
              {e.pr.ci ? ` · CI ${e.pr.ci}` : ''}
            </span>
          </span>
          <ExternalLink size={14} className="shrink-0 text-zinc-400" aria-label="Opens in a new tab" />
        </a>
      )}
    </div>
  );
};

const CheckOutput: React.FC<{ c: ReviewItem['evidence']['checks'][number] }> = ({ c }) => {
  const [all, setAll] = useState(false);
  const t = tailLines(c.output_tail);
  return (
    <div className="space-y-1" data-check={c.name}>
      <p className="flex flex-wrap items-center gap-x-2 text-meta">
        {c.result === 'pass' ? <CheckCircle2 size={13} className="text-ok" aria-hidden="true" /> : <XCircle size={13} className="text-bad" aria-hidden="true" />}
        <span className={`font-semibold ${c.result === 'pass' ? 'text-ok' : 'text-bad'}`}>{c.result === 'pass' ? 'Passed' : 'Failed'}</span>
        <span className="font-semibold text-zinc-100">{c.name}</span>
        <code className="font-mono text-zinc-300 break-all">{c.command}</code>
        <span className="text-zinc-400">
          · {c.summary} · ran by {c.ran_by === 'checker' ? 'the checker' : 'the author'}
        </span>
      </p>
      {c.output_tail && (
        <pre className="max-h-72 overflow-auto rounded-lg border border-hairline bg-black/30 px-3 py-2 font-mono text-[12px] leading-[18px] text-zinc-200 whitespace-pre-wrap break-words" tabIndex={0} aria-label={`${c.name} output`}>
          {all ? c.output_tail : t.lines}
        </pre>
      )}
      {t.more && (
        <button type="button" onClick={() => setAll((x) => !x)} className="text-meta text-violet-300 hover:underline rounded kw-focus kw-hit" aria-expanded={all}>
          {all ? 'Show the last 12 lines' : 'Show all'}
        </button>
      )}
    </div>
  );
};

/** A modal: a 560 px dialog on desktop, a bottom sheet with a grab handle on phone. Focus stays inside. */
const Modal: React.FC<{ title: string; titleIcon?: React.ReactNode; phone: boolean; onClose: () => void; children: React.ReactNode; footer?: React.ReactNode }> = ({
  title,
  titleIcon,
  phone,
  onClose,
  children,
  footer,
}) => {
  const box = useRef<HTMLDivElement>(null);
  const id = useId();
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useEffect(() => {
    const el = box.current;
    (el?.querySelector<HTMLElement>('[data-autofocus]') ?? el)?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== 'Tab' || !box.current) return;
      const stops = Array.from(box.current.querySelectorAll<HTMLElement>('a[href], button:not([disabled]), textarea, input, [tabindex="0"]'));
      if (!stops.length) return;
      const first = stops[0];
      const last = stops[stops.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKey, true);
    return () => document.removeEventListener('keydown', onKey, true);
  }, []);
  return (
    <div className={`fixed inset-0 z-[150] flex bg-black/60 ${phone ? 'items-end' : 'items-center justify-center p-4'}`} onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={box}
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${id}-t`}
        tabIndex={-1}
        className={`flex flex-col bg-surface-overlay border border-hairline-strong shadow-raised outline-none ${
          phone ? 'w-full max-h-[92dvh] rounded-t-2xl' : 'w-full max-w-[560px] max-h-[88vh] rounded-2xl'
        }`}
        data-review-dialog
      >
        {phone && <div className="mx-auto mt-2 h-1 w-10 shrink-0 rounded-full bg-zinc-600" aria-hidden="true" />}
        <div className="flex items-center gap-2 px-4 pt-3 pb-2">
          {titleIcon}
          <h2 id={`${id}-t`} className="min-w-0 flex-1 text-[15px] font-semibold text-zinc-50 break-words">
            {title}
          </h2>
          {!phone && (
            <button type="button" onClick={onClose} aria-label="Close" className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-zinc-400 hover:bg-white/[0.06] kw-focus">
              <X size={16} aria-hidden="true" />
            </button>
          )}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-3">{children}</div>
        {footer && <div className="border-t border-hairline px-4 py-3" style={phone ? { paddingBottom: 'calc(env(safe-area-inset-bottom, 0px) + 0.75rem)' } : undefined}>{footer}</div>}
      </div>
    </div>
  );
};

const Sheet: React.FC<{ title: string; onClose: () => void; children: React.ReactNode }> = ({ title, onClose, children }) => (
  <Modal title={title} titleIcon={<CornerUpLeft size={15} aria-hidden="true" />} phone onClose={onClose}>
    {children}
  </Modal>
);

/** Roll back (Mira §8): the before-the-run preview, Files, Not undone, the safety line, then restore or fork. */
const RollBackDialog: React.FC<{
  item: ReviewItem;
  api: ReviewItemApi;
  phone: boolean;
  /** Undo a roll back: preview and restore this safety checkpoint instead. */
  undoFrom?: string;
  openHeld: number;
  denyAllOpen: () => Promise<string | null>;
  onClose: () => void;
  onDone: (r: ReviewDone, extra: { undo?: string | null; copy?: string | null }) => void;
}> = ({ item, api, phone, undoFrom, openHeld, denyAllOpen, onClose, onDone }) => {
  const [cp, setCp] = useState<Checkpoint | null>(null);
  const [preview, setPreview] = useState<RestorePreview | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [noCheckpoint, setNoCheckpoint] = useState(false);
  const keys = useRef<Record<string, string>>({});
  const inFlight = useRef(false);

  const load = useCallback(
    async (keepErr = false) => {
      if (!keepErr) setErr(null);
      setPreview(null);
      try {
        const list = await api.checkpoints(item.task_id);
        const pick = undoFrom
          ? list.find((c) => c.id === undoFrom) ?? null
          : [...list].reverse().find((c) => c.kind === 'before_run' && c.restorable) ?? null;
        if (!pick) {
          setNoCheckpoint(true);
          return;
        }
        setCp(pick);
        setPreview(await api.restorePreview(item.task_id, pick.id));
      } catch (e) {
        setErr(restoreErrorText(e));
      }
    },
    [api, item.task_id, undoFrom],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const fork = preview?.via === 'fork';
  const run = async () => {
    if (!cp || !preview || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setErr(null);
    try {
      if (openHeld > 0 && !undoFrom) {
        const e = await denyAllOpen();
        if (e) {
          setErr(e.replace('nothing was sent', 'nothing was rolled back'));
          return;
        }
      }
      const k = `${cp.id}|${preview.preview_sha256}|${fork ? 'fork' : 'restore'}`;
      keys.current[k] ??= newIdempotencyKey();
      if (fork) {
        const t = await api.forkFromCheckpoint(item.task_id, { reason: `Rolled back after approval: ${item.title}`.slice(0, 500), from_checkpoint: cp.id, preview_sha256: preview.preview_sha256 }, keys.current[k]);
        const stepped = t.stepped_down !== undefined ? t.stepped_down : preview.step_down ?? null;
        onDone({ kind: 'rolled_back', text: rolledBackForkText(item.kwant_name, stepped), task_id: item.task_id, copy_task_id: t.id }, { copy: t.id });
      } else {
        const r = await api.restore(item.task_id, cp.id, { preview_sha256: preview.preview_sha256, rerun: false }, keys.current[k]);
        const text = undoFrom ? `Roll back undone. ${rolledBackText(r).replace(/^Rolled back:?\s*/, '')}` : rolledBackText(r);
        onDone({ kind: 'rolled_back', text, task_id: item.task_id, undo_checkpoint_id: undoFrom ? null : r.before_restore_checkpoint_id }, { undo: undoFrom ? null : r.before_restore_checkpoint_id });
      }
    } catch (e) {
      setErr(restoreErrorText(e));
      if (e instanceof MissionError && e.code === 'stale_preview') void load(true);
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  const title = undoFrom ? `Undo the roll back of “${item.title}”?` : `Roll back “${item.title}”?`;
  const nu = preview ? notUndoneTitle(preview) : null;
  const step = preview ? stepDownText(preview.step_down ?? null) : null;
  const cpTime = clock(cp?.created_at);
  const approvedAt = clock(preview?.verified_at);
  const canRun = !!preview && !preview.refused && preview.changes.length > 0 && !busy;

  const footer = (
    <div className={phone ? 'grid grid-cols-2 gap-2' : 'flex justify-end gap-2'}>
      <button type="button" onClick={onClose} className={btnQuiet} data-autofocus data-keep-work>
        {phone ? 'Cancel' : 'Keep the work'}
      </button>
      <button type="button" onClick={() => void run()} disabled={!canRun} className={btnRoseFill} data-rollback-confirm>
        {busy ? <Loader2 size={14} className={spin} aria-hidden="true" /> : <RotateCcw size={14} aria-hidden="true" />}
        {preview ? (undoFrom ? 'Undo the roll back' : rollBackLabel(preview)) : 'Roll back'}
      </button>
    </div>
  );

  return (
    <Modal title={title} titleIcon={<RotateCcw size={16} className="text-zinc-200" aria-hidden="true" />} phone={phone} onClose={onClose} footer={footer}>
      <div className="space-y-3 text-body" data-rollback-body>
        {err && (
          <p className="flex items-start gap-1.5 rounded-xl border border-rose-400/30 bg-rose-500/[0.08] px-3 py-2 text-rose-200" role="alert" data-rollback-error>
            <AlertTriangle size={14} className="mt-1 shrink-0" aria-hidden="true" /> {err}
          </p>
        )}
        {noCheckpoint && <p className="text-zinc-300">There’s no checkpoint from before the run, so this task can’t be rolled back here.</p>}
        {!preview && !noCheckpoint && !err && (
          <p className="flex items-center gap-2 text-zinc-400" data-rollback-loading>
            <Loader2 size={14} className={spin} aria-hidden="true" /> Loading what changes…
          </p>
        )}
        {preview && (
          <>
            {preview.refused && (
              <p className="flex items-start gap-1.5 rounded-xl border border-rose-400/30 bg-rose-500/[0.08] px-3 py-2 text-rose-200" data-rollback-refused={preview.refused}>
                <AlertTriangle size={14} className="mt-1 shrink-0" aria-hidden="true" />
                {preview.refused === 'running' ? 'A bot is working on it now. Roll back once it stops.' : restoreErrorText(new MissionError('', 409, preview.refused))}
              </p>
            )}
            {fork ? (
              <>
                <p className="text-zinc-200" data-after-approval>
                  This was approved{approvedAt ? <> at <strong>{approvedAt}</strong></> : null}. The approved record stays. A copy of the task is made from{' '}
                  <strong>before the run{cpTime ? ` · ${cpTime}` : ''}</strong> with the files put back, and the copy waits on hold until you press Run.
                </p>
                {step && (
                  <p className="rounded-xl border border-amber-500/40 px-3 py-2 text-zinc-100" data-step-down>
                    {item.kwant_name} moves down one Flight School tier for <strong>{step.cls}</strong>: <strong>{step.from} → {step.to}</strong>.
                  </p>
                )}
              </>
            ) : undoFrom ? (
              <p className="text-zinc-300">Puts the files back to how they were just before the roll back (the safety checkpoint{cpTime ? ` · ${cpTime}` : ''}).</p>
            ) : (
              <p className="text-zinc-300">
                Puts this task’s files back to how they were before {item.kwant_name} started (checkpoint <strong className="text-zinc-100">before the run{cpTime ? ` · ${cpTime}` : ''}</strong>). Only files
                this task changed are touched.
              </p>
            )}
            <section aria-labelledby="rb-files" className="space-y-1.5">
              <h3 id="rb-files" className="flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wide text-zinc-300">
                <FolderOpen size={13} aria-hidden="true" /> Files{' '}
                <span className="font-normal normal-case tracking-normal text-zinc-500">
                  {preview.changes.length} {preview.changes.length === 1 ? 'change' : 'changes'}
                  {preview.conflicts.length + preview.skipped.length > 0 ? ` · ${preview.conflicts.length + preview.skipped.length} skipped` : ''}
                </span>
              </h3>
              {preview.changes.length + preview.conflicts.length + preview.skipped.length > 0 ? (
                <ul className="rounded-xl border border-hairline divide-y divide-hairline text-meta">
                  {preview.changes.map((c) => (
                    <li key={c.path} className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-3 py-1.5" data-rb-change={c.action}>
                      <span className={`w-24 shrink-0 font-semibold ${c.action === 'delete' ? 'text-bad' : 'text-zinc-200'}`}>{RESTORE_ACTION_LABELS[c.action]}</span>
                      <span className="min-w-0 flex-1 break-all font-mono text-zinc-100">{c.path}</span>
                    </li>
                  ))}
                  {preview.conflicts.map((c) => (
                    <li key={`c-${c.path}`} className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-3 py-1.5" data-rb-skipped>
                      <span className="w-24 shrink-0 font-semibold text-warn">Skipped</span>
                      <span className="min-w-0 flex-1 break-all font-mono text-zinc-100">{c.path}</span>
                      <span className="text-zinc-400">{conflictText(c).toLowerCase()}</span>
                    </li>
                  ))}
                  {preview.skipped.map((s) => (
                    <li key={`s-${s.path}`} className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-3 py-1.5" data-rb-skipped>
                      <span className="w-24 shrink-0 font-semibold text-warn">Skipped</span>
                      <span className="min-w-0 flex-1 break-all font-mono text-zinc-100">{s.path}</span>
                      <span className="text-zinc-400">{s.why}</span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-meta text-zinc-400">No files need putting back.</p>
              )}
            </section>
            {nu ? (
              <section className="rounded-xl border border-rose-400/35 bg-rose-500/[0.07] px-3 py-2 space-y-1" data-not-undone={preview.effects_known ? 'known' : 'unknown'}>
                <p className="flex items-center gap-1.5 font-semibold text-rose-100">
                  <AlertTriangle size={14} aria-hidden="true" /> {nu}
                </p>
                {preview.effects_known ? (
                  <>
                    <p className="text-meta text-rose-100/90">{preview.not_undone_note || 'These can’t be undone from here. Check them yourself.'}</p>
                    <ul className="list-disc pl-5 text-meta text-zinc-200 space-y-0.5">
                      {preview.external_not_undone.map((x, i) => (
                        <li key={`${x.at}-${i}`} className="break-words">
                          <span className="font-mono">{x.summary}</span>
                          {clock(x.at) ? <span className="text-zinc-400"> · {clock(x.at)}</span> : null}
                        </li>
                      ))}
                    </ul>
                  </>
                ) : (
                  <p className="text-meta text-rose-100/90">{NOT_UNDONE_UNKNOWN}</p>
                )}
              </section>
            ) : (
              <p className="flex items-center gap-1.5 rounded-xl border border-hairline bg-white/[0.02] px-3 py-2 text-meta text-zinc-400" data-not-undone="none">
                <ShieldCheck size={13} aria-hidden="true" /> Nothing left this computer. Everything this task did is undone.
              </p>
            )}
            <p className="flex items-start gap-1.5 text-meta text-zinc-400">
              <ShieldCheck size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
              <span>
                A safety checkpoint is taken first, so you can undo this roll back.
                {openHeld > 0 && !undoFrom ? (openHeld === 1 ? ' The held action is denied first.' : ' The held actions are denied first.') : ''} Afterwards the{' '}
                {fork ? 'copy' : 'task'} waits on hold until you press Run.
              </span>
            </p>
          </>
        )}
      </div>
    </Modal>
  );
};
