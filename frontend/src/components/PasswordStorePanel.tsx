import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, ArrowRight, Check, ChevronDown, Info, KeyRound, Loader2, Lock, LockOpen, Server, ShieldCheck, X } from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { playSfx } from '../audio/chiptune';
import { approvalModeApi } from '../approvalMode';
import {
  BITWARDEN_CLOUDS,
  DEFAULT_BITWARDEN_SERVER,
  StoreError,
  StoreStatus,
  UnlockProblem,
  autofillGapNote,
  checkServerUrl,
  exposedText,
  migrateText,
  serverLabel,
  storeApi,
  unlockProblemText,
} from '../passwordStore';
import { useEscapeLayer } from '../escape';

export interface PasswordStoreState {
  status: StoreStatus | null;
  /** 'needs-token' shows the panel's own token state; the list says the same. */
  problem: 'needs-token' | 'error' | null;
  reload: () => void;
  /** After unlock, lock, connect or disconnect: the server's new status, no extra round trip. */
  set: (s: StoreStatus) => void;
}

/** Where passwords are kept, loaded quietly (a missing token is the list's own state). `peek`
 * never opens the token dialog: for screens where the store is a side note (approvals settings). */
export function usePasswordStore(enabled: boolean, peek = false, initial: StoreStatus | null = null): PasswordStoreState {
  const [status, setStatus] = useState<StoreStatus | null>(initial);
  const [problem, setProblem] = useState<PasswordStoreState['problem']>(null);
  const seq = useRef(0);
  const reload = useCallback(() => {
    if (!enabled) return;
    const mine = ++seq.current;
    (peek ? storeApi.peek() : storeApi.status())
      .then((s) => {
        if (mine !== seq.current) return;
        setStatus(s);
        setProblem(null);
      })
      .catch((e: unknown) => {
        if (mine !== seq.current) return;
        setProblem(e instanceof StoreError && e.problem.kind === 'needs-token' ? 'needs-token' : 'error');
      });
  }, [enabled, peek]);
  useEffect(reload, [reload]);
  useEffect(() => {
    window.addEventListener(PERSON_SIGNED_IN_EVENT, reload);
    return () => window.removeEventListener(PERSON_SIGNED_IN_EVENT, reload);
  }, [reload]);
  const set = useCallback((s: StoreStatus) => {
    seq.current++;
    setStatus(s);
    setProblem(null);
  }, []);
  return { status, problem, reload, set };
}

const field = 'w-full px-2 py-1.5 bg-zinc-950 border border-zinc-800 rounded text-[11px] text-zinc-200 focus:outline-none focus:border-sky-500';
const btnQuiet = 'px-2.5 py-1.5 rounded-lg text-[11px] font-medium bg-zinc-800 hover:bg-zinc-700 text-zinc-200 disabled:opacity-50';
const btnMain = 'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-semibold bg-sky-500 hover:bg-sky-400 text-zinc-950 disabled:opacity-50';

const ProblemLine: React.FC<{ text: string }> = ({ text }) => (
  <p role="alert" className="flex items-start gap-1.5 text-[11px] text-rose-300 leading-snug">
    <AlertTriangle size={12} className="flex-shrink-0 mt-px" aria-hidden="true" /> <span>{text}</span>
  </p>
);

/** Clears a secret input and returns what it held. The value lives only in the DOM field. */
function takeSecret(ref: React.RefObject<HTMLInputElement>, clear: boolean): string {
  const v = ref.current?.value ?? '';
  if (clear && ref.current) ref.current.value = '';
  return v;
}

type Step = { kind: 'two-factor'; providers: Array<{ id: number; name: string; usable: boolean }> } | { kind: 'new-device-code' } | null;

/**
 * Connect (first time: server, email, password or API key) or unlock (after a restart).
 * The master password and client secret are uncontrolled inputs: read at submit, never in state.
 * A two-step code or new-device code is asked for in the same form when Bitwarden wants one.
 */
export const BitwardenLoginForm: React.FC<{
  mode: 'connect' | 'unlock';
  status: StoreStatus | null;
  onDone: (s: StoreStatus) => void;
  onCancel: () => void;
  /** Tests start on a step or with a problem showing. */
  initialStep?: Step;
  initialProblem?: UnlockProblem | null;
  initialApiKey?: boolean;
  initialServer?: string;
}> = ({ mode, status, onDone, onCancel, initialStep = null, initialProblem = null, initialApiKey, initialServer = DEFAULT_BITWARDEN_SERVER }) => {
  const id = useId();
  const bw = status?.bitwarden ?? null;
  const [server, setServer] = useState(initialServer);
  const [selfHosted, setSelfHosted] = useState('');
  const [email, setEmail] = useState('');
  const [apiKey, setApiKey] = useState(initialApiKey ?? (mode === 'unlock' && bw?.login === 'api_key'));
  const [clientId, setClientId] = useState('');
  const [step, setStep] = useState<Step>(initialStep);
  const [provider, setProvider] = useState<number | null>(initialStep?.kind === 'two-factor' ? initialStep.providers.find((p) => p.usable)?.id ?? null : null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<UnlockProblem | null>(initialProblem);
  const [formError, setFormError] = useState<string | null>(null);
  const masterRef = useRef<HTMLInputElement>(null);
  const secretRef = useRef<HTMLInputElement>(null);
  const codeRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (step) codeRef.current?.focus();
  }, [step]);

  // Leaving the form drops whatever was typed in the secret fields.
  useEffect(
    () => () => {
      takeSecret(masterRef, true);
      takeSecret(secretRef, true);
    },
    [],
  );

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setFormError(null);
    let serverUrl: string | undefined;
    if (mode === 'connect') {
      if (server === 'self') {
        const checked = checkServerUrl(selfHosted);
        if ('error' in checked) return setFormError(checked.error);
        serverUrl = checked.url;
      } else serverUrl = server;
      if (!email.includes('@')) return setFormError('Enter the email of your Bitwarden account.');
      if (apiKey && !clientId.trim()) return setFormError('Enter the API key’s client_id.');
    }
    if (!masterRef.current?.value) return setFormError('Enter your master password.');
    if (apiKey && !secretRef.current?.value) return setFormError('Enter the API key’s client_secret.');
    if (step && !code.trim()) return setFormError(step.kind === 'two-factor' ? 'Enter the two-step login code.' : 'Enter the code from the email.');
    setBusy(true);
    setProblem(null);
    try {
      const s = await storeApi.unlock({
        master_password: takeSecret(masterRef, false),
        server_url: serverUrl,
        email: mode === 'connect' ? email : undefined,
        client_id: apiKey && mode === 'connect' ? clientId : undefined,
        client_secret: apiKey ? takeSecret(secretRef, false) : undefined,
        two_factor_provider: step?.kind === 'two-factor' ? provider ?? undefined : undefined,
        two_factor_token: step?.kind === 'two-factor' ? code : undefined,
        new_device_otp: step?.kind === 'new-device-code' ? code : undefined,
      });
      takeSecret(masterRef, true);
      takeSecret(secretRef, true);
      playSfx('coin');
      onDone(s);
    } catch (err) {
      const p = err instanceof StoreError ? err.problem : ({ kind: 'error', message: 'Couldn’t reach the server.' } as UnlockProblem);
      setProblem(p);
      if (p.kind === 'two-factor') {
        setStep(p);
        setProvider(p.providers.find((x) => x.usable)?.id ?? null);
        setCode('');
      } else if (p.kind === 'new-device-code') {
        setStep(p);
        setCode('');
      } else if (p.kind === 'wrong-password') {
        takeSecret(masterRef, true);
        masterRef.current?.focus();
      }
    } finally {
      setBusy(false);
    }
  };

  const noUsableTwoFactor = step?.kind === 'two-factor' && !step.providers.some((p) => p.usable);
  // The hint describes the field rather than naming it, so screen readers hear the label alone first.
  const label = (text: string, input: React.ReactElement, hint?: React.ReactNode) => {
    const hintId = `${id}-${text.replace(/\W+/g, '-').toLowerCase()}-hint`;
    return (
      <div>
        <label className="block">
          <span className="text-[10px] text-zinc-400 block mb-0.5">{text}</span>
          {hint ? React.cloneElement(input as React.ReactElement<{ 'aria-describedby'?: string }>, { 'aria-describedby': hintId }) : input}
        </label>
        {hint && <span id={hintId} className="text-[10px] text-zinc-500 block mt-0.5 leading-snug">{hint}</span>}
      </div>
    );
  };

  return (
    <form onSubmit={submit} className="space-y-2 text-xs" aria-labelledby={`${id}-t`} data-bitwarden-form={mode} noValidate>
      <p id={`${id}-t`} className="text-[11px] font-semibold text-sky-400 flex items-center gap-1.5">
        {mode === 'connect' ? <KeyRound size={12} aria-hidden="true" /> : <LockOpen size={12} aria-hidden="true" />}
        {mode === 'connect' ? 'Connect Bitwarden' : 'Unlock Bitwarden'}
      </p>
      {mode === 'unlock' && bw && (
        <p className="text-[11px] text-zinc-400 break-all">
          {bw.email} on {serverLabel(bw.server_url)}
        </p>
      )}

      {mode === 'connect' && (
        <>
          {label(
            'Server',
            <select value={server} onChange={(e) => setServer(e.target.value)} className={field} disabled={!!step}>
              {BITWARDEN_CLOUDS.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
              <option value="self">Self-hosted (Vaultwarden too)</option>
            </select>,
          )}
          {server === 'self' &&
            label(
              'Server address',
              <input type="url" value={selfHosted} onChange={(e) => setSelfHosted(e.target.value)} placeholder="https://vault.example.com" className={`${field} font-mono`} autoComplete="off" spellCheck={false} disabled={!!step} />,
            )}
          {label(
            'Account email',
            <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="you@example.com" className={field} autoComplete="off" spellCheck={false} disabled={!!step} />,
          )}
          <fieldset className="space-y-1">
            <legend className="text-[10px] text-zinc-400 mb-0.5">Sign in with</legend>
            <div className="flex flex-wrap gap-x-3 gap-y-1">
              {[
                { v: false, l: 'Master password' },
                { v: true, l: 'API key + master password' },
              ].map((o) => (
                <label key={String(o.v)} className="flex items-center gap-1.5 text-[11px] text-zinc-300 cursor-pointer">
                  <input type="radio" name={`${id}-method`} checked={apiKey === o.v} onChange={() => setApiKey(o.v)} disabled={!!step} className="accent-sky-500" />
                  {o.l}
                </label>
              ))}
            </div>
            {apiKey && <p className="text-[10px] text-zinc-500 leading-snug">For accounts whose two-step login can’t be typed here. Find it in Bitwarden under Account settings → Security → Keys.</p>}
          </fieldset>
          {apiKey &&
            label('client_id', <input type="text" value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder="user.…" className={`${field} font-mono`} autoComplete="off" spellCheck={false} />)}
        </>
      )}
      {apiKey &&
        label(
          'client_secret',
          <input ref={secretRef} type="password" className={`${field} font-mono`} autoComplete="off" spellCheck={false} />,
          mode === 'unlock' ? 'Asked for at every unlock, and never kept.' : undefined,
        )}
      {label(
        'Master password',
        <input ref={masterRef} type="password" className={field} autoComplete="off" autoFocus={mode === 'unlock'} />,
        'Used to unlock, then forgotten. It’s never saved.',
      )}

      {step?.kind === 'two-factor' && !noUsableTwoFactor && (
        <div className="p-2 rounded-lg border border-sky-500/30 bg-sky-500/5 space-y-1.5" data-step="two-factor">
          {step.providers.filter((p) => p.usable).length > 1 &&
            label(
              'Method',
              <select value={provider ?? ''} onChange={(e) => setProvider(Number(e.target.value))} className={field}>
                {step.providers
                  .filter((p) => p.usable)
                  .map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
              </select>,
            )}
          {label(
            step.providers.find((p) => p.id === provider)?.name === 'Email' ? 'Code from the email' : 'Two-step login code',
            <input ref={codeRef} type="text" inputMode="numeric" value={code} onChange={(e) => setCode(e.target.value)} className={`${field} font-mono tracking-widest`} autoComplete="one-time-code" />,
          )}
        </div>
      )}
      {step?.kind === 'new-device-code' && (
        <div className="p-2 rounded-lg border border-sky-500/30 bg-sky-500/5" data-step="new-device-code">
          {label(
            'Code from Bitwarden’s email',
            <input ref={codeRef} type="text" inputMode="numeric" value={code} onChange={(e) => setCode(e.target.value)} className={`${field} font-mono tracking-widest`} autoComplete="one-time-code" />,
          )}
        </div>
      )}

      {formError && <ProblemLine text={formError} />}
      {problem && !formError && problem.kind !== 'needs-token' && <ProblemLine text={unlockProblemText(problem)} />}
      {problem?.kind === 'needs-token' && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-zinc-300">
          {unlockProblemText(problem)}
          <button type="button" onClick={requestPersonToken} className="text-sky-400 underline underline-offset-2">
            Enter access token
          </button>
        </p>
      )}

      <div className="flex justify-end gap-1.5 pt-1">
        <button type="button" onClick={onCancel} className={btnQuiet}>
          Cancel
        </button>
        <button type="submit" disabled={busy || noUsableTwoFactor} className={btnMain}>
          {busy && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          {busy ? (mode === 'connect' ? 'Connecting…' : 'Unlocking…') : step ? 'Continue' : mode === 'connect' ? 'Connect' : 'Unlock'}
        </button>
      </div>
    </form>
  );
};

/** The unlock form in a dialog, from the "Locked" state of the list or the store bar. */
export const UnlockDialog: React.FC<{ status: StoreStatus | null; onClose: () => void; onUnlocked: (s: StoreStatus) => void }> = ({ status, onClose, onUnlocked }) => {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  useEscapeLayer(rootRef, true, onClose, { modal: true });
  return (
    <div className="kw-dialog kw-dialog-bottom fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-3" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={rootRef} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby={`${id}-h`} className="kw-dialog-panel outline-none relative w-full max-w-sm rounded-2xl border border-zinc-800 bg-zinc-900 p-4 shadow-2xl" data-unlock-dialog>
        <h2 id={`${id}-h`} className="sr-only">
          Unlock Bitwarden
        </h2>
        <button type="button" onClick={onClose} aria-label="Close" className="absolute right-2 top-2 p-1.5 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800">
          <X size={14} />
        </button>
        <BitwardenLoginForm
          mode="unlock"
          status={status}
          onCancel={onClose}
          onDone={(s) => {
            onUnlocked(s);
            onClose();
          }}
        />
      </div>
    </div>
  );
};

/** "Move N saved passwords to Bitwarden": only when the person presses it; shows moved/failed. */
export const MigrateButton: React.FC<{ count: number; onDone: () => void; initialResult?: string | null }> = ({ count, onDone, initialResult = null }) => {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(initialResult);
  const [error, setError] = useState<string | null>(null);
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await storeApi.migrate();
      setResult(migrateText(r));
      playSfx(r.failed ? 'click' : 'coin');
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t move the passwords.');
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="space-y-1" data-migrate>
      {count > 0 && (
        <>
          <button type="button" onClick={run} disabled={busy} className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-medium bg-sky-500/10 text-sky-300 border border-sky-500/30 hover:bg-sky-500/20 disabled:opacity-50">
            {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowRight size={12} aria-hidden="true" />}
            {busy ? 'Moving…' : `Move ${count} saved ${count === 1 ? 'password' : 'passwords'} to Bitwarden`}
          </button>
          <p className="text-[10px] text-zinc-500 leading-snug">Each one is deleted here only after Bitwarden has it.</p>
        </>
      )}
      {result && (
        <p role="status" className="flex items-start gap-1.5 text-[11px] text-emerald-300 leading-snug">
          <Check size={12} className="flex-shrink-0 mt-px" aria-hidden="true" /> {result}
        </p>
      )}
      {error && <ProblemLine text={error} />}
    </div>
  );
};

/** Disconnect asks for the master password; the items stay in Bitwarden. */
const DisconnectConfirm: React.FC<{ onCancel: () => void; onDone: (s: StoreStatus) => void }> = ({ onCancel, onDone }) => {
  const ref = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => () => void takeSecret(ref, true), []);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!ref.current?.value) return setError('Enter your master password.');
    setBusy(true);
    setError(null);
    try {
      const s = await storeApi.disconnect(takeSecret(ref, false));
      takeSecret(ref, true);
      playSfx('click');
      onDone(s);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Couldn’t disconnect.');
      if (err instanceof StoreError && err.problem.kind === 'wrong-password') takeSecret(ref, true);
    } finally {
      setBusy(false);
    }
  };
  return (
    <form onSubmit={submit} role="alertdialog" aria-label="Disconnect Bitwarden?" className="p-2 rounded-lg border border-rose-500/30 bg-rose-500/5 space-y-1.5" data-disconnect noValidate>
      <p className="text-[11px] text-zinc-200 leading-snug">Disconnect Bitwarden? New logins are saved on this server again. Your items stay in Bitwarden.</p>
      <label className="block">
        <span className="text-[10px] text-zinc-400 block mb-0.5">Master password</span>
        <input ref={ref} type="password" className={field} autoComplete="off" autoFocus />
      </label>
      {error && <ProblemLine text={error} />}
      <div className="flex justify-end gap-1.5">
        <button type="button" onClick={onCancel} className={btnQuiet}>
          Keep Bitwarden
        </button>
        <button type="submit" disabled={busy} className="px-2.5 py-1.5 rounded-lg text-[11px] font-semibold bg-rose-500 hover:bg-rose-400 text-zinc-950 disabled:opacity-50">
          {busy ? 'Disconnecting…' : 'Disconnect'}
        </button>
      </div>
    </form>
  );
};

/** One line about where passwords are kept, opening the picker and Bitwarden's account settings. */
export const PasswordStoreBar: React.FC<{
  store: PasswordStoreState;
  onUnlock: () => void;
  /** The list changed (moved, store switched): load it again. */
  onChanged: () => void;
  /** Tests open it. */
  initialOpen?: boolean;
  initialConnecting?: boolean;
  initialDisconnecting?: boolean;
}> = ({ store, onUnlock, onChanged, initialOpen = false, initialConnecting = false, initialDisconnecting = false }) => {
  const id = useId();
  const [open, setOpen] = useState(initialOpen);
  const [connecting, setConnecting] = useState(initialConnecting);
  const [disconnecting, setDisconnecting] = useState(initialDisconnecting);
  const [locking, setLocking] = useState(false);
  // Keeps the "Moved N" result on screen after the count drops to 0.
  const [migrated, setMigrated] = useState(false);
  const s = store.status;
  if (!s) return null;
  const bw = s.bitwarden;
  const onBitwarden = s.store === 'bitwarden' && !!bw;

  const changed = (next: StoreStatus) => {
    store.set(next);
    onChanged();
  };
  const lock = async () => {
    setLocking(true);
    try {
      changed(await storeApi.lock());
      playSfx('click');
    } catch {
      store.reload();
    } finally {
      setLocking(false);
    }
  };

  const summary = onBitwarden ? (
    <>
      {s.locked ? <Lock size={13} className="text-amber-300 flex-shrink-0" aria-hidden="true" /> : <ShieldCheck size={13} className="text-emerald-400 flex-shrink-0" aria-hidden="true" />}
      <span className="min-w-0 truncate">
        <span className="text-zinc-200 font-medium">Bitwarden</span>
        <span className="text-zinc-500"> · {bw!.email}</span>
      </span>
      <span className={`px-1.5 rounded text-[9px] font-mono border flex-shrink-0 ${s.locked ? 'bg-amber-500/10 text-amber-200 border-amber-500/30' : 'bg-emerald-500/10 text-emerald-300 border-emerald-500/20'}`}>
        {s.locked ? 'Locked' : 'Unlocked'}
      </span>
    </>
  ) : (
    <>
      <Server size={13} className="text-zinc-400 flex-shrink-0" aria-hidden="true" />
      <span className="min-w-0 truncate text-zinc-200 font-medium">Saved on this server</span>
    </>
  );

  return (
    <section className="rounded-xl border border-zinc-800/80 bg-zinc-900/50 text-xs" data-store-bar={onBitwarden ? (s.locked ? 'locked' : 'unlocked') : 'local'}>
      <div className="flex items-center gap-1.5 px-2.5 py-1.5">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={`${id}-p`}
          onClick={() => setOpen((o) => !o)}
          className="flex-1 min-w-0 flex items-center gap-1.5 text-left text-[11px] rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <span className="sr-only">Where passwords are kept: </span>
          {summary}
          <ChevronDown size={12} className={`ml-auto flex-shrink-0 text-zinc-500 transition-transform ${open ? 'rotate-180' : ''}`} aria-hidden="true" />
        </button>
        {onBitwarden && s.locked && (
          <button type="button" onClick={onUnlock} className="flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] font-semibold bg-amber-400 hover:bg-amber-300 text-zinc-950 flex-shrink-0">
            <LockOpen size={11} aria-hidden="true" /> Unlock
          </button>
        )}
        {onBitwarden && !s.locked && (
          <button type="button" onClick={lock} disabled={locking} className="flex items-center gap-1 px-2 py-1 rounded-lg text-[11px] text-zinc-300 bg-zinc-800 hover:bg-zinc-700 flex-shrink-0 disabled:opacity-50">
            <Lock size={11} aria-hidden="true" /> Lock
          </button>
        )}
      </div>

      {open && (
        <div id={`${id}-p`} className="px-2.5 pb-2.5 pt-1 space-y-2 border-t border-zinc-800/80">
          <fieldset className="space-y-1.5">
            <legend className="text-[10px] text-zinc-400 mb-1">Where passwords are kept, for every bot</legend>
            <label className={`flex items-start gap-2 p-2 rounded-lg border cursor-pointer ${!onBitwarden && !connecting ? 'border-sky-500/60 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-700'}`}>
              <input
                type="radio"
                name={`${id}-store`}
                checked={!onBitwarden && !connecting}
                onChange={() => {
                  setConnecting(false);
                  if (onBitwarden) setDisconnecting(true);
                }}
                className="mt-0.5 accent-sky-500"
              />
              <span>
                <span className="block text-[11px] text-zinc-200 font-medium">Saved on this server</span>
                <span className="block text-[10px] text-zinc-500 leading-snug">Encrypted here. Works only while every space and bot is on “Ask before running commands”; otherwise use Bitwarden.</span>
              </span>
            </label>
            <label className={`flex items-start gap-2 p-2 rounded-lg border cursor-pointer ${onBitwarden || connecting ? 'border-sky-500/60 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-700'}`}>
              <input
                type="radio"
                name={`${id}-store`}
                checked={onBitwarden || connecting}
                onChange={() => {
                  setDisconnecting(false);
                  if (!onBitwarden) setConnecting(true);
                }}
                className="mt-0.5 accent-sky-500"
              />
              <span>
                <span className="block text-[11px] text-zinc-200 font-medium">Bitwarden</span>
                <span className="block text-[10px] text-zinc-500 leading-snug">In your own Bitwarden account, in a folder of their own. Locked after every restart until you unlock it.</span>
              </span>
            </label>
          </fieldset>

          {connecting && !onBitwarden && (
            <div className="p-2.5 rounded-xl border border-sky-500/30 bg-zinc-900/90">
              <BitwardenLoginForm
                mode="connect"
                status={s}
                onCancel={() => setConnecting(false)}
                onDone={(next) => {
                  setConnecting(false);
                  changed(next);
                }}
              />
            </div>
          )}

          {onBitwarden && (
            <div className="space-y-2" data-bitwarden-account>
              <dl className="grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 text-[10px]">
                <dt className="text-zinc-500">Account</dt>
                <dd className="text-zinc-300 break-all">{bw!.email}</dd>
                <dt className="text-zinc-500">Server</dt>
                <dd className="text-zinc-300 break-all">{serverLabel(bw!.server_url)}</dd>
                <dt className="text-zinc-500">Sign-in</dt>
                <dd className="text-zinc-300">{bw!.login === 'api_key' ? 'API key + master password' : 'Master password'}</dd>
                {!s.locked && bw!.item_count !== null && (
                  <>
                    <dt className="text-zinc-500">Logins</dt>
                    <dd className="text-zinc-300">{bw!.item_count}</dd>
                  </>
                )}
              </dl>
              {s.locked ? (
                s.local_count > 0 && <p className="text-[10px] text-zinc-500">{s.local_count} {s.local_count === 1 ? 'password is' : 'passwords are'} still saved on this server. Unlock to move them.</p>
              ) : (
                (s.local_count > 0 || migrated) && (
                  <MigrateButton
                    count={s.local_count}
                    onDone={() => {
                      setMigrated(true);
                      store.reload();
                      onChanged();
                    }}
                  />
                )
              )}
              {disconnecting ? (
                s.locked ? (
                  <p className="text-[10px] text-zinc-400">Unlock Bitwarden first, then you can disconnect it.</p>
                ) : (
                  <DisconnectConfirm
                    onCancel={() => setDisconnecting(false)}
                    onDone={(next) => {
                      setDisconnecting(false);
                      changed(next);
                    }}
                  />
                )
              ) : (
                <button type="button" onClick={() => setDisconnecting(true)} className="text-[10px] text-zinc-500 hover:text-rose-300 underline underline-offset-2">
                  Disconnect Bitwarden
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </section>
  );
};

/** What "Ask only when risky" or "Don't ask" means for saved passwords, said once, plainly, for where they're kept. */
export const AutofillGapNote: React.FC<{ status?: StoreStatus | null }> = ({ status }) => {
  const kind = status?.store === 'bitwarden' && status.local_count === 0 ? 'bitwarden' : 'local';
  return (
    <p className="flex items-start gap-2 text-[10px] text-zinc-400 leading-snug" data-autofill-gap={kind}>
      <Info size={12} className="text-zinc-500 flex-shrink-0 mt-px" aria-hidden="true" />
      <span>
        {/* The local sentence already names the modes. */}
        {kind === 'bitwarden' && <span className="text-zinc-300">Some bots run commands without asking. </span>}
        {autofillGapNote(status)}
      </span>
    </p>
  );
};

/**
 * One button that gets the passwords on this server into Bitwarden: connect (first time) or
 * unlock (after a restart) in place, then move them. Pressing it is the person's go-ahead, so the
 * move runs as soon as Bitwarden is ready. Nothing moves until it's pressed.
 */
export const MoveToBitwardenFirst: React.FC<{
  store: PasswordStoreState;
  label: string;
  /** After the move, with what it says ("Moved 3 passwords to Bitwarden."). */
  onMoved?: (result: string) => void;
  /** Tests open a step. */
  initialStep?: 'connect' | 'unlock' | null;
}> = ({ store, label, onMoved, initialStep = null }) => {
  const [step, setStep] = useState<'connect' | 'unlock' | null>(initialStep);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const s = store.status;
  if (!s) return null;
  const ready = s.store === 'bitwarden' && !!s.bitwarden && !s.locked;

  const move = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await storeApi.migrate();
      const text = migrateText(r);
      setResult(text);
      playSfx(r.failed ? 'click' : 'coin');
      store.reload();
      onMoved?.(text);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t move the passwords.');
    } finally {
      setBusy(false);
    }
  };
  const press = () => {
    setError(null);
    if (ready) move();
    else setStep(s.store === 'bitwarden' && s.bitwarden ? 'unlock' : 'connect');
  };

  return (
    <div className="space-y-1.5" data-move-first>
      {s.local_count > 0 && !step && (
        <button type="button" onClick={press} disabled={busy} className={btnMain}>
          {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowRight size={12} aria-hidden="true" />}
          {busy ? 'Moving…' : label}
        </button>
      )}
      {step && (
        <div className="p-2.5 rounded-xl border border-sky-500/30 bg-zinc-900/90 space-y-1.5" data-move-step={step}>
          <p className="text-[10px] text-zinc-400 leading-snug">
            {step === 'connect' ? 'Connect your Bitwarden account; the passwords move as soon as it’s connected.' : 'Unlock Bitwarden; the passwords move as soon as it’s unlocked.'}
          </p>
          <BitwardenLoginForm
            mode={step}
            status={s}
            onCancel={() => setStep(null)}
            onDone={(next) => {
              store.set(next);
              setStep(null);
              move();
            }}
          />
        </div>
      )}
      {result && (
        <p role="status" className="flex items-start gap-1.5 text-[11px] text-emerald-300 leading-snug">
          <Check size={12} className="flex-shrink-0 mt-px" aria-hidden="true" /> {result}
        </p>
      )}
      {error && <ProblemLine text={error} />}
    </div>
  );
};

/**
 * Approvals off with passwords still on this server (set up before that was blocked): said plainly
 * next to saved passwords and in approvals settings, with the two ways out. Nothing changes on its own.
 */
export const ExposedBanner: React.FC<{
  store: PasswordStoreState;
  /** After "Turn approvals back on", so the screen loads its approvals again. */
  onApprovalsOn?: () => void;
}> = ({ store, onApprovalsOn }) => {
  const id = useId();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const s = store.status;
  if (!s?.exposed)
    return done ? (
      <p role="status" className="flex items-start gap-1.5 text-[11px] text-emerald-300 leading-snug" data-exposed-done>
        <Check size={12} className="flex-shrink-0 mt-px" aria-hidden="true" /> {done}
      </p>
    ) : null;

  const turnOn = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await approvalModeApi.turnAllOn();
      playSfx('click');
      store.reload();
      onApprovalsOn?.();
      if (r.failed) setError(`${r.failed} ${r.failed === 1 ? 'setting' : 'settings'} couldn’t be changed. An archived space keeps its approvals until you restore it.`);
      else setDone('Approvals are back on. Bots ask before running commands again.');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t turn approvals back on.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <section role="region" aria-labelledby={`${id}-t`} className="p-2.5 rounded-xl border border-amber-500/40 bg-amber-500/10 space-y-2 text-xs" data-exposed>
      <p id={`${id}-t`} className="flex items-start gap-2 text-[11px] text-amber-100 leading-snug">
        <AlertTriangle size={13} className="flex-shrink-0 mt-px" aria-hidden="true" />
        <span>{exposedText(s.local_count)}</span>
      </p>
      <div className="flex flex-wrap items-start gap-1.5">
        <MoveToBitwardenFirst store={store} label="Move to Bitwarden" onMoved={setDone} />
        <button type="button" onClick={turnOn} disabled={busy} className={`${btnQuiet} flex items-center gap-1.5`}>
          {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ShieldCheck size={12} aria-hidden="true" />}
          {busy ? 'Turning on…' : 'Turn approvals back on'}
        </button>
      </div>
      {error && <ProblemLine text={error} />}
    </section>
  );
};
