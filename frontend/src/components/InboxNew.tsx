// 0.9.0 V1: Inbox › New. Two panes at ≥1024 px (the queue is 392 px, the item pane takes the rest); below
// that the queue is the page and an item opens as a full page. The queue keeps H5's order and adds
// To review between Failed or stuck and Finished. Questions, approvals and finished work behave as they
// did on the old page; review items and the Low-risk card open in the item pane.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  CircleDashed,
  Eye,
  Hand,
  Info,
  Layers,
  List,
  Loader2,
  MinusCircle,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  XCircle,
} from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ModernApprovalCard } from './ModernApprovalCard';
import { ReviewDone, VERDICT_STYLE } from './ReviewItemStub';
import { ReviewItemView } from './ReviewItemView';
import { type DoneApi, type DoneDetailData, type MessageTarget, DoneChips, DoneItemView, DoneSections, defaultDoneApi, useDoneDetail } from './DoneDetail';
import { DONE_FOOTER, EARLIER_LABEL, changedState, doneTitle, isDoneGroup, isRoutineItem, metaLine, rowKicker, sectionItems } from '../doneAway';
import { usePhoneLayout } from '../usePhoneLayout';
import { approvalFromAway, awayTitle, lateAnswerText } from '../approvals';
import { ApprovalDecision, AwayError, AwayItem, actionFailureNotice, awayApi, awaySummary, itemLabel, windowLabel } from '../away';
import { InboxGroup, InboxItem, ReviewItem, isBeingChecked, isReview, pendingHeld, reviewWord } from '../api/inbox';
import {
  BatchRow,
  Crumb,
  approveAll,
  approveOne,
  batchItems,
  checkAgain,
  clearBatch,
  loadInbox,
  nextAfter,
  resolvePending,
  select,
  setCrumb,
  comeBack,
  markHeld,
  findItem,
  setNotice,
  showEarlier,
  stopAfterThisOne,
  useInbox,
  inboxStore,
} from '../inboxStore';

export const WIDE_QUERY = '(min-width: 1024px)';

export function useWide(): boolean {
  const get = () => typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia(WIDE_QUERY).matches;
  const [wide, setWide] = useState(get);
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const m = window.matchMedia(WIDE_QUERY);
    const on = () => setWide(m.matches);
    m.addEventListener?.('change', on);
    return () => m.removeEventListener?.('change', on);
  }, []);
  return wide;
}

const CRUMB_LABEL: Record<'board' | 'dev', string> = { board: 'Mission board', dev: 'Dev desk' };

/** 0.9.5 F: the queue's two sections (Mira's mockup 2). Rows keep their own group's tint. */
const SECTION: Record<
  'needs' | 'done',
  {
    label: string;
    short: string;
    head: string;
    icon: React.ReactNode;
    empty: string;
  }
> = {
  needs: {
    label: 'Needs you',
    short: 'Needs you',
    head: 'text-amber-300',
    icon: <Hand size={13} aria-hidden="true" />,
    empty: 'Nothing needs you right now.',
  },
  done: {
    label: 'Done while you were away',
    short: 'Done',
    head: 'text-teal-300',
    icon: <CheckCircle2 size={13} aria-hidden="true" />,
    empty: 'Nothing finished since you last looked.',
  },
};

const GROUP_STYLE: Record<InboxGroup, { head: string; icon: React.ReactNode; row: string }> = {
  waiting_on_you: { head: 'text-amber-300', icon: <Hand size={13} aria-hidden="true" />, row: 'border-amber-500/30 bg-amber-950/20' },
  failed_or_stuck: { head: 'text-rose-300', icon: <AlertTriangle size={13} aria-hidden="true" />, row: 'border-rose-500/25 bg-rose-950/20' },
  to_review: { head: 'text-sky-300', icon: <ShieldCheck size={13} aria-hidden="true" />, row: 'border-zinc-800 bg-zinc-900/60' },
  finished: { head: 'text-zinc-400', icon: <CheckCircle2 size={13} aria-hidden="true" />, row: 'border-zinc-800 bg-zinc-900/60' },
};

const okBtn =
  'kw-focus inline-flex items-center justify-center gap-1.5 rounded-lg bg-teal-300 text-zinc-950 font-semibold hover:bg-teal-200 disabled:opacity-50 disabled:cursor-not-allowed';
const plainBtn =
  'kw-focus inline-flex items-center justify-center gap-1.5 rounded-lg border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 disabled:opacity-50 disabled:cursor-not-allowed';

/** "14:08": the queue shows the clock, like the mockups. */
export function clock(ts: string): string {
  const t = Date.parse(ts);
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** A row's title: an approve that came back says so. */
export function rowTitle(i: InboxItem): string {
  if (isReview(i) && (i.stale || i.approve_error)) return `Couldn’t approve: ${i.title}`;
  if (isDoneGroup(i.group) && isRoutineItem(i)) return doneTitle(i);
  return isReview(i) ? i.title : awayTitle(i as AwayItem);
}

/** 0.9.5 F: a finished item that opens in the pane (what ran, the answer, the trail) instead of its chat. */
export const opensInPane = (i: InboxItem | null | undefined): boolean =>
  !!i && (isReview(i) || (i.group === 'finished' && (i.kind === 'routine_run' || i.kind === 'delegation')));

export function staleLine(paths: string[]): string {
  if (!paths.length) return 'Its files changed after its check';
  return `${paths[0]}${paths.length > 1 ? ` and ${paths.length - 1} more` : ''} changed after its check`;
}

const names = (xs: string[]): string => {
  const u = [...new Set(xs)];
  return u.length <= 1 ? u.join('') : `${u.slice(0, -1).join(', ')} and ${u[u.length - 1]}`;
};

// ---------- Rows ----------

/** D7 "Being checked": the verifier holds it and there is no verdict yet. */
const BEING_CHECKED_STYLE = { icon: Eye, text: 'text-zinc-200' } as const;

const ReviewSubline: React.FC<{ item: ReviewItem }> = ({ item }) => {
  if (item.stale) return <span className="text-rose-300">{staleLine(item.stale.paths)}</span>;
  if (item.approve_error) return <span className="text-rose-300">{item.approve_error.message}</span>;
  const checking = isBeingChecked(item);
  const v = checking ? BEING_CHECKED_STYLE : VERDICT_STYLE[item.verdict.result];
  const Icon = v.icon;
  // V1-3: only held actions still waiting count; a denied or allowed one is done.
  const heldOpen = pendingHeld(item).length;
  const lastRoundFail = item.verdict.result === 'fail' && item.round.n >= item.round.max;
  return (
    <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
      {item.round.n > 1 && (
        <span className="px-1.5 rounded-md border border-violet-400/40 bg-violet-500/10 text-violet-200 text-[11px]" data-round-chip>
          Round {item.round.n}
        </span>
      )}
      <span className={`inline-flex items-center gap-1 font-semibold ${v.text}`} data-verdict-word={checking ? 'being_checked' : item.verdict.result}>
        <Icon size={12} aria-hidden="true" />
        {reviewWord(item)}
      </span>
      {item.verdict.confidence && <span className="text-zinc-400">· {item.verdict.confidence}</span>}
      {lastRoundFail && (
        <span className="text-zinc-400">
          · round {item.round.n} of {item.round.max} <span className="text-zinc-300">your call</span>
        </span>
      )}
      {item.risk.level === 'high' && <span className="px-1.5 rounded-md border border-amber-500/40 bg-amber-500/10 text-amber-200 text-[11px]">High risk</span>}
      {item.risk.level === 'low' && <span className="px-1.5 rounded-md border border-zinc-600 text-zinc-300 text-[11px]">Low risk</span>}
      {heldOpen > 0 && <span className="text-zinc-400" data-held-count>{heldOpen} held</span>}
    </span>
  );
};

const AwaySubline: React.FC<{ item: InboxItem }> = ({ item }) => {
  const { label, tone } = itemLabel(item as AwayItem);
  const toneText = tone === 'amber' ? 'text-amber-300' : tone === 'rose' ? 'text-rose-300' : tone === 'emerald' ? 'text-teal-300' : tone === 'sky' ? 'text-sky-300' : 'text-zinc-400';
  const sum = awaySummary(item as AwayItem);
  const showSum = sum && item.kind !== 'approval' && !(item.kind === 'question' && item.choices?.length);
  return (
    <span className="flex items-center gap-1.5 min-w-0">
      <span className={`shrink-0 font-semibold ${toneText}`}>{label}</span>
      {showSum && <span className="min-w-0 truncate text-zinc-400">{sum}</span>}
    </span>
  );
};

interface RowProps {
  item: InboxItem;
  selected: boolean;
  focusable: boolean;
  wide: boolean;
  busy: string | null;
  onPick: (i: InboxItem) => void;
  onAnswer: (i: InboxItem, choice: number) => void;
  onResolve: (i: InboxItem, d: ApprovalDecision) => void;
  /** A routine's review task in Done: its run's first line, as the run's own row would show it. */
  doneSummary?: string | null;
  /** The conversation it happened in ("@Spring launch"), said before the bot's name like the mockup. */
  place?: string | null;
}

const QueueRow: React.FC<RowProps> = ({ item, selected, focusable, wide, busy, onPick, onAnswer, onResolve, doneSummary, place }) => {
  const review = isReview(item) ? item : null;
  const title = rowTitle(item);
  // The server decides one_tap; an item with no verdict (Not checked yet, Being checked) never gets inline Approve.
  const oneTap = !!review && review.one_tap && !review.stale && !review.approve_error && review.verdict.result !== 'none' && !isBeingChecked(review);
  const stale = !!review?.stale;
  const disabled = busy !== null;
  const tint = selected ? 'border-violet-400/70 bg-violet-500/10' : GROUP_STYLE[item.group].row;
  const kicker = (isDoneGroup(item.group) ? rowKicker(item) : null) ?? place ?? null;
  const doneRow = !review && isDoneGroup(item.group) && opensInPane(item);
  return (
    <li role="presentation" className={`relative rounded-xl border transition-colors ${tint}`} data-row={item.id} data-group={item.group}>
      <div
        role="option"
        id={`inbox-opt-${item.id}`}
        aria-selected={selected}
        tabIndex={focusable ? 0 : -1}
        data-option={item.id}
        data-review={review ? '' : undefined}
        data-done={doneRow ? '' : undefined}
        onClick={() => onPick(item)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onPick(item);
          }
        }}
        className={`kw-focus flex items-start gap-3 p-3 rounded-xl cursor-pointer ${oneTap ? (wide ? 'pr-[118px]' : 'pr-[124px]') : ''}`}
      >
        <ModernAvatar role={item.kwant_name} name={item.kwant_name} size={30} kwantId={item.kwant_id} />
        <span className="flex-1 min-w-0">
          <span className="flex items-center gap-2 text-[11px]">
            <span className="font-semibold text-zinc-300 truncate">
              {kicker && (
                <span className="font-normal text-zinc-400" data-kicker>
                  {kicker} ·{' '}
                </span>
              )}
              {item.kwant_name}
            </span>
            <time dateTime={item.timestamp} className="ml-auto shrink-0 tabular-nums text-zinc-400">
              {clock(item.timestamp)}
            </time>
          </span>
          <span className="block mt-0.5 text-sm font-semibold text-zinc-100 break-words max-md:line-clamp-3 md:truncate">{title}</span>
          <span className="block mt-1 text-xs">
            {review && doneSummary ? (
              <span className="block text-zinc-400 max-md:line-clamp-2 md:truncate" data-done-summary>
                {doneSummary}
              </span>
            ) : review ? (
              <ReviewSubline item={review} />
            ) : doneRow && item.summary ? (
              <span className="block text-zinc-400 max-md:line-clamp-2 md:truncate" data-done-summary>
                {item.summary}
              </span>
            ) : (
              <AwaySubline item={item} />
            )}
          </span>
        </span>
      </div>

      {oneTap && (
        <button
          type="button"
          disabled={disabled}
          onClick={() => approveOne(item)}
          className={`${okBtn} absolute right-3 top-1/2 -translate-y-1/2 px-3 text-xs ${wide ? 'h-7' : 'h-11 min-w-[104px]'}`}
          aria-label={`Approve “${item.title}”`}
          data-inline-approve={item.id}
        >
          {busy === item.id ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={13} aria-hidden="true" />}
          {busy === item.id ? 'Approving…' : 'Approve'}
        </button>
      )}

      {stale && (
        <div className="px-3 pb-3 pl-[54px]">
          <button
            type="button"
            disabled={disabled}
            onClick={() => checkAgain(item)}
            className={`${plainBtn} px-3 text-xs ${wide ? 'h-7' : 'h-11'}`}
            data-check-again={item.id}
          >
            {busy === item.id ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <RotateCcw size={12} aria-hidden="true" />}
            Check again
          </button>
        </div>
      )}

      {item.kind === 'question' && (item.choices?.length ?? 0) > 0 && (
        <div role="group" aria-label={`Answer: ${title}`} className="px-3 pb-3 pl-[54px] flex flex-wrap gap-2">
          {item.choices!.map((c, n) => (
            <button key={n} type="button" disabled={disabled} onClick={() => onAnswer(item, n)} className={`${plainBtn} px-3 text-xs ${wide ? 'min-h-[30px]' : 'min-h-[44px]'}`}>
              {c}
            </button>
          ))}
        </div>
      )}

      {item.kind === 'approval' && (
        <div className="px-3 pb-1 pl-[54px] [&>div]:my-0 [&>div]:mb-2">
          <ModernApprovalCard approval={approvalFromAway(item as AwayItem)} disabled={disabled} onResolve={(_, d) => onResolve(item, d)} />
        </div>
      )}

      {busy === item.id && !review && (
        <p className="px-3 pb-3 pl-[54px] inline-flex items-center gap-1.5 text-[11px] text-zinc-400">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Sending…
        </p>
      )}
    </li>
  );
};

/** Progress of a run, for the card and the pane. */
export function batchProgress(rows: BatchRow[]) {
  const done = rows.filter((r) => r.status === 'approved' || r.status === 'came_back' || r.status === 'error').length;
  return {
    done,
    total: rows.length,
    approved: rows.filter((r) => r.status === 'approved').length,
    back: rows.filter((r) => r.status === 'came_back').length,
    failed: rows.filter((r) => r.status === 'error').length,
    current: rows.findIndex((r) => r.status === 'approving'),
  };
}

export function progressText(rows: BatchRow[]): string {
  const p = batchProgress(rows);
  const parts = [`${p.done} of ${p.total} done`, `${p.approved} approved`];
  if (p.back) parts.push(`${p.back} came back`);
  if (p.failed) parts.push(`${p.failed} couldn’t be approved`);
  return parts.join(' · ');
}

const ProgressBar: React.FC<{ rows: BatchRow[] }> = ({ rows }) => {
  const p = batchProgress(rows);
  return (
    <div className="h-1.5 rounded-full bg-zinc-800 overflow-hidden" role="progressbar" aria-valuemin={0} aria-valuemax={p.total} aria-valuenow={p.done} aria-label="Approve all">
      <div
        className="h-full w-full origin-left rounded-full bg-violet-400 transition-transform duration-300 motion-reduce:transition-none"
        style={{ transform: `scaleX(${p.total ? p.done / p.total : 0})` }}
      />
    </div>
  );
};

const BatchCard: React.FC<{ items: InboxItem[]; selected: boolean; focusable: boolean; wide: boolean; onPick: () => void }> = ({ items, selected, focusable, wide, onPick }) => {
  const { batch, busy } = useInbox();
  const running = !!batch?.running;
  const n = running ? batch!.rows.length : items.length;
  const people = running ? batch!.rows : items;
  return (
    <li role="presentation" className={`rounded-xl border transition-colors ${selected ? 'border-violet-400/70 bg-violet-500/10' : 'border-violet-400/30 bg-violet-950/20'}`} data-batch-card>
      <div
        role="option"
        id="inbox-opt-batch"
        aria-selected={selected}
        tabIndex={focusable ? 0 : -1}
        data-option="batch"
        onClick={onPick}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onPick();
          }
        }}
        className="kw-focus block p-3 rounded-xl cursor-pointer"
      >
        <span className="flex items-center gap-1.5 text-sm font-semibold text-zinc-100">
          <Layers size={15} className="text-zinc-300" aria-hidden="true" /> Low risk, checks passed · {n}
        </span>
        <span className="block mt-1 text-xs text-zinc-400">
          Each one passed every required check and the checker passed it with high confidence. Nothing held, nothing it couldn’t check.
        </span>
        {running && (
          <span className="block mt-2 space-y-1">
            <ProgressBar rows={batch!.rows} />
            <span className="block text-[11px] text-zinc-400" data-batch-card-progress>
              {progressText(batch!.rows)}
            </span>
          </span>
        )}
      </div>
      <div className="px-3 pb-3 flex items-center gap-2">
        <span className="flex -space-x-1.5 shrink-0" aria-hidden="true">
          {people.slice(0, 5).map((p) => (
            <span key={p.id} className="rounded-full ring-2 ring-zinc-950">
              <ModernAvatar role={p.kwant_name} name={p.kwant_name} size={20} kwantId={p.kwant_id} />
            </span>
          ))}
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-zinc-400">{names(people.map((p) => p.kwant_name))}</span>
        <button
          type="button"
          disabled={running || busy !== null}
          onClick={() => {
            onPick();
            void approveAll();
          }}
          className={`${okBtn} shrink-0 px-3 text-xs ${wide ? 'h-8' : 'h-11'}`}
          data-approve-all
        >
          {running ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={13} aria-hidden="true" />}
          {running ? `Approving ${Math.max(1, batchProgress(batch!.rows).current + 1)} of ${n}…` : `Approve all ${n}`}
        </button>
      </div>
    </li>
  );
};

// ---------- Panes ----------

const ROW_STATUS: Record<BatchRow['status'], { word: string; tone: string; icon: React.ReactNode }> = {
  approved: { word: 'Approved', tone: 'text-teal-300', icon: <Check size={14} className="text-teal-300" aria-hidden="true" /> },
  came_back: { word: 'Came back', tone: 'text-rose-300', icon: <XCircle size={14} className="text-rose-300" aria-hidden="true" /> },
  approving: { word: 'Approving…', tone: 'text-violet-200', icon: <Loader2 size={14} className="text-violet-200 animate-spin motion-reduce:animate-none" aria-hidden="true" /> },
  waiting: { word: 'Waiting', tone: 'text-zinc-400', icon: <CircleDashed size={14} className="text-zinc-500" aria-hidden="true" /> },
  error: { word: 'Couldn’t approve', tone: 'text-rose-300', icon: <AlertTriangle size={14} className="text-rose-300" aria-hidden="true" /> },
  stopped: { word: 'Left in To review', tone: 'text-zinc-400', icon: <MinusCircle size={14} className="text-zinc-500" aria-hidden="true" /> },
};

export const BatchPane: React.FC<{ wide: boolean; onOpenItem: (id: string) => void }> = ({ wide, onOpenItem }) => {
  const { data, batch, busy } = useInbox();
  const items = batchItems(data);
  const rows: BatchRow[] =
    batch?.rows ?? items.map((i) => ({ id: i.id, task_id: i.task_id ?? '', title: i.title, kwant_id: i.kwant_id, kwant_name: i.kwant_name, status: 'waiting' as const }));
  const running = !!batch?.running;
  const finished = !!batch && !running;
  const p = batchProgress(rows);
  return (
    <div className="flex flex-col min-h-full" data-batch-pane>
      <div className="flex-1 space-y-4">
        <header className="flex items-start gap-3">
          <span className="shrink-0 w-[34px] h-[34px] rounded-lg bg-teal-400/10 border border-teal-400/25 grid place-items-center text-teal-300">
            <Layers size={17} aria-hidden="true" />
          </span>
          <span className="min-w-0">
            <h2 className="text-base font-semibold text-zinc-100">Low risk, checks passed</h2>
            <p className="text-xs text-zinc-400">
              {rows.length} items from {names(rows.map((r) => r.kwant_name))}
            </p>
          </span>
        </header>

        <section className="rounded-xl border border-teal-400/25 bg-teal-400/5 p-3 space-y-2">
          <p className="flex items-center gap-1.5 text-sm font-semibold text-zinc-100">
            <Layers size={15} className="text-teal-300" aria-hidden="true" />
            {running ? `Approving ${rows.length} low-risk items, one at a time` : finished ? `Approve all finished` : `Approve ${rows.length} low-risk items, one at a time`}
          </p>
          <p className="text-xs text-zinc-300">
            Before each one, the app checks that its files haven’t changed since the checker looked. One that changed goes back to the queue on its own; the rest carry on.
          </p>
          <ProgressBar rows={rows} />
          <p className="text-[11px] text-zinc-400" data-batch-progress>
            {progressText(rows)}
          </p>
          {batch?.halted && <p className="text-xs text-rose-300">{batch.halted}</p>}
        </section>

        <section aria-labelledby="batch-rows">
          <h3 id="batch-rows" className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
            <List size={13} aria-hidden="true" /> In this batch <span className="font-normal">{rows.length}</span>
          </h3>
          <ul className="rounded-xl border border-zinc-800 divide-y divide-zinc-800 overflow-hidden">
            {rows.map((r) => {
              const s = ROW_STATUS[r.status];
              const back = r.status === 'came_back' || r.status === 'error';
              return (
                <li key={r.id} className={back ? 'bg-rose-950/20' : ''} data-batch-row={r.id} data-status={r.status}>
                  <button
                    type="button"
                    onClick={() => onOpenItem(r.id)}
                    className={`kw-focus w-full flex items-center gap-2.5 px-3 text-left ${wide ? 'min-h-[40px] py-1.5' : 'min-h-[52px] py-2'}`}
                  >
                    {s.icon}
                    <ModernAvatar role={r.kwant_name} name={r.kwant_name} size={20} kwantId={r.kwant_id} />
                    <span className="min-w-0 flex-1">
                      <span className="text-[13px] font-semibold text-zinc-100">{r.title}</span>
                      <span className="text-xs text-zinc-400"> · {r.kwant_name}{r.status !== 'came_back' && r.status !== 'error' ? ' · pass, high' : ''}</span>
                      {r.note && <span className="block text-[11px] text-rose-300">{r.note}</span>}
                    </span>
                    {/* V1-4: "Came back" is drawn; an error row's note already starts with "Couldn’t approve". */}
                    <span className={`shrink-0 text-[11px] ${s.tone}`} data-batch-word>{r.status === 'error' ? '' : s.word}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </section>

        <p className="flex items-start gap-1.5 rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 text-xs text-zinc-300">
          <Info size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
          You can leave this page. The batch keeps going, and the Inbox shows what came back.
        </p>
      </div>

      <div className="sticky bottom-0 mt-4 -mx-4 sm:-mx-6 px-4 sm:px-6 py-3 border-t border-zinc-800 bg-zinc-950/95 flex flex-wrap gap-2">
        {running ? (
          <>
            <button type="button" disabled className={`${plainBtn} px-4 text-sm ${wide ? 'h-9' : 'h-11'}`} data-batch-running>
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              Approving {Math.max(1, p.current + 1)} of {rows.length}…
            </button>
            <button type="button" disabled={batch!.stopRequested} onClick={stopAfterThisOne} className={`${plainBtn} px-4 text-sm ${wide ? 'h-9' : 'h-11'}`} data-stop-after>
              {batch!.stopRequested ? 'Stopping after this one…' : 'Stop after this one'}
            </button>
          </>
        ) : finished ? (
          <button type="button" onClick={clearBatch} className={`${plainBtn} px-4 text-sm ${wide ? 'h-9' : 'h-11'}`} data-batch-done>
            Done
          </button>
        ) : (
          <button type="button" disabled={rows.length < 2 || busy !== null} onClick={() => void approveAll()} className={`${okBtn} px-4 text-sm ${wide ? 'h-9' : 'h-11'}`} data-approve-all-pane>
            <Check size={14} aria-hidden="true" /> Approve all {rows.length}
          </button>
        )}
      </div>
    </div>
  );
};

const EmptyQueue: React.FC = () => (
  <div className="py-16 flex flex-col items-center text-center gap-2" data-all-clear>
    <CheckCircle2 size={30} className="text-teal-300" aria-hidden="true" />
    <p className="text-sm font-semibold text-zinc-100">You’re all caught up</p>
    <p className="text-xs text-zinc-400 max-w-xs">Nothing waits on you and nothing is left to review. Questions, approvals and finished work will show up here.</p>
  </div>
);

const Skeleton: React.FC = () => (
  <ul className="space-y-2" aria-hidden="true" data-skeleton>
    {[0, 1, 2].map((n) => (
      <li key={n} className="h-[74px] rounded-xl border border-zinc-800 bg-zinc-900/60 animate-pulse motion-reduce:animate-none" />
    ))}
  </ul>
);

// ---------- The item pane ----------

/**
 * A review opens in ReviewItemView. A routine's review task in Done gets the done layout inside it (meta line,
 * chips, answer, trail, Why above Looks good and Ask to redo); a finished item with no review task opens in
 * DoneItemView.
 */
const ItemPane: React.FC<{
  item: InboxItem;
  runItem: InboxItem | null;
  entry: Exclude<Crumb, null> | 'inbox';
  onDone: (r: ReviewDone) => void;
  onHeldResolved: (approvalId: string, status: 'allowed' | 'denied' | 'expired') => void;
  onOpenItemChat: (i: InboxItem) => void;
  onOpenMessage?: (t: MessageTarget) => void;
  onEditRoutine?: (routineId: string) => void;
  placeName?: (channelId: string | null) => string | null;
  api?: DoneApi;
  initial?: DoneDetailData;
}> = ({ item, runItem, entry, onDone, onHeldResolved, onOpenItemChat, onOpenMessage, onEditRoutine, placeName, api, initial }) => {
  const phone = usePhoneLayout();
  const d = useDoneDetail(item, runItem, api ?? defaultDoneApi, initial);
  const doneLayout = isDoneGroup(item.group) && (!isReview(item) || isRoutineItem(item));
  if (isReview(item) && !doneLayout) return <ReviewItemView item={item} entry={entry} onDone={onDone} onHeldResolved={onHeldResolved} />;
  const openChat = () => (d.target && onOpenMessage ? onOpenMessage(d.target) : onOpenItemChat(runItem ?? item));
  const place = d.routine ? (placeName?.(d.routine.channel_id) ?? null) : null;
  const routineId = d.routine?.id;
  const edit = onEditRoutine && routineId ? () => onEditRoutine(routineId) : undefined;
  if (!isReview(item)) return <DoneItemView item={item} d={d} place={place} phone={phone} onOpenChat={openChat} onEditRoutine={onEditRoutine} />;
  return (
    <ReviewItemView
      item={item}
      entry={entry}
      onDone={onDone}
      onHeldResolved={onHeldResolved}
      done={{
        title: doneTitle(item),
        meta: metaLine(item, d.routine, d.run),
        chips: <DoneChips item={item} d={d} onAddCheck={edit} />,
        body: <DoneSections item={item} d={d} place={place} onOpenChat={openChat} onEditRoutine={onEditRoutine} />,
        changed: changedState(item, d.message),
      }}
    />
  );
};

// ---------- Page ----------

export interface InboxNewProps {
  /** A non-review row: open its chat, as the old page did. */
  onOpen: (item: AwayItem) => void;
  /** Unseen counts for the badge fallback (older servers). */
  onCounts?: (waiting: number) => void;
  onApprovalResolved?: (id: string) => void;
  /** Phone: back to the chat list. */
  onBack?: () => void;
  /** The back crumb: return to the page the item was opened from. */
  onCrumb?: (c: Exclude<Crumb, null>) => void;
  top?: React.ReactNode;
  now?: number;
  /** Tests and screenshots: skip the first load. */
  noLoad?: boolean;
  /** Tests: force the layout. */
  wide?: boolean;
  /** 0.9.5 F: Open in chat at the answer itself; without it, the item's chat opens. */
  onOpenMessage?: (t: MessageTarget) => void;
  /** Edit routine / Add one: open the routine in Routines. */
  onEditRoutine?: (routineId: string) => void;
  /** A channel's name for "Wrote the answer in …" (null: the bot's direct chat). */
  placeName?: (channelId: string | null) => string | null;
  /** Tests: where a done item's routine, run and answer come from. */
  doneApi?: DoneApi;
  /** Tests and screenshots: a done item's details by item id, instead of loading them. */
  doneDetails?: Record<string, DoneDetailData>;
}

/** Writes `?item=` for the open item; the Shell sets the path first, so this waits a tick. */
function writeItemParam(id: string | null) {
  setTimeout(() => {
    const q = new URLSearchParams(window.location.search);
    const had = q.get('item');
    if ((had ?? null) === id) return;
    if (id) q.set('item', id);
    else q.delete('item');
    const s = q.toString();
    window.history.replaceState(window.history.state, '', `${window.location.pathname}${s ? `?${s}` : ''}`);
  }, 0);
}

export const InboxNew: React.FC<InboxNewProps> = ({
  onOpen,
  onCounts,
  onApprovalResolved,
  onBack,
  onCrumb,
  top,
  now = Date.now(),
  noLoad = false,
  wide: forceWide,
  onOpenMessage,
  onEditRoutine,
  placeName,
  doneApi,
  doneDetails,
}) => {
  const st = useInbox();
  const mqWide = useWide();
  const wide = forceWide ?? mqWide;
  const { data, loading, error, notice, busy, selectedId, crumb } = st;
  const listRef = useRef<HTMLDivElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);
  const onCountsRef = useRef(onCounts);
  onCountsRef.current = onCounts;

  // First load, then mark the window seen (finished and failed clear; review items stay, ENG).
  useEffect(() => {
    const key = new URLSearchParams(window.location.search).get('item');
    if (key && !inboxStore.get().pendingKey && inboxStore.get().selectedId !== key) inboxStore.patch({ pendingKey: key });
    if (noLoad) return;
    let cancelled = false;
    loadInbox().then(async (first) => {
      if (cancelled || !first) return;
      try {
        await awayApi.seen();
        onCountsRef.current?.(first.counts.waiting_on_you);
      } catch {
        // the badge just stays until the next visit
      }
    });
    return () => {
      cancelled = true;
    };
  }, [noLoad]);

  // A pending `?item=` or Review press resolves once the list has it.
  useEffect(() => {
    if (st.pendingKey && data) resolvePending();
  }, [st.pendingKey, data]);

  const sections = data ? sectionItems(data) : null;
  const bItems = batchItems(data);
  const showBatch = !!data?.batch && (bItems.length >= 2 || !!st.batch?.running);
  const folded = new Set(Object.values(sections?.runFor ?? {}).map((r) => r.id));
  const selectable = data ? [...data.items.filter((i) => opensInPane(i) && !folded.has(i.id)).map((i) => i.id), ...(showBatch ? ['batch'] : [])] : [];
  const selItem =
    selectedId && selectedId !== 'batch' ? (data?.items.find((i) => i.id === selectedId) ?? bItems.find((i) => i.id === selectedId) ?? null) : null;
  const selValid = selectedId === 'batch' ? showBatch || !!st.batch : !!selItem && opensInPane(selItem);

  // Desktop: something is always open. The first item (in queue order) on load, the next one after.
  useEffect(() => {
    if (!wide || !data || st.pendingKey) return;
    if (selValid) return;
    // Needs you first, then Done: the first item that opens here, in the order the queue draws them.
    const order = sections ? [...sections.needs, ...sections.done] : data.items;
    const first = order.find((i) => opensInPane(i) && !folded.has(i.id))?.id ?? (showBatch ? 'batch' : null);
    if (first !== selectedId) select(first);
    // sections and folded come from data
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wide, data, selValid, selectedId, showBatch, st.pendingKey]);

  // The address carries the open item, so a reload or a shared link opens it again.
  useEffect(() => {
    writeItemParam(selValid && selectedId !== 'batch' ? selectedId : null);
  }, [selValid, selectedId]);

  const pick = useCallback(
    (i: InboxItem) => {
      if (opensInPane(i)) {
        select(i.id);
        setCrumb(null);
        return;
      }
      onOpen(i as AwayItem);
    },
    [onOpen],
  );

  const act = async (item: InboxItem, run: () => Promise<unknown>, denied = false) => {
    if (inboxStore.get().busy) return;
    inboxStore.patch({ busy: item.id, notice: null });
    const title = awayTitle(item as AwayItem);
    try {
      const reply = await run();
      const late = item.kind === 'approval' && !denied ? lateAnswerText(reply, item.kwant_name) : null;
      const done = denied ? 'Denied, nothing ran:' : late ?? `Sent to ${item.kwant_name}:`;
      inboxStore.patch({ busy: null, notice: { text: `${done} “${title}”`, tone: 'ok' } });
    } catch (err) {
      const status = err instanceof AwayError ? err.status : 0;
      const n = actionFailureNotice(status, err instanceof Error ? err.message : 'Something went wrong.', title);
      inboxStore.patch({ busy: null, notice: { text: n.text, tone: n.error ? 'error' : 'info' } });
    }
    const next = await loadInbox();
    if (next) onCountsRef.current?.(next.counts.waiting_on_you);
  };

  const onAnswer = (item: InboxItem, choice: number) => act(item, () => awayApi.answer(item.id, choice));
  const onResolve = (item: InboxItem, decision: ApprovalDecision) =>
    act(
      item,
      async () => {
        try {
          const r = await awayApi.resolve(item.id, decision);
          onApprovalResolved?.(item.id);
          return r;
        } catch (err) {
          if (err instanceof AwayError && (err.status === 409 || err.status === 404)) onApprovalResolved?.(item.id);
          throw err;
        }
      },
      decision === 'deny',
    );

  // J/K and arrows move through the queue; on desktop the selection follows a review row.
  const onListKey = (e: React.KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (t.getAttribute('role') !== 'option') return;
    const k = e.key;
    const dir = k === 'j' || k === 'J' || k === 'ArrowDown' ? 1 : k === 'k' || k === 'K' || k === 'ArrowUp' ? -1 : 0;
    if (!dir) return;
    e.preventDefault();
    const opts = Array.from(listRef.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? []);
    const at = opts.indexOf(t);
    const next = opts[Math.min(opts.length - 1, Math.max(0, at + dir))];
    if (!next) return;
    next.focus();
    const id = next.dataset.option!;
    if (wide && (id === 'batch' || next.dataset.review !== undefined || next.dataset.done !== undefined)) select(id);
  };

  const onDone = (r: ReviewDone) => {
    const s = inboxStore.get();
    setNotice({ text: r.text, tone: r.kind === 'stale' ? 'error' : 'ok' });
    if (r.kind === 'stale') {
      // r.text leaves out the paths: the row in Failed or stuck (and the pane) say what changed, once.
      const it = findItem(s.data, r.task_id) ?? findItem(s.data, s.selectedId);
      if (it) comeBack(it, r.paths ?? []);
      return;
    }
    const id = s.selectedId;
    if (id) select(nextAfter(s.data, id));
    void loadInbox();
  };

  const firstRow = sections ? [...sections.needs, ...sections.done][0]?.id : undefined;
  const firstFocusable = selectable.includes(selectedId ?? '') ? selectedId : (firstRow ?? (showBatch ? 'batch' : null));
  const sinceAt = data ? Date.parse(data.since) : NaN;
  const weekShown = Number.isFinite(sinceAt) && now - sinceAt >= 7 * 86400_000 - 60_000;
  const empty = !!data && data.items.length === 0 && !showBatch;
  const header = (
    <div className={`h-14 shrink-0 border-b border-zinc-800/80 ${onBack && !wide ? 'px-2' : 'px-4'} flex items-center justify-between gap-3 select-none`}>
      {onBack && !wide && (
        <button type="button" onClick={onBack} aria-label="Back to chats" className="kw-focus shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800">
          <ChevronLeft size={18} />
        </button>
      )}
      <div className="min-w-0 flex-1">
        <h2 className="text-sm font-semibold text-zinc-100 truncate">Inbox</h2>
        <p className="text-[11px] text-zinc-400 truncate mt-0.5">{data ? windowLabel(data.since, now) : 'What your bots need, finished and failed'}</p>
      </div>
      <button
        type="button"
        onClick={() => void loadInbox()}
        disabled={loading}
        className="kw-focus inline-flex items-center gap-1.5 px-2.5 py-1 min-h-[32px] rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60"
      >
        <RefreshCw size={12} className={loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
        Refresh
      </button>
    </div>
  );

  const status = (
    <div role="status" aria-live="polite" data-inbox-status>
      {notice && (
        <p
          className={`px-3 py-2 rounded-lg border text-xs ${
            notice.tone === 'error' ? 'border-rose-500/30 bg-rose-500/10 text-rose-200' : notice.tone === 'info' ? 'border-slate-500/30 bg-slate-500/10 text-slate-200' : 'border-teal-400/30 bg-teal-400/10 text-teal-100'
          }`}
        >
          {notice.text}
        </p>
      )}
    </div>
  );

  const queue = (
    <div className="flex-1 overflow-y-auto p-3 sm:p-4" aria-busy={loading}>
      <div className="space-y-4">
        {top}
        {error && (
          <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300" data-inbox-error>
            <span>Couldn’t load the Inbox: {error}</span>
            <button type="button" onClick={() => void loadInbox()} className="kw-focus ml-auto px-2 py-0.5 min-h-[28px] rounded border border-rose-500/30 hover:bg-rose-500/10">
              Retry
            </button>
          </div>
        )}
        {status}
        {!data && loading && <Skeleton />}
        {empty && <EmptyQueue />}
        {data && !empty && sections && (
          <div
            ref={listRef}
            role="listbox"
            aria-label="Inbox"
            aria-activedescendant={undefined}
            onKeyDown={onListKey}
            className={`space-y-5 ${error ? 'opacity-50' : ''}`}
          >
            {(['needs', 'done'] as const).map((key) => {
              const rows = sections[key];
              const reviewsEnd = key === 'done' ? rows.filter((i) => i.group === 'to_review').length : 0;
              const withBatch = key === 'done' && showBatch;
              const count = rows.length;
              const sec = SECTION[key];
              const row = (i: InboxItem) => (
                <QueueRow
                  key={`${i.kind}:${i.id}`}
                  item={i}
                  selected={selectedId === i.id && opensInPane(i)}
                  focusable={firstFocusable === i.id}
                  wide={wide}
                  busy={busy}
                  onPick={pick}
                  onAnswer={onAnswer}
                  onResolve={onResolve}
                  doneSummary={sections.runFor[i.id]?.summary || null}
                  place={i.link?.channel_id ? (placeName?.(i.link.channel_id) ?? null) : null}
                />
              );
              return (
                <div key={key} role="group" aria-labelledby={`inbox-${key}`} data-section={key}>
                  <h3 id={`inbox-${key}`} className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider">
                    <span className={`inline-flex items-center gap-1.5 ${sec.head}`}>
                      {sec.icon}
                      {sec.label}
                    </span>{' '}
                    <span className="normal-case tracking-normal font-normal text-zinc-400">· {count}</span>
                  </h3>
                  {count > 0 || withBatch ? (
                    <ul role="presentation" className="space-y-2">
                      {rows.slice(0, key === 'done' ? reviewsEnd : rows.length).map(row)}
                      {withBatch && (
                        <BatchCard
                          items={bItems}
                          selected={wide && selectedId === 'batch'}
                          focusable={firstFocusable === 'batch'}
                          wide={wide}
                          onPick={() => select('batch')}
                        />
                      )}
                      {key === 'done' && rows.slice(reviewsEnd).map(row)}
                    </ul>
                  ) : (
                    <p className="px-1 text-xs text-zinc-400">{sec.empty}</p>
                  )}
                  {key === 'done' && (
                    <p className="mt-3 px-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-zinc-400" data-done-footer>
                      <span>{DONE_FOOTER}</span>
                      {!weekShown && (
                        <button
                          type="button"
                          onClick={() => void showEarlier(now)}
                          disabled={loading}
                          className="kw-focus kw-hit inline-flex items-center gap-0.5 rounded font-semibold text-violet-300 hover:text-violet-200 disabled:opacity-60"
                          data-earlier
                        >
                          {EARLIER_LABEL} <ChevronRight size={13} aria-hidden="true" />
                        </button>
                      )}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );

  const openFromBatch = (id: string) => {
    const it = data?.items.find((i) => i.id === id) ?? bItems.find((i) => i.id === id);
    if (it) select(id);
  };

  const crumbBar = crumb ? (
    <button
      type="button"
      onClick={() => {
        setCrumb(null);
        onCrumb?.(crumb);
      }}
      className="kw-focus mb-3 inline-flex items-center gap-1 min-h-[32px] px-1.5 -ml-1.5 rounded-md text-xs text-zinc-300 hover:text-zinc-100"
      data-crumb={crumb}
    >
      <ChevronLeft size={14} aria-hidden="true" /> Back to {CRUMB_LABEL[crumb]}
    </button>
  ) : null;

  const paneBody =
    selectedId === 'batch' && selValid ? (
      <BatchPane wide={wide} onOpenItem={openFromBatch} />
    ) : selItem && opensInPane(selItem) ? (
      <ItemPane
        key={selItem.id}
        item={selItem}
        runItem={sections?.runFor[selItem.id] ?? null}
        entry={crumb ?? 'inbox'}
        onDone={onDone}
        onHeldResolved={(approvalId, status) => markHeld(selItem.id, approvalId, status)}
        onOpenItemChat={(i) => onOpen(i as AwayItem)}
        onOpenMessage={onOpenMessage}
        onEditRoutine={onEditRoutine}
        placeName={placeName}
        api={doneApi}
        initial={doneDetails?.[selItem.id]}
      />
    ) : empty ? (
      <div className="h-full grid place-items-center text-center" data-pane-empty>
        <div className="space-y-1.5">
          <CheckCircle2 size={26} className="mx-auto text-teal-300" aria-hidden="true" />
          <p className="text-sm font-semibold text-zinc-100">Nothing to review</p>
          <p className="text-xs text-zinc-400">Approved and sent-back items leave the Inbox.</p>
        </div>
      </div>
    ) : null;

  const paneLabel = selectedId === 'batch' ? 'Low risk, checks passed' : selItem ? selItem.title : 'Item';

  if (wide) {
    return (
      <div className="flex-1 min-w-0 flex h-full bg-zinc-950 overflow-hidden" data-inbox-new="wide">
        <div className="w-[392px] shrink-0 flex flex-col border-r border-zinc-800/80 min-h-0">
          {header}
          {queue}
        </div>
        <div ref={paneRef} role="region" aria-label={paneLabel} className={`flex-1 min-w-0 overflow-y-auto px-6 pt-4 ${error ? 'opacity-60' : ''}`} data-item-pane>
          <div className="max-w-[640px] min-h-full flex flex-col pb-4">
            {crumbBar}
            <div className="flex-1">{paneBody}</div>
          </div>
        </div>
      </div>
    );
  }

  // Phone and narrow windows: the queue, or the open item as a full page.
  const open = selValid && selectedId && (selectedId === 'batch' || selItem);
  const position = (() => {
    if (!sections || !selItem || crumb) return null;
    for (const key of ['needs', 'done'] as const) {
      const rows = sections[key].filter((i) => opensInPane(i));
      const at = rows.findIndex((i) => i.id === selItem.id);
      if (at >= 0) return `${SECTION[key].short} · ${at + 1} of ${rows.length}`;
    }
    return null;
  })();
  if (open) {
    const back = () => {
      if (crumb) {
        setCrumb(null);
        select(null);
        onCrumb?.(crumb);
      } else select(null);
    };
    return (
      <div className="flex-1 min-w-0 flex flex-col h-full bg-zinc-950 overflow-hidden" data-inbox-new="item">
        <div className="h-14 shrink-0 border-b border-zinc-800/80 px-2 flex items-center gap-1">
          <button type="button" onClick={back} className="kw-focus inline-flex items-center gap-1 min-h-[44px] px-2 rounded-lg text-sm text-zinc-200 hover:bg-zinc-800" data-item-back>
            <ChevronLeft size={18} aria-hidden="true" /> {crumb ? CRUMB_LABEL[crumb] : 'Inbox'}
          </button>
          {position && (
            <span className="ml-auto pr-2 text-xs text-zinc-400 tabular-nums" data-item-position>
              {position}
            </span>
          )}
        </div>
        <div role="region" aria-label={paneLabel} className="flex-1 overflow-y-auto px-4 pt-4 flex flex-col" data-item-pane>
          <div className="flex-1 flex flex-col pb-4">
            {status}
            <div className="flex-1 mt-2">{paneBody}</div>
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="flex-1 min-w-0 flex flex-col h-full bg-zinc-950 overflow-hidden" data-inbox-new="list">
      {header}
      {queue}
    </div>
  );
};
