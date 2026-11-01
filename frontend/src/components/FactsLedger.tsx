// Truth Ledger on the board (Phase C2): disputes first, as Needs you (keep one fact or mark them as
// not a conflict), then the facts bots read, labelled when unconfirmed or disputed, each with where it
// came from and which tasks rely on it. Renders nothing unless the ledger is `on`: with it off or in
// shadow (or on a server without it) the board looks as before. Live through `fact_changed` and
// `fact_dispute`.

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, BookOpenCheck, Check, ChevronDown, Loader2, Scale } from 'lucide-react';
import { MissionError, ago, factsApi, newIdempotencyKey, onMission } from '../missions';
import {
  Dispute,
  Fact,
  FactDetail,
  FactFilter,
  LedgerSetting,
  causeText,
  citationText,
  disputeConflictText,
  disputeOutcome,
  factConflictText,
  factLabel,
  filterFacts,
  scopeText,
  sortFacts,
  sourceText,
} from '../truthLedger';

export type LedgerApi = Pick<typeof factsApi, 'list' | 'disputes' | 'get' | 'resolve' | 'dismiss' | 'accept' | 'reject'>;

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';
const btn = `inline-flex items-center justify-center gap-1 px-3 min-h-[40px] sm:min-h-[32px] rounded-lg text-xs font-medium disabled:opacity-50 ${focusRing}`;
const TONE = {
  amber: 'border-amber-500/40 bg-amber-500/10 text-amber-200',
  rose: 'border-rose-500/40 bg-rose-500/10 text-rose-200',
  zinc: 'border-zinc-700 bg-zinc-800 text-zinc-400',
};

export const FactBadge: React.FC<{ status: string }> = ({ status }) => {
  const l = factLabel(status);
  if (!l) return null;
  return (
    <span className={`inline-flex shrink-0 items-center rounded-full border px-1.5 py-px text-[10px] font-medium ${TONE[l.tone]}`} data-fact-label={status}>
      {l.text}
    </span>
  );
};

const errText = (e: unknown, fallback: string) => (e instanceof Error && e.message ? e.message : fallback);

/** One open (or just decided) dispute: both facts side by side, keep one, or not a conflict. */
export const DisputeCard: React.FC<{
  dispute: Dispute;
  names: Record<string, string>;
  teams?: Record<string, string>;
  now?: number;
  busy?: boolean;
  error?: string | null;
  onKeep: (d: Dispute, winner: Fact) => void;
  onDismiss: (d: Dispute) => void;
}> = ({ dispute: d, names, teams = {}, now = Date.now(), busy = false, error = null, onKeep, onDismiss }) => {
  const id = useId();
  const sides = d.facts ?? [];
  const open = d.status === 'open';
  return (
    <li
      aria-labelledby={`${id}-t`}
      className={`rounded-xl border px-3 py-2.5 space-y-2 ${open ? 'border-amber-500/30 bg-amber-500/5' : 'border-zinc-800 bg-zinc-900/40'}`}
      data-dispute={d.id}
      data-status={d.status}
    >
      <div className="flex items-start gap-2">
        <Scale size={16} className={`mt-0.5 shrink-0 ${open ? 'text-amber-300' : 'text-zinc-500'}`} aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p id={`${id}-t`} className="text-xs font-medium text-zinc-100">
            Two facts disagree
          </p>
          <p className="text-[11px] text-zinc-400">
            {causeText(d.cause)} <span className="text-zinc-600">· {ago(d.created_at, now)}</span>
          </p>
        </div>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        {[0, 1].map((i) => {
          const f = sides[i];
          const won = !open && d.winner && f?.id === d.winner;
          return (
            <div key={i} className={`rounded-lg border px-2.5 py-2 space-y-1.5 ${won ? 'border-emerald-500/40 bg-emerald-500/5' : 'border-zinc-800 bg-zinc-950/50'}`} data-side={i === 0 ? 'a' : 'b'}>
              {f ? (
                <>
                  <p className="text-xs text-zinc-100 break-words">
                    <span className="sr-only">{i === 0 ? 'First fact: ' : 'Second fact: '}</span>“{f.statement}”
                  </p>
                  <p className="text-[11px] text-zinc-500 break-words">
                    {scopeText(f, names, teams)} · {sourceText(f, names)} · {ago(f.created_at, now)}
                  </p>
                  {open && (
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => onKeep(d, f)}
                      aria-label={`Keep “${f.statement}” and replace the other`}
                      className={`${btn} w-full sm:w-auto border border-emerald-500/40 text-emerald-200 hover:bg-emerald-500/10`}
                      data-keep={f.id}
                    >
                      {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={12} aria-hidden="true" />}
                      Keep this one
                    </button>
                  )}
                </>
              ) : (
                <p className="text-[11px] text-zinc-500">This fact is gone.</p>
              )}
            </div>
          );
        })}
      </div>
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-[11px] text-rose-300" data-dispute-error>
          <AlertTriangle size={12} className="mt-px shrink-0" aria-hidden="true" /> <span>{error}</span>
        </p>
      )}
      {open ? (
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" disabled={busy} onClick={() => onDismiss(d)} className={`${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700`} data-dismiss>
            Not a conflict
          </button>
          <span className="text-[10px] text-zinc-500">Until you choose, bots see both, labelled disputed.</span>
        </div>
      ) : (
        <p className={`text-[11px] ${d.status === 'resolved' ? 'text-emerald-300' : 'text-zinc-400'}`} data-outcome={d.status}>
          {disputeOutcome(d, sides)}
        </p>
      )}
    </li>
  );
};

/** A fact's details, loaded when opened: which tasks rely on it, its evidence and history; confirm or reject if unconfirmed. */
const FactDetails: React.FC<{
  fact: Fact;
  api: LedgerApi;
  taskTitles: Record<string, string>;
  onOpenTask?: (id: string) => void;
  now: number;
  version: number;
  onChanged: () => void;
}> = ({ fact, api, taskTitles, onOpenTask, now, version, onChanged }) => {
  const [detail, setDetail] = useState<FactDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const keys = useRef<Record<string, string>>({});
  const load = useCallback(() => {
    let live = true;
    // A refresh keeps any message (a 409 explains itself); Try again clears it first.
    api.get(fact.id).then(
      (d) => live && setDetail(d),
      (e) => live && setError(errText(e, 'Couldn’t load this fact.')),
    );
    return () => {
      live = false;
    };
  }, [api, fact.id]);
  useEffect(load, [load, version]);

  const decide = async (accept: boolean) => {
    const k = accept ? 'accept' : 'reject';
    const key = (keys.current[k] ??= newIdempotencyKey());
    setBusy(true);
    setError(null);
    try {
      setDetail(await (accept ? api.accept : api.reject)(fact.id, key));
      setNote(accept ? 'Confirmed: bots now read it without the label.' : 'Rejected: bots no longer read it.');
      onChanged();
    } catch (e) {
      setError(e instanceof MissionError && e.status === 409 ? factConflictText(e) : errText(e, 'Couldn’t save that. Try again.'));
      if (e instanceof MissionError && e.status === 409) {
        load();
        onChanged();
      }
    } finally {
      setBusy(false);
    }
  };

  if (!detail && !error)
    return (
      <p className="flex items-center gap-1.5 text-[11px] text-zinc-400">
        <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
      </p>
    );
  const f = detail?.fact ?? fact;
  const tasks = (detail?.citations ?? []).filter((c) => c.cited_by_kind === 'task');
  return (
    <div className="space-y-1.5 text-[11px]" data-fact-detail={fact.id}>
      {detail && (
        <>
          <p className="text-zinc-300" data-citations={tasks.length}>
            {citationText(detail.citations)}
          </p>
          {tasks.length > 0 && (
            <ul className="space-y-0.5" aria-label="Tasks that rely on it">
              {tasks.slice(0, 8).map((c) => {
                const title = taskTitles[c.cited_by_id] ?? 'A task';
                return (
                  <li key={c.cited_by_id}>
                    {onOpenTask ? (
                      <button type="button" onClick={() => onOpenTask(c.cited_by_id)} className={`text-left text-sky-300 hover:underline rounded break-words ${focusRing}`}>
                        {title}
                      </button>
                    ) : (
                      <span className="text-zinc-400">{title}</span>
                    )}
                    <span className="text-zinc-600"> · since {ago(c.created_at, now)}</span>
                  </li>
                );
              })}
            </ul>
          )}
          {detail.evidence.length > 0 && (
            <p className="text-zinc-400">
              Backed by {detail.evidence.length} piece{detail.evidence.length === 1 ? '' : 's'} of evidence
              {detail.evidence.some((e) => e.check_id) ? ', including a check' : ''}.
            </p>
          )}
          {f.valid_until && (
            <p className="text-zinc-500">
              {Date.parse(f.valid_until) > now ? `Bots stop reading it on ${new Date(f.valid_until).toLocaleDateString()}.` : `Bots stopped reading it ${ago(f.valid_until, now)}.`}
            </p>
          )}
        </>
      )}
      {f.status === 'proposed' && (
        <div className="flex flex-wrap items-center gap-2 pt-0.5" role="group" aria-label={`Unconfirmed: ${f.statement}`}>
          <button type="button" disabled={busy} onClick={() => void decide(true)} className={`${btn} bg-emerald-600/90 text-white hover:bg-emerald-500`} data-confirm-fact>
            {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={12} aria-hidden="true" />} Confirm
          </button>
          <button type="button" disabled={busy} onClick={() => void decide(false)} className={`${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700`} data-reject-fact>
            Reject
          </button>
        </div>
      )}
      <p aria-live="polite" className="text-emerald-300 min-h-0">
        {note}
      </p>
      {error && (
        <p role="alert" className="flex flex-wrap items-center gap-2 text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
          {!detail && (
            <button
              type="button"
              onClick={() => {
                setError(null);
                load();
              }}
              className={`underline hover:text-rose-200 rounded ${focusRing}`}
            >
              Try again
            </button>
          )}
        </p>
      )}
    </div>
  );
};

const FactRow: React.FC<{
  fact: Fact;
  names: Record<string, string>;
  teams: Record<string, string>;
  taskTitles: Record<string, string>;
  onOpenTask?: (id: string) => void;
  api: LedgerApi;
  now: number;
  version: number;
  onChanged: () => void;
}> = ({ fact: f, names, teams, taskTitles, onOpenTask, api, now, version, onChanged }) => {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <li className="px-2.5 py-2 space-y-1.5" data-fact={f.id} data-status={f.status}>
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <p className={`text-xs break-words ${f.status === 'superseded' ? 'text-zinc-500 line-through' : 'text-zinc-100'}`}>
            {f.statement} <FactBadge status={f.status} />
          </p>
          <p className="text-[11px] text-zinc-500 break-words">
            {scopeText(f, names, teams)} · {sourceText(f, names)} · {ago(f.created_at, now)}
          </p>
        </div>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={`${id}-d`}
          onClick={() => setOpen((v) => !v)}
          className={`shrink-0 inline-flex items-center gap-0.5 px-2 min-h-[40px] sm:min-h-[28px] rounded text-[11px] text-sky-300 hover:text-sky-200 ${focusRing}`}
          data-fact-toggle
        >
          {open ? 'Less' : 'Details'}
          <span className="sr-only"> about “{f.statement}”</span>
          <ChevronDown size={12} className={open ? 'rotate-180' : ''} aria-hidden="true" />
        </button>
      </div>
      {open && (
        <div id={`${id}-d`} className="rounded-lg bg-zinc-950/60 px-2.5 py-2">
          <FactDetails fact={f} api={api} taskTitles={taskTitles} onOpenTask={onOpenTask} now={now} version={version} onChanged={onChanged} />
        </div>
      )}
    </li>
  );
};

const FILTERS: Array<{ id: FactFilter; label: string }> = [
  { id: 'all', label: 'In force' },
  { id: 'proposed', label: 'Unconfirmed' },
  { id: 'disputed', label: 'Disputed' },
];

/**
 * The board's Facts section. Loads its own state; tests pass `initial` (the setting, facts and open
 * disputes as loaded) and the API.
 */
export const FactsLedger: React.FC<{
  names: Record<string, string>;
  teams?: Record<string, string>;
  taskTitles?: Record<string, string>;
  onOpenTask?: (id: string) => void;
  now?: number;
  initial?: { setting: LedgerSetting; facts: Fact[]; disputes: Dispute[] };
  /** Open the facts list from the start (screenshots, a notification tap). */
  defaultOpen?: boolean;
  api?: LedgerApi;
}> = ({ names, teams = {}, taskTitles = {}, onOpenTask, now = Date.now(), initial, defaultOpen = false, api = factsApi }) => {
  const headId = useId();
  const listId = useId();
  const [setting, setSetting] = useState<LedgerSetting | null>(initial?.setting ?? null);
  const [facts, setFacts] = useState<Fact[]>(initial?.facts ?? []);
  const [disputes, setDisputes] = useState<Dispute[]>(initial?.disputes ?? []);
  const [decided, setDecided] = useState<Dispute[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);
  const [showList, setShowList] = useState(defaultOpen);
  const [filter, setFilter] = useState<FactFilter>('all');
  const [version, setVersion] = useState(0);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const keys = useRef<Record<string, string>>({});
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  const reload = useCallback(() => {
    setReloading(true);
    Promise.all([api.list(), api.disputes('open')]).then(
      ([l, d]) => {
        if (!alive.current) return;
        setSetting(l.setting);
        setFacts(l.facts);
        setDisputes(d.disputes);
        setLoadError(null);
        setReloading(false);
        setVersion((v) => v + 1);
      },
      (e) => {
        if (!alive.current) return;
        setReloading(false);
        // Before the first answer an error (an older server's 404) just means: show nothing.
        setLoadError(errText(e, 'Couldn’t load the facts.'));
      },
    );
  }, [api]);

  useEffect(() => {
    if (!initial) reload();
  }, [initial, reload]);

  // Live: a burst of changes (a dispute moves both facts and their tasks) reads once.
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | null = null;
    const soon = () => {
      if (t) clearTimeout(t);
      t = setTimeout(reload, 250);
    };
    const offA = onMission('fact_changed', soon);
    const offB = onMission('fact_dispute', soon);
    return () => {
      offA();
      offB();
      if (t) clearTimeout(t);
    };
  }, [reload]);

  const decide = async (d: Dispute, what: 'resolve' | 'dismiss', winner?: Fact) => {
    if (busyId) return;
    const k = `${what}:${d.id}:${winner?.id ?? ''}`;
    const key = (keys.current[k] ??= newIdempotencyKey());
    setBusyId(d.id);
    setErrors(({ [d.id]: _gone, ...rest }) => rest);
    try {
      const after = what === 'resolve' && winner ? await api.resolve(d.id, winner.id, key) : await api.dismiss(d.id, key);
      setDecided((cur) => [{ ...after, facts: after.facts ?? d.facts }, ...cur.filter((x) => x.id !== d.id)]);
      setDisputes((cur) => cur.filter((x) => x.id !== d.id));
      reload();
    } catch (e) {
      if (e instanceof MissionError && e.status === 409) {
        setErrors((x) => ({ ...x, [d.id]: disputeConflictText(e) }));
        // Show what holds now: the settled dispute with its outcome, from the full list.
        api.disputes('all').then(
          (r) => {
            if (!alive.current) return;
            const nowD = r.disputes.find((x) => x.id === d.id);
            if (nowD && nowD.status !== 'open') {
              setDecided((cur) => [nowD, ...cur.filter((x) => x.id !== d.id)]);
              setDisputes((cur) => cur.filter((x) => x.id !== d.id));
            }
          },
          () => {},
        );
        reload();
      } else {
        // Another press retries with the same key: a lost answer replays rather than repeats.
        setErrors((x) => ({ ...x, [d.id]: errText(e, 'Couldn’t save that. Try again.') }));
      }
    } finally {
      setBusyId(null);
    }
  };

  // Off, shadow, an older server, or still loading the first time: the board looks as before.
  if (setting !== 'on') return null;

  // One decided here stays shown with its outcome, even if a reload raced the decision.
  const decidedIds = new Set(decided.map((d) => d.id));
  const openDisputes = disputes.filter((d) => !decidedIds.has(d.id)).sort((a, b) => a.created_at.localeCompare(b.created_at));
  const shownDisputes = [...openDisputes, ...decided];
  const inForce = facts.filter((f) => f.status !== 'superseded');
  const unconfirmed = facts.filter((f) => f.status === 'proposed').length;
  const disputed = facts.filter((f) => f.status === 'disputed').length;
  const listed = sortFacts(filterFacts(facts, filter));

  return (
    <section aria-labelledby={headId} className="space-y-2 rounded-xl border border-zinc-800 bg-zinc-900/40 p-3" data-facts-ledger>
      <div className="flex flex-wrap items-center gap-2">
        <h3 id={headId} className="flex items-center gap-1.5 text-xs font-medium text-zinc-200">
          <BookOpenCheck size={14} className="text-sky-300" aria-hidden="true" /> Facts
        </h3>
        <p className="text-[11px] text-zinc-500" data-fact-counts>
          {inForce.length === 0 ? 'None saved yet' : `${inForce.length} in force`}
          {unconfirmed > 0 && <span className="text-amber-300"> · {unconfirmed} unconfirmed</span>}
          {disputed > 0 && <span className="text-rose-300"> · {disputed} disputed</span>}
        </p>
        {reloading && <Loader2 size={11} className="animate-spin motion-reduce:animate-none text-zinc-500" aria-label="Updating" />}
        <button
          type="button"
          aria-expanded={showList}
          aria-controls={listId}
          onClick={() => setShowList((v) => !v)}
          className={`ml-auto inline-flex items-center gap-0.5 px-2 min-h-[40px] sm:min-h-[28px] rounded text-[11px] text-sky-300 hover:text-sky-200 ${focusRing}`}
          data-facts-toggle
        >
          {showList ? 'Hide facts' : 'Show facts'}
          <ChevronDown size={12} className={showList ? 'rotate-180' : ''} aria-hidden="true" />
        </button>
      </div>

      {loadError && (
        <p role="alert" className="flex flex-wrap items-center gap-2 text-[11px] text-rose-300" data-facts-error>
          <AlertTriangle size={12} aria-hidden="true" /> {loadError}
          <button type="button" onClick={reload} className={`underline hover:text-rose-200 rounded ${focusRing}`}>
            Try again
          </button>
        </p>
      )}

      {shownDisputes.length > 0 && (
        <div className="space-y-2" data-disputes={openDisputes.length}>
          <h4 className="text-xs font-medium text-amber-200">
            Needs you: {openDisputes.length === 0 ? 'disputes' : openDisputes.length === 1 ? 'one dispute' : `${openDisputes.length} disputes`} to settle
          </h4>
          <ul className="space-y-2">
            {shownDisputes.map((d) => (
              <DisputeCard
                key={d.id}
                dispute={d}
                names={names}
                teams={teams}
                now={now}
                busy={busyId === d.id}
                error={errors[d.id] ?? null}
                onKeep={(x, w) => void decide(x, 'resolve', w)}
                onDismiss={(x) => void decide(x, 'dismiss')}
              />
            ))}
          </ul>
          {openDisputes.length === 0 && (
            <p className="text-[11px] text-zinc-500">
              All settled.{' '}
              <button type="button" onClick={() => setDecided([])} className={`underline hover:text-zinc-300 rounded ${focusRing}`}>
                Hide
              </button>
            </p>
          )}
        </div>
      )}

      {showList && (
        <div id={listId} className="space-y-2">
          <div role="group" aria-label="Show" className="flex flex-wrap gap-1.5">
            {FILTERS.map((x) => (
              <button
                key={x.id}
                type="button"
                aria-pressed={filter === x.id}
                onClick={() => setFilter(x.id)}
                className={`px-2.5 min-h-[36px] sm:min-h-[28px] rounded-full border text-[11px] ${focusRing} ${filter === x.id ? 'border-sky-500/60 bg-sky-500/15 text-sky-100' : 'border-zinc-700 text-zinc-400 hover:text-zinc-200'}`}
                data-filter={x.id}
              >
                {x.label}
              </button>
            ))}
          </div>
          {listed.length === 0 ? (
            <p className="text-[11px] text-zinc-500" data-facts-empty>
              {filter === 'all'
                ? 'No facts yet. When a bot saves a note or you write a shared fact, it shows here.'
                : filter === 'proposed'
                  ? 'Nothing unconfirmed.'
                  : 'Nothing disputed.'}
            </p>
          ) : (
            <ul className="divide-y divide-zinc-800 rounded-lg border border-zinc-800" aria-label="Facts">
              {listed.slice(0, 100).map((f) => (
                <FactRow key={f.id} fact={f} names={names} teams={teams} taskTitles={taskTitles} onOpenTask={onOpenTask} api={api} now={now} version={version} onChanged={reload} />
              ))}
            </ul>
          )}
          {listed.length > 100 && <p className="text-[11px] text-zinc-500">Showing the first 100 of {listed.length}.</p>}
          <p className="text-[10px] text-zinc-600">Bots read the facts in force; unconfirmed and disputed ones come with that label.</p>
        </div>
      )}
    </section>
  );
};

/** A task's needs-you about a fact: a fact it relied on changed (stop relying on it), or is disputed (settle it under Facts). */
export const FactNeedsYouAction: React.FC<{
  taskId: string;
  item: { kind: string; id: string; fact_id?: string };
  api?: Pick<LedgerApi, never> & { uncite: (id: string, taskId: string) => Promise<{ ok: boolean }> };
}> = ({ taskId, item, api = factsApi }) => {
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (item.kind === 'rests_on_disputed_fact')
    return (
      <span className="text-[11px] text-amber-200/80" data-settle-under-facts>
        Settle it under Facts on the board.
      </span>
    );
  if (item.kind !== 'fact_changed') return null;
  if (done)
    return (
      <span role="status" className="text-[11px] text-emerald-300" data-uncited>
        Done: this task no longer relies on it.
      </span>
    );
  return (
    <span className="inline-flex flex-wrap items-center gap-2">
      <button
        type="button"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          setError(null);
          try {
            await api.uncite(item.fact_id ?? item.id, taskId);
            setDone(true);
          } catch (e) {
            setError(errText(e, 'Couldn’t save that. Try again.'));
          } finally {
            setBusy(false);
          }
        }}
        className="px-2 py-1 rounded border border-amber-500/40 hover:bg-amber-500/10 min-h-[32px] disabled:opacity-50"
        data-uncite
      >
        {busy ? 'Saving…' : 'Stop relying on it'}
      </button>
      {error && (
        <span role="alert" className="text-[11px] text-rose-300">
          {error}
        </span>
      )}
    </span>
  );
};
