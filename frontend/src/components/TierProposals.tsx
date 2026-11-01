// Flight School scorecards and proposals (Phase C4, docs/mission-control.md §14). A bot that has
// earned the next tier asks for it; only the person grants or declines, and revoking stays one tap
// in the bot's settings. A grant or decline that another tab or device got to first (409) says so
// and shows what holds now. With Flight School off the server has no open proposals, so nothing shows.

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Check, GraduationCap, Loader2, X } from 'lucide-react';
import {
  ProposalDecision,
  Scorecard,
  TierProposal,
  classLabel,
  isOpenProposal,
  openProposals,
  proposalChange,
  proposalConflictText,
  proposalOutcome,
  proposalTitle,
  scorecardRows,
  scorecardStatus,
  upsertProposal,
} from '../flightSchool';
import { MissionError, ago, capabilitiesApi, newIdempotencyKey, onMission } from '../missions';

export interface ProposalsApi {
  proposals: typeof capabilitiesApi.proposals;
  grant: (id: string, key: string) => Promise<ProposalDecision>;
  decline: (id: string, key: string, note?: string) => Promise<ProposalDecision>;
}

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';
const btn = `inline-flex items-center justify-center gap-1 px-3 min-h-[40px] sm:min-h-[32px] rounded-lg text-xs font-medium disabled:opacity-50 ${focusRing}`;

/** A scorecard's numbers as a small definition list, with what it can ask for under them. */
export const ScorecardView: React.FC<{ card: Partial<Scorecard>; compact?: boolean }> = ({ card, compact = false }) => {
  const rows = scorecardRows(card);
  const status = card.tier && card.missing ? scorecardStatus(card as Scorecard) : null;
  return (
    <div className="space-y-1.5" data-scorecard={card.task_class ?? ''}>
      {rows.length === 0 ? (
        <p className="text-[11px] text-zinc-500">No settled work in this kind yet.</p>
      ) : (
        <dl className={`grid gap-x-3 gap-y-0.5 text-[11px] ${compact ? 'grid-cols-[auto_1fr]' : 'grid-cols-[auto_1fr] sm:grid-cols-[auto_1fr_auto_1fr]'}`}>
          {rows.map((r) => (
            <React.Fragment key={r.label}>
              <dt className="text-zinc-500">{r.label}</dt>
              <dd className={`tabular-nums ${r.bad ? 'text-rose-300' : 'text-zinc-200'}`}>{r.value}</dd>
            </React.Fragment>
          ))}
        </dl>
      )}
      {status && (
        <p className={`text-[11px] ${status.ready ? 'text-emerald-300' : 'text-zinc-400'}`} data-scorecard-status={status.ready ? 'ready' : 'not-yet'}>
          {status.text}
        </p>
      )}
      {card.window_days ? <p className="text-[10px] text-zinc-600">From the board’s own records of the last {card.window_days} days.</p> : null}
    </div>
  );
};

/**
 * One proposal: who asks for what and why, its scorecard when it asked, and Grant / Decline. A
 * decided one shows what happened instead of the buttons.
 */
export const ProposalCard: React.FC<{
  proposal: TierProposal;
  bot: string;
  now?: number;
  busy?: boolean;
  /** Set while this card's decision failed or someone else decided first. */
  error?: string | null;
  onGrant: (p: TierProposal) => void;
  onDecline: (p: TierProposal, note: string) => void;
}> = ({ proposal: p, bot, now = Date.now(), busy = false, error = null, onGrant, onDecline }) => {
  const id = useId();
  const [declining, setDeclining] = useState(false);
  const [note, setNote] = useState('');
  const noteRef = useRef<HTMLInputElement>(null);
  const open = isOpenProposal(p);
  useEffect(() => {
    if (declining) noteRef.current?.focus();
  }, [declining]);
  const submitDecline = () => {
    if (!busy) onDecline(p, note.trim());
  };
  // No <form>: in a bot's settings this card sits inside the Edit bot form, and forms can't nest.
  const onNoteKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      setDeclining(false);
    } else if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submitDecline();
    }
  };
  return (
    <li
      aria-labelledby={`${id}-t`}
      className={`rounded-xl border px-3 py-2.5 space-y-2 ${open ? 'border-amber-500/30 bg-amber-500/5' : 'border-zinc-800 bg-zinc-900/40'}`}
      data-proposal={p.id}
      data-status={p.status}
    >
      <div className="flex items-start gap-2">
        <GraduationCap size={16} className={`mt-0.5 shrink-0 ${open ? 'text-amber-300' : 'text-zinc-500'}`} aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p id={`${id}-t`} className="text-xs font-medium text-zinc-100 break-words">
            {proposalTitle(p, bot)}
          </p>
          <p className="text-[11px] text-zinc-400">
            {proposalChange(p)} <span className="text-zinc-600">· asked {ago(p.created_at, now)}</span>
          </p>
        </div>
      </div>

      {p.reason && (
        <blockquote className="border-l-2 border-zinc-700 pl-2 text-[11px] text-zinc-300 break-words">
          <span className="sr-only">{bot} says: </span>
          {p.reason}
        </blockquote>
      )}

      {p.scorecard && (
        <details className="text-[11px]">
          <summary className={`cursor-pointer text-zinc-400 hover:text-zinc-200 rounded ${focusRing}`}>Its scorecard when it asked</summary>
          <div className="mt-1.5">
            <ScorecardView card={{ ...p.scorecard, task_class: p.task_class }} />
          </div>
        </details>
      )}

      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-[11px] text-rose-300" data-proposal-error>
          <AlertTriangle size={12} className="mt-px shrink-0" aria-hidden="true" /> <span>{error}</span>
        </p>
      )}

      {!open ? (
        <p className={`text-[11px] ${p.status === 'granted' ? 'text-emerald-300' : 'text-zinc-400'}`} data-outcome={p.status}>
          {proposalOutcome(p, bot)}
        </p>
      ) : declining ? (
        <div role="group" aria-label={`Decline: ${proposalTitle(p, bot)}`} className="flex flex-wrap items-center gap-2" data-decline-form>
          <label htmlFor={`${id}-n`} className="sr-only">
            Why not? Optional, {bot} sees it
          </label>
          <input
            ref={noteRef}
            id={`${id}-n`}
            value={note}
            maxLength={500}
            onChange={(e) => setNote(e.target.value)}
            onKeyDown={onNoteKey}
            placeholder="Why not? (optional)"
            className="flex-1 min-w-[10rem] bg-zinc-950 border border-zinc-800 rounded-lg px-2.5 min-h-[40px] sm:min-h-[32px] text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none"
          />
          <button type="button" disabled={busy} onClick={submitDecline} className={`${btn} border border-rose-500/40 text-rose-200 hover:bg-rose-500/10`} data-confirm-decline>
            {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null} Decline
          </button>
          <button type="button" disabled={busy} onClick={() => setDeclining(false)} className={`${btn} text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800`}>
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2" role="group" aria-label={`Decide: ${proposalTitle(p, bot)}`}>
          <button type="button" disabled={busy} onClick={() => onGrant(p)} className={`${btn} bg-emerald-600/90 text-white hover:bg-emerald-500`} data-grant>
            {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Check size={12} aria-hidden="true" />}
            Grant {p.to_tier === 'act' ? 'Act' : p.to_tier === 'sandbox' ? 'Sandbox' : 'Observe'}
          </button>
          <button type="button" disabled={busy} onClick={() => setDeclining(true)} className={`${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700`} data-decline>
            <X size={12} aria-hidden="true" /> Decline…
          </button>
          <span className="text-[10px] text-zinc-500">Payments always ask you, whatever the tier.</span>
        </div>
      )}
    </li>
  );
};

/**
 * Grant/decline with one key per proposal and press, conflict handling and the decided proposal
 * kept on screen. Shared by the board's Needs you list and the bot's settings.
 */
export function useProposalDecisions(api: ProposalsApi, onDecided: (p: TierProposal, d: ProposalDecision | null) => void) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const keys = useRef<Record<string, string>>({});
  const decide = useCallback(
    async (p: TierProposal, run: (key: string) => Promise<ProposalDecision>, what: 'grant' | 'decline') => {
      if (busyId) return;
      const k = `${what}:${p.id}`;
      const key = (keys.current[k] ??= newIdempotencyKey());
      setBusyId(p.id);
      setErrors(({ [p.id]: _gone, ...rest }) => rest);
      try {
        onDecided(p, await run(key));
      } catch (e) {
        if (e instanceof MissionError && e.status === 409) {
          setErrors((x) => ({ ...x, [p.id]: proposalConflictText(e.code, e.message) }));
          onDecided(p, null);
        } else {
          // Another press is a new try with the same key: a lost answer replays, not repeats.
          setErrors((x) => ({ ...x, [p.id]: e instanceof Error ? e.message : 'Couldn’t save that. Try again.' }));
        }
      } finally {
        setBusyId(null);
      }
    },
    [busyId, onDecided],
  );
  return {
    busyId,
    errors,
    grant: (p: TierProposal) => decide(p, (key) => api.grant(p.id, key), 'grant'),
    decline: (p: TierProposal, note: string) => decide(p, (key) => api.decline(p.id, key, note || undefined), 'decline'),
  };
}

/**
 * The board's Needs-you list of tier requests. Loads its own list, keeps it live, and shows nothing
 * when no bot is asking (which is always the case with Flight School off).
 */
export const TierRequests: React.FC<{
  names: Record<string, string>;
  now?: number;
  /** Tests pass the loaded list and the API. */
  initial?: TierProposal[];
  api?: ProposalsApi;
}> = ({ names, now, initial, api = capabilitiesApi }) => {
  const headId = useId();
  const [list, setList] = useState<TierProposal[]>(initial ?? []);
  // Decided here this session: kept on screen with their outcome until the board reloads.
  const [decided, setDecided] = useState<TierProposal[]>([]);
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  const reload = useCallback(() => {
    api.proposals('proposed').then(
      (ps) => alive.current && setList(ps),
      // An older server (404) or Flight School off: nothing to ask about.
      () => {},
    );
  }, [api]);

  useEffect(() => {
    if (initial === undefined) reload();
    return onMission('capability_changed', ({ capability }) => {
      const p = capability.proposal;
      if (p?.id) {
        setList((cur) => (isOpenProposal(p) ? upsertProposal(cur, p) : cur.filter((x) => x.id !== p.id)));
        if (!isOpenProposal(p)) setDecided((cur) => (cur.some((x) => x.id === p.id) ? cur.map((x) => (x.id === p.id ? p : x)) : cur));
      } else reload();
    });
  }, [initial, reload]);

  const onDecided = useCallback(
    (p: TierProposal, d: ProposalDecision | null) => {
      if (d) {
        setList((cur) => cur.filter((x) => x.id !== p.id));
        setDecided((cur) => [d.proposal, ...cur.filter((x) => x.id !== p.id)]);
      } else {
        // 409: someone else decided. Read what holds now; the error stays on the card.
        setDecided((cur) => [{ ...p }, ...cur.filter((x) => x.id !== p.id)]);
        setList((cur) => cur.filter((x) => x.id !== p.id));
        api.proposals(undefined).then(
          (all) => {
            if (!alive.current) return;
            const now = all.find((x) => x.id === p.id);
            if (now) setDecided((cur) => cur.map((x) => (x.id === p.id ? now : x)));
            setList(openProposals(all));
          },
          () => {},
        );
      }
    },
    [api],
  );
  const { busyId, errors, grant, decline } = useProposalDecisions(api, onDecided);

  const open = openProposals(list);
  if (!open.length && !decided.length) return null;
  const bot = (p: TierProposal) => names[p.kwant_id] ?? 'A bot';
  return (
    <section aria-labelledby={headId} className="space-y-2" data-tier-requests={open.length}>
      <h3 id={headId} className="flex items-center gap-1.5 text-xs font-medium text-amber-200">
        <GraduationCap size={14} aria-hidden="true" /> Needs you: {open.length === 1 ? 'a bot asks' : open.length > 1 ? `${open.length} bots ask` : 'tier requests'} for more freedom
      </h3>
      <ul className="grid gap-2 md:grid-cols-2">
        {[...open, ...decided].map((p) => (
          <ProposalCard key={p.id} proposal={p} bot={bot(p)} now={now} busy={busyId === p.id} error={errors[p.id] ?? null} onGrant={grant} onDecline={decline} />
        ))}
      </ul>
      {open.length === 0 && (
        <p className="text-[11px] text-zinc-500" data-requests-done>
          Nothing else is waiting.{' '}
          <button type="button" onClick={() => setDecided([])} className={`underline hover:text-zinc-300 rounded ${focusRing}`}>
            Hide
          </button>
        </p>
      )}
    </section>
  );
};

/** Keeps a bot's scorecard for one kind of work, loaded when first shown. */
export function useScorecard(kwantId: string, taskClass: string, shown: boolean, api: Pick<typeof capabilitiesApi, 'scorecard'> = capabilitiesApi) {
  const [card, setCard] = useState<Scorecard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    let live = true;
    api
      .scorecard(kwantId, taskClass)
      .then((c) => live && setCard(c))
      .catch((e) => live && setError(e instanceof Error ? e.message : 'Couldn’t load the scorecard.'))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [api, kwantId, taskClass]);
  useEffect(() => {
    if (shown) return load();
  }, [shown, load]);
  return { card, error, loading, reload: load };
}

export const ScorecardDisclosure: React.FC<{
  kwantId: string;
  taskClass: string;
  api?: Pick<typeof capabilitiesApi, 'scorecard'>;
  /** Tests open it with a loaded card. */
  initial?: Scorecard;
  /** Bumped by live updates so an open scorecard reads again. */
  version?: number;
}> = ({ kwantId, taskClass, api, initial, version = 0 }) => {
  const [shown, setShown] = useState(Boolean(initial));
  const id = useId();
  const { card, error, loading, reload } = useScorecard(kwantId, taskClass, shown && !initial, api);
  useEffect(() => {
    if (shown && !initial && version > 0) reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);
  const c = initial ?? card;
  return (
    <div className="w-full">
      <button
        type="button"
        aria-expanded={shown}
        aria-controls={`${id}-c`}
        onClick={() => setShown((v) => !v)}
        className={`text-[11px] text-sky-300 hover:text-sky-200 rounded ${focusRing}`}
        data-scorecard-toggle={taskClass}
      >
        {shown ? 'Hide scorecard' : 'Scorecard'}
        <span className="sr-only"> for {classLabel(taskClass)}</span>
      </button>
      {shown && (
        <div id={`${id}-c`} className="mt-1.5 rounded-lg bg-zinc-950/60 px-2.5 py-2">
          {loading && !c ? (
            <p className="flex items-center gap-1.5 text-[11px] text-zinc-400">
              <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading the scorecard…
            </p>
          ) : error && !c ? (
            <p role="alert" className="flex flex-wrap items-center gap-2 text-[11px] text-rose-300">
              <AlertTriangle size={12} aria-hidden="true" /> {error}
              <button type="button" onClick={reload} className={`underline hover:text-rose-200 rounded ${focusRing}`}>
                Try again
              </button>
            </p>
          ) : c ? (
            <ScorecardView card={c} compact />
          ) : null}
        </div>
      )}
    </div>
  );
};
