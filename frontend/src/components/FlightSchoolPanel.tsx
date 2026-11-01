// A bot's Flight School tiers (Phase C3/C4, docs/mission-control.md §14): one tier for all its work,
// one per kind of work, where each comes from, a one-click Revoke, its scorecard per kind of work, the
// tiers it is asking for, and the recent changes. Only shown with Flight School on; on a server without
// it, or with it off, this renders nothing. Changes made elsewhere arrive live (`capability_changed`).

import React, { useCallback, useEffect, useId, useState } from 'react';
import { AlertTriangle, GraduationCap, Loader2 } from 'lucide-react';
import {
  ANY_CLASS,
  Capabilities,
  CapabilityChange,
  TIERS,
  TIER_HINTS,
  TIER_LABELS,
  Tier,
  capabilityEventText,
  classLabel,
  fromText,
  isOpenProposal,
  isTier,
  openProposals,
  ProposalDecision,
  Scorecard,
  TierProposal,
  revokeTarget,
  toClass,
  validClass,
} from '../flightSchool';
import { ago, capabilitiesApi, onMission } from '../missions';
import { ProposalCard, ScorecardDisclosure, useProposalDecisions } from './TierProposals';

export interface CapabilitiesApi {
  get: (kwantId: string) => Promise<Capabilities>;
  set: (kwantId: string, taskClass: string, tier: Tier) => Promise<CapabilityChange>;
  revoke: (kwantId: string, taskClass: string, toTier: Tier) => Promise<CapabilityChange>;
  scorecard: (kwantId: string, taskClass: string) => Promise<Scorecard>;
  proposals: (status?: string, kwantId?: string) => Promise<TierProposal[]>;
  grant: (id: string, key: string) => Promise<ProposalDecision>;
  decline: (id: string, key: string, note?: string) => Promise<ProposalDecision>;
}

const select =
  'bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1.5 min-h-[36px] text-xs text-zinc-100 focus:border-sky-500 focus:outline-none disabled:opacity-60';
const revokeBtn =
  'px-2 py-1 min-h-[36px] rounded-lg border border-rose-500/40 text-[11px] font-medium text-rose-200 hover:bg-rose-500/10 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-400';

const TierSelect: React.FC<{ id?: string; label: string; value: Tier; disabled: boolean; onChange: (t: Tier) => void }> = ({ id, label, value, disabled, onChange }) => (
  <select id={id} aria-label={id ? undefined : label} value={value} disabled={disabled} onChange={(e) => isTier(e.target.value) && onChange(e.target.value)} className={select}>
    {TIERS.map((t) => (
      <option key={t} value={t}>
        {TIER_LABELS[t]}
      </option>
    ))}
  </select>
);

const RevokeButton: React.FC<{ what: string; tier: Tier; disabled: boolean; onRevoke: (to: Tier) => void }> = ({ what, tier, disabled, onRevoke }) => {
  const to = revokeTarget(tier);
  if (!to) return null;
  return (
    <button type="button" disabled={disabled} onClick={() => onRevoke(to)} className={revokeBtn} aria-label={`Revoke ${what} to ${TIER_LABELS[to]}`} data-revoke={to}>
      Revoke to {TIER_LABELS[to]}
    </button>
  );
};

export const FlightSchoolPanel: React.FC<{
  kwantId: string;
  kwantName: string;
  /** Tests pass the loaded state (null: an older server) and the API. */
  initial?: Capabilities | null;
  api?: CapabilitiesApi;
  now?: number;
}> = ({ kwantId, kwantName, initial, api = capabilitiesApi, now = Date.now() }) => {
  const [caps, setCaps] = useState<Capabilities | null | undefined>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [newClass, setNewClass] = useState('');
  const [newTier, setNewTier] = useState<Tier>('sandbox');
  // Bumped by a live change so open scorecards read again.
  const [version, setVersion] = useState(0);
  // Decided here this session, shown with what happened until the panel closes.
  const [decided, setDecided] = useState<TierProposal[]>([]);
  const allId = useId();
  const newId = useId();

  const reload = useCallback(() => {
    api
      .get(kwantId)
      .then(setCaps)
      .catch(() => {});
  }, [api, kwantId]);

  useEffect(() => {
    if (initial !== undefined) return;
    let live = true;
    setCaps(undefined);
    api
      .get(kwantId)
      .then((c) => live && setCaps(c))
      .catch(() => live && setCaps(null));
    return () => {
      live = false;
    };
  }, [kwantId, initial, api]);

  // A grant, decline, revoke or new request from another tab, device or the bot itself.
  useEffect(
    () =>
      onMission('capability_changed', ({ capability }) => {
        if (capability.kwant_id !== kwantId) return;
        reload();
        setVersion((v) => v + 1);
      }),
    [kwantId, reload],
  );

  const onDecided = useCallback(
    (p: TierProposal, d: ProposalDecision | null) => {
      if (d) {
        setCaps(d.capabilities);
        setDecided((cur) => [d.proposal, ...cur.filter((x) => x.id !== p.id)]);
      } else {
        // 409: someone else decided. Keep the card with the conflict, and read what holds now.
        setDecided((cur) => [p, ...cur.filter((x) => x.id !== p.id)]);
        api.get(kwantId).then(
          (c) => {
            setCaps(c);
            const now = c.proposals?.find((x) => x.id === p.id);
            if (now) setDecided((cur) => cur.map((x) => (x.id === p.id ? now : x)));
          },
          () => {},
        );
      }
    },
    [api, kwantId],
  );
  const decisions = useProposalDecisions(api, onDecided);

  // Nothing while loading, on an older server, or with Flight School off: the bot's settings look as before.
  if (!caps || !caps.flight_school) return null;
  const decidedIds = new Set(decided.map((p) => p.id));
  const asking = [...openProposals(caps.proposals ?? []).filter((p) => !decidedIds.has(p.id)), ...decided];

  const change = async (run: () => Promise<CapabilityChange>, done: string) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      const r = await run();
      setCaps(r.capabilities);
      setNote(r.unchanged ? 'It already had that tier.' : done);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t change the tier.');
      // Read them again so the screen shows what holds.
      reload();
      return false;
    } finally {
      setBusy(false);
    }
  };
  const setTier = (cls: string, tier: Tier) => change(() => api.set(kwantId, cls, tier), `${classLabel(cls)} is now ${TIER_LABELS[tier]}.`);
  const revoke = (cls: string, to: Tier) =>
    change(() => api.revoke(kwantId, cls, to), `Revoked: ${classLabel(cls)} is now ${TIER_LABELS[to]}. A running task is held to it from its next action.`);

  const slug = toClass(newClass);
  const addClass = async () => {
    if (!validClass(slug) || busy) return;
    if (await setTier(slug, newTier)) setNewClass('');
  };
  // The panel sits inside the Edit bot form, so this row is no form of its own (forms can't nest):
  // Enter here adds the kind of work and must not submit the Edit form.
  const onNewClassKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'Enter' || e.nativeEvent.isComposing) return;
    e.preventDefault();
    void addClass();
  };

  return (
    <section aria-labelledby={`${allId}-h`} className="space-y-2 rounded-xl border border-sky-500/20 bg-sky-950/10 p-3" data-flight-school>
      <h3 id={`${allId}-h`} className="flex items-center gap-1.5 text-xs font-medium text-zinc-200">
        <GraduationCap size={14} className="text-sky-300" aria-hidden="true" /> Flight School tier
      </h3>
      <p className="text-[11px] text-zinc-500">
        How much {kwantName} may do on its own, per kind of work. A tier only adds approval cards. Payments always ask you.
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor={allId} className="text-[11px] text-zinc-300">
          All work
        </label>
        <TierSelect id={allId} label="All work" value={caps.default_tier} disabled={busy} onChange={(t) => setTier(ANY_CLASS, t)} />
        <RevokeButton what="all work" tier={caps.default_tier} disabled={busy} onRevoke={(to) => revoke(ANY_CLASS, to)} />
      </div>
      <p className="text-[11px] text-zinc-500">{TIER_HINTS[caps.default_tier]}</p>

      {asking.length > 0 && (
        <ul className="space-y-2" aria-label={`Tiers ${kwantName} is asking for`} data-bot-proposals={asking.filter(isOpenProposal).length}>
          {asking.map((p) => (
            <ProposalCard
              key={p.id}
              proposal={p}
              bot={kwantName}
              now={now}
              busy={decisions.busyId === p.id}
              error={decisions.errors[p.id] ?? null}
              onGrant={decisions.grant}
              onDecline={decisions.decline}
            />
          ))}
        </ul>
      )}

      <ul className="divide-y divide-zinc-800 rounded-lg border border-zinc-800" aria-label="Tier per kind of work">
        {caps.effective.map((e) => (
          <li key={e.task_class} className="flex flex-wrap items-center gap-2 px-2.5 py-2" data-class={e.task_class} data-tier={e.tier} data-from={e.from}>
            <span className="flex-1 min-w-[8rem]">
              <span className="block text-xs text-zinc-200 break-all">{classLabel(e.task_class)}</span>
              <span className="block text-[11px] text-zinc-500">{fromText(e.from)}</span>
            </span>
            <TierSelect label={`Tier for ${classLabel(e.task_class)}`} value={e.tier} disabled={busy} onChange={(t) => setTier(e.task_class, t)} />
            <RevokeButton what={classLabel(e.task_class)} tier={e.tier} disabled={busy} onRevoke={(to) => revoke(e.task_class, to)} />
            {e.task_class !== ANY_CLASS && <ScorecardDisclosure kwantId={kwantId} taskClass={e.task_class} api={api} version={version} />}
          </li>
        ))}
      </ul>

      <div className="flex flex-wrap items-center gap-2" data-add-class>
        <label htmlFor={newId} className="sr-only">
          Another kind of work
        </label>
        <input
          id={newId}
          value={newClass}
          onChange={(e) => setNewClass(e.target.value)}
          onKeyDown={onNewClassKey}
          placeholder="Another kind of work, e.g. release"
          className="flex-1 min-w-[10rem] bg-zinc-950 border border-zinc-800 rounded-lg px-2.5 py-1.5 min-h-[36px] text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none"
        />
        <TierSelect label="Tier for the new kind of work" value={newTier} disabled={busy} onChange={setNewTier} />
        <button
          type="button"
          onClick={() => void addClass()}
          disabled={busy || !validClass(slug)}
          className="px-2.5 py-1 min-h-[36px] rounded-lg bg-zinc-800 text-[11px] font-medium text-zinc-100 hover:bg-zinc-700 disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
        >
          Set
        </button>
      </div>

      <p aria-live="polite" className="min-h-[1rem] text-[11px]">
        {busy ? (
          <span className="inline-flex items-center gap-1 text-zinc-400">
            <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Saving…
          </span>
        ) : note ? (
          <span className="text-emerald-300">{note}</span>
        ) : null}
      </p>
      {error && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}

      {caps.events.length > 0 && (
        <details className="text-[11px]">
          <summary className="cursor-pointer text-zinc-400 hover:text-zinc-200">Recent tier changes ({Math.min(caps.events.length, 5)})</summary>
          <ul className="mt-1 space-y-0.5" data-tier-events>
            {caps.events.slice(0, 5).map((ev) => (
              <li key={ev.id} className="text-zinc-400">
                {capabilityEventText(ev)} <span className="text-zinc-600">· {ago(ev.created_at, now)}</span>
                {ev.detail && <span className="block text-zinc-500 break-words">{ev.detail}</span>}
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
};
