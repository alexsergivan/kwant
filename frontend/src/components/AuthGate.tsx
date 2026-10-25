import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { Check, ChevronRight, Copy, KeyRound, Loader2, RefreshCw, Terminal, X } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import {
  PERSON_PROMPT_CLOSED_EVENT,
  PERSON_SIGNED_IN_EVENT,
  PERSON_TOKEN_EVENT,
  UNAUTHORIZED_EVENT,
  LINK_SPENT,
  consumeTokenLink,
  endSession,
  extractHashToken,
  fetchAuthStatus,
  installUnauthorizedListener,
  redeemSignIn,
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
    const onUnauthorized = () => check('Your session ended. Sign in again to continue.');
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

/** Where a locked page is: Docker publishes kwant on 127.0.0.1 and still asks for the token there. */
export function isLoopback(hostname: string): boolean {
  const h = hostname.replace(/^\[|\]$/g, '').toLowerCase();
  return h === 'localhost' || h === '::1' || h.startsWith('127.');
}

/**
 * The SSH tunnel that skips the token, or null when it wouldn't work: only for kwant itself on plain http at a
 * network address with its port in the URL. Behind an HTTPS proxy, or with no port, the page can't know kwant's port.
 */
export function tunnelCommand(loc: Pick<Location, 'protocol' | 'hostname' | 'port'>): { command: string; url: string } | null {
  if (loc.protocol !== 'http:' || !loc.port || !loc.hostname || isLoopback(loc.hostname)) return null;
  const host = loc.hostname.includes(':') ? `[${loc.hostname}]` : loc.hostname;
  return { command: `ssh -N -L ${loc.port}:127.0.0.1:${loc.port} user@${host}`, url: `http://127.0.0.1:${loc.port}` };
}

const DOCKER_COMMAND = 'docker compose exec kwnt kwnt token --raw';
const FILE_COMMAND = 'cat ~/.kwnt/api-token';
const DOCKER_LINK_COMMAND = 'docker compose exec kwnt kwnt signin-link';
const LINK_COMMAND = 'kwnt signin-link';

/** The token from what was pasted: the token itself, or the whole sign-in link. */
export function tokenFromPaste(value: string): string {
  const v = value.trim();
  const hash = v.indexOf('#');
  if (hash >= 0 && v.includes('kwnt_token=')) return extractHashToken(v.slice(hash)).token ?? '';
  return v;
}

/** One command, selectable in a tap, with a copy button where the browser allows it. */
const Command: React.FC<{ children: string }> = ({ children }) => {
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
  );
};

/**
 * The fallback: paste a link or the API token (`children`), and the ways to get the token, folded
 * away. `open` when a pasted value was refused, so its error stays in view. Exported for tests.
 */
export const OtherSetups: React.FC<{
  loc: Pick<Location, 'protocol' | 'hostname' | 'port'>;
  docker: boolean;
  open?: boolean;
  onToggle?: (open: boolean) => void;
  children?: React.ReactNode;
}> = ({ loc, docker, open, onToggle, children }) => {
  const tunnel = tunnelCommand(loc);
  return (
    <details className="group text-xs text-zinc-400" open={open} onToggle={(e) => onToggle?.((e.currentTarget as HTMLDetailsElement).open)}>
      <summary className="flex items-center gap-1 w-fit min-h-[32px] cursor-pointer select-none list-none [&::-webkit-details-marker]:hidden text-zinc-400 hover:text-zinc-200 rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500">
        <ChevronRight size={13} className="transition-transform group-open:rotate-90 motion-reduce:transition-none" aria-hidden="true" />
        Other setups
      </summary>
      <div className="mt-2 space-y-2.5 leading-relaxed">
        {children}
        <p className="text-[11px] text-zinc-500 leading-snug">Get the access token on the machine running kwant bots:</p>
        <Setup label={docker ? 'Docker, in the folder with docker-compose.yml' : 'As the user that runs kwant bots'}>{docker ? DOCKER_COMMAND : FILE_COMMAND}</Setup>
        {docker ? (
          <Setup label="Without Docker, as the user that runs kwant bots">{FILE_COMMAND}</Setup>
        ) : (
          <Setup label="Docker, in the folder with docker-compose.yml">{DOCKER_COMMAND}</Setup>
        )}
        <Setup label="systemd service">sudo cat /var/lib/kwnt/api-token</Setup>
        <Setup label="Built from source and not on your PATH">./target/release/kwnt token --raw</Setup>
        {tunnel && (
          <Setup label={`No sign-in: open an SSH tunnel on this computer, then go to ${tunnel.url} (not for Docker)`}>{tunnel.command}</Setup>
        )}
      </div>
    </details>
  );
};

const Setup: React.FC<{ label: string; children: string }> = ({ label, children }) => (
  <div className="space-y-1">
    <p className="text-[11px] text-zinc-500 leading-snug">{label}</p>
    <Command>{children}</Command>
  </div>
);

/** After a rejected token, focus and select the box so the next paste replaces it. Exported for tests. */
export const selectForRetry = (el: Pick<HTMLInputElement, 'focus' | 'select'> | null) => {
  if (!el) return;
  el.focus();
  el.select();
};

export const WRONG_TOKEN = "That token isn't right. It has to come from the machine running kwant bots, as the user running it.";
/** Only shown with the wrong-token error, not up front. */
export const OWN_TOKEN_HINT = "Each install has its own token, so one copied from another computer won't work here.";

export const TokenForm: React.FC<{ notice: string | null; onSignedIn: () => void; personOnly?: boolean; onCancel?: () => void }> = ({
  notice,
  onSignedIn,
  personOnly = false,
  onCancel,
}) => {
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [othersOpen, setOthersOpen] = useState(false);
  const loc = typeof window === 'undefined' ? { protocol: 'https:', hostname: '', port: '' } : window.location;
  // A locked page on loopback is Docker, so its command comes first there
  const docker = isLoopback(loc.hostname);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (error && !busy) {
      setOthersOpen(true);
      selectForRetry(inputRef.current);
    }
  }, [error, busy]);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const value = tokenFromPaste(token);
    if (!value) return;
    setBusy(true);
    setError(null);
    try {
      const result = await redeemSignIn(value);
      if (result === 'ok') {
        setToken('');
        onSignedIn();
        return;
      }
      setError(result === 'link_used_or_expired' ? LINK_SPENT : WRONG_TOKEN);
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
          <KeyRound size={15} className="text-sky-400" /> {personOnly ? 'Confirm it’s you' : 'Sign in'}
        </h2>
        {personOnly && (
          <p className="text-xs text-zinc-400 leading-relaxed">
            Saved passwords, answers to approval cards and deleting bots or teams need you signed in from another computer, so a bot can’t do them for you. You need it once per browser.
          </p>
        )}
      </div>
      {notice && (
        <p className={`text-xs leading-relaxed ${notice === LINK_SPENT ? 'text-rose-300' : 'text-amber-300'}`} role="status">
          {notice}
        </p>
      )}
      <div className="space-y-1.5">
        <p className="text-xs text-zinc-400 leading-relaxed">
          Open the one-time sign-in link kwant bots printed when it started, or scan its QR code with your phone. Used or lost it? Make a new one on the
          machine running kwant bots:
        </p>
        <Command>{docker ? DOCKER_LINK_COMMAND : LINK_COMMAND}</Command>
      </div>
      <OtherSetups loc={loc} docker={docker} open={othersOpen} onToggle={setOthersOpen}>
        <div className="space-y-1.5">
          <label htmlFor="kwnt-token" className="block text-[11px] text-zinc-500 leading-snug">
            Paste a sign-in link or the access token
          </label>
          <div className="flex gap-2">
            <input
              id="kwnt-token"
              ref={inputRef}
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={token}
              onChange={(e) => setToken(e.target.value)}
              aria-invalid={!!error}
              aria-describedby={error ? 'kwnt-token-error' : undefined}
              className="flex-1 min-w-0 bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs font-mono text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors"
              placeholder="Link or token"
            />
            <button
              type="submit"
              disabled={busy || !token.trim()}
              className="shrink-0 flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50"
            >
              {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" />} Continue
            </button>
          </div>
          {error && (
            <p id="kwnt-token-error" className="text-[11px] text-rose-400 leading-snug" role="alert">
              {error}
              {error === WRONG_TOKEN && ` ${OWN_TOKEN_HINT}`}
            </p>
          )}
        </div>
      </OtherSetups>
      <p className="text-[11px] text-zinc-500">Stays signed in for a year. Anyone with a sign-in link or the token controls this server.</p>
      {onCancel && (
        <button
          type="button"
          onClick={onCancel}
          className="w-full px-4 py-2 rounded-lg text-xs font-medium text-zinc-200 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 transition-colors"
        >
          Not now
        </button>
      )}
    </form>
  );
};
