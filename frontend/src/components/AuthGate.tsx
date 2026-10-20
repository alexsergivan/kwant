import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { KeyRound, Loader2, RefreshCw, Terminal, X } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import {
  PERSON_SIGNED_IN_EVENT,
  PERSON_TOKEN_EVENT,
  UNAUTHORIZED_EVENT,
  consumeTokenLink,
  endSession,
  fetchAuthStatus,
  installUnauthorizedListener,
  startSession,
} from '../auth';

interface AuthContextValue {
  /** True when this browser needs the API token (Docker, a VPS reached directly). */
  authRequired: boolean;
  signOut: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue>({ authRequired: false, signOut: async () => undefined });

export const useAuth = () => useContext(AuthContext);

type GateState =
  | { kind: 'checking' }
  | { kind: 'ready'; authRequired: boolean }
  | { kind: 'locked'; notice: string | null }
  | { kind: 'offline'; message: string };

installUnauthorizedListener();

/** Shows its children once the API answers, and the token screen while it needs a token. */
export const AuthGate: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [state, setState] = useState<GateState>({ kind: 'checking' });
  // A person-only action (passwords, approvals) answered 401: ask for the token over the app.
  const [personPrompt, setPersonPrompt] = useState(false);
  const checking = useRef(false);

  useEffect(() => {
    const onPerson = () => setPersonPrompt(true);
    window.addEventListener(PERSON_TOKEN_EVENT, onPerson);
    return () => window.removeEventListener(PERSON_TOKEN_EVENT, onPerson);
  }, []);

  const check = useCallback(async (notice: string | null = null) => {
    if (checking.current) return;
    checking.current = true;
    try {
      const s = await fetchAuthStatus();
      setState(s.auth_required && !s.authenticated ? { kind: 'locked', notice } : { kind: 'ready', authRequired: s.auth_required });
    } catch (e) {
      setState({ kind: 'offline', message: e instanceof Error && e.message !== 'Failed to fetch' ? e.message : "Can't reach the kwant bots server. Is it running?" });
    } finally {
      checking.current = false;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    consumeTokenLink().then((linkError) => {
      if (!cancelled) check(linkError);
    });
    const onUnauthorized = () => check('Your session ended. Paste the token again to continue.');
    // A token link opened in a tab that already shows kwnt only changes the hash.
    const onHash = () => {
      if (window.location.hash.includes('kwnt_token=')) consumeTokenLink().then((linkError) => check(linkError));
    };
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    window.addEventListener('hashchange', onHash);
    return () => {
      cancelled = true;
      window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
      window.removeEventListener('hashchange', onHash);
    };
  }, [check]);

  const signOut = useCallback(async () => {
    try {
      await endSession();
    } finally {
      setState({ kind: 'locked', notice: 'Signed out.' });
    }
  }, []);

  if (state.kind === 'ready') {
    return (
      <AuthContext.Provider value={{ authRequired: state.authRequired, signOut }}>
        {children}
        {personPrompt && (
          <PersonTokenDialog
            onClose={() => setPersonPrompt(false)}
            onSignedIn={() => {
              setPersonPrompt(false);
              window.dispatchEvent(new Event(PERSON_SIGNED_IN_EVENT));
            }}
          />
        )}
      </AuthContext.Provider>
    );
  }

  return (
    <div className="min-h-screen w-screen flex items-center justify-center bg-zinc-950 text-zinc-100 p-4 font-sans">
      <div className="w-full max-w-md bg-zinc-900 border border-zinc-800 rounded-2xl p-6 shadow-2xl space-y-5">
        <div className="flex items-center gap-3">
          <ModernAvatar role="brand" name="kwant bots" size={36} />
          <div>
            <h1 className="text-sm font-semibold">kwant bots</h1>
            <p className="text-xs text-zinc-400">Your team of AI bots</p>
          </div>
        </div>
        {state.kind === 'checking' && (
          <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
            <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Connecting…
          </p>
        )}
        {state.kind === 'offline' && (
          <div className="space-y-3" role="alert">
            <p className="text-xs text-rose-300">{state.message}</p>
            <button
              type="button"
              onClick={() => {
                setState({ kind: 'checking' });
                check();
              }}
              className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs font-medium bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 transition-colors"
            >
              <RefreshCw size={13} /> Try again
            </button>
          </div>
        )}
        {state.kind === 'locked' && <TokenForm notice={state.notice} onSignedIn={() => check()} />}
      </div>
    </div>
  );
};

/** The token screen as a dialog, for an action only the person may take. The app stays open under it. */
export const PersonTokenDialog: React.FC<{ onClose: () => void; onSignedIn: () => void }> = ({ onClose, onSignedIn }) => {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 p-4 font-sans" onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="kwnt-person-token-title"
        className="relative w-full max-w-md bg-zinc-900 border border-zinc-800 rounded-2xl p-6 shadow-2xl text-zinc-100"
        onClick={(e) => e.stopPropagation()}
      >
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="absolute top-3 right-3 p-1.5 rounded-lg text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
        >
          <X size={14} />
        </button>
        <TokenForm notice={null} onSignedIn={onSignedIn} personOnly onCancel={onClose} />
      </div>
    </div>
  );
};

const TokenForm: React.FC<{ notice: string | null; onSignedIn: () => void; personOnly?: boolean; onCancel?: () => void }> = ({
  notice,
  onSignedIn,
  personOnly = false,
  onCancel,
}) => {
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const value = token.trim();
    if (!value) return;
    setBusy(true);
    setError(null);
    try {
      if (await startSession(value)) {
        setToken('');
        onSignedIn();
        return;
      }
      setError("That token isn't right. Copy it again from the command below.");
    } catch (err) {
      setError(err instanceof Error && err.message !== 'Failed to fetch' ? err.message : 'Could not reach the server.');
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <div className="space-y-1.5">
        <h2 id={personOnly ? 'kwnt-person-token-title' : undefined} className="flex items-center gap-2 text-sm font-medium">
          <KeyRound size={15} className="text-sky-400" /> {personOnly ? 'Confirm it’s you' : 'Enter your access token'}
        </h2>
        <p className="text-xs text-zinc-400 leading-relaxed">
          {personOnly
            ? 'Saved passwords and answers to approval cards need your access token, even on this computer, so a bot can’t reach them. You need it once per browser. Get it on the server:'
            : 'This server is reachable from other machines, so it needs its token once per browser. Get it on the server:'}
        </p>
        <div className="space-y-1 text-[11px] font-mono text-zinc-300">
          <p className="flex items-center gap-1.5 bg-zinc-950 border border-zinc-800 rounded-md px-2 py-1">
            <Terminal size={12} className="text-zinc-500" /> kwnt token
          </p>
          <p className="flex items-center gap-1.5 bg-zinc-950 border border-zinc-800 rounded-md px-2 py-1">
            <Terminal size={12} className="text-zinc-500" /> docker compose exec kwnt kwnt token
          </p>
        </div>
      </div>
      {notice && <p className="text-xs text-amber-300" role="status">{notice}</p>}
      <div>
        <label htmlFor="kwnt-token" className="block text-xs font-medium text-zinc-400 mb-1">
          Token
        </label>
        <input
          id="kwnt-token"
          type="password"
          autoComplete="off"
          spellCheck={false}
          autoFocus
          value={token}
          onChange={(e) => setToken(e.target.value)}
          aria-invalid={!!error}
          aria-describedby={error ? 'kwnt-token-error' : undefined}
          className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs font-mono text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
          placeholder="Paste the token"
        />
        {error && (
          <p id="kwnt-token-error" className="mt-1.5 text-[11px] text-rose-400" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="flex gap-2">
        {onCancel && (
          <button
            type="button"
            onClick={onCancel}
            className="flex-1 px-4 py-2 rounded-lg text-xs font-medium text-zinc-200 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 transition-colors"
          >
            Not now
          </button>
        )}
        <button
          type="submit"
          disabled={busy || !token.trim()}
          className="flex-1 w-full flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50"
        >
          {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" />} Continue
        </button>
      </div>
      <p className="text-[11px] text-zinc-500">The browser keeps you signed in for a year. Anyone with the token controls this server.</p>
    </form>
  );
};
