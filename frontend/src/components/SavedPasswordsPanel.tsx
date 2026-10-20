import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, Check, Copy, ExternalLink, Eye, EyeOff, Key, KeyRound, Loader2, Lock, LockOpen, Plus, Search, ShieldCheck, Trash2 } from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { playSfx } from '../audio/chiptune';
import {
  PROBLEM_TEXT,
  PasswordsError,
  PasswordsProblem,
  PendingLogin,
  REVEAL_MS,
  SavedLogin,
  filterLogins,
  loginMatchesPage,
  originFromInput,
  originLabel,
  passwordsApi,
} from '../savedPasswords';
import { AutofillGapNote, ExposedBanner, PasswordStoreBar, PasswordStoreState, UnlockDialog } from './PasswordStorePanel';

export interface SavedPasswordsState {
  logins: SavedLogin[];
  pending: PendingLogin | null;
  /** True until the first answer. */
  loading: boolean;
  problem: PasswordsProblem | null;
  reload: () => void;
}

/**
 * Loads the saved logins for a team, channel or bot. No event fires when the browser catches a
 * login, so `refreshKey` (a new screenshot or page) loads again, at most every 1.5s.
 */
export function useSavedPasswords(sandboxId: string | undefined, refreshKey: unknown): SavedPasswordsState {
  const [logins, setLogins] = useState<SavedLogin[]>([]);
  const [pending, setPending] = useState<PendingLogin | null>(null);
  const [loading, setLoading] = useState(true);
  const [problem, setProblem] = useState<PasswordsProblem | null>(null);
  const seq = useRef(0);
  const usable = !!sandboxId && sandboxId !== 'default';

  const reload = useCallback(() => {
    if (!usable) return;
    const mine = ++seq.current;
    passwordsApi
      .list(sandboxId!)
      .then((l) => {
        if (mine !== seq.current) return;
        setLogins(l.passwords);
        setPending(l.pending);
        setProblem(null);
      })
      .catch((e: unknown) => {
        if (mine !== seq.current) return;
        setProblem(e instanceof PasswordsError ? e.problem : 'error');
        setLogins([]);
        setPending(null);
      })
      .finally(() => mine === seq.current && setLoading(false));
  }, [sandboxId, usable]);

  useEffect(() => {
    setLogins([]);
    setPending(null);
    setProblem(null);
    setLoading(usable);
    reload();
  }, [reload, usable]);

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const t = setTimeout(reload, 1500);
    return () => clearTimeout(t);
  }, [refreshKey, reload]);

  useEffect(() => {
    window.addEventListener(PERSON_SIGNED_IN_EVENT, reload);
    return () => window.removeEventListener(PERSON_SIGNED_IN_EVENT, reload);
  }, [reload]);

  return { logins, pending, loading, problem, reload };
}

const errorText = (e: unknown, fallback: string) => (e instanceof PasswordsError || e instanceof Error ? e.message : fallback);

/** "Save password for <site> as <user>?" for a login the browser caught. Nothing is saved until Save. */
export const PendingLoginPrompt: React.FC<{
  sandboxId: string;
  pending: PendingLogin;
  onDone: () => void;
}> = ({ sandboxId, pending, onDone }) => {
  const [busy, setBusy] = useState<'save' | 'discard' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const act = async (kind: 'save' | 'discard') => {
    setBusy(kind);
    setError(null);
    try {
      if (kind === 'save') await passwordsApi.savePending(sandboxId);
      else await passwordsApi.discardPending(sandboxId);
      playSfx(kind === 'save' ? 'coin' : 'click');
      onDone();
    } catch (e) {
      setError(errorText(e, 'Couldn’t do that. Try again.'));
      // 404: nothing is waiting any more (saved or discarded elsewhere).
      if (e instanceof PasswordsError && e.status === 404) onDone();
    } finally {
      setBusy(null);
    }
  };
  return (
    <div role="region" aria-label="Save password?" data-pending-login className="mx-3 mt-2 p-2.5 rounded-xl border border-sky-500/30 bg-sky-500/5 text-xs space-y-2">
      <p className="flex items-start gap-2 text-zinc-200 leading-snug">
        <KeyRound size={14} className="text-sky-400 flex-shrink-0 mt-px" aria-hidden="true" />
        <span>
          Save password for <span className="font-semibold break-all">{originLabel(pending.origin)}</span>
          {pending.username ? (
            <>
              {' '}as <span className="font-mono break-all">{pending.username}</span>
            </>
          ) : null}
          ?
        </span>
      </p>
      <p className="text-[11px] text-zinc-500 pl-[22px]">It’s filled in only on this exact address. Nothing is saved until you choose Save.</p>
      {error && (
        <p role="alert" className="flex items-center gap-1.5 text-[11px] text-rose-300 pl-[22px]">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
      <div className="flex justify-end gap-1.5">
        <button
          type="button"
          disabled={!!busy}
          onClick={() => act('discard')}
          className="px-2.5 py-1.5 rounded-lg text-[11px] font-medium bg-zinc-800 hover:bg-zinc-700 text-zinc-200 disabled:opacity-50"
        >
          {busy === 'discard' ? 'Discarding…' : 'Don’t save'}
        </button>
        <button
          type="button"
          disabled={!!busy}
          onClick={() => act('save')}
          className="px-2.5 py-1.5 rounded-lg text-[11px] font-semibold bg-sky-500 hover:bg-sky-400 text-zinc-950 disabled:opacity-50"
        >
          {busy === 'save' ? 'Saving…' : 'Save'}
        </button>
      </div>
    </div>
  );
};

/** The panel's own message when the list can't be shown. `locked` is ready for a store that unlocks. */
export const PasswordsProblemNotice: React.FC<{ problem: PasswordsProblem; onRetry: () => void; onUnlock?: () => void }> = ({ problem, onRetry, onUnlock }) => (
  <div className="text-center py-10 px-4 text-xs" data-passwords-problem={problem}>
    <Lock size={28} className="mx-auto mb-2 opacity-40 text-zinc-400" aria-hidden="true" />
    <p className="text-zinc-300 leading-relaxed">{PROBLEM_TEXT[problem]}</p>
    <div className="mt-3 flex justify-center">
      {problem === 'needs-token' ? (
        <button
          type="button"
          onClick={requestPersonToken}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium text-white bg-sky-600 hover:bg-sky-500"
        >
          <KeyRound size={12} aria-hidden="true" /> Enter access token
        </button>
      ) : problem === 'locked' && onUnlock ? (
        <button
          type="button"
          onClick={onUnlock}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-semibold text-zinc-950 bg-amber-400 hover:bg-amber-300"
        >
          <LockOpen size={12} aria-hidden="true" /> Unlock Bitwarden
        </button>
      ) : (
        <button type="button" onClick={onRetry} className="px-3 py-1.5 rounded-lg text-[11px] font-medium bg-zinc-800 hover:bg-zinc-700 text-zinc-200">
          Try again
        </button>
      )}
    </div>
  </div>
);

interface RowProps {
  sandboxId: string;
  login: SavedLogin;
  onThisPage: boolean;
  onOpen?: (origin: string) => void;
  onDeleted: () => void;
  /** Bitwarden: Reveal asks for the master password first. */
  askMasterPassword?: boolean;
  /** Tests start a row revealed, asking to delete or asking for the master password. */
  initialRevealed?: string | null;
  initialConfirmDelete?: boolean;
  initialAsking?: boolean;
}

/** One saved login: site and username; the password only after Reveal, for REVEAL_MS. */
export const SavedLoginRow: React.FC<RowProps> = ({
  sandboxId,
  login,
  onThisPage,
  onOpen,
  onDeleted,
  askMasterPassword = false,
  initialRevealed = null,
  initialConfirmDelete = false,
  initialAsking = false,
}) => {
  const [revealed, setRevealed] = useState<string | null>(initialRevealed);
  // The master password stays in its input (never in state) and is cleared once it's been sent.
  const [asking, setAsking] = useState(initialAsking);
  const masterRef = useRef<HTMLInputElement>(null);
  const [revealing, setRevealing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(initialConfirmDelete);
  const [deleting, setDeleting] = useState(false);
  const [copied, setCopied] = useState<'user' | 'pass' | null>(null);
  const [error, setError] = useState<string | null>(null);

  // A revealed password disappears on its own; it's never kept anywhere else.
  useEffect(() => {
    if (revealed === null) return;
    const t = setTimeout(() => setRevealed(null), REVEAL_MS);
    return () => clearTimeout(t);
  }, [revealed]);

  const reveal = async () => {
    if (revealed !== null) {
      setRevealed(null);
      return;
    }
    if (askMasterPassword && !asking) {
      setError(null);
      setAsking(true);
      return;
    }
    let master: string | undefined;
    if (askMasterPassword) {
      master = masterRef.current?.value ?? '';
      if (!master) {
        setError('Enter your Bitwarden master password.');
        masterRef.current?.focus();
        return;
      }
    }
    setRevealing(true);
    setError(null);
    try {
      const pw = await passwordsApi.reveal(sandboxId, login.id, master);
      if (masterRef.current) masterRef.current.value = '';
      setAsking(false);
      setRevealed(pw);
      playSfx('click');
    } catch (e) {
      if (masterRef.current) masterRef.current.value = '';
      masterRef.current?.focus();
      setError(errorText(e, 'Couldn’t show this password.'));
    } finally {
      master = undefined;
      setRevealing(false);
    }
  };

  const remove = async () => {
    setDeleting(true);
    setError(null);
    try {
      await passwordsApi.remove(sandboxId, login.id);
      playSfx('click');
      onDeleted();
    } catch (e) {
      setError(errorText(e, 'Couldn’t delete this login.'));
      setDeleting(false);
    }
  };

  const copy = (text: string, which: 'user' | 'pass') => {
    navigator.clipboard?.writeText(text).catch(() => undefined);
    setCopied(which);
    setTimeout(() => setCopied((c) => (c === which ? null : c)), 2000);
  };

  const site = originLabel(login.origin || `https://${login.domain}`);
  return (
    <li
      data-login-row={login.id}
      className={`p-2.5 rounded-xl border text-xs bg-zinc-900/60 ${onThisPage ? 'border-emerald-500/40' : 'border-zinc-800/80 hover:border-zinc-700'}`}
    >
      <div className="flex items-center justify-between gap-2 mb-1.5">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="font-semibold text-zinc-200 truncate" title={login.origin}>
            {site}
          </span>
          {onThisPage && (
            <span className="px-1.5 rounded bg-emerald-500/10 text-emerald-300 border border-emerald-500/20 text-[9px] font-mono flex-shrink-0">This page</span>
          )}
        </div>
        <div className="flex items-center gap-0.5 flex-shrink-0">
          {onOpen && login.origin && (
            <button
              type="button"
              onClick={() => onOpen(login.origin)}
              aria-label={`Open ${site}`}
              title="Open this site in the browser"
              className="text-zinc-400 hover:text-sky-400 p-1.5 rounded hover:bg-zinc-800/60"
            >
              <ExternalLink size={12} />
            </button>
          )}
          <button
            type="button"
            onClick={() => setConfirmDelete(true)}
            aria-label={`Delete login for ${site}`}
            title="Delete"
            className="text-zinc-500 hover:text-rose-400 p-1.5 rounded hover:bg-zinc-800/60"
          >
            <Trash2 size={12} />
          </button>
        </div>
      </div>

      <div className="flex items-center justify-between py-1 px-2 rounded bg-zinc-950/70 border border-zinc-800/50 mb-1">
        <span className="text-[11px] font-mono text-zinc-300 truncate">{login.username || <span className="text-zinc-500 font-sans">No username</span>}</span>
        {login.username && (
          <button type="button" onClick={() => copy(login.username, 'user')} aria-label="Copy username" className="text-zinc-500 hover:text-zinc-200 p-1 ml-1">
            {copied === 'user' ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
          </button>
        )}
      </div>

      <div className="flex items-center justify-between py-1 px-2 rounded bg-zinc-950/70 border border-zinc-800/50">
        <span className="text-[11px] font-mono text-zinc-400 truncate" aria-live="polite">
          {revealed !== null ? <span data-revealed className="text-zinc-100">{revealed}</span> : <span aria-label="Password hidden">••••••••••••</span>}
        </span>
        <div className="flex items-center gap-0.5 ml-1 flex-shrink-0">
          {revealed !== null && (
            <button type="button" onClick={() => copy(revealed, 'pass')} aria-label="Copy password" className="text-zinc-500 hover:text-zinc-200 p-1">
              {copied === 'pass' ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
            </button>
          )}
          <button
            type="button"
            onClick={reveal}
            disabled={revealing}
            className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/60 disabled:opacity-50"
          >
            {revealing ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" /> : revealed !== null ? <EyeOff size={11} /> : <Eye size={11} />}
            {revealed !== null ? 'Hide' : 'Reveal'}
          </button>
        </div>
      </div>
      {revealed !== null && <p className="mt-1 text-[10px] text-zinc-500">Hides again in 30 seconds.</p>}
      {asking && revealed === null && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            reveal();
          }}
          className="mt-1.5 flex items-end gap-1.5"
          data-ask-master
          noValidate
        >
          <label className="flex-1 min-w-0">
            <span className="text-[10px] text-zinc-400 block mb-0.5">Bitwarden master password, to show it</span>
            <input
              ref={masterRef}
              type="password"
              autoFocus
              autoComplete="off"
              className="w-full px-2 py-1 bg-zinc-950 border border-zinc-800 rounded text-[11px] text-zinc-200 focus:outline-none focus:border-sky-500"
            />
          </label>
          <button
            type="button"
            onClick={() => {
              if (masterRef.current) masterRef.current.value = '';
              setAsking(false);
              setError(null);
            }}
            className="px-2 py-1 rounded text-[11px] bg-zinc-800 hover:bg-zinc-700 text-zinc-200"
          >
            Cancel
          </button>
          <button type="submit" disabled={revealing} className="px-2 py-1 rounded text-[11px] font-semibold bg-sky-500 hover:bg-sky-400 text-zinc-950 disabled:opacity-50">
            Show
          </button>
        </form>
      )}

      {confirmDelete && (
        <div role="alertdialog" aria-label={`Delete login for ${site}?`} className="mt-2 p-2 rounded-lg border border-rose-500/30 bg-rose-500/5 space-y-1.5">
          <p className="text-[11px] text-zinc-200">Delete this login? The bots can’t sign in to {site} with it any more.</p>
          <div className="flex justify-end gap-1.5">
            <button type="button" autoFocus onClick={() => setConfirmDelete(false)} className="px-2.5 py-1 rounded text-[11px] bg-zinc-800 hover:bg-zinc-700 text-zinc-200">
              Keep
            </button>
            <button type="button" disabled={deleting} onClick={remove} className="px-2.5 py-1 rounded text-[11px] font-semibold bg-rose-500 hover:bg-rose-400 text-zinc-950 disabled:opacity-50">
              {deleting ? 'Deleting…' : 'Delete'}
            </button>
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="mt-1.5 flex items-center gap-1.5 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
      {login.last_used && <p className="mt-1 text-[9px] text-zinc-500 text-right">Last used {new Date(login.last_used).toLocaleDateString()}</p>}
    </li>
  );
};

const AddLoginForm: React.FC<{ sandboxId: string; onSaved: () => void; onCancel: () => void }> = ({ sandboxId, onSaved, onCancel }) => {
  const [site, setSite] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const origin = originFromInput(site);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!origin) {
      setError('Enter an address like example.com or https://example.com.');
      return;
    }
    if (!password) return;
    setBusy(true);
    setError(null);
    try {
      await passwordsApi.add(sandboxId, origin, username.trim(), password);
      setPassword('');
      playSfx('coin');
      onSaved();
    } catch (err) {
      setError(errorText(err, 'Couldn’t save this login.'));
      // It can't be saved here until Bitwarden is in use or approvals are back on: don't keep it typed.
      if (err instanceof PasswordsError && err.needsBitwarden) setPassword('');
    } finally {
      setBusy(false);
    }
  };

  const field = 'w-full px-2 py-1.5 bg-zinc-950 border border-zinc-800 rounded text-[11px] text-zinc-200 focus:outline-none focus:border-sky-500 font-mono';
  return (
    <form onSubmit={submit} className="p-3 bg-zinc-900/90 border border-sky-500/30 rounded-xl space-y-2 text-xs">
      <div className="text-[11px] font-semibold text-sky-400 flex items-center gap-1">
        <Key size={12} aria-hidden="true" /> Add a login
      </div>
      <label className="block">
        <span className="text-[10px] text-zinc-400 block mb-0.5">Site</span>
        <input type="text" required value={site} onChange={(e) => setSite(e.target.value)} placeholder="example.com" className={field} autoComplete="off" spellCheck={false} />
        {origin && <span className="text-[10px] text-zinc-500 block mt-0.5">Filled in only on {origin}</span>}
      </label>
      <label className="block">
        <span className="text-[10px] text-zinc-400 block mb-0.5">Username or email</span>
        <input type="text" value={username} onChange={(e) => setUsername(e.target.value)} placeholder="you@example.com" className={field} autoComplete="off" spellCheck={false} />
      </label>
      <label className="block">
        <span className="text-[10px] text-zinc-400 block mb-0.5">Password</span>
        <input type="password" required value={password} onChange={(e) => setPassword(e.target.value)} className={field} autoComplete="new-password" />
      </label>
      {error && (
        <p role="alert" className="flex items-center gap-1.5 text-[11px] text-rose-300">
          <AlertTriangle size={12} aria-hidden="true" /> {error}
        </p>
      )}
      <div className="flex justify-end gap-1.5 pt-1">
        <button type="button" onClick={onCancel} className="px-2.5 py-1 text-[11px] rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-300">
          Cancel
        </button>
        <button type="submit" disabled={busy || !password} className="px-2.5 py-1 text-[11px] rounded bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold disabled:opacity-50">
          {busy ? 'Saving…' : 'Save login'}
        </button>
      </div>
    </form>
  );
};

/** The Saved logins tab: site and username only, Reveal per row, Delete with a confirm. */
export const SavedPasswordsPanel: React.FC<{
  sandboxId: string;
  state: SavedPasswordsState;
  pageUrl: string;
  onOpen?: (origin: string) => void;
  /** Where passwords are kept (Bitwarden or this server); missing on older servers. */
  store?: PasswordStoreState;
  /** Some bot runs without asking: say plainly what autofill then exposes. */
  approvalsOff?: boolean;
  /** Tests open the add form or the unlock dialog. */
  initialAdding?: boolean;
  initialUnlocking?: boolean;
}> = ({ sandboxId, state, pageUrl, onOpen, store, approvalsOff = false, initialAdding = false, initialUnlocking = false }) => {
  const [query, setQuery] = useState('');
  const [adding, setAdding] = useState(initialAdding);
  const [unlocking, setUnlocking] = useState(initialUnlocking);
  const shown = filterLogins(state.logins, query);
  const blocked = state.problem !== null;
  const bitwarden = store?.status?.store === 'bitwarden';
  const openUnlock = useCallback(() => setUnlocking(true), []);
  const closeUnlock = useCallback(() => setUnlocking(false), []);
  // The list says locked but the bar still shows unlocked (Bitwarden locked elsewhere): catch up.
  const storeReload = store?.reload;
  const staleUnlocked = state.problem === 'locked' && store?.status?.locked === false;
  useEffect(() => {
    if (staleUnlocked) storeReload?.();
  }, [staleUnlocked, storeReload]);

  return (
    <div className="flex-1 flex flex-col p-3 gap-2 overflow-hidden bg-zinc-950" data-saved-passwords>
      {store && state.problem !== 'needs-token' && <PasswordStoreBar store={store} onUnlock={openUnlock} onChanged={state.reload} />}
      {store && state.problem !== 'needs-token' && <ExposedBanner store={store} onApprovalsOn={state.reload} />}
      {!blocked && (
        <div className="flex items-center gap-1.5">
          <label className="flex-1 flex items-center gap-1.5 px-2 py-1.5 bg-zinc-900 border border-zinc-800 rounded-lg text-xs focus-within:border-sky-500">
            <Search size={12} className="text-zinc-500 flex-shrink-0" aria-hidden="true" />
            <span className="sr-only">Search saved logins</span>
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search sites or usernames"
              className="w-full bg-transparent text-zinc-200 text-[11px] focus:outline-none placeholder:text-zinc-600"
            />
          </label>
          <button
            type="button"
            aria-expanded={adding}
            onClick={() => setAdding((a) => !a)}
            className={`flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-[11px] font-medium transition-colors ${
              adding ? 'bg-zinc-800 text-zinc-200' : 'bg-sky-500/10 text-sky-400 border border-sky-500/20 hover:bg-sky-500/20'
            }`}
          >
            <Plus size={12} aria-hidden="true" /> {adding ? 'Close' : 'Add'}
          </button>
        </div>
      )}

      {adding && !blocked && (
        <AddLoginForm
          sandboxId={sandboxId}
          onCancel={() => setAdding(false)}
          onSaved={() => {
            setAdding(false);
            state.reload();
          }}
        />
      )}

      <div className="flex-1 overflow-y-auto pr-0.5">
        {state.problem ? (
          <PasswordsProblemNotice problem={state.problem} onRetry={state.reload} onUnlock={store ? openUnlock : undefined} />
        ) : state.loading ? (
          <p className="flex items-center justify-center gap-2 py-10 text-xs text-zinc-500" role="status">
            <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Loading saved logins…
          </p>
        ) : shown.length === 0 ? (
          <div className="text-center py-10 px-4 text-zinc-500 text-xs">
            <Lock size={28} className="mx-auto mb-2 opacity-30 text-zinc-400" aria-hidden="true" />
            <p className="font-medium text-zinc-400">{state.logins.length === 0 ? 'No saved logins yet' : 'No logins match'}</p>
            {state.logins.length === 0 && (
              <p className="text-[11px] text-zinc-500 mt-1 leading-relaxed">
                When a bot signs in on a site, you’re asked whether to save that login. You can also add one here.
              </p>
            )}
          </div>
        ) : (
          <ul className="space-y-2" aria-label="Saved logins">
            {shown.map((l) => (
              <SavedLoginRow
                key={l.id}
                sandboxId={sandboxId}
                login={l}
                onThisPage={loginMatchesPage(l, pageUrl)}
                onOpen={onOpen}
                onDeleted={state.reload}
                askMasterPassword={bitwarden}
              />
            ))}
          </ul>
        )}
      </div>

      <div className="p-2 rounded-lg bg-zinc-900/40 border border-zinc-800/60 space-y-1.5 text-[10px] text-zinc-400">
        <div className="flex items-start gap-2">
          <ShieldCheck size={14} className="text-emerald-400 flex-shrink-0 mt-0.5" aria-hidden="true" />
          <p className="leading-snug">
            <span className="text-zinc-300 font-medium">{bitwarden ? 'Kept in your Bitwarden, locked after every restart.' : 'Encrypted on the server, out of the bots’ folders.'}</span> A login is filled in only on the exact address it was saved for.
          </p>
        </div>
        {/* The exposed banner above already says it. */}
        {(approvalsOff || store?.status?.approvals_off) && !store?.status?.exposed && <AutofillGapNote status={store?.status} />}
      </div>
      {unlocking && store && (
        <UnlockDialog
          status={store.status}
          onClose={closeUnlock}
          onUnlocked={(next) => {
            store.set(next);
            state.reload();
          }}
        />
      )}
    </div>
  );
};
