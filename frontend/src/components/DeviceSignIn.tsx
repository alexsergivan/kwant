import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, Check, Copy, Loader2, MonitorSmartphone, QrCode, RefreshCw, ShieldCheck, X } from 'lucide-react';
import {
  type DeviceRequest,
  type MintedLink,
  type PendingDevice,
  type SignedInSession,
  ago,
  countdown,
  createSigninLink,
  decideDeviceRequest,
  listDeviceRequests,
  listSessions,
  newLinkSession,
  pollDeviceRequest,
  qrPath,
  revokeSession,
  startDeviceRequest,
  viaLabel,
} from '../deviceSignIn';

// ---------------------------------------------------------------- the new browser: show a code, wait

export type WaitState =
  | { kind: 'idle' }
  | { kind: 'starting' }
  | { kind: 'waiting'; request: DeviceRequest; expiresAt: number }
  | { kind: 'approved' }
  | { kind: 'denied' }
  | { kind: 'expired' }
  | { kind: 'too_many'; retryAfter: number }
  | { kind: 'error'; message: string };

const primary =
  'w-full flex items-center justify-center gap-1.5 min-h-[44px] px-4 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50';
const secondary =
  'flex items-center justify-center gap-1.5 min-h-[44px] px-4 rounded-lg text-xs font-medium text-zinc-200 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 transition-colors';

/** What the "Sign in from another device" part of the card shows in each state. Exported for tests. */
export const DeviceWaitView: React.FC<{ state: WaitState; now?: number; onStart: () => void; onCancel: () => void }> = ({
  state,
  now = Date.now(),
  onStart,
  onCancel,
}) => {
  if (state.kind === 'waiting' || state.kind === 'approved') {
    const code = state.kind === 'waiting' ? state.request.user_code : null;
    return (
      <div className="space-y-3 rounded-xl border border-sky-500/30 bg-sky-500/5 p-4" role="status" aria-live="polite">
        {code ? (
          <>
            <p className="text-xs text-zinc-300 leading-relaxed">
              On a phone or computer where you’re already signed in to kwant bots, open it and approve this code:
            </p>
            <p className="text-center font-mono text-2xl font-semibold tracking-[0.15em] text-zinc-50 select-all" aria-label={`Code ${code.split('').join(' ')}`}>
              {code}
            </p>
            <p className="flex items-center justify-center gap-2 text-[11px] text-zinc-400">
              <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              Waiting for approval · {state.kind === 'waiting' ? countdown(state.expiresAt, now) : ''} left
            </p>
            <button type="button" onClick={onCancel} className={`${secondary} w-full`}>
              Cancel
            </button>
          </>
        ) : (
          <p className="flex items-center justify-center gap-2 text-xs text-emerald-300">
            <ShieldCheck size={14} aria-hidden="true" /> Approved. Signing you in…
          </p>
        )}
      </div>
    );
  }
  const problem =
    state.kind === 'denied'
      ? 'The request was denied on your other device. If that wasn’t you, nothing happened: this browser is still signed out.'
      : state.kind === 'expired'
        ? 'The code expired before it was approved. Codes work for 10 minutes.'
        : state.kind === 'too_many'
          ? `Too many sign-in requests right now. Try again in ${Math.max(1, Math.ceil(state.retryAfter / 60))} min.`
          : state.kind === 'error'
            ? state.message
            : null;
  return (
    <div className="space-y-2">
      {problem && (
        <p className="flex items-start gap-1.5 text-xs text-rose-300 leading-relaxed" role="alert">
          <AlertTriangle size={13} className="shrink-0 mt-0.5" aria-hidden="true" /> {problem}
        </p>
      )}
      <button type="button" onClick={onStart} disabled={state.kind === 'starting'} className={primary}>
        {state.kind === 'starting' ? (
          <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
        ) : problem ? (
          <RefreshCw size={14} aria-hidden="true" />
        ) : (
          <MonitorSmartphone size={14} aria-hidden="true" />
        )}
        {problem ? 'Get a new code' : 'Sign in from another device'}
      </button>
    </div>
  );
};

/** "Sign in from another device": asks for a code, waits for approval and signs in by itself. */
export const DeviceWait: React.FC<{ onSignedIn: () => void }> = ({ onSignedIn }) => {
  const [state, setState] = useState<WaitState>({ kind: 'idle' });
  const [now, setNow] = useState(Date.now());
  // Bumped to stop the current poll loop (cancel, unmount, a new code)
  const run = useRef(0);

  useEffect(() => () => void (run.current += 1), []);

  const start = useCallback(async () => {
    const mine = ++run.current;
    setState({ kind: 'starting' });
    try {
      const started = await startDeviceRequest();
      if (mine !== run.current) return;
      if (started.kind === 'too_many') {
        setState({ kind: 'too_many', retryAfter: started.retryAfter });
        return;
      }
      const { request } = started;
      const expiresAt = Date.now() + request.expires_in * 1000;
      setState({ kind: 'waiting', request, expiresAt });
      const every = Math.max(1, request.interval) * 1000;
      let failures = 0;
      while (mine === run.current) {
        await new Promise((r) => setTimeout(r, every));
        if (mine !== run.current) return;
        setNow(Date.now());
        let result;
        try {
          result = await pollDeviceRequest(request.device_code);
          failures = 0;
        } catch {
          // A blip (phone switching networks): keep waiting, give up after a minute of failures
          if (++failures * every >= 60_000) {
            setState({ kind: 'error', message: 'Lost the connection to the server while waiting. Get a new code to try again.' });
            return;
          }
          continue;
        }
        if (mine !== run.current) return;
        if (result === 'approved') {
          setState({ kind: 'approved' });
          onSignedIn();
          return;
        }
        if (result === 'denied' || result === 'expired') {
          setState({ kind: result });
          return;
        }
        if (Date.now() >= expiresAt) {
          setState({ kind: 'expired' });
          return;
        }
      }
    } catch (e) {
      if (mine === run.current)
        setState({ kind: 'error', message: e instanceof Error && e.message !== 'Failed to fetch' ? e.message : 'Could not reach the server.' });
    }
  }, [onSignedIn]);

  // The countdown ticks between polls too
  useEffect(() => {
    if (state.kind !== 'waiting') return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [state.kind]);

  return (
    <DeviceWaitView
      state={state}
      now={now}
      onStart={start}
      onCancel={() => {
        run.current += 1;
        setState({ kind: 'idle' });
      }}
    />
  );
};

// ---------------------------------------------------------------- a signed-in device: approve or deny

/** The approval card for one waiting device. Exported for tests. */
export const DeviceRequestCard: React.FC<{
  request: PendingDevice;
  more: number;
  busy: 'approve' | 'deny' | null;
  error: string | null;
  now?: number;
  onApprove: () => void;
  onDeny: () => void;
  onLater: () => void;
}> = ({ request, more, busy, error, now = Date.now(), onApprove, onDeny, onLater }) => {
  const [matches, setMatches] = useState(false);
  useEffect(() => setMatches(false), [request.user_code]);
  return (
    <div className="kw-dialog kw-dialog-bottom fixed inset-0 z-[90] flex items-center justify-center bg-black/70 p-4 font-sans">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="kwnt-device-request-title"
        aria-describedby="kwnt-device-request-warning"
        className="kw-dialog-panel relative w-full max-w-sm bg-zinc-900 border border-zinc-800 rounded-2xl p-5 shadow-2xl text-zinc-100 space-y-4"
      >
        <button
          type="button"
          onClick={onLater}
          aria-label="Decide later"
          title="Decide later"
          className="absolute top-3 right-3 p-1.5 rounded-lg text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
        >
          <X size={14} />
        </button>
        <h2 id="kwnt-device-request-title" className="flex items-center gap-2 pr-8 text-sm font-semibold">
          <MonitorSmartphone size={16} className="text-sky-400" aria-hidden="true" /> New device wants access
        </h2>
        <div className="rounded-xl bg-zinc-950 border border-zinc-800 p-3 space-y-1.5 text-center">
          <p className="text-[11px] text-zinc-500">Code</p>
          <p className="font-mono text-2xl font-semibold tracking-[0.15em] text-zinc-50" aria-label={`Code ${request.user_code.split('').join(' ')}`}>
            {request.user_code}
          </p>
          <p className="text-xs text-zinc-300">{request.browser || 'Unknown browser'}</p>
          <p className="text-[11px] text-zinc-500 break-words">
            IP {request.ip || 'unknown'} · asked {ago(request.created_at, now)}
          </p>
        </div>
        <p id="kwnt-device-request-warning" className="text-xs text-amber-200/90 leading-relaxed">
          Approve only if this exact code is on a screen you’re holding right now. That device gets full control of kwant bots, as you. If someone asked you to
          approve a code, deny it.
        </p>
        <label className="flex items-start gap-2 text-xs text-zinc-200 cursor-pointer min-h-[32px]">
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4 accent-sky-500"
            checked={matches}
            onChange={(e) => setMatches(e.target.checked)}
          />
          <span>The code matches the one on my other device</span>
        </label>
        {error && (
          <p className="text-[11px] text-rose-400 leading-snug" role="alert">
            {error}
          </p>
        )}
        <div className="flex gap-2">
          <button type="button" onClick={onDeny} disabled={!!busy} className={`${secondary} flex-1 hover:text-rose-200 disabled:opacity-50`}>
            {busy === 'deny' && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />} Deny
          </button>
          <button
            type="button"
            onClick={onApprove}
            disabled={!matches || !!busy}
            className="flex-1 flex items-center justify-center gap-1.5 min-h-[44px] px-4 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors disabled:opacity-50"
          >
            {busy === 'approve' && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />} Approve
          </button>
        </div>
        {more > 0 && <p className="text-[11px] text-zinc-500 text-center">{more === 1 ? '1 more request is waiting.' : `${more} more requests are waiting.`}</p>}
      </div>
    </div>
  );
};

/** How often a signed-in tab checks for devices waiting to be let in. */
export const REQUEST_CHECK_MS = 4000;

/** Watches for devices waiting to be let in and asks this signed-in person to approve or deny each. */
export const DeviceRequestPrompt: React.FC = () => {
  const [requests, setRequests] = useState<PendingDevice[]>([]);
  // Codes put off with "Decide later", until they expire
  const [later, setLater] = useState<string[]>([]);
  const [busy, setBusy] = useState<'approve' | 'deny' | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await listDeviceRequests();
      if (list) setRequests(list);
    } catch {
      // Offline for a moment; the next check tries again
    }
  }, []);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const tick = async () => {
      if (document.visibilityState === 'visible') await refresh();
      if (!stopped) timer = setTimeout(tick, REQUEST_CHECK_MS);
    };
    tick();
    const onVisible = () => {
      if (document.visibilityState === 'visible') refresh();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      stopped = true;
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [refresh]);

  const open = requests.filter((r) => !later.includes(r.user_code));
  const current = open[0];
  useEffect(() => setError(null), [current?.user_code]);
  if (!current) return null;

  const decide = async (approve: boolean) => {
    setBusy(approve ? 'approve' : 'deny');
    setError(null);
    try {
      // "gone": it expired or was answered on another device; either way it's off this one
      await decideDeviceRequest(current.user_code, approve);
      setRequests((list) => list.filter((r) => r.user_code !== current.user_code));
    } catch (e) {
      setError(e instanceof Error && e.message !== 'Failed to fetch' ? e.message : 'Could not reach the server.');
    } finally {
      setBusy(null);
    }
  };

  return (
    <DeviceRequestCard
      request={current}
      more={open.length - 1}
      busy={busy}
      error={error}
      onApprove={() => decide(true)}
      onDeny={() => decide(false)}
      onLater={() => setLater((l) => [...l, current.user_code])}
    />
  );
};

// ---------------------------------------------------------------- Settings: signed-in devices

export type SessionsState = { kind: 'loading' } | { kind: 'error'; message: string } | { kind: 'ready'; sessions: SignedInSession[] };

/** The list of signed-in browsers. Exported for tests. */
export const SessionsList: React.FC<{
  state: SessionsState;
  revoking: string | null;
  now?: number;
  onRevoke: (id: string) => void;
  onRetry: () => void;
}> = ({ state, revoking, now = Date.now(), onRevoke, onRetry }) => {
  if (state.kind === 'loading')
    return (
      <p className="flex items-center gap-2 text-[11px] text-zinc-500" role="status">
        <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading signed-in devices…
      </p>
    );
  if (state.kind === 'error')
    return (
      <div className="flex flex-wrap items-center gap-2 text-[11px] text-rose-300" role="alert">
        <span>Couldn’t load signed-in devices: {state.message}</span>
        <button type="button" onClick={onRetry} className="px-2 py-1 rounded-md text-zinc-300 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700">
          Try again
        </button>
      </div>
    );
  if (state.sessions.length === 0) return <p className="text-[11px] text-zinc-500">No browsers are signed in with a session yet.</p>;
  return (
    <ul className="divide-y divide-zinc-800 rounded-lg border border-zinc-800">
      {state.sessions.map((s) => (
        <li key={s.id} className="flex items-center gap-3 px-3 py-2">
          <div className="flex-1 min-w-0">
            <p className="flex flex-wrap items-center gap-1.5 text-xs text-zinc-200">
              <span className="truncate">{s.browser || 'Unknown browser'}</span>
              {s.current && (
                <span className="px-1.5 py-0.5 rounded bg-sky-500/15 text-sky-300 text-[10px] font-medium">This device</span>
              )}
            </p>
            <p className="text-[11px] text-zinc-500 break-words">
              {viaLabel(s.via)} · IP {s.ip || 'unknown'} · active {ago(s.last_seen, now)}
            </p>
          </div>
          {!s.current && (
            <button
              type="button"
              onClick={() => onRevoke(s.id)}
              disabled={revoking === s.id}
              aria-label={`Sign out ${s.browser || 'this browser'}`}
              className="shrink-0 flex items-center gap-1 min-h-[32px] px-2.5 rounded-md text-[11px] font-medium text-zinc-300 bg-zinc-800 hover:bg-zinc-700 hover:text-rose-200 border border-zinc-700 transition-colors disabled:opacity-50"
            >
              {revoking === s.id && <Loader2 size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />} Sign out
            </button>
          )}
        </li>
      ))}
    </ul>
  );
};

// ---------------------------------------------------------------- Settings: add a phone or browser

export type AddState =
  | { kind: 'idle' }
  | { kind: 'minting' }
  | { kind: 'showing'; link: MintedLink }
  | { kind: 'expired' }
  | { kind: 'signed_in'; browser: string }
  | { kind: 'error'; message: string };

/** The QR code, drawn here: the sign-in code never goes to a QR service. */
export const QrImage: React.FC<{ text: string }> = ({ text }) => {
  const { size, d } = qrPath(text);
  return (
    <svg
      viewBox={`0 0 ${size} ${size}`}
      role="img"
      aria-label="QR code for the one-time sign-in link"
      shapeRendering="crispEdges"
      className="block w-full h-auto rounded-lg bg-white"
    >
      <path d={d} fill="#000" />
    </svg>
  );
};

/** What "Add a phone or browser" shows in each state. Exported for tests. */
export const AddDeviceView: React.FC<{
  state: AddState;
  now?: number;
  copied?: 'yes' | 'failed' | null;
  onShow: () => void;
  onHide: () => void;
  onCopy: () => void;
}> = ({ state, now = Date.now(), copied = null, onShow, onHide, onCopy }) => {
  if (state.kind === 'idle' || state.kind === 'minting' || state.kind === 'error')
    return (
      <div className="space-y-2">
        <button type="button" onClick={onShow} disabled={state.kind === 'minting'} className={`${secondary} w-full sm:w-auto disabled:opacity-50`}>
          {state.kind === 'minting' ? (
            <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
          ) : (
            <QrCode size={14} aria-hidden="true" />
          )}
          Add a phone or browser
        </button>
        {state.kind === 'error' && (
          <p className="flex items-center gap-1.5 text-[11px] text-rose-300" role="alert">
            <AlertTriangle size={12} aria-hidden="true" /> Couldn’t make a sign-in code: {state.message}
          </p>
        )}
      </div>
    );

  if (state.kind === 'signed_in')
    return (
      <div className="space-y-3 rounded-xl border border-emerald-500/30 bg-emerald-500/5 p-4" role="status" aria-live="polite">
        <p className="flex items-center gap-2 text-xs font-medium text-emerald-200">
          <Check size={14} aria-hidden="true" /> Signed in ✓
        </p>
        <p className="text-[11px] text-zinc-400 leading-relaxed">
          {state.browser || 'A new browser'} signed in with the code. It’s used up now. If that wasn’t you, sign it out below.
        </p>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onShow} className={secondary}>
            Add another
          </button>
          <button type="button" onClick={onHide} className={secondary}>
            Done
          </button>
        </div>
      </div>
    );

  if (state.kind === 'expired')
    return (
      <div className="space-y-3 rounded-xl border border-zinc-700 bg-zinc-900/60 p-4" role="status" aria-live="polite">
        <p className="text-xs text-zinc-300">The code expired, so it was hidden. Make a new one if you still need it.</p>
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={onShow} className={secondary}>
            <RefreshCw size={14} aria-hidden="true" /> New code
          </button>
          <button type="button" onClick={onHide} className={secondary}>
            Hide
          </button>
        </div>
      </div>
    );

  const { link } = state;
  return (
    <div className="space-y-3 rounded-xl border border-sky-500/30 bg-sky-500/5 p-4">
      <p className="text-xs text-zinc-300 leading-relaxed">
        Scan this with the phone’s camera, or open the link in the other browser. It signs that device in once.
      </p>
      <div className="mx-auto w-full max-w-[220px]">
        <QrImage text={link.url} />
      </div>
      <p className="flex items-start gap-1.5 text-[11px] text-amber-200/90 leading-relaxed">
        <AlertTriangle size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
        Anyone who scans this can sign in once. Don’t share or screenshot it.
      </p>
      <div className="flex gap-2">
        <label className="sr-only" htmlFor="add-device-link">
          One-time sign-in link
        </label>
        <input
          id="add-device-link"
          readOnly
          value={link.url}
          onFocus={(e) => e.currentTarget.select()}
          className="flex-1 min-w-0 rounded-lg bg-zinc-950 border border-zinc-700 px-2.5 text-[11px] font-mono text-zinc-300"
        />
        <button type="button" onClick={onCopy} className={`${secondary} shrink-0`}>
          {copied === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
          {copied === 'yes' ? 'Copied' : 'Copy link'}
        </button>
      </div>
      {copied === 'failed' && <p className="text-[11px] text-rose-300">Couldn’t copy. Select the link and copy it yourself.</p>}
      <p className="text-[11px] text-zinc-400" role="timer" aria-live="off">
        Works once · {countdown(link.expiresAt, now)} left
      </p>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={onShow} className={secondary}>
          <RefreshCw size={14} aria-hidden="true" /> New code
        </button>
        <button type="button" onClick={onHide} className={secondary}>
          <X size={14} aria-hidden="true" /> Hide
        </button>
      </div>
    </div>
  );
};

/** "Add a phone or browser": mints a one-time link, shows it as a QR code with a countdown, hides it
 *  when it expires or the page is left, and says "Signed in ✓" once a new browser used it. */
export const AddDevice: React.FC<{ onSignedIn: () => void }> = ({ onSignedIn }) => {
  const [state, setState] = useState<AddState>({ kind: 'idle' });
  const [now, setNow] = useState(() => Date.now());
  const [copied, setCopied] = useState<'yes' | 'failed' | null>(null);
  // Sessions that existed when the code was made; null when the list couldn't be read
  const known = useRef<Set<string> | null>(null);
  const run = useRef(0);

  const show = async () => {
    const mine = ++run.current;
    setState({ kind: 'minting' });
    setCopied(null);
    try {
      known.current = await listSessions()
        .then((list) => new Set(list.map((s) => s.id)))
        .catch(() => null);
      const link = await createSigninLink(window.location.origin);
      if (mine !== run.current) return;
      setNow(Date.now());
      setState({ kind: 'showing', link });
    } catch (e) {
      if (mine !== run.current) return;
      setState({ kind: 'error', message: e instanceof Error && e.message !== 'Failed to fetch' ? e.message : 'Could not reach the server.' });
    }
  };
  const hide = useCallback(() => {
    run.current++;
    known.current = null;
    setCopied(null);
    setState({ kind: 'idle' });
  }, []);

  const showing = state.kind === 'showing' ? state.link : null;

  // The countdown; at zero the code is dropped from the page
  useEffect(() => {
    if (!showing) return;
    const tick = () => {
      const t = Date.now();
      setNow(t);
      if (t >= showing.expiresAt) {
        setCopied(null);
        setState({ kind: 'expired' });
      }
    };
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [showing]);

  // Leaving the page (another tab, the app in the background) hides the code
  useEffect(() => {
    if (!showing) return;
    const onHidden = () => {
      if (document.visibilityState === 'hidden') hide();
    };
    document.addEventListener('visibilitychange', onHidden);
    window.addEventListener('pagehide', hide);
    return () => {
      document.removeEventListener('visibilitychange', onHidden);
      window.removeEventListener('pagehide', hide);
    };
  }, [showing, hide]);

  // Watch the sessions list for the browser that used this code
  useEffect(() => {
    if (!showing || !known.current) return;
    let stop = false;
    const check = async () => {
      const ids = known.current;
      if (!ids) return;
      const list = await listSessions().catch(() => null);
      if (stop || !list) return;
      const fresh = newLinkSession(list, ids);
      if (fresh) {
        run.current++;
        setCopied(null);
        setState({ kind: 'signed_in', browser: fresh.browser });
        onSignedIn();
      }
    };
    const id = window.setInterval(check, 3000);
    return () => {
      stop = true;
      window.clearInterval(id);
    };
  }, [showing, onSignedIn]);

  const copy = async () => {
    if (!showing) return;
    try {
      await navigator.clipboard.writeText(showing.url);
      setCopied('yes');
    } catch {
      setCopied('failed');
    }
  };

  return <AddDeviceView state={state} now={now} copied={copied} onShow={show} onHide={hide} onCopy={copy} />;
};

/** Settings section: who is signed in, with Sign out for each other browser. */
export const SignedInDevices: React.FC = () => {
  const [state, setState] = useState<SessionsState>({ kind: 'loading' });
  const [revoking, setRevoking] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      setState({ kind: 'ready', sessions: await listSessions() });
    } catch (e) {
      setState({ kind: 'error', message: e instanceof Error && e.message !== 'Failed to fetch' ? e.message : 'Could not reach the server.' });
    }
  }, []);
  useEffect(() => {
    load();
  }, [load]);
  return (
    <fieldset className="space-y-3 pt-2 border-t border-zinc-800">
      <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
        <MonitorSmartphone size={14} aria-hidden="true" /> Signed-in devices
      </legend>
      <p className="text-[11px] text-zinc-500 leading-relaxed">
        Browsers that can use kwant bots as you. Sign out any you don’t recognise; signing out here doesn’t change the access token.
      </p>
      <AddDevice onSignedIn={load} />
      <SessionsList
        state={state}
        revoking={revoking}
        onRetry={() => {
          setState({ kind: 'loading' });
          load();
        }}
        onRevoke={async (id) => {
          setRevoking(id);
          try {
            await revokeSession(id);
            await load();
          } catch (e) {
            setState({ kind: 'error', message: e instanceof Error ? e.message : 'Could not reach the server.' });
          } finally {
            setRevoking(null);
          }
        }}
      />
    </fieldset>
  );
};
