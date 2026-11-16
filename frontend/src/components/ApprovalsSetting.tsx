// The approvals switch: the space's setting, a bot's override, and the marker shown wherever a
// bot runs without asking. The marker always has an icon and words, never colour alone.

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, CheckCheck, Loader2, RotateCcw, ShieldCheck, ShieldOff, Undo2 } from 'lucide-react';
import { AlwaysAllowed, alwaysAllowedApi, grantTool, grantWhere } from '../alwaysAllowed';
import { relativeTime } from '../away';
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
import { StoreStatus, acceptRiskLabel, acceptRiskText } from '../passwordStore';
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
 * server and the risk not accepted yet, it says so once (0.7.3 A3): keep them here (sends accept_risk,
 * once per server) or move them to Bitwarden first.
 */
export const OffConfirm: React.FC<{
  title: string;
  busy: boolean;
  onCancel: () => void;
  /** `acceptRisk`: the person kept passwords on this server (the one-time confirm). */
  onConfirm: (acceptRisk?: boolean) => void;
  /** Where passwords are kept; missing on screens (and older servers) that don't know. */
  passwords?: PasswordStoreState;
}> = ({ title, busy, onCancel, onConfirm, passwords }) => {
  const id = useId();
  const status = passwords?.status ?? null;
  const mustMove = !!status && status.local_count > 0 && !status.risk_accepted;
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
        <div className="space-y-1.5 pt-0.5" data-off-accept-risk>
          <p className="text-[11px] text-amber-100 leading-snug">{acceptRiskText(status!.local_count)}</p>
          <MoveToBitwardenFirst store={passwords!} label="Move them to Bitwarden first" />
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
        <button
          type="button"
          onClick={() => onConfirm(mustMove)}
          disabled={busy || checking}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-rose-600 hover:bg-rose-500 disabled:opacity-50"
          data-off-confirm={mustMove ? 'accept-risk' : 'off'}
        >
          {(busy || checking) && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          {checking ? 'Checking saved passwords…' : mustMove ? acceptRiskLabel(status!.local_count) : 'Turn off approvals'}
        </button>
      </div>
    </div>
  );
};

/** A refused "off": show the one-time confirm when the store is known, else the server's sentence. */
function refusedOff(err: unknown, passwords: PasswordStoreState): string | null {
  if (!(err instanceof NeedsBitwardenError)) return err instanceof Error ? err.message : null;
  const known = passwords.status;
  if (known) passwords.set({ ...known, local_count: Math.max(err.localCount ?? 1, 1), risk_accepted: false });
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

  const save = async (v: SpaceApprovals, acceptRisk = false) => {
    setBusy(true);
    setError(null);
    try {
      const next = await approvalModeApi.setSpace(spaceId, v, acceptRisk);
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
        {current === 'off' && <ApprovalsOffBadge title={`Approvals are off in ${spaceName}. Payments still ask you.`} />}
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
        <OffConfirm title={offConfirmTitle({ space: spaceName })} busy={busy} passwords={passwords} onCancel={() => setConfirming(false)} onConfirm={(accept) => save('off', accept)} />
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

const takeBackCls =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[36px] px-3 rounded-lg text-xs font-medium text-zinc-200 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 disabled:opacity-50 disabled:cursor-not-allowed';

const grantKey = (g: Pick<AlwaysAllowed, 'conversation' | 'tool_name'>) => `${g.conversation}\u0000${g.tool_name}`;

/**
 * "Allowed without asking" (25e): each "Always allow" this bot was given, kept across restarts,
 * with Take back on each and Take back all at two or more. Nothing at all when there are none
 * (or on an older server without the list).
 */
export const AlwaysAllowedList: React.FC<{
  kwantId: string;
  kwantName: string;
  /** Tests pass the loaded rows instead of fetching them. */
  initial?: AlwaysAllowed[] | null;
  now?: number;
}> = ({ kwantId, kwantName, initial, now }) => {
  const [rows, setRows] = useState<AlwaysAllowed[] | null | undefined>(initial);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const headingId = useId();
  const listRef = useRef<HTMLUListElement>(null);
  const headingRef = useRef<HTMLParagraphElement>(null);

  const load = useCallback(() => {
    let live = true;
    setRows(undefined);
    setLoadError(null);
    alwaysAllowedApi
      .list(kwantId)
      .then((r) => live && setRows(r))
      .catch((e) => {
        if (!live) return;
        setRows(null);
        setLoadError(e instanceof Error ? e.message : 'Couldn’t load what this bot does without asking.');
      });
    return () => {
      live = false;
    };
  }, [kwantId]);

  useEffect(() => {
    if (initial !== undefined) return;
    return load();
  }, [initial, load]);

  const takeBack = async (one?: AlwaysAllowed) => {
    if (busy) return;
    const key = one ? grantKey(one) : '*';
    const index = one && rows ? rows.findIndex((r) => grantKey(r) === key) : -1;
    setBusy(key);
    setError(null);
    setDone(null);
    try {
      const left = await alwaysAllowedApi.revoke(kwantId, one);
      setRows(left);
      setDone(
        one
          ? `Taken back. ${kwantName} asks again before it can ${grantTool(one.tool_name).toLowerCase()} ${grantWhere(one)}.`
          : `Taken back. ${kwantName} asks again before using any of these.`,
      );
      // Keep the keyboard in the list: the next row's Take back, else the heading
      requestAnimationFrame(() => {
        const buttons = listRef.current?.querySelectorAll<HTMLButtonElement>('button[data-take-back]');
        const next = buttons && buttons.length ? buttons[Math.min(Math.max(index, 0), buttons.length - 1)] : null;
        (next ?? headingRef.current)?.focus();
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t take that back. Try again.');
    } finally {
      setBusy(null);
    }
  };

  if (rows === undefined) {
    return (
      <p role="status" className="flex items-center gap-1.5 text-[11px] text-zinc-500" data-always-allowed-loading>
        <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
        Checking what runs without asking…
      </p>
    );
  }
  if (loadError) {
    return (
      <div role="alert" className="flex flex-wrap items-center gap-2 text-[11px] text-rose-300" data-always-allowed-error>
        <AlertTriangle size={12} aria-hidden="true" />
        <span className="min-w-0 flex-1">{loadError}</span>
        <button type="button" onClick={load} className={takeBackCls}>
          <RotateCcw size={12} aria-hidden="true" /> Try again
        </button>
      </div>
    );
  }
  const list = rows ?? [];
  if (list.length === 0) {
    // Nothing to list: no heading, only the word that the last one went
    return done ? (
      <p role="status" tabIndex={-1} ref={headingRef} className="flex items-center gap-1.5 text-[11px] text-zinc-400 focus:outline-none" data-always-allowed-done>
        <Undo2 size={12} aria-hidden="true" /> {done}
      </p>
    ) : null;
  }

  return (
    <section aria-labelledby={headingId} className="space-y-2 pt-1" data-always-allowed>
      <div className="flex items-center justify-between gap-2">
        <p id={headingId} ref={headingRef} tabIndex={-1} className="flex items-center gap-1.5 text-xs font-medium text-zinc-400 focus:outline-none">
          <CheckCheck size={14} aria-hidden="true" />
          Allowed without asking
          <span className="text-zinc-500 font-normal tabular-nums">({list.length})</span>
        </p>
        {list.length >= 2 && (
          <button type="button" onClick={() => takeBack()} disabled={!!busy} className={takeBackCls} data-take-back-all>
            {busy === '*' && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            Take back all
          </button>
        )}
      </div>
      <p className="text-[11px] text-zinc-500">
        You answered Always allow on these. Each one runs without a card in that conversation only, also after a restart.
      </p>
      <ul ref={listRef} className="rounded-lg border border-zinc-800 divide-y divide-zinc-800 bg-zinc-950/60">
        {list.map((g) => {
          const key = grantKey(g);
          const tool = grantTool(g.tool_name);
          const where = grantWhere(g);
          const when = g.created_at ? relativeTime(g.created_at, now) : '';
          const full = g.created_at && !Number.isNaN(Date.parse(g.created_at)) ? new Date(g.created_at).toLocaleString() : undefined;
          return (
            <li key={key} className="flex flex-col gap-2 p-2.5 sm:flex-row sm:items-center sm:justify-between" data-always-allowed-row>
              <div className="min-w-0">
                <p className="text-xs font-medium text-zinc-100 break-words" title={g.tool_name}>
                  {tool}
                </p>
                <p className="text-[11px] text-zinc-500 break-words">
                  {where}
                  {when && (
                    <>
                      {' · '}
                      <time dateTime={g.created_at} title={full ? `Allowed ${full}` : undefined}>
                        allowed {when}
                      </time>
                    </>
                  )}
                </p>
              </div>
              <button
                type="button"
                onClick={() => takeBack(g)}
                disabled={!!busy}
                aria-label={`Take back: ${tool} ${where}`}
                className={`${takeBackCls} self-start sm:self-auto`}
                data-take-back
              >
                {busy === key && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                Take back
              </button>
            </li>
          );
        })}
      </ul>
      {done && (
        <p role="status" className="flex items-center gap-1.5 text-[11px] text-zinc-400" data-always-allowed-done>
          <Undo2 size={12} aria-hidden="true" /> {done}
        </p>
      )}
      {error && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
    </section>
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
  /** Tests pass the "Allowed without asking" rows instead of fetching them. */
  initialAllowed?: AlwaysAllowed[] | null;
  now?: number;
}> = ({ kwantId, kwantName, onChanged, initial, initialPending = null, initialPasswords = null, initialAllowed, now }) => {
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

  const save = async (v: BotApprovals, acceptRisk = false) => {
    setBusy(true);
    setError(null);
    try {
      const next = await approvalModeApi.setBot(kwantId, v, undefined, acceptRisk);
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
        <OffConfirm title={offConfirmTitle({ bot: kwantName })} busy={busy} passwords={passwords} onCancel={() => setPending(null)} onConfirm={(accept) => save(pending, accept)} />
      )}
      {error && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
      <AlwaysAllowedList kwantId={kwantId} kwantName={kwantName} initial={initialAllowed} now={now} />
    </div>
  );
};
