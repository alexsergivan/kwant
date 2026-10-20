// The approvals switch: the space's setting, a bot's override, and the marker shown wherever a
// bot runs without asking. The marker always has an icon and words, never colour alone.

import React, { useEffect, useId, useState } from 'react';
import { AlertTriangle, Loader2, ShieldCheck, ShieldOff } from 'lucide-react';
import {
  BotApprovals,
  BotApprovalsState,
  OFF_CONSEQUENCES,
  SPACE_OPTIONS,
  SpaceApprovals,
  approvalModeApi,
  botOptions,
  effectiveApprovals,
  offConfirmTitle,
} from '../approvalMode';

/** "Approvals off": in a chat header (full) or a sidebar row (icon, with the words for screen readers). */
export const ApprovalsOffBadge: React.FC<{ compact?: boolean; onClick?: () => void; title?: string }> = ({ compact, onClick, title }) => {
  const tip = title ?? 'Approvals are off: this bot runs commands, file writes and tools without asking.';
  if (compact) {
    return (
      <span className="shrink-0 inline-flex items-center text-amber-300" title={tip}>
        <ShieldOff size={11} aria-hidden="true" />
        <span className="sr-only">Approvals off</span>
      </span>
    );
  }
  const cls =
    'inline-flex items-center gap-1 shrink-0 px-2 py-0.5 rounded-full border border-amber-500/40 bg-amber-500/10 text-[11px] font-medium text-amber-200';
  const body = (
    <>
      <ShieldOff size={11} aria-hidden="true" />
      Approvals off
    </>
  );
  return onClick ? (
    <button type="button" onClick={onClick} title={tip} className={`${cls} hover:bg-amber-500/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400`}>
      {body}
    </button>
  ) : (
    <span className={cls} title={tip}>
      {body}
    </span>
  );
};

/** The plain-words confirm before anything runs without asking. */
export const OffConfirm: React.FC<{ title: string; busy: boolean; onCancel: () => void; onConfirm: () => void }> = ({ title, busy, onCancel, onConfirm }) => {
  const id = useId();
  return (
    <div role="alertdialog" aria-labelledby={`${id}-t`} aria-describedby={`${id}-d`} className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 space-y-2.5">
      <p id={`${id}-t`} className="flex items-center gap-2 text-xs font-semibold text-amber-100">
        <AlertTriangle size={14} aria-hidden="true" /> {title}
      </p>
      <ul id={`${id}-d`} className="list-disc pl-5 space-y-1 text-[11px] text-amber-100/90">
        {OFF_CONSEQUENCES.map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
      <p className="text-[11px] text-amber-100/80">Bots can’t change this. You can turn approvals back on here at any time.</p>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onCancel}
          autoFocus
          className="px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-200 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70"
        >
          Keep asking
        </button>
        <button
          type="button"
          onClick={onConfirm}
          disabled={busy}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-500 disabled:opacity-50"
        >
          {busy && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          Turn approvals off
        </button>
      </div>
    </div>
  );
};

const radioCls = (on: boolean) =>
  `flex items-start gap-2.5 p-2.5 rounded-lg border cursor-pointer transition-colors ${on ? 'border-sky-500/60 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-700'}`;

/** Space settings: Ask (default) / Don't ask. Ask saves at once; Don't ask needs the confirm. */
export const SpaceApprovalsSetting: React.FC<{
  spaceId: string;
  spaceName: string;
  value: SpaceApprovals;
  /** Bots in the space that override it, so the person knows who won't follow. */
  overrides?: { ask: string[]; off: string[] };
  onChanged: (v: SpaceApprovals) => void;
  /** Tests open it with the confirm showing. */
  initialConfirm?: boolean;
}> = ({ spaceId, spaceName, value, overrides, onChanged, initialConfirm = false }) => {
  const [current, setCurrent] = useState<SpaceApprovals>(value);
  const [confirming, setConfirming] = useState(initialConfirm);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setCurrent(value), [value]);

  const save = async (v: SpaceApprovals) => {
    setBusy(true);
    setError(null);
    try {
      const next = await approvalModeApi.setSpace(spaceId, v);
      setCurrent(next);
      setConfirming(false);
      onChanged(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t change approvals.');
    } finally {
      setBusy(false);
    }
  };
  const pick = (v: SpaceApprovals) => {
    if (v === current || busy) return;
    if (v === 'off') setConfirming(true);
    else save(v);
  };
  const exceptions = current === 'off' ? overrides?.ask ?? [] : overrides?.off ?? [];

  return (
    <div className="rounded-xl border border-zinc-800 bg-zinc-950/60 p-4 space-y-3">
      <p className="flex items-center gap-2 text-xs font-medium text-zinc-200">
        {current === 'off' ? <ShieldOff size={14} className="text-amber-300" aria-hidden="true" /> : <ShieldCheck size={14} className="text-zinc-400" aria-hidden="true" />}
        Approvals
        {current === 'off' && <ApprovalsOffBadge title={`Approvals are off in ${spaceName}.`} />}
      </p>
      <div role="radiogroup" aria-label={`Approvals in ${spaceName}`} className="space-y-1.5">
        {SPACE_OPTIONS.map((o) => (
          <label key={o.value} className={radioCls(current === o.value)}>
            <input
              type="radio"
              name={`approvals-${spaceId}`}
              value={o.value}
              checked={current === o.value}
              disabled={busy}
              onChange={() => pick(o.value)}
              className="mt-0.5 accent-sky-500"
            />
            <span className="min-w-0">
              <span className="block text-xs font-medium text-zinc-200">
                {o.label}
                {o.value === 'ask' && <span className="ml-1 text-zinc-500 font-normal">(default)</span>}
              </span>
              <span className="block text-[11px] text-zinc-500">{o.hint}</span>
            </span>
          </label>
        ))}
      </div>
      {confirming && <OffConfirm title={offConfirmTitle({ space: spaceName })} busy={busy} onCancel={() => setConfirming(false)} onConfirm={() => save('off')} />}
      {exceptions.length > 0 && !confirming && (
        <p className="text-[11px] text-zinc-500">
          {current === 'off' ? 'Still asks, set on the bot: ' : 'Runs without asking, set on the bot: '}
          {exceptions.join(', ')}.
        </p>
      )}
      {busy && !confirming && (
        <p role="status" className="text-[11px] text-zinc-400">
          Saving…
        </p>
      )}
      {error && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
    </div>
  );
};

/** A bot's override, in its settings. Loads its own state; saves on change, with the confirm when it turns off. */
export const BotApprovalsSetting: React.FC<{
  kwantId: string;
  kwantName: string;
  onChanged?: (s: BotApprovalsState) => void;
  /** Tests pass the loaded state instead of fetching it. */
  initial?: BotApprovalsState | null;
}> = ({ kwantId, kwantName, onChanged, initial }) => {
  const [state, setState] = useState<BotApprovalsState | null | undefined>(initial === undefined ? undefined : initial);
  const [pending, setPending] = useState<BotApprovals | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selectId = useId();

  useEffect(() => {
    if (initial !== undefined) return;
    let live = true;
    setState(undefined);
    approvalModeApi.bot(kwantId).then((s) => live && setState(s));
    return () => {
      live = false;
    };
  }, [kwantId, initial]);

  if (state === undefined) return <p className="text-[11px] text-zinc-500">Loading approvals…</p>;
  // An older server without the switch: show nothing rather than a control that can't save.
  if (state === null) return null;

  const save = async (v: BotApprovals) => {
    setBusy(true);
    setError(null);
    try {
      const next = await approvalModeApi.setBot(kwantId, v);
      setState(next);
      setPending(null);
      onChanged?.(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t change this bot’s approvals.');
    } finally {
      setBusy(false);
    }
  };
  const pick = (v: BotApprovals) => {
    if (v === state.approvals || busy) return;
    if (effectiveApprovals(v, state.space_approvals) === 'off' && state.approvals_effective !== 'off') setPending(v);
    else save(v);
  };

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={selectId} className="text-xs font-medium text-zinc-400">
          Approvals
        </label>
        {state.approvals_effective === 'off' && <ApprovalsOffBadge title={`${kwantName} runs commands, file writes and tools without asking.`} />}
      </div>
      <select
        id={selectId}
        value={pending ?? state.approvals}
        disabled={busy}
        onChange={(e) => pick(e.target.value as BotApprovals)}
        className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none transition-colors disabled:opacity-60"
      >
        {botOptions(state.space_approvals).map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <p className="text-[11px] text-zinc-500">Saved as soon as you change it.</p>
      {pending && <OffConfirm title={offConfirmTitle({ bot: kwantName })} busy={busy} onCancel={() => setPending(null)} onConfirm={() => save(pending)} />}
      {error && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
    </div>
  );
};
