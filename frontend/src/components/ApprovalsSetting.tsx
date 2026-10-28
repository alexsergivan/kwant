// The approvals switch: the space's setting, a bot's override, and the marker shown wherever a
// bot runs without asking. The marker always has an icon and words, never colour alone.

import React, { useEffect, useId, useState } from 'react';
import { AlertTriangle, Loader2, ShieldCheck, ShieldOff } from 'lucide-react';
import {
  BotApprovals,
  BotApprovalsState,
  NeedsBitwardenError,
  SPACE_OPTIONS,
  SpaceApprovals,
  approvalModeApi,
  botOptions,
  effectiveApprovals,
  offConfirmTitle,
  needsOffConfirm,
  OFF_ADVICE,
  OFF_CONSEQUENCES,
  OFF_HINT,
  offConsequences,
} from '../approvalMode';
import { StoreStatus, moveFirstLabel, moveFirstText } from '../passwordStore';
import { ExposedBanner, MoveToBitwardenFirst, PasswordStoreState, usePasswordStore } from './PasswordStorePanel';

/** "Approvals off": in a chat header (full) or a sidebar row (icon, with the words for screen readers). */
export const ApprovalsOffBadge: React.FC<{ compact?: boolean; onClick?: () => void; title?: string }> = ({ compact, onClick, title }) => {
  const tip = title ?? 'Approvals are off: this bot acts without asking. Payments still ask you.';
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

/** The standing reminder next to the setting while approvals are off. */
export const OffHint: React.FC = () => (
  <p data-approvals-off-hint className="flex items-start gap-1.5 text-[11px] leading-snug text-amber-200/90">
    <ShieldOff size={12} className="mt-px shrink-0" aria-hidden="true" />
    {OFF_HINT}
  </p>
);

/**
 * The plain-words confirm before anything runs without asking. With passwords still saved on this
 * server, "off" isn't offered: they move to Bitwarden first (the server refuses otherwise, F-10).
 */
export const OffConfirm: React.FC<{
  title: string;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  /** Where passwords are kept; missing on screens (and older servers) that don't know. */
  passwords?: PasswordStoreState;
}> = ({ title, busy, onCancel, onConfirm, passwords }) => {
  const id = useId();
  const status = passwords?.status ?? null;
  const mustMove = !!status && status.local_count > 0;
  // Still finding out: don't offer "off" for the moment it takes.
  const checking = !!passwords && !status && !passwords.problem;
  return (
    <div
      role="alertdialog"
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !busy) {
          e.stopPropagation();
          onCancel();
        }
      }}
      aria-labelledby={`${id}-t`}
      aria-describedby={`${id}-d`}
      className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 space-y-2.5"
    >
      <p id={`${id}-t`} className="flex items-center gap-2 text-xs font-semibold text-amber-100">
        <AlertTriangle size={14} aria-hidden="true" /> {title}
      </p>
      <ul id={`${id}-d`} className="list-disc pl-5 space-y-1 text-[11px] text-amber-100/90">
        {/* With a move needed, the paragraph below says it; the note would repeat it. */}
        {(mustMove ? OFF_CONSEQUENCES : offConsequences(status)).map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
      {mustMove ? (
        <div className="space-y-1.5 pt-0.5" data-off-needs-bitwarden>
          <p className="text-[11px] text-amber-100 leading-snug">{moveFirstText(status!.local_count)}</p>
          <MoveToBitwardenFirst store={passwords!} label={moveFirstLabel(status!.local_count)} />
        </div>
      ) : (
        <>
          <p className="text-[11px] font-medium text-amber-100">{OFF_ADVICE}</p>
          <p className="text-[11px] text-amber-100/80">Bots can’t change this. You can turn approvals back on here at any time.</p>
        </>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onCancel}
          autoFocus
          className="px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-200 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70"
        >
          Cancel
        </button>
        {!mustMove && (
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy || checking}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-500 disabled:opacity-50"
          >
            {(busy || checking) && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            {checking ? 'Checking saved passwords…' : 'Turn off approvals'}
          </button>
        )}
      </div>
    </div>
  );
};

/** A refused "off": show the move step when the store is known, else the server's sentence. */
function refusedOff(err: unknown, passwords: PasswordStoreState): string | null {
  if (!(err instanceof NeedsBitwardenError)) return err instanceof Error ? err.message : null;
  const known = passwords.status;
  if (known) passwords.set({ ...known, local_count: Math.max(err.localCount ?? 1, 1) });
  passwords.reload();
  return known ? null : err.message;
}

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
  /** Tests open it with the confirm showing, and say where passwords are kept. */
  initialConfirm?: boolean;
  initialPasswords?: StoreStatus | null;
}> = ({ spaceId, spaceName, value, overrides, onChanged, initialConfirm = false, initialPasswords = null }) => {
  const [current, setCurrent] = useState<SpaceApprovals>(value);
  const [confirming, setConfirming] = useState(initialConfirm);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const passwords = usePasswordStore(true, true, initialPasswords);
  const storeReload = passwords.reload;
  useEffect(() => setCurrent(value), [value]);
  // Fresh numbers each time the confirm opens (a password may have been saved since).
  useEffect(() => {
    if (confirming) storeReload();
  }, [confirming, storeReload]);

  const save = async (v: SpaceApprovals) => {
    setBusy(true);
    setError(null);
    try {
      const next = await approvalModeApi.setSpace(spaceId, v);
      setCurrent(next);
      setConfirming(false);
      onChanged(next);
      storeReload();
    } catch (err) {
      setError(v === 'off' ? refusedOff(err, passwords) : err instanceof Error ? err.message : 'Couldn’t change approvals.');
    } finally {
      setBusy(false);
    }
  };
  const pick = (v: SpaceApprovals) => {
    if (v === current || busy) return;
    if (needsOffConfirm(current, v)) setConfirming(true);
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
      <ExposedBanner
        store={passwords}
        onApprovalsOn={() => {
          setCurrent('ask');
          onChanged('ask');
        }}
      />
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
      {confirming && (
        <OffConfirm title={offConfirmTitle({ space: spaceName })} busy={busy} passwords={passwords} onCancel={() => setConfirming(false)} onConfirm={() => save('off')} />
      )}
      {current === 'off' && !confirming && <OffHint />}
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
  /** Tests pass the loaded state instead of fetching it, a choice waiting on the confirm, and the store. */
  initial?: BotApprovalsState | null;
  initialPending?: BotApprovals | null;
  initialPasswords?: StoreStatus | null;
}> = ({ kwantId, kwantName, onChanged, initial, initialPending = null, initialPasswords = null }) => {
  const [state, setState] = useState<BotApprovalsState | null | undefined>(initial === undefined ? undefined : initial);
  const [pending, setPending] = useState<BotApprovals | null>(initialPending);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selectId = useId();
  const passwords = usePasswordStore(true, true, initialPasswords);
  const storeReload = passwords.reload;
  useEffect(() => {
    if (pending) storeReload();
  }, [pending, storeReload]);

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
      storeReload();
    } catch (err) {
      setError(pending ? refusedOff(err, passwords) : err instanceof Error ? err.message : 'Couldn’t change this bot’s approvals.');
    } finally {
      setBusy(false);
    }
  };
  const pick = (v: BotApprovals) => {
    if (v === state.approvals || busy) return;
    // "Same as space" counts as off when the space is: that needs the confirm too.
    if (needsOffConfirm(state.approvals_effective, effectiveApprovals(v, state.space_approvals))) setPending(v);
    else save(v);
  };

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={selectId} className="text-xs font-medium text-zinc-400">
          Approvals
        </label>
        {state.approvals_effective === 'off' && <ApprovalsOffBadge title={`${kwantName} acts without asking. Payments still ask you.`} />}
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
      {state.approvals_effective === 'off' && !pending && <OffHint />}
      <ExposedBanner
        store={passwords}
        onApprovalsOn={() =>
          approvalModeApi.bot(kwantId).then((s) => {
            if (!s) return;
            setState(s);
            onChanged?.(s);
          })
        }
      />
      {pending && (
        <OffConfirm title={offConfirmTitle({ bot: kwantName })} busy={busy} passwords={passwords} onCancel={() => setPending(null)} onConfirm={() => save(pending)} />
      )}
      {error && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
    </div>
  );
};
