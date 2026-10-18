import React, { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, ChevronRight, Copy, ExternalLink, KeyRound, Loader2, LogIn, LogOut, Pencil, Plus, Star, Trash2 } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import type { EngineInfo, Tone } from '../engines';
import { KIND_LABELS, Login, Profile, accountState, isLoginDone, isThirdPartyLabel, newAccountFor, profilesApi, safeExternalUrl } from '../profiles';

const input =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';
const smallBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;
const primaryBtn = `${smallBtn} border-sky-500/40 bg-sky-600 text-white hover:bg-sky-500`;

const PILL: Record<Tone, string> = {
  ok: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  warn: 'border-amber-500/30 bg-amber-500/10 text-amber-200',
  error: 'border-rose-500/30 bg-rose-500/10 text-rose-200',
  muted: 'border-zinc-700 bg-zinc-800/60 text-zinc-400',
  busy: 'border-sky-500/30 bg-sky-500/10 text-sky-200',
  info: 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200',
};
const PILL_DOT: Record<Tone, string> = {
  ok: 'bg-emerald-400',
  warn: 'bg-amber-400',
  error: 'bg-rose-400',
  muted: 'bg-zinc-500',
  busy: 'bg-sky-400 animate-pulse motion-reduce:animate-none',
  info: 'bg-cyan-400',
};

/** A status in words with a colored dot, so it never relies on color alone. */
export const StatusPill: React.FC<{ tone: Tone; children: React.ReactNode; className?: string }> = ({ tone, children, className = '' }) => (
  <span className={`inline-flex items-center gap-1.5 px-1.5 py-0.5 rounded-full border text-[10.5px] font-medium whitespace-nowrap ${PILL[tone]} ${className}`}>
    <span className={`w-1.5 h-1.5 rounded-full ${PILL_DOT[tone]}`} aria-hidden="true" />
    {children}
  </span>
);

/** An account's chip and the line under its name. */
export interface RowStatus {
  tone: Tone;
  label: string;
  detail: string | null;
}

/** "This machine" is the engine's own login, so its state comes from the engine's check; Off wins either way. */
export function accountStatus(p: Profile, engine: EngineInfo, now = Date.now()): RowStatus {
  if (engine.enabled === false || engine.status === 'disabled') return { tone: 'muted', label: 'Off', detail: `Not checked while ${engine.name} is off` };
  if (p.kind !== 'system') return accountState(p, now);
  if (!p.enabled) return { tone: 'muted', label: 'Off', detail: 'Bots skip it while another account here is on' };
  switch (engine.status) {
    case 'ready':
      return isThirdPartyLabel(p.account_label) ? accountState(p, now) : { tone: 'ok', label: 'Ready', detail: p.account_label };
    case 'signed_out':
      return { tone: 'warn', label: 'Needs sign-in', detail: null };
    case 'not_installed':
      return { tone: 'muted', label: 'Not installed', detail: null };
    case 'checking':
      return { tone: 'busy', label: 'Checking…', detail: null };
    case 'error':
      return { tone: 'error', label: 'Check failed', detail: engine.detail };
    default:
      return accountState(p, now);
  }
}

/** A small on/off switch with its name for screen readers. */
const Switch: React.FC<{ on: boolean; label: string; disabled?: boolean; onChange: (on: boolean) => void }> = ({ on, label, disabled, onChange }) => (
  <button
    type="button"
    role="switch"
    aria-checked={on}
    aria-label={label}
    title={on ? 'On: bots may use it' : 'Off: bots skip it'}
    disabled={disabled}
    onClick={() => {
      playSfx('click');
      onChange(!on);
    }}
    className={`relative shrink-0 w-8 h-[18px] rounded-full transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 ${
      on ? 'bg-sky-600' : 'bg-zinc-700'
    }`}
  >
    <span
      aria-hidden="true"
      className={`absolute top-[2px] left-[2px] w-[14px] h-[14px] rounded-full bg-white transition-transform motion-reduce:transition-none ${on ? 'translate-x-[14px]' : ''}`}
    />
  </button>
);

/**
 * An engine's accounts: "This machine" first, then any added ones. `hostAction` is the sign-in or
 * sign-out for "This machine" (Claude's runs through the engine's own login).
 */
export const AccountList: React.FC<{
  engine: EngineInfo;
  accounts: Profile[];
  onChanged: () => Promise<void>;
  hostAction?: React.ReactNode;
}> = ({ engine, accounts, onChanged, hostAction }) => {
  const [adding, setAdding] = useState(false);
  const addRef = useRef<HTMLButtonElement>(null);
  const spec = newAccountFor(engine.id);
  return (
    <div className="space-y-2">
      {accounts.length > 0 ? (
        <ul className="space-y-1.5" aria-label={`${engine.name} accounts`}>
          {accounts.map((p) => (
            <AccountRow
              key={p.id}
              profile={p}
              engine={engine}
              inUse={engine.uses?.profile_id === p.id}
              onChanged={onChanged}
              hostAction={p.kind === 'system' ? hostAction : undefined}
            />
          ))}
        </ul>
      ) : (
        <p className="text-[11px] text-zinc-500">{spec?.kind === 'api_key' ? 'No accounts with a key yet.' : 'No accounts yet.'}</p>
      )}
      {spec &&
        (adding ? (
          <AddAccountForm
            spec={spec}
            onCancel={() => {
              setAdding(false);
              requestAnimationFrame(() => addRef.current?.focus());
            }}
            onCreated={async () => {
              setAdding(false);
              playSfx('coin');
              await onChanged();
            }}
          />
        ) : (
          <button
            ref={addRef}
            type="button"
            onClick={() => {
              playSfx('click');
              setAdding(true);
            }}
            className={quietBtn}
          >
            <Plus size={12} aria-hidden="true" /> {spec.kind === 'api_key' ? 'Add an account with a key' : 'Add account'}
          </button>
        ))}
    </div>
  );
};

const AddAccountForm: React.FC<{
  spec: { engine: Profile['engine']; kind: 'cli_home' | 'api_key' };
  onCancel: () => void;
  onCreated: () => void;
}> = ({ spec, onCancel, onCreated }) => {
  const { engine, kind } = spec;
  const [name, setName] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [homeDir, setHomeDir] = useState('');
  const [isDefault, setIsDefault] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = `new-${engine}-${kind}`;

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await profilesApi.create({
        engine,
        name: name.trim(),
        kind,
        is_default: isDefault,
        ...(kind === 'api_key' && apiKey.trim() ? { api_key: apiKey.trim() } : {}),
        ...(kind === 'cli_home' && homeDir.trim() ? { home_dir: homeDir.trim() } : {}),
      });
      setApiKey('');
      onCreated();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not add the account.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      onSubmit={submit}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onCancel();
        }
      }}
      className="bg-zinc-950/60 border border-sky-500/30 rounded-lg p-3 space-y-3"
      aria-label="New account"
    >
      <div>
        <label htmlFor={`${id}-name`} className="block text-[11px] font-medium text-zinc-400 mb-1">
          Name
        </label>
        <input id={`${id}-name`} autoFocus required maxLength={60} value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Work" className={input} />
      </div>
      {kind === 'api_key' ? (
        <div>
          <label htmlFor={`${id}-key`} className="block text-[11px] font-medium text-zinc-400 mb-1">
            API key <span className="text-zinc-600 font-normal">· can be added later, never shown again</span>
          </label>
          <input id={`${id}-key`} type="password" autoComplete="off" spellCheck={false} value={apiKey} onChange={(e) => setApiKey(e.target.value)} className={input} />
        </div>
      ) : (
        <div>
          <label htmlFor={`${id}-home`} className="block text-[11px] font-medium text-zinc-400 mb-1">
            Config folder <span className="text-zinc-600 font-normal">· optional; leave empty and kwnt keeps its own</span>
          </label>
          <input id={`${id}-home`} value={homeDir} onChange={(e) => setHomeDir(e.target.value)} placeholder="/home/me/.claude-work" className={`${input} font-mono`} />
          <p className="mt-1 text-[11px] text-zinc-500">Next, use “Sign in” on the new account to log it in.</p>
        </div>
      )}
      <label className="flex items-center gap-2 text-[11px] text-zinc-300">
        <input type="checkbox" checked={isDefault} onChange={(e) => setIsDefault(e.target.checked)} className="accent-sky-500" />
        Make it the default
      </label>
      {error && (
        <p className="text-[11px] text-rose-300" role="alert">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        <button type="submit" disabled={busy || !name.trim()} className={primaryBtn}>
          {busy && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />} Add account
        </button>
      </div>
    </form>
  );
};

const AccountRow: React.FC<{
  profile: Profile;
  engine: EngineInfo;
  inUse: boolean;
  onChanged: () => Promise<void>;
  hostAction?: React.ReactNode;
}> = ({ profile: p, engine, inUse, onChanged, hostAction }) => {
  const [editingName, setEditingName] = useState(false);
  const [name, setName] = useState(p.name);
  const [priority, setPriority] = useState(String(p.priority));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [keyOpen, setKeyOpen] = useState(false);
  const [keyValue, setKeyValue] = useState('');
  const [login, setLogin] = useState<Login | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const status = accountStatus(p, engine);
  const isSystem = p.kind === 'system';
  const nameId = `account-${p.id}-name`;

  useEffect(() => setName(p.name), [p.name]);
  useEffect(() => setPriority(String(p.priority)), [p.priority]);

  const run = async (action: () => Promise<unknown>, after?: () => void) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      after?.();
      await onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'That didn’t work. Try again.');
    } finally {
      setBusy(false);
    }
  };

  const saveName = () => {
    const next = name.trim();
    if (!next || next === p.name) {
      setName(p.name);
      setEditingName(false);
      return;
    }
    run(() => profilesApi.update(p.id, { name: next }), () => setEditingName(false));
  };

  const savePriority = () => {
    const n = Number.parseInt(priority, 10);
    if (!Number.isFinite(n) || n === p.priority) {
      setPriority(String(p.priority));
      return;
    }
    run(() => profilesApi.update(p.id, { priority: Math.min(10000, Math.max(0, n)) }));
  };

  const off = status.label === 'Off';
  const isClaude = p.engine === 'claude';
  const kindText = isSystem ? `The ${engine.name.replace(/ CLI$/, '')} login on this computer` : p.kind === 'api_key' ? 'API key' : `${engine.name.replace(/ CLI$/, '')} subscription`;
  const needsSignIn = p.kind === 'cli_home' && !p.signed_in && !login;

  return (
    <li
      data-account={p.kind}
      data-state={status.label}
      aria-labelledby={nameId}
      className={`rounded-lg border px-3 py-2.5 space-y-2 transition-colors ${
        inUse ? 'border-emerald-500/35 bg-emerald-500/[0.05]' : off ? 'border-zinc-800/70 bg-zinc-950/30' : 'border-zinc-800 bg-zinc-950/50'
      }`}
    >
      <div className="flex items-start gap-2">
        <div className={`flex-1 min-w-0 space-y-0.5 ${off ? 'opacity-70' : ''}`}>
          <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
            {editingName ? (
              <form
                className="flex items-center gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault();
                  saveName();
                }}
              >
                <label htmlFor={`name-${p.id}`} className="sr-only">
                  Account name
                </label>
                <input
                  id={`name-${p.id}`}
                  autoFocus
                  maxLength={60}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Escape') {
                      e.stopPropagation();
                      setName(p.name);
                      setEditingName(false);
                    }
                  }}
                  className={`${input} py-1 w-36`}
                />
                <button type="submit" disabled={busy} className={quietBtn} aria-label="Save name">
                  <Check size={12} />
                </button>
              </form>
            ) : (
              <span className="inline-flex items-center gap-1 min-w-0">
                <span id={nameId} className={`text-xs font-semibold truncate max-w-[12rem] ${off ? 'text-zinc-400' : 'text-zinc-100'}`}>
                  {isSystem ? KIND_LABELS.system : p.name}
                </span>
                {!isSystem && (
                  <button
                    type="button"
                    onClick={() => setEditingName(true)}
                    className="p-0.5 rounded text-zinc-500 hover:text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
                    aria-label={`Rename ${p.name}`}
                  >
                    <Pencil size={11} />
                  </button>
                )}
              </span>
            )}
            {p.is_default && (
              <span className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded-md bg-sky-500/15 text-[10px] text-sky-200" title="Bots try this account first">
                <Star size={9} aria-hidden="true" /> Default
              </span>
            )}
            {inUse && <span className="px-1.5 py-0.5 rounded-md bg-emerald-500/15 text-[10px] font-medium text-emerald-200">In use</span>}
          </div>
          <p className="text-[11px] leading-snug text-zinc-400 break-words">
            {status.detail && <span className={status.tone === 'warn' || status.tone === 'error' ? 'text-amber-200' : status.tone === 'ok' ? 'text-zinc-200' : ''}>{status.detail}</span>}
            {status.detail && <span className="text-zinc-600"> · </span>}
            <span className="text-zinc-500">{kindText}</span>
          </p>
        </div>
        <StatusPill tone={status.tone} className="mt-px">
          {status.label}
        </StatusPill>
        <Switch
          on={p.enabled}
          disabled={busy}
          label={`Use ${isSystem ? KIND_LABELS.system : p.name} for bots`}
          onChange={(on) => run(() => profilesApi.update(p.id, { enabled: on }))}
        />
      </div>

      {hostAction && <div>{hostAction}</div>}

      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
        {p.kind === 'cli_home' && !login && (
          <button
            type="button"
            disabled={busy}
            onClick={() => run(async () => setLogin(await profilesApi.startLogin(p.id)))}
            className={p.signed_in ? quietBtn : primaryBtn}
            aria-describedby={needsSignIn ? `signin-hint-${p.id}` : undefined}
          >
            <LogIn size={11} aria-hidden="true" /> {p.signed_in ? 'Sign in again' : 'Sign in'}
          </button>
        )}
        {needsSignIn && (
          <span id={`signin-hint-${p.id}`} className="text-[10.5px] text-zinc-500">
            {isClaude ? 'Opens claude.ai, then you paste back a one-time code.' : 'Opens the sign-in page; you may copy a code between it and here.'}
          </span>
        )}
        {p.kind === 'cli_home' && p.signed_in && (
          <button type="button" disabled={busy} onClick={() => run(() => profilesApi.logout(p.id))} className={quietBtn}>
            <LogOut size={11} aria-hidden="true" /> Sign out
          </button>
        )}
        {p.kind === 'api_key' && (
          <button type="button" disabled={busy} onClick={() => setKeyOpen((v) => !v)} className={p.has_key ? quietBtn : primaryBtn} aria-expanded={keyOpen}>
            <KeyRound size={11} aria-hidden="true" /> {p.has_key ? 'Replace key' : 'Add key'}
          </button>
        )}
        {p.kind === 'api_key' && p.has_key && (
          <button type="button" disabled={busy} onClick={() => run(() => profilesApi.removeKey(p.id))} className={quietBtn}>
            Remove key
          </button>
        )}
        {!p.is_default && p.enabled && (
          <button type="button" disabled={busy} onClick={() => run(() => profilesApi.update(p.id, { is_default: true }))} className={quietBtn}>
            <Star size={11} aria-hidden="true" /> Make default
          </button>
        )}
        {busy && <Loader2 size={12} className="text-sky-400 animate-spin motion-reduce:animate-none" aria-label="Saving" />}
        <span className="ml-auto inline-flex items-center gap-1">
          <button
            type="button"
            aria-expanded={detailsOpen}
            aria-controls={`account-${p.id}-details`}
            onClick={() => setDetailsOpen((v) => !v)}
            className="inline-flex items-center gap-0.5 px-1.5 py-1 rounded-md text-[11px] text-zinc-500 hover:text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            {detailsOpen ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />} Details
          </button>
          {!isSystem &&
            (confirmDelete ? (
              <>
                <span className="text-[11px] text-rose-300">Remove?</span>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => run(() => profilesApi.remove(p.id))}
                  className={`${smallBtn} border-rose-500/40 bg-rose-500/15 text-rose-200 hover:bg-rose-500/25`}
                >
                  Remove
                </button>
                <button type="button" onClick={() => setConfirmDelete(false)} className={quietBtn}>
                  Keep
                </button>
              </>
            ) : (
              <button
                type="button"
                disabled={busy}
                onClick={() => setConfirmDelete(true)}
                className={`${quietBtn} hover:text-rose-200`}
                aria-label={`Remove ${p.name}`}
              >
                <Trash2 size={11} />
              </button>
            ))}
        </span>
      </div>

      {detailsOpen && (
        <div id={`account-${p.id}-details`} className="rounded-md border border-zinc-800 bg-zinc-950/60 px-2.5 py-2 space-y-1.5">
          <label className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-zinc-400">
            <span>Priority</span>
            <input
              type="number"
              min={0}
              max={10000}
              value={priority}
              disabled={busy}
              onChange={(e) => setPriority(e.target.value)}
              onBlur={savePriority}
              onKeyDown={(e) => e.key === 'Enter' && (e.currentTarget as HTMLInputElement).blur()}
              className="w-16 bg-zinc-950 border border-zinc-800 rounded-md px-1.5 py-0.5 text-[11px] text-zinc-100 focus:border-sky-500 focus:outline-none"
            />
            <span className="text-zinc-500">After the default, bots try lower numbers first.</span>
          </label>
          {p.home_dir && (
            <p className="text-[10.5px] text-zinc-500">
              Login folder: <span className="font-mono text-zinc-400 break-all select-text">{p.home_dir}</span>
            </p>
          )}
        </div>
      )}

      {confirmDelete && <p className="text-[11px] text-zinc-500">Bots pinned to it go back to the default. Its folder stays on disk.</p>}

      {keyOpen && p.kind === 'api_key' && (
        <form
          className="flex items-center gap-1.5"
          onSubmit={(e) => {
            e.preventDefault();
            const k = keyValue.trim();
            if (!k) return;
            run(
              () => profilesApi.setKey(p.id, k),
              () => {
                setKeyValue('');
                setKeyOpen(false);
              },
            );
          }}
        >
          <label htmlFor={`key-${p.id}`} className="sr-only">
            API key for {p.name}
          </label>
          <input
            id={`key-${p.id}`}
            type="password"
            autoComplete="off"
            spellCheck={false}
            autoFocus
            value={keyValue}
            onChange={(e) => setKeyValue(e.target.value)}
            placeholder="Paste the key (it can't be shown again)"
            className={`${input} py-1`}
          />
          <button type="submit" disabled={busy || !keyValue.trim()} className={primaryBtn}>
            Save
          </button>
        </form>
      )}

      {login && (
        <LoginPanel
          profileId={p.id}
          accountName={p.name}
          login={login}
          onUpdate={setLogin}
          onDone={async (succeeded) => {
            setLogin(null);
            if (succeeded) playSfx('coin');
            await onChanged();
          }}
        />
      )}

      {error && (
        <p className="text-[11px] text-rose-300" role="alert">
          {error}
        </p>
      )}
    </li>
  );
};

const LOGIN_POLL_MS = 1500;

const LoginPanel: React.FC<{
  profileId: string;
  accountName: string;
  login: Login;
  onUpdate: (l: Login) => void;
  onDone: (succeeded: boolean) => void;
}> = ({ profileId, accountName, login, onUpdate, onDone }) => {
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const done = isLoginDone(login.status);
  const url = safeExternalUrl(login.url);
  const finished = useRef(false);
  const finish = (succeeded: boolean) => {
    if (finished.current) return;
    finished.current = true;
    onDone(succeeded);
  };

  // Poll while the CLI is still waiting; stops on any final state and when the panel closes.
  useEffect(() => {
    if (done) return;
    let cancelled = false;
    const t = setTimeout(async () => {
      try {
        const next = await profilesApi.getLogin(profileId, login.flow_id);
        if (!cancelled) onUpdate(next);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : 'Lost track of the sign-in.');
      }
    }, LOGIN_POLL_MS);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [login, done, profileId, onUpdate]);

  useEffect(() => {
    if (login.status === 'succeeded') finish(true);
  }, [login.status]);

  const cancel = async () => {
    setBusy(true);
    try {
      if (!done) await profilesApi.cancelLogin(profileId, login.flow_id);
    } catch {
      // already finished on the server
    } finally {
      setBusy(false);
      finish(false);
    }
  };

  const submitCode = async (e: React.FormEvent) => {
    e.preventDefault();
    const c = code.trim();
    if (!c) return;
    setBusy(true);
    setError(null);
    try {
      onUpdate(await profilesApi.submitCode(profileId, login.flow_id, c));
      setCode('');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'The code was not accepted.');
    } finally {
      setBusy(false);
    }
  };

  const failure =
    login.status === 'failed'
      ? login.error || 'Sign-in failed. Start it again.'
      : login.status === 'expired'
        ? 'The sign-in expired. Start it again.'
        : login.status === 'cancelled'
          ? 'Sign-in cancelled.'
          : null;

  return (
    <div className="rounded-lg border border-sky-500/30 bg-sky-500/5 p-3 space-y-2.5" aria-live="polite">
      {login.status === 'starting' && (
        <p className="flex items-center gap-2 text-[11px] text-zinc-300">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" /> Starting the sign-in…
        </p>
      )}
      {login.status === 'waiting' && (
        <>
          <p className="text-[11px] font-medium text-zinc-100">Sign in to {accountName}</p>
          <ol className="list-decimal list-outside pl-4 space-y-1 text-[11px] text-zinc-300 marker:text-zinc-500">
            <li>
              {url ? (
                <a href={url} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-sky-300 underline underline-offset-2 hover:text-sky-200">
                  Open the sign-in page <ExternalLink size={11} />
                </a>
              ) : (
                'Waiting for the sign-in link…'
              )}{' '}
              and sign in with the account you want for {accountName}.
            </li>
            {login.user_code && (
              <li>
                Enter this code there:{' '}
                <span className="font-mono text-sm tracking-wider text-zinc-100 bg-zinc-950 border border-zinc-800 rounded px-1.5 py-0.5 select-all">{login.user_code}</span>{' '}
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(login.user_code ?? '');
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1500);
                    } catch {
                      // clipboard blocked (http): the code is selectable
                    }
                  }}
                  className="inline-flex items-center gap-0.5 text-[10px] text-zinc-400 hover:text-zinc-100"
                >
                  <Copy size={10} /> {copied ? 'Copied' : 'Copy'}
                </button>
              </li>
            )}
            {login.needs_code && (
              <>
                <li>The page then shows a one-time code. Copy it.</li>
                <li>Paste it below and press Finish sign-in.</li>
              </>
            )}
          </ol>
          {login.needs_code && (
            <form onSubmit={submitCode} className="flex items-center gap-1.5">
              <label htmlFor={`code-${login.flow_id}`} className="sr-only">
                Code from the sign-in page
              </label>
              <input
                id={`code-${login.flow_id}`}
                autoComplete="off"
                spellCheck={false}
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="Paste the code"
                className={`${input} py-1 font-mono`}
              />
              <button type="submit" disabled={busy || !code.trim()} className={`${primaryBtn} shrink-0`}>
                {busy && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />} Finish sign-in
              </button>
            </form>
          )}
          {login.needs_code && (
            <p className="text-[10.5px] text-zinc-500 leading-snug">
              Why a code? kwnt runs the sign-in in the background, so the page can’t hand the login back by itself. The code works once and isn’t kept.
            </p>
          )}
          <p className="flex items-center gap-1.5 text-[10px] text-zinc-500">
            <Loader2 size={10} className="animate-spin motion-reduce:animate-none" /> Waiting for you to finish…
          </p>
        </>
      )}
      {failure && (
        <p className="text-[11px] text-rose-300" role="alert">
          {failure}
        </p>
      )}
      {error && (
        <p className="text-[11px] text-rose-300" role="alert">
          {error}
        </p>
      )}
      <div className="flex justify-end">
        <button type="button" disabled={busy} onClick={cancel} className={quietBtn}>
          {done ? 'Close' : 'Cancel sign-in'}
        </button>
      </div>
    </div>
  );
};
