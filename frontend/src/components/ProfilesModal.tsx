import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  X,
  UserRound,
  Plus,
  Trash2,
  Pencil,
  Check,
  Star,
  KeyRound,
  LogIn,
  LogOut,
  Loader2,
  ExternalLink,
  Copy,
  RefreshCw,
} from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import {
  ENGINES,
  Engine,
  KIND_LABELS,
  Login,
  Profile,
  isLoginDone,
  profileStatus,
  profilesApi,
  profilesForEngine,
  safeExternalUrl,
} from '../profiles';

interface ProfilesModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Called with the fresh list after every change, so kwant pickers stay current. */
  onProfilesChanged?: (profiles: Profile[]) => void;
}

const input =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';
const smallBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;
const toneClass = { ok: 'text-emerald-300', warn: 'text-amber-300', error: 'text-rose-300', muted: 'text-zinc-500' };
const dotClass = { ok: 'bg-emerald-400', warn: 'bg-amber-400', error: 'bg-rose-500', muted: 'bg-zinc-600' };

export const ProfilesModal: React.FC<ProfilesModalProps> = ({ isOpen, onClose, onProfilesChanged }) => {
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [adding, setAdding] = useState<Engine | null>(null);
  const changedRef = useRef(onProfilesChanged);
  changedRef.current = onProfilesChanged;
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  const reload = useCallback(async () => {
    try {
      const list = await profilesApi.list();
      setProfiles(list);
      setLoadError(null);
      changedRef.current?.(list);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Could not load profiles.');
    }
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    setProfiles(null);
    setAdding(null);
    reload();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') closeRef.current();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen, reload]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-2 sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="profiles-title"
        className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-2xl w-full max-h-[94vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 pb-4">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
              <UserRound size={16} />
            </div>
            <div>
              <h2 id="profiles-title" className="text-sm font-semibold text-zinc-100">LLM profiles</h2>
              <p className="text-xs text-zinc-400">Accounts each engine can use, like a work and a personal subscription</p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => {
              playSfx('click');
              onClose();
            }}
            aria-label="Close"
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        <p className="text-[11px] text-zinc-500 leading-relaxed">
          Kwants use their engine's <span className="text-zinc-300">default</span> profile unless you pick one for them. When a profile runs out
          of usage or is signed out before anything happened in a turn, the turn moves to the next enabled profile (lowest priority number
          first).
        </p>

        {loadError && (
          <div className="flex items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs" role="alert">
            <span>{loadError}</span>
            <button type="button" onClick={reload} className={quietBtn}>
              <RefreshCw size={12} /> Retry
            </button>
          </div>
        )}

        {!profiles && !loadError && (
          <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
            <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Loading profiles…
          </p>
        )}

        {profiles &&
          ENGINES.map((engine) => {
            const list = profilesForEngine(profiles, engine.id);
            return (
              <section key={engine.id} aria-labelledby={`engine-${engine.id}`} className="space-y-2">
                <div className="flex items-center justify-between">
                  <h3 id={`engine-${engine.id}`} className="text-xs font-semibold text-zinc-300">
                    {engine.label}
                  </h3>
                  {adding !== engine.id && (
                    <button
                      type="button"
                      onClick={() => {
                        playSfx('click');
                        setAdding(engine.id);
                      }}
                      className={quietBtn}
                    >
                      <Plus size={12} /> Add profile
                    </button>
                  )}
                </div>
                {list.length === 0 && adding !== engine.id && <p className="text-[11px] text-zinc-500">No profiles yet.</p>}
                <ul className="space-y-2">
                  {list.map((p) => (
                    <ProfileCard key={p.id} profile={p} onChanged={reload} />
                  ))}
                </ul>
                {adding === engine.id && (
                  <AddProfileForm
                    engine={engine.id}
                    onCancel={() => setAdding(null)}
                    onCreated={async () => {
                      setAdding(null);
                      playSfx('coin');
                      await reload();
                    }}
                  />
                )}
              </section>
            );
          })}
      </div>
    </div>
  );
};

const AddProfileForm: React.FC<{ engine: Engine; onCancel: () => void; onCreated: () => void }> = ({ engine, onCancel, onCreated }) => {
  const [name, setName] = useState('');
  const [kind, setKind] = useState<'cli_home' | 'api_key'>('cli_home');
  const [apiKey, setApiKey] = useState('');
  const [homeDir, setHomeDir] = useState('');
  const [isDefault, setIsDefault] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = `new-${engine}`;

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
      setError(err instanceof Error ? err.message : 'Could not create the profile.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="bg-zinc-950/60 border border-sky-500/30 rounded-xl p-3 space-y-3">
      <div className="grid sm:grid-cols-2 gap-3">
        <div>
          <label htmlFor={`${id}-name`} className="block text-[11px] font-medium text-zinc-400 mb-1">
            Name
          </label>
          <input id={`${id}-name`} autoFocus required maxLength={60} value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Work" className={input} />
        </div>
        <fieldset>
          <legend className="block text-[11px] font-medium text-zinc-400 mb-1">Signs in with</legend>
          <div className="flex p-0.5 rounded-lg bg-zinc-950 border border-zinc-800">
            {(['cli_home', 'api_key'] as const).map((k) => (
              <label
                key={k}
                className={`flex-1 text-center px-2 py-1.5 rounded-md text-[11px] font-medium cursor-pointer transition-colors has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-sky-500 ${
                  kind === k ? 'bg-sky-500/15 text-sky-200' : 'text-zinc-500 hover:text-zinc-300'
                }`}
              >
                <input type="radio" name={`${id}-kind`} value={k} checked={kind === k} onChange={() => setKind(k)} className="sr-only" />
                {KIND_LABELS[k]}
              </label>
            ))}
          </div>
        </fieldset>
      </div>
      {kind === 'api_key' ? (
        <div>
          <label htmlFor={`${id}-key`} className="block text-[11px] font-medium text-zinc-400 mb-1">
            API key <span className="text-zinc-600 font-normal">· optional now, stored write-only</span>
          </label>
          <input id={`${id}-key`} type="password" autoComplete="off" spellCheck={false} value={apiKey} onChange={(e) => setApiKey(e.target.value)} className={input} />
        </div>
      ) : (
        <div>
          <label htmlFor={`${id}-home`} className="block text-[11px] font-medium text-zinc-400 mb-1">
            Config folder <span className="text-zinc-600 font-normal">· optional; empty keeps a private one kwnt manages</span>
          </label>
          <input id={`${id}-home`} value={homeDir} onChange={(e) => setHomeDir(e.target.value)} placeholder="/home/me/.claude-work" className={`${input} font-mono`} />
          <p className="mt-1 text-[11px] text-zinc-500">After creating it, use “Sign in” to log this profile into its own account.</p>
        </div>
      )}
      <label className="flex items-center gap-2 text-[11px] text-zinc-300">
        <input type="checkbox" checked={isDefault} onChange={(e) => setIsDefault(e.target.checked)} className="accent-sky-500" />
        Make it the default for this engine
      </label>
      {error && <p className="text-[11px] text-rose-400" role="alert">{error}</p>}
      <div className="flex justify-end gap-2">
        <button type="button" onClick={onCancel} className="px-3 py-1.5 rounded-lg text-xs font-medium text-zinc-300 hover:bg-zinc-800 transition-colors">
          Cancel
        </button>
        <button
          type="submit"
          disabled={busy || !name.trim()}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50"
        >
          {busy && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" />} Create profile
        </button>
      </div>
    </form>
  );
};

const ProfileCard: React.FC<{ profile: Profile; onChanged: () => Promise<void> }> = ({ profile: p, onChanged }) => {
  const [editingName, setEditingName] = useState(false);
  const [name, setName] = useState(p.name);
  const [priority, setPriority] = useState(String(p.priority));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [keyOpen, setKeyOpen] = useState(false);
  const [keyValue, setKeyValue] = useState('');
  const [login, setLogin] = useState<Login | null>(null);
  const status = profileStatus(p);
  const isSystem = p.kind === 'system';

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
      setError(e instanceof Error ? e.message : 'That didn’t work.');
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

  return (
    <li className={`rounded-xl border p-3 space-y-2 ${p.enabled ? 'border-zinc-800 bg-zinc-950/60' : 'border-zinc-800/60 bg-zinc-950/30'}`}>
      <div className="flex flex-wrap items-center gap-2">
        {editingName ? (
          <form
            className="flex items-center gap-1.5"
            onSubmit={(e) => {
              e.preventDefault();
              saveName();
            }}
          >
            <label htmlFor={`name-${p.id}`} className="sr-only">
              Profile name
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
              className={`${input} py-1 w-40`}
            />
            <button type="submit" disabled={busy} className={quietBtn} aria-label="Save name">
              <Check size={12} />
            </button>
          </form>
        ) : (
          <>
            <span className="text-xs font-semibold text-zinc-100 truncate max-w-[14rem]">{p.name}</span>
            <button type="button" onClick={() => setEditingName(true)} className="p-0.5 rounded text-zinc-500 hover:text-zinc-200" aria-label={`Rename ${p.name}`}>
              <Pencil size={11} />
            </button>
          </>
        )}
        <span className="px-1.5 py-0.5 rounded-md bg-zinc-800 text-[10px] text-zinc-400">{KIND_LABELS[p.kind]}</span>
        {p.is_default && (
          <span className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded-md bg-sky-500/15 text-[10px] text-sky-200">
            <Star size={9} /> Default
          </span>
        )}
        <span className={`ml-auto inline-flex items-center gap-1.5 text-[11px] ${toneClass[status.tone]}`}>
          <span className={`w-1.5 h-1.5 rounded-full ${dotClass[status.tone]}`} aria-hidden="true" />
          {status.text}
        </span>
      </div>

      {p.home_dir && <p className="text-[10px] font-mono text-zinc-600 truncate" title={p.home_dir}>{p.home_dir}</p>}

      <div className="flex flex-wrap items-center gap-2">
        <label className="inline-flex items-center gap-1.5 text-[11px] text-zinc-400">
          <input
            type="checkbox"
            checked={p.enabled}
            disabled={busy}
            onChange={(e) => run(() => profilesApi.update(p.id, { enabled: e.target.checked }))}
            className="accent-sky-500"
          />
          Enabled
        </label>
        <label className="inline-flex items-center gap-1 text-[11px] text-zinc-400">
          Priority
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
        </label>
        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          {!p.is_default && (
            <button type="button" disabled={busy} onClick={() => run(() => profilesApi.update(p.id, { is_default: true }))} className={quietBtn}>
              <Star size={11} /> Make default
            </button>
          )}
          {p.kind === 'cli_home' && !login && (
            <button type="button" disabled={busy} onClick={() => run(async () => setLogin(await profilesApi.startLogin(p.id)))} className={quietBtn}>
              <LogIn size={11} /> {p.signed_in ? 'Sign in again' : 'Sign in'}
            </button>
          )}
          {p.kind === 'cli_home' && p.signed_in && (
            <button type="button" disabled={busy} onClick={() => run(() => profilesApi.logout(p.id))} className={quietBtn}>
              <LogOut size={11} /> Sign out
            </button>
          )}
          {p.kind === 'api_key' && (
            <button type="button" disabled={busy} onClick={() => setKeyOpen((v) => !v)} className={quietBtn} aria-expanded={keyOpen}>
              <KeyRound size={11} /> {p.has_key ? 'Replace key' : 'Set key'}
            </button>
          )}
          {p.kind === 'api_key' && p.has_key && (
            <button type="button" disabled={busy} onClick={() => run(() => profilesApi.removeKey(p.id))} className={quietBtn}>
              Remove key
            </button>
          )}
          {!isSystem &&
            (confirmDelete ? (
              <span className="inline-flex items-center gap-1">
                <span className="text-[11px] text-rose-300">Delete?</span>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => run(() => profilesApi.remove(p.id))}
                  className={`${smallBtn} border-rose-500/40 bg-rose-500/15 text-rose-200 hover:bg-rose-500/25`}
                >
                  Delete
                </button>
                <button type="button" onClick={() => setConfirmDelete(false)} className={quietBtn}>
                  Keep
                </button>
              </span>
            ) : (
              <button type="button" disabled={busy} onClick={() => setConfirmDelete(true)} className={`${quietBtn} hover:text-rose-200`} aria-label={`Delete ${p.name}`}>
                <Trash2 size={11} />
              </button>
            ))}
        </div>
      </div>

      {confirmDelete && <p className="text-[11px] text-zinc-500">Kwants pinned to it go back to the default. Its folder stays on disk.</p>}

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
          <button type="submit" disabled={busy || !keyValue.trim()} className={`${smallBtn} border-sky-500/40 bg-sky-600 text-white hover:bg-sky-500`}>
            Save
          </button>
        </form>
      )}

      {login && (
        <LoginPanel
          profileId={p.id}
          login={login}
          onUpdate={setLogin}
          onDone={async (succeeded) => {
            setLogin(null);
            if (succeeded) playSfx('coin');
            await onChanged();
          }}
        />
      )}

      {error && <p className="text-[11px] text-rose-400" role="alert">{error}</p>}
    </li>
  );
};

const LOGIN_POLL_MS = 1500;

const LoginPanel: React.FC<{
  profileId: string;
  login: Login;
  onUpdate: (l: Login) => void;
  onDone: (succeeded: boolean) => void;
}> = ({ profileId, login, onUpdate, onDone }) => {
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
      ? login.error || 'Sign-in failed.'
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
          <ol className="list-decimal list-inside space-y-1 text-[11px] text-zinc-300">
            <li>
              {url ? (
                <a href={url} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-sky-300 underline underline-offset-2 hover:text-sky-200">
                  Open the sign-in page <ExternalLink size={11} />
                </a>
              ) : (
                'Waiting for the sign-in link…'
              )}{' '}
              and sign in with the account for this profile.
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
            {login.needs_code && <li>Paste the code the page shows you:</li>}
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
                placeholder="Code"
                className={`${input} py-1 font-mono`}
              />
              <button type="submit" disabled={busy || !code.trim()} className={`${smallBtn} border-sky-500/40 bg-sky-600 text-white hover:bg-sky-500`}>
                Submit
              </button>
            </form>
          )}
          <p className="flex items-center gap-1.5 text-[10px] text-zinc-500">
            <Loader2 size={10} className="animate-spin motion-reduce:animate-none" /> Waiting for you to finish…
          </p>
        </>
      )}
      {failure && <p className="text-[11px] text-rose-300" role="alert">{failure}</p>}
      {error && <p className="text-[11px] text-rose-400" role="alert">{error}</p>}
      <div className="flex justify-end">
        <button type="button" disabled={busy} onClick={cancel} className={quietBtn}>
          {done ? 'Close' : 'Cancel sign-in'}
        </button>
      </div>
    </div>
  );
};
