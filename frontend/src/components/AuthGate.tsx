import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { Check, Copy, KeyRound, Loader2, RefreshCw, Terminal, X } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import {
  PERSON_PROMPT_CLOSED_EVENT,
  PERSON_SIGNED_IN_EVENT,
  PERSON_TOKEN_EVENT,
  UNAUTHORIZED_EVENT,
  consumeTokenLink,
  endSession,
  extractHashToken,
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

/** What the start screen says when the server didn't answer. */
export function offlineMessage(e: unknown, online: boolean): string {
  if (!online) return "You're offline. kwant bots opens once your phone is connected again.";
  if (e instanceof Error && e.message !== 'Failed to fetch' && e.message !== 'Load failed') return e.message;
  return "Can't reach the kwant bots server. Check that it's running and that this device can reach it.";
}

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
      setState({ kind: 'offline', message: offlineMessage(e, navigator.onLine) });
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
    // Back on the network (the installed app opened offline): try again without a tap.
    const onOnline = () => check();
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    window.addEventListener('hashchange', onHash);
    window.addEventListener('online', onOnline);
    return () => {
      cancelled = true;
      window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
      window.removeEventListener('hashchange', onHash);
      window.removeEventListener('online', onOnline);
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
            onClose={() => {
              setPersonPrompt(false);
              window.dispatchEvent(new Event(PERSON_PROMPT_CLOSED_EVENT));
            }}
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
    <div className="min-h-[100dvh] w-screen flex items-center justify-center bg-zinc-950 text-zinc-100 p-4 pt-[max(1rem,env(safe-area-inset-top))] pb-[max(1rem,env(safe-area-inset-bottom))] font-sans">
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
              className="flex items-center gap-1.5 min-h-[44px] px-4 rounded-lg text-xs font-medium bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 transition-colors"
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

/** Where kwant runs, as the person sees it: decides which steps get the token. */
export type TokenPlace = 'server' | 'docker' | 'here';

/** The port kwant listens on, as far as this page can tell (behind a proxy it can't). */
function kwantPort(loc: Pick<Location, 'port'>): string {
  return loc.port || '8799';
}

/** Docker publishes kwant on 127.0.0.1 and still asks for the token there, so a locked loopback page is Docker. */
export function guessPlace(hostname: string): TokenPlace {
  const h = hostname.replace(/^\[|\]$/g, '').toLowerCase();
  return h === 'localhost' || h === '::1' || h.startsWith('127.') ? 'docker' : 'server';
}

/** The token from what was pasted: the token itself, or the whole sign-in link. */
export function tokenFromPaste(value: string): string {
  const v = value.trim();
  const hash = v.indexOf('#');
  if (hash >= 0 && v.includes('kwnt_token=')) return extractHashToken(v.slice(hash)).token ?? '';
  return v;
}

const PLACES: { value: TokenPlace; label: string }[] = [
  { value: 'server', label: 'On a server' },
  { value: 'docker', label: 'In Docker' },
  { value: 'here', label: 'On this computer' },
];

/** One command, selectable in a tap, with a copy button where the browser allows it. */
const Command: React.FC<{ children: string; note?: string }> = ({ children, note }) => {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      // Plain http on an IP isn't a secure context, so the Clipboard API may be missing
      if (navigator.clipboard?.writeText) await navigator.clipboard.writeText(children);
      else {
        const area = document.createElement('textarea');
        area.value = children;
        area.style.position = 'fixed';
        area.style.opacity = '0';
        document.body.appendChild(area);
        area.select();
        const ok = document.execCommand('copy');
        area.remove();
        if (!ok) return;
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Nothing to do: the command stays selectable
    }
  };
  return (
    <div className="space-y-0.5">
      <div className="flex items-start gap-1.5 bg-zinc-950 border border-zinc-800 rounded-md pl-2 pr-1 py-1">
        <Terminal size={12} className="shrink-0 mt-[7px] text-zinc-500" aria-hidden="true" />
        <code className="flex-1 min-w-0 py-1 break-words select-all text-[11px] font-mono text-zinc-200">{children}</code>
        <button
          type="button"
          onClick={copy}
          aria-label={copied ? 'Copied' : `Copy: ${children}`}
          className="shrink-0 p-1.5 rounded text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
        >
          {copied ? <Check size={12} className="text-emerald-400" /> : <Copy size={12} />}
        </button>
      </div>
      {note && <p className="text-[11px] text-zinc-500 leading-snug">{note}</p>}
    </div>
  );
};

/** How to get the token, for where kwant runs. Exported for tests. */
export const TokenSteps: React.FC<{ place: TokenPlace; hostname: string; port: string }> = ({ place, hostname, port }) => {
  if (place === 'here') {
    const url = `http://127.0.0.1:${port}`;
    return (
      <div className="space-y-2 text-xs text-zinc-400 leading-relaxed">
        <p>
          Open kwant bots at{' '}
          <a href={url} className="font-mono text-sky-400 hover:text-sky-300 underline underline-offset-2">
            {url}
          </a>{' '}
          instead. On the computer it runs on, kwant bots doesn't ask for a token, but only at <span className="font-mono">127.0.0.1</span> or{' '}
          <span className="font-mono">localhost</span>, not at this computer's network address.
        </p>
        <p>
          Started with Docker? Docker still asks for the token: choose <span className="text-zinc-200">In Docker</span>.
        </p>
      </div>
    );
  }
  if (place === 'docker') {
    return (
      <div className="space-y-2 text-xs text-zinc-400 leading-relaxed">
        <p>On the machine that runs Docker (over SSH if it's a server), in the folder with <span className="font-mono">docker-compose.yml</span>:</p>
        <Command>docker compose exec kwnt kwnt token --raw</Command>
        <Command note="Started with docker run instead? Use your container's name or ID (docker ps lists it).">
          {'docker exec <container> kwnt token --raw'}
        </Command>
      </div>
    );
  }
  const host = hostname.includes(':') ? `[${hostname}]` : hostname;
  return (
    <div className="space-y-2 text-xs text-zinc-400 leading-relaxed">
      <p>SSH into the server as the user that runs kwant bots, then run:</p>
      <Command note="Running it as a systemd service with KWNT_DATA_DIR? Read that folder instead, e.g. sudo cat /var/lib/kwnt/api-token.">
        cat ~/.kwnt/api-token
      </Command>
      <p>
        Or <span className="font-mono">kwnt token --raw</span>. Built from source and not on your PATH? Run it from the source folder:
      </p>
      <Command>./target/release/kwnt token --raw</Command>
      <p className="pt-1">
        <span className="text-zinc-200">No token at all:</span> on this computer, open an SSH tunnel, then go to{' '}
        <span className="font-mono">http://127.0.0.1:{port}</span>. kwant bots sees the tunnel as local. Not for Docker.
      </p>
      <Command>{`ssh -N -L ${port}:127.0.0.1:${port} user@${host}`}</Command>
    </div>
  );
};

/** Shown when the server rejects a pasted token. Exported for tests. */
/** After a rejected token, focus and select the box so the next paste replaces it. Exported for tests. */
export const selectForRetry = (el: Pick<HTMLInputElement, 'focus' | 'select'> | null) => {
  if (!el) return;
  el.focus();
  el.select();
};

export const WRONG_TOKEN = "That token isn't right. It has to come from the machine running kwant bots, as the user running it.";

export const TokenForm: React.FC<{ notice: string | null; onSignedIn: () => void; personOnly?: boolean; onCancel?: () => void }> = ({
  notice,
  onSignedIn,
  personOnly = false,
  onCancel,
}) => {
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const hostname = typeof window === 'undefined' ? 'your-server' : window.location.hostname || 'your-server';
  const port = typeof window === 'undefined' ? '8799' : kwantPort(window.location);
  const [place, setPlace] = useState<TokenPlace>(() => guessPlace(hostname));
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (error && !busy) selectForRetry(inputRef.current);
  }, [error, busy]);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const value = tokenFromPaste(token);
    if (!value) return;
    setBusy(true);
    setError(null);
    try {
      if (await startSession(value)) {
        setToken('');
        onSignedIn();
        return;
      }
      setError(WRONG_TOKEN);
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
            ? 'Saved passwords, answers to approval cards and deleting bots or teams need your access token from another computer, so a bot can’t do them for you. You need it once per browser.'
            : 'You’re opening kwant bots from another computer, so this browser needs its access token once.'}
        </p>
      </div>
      <fieldset className="space-y-3">
        <legend className="text-xs font-medium text-zinc-300 mb-2">Where does kwant bots run?</legend>
        <div className="grid grid-cols-3 gap-1 p-1 bg-zinc-950 border border-zinc-800 rounded-lg">
          {PLACES.map((p) => (
            <label
              key={p.value}
              className={`relative flex items-center justify-center text-center min-h-[36px] px-1.5 rounded-md text-[11px] font-medium cursor-pointer transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-sky-500 ${
                place === p.value ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-400 hover:text-zinc-200'
              }`}
            >
              <input
                type="radio"
                name="kwnt-token-place"
                value={p.value}
                checked={place === p.value}
                onChange={() => setPlace(p.value)}
                className="sr-only"
              />
              {p.label}
            </label>
          ))}
        </div>
        <TokenSteps place={place} hostname={hostname} port={port} />
      </fieldset>
      {notice && <p className="text-xs text-amber-300" role="status">{notice}</p>}
      {place !== 'here' && (
        <div>
          <label htmlFor="kwnt-token" className="block text-xs font-medium text-zinc-400 mb-1">
            Token
          </label>
          <p id="kwnt-token-hint" className="text-[11px] text-zinc-500 leading-snug mb-1.5">
            Each install has its own token, so a token copied from another computer won't work here.
          </p>
          <input
            id="kwnt-token"
            ref={inputRef}
            type="password"
            autoComplete="off"
            spellCheck={false}
            autoFocus
            value={token}
            onChange={(e) => setToken(e.target.value)}
            aria-invalid={!!error}
            aria-describedby={error ? ['kwnt-token-hint', 'kwnt-token-error'].join(' ') : 'kwnt-token-hint'}
            className="w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs font-mono text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
            placeholder="Paste the token or the sign-in link"
          />
          {error && (
            <p id="kwnt-token-error" className="mt-1.5 text-[11px] text-rose-400" role="alert">
              {error}
            </p>
          )}
        </div>
      )}
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
        {place !== 'here' && (
          <button
            type="submit"
            disabled={busy || !token.trim()}
            className="flex-1 w-full flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50"
          >
            {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" />} Continue
          </button>
        )}
      </div>
      <p className="text-[11px] text-zinc-500">The browser keeps you signed in for a year. Anyone with the token controls this server.</p>
    </form>
  );
};
