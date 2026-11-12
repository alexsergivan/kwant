// Settings › This space › Apps & accounts (0.9.5): app tiles first, then Composio search, the AI
// accounts the bots think with, and a closed Advanced (tool servers, Telegram, notes from bots).
// Connecting an app is one sheet with three steps; the bots picked in step 3 get it, turned on.

import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, Check, ChevronRight, ExternalLink, Loader2, RefreshCw, X } from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { CatalogEntry, ComposioApp, Connection, ConnectionsError, ConnectionsList, composioBody, composioErrorText, connectionsApi } from '../connections';
import {
  APP_TILES,
  AccountRow,
  AppTile,
  AppsSection,
  GiveError,
  accountNeedsYou,
  accountRows,
  appInitials,
  approvalLine,
  botsWithServer,
  giveServerTo,
  listNames,
  serverIdOf,
  tileFor,
  usedByLine,
} from '../appsAccounts';
import { inDesktopApp } from '../externalLinks';
import { useEscapeLayer } from '../escape';
import { useProfiles } from '../useProfiles';
import type { Profile } from '../profiles';
import type { Kwant } from './Sidebar';
import { ComposioAppSearch } from './ConnectionsView';
import { usePhoneLayout } from '../usePhoneLayout';

/** The tile mark: plain initials on the overlay colour, never a brand logo. */
const TILE_BG = '#231d4b';
export const COMPOSIO_HOME = 'https://platform.composio.dev';

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[32px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const primaryBtn = `${btn} bg-sky-600 text-white hover:bg-sky-500`;
const linkBtn =
  'inline-flex items-center gap-1 min-h-[32px] text-xs font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const field =
  'w-full min-h-[40px] sm:min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-500';
const sectionTitle = 'text-xs font-semibold text-zinc-300 uppercase tracking-wider';

const errText = (e: unknown) => (e instanceof Error ? e.message : 'Something went wrong.');

export const AppMark: React.FC<{ name: string; size?: number }> = ({ name, size = 36 }) => (
  <span
    aria-hidden="true"
    className="shrink-0 inline-flex items-center justify-center rounded-lg font-semibold text-violet-100 border border-violet-400/20"
    style={{ background: TILE_BG, width: size, height: size, fontSize: size * 0.36 }}
  >
    {appInitials(name)}
  </span>
);

// ── The Connect sheet ────────────────────────────────────────────────────────────────────────

export interface ConnectApi {
  addComposio: (body: { url: string; api_key?: string; enabled: boolean }) => Promise<{ connection_id: string }>;
  connect: (slug: string) => Promise<{ redirect_url: string }>;
  give: (serverId: string, kwantIds: string[], chosen: string[]) => Promise<void>;
  botsWith: (serverId: string, kwantIds: string[]) => Promise<string[]>;
}

const defaultConnectApi: ConnectApi = {
  addComposio: (b) => connectionsApi.addComposio(b),
  connect: (slug) => connectionsApi.connectComposioApp(slug),
  give: (s, k, c) => giveServerTo(s, k, c),
  botsWith: (s, k) => botsWithServer(s, k),
};

export interface ConnectSheetProps {
  /** The app being connected; null sets up Composio on its own. */
  app: AppTile | null;
  /** Composio's catalogue entry: added or not, and what kwant can do with it. */
  composio: CatalogEntry;
  kwants: Kwant[];
  spaceName: string;
  spaceMode?: string;
  onClose: () => void;
  /** After the bots have it: what to tell the person. */
  onDone: (message: string) => void;
  api?: ConnectApi;
  phone?: boolean;
  /** Tests and screenshots: start with these filled in. */
  initial?: { url?: string; picked?: string[]; opened?: boolean; error?: string };
}

/** One numbered step of the sheet. Outside the sheet, so typing in a step keeps its focus. */
const Step: React.FC<{ n: 1 | 2 | 3; title: string; done: boolean; error: string | null; children: React.ReactNode }> = ({ n, title, done, error, children }) => (
  <li className="flex gap-3" data-sheet-step={n} data-done={done || undefined}>
    <span
      aria-hidden="true"
      className={`shrink-0 mt-0.5 w-6 h-6 rounded-full inline-flex items-center justify-center text-[11px] font-semibold ${
        done ? 'bg-emerald-500/20 text-emerald-200 border border-emerald-500/40' : 'bg-zinc-800 text-zinc-200 border border-zinc-700'
      }`}
    >
      {done ? <Check size={12} /> : n}
    </span>
    <div className="min-w-0 flex-1 space-y-2">
      <p className="text-xs font-medium text-zinc-100">
        <span className="sr-only">Step {n}{done ? ', done' : ''}: </span>
        {title}
      </p>
      {children}
      {error && (
        <p role="alert" className="px-3 py-2 rounded-lg border border-rose-500/30 bg-rose-500/10 text-xs text-rose-200" data-sheet-error={n}>
          {error}
        </p>
      )}
    </div>
  </li>
);

export const ConnectSheet: React.FC<ConnectSheetProps> = ({ app, composio, kwants, spaceName, spaceMode, onClose, onDone, api = defaultConnectApi, phone, initial = {} }) => {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const phoneLayout = usePhoneLayout();
  const isPhone = phone ?? phoneLayout;
  // Rendered into <body> once mounted, so it sits above Settings' own layer and the phone install hint.
  // The focus trap starts there too, on the element that stays.
  const [inBody, setInBody] = useState(false);
  useEffect(() => setInBody(true), []);
  useEscapeLayer(rootRef, inBody, onClose, { modal: true });

  // Composio set up already (or just now, if giving it to bots failed after): skip to the app.
  const [serverId, setServerId] = useState<string | null>(composio.added ? serverIdOf(composio.connection_id) : null);
  const ready = composio.added;
  const [opened, setOpened] = useState(!!initial.opened);
  const [url, setUrl] = useState(initial.url ?? '');
  const [key, setKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [picked, setPicked] = useState<string[]>(initial.picked ?? []);
  const [pickedLoaded, setPickedLoaded] = useState(!ready || !!initial.picked);
  const [error, setError] = useState<{ step: 1 | 2 | 3; text: string } | null>(initial.error ? { step: 2, text: initial.error } : null);
  const [saving, setSaving] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  const [signInOpened, setSignInOpened] = useState(false);

  // Already set up: start from the bots that have Composio now (the sheet changes, not adds).
  useEffect(() => {
    if (!ready || !serverId || initial.picked) return;
    let live = true;
    api.botsWith(serverId, kwants.map((k) => k.id)).then(
      (ids) => live && (setPicked(ids), setPickedLoaded(true)),
      () => live && setPickedLoaded(true),
    );
    return () => {
      live = false;
    };
  }, [ready, serverId]); // eslint-disable-line react-hooks/exhaustive-deps

  const name = app?.name ?? 'Composio';
  const title = app ? `Connect ${app.name}` : 'Set up Composio';
  const chosenBots = kwants.filter((k) => picked.includes(k.id));
  const approval = approvalLine(app?.acts ?? 'Send, post or change things', chosenBots, spaceMode);
  const toggle = (kid: string) => (setPicked((p) => (p.includes(kid) ? p.filter((x) => x !== kid) : [...p, kid])), setError((x) => (x?.step === 3 ? null : x)));

  const signIn = async () => {
    if (!app) return;
    const tab = typeof window !== 'undefined' && !inDesktopApp() ? window.open('', '_blank') : null;
    if (tab) tab.opener = null;
    setSigningIn(true);
    setError(null);
    try {
      const { redirect_url } = await api.connect(app.slug);
      if (tab) tab.location.href = redirect_url;
      else if (inDesktopApp()) window.location.assign(redirect_url);
      else window.open(redirect_url, '_blank', 'noopener,noreferrer');
      setSignInOpened(true);
    } catch (e) {
      tab?.close();
      // 409: Composio refused the saved key. Anything else (502, no answer) is Composio being down or slow.
      const keyRefused = (e as { status?: number } | null)?.status === 409;
      setError({
        step: 1,
        text: keyRefused
          ? `Composio refused your API key, so the ${app.name} sign-in can’t start here. Fix the key under Advanced › Manage tools, or sign in to ${app.name} on composio.dev.`
          : `Couldn’t start the ${app.name} sign-in on Composio: ${errText(e)
              .replace(/[;,.]?\s*try again later\.?$/i, '')
              .replace(/\.$/, '')}. Composio may be down; try again in a minute.`,
      });
    } finally {
      setSigningIn(false);
    }
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    let sid = serverId;
    if (!sid) {
      const body = composioBody(url, key, false);
      if ('error' in body) return setError({ step: 2, text: body.error });
    }
    if (picked.length === 0 && !ready) return setError({ step: 3, text: 'Tick at least one bot. Only the bots you tick can use it.' });
    setSaving(true);
    try {
      if (!sid) {
        const body = composioBody(url, key, false) as { url: string; api_key?: string; enabled: boolean };
        try {
          const r = await api.addComposio(body);
          sid = serverIdOf(r.connection_id);
          setServerId(sid);
        } catch (err) {
          setError({ step: 2, text: composioErrorText(err) });
          return;
        }
        if (!sid) return setError({ step: 2, text: 'Composio was added, but kwant bots didn’t say which server it is. Open Advanced to finish there.' });
      }
      try {
        await api.give(sid, kwants.map((k) => k.id), picked);
      } catch (err) {
        const why = errText(err).replace(/\.$/, '');
        setError({ step: 3, text: ready ? `Giving it to your bots didn’t work: ${why}. Try again.` : `Composio is added, but giving it to your bots didn’t work: ${why}. Try again.` });
        return;
      }
      const who = chosenBots.map((k) => k.name);
      onDone(who.length ? `${name} is on for ${listNames(who)}.` : `No bot has Composio now.`);
    } finally {
      setSaving(false);
    }
  };

  const stepDone = (n: 1 | 2 | 3) => (n === 3 ? false : n === 1 ? (ready ? signInOpened : opened) : ready || !!serverId);
  const stepProps = (n: 1 | 2 | 3) => ({ n, done: stepDone(n), error: error?.step === n ? error.text : null });

  const panel = isPhone
    ? 'absolute inset-x-0 bottom-0 max-h-[92vh] rounded-t-2xl border-t'
    : 'absolute right-0 top-0 h-full w-full max-w-[460px] border-l';

  const sheet = (
    <div className="fixed inset-0 z-[60]" data-connect-sheet={app?.slug ?? 'composio'}>
      <div className="absolute inset-0 bg-black/60" aria-hidden="true" onClick={onClose} />
      <div ref={rootRef} role="dialog" aria-modal="true" aria-labelledby={`${id}-t`} tabIndex={-1} className={`${panel} flex flex-col focus:outline-none bg-zinc-900 border-zinc-800 shadow-2xl`}>
        {isPhone && <span aria-hidden="true" className="mx-auto mt-2 h-1 w-10 rounded-full bg-zinc-700" />}
        <div className="flex items-start gap-3 px-4 sm:px-5 pt-4 pb-3 border-b border-zinc-800">
          {app ? <AppMark name={app.name} /> : <AppMark name="Composio" />}
          <div className="min-w-0 flex-1">
            <h2 id={`${id}-t`} className="text-sm font-semibold text-zinc-100">
              {title}
            </h2>
            <p className="text-[11px] text-zinc-400">{ready ? 'Composio is set up, so this takes a minute' : 'Takes about 2 minutes'}</p>
          </div>
          <button type="button" onClick={onClose} aria-label="Close" className="p-1.5 -mr-1 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
            <X size={17} />
          </button>
        </div>

        <form onSubmit={submit} noValidate className="flex-1 min-h-0 flex flex-col" aria-label={title}>
          <div className="flex-1 min-h-0 overflow-y-auto px-4 sm:px-5 py-4 space-y-4">
            <div className="rounded-xl border border-zinc-800 bg-zinc-950/60 p-3 space-y-2" data-sheet-can>
              <p className="text-xs font-medium text-zinc-200">Your bots will be able to</p>
              <ul className="space-y-1">
                {(app?.can ?? ['Use the apps you connect on Composio']).map((c) => (
                  <li key={c} className="flex items-start gap-2 text-xs text-zinc-300">
                    <Check size={13} className="mt-px shrink-0 text-emerald-400" aria-hidden="true" />
                    {c}
                  </li>
                ))}
                <li className={`flex items-start gap-2 text-xs ${approval.tone === 'ok' ? 'text-zinc-300' : 'text-amber-200'}`} data-sheet-approval={approval.tone}>
                  <span aria-hidden="true" className={`mt-1 shrink-0 w-2 h-2 rounded-full ${approval.tone === 'ok' ? 'bg-sky-400' : 'bg-amber-400'}`} />
                  {approval.text}
                </li>
              </ul>
              <p className="text-[11px] text-zinc-400">
                Through Composio, a service with a free plan. {app ? `Your ${app.name} sign-in stays on Composio’s servers, not in kwant bots.` : 'Your app sign-ins stay on Composio’s servers, not in kwant bots.'}
              </p>
            </div>

            <ol className="space-y-4" aria-label="Steps">
              {ready ? (
                app && (
                  <Step {...stepProps(1)} title={`Sign in to ${app.name}`}>
                    {composio.can_connect ? (
                      <>
                        <p className="text-[11px] text-zinc-400">Opens {app.name}’s own sign-in on Composio {inDesktopApp() ? 'in your browser' : 'in a new tab'}. Come back here when it says you’re connected.</p>
                        <button type="button" className={quietBtn} onClick={signIn} disabled={signingIn} data-sheet-signin>
                          {signingIn ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ExternalLink size={12} aria-hidden="true" />}
                          {signInOpened ? `Open ${app.name} sign-in again` : `Sign in to ${app.name}`}
                        </button>
                        {signInOpened && <p className="text-[11px] text-emerald-200">Opened. Finish there, then choose the bots below.</p>}
                      </>
                    ) : (
                      <p className="text-[11px] text-zinc-400">
                        On{' '}
                        <a href={COMPOSIO_HOME} target="_blank" rel="noopener noreferrer" className="text-sky-300 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 rounded">
                          composio.dev ↗
                        </a>
                        , add {app.name} to the same MCP server you gave kwant bots and sign in there.{' '}
                        {composio.can_search
                          ? 'kwant bots can’t start the sign-in for you because your Composio link has no user_id.'
                          : 'kwant bots can’t start the sign-in for you without your Composio API key (Advanced › Manage tools).'}
                      </p>
                    )}
                  </Step>
                )
              ) : (
                <>
                  <Step {...stepProps(1)} title="Make a free Composio account">
                    <p className="text-[11px] text-zinc-400">
                      Sign up, then {app ? `add ${app.name} and sign in to it there` : 'add the apps you want and sign in to them there'}. Composio keeps that sign-in for your bots.
                    </p>
                    <a href={COMPOSIO_HOME} target="_blank" rel="noopener noreferrer" className={quietBtn} onClick={() => setOpened(true)} data-sheet-open-composio>
                      Open Composio <ExternalLink size={12} aria-hidden="true" />
                      <span className="sr-only">(opens in a new tab)</span>
                    </a>
                  </Step>
                  <Step {...stepProps(2)} title="Paste your Composio link">
                    <label htmlFor={`${id}-url`} className="sr-only">
                      Composio MCP link
                    </label>
                    <input
                      id={`${id}-url`}
                      className={field}
                      type="url"
                      inputMode="url"
                      autoComplete="off"
                      spellCheck={false}
                      placeholder="https://backend.composio.dev/v3/mcp/…"
                      value={url}
                      onChange={(e) => (setUrl(e.target.value), setError((x) => (x?.step === 2 ? null : x)))}
                      aria-describedby={`${id}-url-h`}
                      aria-invalid={error?.step === 2 ? true : undefined}
                      disabled={!!serverId}
                    />
                    <p id={`${id}-url-h`} className="text-[11px] text-zinc-400">
                      In Composio, open <span className="text-zinc-200">MCP</span> in the left menu, pick your server (or create one with {app ? app.name : 'your apps'}), and copy its URL. It starts with https:// and is on composio.dev.
                    </p>
                    {showKey ? (
                      <div className="space-y-1">
                        <label htmlFor={`${id}-key`} className="block text-[11px] font-medium text-zinc-300">
                          Composio API key <span className="font-normal text-zinc-500">(optional)</span>
                        </label>
                        <input id={`${id}-key`} className={field} type="password" autoComplete="off" value={key} onChange={(e) => setKey(e.target.value)} />
                        <p className="text-[11px] text-zinc-500">Lets you find and sign in to apps from kwant bots. Kept on this computer and never shown again.</p>
                      </div>
                    ) : (
                      <button type="button" className={linkBtn} onClick={() => setShowKey(true)}>
                        Add your API key too (optional)
                      </button>
                    )}
                  </Step>
                </>
              )}
              <Step {...stepProps(3)} title="Which bots get it?">
                {!pickedLoaded ? (
                  <p className="flex items-center gap-2 text-[11px] text-zinc-400">
                    <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Checking which bots have it…
                  </p>
                ) : kwants.length === 0 ? (
                  <p className="text-[11px] text-zinc-400">{spaceName} has no bots yet. Make one first, then come back.</p>
                ) : (
                  <fieldset>
                    <legend className="sr-only">Bots in {spaceName}</legend>
                    <div className="flex flex-wrap gap-2">
                      {kwants.map((k) => {
                        const on = picked.includes(k.id);
                        return (
                          <label
                            key={k.id}
                            data-sheet-bot={k.id}
                            className={`inline-flex items-center gap-2 min-h-[40px] sm:min-h-[32px] px-3 rounded-full border text-xs cursor-pointer focus-within:outline focus-within:outline-2 focus-within:outline-sky-500 ${
                              on ? 'border-sky-500/60 bg-sky-500/15 text-zinc-100' : 'border-zinc-700 bg-zinc-800/60 text-zinc-300 hover:border-zinc-600'
                            }`}
                          >
                            <input type="checkbox" className="accent-sky-500" checked={on} onChange={() => toggle(k.id)} />
                            {k.name}
                          </label>
                        );
                      })}
                    </div>
                  </fieldset>
                )}
                <p className="text-[11px] text-zinc-400">
                  {picked.length === 0 ? (ready ? 'No bot has it. Tick the ones that should.' : 'None until you tick one.') : `${listNames(chosenBots.map((k) => k.name))} ${chosenBots.length === 1 ? 'gets' : 'get'} it, turned on.`}{' '}
                  Composio is one connection, so these bots can use every app you’ve connected on Composio.
                </p>
              </Step>
            </ol>
          </div>
          <div className="flex flex-wrap gap-2 px-4 sm:px-5 py-3 border-t border-zinc-800 bg-zinc-900" style={isPhone ? { paddingBottom: 'max(0.75rem, env(safe-area-inset-bottom))' } : undefined}>
            <button type="submit" className={`${primaryBtn} ${isPhone ? 'flex-1' : ''}`} disabled={saving || !pickedLoaded} data-sheet-submit>
              {saving && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
              {ready ? 'Save' : title}
            </button>
            <button type="button" className={quietBtn} onClick={onClose}>
              Cancel
            </button>
          </div>
        </form>
      </div>
    </div>
  );
  return inBody ? createPortal(sheet, document.body) : sheet;
};

// ── The page ─────────────────────────────────────────────────────────────────────────────────

export interface AppsAccountsProps {
  spaceName: string;
  spaceMode?: string;
  kwants: Kwant[];
  /** Where to open: the top, AI accounts, or with Advanced open. */
  section?: AppsSection;
  /** Changes when the same section is asked for again, to scroll there again. */
  sectionSeq?: number;
  /** Bumped to open "Sign in with Claude" (it's inside Manage AI accounts). */
  focusClaudeSignIn?: number;
  /** Opens Advanced (a bot's tool note was asked for). */
  openAdvanced?: boolean;
  /** Manage AI accounts: order, sign-ins and API keys (the old Accounts tab). */
  manageAccounts: React.ReactNode;
  /** Advanced: tool servers, Telegram and notes from bots (the old Connections list); gets Composio's "Choose bots". */
  advanced: (chooseComposioBots: () => void) => React.ReactNode;
  onNewBot?: () => void;
  /** Tests and screenshots. */
  data?: ConnectionsList | null;
  profiles?: Profile[] | null;
  connectApi?: ConnectApi;
  initialSheet?: { slug: string | null; initial?: ConnectSheetProps['initial'] };
  phone?: boolean;
}

// An older server (or an empty reply) may leave out a list; treat it as empty rather than crash.
const tidyList = (d: Partial<ConnectionsList> | null | undefined): ConnectionsList =>
  ({ ...(d && !Array.isArray(d) ? d : {}), connections: Array.isArray(d?.connections) ? d!.connections : [], catalog: Array.isArray(d?.catalog) ? d!.catalog : [] }) as ConnectionsList;

type Load = { state: 'loading' } | { state: 'error'; text: string } | { state: 'token' } | { state: 'ok'; data: ConnectionsList };

export const AppsAccounts: React.FC<AppsAccountsProps> = (p) => {
  const [load, setLoad] = useState<Load>(p.data ? { state: 'ok', data: tidyList(p.data) } : { state: 'loading' });
  const profilesHook = useProfiles(p.profiles === undefined);
  const profiles = p.profiles !== undefined ? p.profiles : profilesHook.profiles;
  const [sheet, setSheet] = useState<{ app: AppTile | null; initial?: ConnectSheetProps['initial'] } | null>(
    p.initialSheet ? { app: p.initialSheet.slug ? tileFor(APP_TILES.find((t) => t.slug === p.initialSheet!.slug) ?? { slug: p.initialSheet.slug, name: p.initialSheet.slug }) : null, initial: p.initialSheet.initial } : null,
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [manageOpen, setManageOpen] = useState(!!p.focusClaudeSignIn);
  const [advancedOpen, setAdvancedOpen] = useState(p.section === 'advanced' || !!p.openAdvanced);
  const [connectedApps, setConnectedApps] = useState<Record<string, boolean>>({});
  const accountsRef = useRef<HTMLElement>(null);
  const manageRef = useRef<HTMLDetailsElement>(null);
  const advancedRef = useRef<HTMLDetailsElement>(null);
  const alive = useRef(true);
  const fixed = p.data !== undefined;

  const reload = useCallback(async () => {
    if (fixed) return;
    try {
      const data = await connectionsApi.list();
      if (alive.current) setLoad({ state: 'ok', data: tidyList(data) });
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ConnectionsError && e.status === 401) setLoad({ state: 'token' });
      else setLoad({ state: 'error', text: errText(e) });
    }
  }, [fixed]);

  useEffect(() => {
    alive.current = true;
    reload();
    window.addEventListener(PERSON_SIGNED_IN_EVENT, reload);
    return () => {
      alive.current = false;
      window.removeEventListener(PERSON_SIGNED_IN_EVENT, reload);
    };
  }, [reload]);

  // Opened at a section: scroll there once it's drawn.
  useEffect(() => {
    if (p.section === 'accounts') accountsRef.current?.scrollIntoView({ block: 'start' });
    if (p.section === 'advanced' || p.openAdvanced) (setAdvancedOpen(true), advancedRef.current?.scrollIntoView({ block: 'start' }));
  }, [p.section, p.sectionSeq, p.openAdvanced, load.state === 'ok']); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!p.focusClaudeSignIn) return;
    setManageOpen(true);
    manageRef.current?.scrollIntoView({ block: 'start' });
  }, [p.focusClaudeSignIn]);

  const data = load.state === 'ok' ? load.data : null;
  const composio = data?.catalog.find((e) => e.id === 'composio');

  // Which tiles are signed in on Composio: only when kwant can ask (the link names a user).
  useEffect(() => {
    if (fixed || !composio?.added || !composio.can_connect) return;
    let live = true;
    APP_TILES.forEach((t) =>
      connectionsApi.composioAppStatus(t.slug).then(
        (r) => live && setConnectedApps((m) => ({ ...m, [t.slug]: r.connected })),
        () => undefined,
      ),
    );
    return () => {
      live = false;
    };
  }, [fixed, composio?.added, composio?.can_connect]);

  const rows = data ? accountRows(data.connections, profiles ?? null) : [];
  const needs = data ? data.connections.filter((c) => accountNeedsYou(c)).length : 0;
  const openManage = () => {
    setManageOpen(true);
    window.setTimeout(() => manageRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }), 0);
  };

  const openApp = (app: AppTile | null) => {
    setNotice(null);
    setSheet({ app });
  };

  const searchApi = {
    search: connectionsApi.searchComposioApps,
    connect: connectionsApi.connectComposioApp,
  };

  return (
    <div className="space-y-6" data-apps-accounts>
      <p className="text-xs text-zinc-400">
        The apps your bots can use and the AI they think with. Apps connect through Composio; nothing secret is shown here.
      </p>

      <div role="status" aria-live="polite">
        {notice && (
          <p className="flex items-start gap-2 px-3 py-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 text-xs text-emerald-100" data-apps-notice>
            <Check size={14} className="mt-px shrink-0" aria-hidden="true" />
            {notice}
          </p>
        )}
      </div>

      {load.state === 'error' && (
        <div role="alert" className="flex flex-wrap items-center gap-2 px-3 py-2.5 rounded-xl border border-rose-500/30 bg-rose-500/10 text-xs text-rose-200" data-apps-error>
          <AlertTriangle size={14} className="shrink-0" aria-hidden="true" />
          <span className="flex-1 min-w-0">Couldn’t load your apps and accounts. {load.text}</span>
          <button type="button" className={quietBtn} onClick={() => (setLoad({ state: 'loading' }), reload())}>
            <RefreshCw size={12} aria-hidden="true" /> Try again
          </button>
        </div>
      )}
      {load.state === 'token' && (
        <div className="flex flex-wrap items-center gap-2 px-3 py-2.5 rounded-xl border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100" data-apps-token>
          <span className="flex-1 min-w-0">Only you can see apps and accounts. Confirm it’s you to see them.</span>
          <button type="button" className={quietBtn} onClick={requestPersonToken}>
            Confirm it’s you
          </button>
        </div>
      )}

      {/* Apps */}
      <section aria-labelledby="apps-title" className="space-y-3" data-apps-tiles>
        <div className="flex items-baseline justify-between gap-2">
          <h3 id="apps-title" className={sectionTitle}>
            Apps
          </h3>
          {composio?.added && <span className="text-[11px] text-zinc-500">Through Composio</span>}
        </div>
        <ul className="grid grid-cols-2 sm:grid-cols-3 gap-2" aria-busy={load.state === 'loading'}>
          {APP_TILES.map((t) => {
            const connected = connectedApps[t.slug];
            return (
              <li key={t.slug}>
                <button
                  type="button"
                  data-app-tile={t.slug}
                  disabled={!composio}
                  onClick={() => openApp(t)}
                  className="w-full h-full min-h-[88px] flex flex-col items-start gap-2 p-3 rounded-xl border border-zinc-800 bg-zinc-900/60 text-left hover:border-zinc-600 hover:bg-zinc-900 transition-colors disabled:opacity-60 disabled:cursor-default focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                >
                  <span className="flex items-center gap-2 w-full min-w-0">
                    <AppMark name={t.name} size={30} />
                    <span className="min-w-0">
                      <span className="block text-xs font-medium text-zinc-100 truncate">{t.name}</span>
                      <span className="block text-[11px] text-zinc-500 truncate" title={t.blurb}>{t.blurb}</span>
                    </span>
                  </span>
                  <span className={`text-[11px] font-medium ${connected ? 'text-emerald-300' : 'text-sky-300'}`}>
                    {load.state === 'loading' ? 'Checking…' : connected ? '✓ Connected · Choose bots' : 'Connect'}
                    <span className="sr-only"> {t.name}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
        {data && !composio && <p className="text-[11px] text-zinc-400">This server can’t connect apps yet. Update kwant bots to connect them here.</p>}
      </section>

      {/* Find any other app */}
      {composio && (
        <section aria-labelledby="apps-find" className="space-y-2" data-apps-search>
          <h3 id="apps-find" className={sectionTitle}>
            Another app
          </h3>
          {composio.added ? (
            <ComposioAppSearch
              entry={composio}
              api={searchApi}
              onPick={(a: ComposioApp) => openApp(tileFor(a))}
            />
          ) : (
            <p className="text-[11px] text-zinc-400">
              Composio has hundreds more, like Figma, Linear or HubSpot.{' '}
              <button type="button" className={linkBtn} onClick={() => openApp(null)} data-apps-setup-composio>
                Set up Composio
              </button>{' '}
              and you can search them here.
            </p>
          )}
        </section>
      )}

      {/* AI accounts */}
      <section ref={accountsRef} aria-labelledby="apps-ai" className="space-y-2 scroll-mt-4" data-ai-accounts>
        <div className="flex items-baseline justify-between gap-2">
          <h3 id="apps-ai" className={sectionTitle}>
            AI accounts
          </h3>
          {needs > 0 && (
            <span className="text-[11px] font-medium text-amber-300" data-ai-needs>
              {needs} {needs === 1 ? 'needs' : 'need'} you
            </span>
          )}
        </div>
        {load.state === 'loading' ? (
          <div className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 space-y-2" aria-hidden="true">
            <div className="h-3 w-40 rounded bg-zinc-800 animate-pulse motion-reduce:animate-none" />
            <div className="h-3 w-56 rounded bg-zinc-800 animate-pulse motion-reduce:animate-none" />
          </div>
        ) : data && rows.length === 0 ? (
          <p className="px-3 py-3 rounded-xl border border-dashed border-zinc-800 text-xs text-zinc-400" data-empty="account">
            No AI accounts yet.{' '}
            <button type="button" className={linkBtn} onClick={openManage}>
              Sign in to one
            </button>
          </p>
        ) : (
          data && (
            <ul className="rounded-xl border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800">
              {rows.map((r) => (
                <AccountItem key={r.id} r={r} onManage={openManage} />
              ))}
            </ul>
          )
        )}
        <details
          ref={manageRef}
          open={manageOpen}
          onToggle={(e) => setManageOpen((e.target as HTMLDetailsElement).open)}
          className="group rounded-xl border border-zinc-800 scroll-mt-4"
          data-manage-accounts
        >
          <summary className="flex items-center gap-2 min-h-[40px] px-3 cursor-pointer list-none text-xs font-medium text-zinc-300 hover:text-zinc-100 rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500">
            <ChevronRight size={14} className="transition-transform group-open:rotate-90 motion-reduce:transition-none" aria-hidden="true" />
            Sign in, add an account or an API key
          </summary>
          {manageOpen && <div className="px-3 pb-3 space-y-3">{p.manageAccounts}</div>}
        </details>
      </section>

      {/* Advanced */}
      <details
        ref={advancedRef}
        open={advancedOpen}
        onToggle={(e) => setAdvancedOpen((e.target as HTMLDetailsElement).open)}
        className="group rounded-xl border border-zinc-800 scroll-mt-4"
        data-apps-advanced
      >
        <summary className="flex items-center gap-2 min-h-[44px] px-3 cursor-pointer list-none rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500">
          <ChevronRight size={14} className="shrink-0 text-zinc-400 transition-transform group-open:rotate-90 motion-reduce:transition-none" aria-hidden="true" />
          <span className="min-w-0">
            <span className="block text-xs font-medium text-zinc-200">Advanced</span>
            <span className="block text-[11px] text-zinc-500">Tool servers (MCP), Telegram, notes from bots, and where logins are kept</span>
          </span>
        </summary>
        {advancedOpen && <div className="px-1 sm:px-3 pb-3" data-settings-connections>{p.advanced(() => void reload().then(() => openApp(null)))}</div>}
      </details>

      {sheet && composio && (
        <ConnectSheet
          app={sheet.app}
          composio={composio}
          kwants={p.kwants}
          spaceName={p.spaceName}
          spaceMode={p.spaceMode}
          api={p.connectApi}
          phone={p.phone}
          initial={sheet.initial}
          onClose={() => setSheet(null)}
          onDone={(msg) => {
            setSheet(null);
            setNotice(msg);
            reload();
          }}
        />
      )}
    </div>
  );
};

const AccountItem: React.FC<{ r: AccountRow; onManage: () => void }> = ({ r, onManage }) => {
  const badge =
    r.state === 'needs_you'
      ? { text: r.statusText ?? 'Needs sign-in', cls: 'border-amber-500/30 bg-amber-500/10 text-amber-200' }
      : r.state === 'ready'
        ? { text: 'Ready', cls: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200' }
        : r.state === 'off'
          ? { text: 'Off', cls: 'border-zinc-700 bg-zinc-800 text-zinc-300' }
          : null;
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-3 py-2.5" data-ai-account={r.profileId} data-ai-state={r.state}>
      <div className="min-w-0 flex-1 basis-48">
        <p className="flex flex-wrap items-center gap-2 text-xs font-medium text-zinc-100">
          {r.title}
          {badge && <span className={`px-1.5 py-px rounded border text-[11px] font-normal ${badge.cls}`}>{badge.text}</span>}
        </p>
        <p className="text-[11px] text-zinc-400 truncate">{r.detail}</p>
        <p className={`text-[11px] ${r.usedBy.length ? 'text-zinc-300' : 'text-zinc-500'}`}>{usedByLine(r.usedBy)}</p>
      </div>
      {r.state === 'needs_you' ? (
        <button type="button" className={`${btn} border border-amber-500/40 bg-amber-500/10 text-amber-100 hover:bg-amber-500/20`} onClick={onManage}>
          Sign in<span className="sr-only"> to {r.title}</span>
        </button>
      ) : r.state === 'sign_in' ? (
        <button type="button" className={linkBtn} onClick={onManage} data-quiet-sign-in>
          Sign in<span className="sr-only"> to {r.title}</span>
        </button>
      ) : (
        <button type="button" className={linkBtn} onClick={onManage}>
          Manage<span className="sr-only"> {r.title}</span>
        </button>
      )}
    </li>
  );
};

export type { Connection };
export { GiveError };
