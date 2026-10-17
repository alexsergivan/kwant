import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, ExternalLink, Loader2, LogIn, LogOut, RotateCcw } from 'lucide-react';
import { ClaudeLoginError, ClaudeLoginSession, claudeLoginApi, isClaudeSignInUrl, isLoginActive, loginFailure } from '../claudeLogin';
import type { EngineInfo } from '../engines';
import { playSfx } from '../audio/chiptune';

const POLL_MS = 2000;

const primaryBtn =
  'inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] font-medium text-white bg-sky-600 hover:bg-sky-500 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';
const quietBtn =
  'inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[11px] font-medium text-zinc-300 border border-zinc-700 hover:text-zinc-100 hover:border-zinc-500 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';
const link = 'inline-flex items-center gap-1 font-medium text-sky-300 underline underline-offset-2 hover:text-sky-200';

/** A blank tab opened while the click still counts as a user gesture, so popup blockers allow it.
 *  `noopener` would make window.open return null, so the opener is cut by hand before navigating. */
function openBlankTab(): Window | null {
  try {
    const tab = window.open('about:blank', '_blank');
    if (tab) tab.opener = null;
    return tab;
  } catch {
    return null;
  }
}

/**
 * The Claude row's "Sign in with Claude": starts Claude Code's sign-in on the server, opens the
 * claude.ai link in a new tab and takes the code claude.ai shows back. `onSignedIn` runs once the
 * server has re-checked Claude, so reloading the engines shows it Ready.
 */
export const ClaudeSignIn: React.FC<{ onSignedIn: () => void; focusRequest?: number }> = ({ onSignedIn, focusRequest }) => {
  const [session, setSession] = useState<ClaudeLoginSession | null>(null);
  const [code, setCode] = useState('');
  const [busy, setBusy] = useState<'start' | 'code' | 'cancel' | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The new tab didn't open: show the link instead. */
  const [blocked, setBlocked] = useState(false);
  const tab = useRef<Window | null>(null);
  const startRef = useRef<HTMLButtonElement>(null);
  const codeRef = useRef<HTMLInputElement>(null);
  const retryRef = useRef<HTMLButtonElement>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => void (alive.current = false);
  }, []);

  const state = session?.state ?? null;
  const active = state !== null && isLoginActive(state);

  /** Sends the blank tab to the link once there is one; once only. */
  const showLink = useCallback((s: ClaudeLoginSession) => {
    const t = tab.current;
    if (!t || !isClaudeSignInUrl(s.url)) return;
    tab.current = null;
    if (t.closed) setBlocked(true);
    else t.location.replace(s.url);
  }, []);

  const start = async () => {
    playSfx('click');
    const t = openBlankTab();
    tab.current = t;
    setBlocked(!t);
    setBusy('start');
    setError(null);
    setCode('');
    try {
      let s: ClaudeLoginSession;
      try {
        s = await claudeLoginApi.start();
      } catch (e) {
        // One is already running (another tab, or before Settings was closed): carry on with it.
        if (e instanceof ClaudeLoginError && e.status === 409 && e.sessionId) s = await claudeLoginApi.get(e.sessionId);
        else throw e;
      }
      if (!alive.current) return;
      setSession(s);
      if (isLoginActive(s.state)) showLink(s);
      else t?.close();
    } catch (e) {
      t?.close();
      tab.current = null;
      if (alive.current) setError(e instanceof Error ? e.message : 'Couldn’t start the sign-in.');
    } finally {
      if (alive.current) setBusy(null);
    }
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const c = code.trim();
    if (!session || !c || busy) return;
    playSfx('click');
    setBusy('code');
    setError(null);
    setSession({ ...session, state: 'verifying' });
    try {
      const s = await claudeLoginApi.submitCode(session.session_id, c);
      if (!alive.current) return;
      setCode('');
      setSession(s);
    } catch (err) {
      if (!alive.current) return;
      if (err instanceof ClaudeLoginError && err.status === 404) {
        setCode('');
        setSession({ ...session, state: 'expired', detail: 'This sign-in has stopped. Start it again.' });
      } else {
        // A malformed code: the sign-in is still waiting for a good one.
        setSession({ ...session, state: 'waiting_for_code' });
        setError(err instanceof Error ? err.message : 'Couldn’t send the code.');
      }
    } finally {
      if (alive.current) setBusy(null);
    }
  };

  const cancel = async () => {
    if (!session) return;
    playSfx('click');
    setBusy('cancel');
    setError(null);
    try {
      await claudeLoginApi.cancel(session.session_id);
    } catch (err) {
      // Already gone is as good as cancelled.
      if (!(err instanceof ClaudeLoginError && err.status === 404)) {
        if (alive.current) setError(err instanceof Error ? err.message : 'Couldn’t cancel the sign-in.');
        if (alive.current) setBusy(null);
        return;
      }
    }
    tab.current?.close();
    tab.current = null;
    if (!alive.current) return;
    setBusy(null);
    setSession(null);
    setCode('');
    setBlocked(false);
    // Focus goes back to where it started.
    requestAnimationFrame(() => startRef.current?.focus());
  };

  // While it's running, look again: it can time out, or finish from another tab.
  useEffect(() => {
    if (!session || !active || busy) return;
    const sid = session.session_id;
    const t = window.setTimeout(async () => {
      try {
        const s = await claudeLoginApi.get(sid);
        if (!alive.current) return;
        setSession((cur) => (cur?.session_id === sid ? s : cur));
        if (s.state === 'waiting_for_code') showLink(s);
      } catch (err) {
        if (alive.current && err instanceof ClaudeLoginError && err.status === 404) {
          setSession((cur) => (cur?.session_id === sid ? { ...cur, state: 'expired', detail: 'This sign-in has stopped. Start it again.' } : cur));
        }
        // Anything else: try again next time.
      }
    }, POLL_MS);
    return () => window.clearTimeout(t);
  }, [session, active, busy, showLink]);

  // Done: the server has re-checked Claude already.
  const doneFor = useRef<string | null>(null);
  useEffect(() => {
    if (session?.state === 'done' && doneFor.current !== session.session_id) {
      doneFor.current = session.session_id;
      playSfx('coin');
      onSignedIn();
    }
  }, [session, onSignedIn]);

  // Focus follows the step: the code box once the link is out, Try again after a failure.
  useEffect(() => {
    if (state === 'waiting_for_code' && busy === null) codeRef.current?.focus();
    if (state === 'failed' || state === 'expired') retryRef.current?.focus();
  }, [state, busy]);

  // "Sign in with Claude" from a warning elsewhere: bring the button into view and focus it.
  useEffect(() => {
    if (!focusRequest) return;
    const b = startRef.current;
    if (!b) return;
    b.scrollIntoView?.({ block: 'center', behavior: 'smooth' });
    b.focus({ preventScroll: true });
  }, [focusRequest]);

  const failure = session ? loginFailure(session) : null;
  const showStart = !session || state === 'cancelled';
  const url = session && isClaudeSignInUrl(session.url) ? session.url : null;

  return (
    <div className="mt-2 space-y-2" data-login-state={state ?? 'idle'}>
      {showStart && (
        <div className="flex items-center gap-2 flex-wrap">
          <button ref={startRef} type="button" onClick={start} disabled={busy === 'start'} className={primaryBtn}>
            {busy === 'start' ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <LogIn size={12} aria-hidden="true" />}
            {busy === 'start' ? 'Starting…' : 'Sign in with Claude'}
          </button>
          <span className="text-[10.5px] text-zinc-500">Opens claude.ai in a new tab.</span>
        </div>
      )}

      {state === 'starting' && (
        <div className="flex items-center gap-2 flex-wrap">
          <p className="flex items-center gap-1.5 text-[11px] text-zinc-300" role="status">
            <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Starting Claude Code’s sign-in…
          </p>
          <button type="button" onClick={cancel} disabled={busy === 'cancel'} className={quietBtn}>
            Cancel
          </button>
        </div>
      )}

      {(state === 'waiting_for_code' || state === 'verifying') && session && (
        <form onSubmit={submit} className="rounded-lg border border-sky-500/30 bg-sky-500/5 p-2.5 space-y-2">
          <p id={`claude-code-help-${session.session_id}`} className="text-[11px] text-zinc-200 leading-snug">
            Finish signing in on claude.ai, then paste the code it shows you here.
          </p>
          {url && (
            <p className="text-[11px] text-zinc-400">
              {blocked ? 'Your browser didn’t open a new tab. ' : 'Closed the tab? '}
              <a href={url} target="_blank" rel="noopener noreferrer" className={link} onClick={() => setBlocked(false)}>
                {blocked ? 'Open claude.ai to sign in' : 'Open claude.ai again'} <ExternalLink size={10} aria-hidden="true" />
              </a>
            </p>
          )}
          <div className="flex items-center gap-1.5 flex-wrap">
            <label htmlFor={`claude-code-${session.session_id}`} className="sr-only">
              Code from claude.ai
            </label>
            <input
              ref={codeRef}
              id={`claude-code-${session.session_id}`}
              aria-describedby={`claude-code-help-${session.session_id}`}
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={code}
              onChange={(e) => setCode(e.target.value)}
              readOnly={state === 'verifying'}
              placeholder="Paste the code"
              className="flex-1 min-w-[10rem] bg-zinc-950 border border-zinc-700 rounded-md px-2 py-1 text-[11px] font-mono text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-sky-500 select-text"
            />
            <button type="submit" disabled={state === 'verifying' || !code.trim()} className={primaryBtn}>
              {state === 'verifying' && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
              {state === 'verifying' ? 'Checking…' : 'Finish sign-in'}
            </button>
            <button type="button" onClick={cancel} disabled={busy === 'cancel' || busy === 'code'} className={quietBtn}>
              Cancel
            </button>
          </div>
          <p className="sr-only" role="status">
            {state === 'verifying' ? 'Checking the code with Claude…' : ''}
          </p>
        </form>
      )}

      {state === 'done' && (
        <p className="flex items-center gap-1.5 text-[11px] text-emerald-300" role="status">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Signed in. Updating…
        </p>
      )}

      {failure && (
        <div className="flex items-center gap-2 flex-wrap">
          <p className="flex items-center gap-1.5 text-[11px] text-rose-300" role="alert">
            <AlertTriangle size={12} className="shrink-0" aria-hidden="true" /> {failure}
          </p>
          <button ref={retryRef} type="button" onClick={start} disabled={busy === 'start'} className={quietBtn}>
            <RotateCcw size={11} aria-hidden="true" /> Try again
          </button>
        </div>
      )}

      {error && (
        <p className="flex items-center gap-1.5 text-[11px] text-rose-300" role="alert">
          <AlertTriangle size={12} className="shrink-0" aria-hidden="true" /> {error}
        </p>
      )}
    </div>
  );
};

/** "Sign out" on a Ready Claude row, behind a confirm dialog. `onSignedOut` gets the new engine list. */
export const ClaudeSignOut: React.FC<{ onSignedOut: (engines: EngineInfo[]) => void }> = ({ onSignedOut }) => {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) cancelRef.current?.focus();
  }, [open]);

  const close = () => {
    if (busy) return;
    setOpen(false);
    setError(null);
    requestAnimationFrame(() => triggerRef.current?.focus());
  };

  const confirm = async () => {
    playSfx('click');
    setBusy(true);
    setError(null);
    try {
      const d = await claudeLoginApi.logout();
      setBusy(false);
      setOpen(false);
      if (Array.isArray(d?.engines)) onSignedOut(d.engines);
    } catch (e) {
      setBusy(false);
      setError(e instanceof Error ? e.message : 'Couldn’t sign out.');
    }
  };

  // Keyboard: Escape closes (without closing Settings too), Tab stays on the two buttons.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      close();
    } else if (e.key === 'Tab') {
      const items = [cancelRef.current, confirmRef.current].filter((x): x is HTMLButtonElement => !!x && !x.disabled);
      if (items.length === 0) return e.preventDefault();
      const i = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : i === items.length - 1 ? 0 : i + 1;
      e.preventDefault();
      items[next].focus();
    }
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => {
          playSfx('click');
          setOpen(true);
        }}
        className="mt-1 inline-flex items-center gap-1 text-[10.5px] text-zinc-500 hover:text-rose-300 underline-offset-2 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 rounded"
      >
        <LogOut size={10} aria-hidden="true" /> Sign out
      </button>
      {open && (
        <div className="fixed inset-0 z-[60] bg-black/60 flex items-center justify-center p-4" onMouseDown={(e) => e.target === e.currentTarget && close()}>
          <div
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="claude-signout-title"
            aria-describedby="claude-signout-desc"
            onKeyDown={onKeyDown}
            className="w-full max-w-sm bg-zinc-900 border border-zinc-700 rounded-xl p-4 shadow-2xl space-y-3 text-zinc-100"
          >
            <h4 id="claude-signout-title" className="text-sm font-semibold">
              Sign out of Claude?
            </h4>
            <div id="claude-signout-desc" className="space-y-1.5 text-xs text-zinc-300 leading-relaxed">
              <p>Every kwant on this server will stop using Claude until you sign in again.</p>
              <p className="text-zinc-400">This signs Claude Code out on this computer, so anything else here that uses the same Claude Code login is signed out too.</p>
            </div>
            {error && (
              <p className="flex items-center gap-1.5 text-[11px] text-rose-300" role="alert">
                <AlertTriangle size={12} className="shrink-0" aria-hidden="true" /> {error}
              </p>
            )}
            <div className="flex justify-end gap-2 pt-1">
              <button ref={cancelRef} type="button" onClick={close} disabled={busy} className={quietBtn}>
                Cancel
              </button>
              <button
                ref={confirmRef}
                type="button"
                onClick={confirm}
                disabled={busy}
                className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] font-medium text-white bg-rose-600 hover:bg-rose-500 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-rose-400"
              >
                {busy && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                {busy ? 'Signing out…' : 'Sign out'}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
};
