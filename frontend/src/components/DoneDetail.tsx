// 0.9.5 F: a done item's pane (mockup ux-audit-090/mockups/2-while-you-were-away.html). In order: the meta line
// (what ran, when, why), the chips, the answer inline with Open in chat, "What <bot> did", "Why", then the
// actions. A routine's review task draws these inside ReviewItemView (which keeps Looks good, Ask to redo and
// Roll back); a finished item with no review task draws DoneItemView. Text from the server is text only.

import React, { useEffect, useId, useState } from 'react';
import { AlertTriangle, ChevronRight, CornerUpLeft, Eye, Loader2 } from 'lucide-react';
import type { InboxItem } from '../api/inbox';
import { type Routine, type RoutineRun, continuityApi } from '../continuity';
import {
  type DoneMessage,
  answerText,
  changedState,
  doneTitle,
  findRoutine,
  findRun,
  isRoutineItem,
  metaLine,
  monthDay,
  noChecksSet,
  redoLabel,
  trailDropped,
  trailFrom,
} from '../doneAway';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ModernAvatar } from './ModernAvatar';

/** Where an answer lives, for Open in chat. */
export interface MessageTarget {
  message_id: string;
  channel_id: string | null;
  kwant_id: string;
  thread_id: string | null;
}

export interface DoneApi {
  routines: () => Promise<Routine[]>;
  runs: (routineId: string) => Promise<RoutineRun[]>;
  /** The result message, or null when it is gone. */
  message: (t: MessageTarget) => Promise<DoneMessage | null>;
}

async function getJson<T>(url: string): Promise<T | null> {
  const r = await fetch(url);
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return (await r.json()) as T;
}

export const defaultDoneApi: DoneApi = {
  routines: () => continuityApi.routines(),
  runs: (id) => continuityApi.routineRuns(id, 50),
  async message(t) {
    const enc = encodeURIComponent;
    const url = t.thread_id
      ? `/api/threads/${enc(t.thread_id)}/messages`
      : `/api/channels/${enc(t.channel_id ?? t.kwant_id)}/messages?around=${enc(t.message_id)}&limit=1`;
    const list = await getJson<DoneMessage[]>(url);
    return (Array.isArray(list) ? list : []).find((m) => m && m.id === t.message_id) ?? null;
  },
};

export interface DoneDetailData {
  loading: boolean;
  error: string | null;
  routine: Routine | null;
  run: RoutineRun | null;
  message: DoneMessage | null;
  target: MessageTarget | null;
}

const EMPTY: DoneDetailData = {
  loading: false,
  error: null,
  routine: null,
  run: null,
  message: null,
  target: null,
};

/**
 * Loads what today's API has for a done item: its routine (by bot and name), the run (by id for a run row,
 * else the one that ended closest), and the run's result message (finish_routine_run saves its id).
 */
export function useDoneDetail(item: InboxItem, runItem: InboxItem | null, api: DoneApi = defaultDoneApi, initial?: DoneDetailData): DoneDetailData {
  const routineLike = isRoutineItem(item);
  const [d, setD] = useState<DoneDetailData>(initial ?? (routineLike ? { ...EMPTY, loading: true } : EMPTY));
  const runId = item.kind === 'routine_run' ? item.id : (runItem?.id ?? null);
  useEffect(() => {
    if (initial || !routineLike) return;
    let cancelled = false;
    setD({ ...EMPTY, loading: true });
    (async () => {
      const routine = findRoutine(await api.routines(), item);
      if (!routine) return { ...EMPTY };
      const run = findRun(await api.runs(routine.id), runId, runItem?.timestamp ?? item.timestamp);
      const target: MessageTarget | null = run?.message_id
        ? {
            message_id: run.message_id,
            channel_id: routine.channel_id,
            kwant_id: routine.kwant_id,
            thread_id: routine.thread_id ?? null,
          }
        : null;
      const message = target ? await api.message(target).catch(() => null) : null;
      return { loading: false, error: null, routine, run, message, target };
    })().then(
      (v) => !cancelled && setD(v),
      (e) =>
        !cancelled &&
        setD({
          ...EMPTY,
          error: e instanceof Error ? e.message : 'Something went wrong.',
        }),
    );
    return () => {
      cancelled = true;
    };
    // item.id and runId name the item; the rest is read once per item.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item.id, runId, routineLike, initial]);
  return d;
}

const label = 'text-[11px] font-semibold uppercase tracking-[0.06em] text-zinc-400';
const linkBtn = 'kw-focus kw-hit inline-flex items-center gap-0.5 rounded text-meta font-semibold text-violet-300 hover:text-violet-200';

/** "Read only · changed nothing" and "No checks set · Add one". */
export const DoneChips: React.FC<{
  item: InboxItem;
  d: DoneDetailData;
  onAddCheck?: () => void;
}> = ({ item, d, onAddCheck }) => {
  const changed = changedState(item, d.message);
  const noChecks = noChecksSet(item);
  if (changed !== false && !noChecks) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5" data-done-chips>
      {changed === false && (
        <span className="kw-chip !py-0 border-teal-300/30 bg-teal-300/[0.08] text-teal-200" data-chip="read-only">
          <Eye size={12} aria-hidden="true" /> Read only · changed nothing
        </span>
      )}
      {noChecks && (
        <span className="kw-chip !py-0 border-amber-300/30 bg-amber-300/[0.08] text-amber-200" data-chip="no-checks">
          No checks set
          {onAddCheck && (
            <>
              {' · '}
              <button
                type="button"
                onClick={onAddCheck}
                className="kw-focus rounded font-semibold underline underline-offset-2 hover:text-amber-100"
                data-add-check
              >
                Add one
              </button>
            </>
          )}
        </span>
      )}
    </div>
  );
};

/** The answer, "What <bot> did" and "Why", in the mockup's order. */
export const DoneSections: React.FC<{
  item: InboxItem;
  d: DoneDetailData;
  place: string | null;
  onOpenChat?: () => void;
  onEditRoutine?: (id: string) => void;
}> = ({ item, d, place, onOpenChat, onEditRoutine }) => {
  const uid = useId();
  const bot = item.kwant_name;
  const text = answerText(d.message) ?? (item.summary?.trim() ? item.summary : null);
  const fromMessage = !!answerText(d.message);
  const trail = trailFrom(d.message, place);
  const dropped = trailDropped(d.message);
  const routine = d.routine;
  const routineLike = isRoutineItem(item);
  return (
    <>
      <section aria-labelledby={`${uid}-ans`} className="rounded-2xl border border-hairline bg-surface-raised p-4 space-y-2 min-w-0" data-done-answer>
        <h3 id={`${uid}-ans`} className={label}>
          {bot}’s answer
        </h3>
        {d.loading && !text ? (
          <p className="inline-flex items-center gap-1.5 text-meta text-zinc-400" role="status">
            <Loader2 size={13} className="motion-safe:animate-spin" aria-hidden="true" /> Loading the answer…
          </p>
        ) : text ? (
          fromMessage ? (
            <div className="min-w-0 overflow-x-auto [&_.markdown-content]:text-body" data-answer-text>
              <MarkdownRenderer content={text} />
            </div>
          ) : (
            <p className="text-body text-zinc-100 whitespace-pre-wrap break-words" data-answer-text>
              {text}
            </p>
          )
        ) : (
          <p className="text-meta text-zinc-400">The answer isn’t here any more. Open the chat to see what is.</p>
        )}
        {onOpenChat && (
          <button type="button" onClick={onOpenChat} className={linkBtn} data-open-in-chat>
            Open in chat <ChevronRight size={14} aria-hidden="true" />
          </button>
        )}
      </section>

      {routineLike && (
        <section aria-labelledby={`${uid}-did`} className="space-y-2 min-w-0" data-done-trail>
          <h3 id={`${uid}-did`} className={label}>
            What {bot} did
          </h3>
          {trail.length ? (
            <ol className="space-y-1.5">
              {dropped > 0 && (
                <li className="text-meta text-zinc-400">
                  {dropped} earlier {dropped === 1 ? 'step' : 'steps'} not kept
                </li>
              )}
              {trail.map((r, i) => (
                <li key={i} className="grid grid-cols-[64px_minmax(0,1fr)] gap-x-2 text-body" data-trail-row>
                  <time className="tabular-nums text-meta text-zinc-400 pt-px">{r.at}</time>
                  <span className={`min-w-0 break-words ${r.failed ? 'text-rose-200' : 'text-zinc-100'}`}>
                    {r.failed && <AlertTriangle size={13} className="inline -mt-0.5 mr-1" aria-label="Failed" />}
                    {r.what}
                    {r.detail && <span className="text-zinc-400"> · {r.detail}</span>}
                  </span>
                </li>
              ))}
            </ol>
          ) : (
            <p className="text-meta text-zinc-400" data-trail-empty>
              {d.loading ? 'Loading…' : 'This run kept no steps. Open the chat to see the whole reply.'}
            </p>
          )}
        </section>
      )}

      {routine && (
        <section aria-labelledby={`${uid}-why`} className="space-y-2 min-w-0" data-done-why>
          <h3 id={`${uid}-why`} className={label}>
            Why
          </h3>
          <p className="text-body text-zinc-300 break-words">
            You set up this routine
            {monthDay(routine.created_at) ? ` on ${monthDay(routine.created_at)}` : ''}:{' '}
            <q className="text-zinc-100 [quotes:'“'_'”']" data-why-quote>
              {routine.instructions}
            </q>{' '}
            {onEditRoutine && (
              <button type="button" onClick={() => onEditRoutine(routine.id)} className={linkBtn} data-edit-routine>
                Edit routine
              </button>
            )}
          </p>
        </section>
      )}
      {d.error && (
        <p className="text-meta text-zinc-400" role="status" data-done-error>
          Couldn’t load the run’s details. {d.error}
        </p>
      )}
    </>
  );
};

/** A done item with no review task: what ran and the answer; Ask to redo opens the chat. */
export const DoneItemView: React.FC<{
  item: InboxItem;
  d: DoneDetailData;
  place: string | null;
  hideTitle?: boolean;
  phone?: boolean;
  onOpenChat?: () => void;
  onEditRoutine?: (id: string) => void;
}> = ({ item, d, place, hideTitle = false, phone = false, onOpenChat, onEditRoutine }) => {
  const uid = useId();
  const title = doneTitle(item);
  return (
    <section role="region" aria-labelledby={`${uid}-t`} className="space-y-4 min-w-0" data-done-item={item.id}>
      <header className="flex items-start gap-3 min-w-0">
        {!hideTitle && <ModernAvatar role={item.kwant_name} name={item.kwant_name} size={34} kwantId={item.kwant_id} />}
        <div className="min-w-0 flex-1 space-y-1">
          <h2 id={`${uid}-t`} className={hideTitle ? 'sr-only' : 'text-[16px] leading-snug font-[650] text-zinc-50 break-words'}>
            {title}
          </h2>
          <p className="text-meta text-zinc-400 break-words" data-done-meta>
            {metaLine(item, d.routine, d.run)}
          </p>
        </div>
      </header>
      <DoneChips item={item} d={d} onAddCheck={d.routine && onEditRoutine ? () => onEditRoutine(d.routine!.id) : undefined} />
      <DoneSections item={item} d={d} place={place} onOpenChat={onOpenChat} onEditRoutine={onEditRoutine} />
      {onOpenChat && (
        <div className={phone ? 'grid gap-2' : 'flex flex-wrap gap-2'} data-actions>
          <button
            type="button"
            onClick={onOpenChat}
            className="kw-focus inline-flex items-center justify-center gap-1.5 rounded-xl px-3.5 min-h-[36px] max-md:min-h-[44px] text-meta font-semibold border border-hairline-strong bg-white/[0.03] text-zinc-200 hover:bg-white/[0.08]"
            data-redo
          >
            <CornerUpLeft size={14} aria-hidden="true" /> {redoLabel(item.kwant_name)}
          </button>
        </div>
      )}
    </section>
  );
};
