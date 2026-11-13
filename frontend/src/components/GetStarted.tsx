// O2 (0.7.6): the "Get started" card in #general. One card, three steps: Choose your AI → Sign in →
// Pick a team. Design: next-076/onboarding/design.md §3–§13 (Mira), C1–C6 accepted by Ada.
// The step logic is in ../gettingStarted.ts and the install in ../gettingStartedInstall.ts; this file
// only draws them. Text comes from code, never from data as HTML.
import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { AlertCircle, Check, Clock, Copy, ExternalLink, HardDrive, Info, KeyRound, RotateCw, Server, WifiOff } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { TemplateGlyph } from './TeamPreviewSheet';
import { NewKeyAccountForm } from './EngineAccounts';
import { useReadyTeams } from './ReadyTeams';
import { ImportDialog, useLocalSources } from './ImportDialog';
import { tickable } from '../importSources';
import type { WelcomeBot } from './FirstRunWelcome';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { inDesktopApp, leaveForSignIn } from '../externalLinks';
import { plural } from '../teams/teamTemplates';
import type { EngineInfo } from '../engines';
import { type SetupItem, setupApi } from '../setupInstall';
import {
  type AiChoice,
  type LoginApi,
  type LoginEngine,
  type LoginSession,
  type Phase,
  type StepsView,
  CHOICES,
  LoginError,
  bannerView,
  choiceName,
  cleanPastedCode,
  cliName,
  codeAriaLabel,
  deviceLabel,
  dismiss,
  heroText,
  installFailView,
  installable,
  isImported,
  isKept,
  keepOwnBotsLine,
  isLive,
  isSignInUrl,
  loginApi as defaultLoginApi,
  markStarted,
  minutesLeft,
  needsInstall,
  noDeviceRoute,
  signInEnd,
  startOrResume,
  step1Value,
  stepsView,
  storeChoice,
  storedChoice,
  tileStatus,
  whereLabel,
} from '../gettingStarted';
import { type StepTeam, type TeamStepView, teamDoneValue, teamStepView } from '../firstScreen';
import { type InstallTrack, forgetInstall, installDetails, startInstall, useInstallTrack, useStartVersion } from '../gettingStartedInstall';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';
const SLOW_S = 40;
const KEY_PROVIDERS = [
  { card: 'anthropic_api', label: 'Anthropic', note: 'Paid per use on your Anthropic account.' },
  { card: 'openai_api', label: 'OpenAI', note: 'Paid per use on your OpenAI account.' },
  { card: 'xai_api', label: 'xAI', note: 'Paid per use on your xAI account.' },
] as const;

/** Where kwant runs, for "on {host}" (§10): the address the page was opened on, unless it's this computer. */
export function hostOf(w: { location?: { hostname?: string } } | undefined = typeof window !== 'undefined' ? window : undefined): string | null {
  if (!w || inDesktopApp(w)) return null;
  const h = w.location?.hostname || '';
  if (!h || h === 'localhost' || h === '127.0.0.1' || h === '::1' || h === '[::1]') return null;
  return h;
}
const isMac = () => typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || navigator.userAgent || '');

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** Counts seconds since `from` while `on`; the number is only shown, never announced. */
function useSeconds(from: number | null, on: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [on, from]);
  return from === null ? 0 : Math.max(0, Math.floor((now - from) / 1000));
}

// ------------------------------------------------------------------ small parts

const Spinner: React.FC = () => (
  <span
    aria-hidden="true"
    className="inline-block w-3.5 h-3.5 shrink-0 rounded-full border-2 border-sky-300/25 border-t-sky-300 animate-spin [animation-duration:.8s] motion-reduce:animate-none"
  />
);

const Btn: React.FC<React.ButtonHTMLAttributes<HTMLButtonElement> & { kind?: 'primary' | 'secondary' | 'link'; phone: boolean }> = ({
  kind = 'secondary',
  phone,
  className = '',
  ...rest
}) => {
  const base =
    kind === 'primary'
      ? `${phone ? 'h-11' : 'h-9'} px-3.5 rounded-lg bg-accent-gradient text-[13px] font-semibold text-white disabled:opacity-45 disabled:cursor-not-allowed`
      : kind === 'secondary'
        ? `${phone ? 'h-11' : 'h-9'} px-3 rounded-lg border border-white/[.14] bg-white/[.04] text-xs text-zinc-200 hover:bg-white/[.08]`
        : `${phone ? 'min-h-11' : 'min-h-8'} px-1 text-xs text-sky-300 hover:text-sky-200`;
  return <button type="button" {...rest} className={`inline-flex items-center justify-center gap-1.5 ${base} ${FOCUS} ${className}`} />;
};

/** One failure row (§9): icon, text, reason, then actions lined up with the text. */
const FailRow: React.FC<{
  tone: 'warn' | 'bad' | 'neutral';
  icon: 'wifi' | 'alert' | 'info' | 'clock';
  title: string;
  reason?: string | null;
  children?: React.ReactNode;
}> = ({ tone, icon, title, reason, children }) => {
  const Icon = icon === 'wifi' ? WifiOff : icon === 'alert' ? AlertCircle : icon === 'clock' ? Clock : Info;
  const color = tone === 'bad' ? 'text-bad' : tone === 'warn' ? 'text-warn' : 'text-zinc-400';
  return (
    <div data-fail-row={tone} role={tone === 'neutral' ? 'status' : 'alert'}>
      <div className="flex items-start gap-2">
        <Icon size={15} className={`mt-0.5 shrink-0 ${color}`} aria-hidden="true" />
        <div className="min-w-0">
          <p className="text-[13px] text-zinc-200">{title}</p>
          {reason ? <p className="text-xs text-zinc-400 mt-0.5 break-words">{reason}</p> : null}
        </div>
      </div>
      {children ? <div className="pl-[23px] mt-2 flex flex-wrap items-center gap-2">{children}</div> : null}
    </div>
  );
};

// ------------------------------------------------------------------ step 1: the tiles

const Tiles: React.FC<{
  engines: EngineInfo[] | null;
  picked: AiChoice | null;
  where: string;
  phone: boolean;
  onPick: (c: AiChoice) => void;
}> = ({ engines, picked, where, phone, onPick }) => {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const current = picked ? CHOICES.findIndex((c) => c.id === picked) : -1;
  const onKey = (e: React.KeyboardEvent, i: number) => {
    const n = CHOICES.length;
    const go = (j: number) => {
      e.preventDefault();
      refs.current[(j + n) % n]?.focus();
    };
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') go(i + 1);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') go(i - 1);
    else if (e.key === 'Home') go(0);
    else if (e.key === 'End') go(n - 1);
  };
  return (
    <div role="radiogroup" aria-label="Choose your AI" className={`grid gap-2 ${phone ? 'grid-cols-2' : 'grid-cols-4'}`} data-ai-tiles>
      {CHOICES.map((c, i) => {
        const st = tileStatus(engines, c.id);
        const on = picked === c.id;
        return (
          <button
            key={c.id}
            ref={(el) => (refs.current[i] = el)}
            type="button"
            role="radio"
            aria-checked={on}
            tabIndex={current === -1 ? (i === 0 ? 0 : -1) : current === i ? 0 : -1}
            data-ai-tile={c.id}
            onClick={() => onPick(c.id)}
            onKeyDown={(e) => onKey(e, i)}
            className={`flex flex-col items-start gap-0.5 rounded-[10px] border px-3 py-2.5 text-left transition-colors motion-reduce:transition-none ${phone ? 'min-h-[76px]' : 'min-h-[72px]'} ${
              on
                ? 'border-sky-400/70 bg-sky-500/10'
                : st === 'signed_in'
                  ? 'border-emerald-400/45 bg-zinc-900/80 hover:bg-zinc-800/80'
                  : 'border-zinc-700 bg-zinc-900/80 hover:border-zinc-600 hover:bg-zinc-800/80'
            } ${FOCUS}`}
          >
            <span className="flex items-center gap-1.5 text-sm font-semibold text-zinc-100">
              {c.id === 'key' ? <KeyRound size={13} className="text-zinc-400" aria-hidden="true" /> : null}
              {c.name}
            </span>
            <span className="text-xs text-zinc-400">{c.line}</span>
            {st === 'signed_in' ? (
              <span className="text-[11px] text-ok inline-flex items-center gap-1">
                <Check size={11} aria-hidden="true" />
                Signed in {where}
              </span>
            ) : st === 'installed' ? (
              <span className="text-[11px] text-zinc-300">Installed · sign in next</span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
};

// ------------------------------------------------------------------ step 2: getting it ready

const InstallPanel: React.FC<{
  track: InstallTrack;
  name: string;
  host: string | null;
  where: string;
  phone: boolean;
  onRetry: () => void;
  onOther: () => void;
}> = ({ track, name, host, where, phone, onRetry, onOther }) => {
  const running = track.state === 'running';
  const s = useSeconds(track.startedAt, running);
  const [copied, setCopied] = useState<string | null>(null);
  const copy = async (what: string, text: string) => {
    if (await copyText(text)) {
      setCopied(what);
      window.setTimeout(() => setCopied(null), 2000);
    }
  };
  // F1: tries again by itself when the browser is back online, and every 15 s.
  const offline = track.state === 'failed' && track.kind === 'offline';
  useEffect(() => {
    if (!offline) return;
    const again = () => onRetry();
    window.addEventListener('online', again);
    const t = window.setInterval(again, 15000);
    return () => {
      window.removeEventListener('online', again);
      window.clearInterval(t);
    };
  }, [offline, onRetry]);

  if (running) {
    const slow = s >= SLOW_S;
    const on = host ? ` on ${host}` : '';
    return (
      <div data-install-line>
        <div className="flex items-center gap-2">
          <Spinner />
          <p className="text-[13px] text-zinc-200">
            <span role="status" aria-live="polite">
              {slow ? 'Still getting' : 'Getting'} {name} ready{on}
            </span>
            <span aria-hidden="true">
              … <span className="tabular-nums text-zinc-500">{s} s</span>
            </span>
          </p>
        </div>
        <p className="text-xs text-zinc-400 mt-1 pl-[22px]">
          {slow ? 'A slow connection can take a minute or two. You can pick a team meanwhile.' : `Next you sign in with your ${name} account.`}
        </p>
      </div>
    );
  }
  if (track.state === 'needs_admin') {
    return (
      <FailRow tone="warn" icon="info" title={`One step needs an admin ${host ? `on ${host}` : where}.`} reason="Run this once as an admin. kwant notices when it’s done.">
        <div className="w-full">
          {track.manual.map((m) => (
            <div key={m.command} className="flex items-center gap-2 mb-2">
              <code className="flex-1 min-w-0 overflow-x-auto whitespace-pre rounded-lg border border-zinc-700 bg-zinc-950/70 px-3 py-2 font-mono text-xs text-zinc-100" aria-label={m.label || 'Command'}>
                {m.command}
              </code>
              <Btn phone={phone} onClick={() => copy(m.command, m.command)} aria-label="Copy the command">
                {copied === m.command ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
                {copied === m.command ? 'Copied' : 'Copy'}
              </Btn>
            </div>
          ))}
          <p className="text-xs text-zinc-400 inline-flex items-center gap-2" role="status">
            <Spinner />
            Checking every few seconds…
          </p>
        </div>
      </FailRow>
    );
  }
  if (track.state === 'failed') {
    const kind = track.kind ?? 'other';
    const v = installFailView(kind, name, where, host);
    if (kind === 'offline')
      return (
        <FailRow tone="warn" icon="wifi" title={v.title} reason={v.reason}>
          <Btn phone={phone} onClick={onRetry}>
            Try now
          </Btn>
        </FailRow>
      );
    const big = kind === 'disk' || kind === 'permission';
    return (
      <FailRow tone="bad" icon="alert" title={v.title} reason={v.reason}>
        <Btn phone={phone} kind="primary" onClick={onRetry}>
          <RotateCw size={13} aria-hidden="true" />
          Try again
        </Btn>
        <Btn phone={phone} onClick={() => copy('details', installDetails(track))}>
          {copied === 'details' ? 'Copied' : 'Copy details'}
        </Btn>
        {!big ? (
          <Btn phone={phone} kind="link" onClick={onOther}>
            Choose another AI
          </Btn>
        ) : null}
      </FailRow>
    );
  }
  return null;
};

/** The O1 install route can't do this one here (e.g. Codex without its standalone build, §11). */
const CantInstall: React.FC<{ engine: EngineInfo | undefined; name: string; choice: LoginEngine }> = ({ engine, name, choice }) => (
  <p className="text-[13px] text-zinc-300" data-cant-install>
    {choice === 'codex' ? 'Needs Node.js first. ' : ''}
    {engine?.detail || `Install ${name} on this computer.`} This step moves on by itself once it’s there.
  </p>
);

// ------------------------------------------------------------------ step 2: sign in

/** Claude: open claude.ai, then paste the code (§5.3). Always the paste flow (D5 unverified). */
const ClaudePaste: React.FC<{ api: LoginApi; phone: boolean; onDone: () => void; onOther: () => void }> = ({ api, phone, onDone, onOther }) => {
  const [s, setS] = useState<LoginSession | null>(null);
  const [opened, setOpened] = useState(false);
  const [code, setCode] = useState('');
  const [bad, setBad] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);
  const tab = useRef<Window | null>(null);
  const fieldId = useId();
  const sid = s?.session_id;
  const live = !!s && isLive(s.state);

  useEffect(() => {
    if (!sid || !live) return;
    const t = window.setInterval(async () => {
      try {
        const next = await api.get('claude', sid);
        setS(next);
        if (next.state === 'done') onDone();
      } catch (e) {
        if (e instanceof LoginError && e.status === 404) setS((p) => (p ? { ...p, state: 'expired' } : p));
      }
    }, 2000);
    return () => window.clearInterval(t);
  }, [api, sid, live, onDone]);

  const go = (url: string) => {
    const t = tab.current;
    tab.current = null;
    if (inDesktopApp()) leaveForSignIn(url);
    else if (t && !t.closed) t.location.replace(url);
    else window.open(url, '_blank', 'noopener,noreferrer');
  };

  const open = async () => {
    setError(null);
    if (s && isLive(s.state) && isSignInUrl('claude', s.url)) {
      go(s.url);
      setOpened(true);
      return;
    }
    if (!inDesktopApp()) {
      try {
        const t = window.open('about:blank', '_blank');
        if (t) t.opener = null;
        tab.current = t;
      } catch {
        tab.current = null;
      }
    }
    try {
      let next = await startOrResume(api, 'claude');
      for (let i = 0; i < 20 && next.state === 'starting' && !next.url; i++) {
        await new Promise((r) => setTimeout(r, 500));
        next = await api.get('claude', next.session_id);
      }
      setS(next);
      if (isSignInUrl('claude', next.url)) {
        go(next.url);
        setOpened(true);
      } else {
        tab.current?.close();
        setError(next.detail || 'Couldn’t get a sign-in link from Claude.');
      }
    } catch (e) {
      tab.current?.close();
      setError(e instanceof Error ? e.message : 'Sign-in didn’t start.');
    }
  };

  const submit = async (e?: React.FormEvent) => {
    e?.preventDefault();
    const c = cleanPastedCode(code);
    if (!c || !sid) return;
    setBad(false);
    try {
      const next = await api.submitCode('claude', sid, c);
      setS(next);
      if (next.state === 'done') onDone();
      else if (next.state === 'failed') {
        setBad(true);
        field.current?.focus();
      }
    } catch {
      setBad(true);
      field.current?.focus();
    }
  };

  const paste = async () => {
    try {
      const t = await navigator.clipboard.readText();
      if (t) setCode(cleanPastedCode(t));
    } catch {
      /* the browser said no; the field still takes a long-press paste */
    }
  };

  const end = s && !isLive(s.state) && s.state !== 'done' && !(s.state === 'failed' && bad) ? signInEnd(s, 'claude') : null;
  if (end)
    return (
      <FailRow tone={end.tone} icon={end.kind === 'expired' ? 'clock' : end.tone === 'bad' ? 'alert' : 'info'} title={end.title} reason={end.reason}>
        <Btn
          phone={phone}
          kind="primary"
          onClick={() => {
            setS(null);
            setOpened(false);
            setCode('');
          }}
        >
          {end.action}
        </Btn>
        <Btn phone={phone} kind="link" onClick={onOther}>
          Choose another AI
        </Btn>
      </FailRow>
    );

  const dot = (n: number, done: boolean) => (
    <span aria-hidden="true" className={`mt-0.5 w-[18px] h-[18px] shrink-0 rounded-full text-[11px] font-semibold flex items-center justify-center ${done ? 'bg-zinc-800 text-ok' : 'border border-zinc-600 text-zinc-300'}`}>
      {done ? <Check size={11} /> : n}
    </span>
  );
  return (
    <ol className="flex flex-col gap-3" data-claude-paste>
      <li className="flex items-start gap-2.5">
        {dot(1, opened)}
        <div className="min-w-0 flex-1">
          <p className="text-[13px] text-zinc-200">Open claude.ai and allow kwant.</p>
          <div className="mt-2">
            <Btn phone={phone} kind={opened ? 'secondary' : 'primary'} onClick={open} data-open-claude>
              {opened ? 'Open again' : 'Open claude.ai'}
              <ExternalLink size={12} aria-hidden="true" />
            </Btn>
          </div>
          {error ? (
            <p className="text-xs text-bad mt-1.5" role="alert">
              {error}
            </p>
          ) : null}
        </div>
      </li>
      <li className="flex items-start gap-2.5">
        {dot(2, false)}
        <form className="min-w-0 flex-1" onSubmit={submit}>
          <label htmlFor={fieldId} className="text-[13px] text-zinc-200">
            Paste the code claude.ai shows you.
          </label>
          <div className="mt-2 flex items-center gap-2">
            <input
              ref={field}
              id={fieldId}
              value={code}
              onChange={(e) => {
                setCode(e.target.value);
                setBad(false);
              }}
              onPaste={(e) => {
                const t = e.clipboardData.getData('text');
                if (t) {
                  e.preventDefault();
                  setCode(cleanPastedCode(t));
                  setBad(false);
                }
              }}
              placeholder="Paste the code"
              autoComplete="off"
              spellCheck={false}
              aria-invalid={bad || undefined}
              aria-describedby={bad ? `${fieldId}-err` : undefined}
              className={`flex-1 min-w-0 rounded-lg border bg-zinc-950/70 px-3 font-mono text-zinc-100 placeholder:text-zinc-500 ${phone ? 'h-11 text-base' : 'h-9 text-[13px]'} ${
                bad ? 'border-rose-400/70' : 'border-zinc-700'
              } ${FOCUS}`}
            />
            {phone && typeof navigator !== 'undefined' && typeof navigator.clipboard?.readText === 'function' ? (
              <Btn phone={phone} onClick={paste}>
                Paste
              </Btn>
            ) : null}
            <Btn phone={phone} kind="primary" type="submit" disabled={!code.trim() || !sid || s?.state === 'verifying'}>
              Sign in
            </Btn>
          </div>
          {bad ? (
            <p id={`${fieldId}-err`} className="text-xs text-bad mt-1.5" role="alert">
              That code didn’t work. Copy the whole code from claude.ai and paste it again.
            </p>
          ) : null}
        </form>
      </li>
    </ol>
  );
};

/** ChatGPT and Grok: the device code (§5.4). Falls back to the Terminal line when O3 isn't on this server. */
const DevicePanel: React.FC<{
  api: LoginApi;
  engine: 'codex' | 'grok';
  phone: boolean;
  onDone: () => void;
  onOther: () => void;
  onNoRoute: () => void;
}> = ({ api, engine, phone, onDone, onOther, onNoRoute }) => {
  const [s, setS] = useState<LoginSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [round, setRound] = useState(0);
  const sid = s?.session_id;
  const live = !!s && isLive(s.state);
  const name = cliName(engine);
  const noRouteRef = useRef(onNoRoute);
  noRouteRef.current = onNoRoute;
  const doneRef = useRef(onDone);
  doneRef.current = onDone;

  // One sign-in per round (Start again / Get a new code start the next); never one per render.
  useEffect(() => {
    let off = false;
    setError(null);
    startOrResume(api, engine)
      .then((x) => !off && setS(x))
      .catch((e) => {
        if (off) return;
        if (noDeviceRoute(e)) noRouteRef.current();
        else setError(e instanceof Error ? e.message : 'Sign-in didn’t start.');
      });
    return () => {
      off = true;
    };
  }, [api, engine, round]);

  useEffect(() => {
    if (!sid || !live) return;
    const t = window.setInterval(async () => {
      try {
        const next = await api.get(engine, sid);
        setS(next);
        if (next.state === 'done') doneRef.current();
      } catch (e) {
        if (e instanceof LoginError && e.status === 404) setS((p) => (p ? { ...p, state: 'expired' } : p));
      }
    }, 1500);
    return () => window.clearInterval(t);
  }, [api, engine, sid, live]);

  const cancel = async () => {
    if (!sid) return;
    try {
      await api.cancel(engine, sid);
    } catch {
      /* it ends by itself */
    }
    setS((p) => (p ? { ...p, state: 'cancelled' } : p));
  };

  const copyCode = async () => {
    if (!s?.user_code) return false;
    const ok = await copyText(s.user_code);
    if (ok) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    }
    return ok;
  };

  // C6: copies the code first, then opens the page.
  const open = async () => {
    if (!s || !isSignInUrl(engine, s.url)) return;
    const url = s.url;
    await copyCode();
    if (inDesktopApp()) leaveForSignIn(url);
    else window.open(url, '_blank', 'noopener,noreferrer');
  };

  if (error)
    return (
      <FailRow tone="bad" icon="alert" title="Sign-in didn’t start." reason={error}>
        <Btn phone={phone} kind="primary" onClick={() => setRound((r) => r + 1)}>
          Try again
        </Btn>
        <Btn phone={phone} kind="link" onClick={onOther}>
          Choose another AI
        </Btn>
      </FailRow>
    );

  const end = s ? signInEnd(s, engine) : null;
  if (end)
    return (
      <FailRow tone={end.tone} icon={end.kind === 'expired' ? 'clock' : end.tone === 'bad' ? 'alert' : 'info'} title={end.title} reason={end.kind === 'failed' && s?.detail ? `${name} said: “${s.detail}”` : end.reason}>
        <Btn
          phone={phone}
          kind="primary"
          onClick={() => {
            setS(null);
            setRound((r) => r + 1);
          }}
        >
          {end.kind === 'failed' ? 'Sign in with another account' : end.action}
        </Btn>
        <Btn phone={phone} kind="link" onClick={onOther}>
          Choose another AI
        </Btn>
      </FailRow>
    );

  const label = deviceLabel(s?.url);
  if (!s || !s.user_code || !label)
    return (
      <p className="text-[13px] text-zinc-300 inline-flex items-center gap-2" role="status">
        <Spinner />
        Getting a sign-in code from {name}…
      </p>
    );
  const mins = minutesLeft(s.expires_at, Date.now());
  const site = label.split('/')[0];
  return (
    <div data-device-panel>
      <p className="text-[13px] text-zinc-200">
        Open <span className="font-semibold">{label}</span> and enter this code:
      </p>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <span
          aria-label={codeAriaLabel(s.user_code)}
          data-user-code
          className={`select-all rounded-[10px] border border-dashed border-zinc-600 bg-zinc-950/70 px-4 py-2 font-mono font-semibold tracking-[.12em] text-zinc-50 ${phone ? 'text-[22px]' : 'text-2xl'}`}
        >
          {s.user_code}
        </span>
        <Btn phone={phone} onClick={copyCode}>
          {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
          {copied ? 'Copied' : 'Copy'}
        </Btn>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1">
        <Btn phone={phone} kind="primary" onClick={open} data-open-device>
          Open {label}
          <ExternalLink size={12} aria-hidden="true" />
        </Btn>
        {!phone ? <span className="text-xs text-zinc-400">Copies the code too.</span> : null}
      </div>
      <div className="mt-3 flex items-start gap-2">
        <span className="mt-0.5">
          <Spinner />
        </span>
        <div className="min-w-0">
          <p className="text-[13px] text-zinc-200" role="status" aria-live="polite">
            Waiting for you to finish on {site}…
          </p>
          <p className="text-xs text-zinc-400">This moves on by itself.{mins !== null ? ` The code works for ${mins} ${mins === 1 ? 'minute' : 'minutes'}.` : ''}</p>
        </div>
      </div>
      <div className="mt-1">
        <Btn phone={phone} kind="link" onClick={cancel}>
          Cancel
        </Btn>
      </div>
    </div>
  );
};

/** Before O3 (§11): "Run `codex login` in Terminal", with Copy, and the engine list is read again. */
const TerminalSignIn: React.FC<{ engine: 'codex' | 'grok'; phone: boolean; recheck: () => void }> = ({ engine, phone, recheck }) => {
  const cmd = `${engine} login`;
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    const t = window.setInterval(recheck, 5000);
    return () => window.clearInterval(t);
  }, [recheck]);
  return (
    <div data-terminal-signin>
      <p className="text-[13px] text-zinc-200">Run this in Terminal and sign in with your {engine === 'codex' ? 'ChatGPT' : 'Grok'} account:</p>
      <div className="mt-2 flex items-center gap-2">
        <code className="rounded-lg border border-zinc-700 bg-zinc-950/70 px-3 py-2 font-mono text-[13px] text-zinc-100">{cmd}</code>
        <Btn
          phone={phone}
          onClick={async () => {
            if (await copyText(cmd)) {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 2000);
            }
          }}
        >
          {copied ? 'Copied' : 'Copy'}
        </Btn>
      </div>
      <p className="mt-2 text-xs text-zinc-400 inline-flex items-center gap-2" role="status">
        <Spinner />
        This moves on by itself when you’re signed in.
      </p>
    </div>
  );
};

const KeyPanel: React.FC<{ phone: boolean; onAdded: () => void }> = ({ phone, onAdded }) => {
  const [card, setCard] = useState<(typeof KEY_PROVIDERS)[number]['card']>('anthropic_api');
  const p = KEY_PROVIDERS.find((x) => x.card === card)!;
  return (
    <div data-key-panel>
      <div role="radiogroup" aria-label="Key from" className="flex flex-wrap gap-1.5">
        {KEY_PROVIDERS.map((k) => (
          <button
            key={k.card}
            type="button"
            role="radio"
            aria-checked={card === k.card}
            onClick={() => setCard(k.card)}
            className={`${phone ? 'h-11' : 'h-[30px]'} px-3 rounded-full border text-xs ${card === k.card ? 'border-sky-400/70 bg-sky-500/10 text-zinc-100' : 'border-zinc-700 text-zinc-300'} ${FOCUS}`}
          >
            {k.label}
          </button>
        ))}
      </div>
      <div className="mt-2">
        <NewKeyAccountForm key={card} card={card} onCancel={() => undefined} onCreated={onAdded} />
      </div>
      <p className="mt-1 text-xs text-zinc-400">{p.note} The key stays on this computer.</p>
    </div>
  );
};

// ------------------------------------------------------------------ step 3: pick a team

const TeamStep: React.FC<{
  phone: boolean;
  spaceName: string;
  /** False outside the main space: Keep names this space's own bots, not the main space's starters. */
  mainSpace: boolean;
  /** The space's bots, for that line. */
  botNames: string[];
  onKeep: () => void;
  onPicked: () => void;
  /** After the import dialog closes with at least one import done: step 3 is done. */
  onImported: () => void;
  /** Each Import or Undo inside the dialog, so the sidebar loads the new bots. */
  onImportChanged?: () => void;
  /** First-screen D: what step 3 shows; a team here gives "Your team" and the Start tile. */
  view: TeamStepView;
}> = ({ phone, spaceName, mainSpace, botNames, onKeep, onPicked, onImported, onImportChanged, view }) => {
  const rt = useReadyTeams();
  const [importing, setImporting] = useState(false);
  const imported = useRef(false);
  const importTile = useRef<HTMLButtonElement>(null);
  // Desktop app only: one look at the usual places, for the "N found" line and the dialog.
  const localSources = useLocalSources();
  const [local] = localSources;
  const found = local.kind === 'ready' ? tickable(local.sources).length : 0;
  const closeImport = () => {
    setImporting(false);
    // An import that went in (even if undone in the dialog) counts; Cancel or Close leaves step 3 as it was.
    if (imported.current) onImported();
    else window.setTimeout(() => importTile.current?.focus(), 0);
  };
  const offered = new Set(view.templateIds);
  const templates = rt?.available ? (rt.templates ?? []).filter((t) => offered.has(t.id)) : [];
  const start = view.start;
  const tile = `w-full rounded-xl border text-left ${phone ? 'min-h-16 p-3' : 'p-3'} ${FOCUS}`;
  return (
    <div data-team-step>
      <p className="text-xs text-zinc-400 mb-2.5">{view.intro}</p>
      {start && (
        <div
          data-start-team={start.handle}
          className={`mb-3 rounded-xl border border-sky-400/60 bg-sky-400/[0.08] p-3 flex gap-3 ${phone ? 'flex-col' : 'items-center'}`}
        >
          <span className="flex -space-x-2 shrink-0" aria-hidden="true">
            {start.members.slice(0, 3).map((b) => (
              <span key={b.id} className="rounded-full ring-2 ring-[#16122b]">
                <ModernAvatar role={b.avatar_sprite} name={b.name} size={26} />
              </span>
            ))}
          </span>
          <span className="flex-1 min-w-0">
            <span className="block text-sm font-semibold text-zinc-50">Start with @{start.handle}</span>
            {start.line && <span className="block text-[12.5px] leading-[18px] text-zinc-300">{start.line}</span>}
          </span>
          <Btn kind="primary" phone={phone} data-start-button onClick={onKeep} className={`shrink-0 text-sm px-4${phone ? ' w-full' : ''}`}>
            Start
          </Btn>
        </div>
      )}
      {start && (
        <p className="text-2xs font-semibold uppercase tracking-wider text-zinc-400 mb-2" data-add-team-label>
          Or add a team for one job
        </p>
      )}
      <ul className={phone ? 'flex flex-col gap-2' : 'grid grid-cols-2 gap-2'}>
        {templates.map((t) => (
          <li key={t.id}>
            <button
              type="button"
              data-template={t.id}
              onClick={() => rt?.open({ kind: 'template', templateId: t.id }, { onDone: onPicked })}
              aria-label={`${t.label} team: ${t.card_line} ${plural(t.bots.length, 'bot')}. Preview`}
              className={`${tile} flex flex-col gap-1.5 border-zinc-700 bg-zinc-900/80 hover:border-sky-300/45`}
            >
              <span className="flex items-center gap-2.5">
                <TemplateGlyph icon={t.icon} box="w-8 h-8 rounded-[9px] border border-sky-300/35 bg-sky-500/10 text-sky-300" />
                <span className="text-sm font-semibold text-zinc-50">{t.label}</span>
              </span>
              <span className="text-[12.5px] leading-[18px] text-zinc-300">{t.card_line}</span>
              <span className="flex items-center gap-2" aria-hidden="true">
                <span className="flex -space-x-[5px]">
                  {t.bots.map((b) => (
                    <span key={b.key} className="rounded-full ring-2 ring-zinc-900">
                      <ModernAvatar role={b.avatar_sprite} name={b.name} size={18} />
                    </span>
                  ))}
                </span>
                <span className="text-xs text-zinc-400">{plural(t.bots.length, 'bot')}</span>
                <span className="ml-auto text-xs text-sky-300">Preview ›</span>
              </span>
            </button>
          </li>
        ))}
        {!start && <li>
          <button type="button" data-keep-starter onClick={onKeep} className={`${tile} h-full flex flex-col gap-1 border-dashed border-zinc-600 bg-transparent hover:border-zinc-500`}>
            <span className="text-sm font-semibold text-zinc-100">{mainSpace ? 'Keep the starter bots' : 'Keep the bots you have'}</span>
            <span className="text-[12.5px] leading-[18px] text-zinc-300">{mainSpace ? 'Architect, TechLead and DevOps, in #general.' : keepOwnBotsLine(botNames, spaceName)}</span>
            <span className="mt-auto text-right text-xs text-sky-300" aria-hidden="true">
              Use ›
            </span>
          </button>
        </li>}
        <li>
          <button
            ref={importTile}
            type="button"
            data-import-bots
            aria-haspopup="dialog"
            aria-label={`Bring your bots: from Grok Bot, ChatGPT or Claude, a file, a folder or a GitHub link. You check a preview first.${found > 0 ? ` ${found} found on this computer.` : ''} Import`}
            onClick={() => {
              imported.current = false;
              setImporting(true);
            }}
            className={`${tile} h-full flex flex-col gap-1 border-dashed border-zinc-600 bg-transparent hover:border-zinc-500`}
          >
            <span className="text-sm font-semibold text-zinc-100">Bring your bots</span>
            <span className="text-[12.5px] leading-[18px] text-zinc-300">From Grok Bot, ChatGPT or Claude, a file, a folder or a GitHub link. You check a preview first.</span>
            {found > 0 && (
              <span className="flex items-center gap-1.5 text-xs text-sky-200" data-found-line>
                <HardDrive size={13} className="text-sky-300 shrink-0" aria-hidden="true" />
                <span className="font-semibold">{found} found on this computer</span>
              </span>
            )}
            <span className="mt-auto text-right text-xs text-sky-300" aria-hidden="true">
              Import ›
            </span>
          </button>
        </li>
      </ul>
      {importing && (
        <ImportDialog
          spaceName={spaceName}
          localSources={localSources}
          onClose={closeImport}
          onChanged={() => {
            imported.current = true;
            onImportChanged?.();
          }}
        />
      )}
    </div>
  );
};

// ------------------------------------------------------------------ the card

export interface GetStartedProps {
  space: string;
  bots: WelcomeBot[];
  engines: EngineInfo[] | null;
  /** Reads /api/engines again (after an install or a sign-in). */
  reloadEngines: () => void;
  /** The card goes for good ("Keep the starter bots" after step 2). */
  onDismissed: () => void;
  loginApiOverride?: LoginApi;
  /** For tests and shots: the address it's opened on. */
  hostOverride?: string | null;
  /** The space's name, for the import dialog's title. */
  spaceName?: string;
  /** After an Import or Undo from "Bring your bots": new bots, teams and team chats. */
  onImported?: () => void;
  /** The space's `is_default`. Only the main space has the starter bots; left out counts as main. */
  mainSpace?: boolean;
  /** The space's teams: with one, step 3 is "Your team" with a Start tile (first-screen D). */
  teams?: StepTeam[];
}

const NO_TEAMS: StepTeam[] = [];

export const GetStarted: React.FC<GetStartedProps> = ({ space, bots, engines, reloadEngines, onDismissed, loginApiOverride, hostOverride, spaceName, onImported, mainSpace = true, teams = NO_TEAMS }) => {
  const api = loginApiOverride ?? defaultLoginApi;
  const phone = useMedia(PHONE_QUERY);
  useStartVersion();
  const track = useInstallTrack(space);
  const host = hostOverride !== undefined ? hostOverride : hostOf();
  const mac = !host && isMac();
  const where = whereLabel({ host, mac });
  const chosen = storedChoice(space);
  const [changing, setChanging] = useState(false);
  const [teamEarly, setTeamEarly] = useState(false);
  const [noRoute, setNoRoute] = useState<Record<string, boolean>>({});
  const [items, setItems] = useState<SetupItem[] | null>(null);
  const [resumed] = useState(() => !!chosen && !(engines && tileStatus(engines, chosen) === 'signed_in'));
  const v: StepsView = stepsView({ engines, chosen, teamEarly, changing });
  const kept = isKept(space);
  const rt = useReadyTeams();
  const teamView = teamStepView({ teams, bots, templates: rt?.templates ?? [], madeFrom: (id) => rt?.made(id)?.templateId });
  const importedHere = isImported(space);
  const titleId = useId();
  const stepIds = [useId(), useId(), useId()];
  const cardRef = useRef<HTMLElement>(null);
  const hadFocus = useRef(false);

  useEffect(() => markStarted(space), [space]);
  useEffect(() => {
    let off = false;
    setupApi
      .status()
      .then((x) => !off && setItems(x))
      .catch(() => !off && setItems([]));
    return () => {
      off = true;
    };
  }, [track?.state]);

  const choice = v.choice;
  const loginEngine: LoginEngine | null = choice && choice !== 'key' ? choice : null;
  const missing = !!loginEngine && needsInstall(engines, loginEngine);
  const canAuto = loginEngine ? installable(items, loginEngine) : null;

  const install = useCallback(
    (c: LoginEngine) => {
      void startInstall(space, c, { onDone: reloadEngines });
    },
    [space, reloadEngines],
  );

  // F7: a running job picks up where it left off; a lost one starts again (409 is followed).
  useEffect(() => {
    if (v.open !== 2 && !(teamEarly && !v.done2)) return;
    if (!loginEngine || !missing || canAuto !== true) return;
    if (track && track.item === loginEngine) return;
    install(loginEngine);
  }, [v.open, teamEarly, v.done2, loginEngine, missing, canAuto, track, install]);

  // Once the CLI is installed, read the engines again so step 2 moves on to sign-in.
  useEffect(() => {
    if (track?.state === 'done') reloadEngines();
  }, [track?.state, reloadEngines]);

  const pick = (c: AiChoice) => {
    if (track && track.item !== c) forgetInstall(space);
    storeChoice(space, c);
    setChanging(false);
    setTeamEarly(false);
  };
  const other = () => {
    forgetInstall(space);
    setChanging(true);
  };
  const signedIn = useCallback(() => {
    reloadEngines();
  }, [reloadEngines]);
  const keep = () => {
    if (v.done2) {
      dismiss(space);
      onDismissed();
    } else markStarted(space, 'kept');
  };
  // Start with @team: today's Keep, then the composer, so the next thing is the first ask.
  const startTeam = () => {
    keep();
    window.setTimeout(() => document.querySelector<HTMLTextAreaElement>('[data-composer-form] textarea')?.focus(), 0);
  };
  // "Bring your bots": like Keep, the card goes once step 2 is done; before that step 3 reads done.
  const importedBots = () => {
    if (v.done2) {
      dismiss(space);
      onDismissed();
    } else markStarted(space, 'imported');
  };

  // §13: focus doesn't jump unless it was inside the step that closed; then it goes to the new heading.
  const lastOpen = useRef(v.open);
  useEffect(() => {
    if (lastOpen.current === v.open) return;
    lastOpen.current = v.open;
    const a = typeof document !== 'undefined' ? document.activeElement : null;
    if (hadFocus.current && (!a || a === document.body)) document.getElementById(stepIds[v.open - 1])?.focus();
  });

  const hero = heroText(v, { host, mac });
  const name = choice ? choiceName(choice) : '';
  const stepTitle2 = choice === 'key' ? 'Add your key' : 'Sign in';
  const slow = !!track && track.state === 'running' && Date.now() - track.startedAt >= SLOW_S * 1000;
  const done3 = kept || importedHere;
  const row = phone ? 'px-3.5 py-3' : 'px-[18px] py-3.5';
  const body = phone ? 'pt-2.5' : 'pl-[33px] pt-2.5';
  const engineOf = (id: string) => engines?.find((e) => e.id === id);

  const step2Body = (() => {
    if (!choice) return null;
    if (choice === 'key') return <KeyPanel phone={phone} onAdded={signedIn} />;
    const le = choice;
    if (track && track.item === le && track.state !== 'done')
      return <InstallPanel track={track} name={cliName(le)} host={host} where={where} phone={phone} onRetry={() => install(le)} onOther={other} />;
    if (missing) {
      if (canAuto === false) return <CantInstall engine={engineOf(le)} name={cliName(le)} choice={le} />;
      return (
        <p className="text-[13px] text-zinc-300 inline-flex items-center gap-2" role="status">
          <Spinner />
          Getting {cliName(le)} ready…
        </p>
      );
    }
    if (le === 'claude') return <ClaudePaste api={api} phone={phone} onDone={signedIn} onOther={other} />;
    if (noRoute[le]) return <TerminalSignIn engine={le} phone={phone} recheck={reloadEngines} />;
    return <DevicePanel key={le} api={api} engine={le} phone={phone} onDone={signedIn} onOther={other} onNoRoute={() => setNoRoute((m) => ({ ...m, [le]: true }))} />;
  })();

  const head = (n: 1 | 2 | 3, title: string, state: 'open' | 'todo' | 'done', value?: string, extra?: React.ReactNode) => {
    const said = state === 'open' ? (n === 2 && track?.state === 'running' ? 'in progress' : 'open') : state === 'done' ? 'done' : 'to do';
    return (
      <div className="flex items-center gap-[11px] min-h-6">
        <span
          aria-hidden="true"
          className={`w-[22px] h-[22px] shrink-0 rounded-full text-xs font-semibold flex items-center justify-center ${
            state === 'open' ? 'bg-sky-300 text-zinc-950' : state === 'done' ? 'bg-zinc-800 border border-zinc-700 text-ok' : 'border border-zinc-600 text-zinc-300'
          }`}
        >
          {state === 'done' ? <Check size={12} /> : n}
        </span>
        <h3 id={stepIds[n - 1]} tabIndex={-1} className={`min-w-0 text-sm ${state === 'open' ? 'font-semibold text-zinc-100' : state === 'done' ? 'font-medium text-zinc-200' : 'font-medium text-zinc-400'} focus:outline-none`}>
          <span className="sr-only">{`Step ${n} of 3, `}</span>
          {title}
          <span className="sr-only">{`, ${said}`}</span>
          {value ? <span className="ml-2 text-xs font-normal text-zinc-400">{value}</span> : null}
        </h3>
        {extra}
      </div>
    );
  };

  return (
    <section
      ref={cardRef}
      aria-labelledby={titleId}
      data-get-started
      data-open-step={v.open}
      onFocus={() => (hadFocus.current = true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null) && e.relatedTarget) hadFocus.current = false;
      }}
      className={`w-full max-w-[640px] mx-auto ${phone ? 'px-3.5 pt-6' : 'pt-6 sm:pt-16'} pb-6`}
    >
      <div className="flex flex-col items-center text-center">
        {bots.length ? (
          <div className="flex -space-x-2" aria-hidden="true">
            {bots.slice(0, 5).map((b) => (
              <span key={b.id} className="rounded-full ring-4 ring-zinc-950">
                <ModernAvatar role={b.avatar_sprite} name={b.name} size={44} />
              </span>
            ))}
          </div>
        ) : null}
        <h2 id={titleId} className="mt-4 text-[17px] leading-6 font-semibold text-zinc-100">
          {hero.title}
        </h2>
        <p className="mt-1 text-sm text-zinc-400">{hero.sub}</p>
        {host ? (
          <span className="mt-3 inline-flex items-center gap-1.5 h-7 px-3 rounded-full border border-white/[.14] bg-white/[.04] text-xs text-zinc-200" data-host-chip>
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" aria-hidden="true" />
            <Server size={12} className="text-zinc-400" aria-hidden="true" />
            Setting up on {host}
          </span>
        ) : null}
      </div>

      {resumed && !v.done2 ? (
        <p className="mt-4 text-xs text-zinc-400 inline-flex items-center gap-1.5" data-resumed>
          <RotateCw size={12} aria-hidden="true" />
          Picking up where you left off.
        </p>
      ) : null}

      <div className={`${resumed && !v.done2 ? 'mt-2' : 'mt-5'} rounded-xl border border-zinc-700/60 bg-zinc-900/60 shadow-raised text-left`}>
        <div className={row} data-step="1">
          {head(
            1,
            'Choose your AI',
            v.open === 1 ? 'open' : v.done1 ? 'done' : 'todo',
            v.open !== 1 && v.done1 ? step1Value(v, engines, where) : undefined,
            v.open !== 1 && v.done1 ? (
              <Btn phone={phone} kind="link" className="ml-auto shrink-0" onClick={() => setChanging(true)} aria-label="Change your AI">
                Change
              </Btn>
            ) : undefined,
          )}
          {v.open === 1 ? (
            <div className={`${body} gs-open`}>
              <p className="text-xs text-zinc-400 mb-2.5">{host ? `Tap one. It gets ready on ${host} by itself.` : 'Tap one. It gets ready by itself: no Terminal, no password for this computer.'}</p>
              <Tiles engines={engines} picked={changing ? chosen : null} where={where} phone={phone} onPick={pick} />
            </div>
          ) : null}
        </div>
        <div className={`${row} border-t border-white/[.07]`} data-step="2">
          {head(
            2,
            stepTitle2,
            v.open === 2 ? 'open' : v.done2 ? 'done' : teamEarly && v.done1 ? 'open' : 'todo',
            v.done2 && v.open !== 1 ? (v.foundReady ? 'Nothing to do' : `${name}, signed in`) : undefined,
            teamEarly && !v.done2 ? (
              <Btn phone={phone} kind="link" className="ml-auto shrink-0" onClick={() => setTeamEarly(false)}>
                Open
              </Btn>
            ) : undefined,
          )}
          {v.open === 2 ? <div className={`${body} gs-open`}>{step2Body}</div> : null}
        </div>
        <div className={`${row} border-t border-white/[.07]`} data-step="3">
          {head(
            3,
            teamView.title,
            v.open === 3 && !done3 ? 'open' : done3 ? 'done' : 'todo',
            done3 ? teamDoneValue({ imported: importedHere, mainSpace, startHandle: teamView.start?.handle ?? null }) : undefined,
            v.open === 2 && slow && !done3 ? (
              <Btn phone={phone} kind="link" className="ml-auto shrink-0" onClick={() => setTeamEarly(true)}>
                Open
              </Btn>
            ) : undefined,
          )}
          {v.open === 3 && !done3 ? (
            <div className={`${body} gs-open`}>
              <TeamStep
                phone={phone}
                spaceName={spaceName ?? 'this space'}
                mainSpace={mainSpace}
                botNames={bots.map((b) => b.name)}
                onKeep={teamView.start ? startTeam : keep}
                view={teamView}
                onPicked={() => undefined}
                onImported={importedBots}
                onImportChanged={onImported}
              />
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
};

// ------------------------------------------------------------------ the banner outside #general (§7.1)

/** What the banner says now; null once step 2 is done (team choice isn't nagged). */
export function bannerPhase(engines: EngineInfo[] | null, choice: AiChoice | null, track: InstallTrack | null): Phase | null {
  const v = stepsView({ engines, chosen: choice });
  if (v.done2) return null;
  if (!v.choice) return { kind: 'choose' };
  if (track && track.item === v.choice) {
    if (track.state === 'running') return { kind: 'installing', seconds: 0 };
    if (track.state === 'failed' || track.state === 'needs_admin') return { kind: 'failed' };
  }
  return v.choice !== 'key' && needsInstall(engines, v.choice) ? { kind: 'installing', seconds: 0 } : { kind: 'signin' };
}

export const GetStartedBanner: React.FC<{ space: string; engines: EngineInfo[] | null; onOpen: () => void }> = ({ space, engines, onOpen }) => {
  const phone = useMedia(PHONE_QUERY);
  useStartVersion();
  const track = useInstallTrack(space);
  const choice = storedChoice(space);
  const phase = useMemo(() => bannerPhase(engines, choice, track), [engines, choice, track]);
  const running = phase?.kind === 'installing' && !!track && track.state === 'running';
  const seconds = useSeconds(track?.startedAt ?? null, running);
  if (!phase) return null;
  const view = bannerView(phase.kind === 'installing' ? { kind: 'installing', seconds } : phase, stepsView({ engines, chosen: choice }).choice);
  if (!view) return null;
  // The seconds are shown, never announced: the live text stops before them.
  const [live, tail] = phase.kind === 'installing' ? [view.title.replace(/… \d+ s$/, ''), `… ${seconds} s`] : [view.title, ''];
  const [liveShort, tailShort] = phase.kind === 'installing' ? [view.short.replace(/… \d+ s$/, ''), `… ${seconds} s`] : [view.short, ''];
  return (
    <div role="status" data-setup-banner data-get-started-banner className={`flex items-center gap-3 border-b border-sky-500/20 bg-sky-500/[.06] ${phone ? 'px-3 py-2' : 'px-6 py-2.5'}`}>
      {view.spinner ? <Spinner /> : null}
      <div className="min-w-0 flex-1">
        <p className="text-xs font-medium text-zinc-100 truncate">
          {phone ? liveShort : live}
          {tail ? (
            <span aria-hidden="true" className="tabular-nums">
              {phone ? tailShort : tail}
            </span>
          ) : null}
        </p>
        {!phone && view.sub ? <p className="text-[11.5px] text-zinc-400 truncate">{view.sub}</p> : null}
      </div>
      <Btn phone={phone} onClick={onOpen} className={phone ? '' : '!h-8'}>
        {view.button}
      </Btn>
    </div>
  );
};
