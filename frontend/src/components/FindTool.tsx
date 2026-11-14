// Find a tool / Another app (composio-simple/design.md §1, §2, "Autocomplete"; one-app-search/design.md): one field
// that finds an app wherever it comes from, from the first letter. Typed, it shows ONE list with one row per app
// (appSearch.mergeRows): each row says where it comes from and how you sign in, its button starts the right flow,
// and the other ways to add the same app open under it. An empty field keeps Connected and Popular.
// Used in Add a tool (browseWhenEmpty) and on Apps & accounts. ARIA 1.2 combobox: focus stays in the field, ↑/↓ move
// one highlight across every option (other ways included when open), Enter runs the highlighted option's button.
//
// Text from Composio and the directory is shown as plain text, never as HTML.

import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  Eye,
  EyeOff,
  KeyRound,
  Loader2,
  Lock,
  LogIn,
  type LucideIcon,
  Minus,
  Monitor,
  Plus,
  RefreshCw,
  Search,
  ShieldCheck,
  Users,
  X,
} from 'lucide-react';
import {
  CatalogEntry,
  ComposioAppList,
  ComposioListedApp,
  Connection,
  ConnectionsError,
  ConnectionsList,
  DirectoryEntry,
  connectionsApi,
} from '../connections';
import { APP_TILES, appInitials, listNames } from '../appsAccounts';
import { COMPOSIO_ONLY_APPS, MCP_PRESETS, McpPreset } from '../mcp';
import { ToolMatch, ghostRest, matchTool, normalize, suggestionsLine } from '../toolSearch';
import {
  MergedRow,
  OTHER_WAYS_SHOWN,
  SignInIcon,
  Source,
  addedServer,
  appKey,
  mergeRows,
  moreOtherWays,
  otherWaysLabel,
  presetPublisher,
  signInOf,
  sourceWords,
} from '../appSearch';
import { usePhoneLayout } from '../usePhoneLayout';

export { presetPublisher };

// ── Data ─────────────────────────────────────────────────────────────────────────────────────

export interface FindToolApi {
  list: () => Promise<ConnectionsList>;
  apps: () => Promise<ComposioAppList>;
  addKey: (key: string) => Promise<unknown>;
  replaceKey: (key: string) => Promise<unknown>;
  connect: (slug: string) => Promise<{ redirect_url: string }>;
  /** null: this server has no public directory yet, so its rows never show. */
  directory: (q: string, signal?: AbortSignal) => Promise<{ entries: DirectoryEntry[]; unavailable: boolean } | null>;
}

export const defaultFindToolApi: FindToolApi = {
  list: () => connectionsApi.list(),
  apps: () => connectionsApi.composioAppList(),
  addKey: (k) => connectionsApi.addComposioKey(k),
  replaceKey: (k) => connectionsApi.replaceComposioKey(k),
  connect: (slug) => connectionsApi.connectComposioApp(slug),
  directory: (q, signal) => connectionsApi.searchDirectory(q, signal),
};

/** What "Choose bots" is for: Composio (one connection, every app on it) or one tool server. */
export type ChooseBotsTarget = { kind: 'composio'; slug: string; name: string } | { kind: 'server'; connectionId: string; name: string };

export interface FindToolProps {
  /** Add a tool: an empty field shows the browse grid (`children`) under "All built-in tools". */
  browseWhenEmpty: boolean;
  /** The page's connections, when it has loaded them; without it Find a tool loads them itself. */
  data?: ConnectionsList | null;
  /** Ask the page to load its connections again (after Composio is added or an app connects). */
  onChanged?: () => void | Promise<unknown>;
  onAddPreset: (preset: McpPreset) => void;
  onAddDirectory?: (entry: DirectoryEntry) => void;
  onAddByAddress?: () => void;
  onChooseBots: (target: ChooseBotsTarget) => void;
  /** Composio is added but can't start a sign-in here (an old link setup): the page's own way. */
  onComposioElsewhere?: (slug: string, name: string) => void;
  /** Esc on an empty field (Add a tool closes). */
  onEscapeEmpty?: () => void;
  autoFocus?: boolean;
  phone?: boolean;
  api?: FindToolApi;
  /** Tests and shots: start with this text. */
  initialQuery?: string;
  /** The field's name for screen readers (Apps & accounts: "Find any app"). */
  label?: string;
  placeholder?: string;
  /** The browse grid. */
  children?: React.ReactNode;
}

type GroupKey = 'connected' | 'popular';

const GROUP_LABEL: Record<GroupKey, string> = {
  connected: 'Connected',
  popular: 'Popular',
};

interface ComposioRowApp {
  slug: string;
  name: string;
  description: string;
  tools: number | null;
  connected: boolean;
}

/** A row of the empty field (Connected, Popular). */
type Row =
  | { key: string; group: GroupKey; kind: 'builtin'; name: string; match: ToolMatch | null; preset: McpPreset; added: Connection | null }
  | { key: string; group: GroupKey; kind: 'composio'; name: string; match: ToolMatch | null; app: ComposioRowApp };

interface Group {
  key: GroupKey;
  rows: Row[];
  /** "{n} more. Type to find them." in Connected; not an option. */
  rest?: number;
}

const POPULAR = ['gmail', 'googlecalendar', 'slack', 'notion', 'github', 'googledrive'];
/** Typed: 8 rows, then "Show {n} more apps" (design §2). */
const ROWS_SHOWN = 8;

/** What an app does, for the Composio rows we know; Composio's lists carry no description. */
const KNOWN_BLURBS: Record<string, string> = {};
for (const a of COMPOSIO_ONLY_APPS) if (a.blurb) KNOWN_BLURBS[a.toolkit] = a.blurb;
for (const t of APP_TILES) KNOWN_BLURBS[t.slug] = `${t.blurb}.`;
const TINT: Record<string, string> = {};
for (const t of APP_TILES) if (t.tint) TINT[t.slug] = t.tint;

const FALLBACK_APPS: ComposioListedApp[] = COMPOSIO_ONLY_APPS.map((a) => ({ slug: a.toolkit, name: a.label }));

/** The directory clips a maker's description at 100 characters; when it stops mid-sentence, show that it was cut. */
export function markCutDescription(description: string): string {
  const t = description.trimEnd();
  return description.length >= 100 && !/[.!?)…]$/.test(t) ? `${t}…` : description;
}

/** The directory's key chips: "API key", "2 keys" or "Sign-in". */
export function directoryNeeds(e: DirectoryEntry): string | null {
  const keys = (e.inputs ?? []).filter((i) => i.secret || /key|token|secret/i.test(i.name)).length;
  if (keys === 1) return 'API key';
  if (keys > 1) return `${keys} keys`;
  return null;
}

/** The empty field's groups: Connected (up to 4) and Popular. A typed field uses `mergeRows` instead. */
export function buildGroups(opts: { composioApps: ComposioRowApp[]; connections: Connection[] }): Group[] {
  const { composioApps, connections } = opts;
  const builtinRow = (group: GroupKey, preset: McpPreset): Row => ({
    key: `${group}-b-${preset.id}`,
    group,
    kind: 'builtin',
    name: preset.label,
    match: null,
    preset,
    added: addedServer(preset, connections),
  });
  const composioRow = (group: GroupKey, app: ComposioRowApp): Row => ({ key: `${group}-c-${app.slug}`, group, kind: 'composio', name: app.name, match: null, app });
  const connected: Row[] = [];
  MCP_PRESETS.forEach((p) => addedServer(p, connections) && connected.push(builtinRow('connected', p)));
  composioApps.filter((a) => a.connected).forEach((a) => connected.push(composioRow('connected', a)));
  const taken = new Set(connected.map((r) => (r.kind === 'builtin' ? r.preset.id : r.app.slug)));
  const popular: Row[] = [];
  POPULAR.forEach((slug) => {
    if (taken.has(slug)) return;
    const preset = MCP_PRESETS.find((p) => p.id === slug);
    if (preset) return void popular.push(builtinRow('popular', preset));
    const app = composioApps.find((a) => a.slug === slug);
    if (app) popular.push(composioRow('popular', app));
  });
  const groups: Group[] = [];
  if (connected.length) groups.push({ key: 'connected', rows: connected.slice(0, 4), rest: Math.max(0, connected.length - 4) || undefined });
  if (popular.length) groups.push({ key: 'popular', rows: popular });
  return groups;
}

// ── Look ─────────────────────────────────────────────────────────────────────────────────────

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[32px] px-3 py-1 rounded-lg text-xs font-medium whitespace-nowrap transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 motion-reduce:transition-none';
const quietBtn = `${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const primaryBtn = `${btn} border border-sky-600 bg-sky-600 text-white hover:bg-sky-500`;
const linkBtn =
  'inline-flex items-center gap-1 min-h-[44px] sm:min-h-[28px] text-xs font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';
const chip = 'inline-flex items-center px-1.5 py-px rounded-md border border-zinc-700 text-[11px] text-zinc-300';
const amberChip = 'inline-flex items-center px-1.5 py-px rounded-md border border-amber-500/40 text-[11px] text-amber-300';
const keycap = 'inline-flex items-center justify-center h-[18px] min-w-[18px] px-1 rounded border border-zinc-700 border-b-2 bg-zinc-900 text-[10.5px] font-medium text-zinc-400';
const fade = 'motion-safe:animate-[findtool-in_160ms_ease-out]';
const activeBg = 'bg-[#272050]';
const activeBar = 'shadow-[inset_2px_0_0_#9d8fff]';

const Mark: React.FC<{ name: string; slug?: string }> = ({ name, slug }) => (
  <span
    aria-hidden="true"
    className={`shrink-0 inline-flex items-center justify-center w-8 h-8 rounded-lg border border-violet-400/20 text-[11px] font-semibold ${(slug && TINT[slug]) || 'text-violet-100'}`}
    style={{ background: '#231d4b' }}
  >
    {appInitials(name)}
  </span>
);

/** The name with the typed letters bold, in one span so the row's gap can't split it. */
const Name: React.FC<{ name: string; bold: [number, number] | null }> = ({ name, bold }) => (
  <span className="text-sm font-medium text-zinc-100" data-row-name>
    {bold ? (
      <>
        {name.slice(0, bold[0])}
        <b className="font-bold text-zinc-50">{name.slice(bold[0], bold[1])}</b>
        {name.slice(bold[1])}
      </>
    ) : (
      name
    )}
  </span>
);

const Spinner: React.FC = () => (
  <>
    <Loader2 size={14} className="shrink-0 animate-spin motion-reduce:hidden" style={{ animationDuration: '700ms' }} aria-hidden="true" />
    <span className="hidden motion-reduce:inline-block w-1.5 h-1.5 rounded-full bg-zinc-400" aria-hidden="true" />
  </>
);

/** Where a way to add an app comes from (design §1): plain chips, never holographic. */
const SourceChip: React.FC<{ s: Source }> = ({ s }) => {
  const base = 'shrink-0 inline-flex items-center gap-1 h-5 px-1.5 rounded-md border text-[11px] font-medium whitespace-nowrap';
  if (s.origin === 'official')
    return (
      <span className={`${base} border-sky-400/45 bg-sky-400/10 text-sky-200`} data-source-chip="official">
        <ShieldCheck size={12} aria-hidden="true" /> Official
        {s.by && <span className="font-normal text-sky-300">· {s.by}</span>}
      </span>
    );
  if (s.origin === 'community')
    return (
      <span className={`${base} border-zinc-700 text-zinc-300 max-w-full`} data-source-chip="community">
        <Users size={12} aria-hidden="true" className="shrink-0" /> Community
        {s.by && <span className="font-normal text-zinc-400 truncate">· {s.by}</span>}
      </span>
    );
  return (
    <span className={`${base} border-zinc-700 text-zinc-200`} data-source-chip={s.origin}>
      {s.origin === 'composio' ? 'Through Composio' : 'Built in'}
    </span>
  );
};

const SIGN_IN_ICON: Record<SignInIcon, LucideIcon> = {
  account: LogIn,
  key: KeyRound,
  local: Monitor,
  none: Minus,
};

const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

const openKeyPage = (url: string) => window.open(url, '_blank', 'noopener,noreferrer');

// ── The key field (§2, and Replace key for a refused saved key) ──────────────────────────────

// ── The key field (§2, and Replace key for a refused saved key) ──────────────────────────────

export type KeyPhase = { s: 'idle' } | { s: 'checking' } | { s: 'refused'; text: string } | { s: 'down'; text: string };

export interface KeyFieldProps {
  id: string;
  /** Overrides for other places (the Composio tile): label, button, helper. */
  label?: string;
  action?: string;
  helper?: string;
  /** No Cancel button when the field is the only way forward (a refused saved key). */
  noCancel?: boolean;
  /** null: no app, "Add Composio". */
  app: { slug: string; name: string } | null;
  replace: boolean;
  keyUrl: string;
  phone: boolean;
  phase: KeyPhase;
  inputRef: React.RefObject<HTMLInputElement>;
  onSubmit: (key: string) => void;
  onCancel: () => void;
}

export const KeyField: React.FC<KeyFieldProps> = ({ id, app, replace, keyUrl, phone, phase, inputRef, onSubmit, onCancel, ...o }) => {
  const [value, setValue] = useState('');
  const [shown, setShown] = useState(false);
  const checking = phase.s === 'checking';
  const err = phase.s === 'refused' || phase.s === 'down' ? phase : null;
  const label = o.label ?? (replace ? 'New Composio API key' : 'Your Composio API key');
  const action = o.action ?? (replace ? 'Save key' : app ? `Connect ${app.name}` : 'Add Composio');
  return (
    <form
      className={`space-y-2 ${fade}`}
      data-composio-key-field={phase.s}
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim() && !checking) onSubmit(value);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={id} className="text-xs font-medium text-zinc-200">
          {label}
        </label>
        <button type="button" className={linkBtn} onClick={() => openKeyPage(keyUrl)} data-key-link>
          {replace ? 'Get a key' : 'Get a free key'} <ExternalLink size={12} aria-hidden="true" />
          <span className="sr-only"> (opens a new tab)</span>
        </button>
      </div>
      <div className={phone ? 'space-y-2' : 'flex items-stretch gap-2'}>
        <div className="relative flex-1 min-w-0">
          <input
            ref={inputRef}
            id={id}
            type={shown ? 'text' : 'password'}
            value={value}
            readOnly={checking}
            onChange={(e) => setValue(e.target.value)}
            placeholder="Paste your key"
            autoComplete="off"
            spellCheck={false}
            aria-invalid={phase.s === 'refused' || undefined}
            aria-describedby={`${id}-help${err ? ` ${id}-err` : ''}`}
            className={`w-full h-10 pl-3 pr-11 rounded-[10px] bg-zinc-950 border text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-400 focus:ring-[3px] focus:ring-sky-400/25 ${
              phone ? 'text-base h-11' : 'text-sm'
            } ${phase.s === 'refused' ? 'border-rose-400' : checking ? 'border-sky-400' : 'border-zinc-700'}`}
          />
          <button
            type="button"
            onClick={() => setShown((v) => !v)}
            aria-label={shown ? 'Hide key' : 'Show key'}
            aria-pressed={shown}
            className="absolute right-0 top-0 h-full w-11 inline-flex items-center justify-center text-zinc-400 hover:text-zinc-200 rounded-r-[10px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
          >
            {shown ? <EyeOff size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}
          </button>
        </div>
        <button type="submit" className={`${primaryBtn} sm:min-h-[40px] ${phone ? 'w-full' : ''}`} disabled={!value.trim() || checking} data-key-submit>
          {checking ? (
            <>
              <Spinner /> Checking your key…
            </>
          ) : (
            action
          )}
        </button>
        {replace && !checking && !o.noCancel && (
          <button type="button" className={`${quietBtn} sm:min-h-[40px] ${phone ? 'w-full' : ''}`} onClick={onCancel}>
            Cancel
          </button>
        )}
      </div>
      {err && (
        <div id={`${id}-err`} role="alert" className="flex items-start gap-2 px-3 py-2 rounded-lg border border-rose-500/40 bg-rose-500/10 text-xs text-rose-200" data-key-error={err.s}>
          <AlertTriangle size={14} className="mt-px shrink-0" aria-hidden="true" />
          <span className="flex-1 min-w-0">
            {err.text}{' '}
            {err.s === 'refused' ? (
              <button type="button" className={linkBtn} onClick={() => openKeyPage(keyUrl)}>
                Open your API keys <ExternalLink size={12} aria-hidden="true" />
                <span className="sr-only"> (opens a new tab)</span>
              </button>
            ) : (
              <button type="button" className={linkBtn} onClick={() => value.trim() && onSubmit(value)}>
                <RefreshCw size={12} aria-hidden="true" /> Try again
              </button>
            )}
          </span>
        </div>
      )}
      <p id={`${id}-help`} className="text-[11px] text-zinc-400">
        {o.helper ?? (replace ? 'A key from another Composio project starts fresh there, so you’d connect your apps again.' : 'Free plan: 10,000 tool calls a month. Kept on this computer and never shown again.')}
      </p>
    </form>
  );
};

// ── Find a tool ──────────────────────────────────────────────────────────────────────────────

type AppsLoad = { s: 'loading' } | { s: 'ok'; list: ComposioAppList } | { s: 'down'; text: string } | { s: 'refused'; text: string } | { s: 'none' };
type DirLoad = { q: string; entries: DirectoryEntry[]; unavailable: boolean; loading: boolean };
type AppState = { s: 'waiting'; url: string | null; since: number } | { s: 'connected' };
/** Where the key field is open: under a Composio row (setup before Composio is added) or Replace key. */
type KeyOpen = { slug: string | null; name: string | null; replace: boolean } | null;

/** One option of the listbox: an empty-field row, a typed app row, one of its other ways, or "Show n more apps". */
type Opt =
  | { t: 'row'; key: string; name: string; match: ToolMatch | null; row: Row }
  | { t: 'app'; key: string; name: string; match: ToolMatch | null; row: MergedRow }
  | { t: 'other'; key: string; name: string; match: null; row: MergedRow; src: Source }
  | { t: 'more'; key: string; name: string; match: null; n: number };

const KEY_DEFAULT = 'https://dashboard.composio.dev';
const POLL_MS = 3000;
const POLL_FOR_MS = 5 * 60 * 1000;

export const FindTool: React.FC<FindToolProps> = (p) => {
  const api = p.api ?? defaultFindToolApi;
  const phoneHook = usePhoneLayout();
  const phone = p.phone ?? phoneHook;
  const uid = useId().replace(/:/g, '');
  const inputRef = useRef<HTMLInputElement>(null);
  const keyRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const alive = useRef(true);

  const [query, setQuery] = useState(p.initialQuery ?? '');
  const [hl, setHl] = useState<string | null>(null);
  const [hlAuto, setHlAuto] = useState(true);
  const [ghostOff, setGhostOff] = useState(false);
  const [caretAtEnd, setCaretAtEnd] = useState(true);
  const [focusIn, setFocusIn] = useState(false);
  const [showAll, setShowAll] = useState(false);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [pins, setPins] = useState<Record<string, string>>({});
  const [own, setOwn] = useState<ConnectionsList | null>(null);
  const [apps, setApps] = useState<AppsLoad>({ s: 'loading' });
  const [dir, setDir] = useState<DirLoad | null>(null);
  const [dirMissing, setDirMissing] = useState(false);
  const [dirTry, setDirTry] = useState(0);
  const [appState, setAppState] = useState<Record<string, AppState>>({});
  const [keyOpen, setKeyOpen] = useState<KeyOpen>(null);
  const [keyPhase, setKeyPhase] = useState<KeyPhase>({ s: 'idle' });
  const [live, setLive] = useState('');
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const data = p.data !== undefined ? p.data : own;
  const composio: CatalogEntry | undefined = data?.catalog.find((e) => e.id === 'composio');
  const connections = data?.connections ?? [];
  const composioAdded = !!composio?.added;
  const keyUrl = (composio?.key_url && /^https:\/\//.test(composio.key_url) ? composio.key_url : null) ?? KEY_DEFAULT;

  const reloadOwn = useCallback(async () => {
    if (p.data !== undefined) return void (await p.onChanged?.());
    try {
      const fresh = await api.list();
      if (alive.current) setOwn(fresh);
    } catch {
      /* Composio rows fall back to the bundled apps; the setup row still works. */
    }
  }, [p.data !== undefined]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (p.data === undefined) void reloadOwn();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const loadApps = useCallback(async () => {
    try {
      const list = await api.apps();
      if (!alive.current) return null;
      setApps({ s: 'ok', list });
      return list;
    } catch (e) {
      if (!alive.current) return null;
      if (e instanceof ConnectionsError && e.status === 409) setApps({ s: 'refused', text: e.message });
      else if (e instanceof ConnectionsError && e.status === 404) setApps({ s: 'none' });
      else setApps({ s: 'down', text: e instanceof Error ? e.message : '' });
      return null;
    }
  }, [api]);

  // Loaded once when Find a tool opens (ETag), and again when Composio's setup changes.
  useEffect(() => {
    void loadApps();
  }, [loadApps, composio?.added, composio?.mode]);

  // ── Composio's apps (a refused key hides them; the others still show) ──
  const composioApps: ComposioRowApp[] = useMemo(() => {
    const listed = apps.s === 'ok' ? apps.list.apps : apps.s === 'loading' || apps.s === 'down' || apps.s === 'none' ? FALLBACK_APPS : [];
    const fromCatalog = new Set((composio?.apps_connected ?? []).map((a) => a.slug));
    return listed.map((a) => ({
      slug: a.slug,
      name: a.name,
      description: KNOWN_BLURBS[a.slug] ?? '',
      tools: typeof a.tools_count === 'number' ? a.tools_count : null,
      connected: !!a.connected || fromCatalog.has(a.slug) || appState[a.slug]?.s === 'connected',
    }));
  }, [apps, composio?.apps_connected, appState]);

  // ── The public list: from 2 letters, 120 ms after the last key, newer cancels older ──
  const q2 = normalize(query).length >= 2;
  useEffect(() => {
    if (dirMissing || !q2) return;
    const ctl = new AbortController();
    const t = window.setTimeout(async () => {
      setDir((d) => (d ? { ...d, loading: true } : { q: query, entries: [], unavailable: false, loading: true }));
      try {
        const answer = await api.directory(query.trim(), ctl.signal);
        if (ctl.signal.aborted || !alive.current) return;
        if (answer === null) return setDirMissing(true);
        setDir({ q: query, entries: answer.entries, unavailable: answer.unavailable, loading: false });
      } catch (e) {
        if ((e as Error)?.name === 'AbortError' || ctl.signal.aborted) return;
        setDir({ q: query, entries: [], unavailable: true, loading: false });
      }
    }, 120);
    return () => {
      window.clearTimeout(t);
      ctl.abort();
    };
  }, [query, q2, dirMissing, dirTry]); // eslint-disable-line react-hooks/exhaustive-deps

  const typed = query.trim().length > 0;
  const dirFresh = !!dir && normalize(dir.q) === normalize(query);
  const dirLoading = typed && q2 && !dirMissing && (!dir || dir.loading || !dirFresh);
  const dirDown = typed && q2 && !dirMissing && !dirLoading && !!dir?.unavailable;
  const composioLoading = typed && apps.s === 'loading';
  const composioDown = typed && apps.s === 'down' && composioAdded;

  // The empty field: Connected and Popular, as before.
  const groups = useMemo(() => buildGroups({ composioApps, connections }), [composioApps, connections]);
  // Typed: one row per app. Entries from an older query stay only while they still match (no flash on each key).
  const merged = useMemo(() => {
    if (!typed) return [];
    let entries: DirectoryEntry[] | null = null;
    if (q2 && !dirMissing && dir) entries = dirFresh ? dir.entries : dir.entries.filter((e) => matchTool(query, { name: e.title?.trim() || e.registry_name }));
    return mergeRows(query, { presets: MCP_PRESETS, composio: composioApps, directory: entries, connections, pinned: pins });
  }, [typed, query, q2, dirMissing, dir, dirFresh, composioApps, connections, pins]);
  const rowsShown = showAll ? merged : merged.slice(0, ROWS_SHOWN);

  const showEmptyGroups = !typed && (p.browseWhenEmpty || focusIn);
  const opts: Opt[] = useMemo(() => {
    if (!typed) return showEmptyGroups ? groups.flatMap((g) => g.rows.map((row): Opt => ({ t: 'row', key: row.key, name: row.name, match: null, row }))) : [];
    const out: Opt[] = [];
    for (const row of rowsShown) {
      out.push({ t: 'app', key: row.key, name: row.name, match: row.match, row });
      if (open.has(row.key)) row.others.slice(0, OTHER_WAYS_SHOWN).forEach((src) => out.push({ t: 'other', key: `${row.key}~${src.id}`, name: row.name, match: null, row, src }));
    }
    const rest = merged.length - rowsShown.length;
    if (rest > 0) out.push({ t: 'more', key: 'more', name: `Show ${rest} more apps`, match: null, n: rest });
    return out;
  }, [typed, showEmptyGroups, groups, rowsShown, merged.length, open]);

  const active = opts.find((o) => o.key === hl) ?? (typed && hlAuto ? opts[0] : undefined) ?? null;
  const ghost = active && typed && !ghostOff && caretAtEnd && (active.t === 'app' || active.t === 'row') && active.match?.kind === 'start' ? ghostRest(query, active.name) : '';
  const optionId = (o: { key: string }) => `${uid}-opt-${o.key.replace(/[^A-Za-z0-9_-]/g, '_')}`;
  const optionCount = typed ? rowsShown.length : opts.length;

  // The highlight belongs to an app, not a position: once the first row is highlighted, it stays on that app when the
  // public list lands and other rows move in above it, so Enter never runs a different app.
  useEffect(() => {
    if (typed && hlAuto && hl === null && active) setHl(active.key);
  }, [typed, hlAuto, hl, active?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  // The polite live line, 400 ms after typing stops.
  useEffect(() => {
    const t = window.setTimeout(() => setLive(typed || showEmptyGroups ? suggestionsLine(optionCount) : ''), 400);
    return () => window.clearTimeout(t);
  }, [optionCount, typed, showEmptyGroups]);

  // The highlighted option scrolls into view.
  useEffect(() => {
    if (!active) return;
    document.getElementById(optionId(active))?.scrollIntoView?.({ block: 'nearest' });
  }, [active?.key]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Waiting rows: poll Composio's list every 3 s for 5 minutes, and when the window is back ──
  const waiting = Object.entries(appState).filter(([, s]) => s.s === 'waiting').map(([slug]) => slug);
  useEffect(() => {
    if (!waiting.length) return;
    const check = async () => {
      const list = await loadApps();
      if (!list || !alive.current) return;
      const now = new Set(list.apps.filter((a) => a.connected).map((a) => a.slug));
      const done = waiting.filter((slug) => now.has(slug));
      if (!done.length) return;
      setAppState((m) => ({ ...m, ...Object.fromEntries(done.map((s) => [s, { s: 'connected' } as AppState])) }));
      void reloadOwn();
    };
    const started = Math.min(...waiting.map((s) => (appState[s] as { since: number }).since));
    const iv = window.setInterval(() => {
      if (Date.now() - started > POLL_FOR_MS) return window.clearInterval(iv);
      void check();
    }, POLL_MS);
    const onFocus = () => void check();
    window.addEventListener('focus', onFocus);
    return () => {
      window.clearInterval(iv);
      window.removeEventListener('focus', onFocus);
    };
  }, [waiting.join(',')]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Actions ──
  const composioUsers = (): string[] => {
    const id = composio?.connection_id;
    return id ? (connections.find((c) => c.id === id)?.used_by ?? []).map((u) => u.name) : [];
  };

  const openKey = (app: { slug: string; name: string } | null, replace = false) => {
    setKeyOpen({ slug: app?.slug ?? null, name: app?.name ?? null, replace });
    setKeyPhase({ s: 'idle' });
    window.setTimeout(() => keyRef.current?.focus(), 0);
  };

  /** Starts an app's sign-in: the tab opens inside the click or key event, its address follows. */
  const startSignIn = async (slug: string, tab: Window | null) => {
    setAppState((m) => ({ ...m, [slug]: { s: 'waiting', url: null, since: Date.now() } }));
    try {
      const { redirect_url } = await api.connect(slug);
      if (!/^https:\/\//.test(redirect_url)) throw new Error('Composio sent an address we can’t open.');
      if (tab && !tab.closed) tab.location.href = redirect_url;
      else window.open(redirect_url, '_blank', 'noopener,noreferrer');
      if (alive.current) setAppState((m) => ({ ...m, [slug]: { s: 'waiting', url: redirect_url, since: Date.now() } }));
    } catch (e) {
      tab?.close();
      if (!alive.current) return;
      setAppState((m) => {
        const next = { ...m };
        delete next[slug];
        return next;
      });
      setNotice(e instanceof Error ? e.message : 'Couldn’t start the sign-in.');
    }
  };

  const connectApp = (app: ComposioRowApp) => {
    setNotice(null);
    if (!composioAdded) return openKey(app);
    if (apps.s === 'refused' || composio?.problem?.action === 'replace_key') return openKey(app, true);
    if (!composio?.can_connect) return p.onComposioElsewhere?.(app.slug, app.name);
    const tab = window.open('about:blank', '_blank');
    void startSignIn(app.slug, tab);
  };

  const submitKey = async (key: string) => {
    if (!keyOpen) return;
    const { slug, name, replace } = keyOpen;
    // Open the sign-in tab now, inside the press, so a popup blocker lets it through.
    const tab = !replace && slug ? window.open('about:blank', '_blank') : null;
    setKeyPhase({ s: 'checking' });
    try {
      await (replace ? api.replaceKey(key) : api.addKey(key));
    } catch (e) {
      tab?.close();
      if (!alive.current) return;
      const text = e instanceof Error ? e.message : 'Something went wrong.';
      const refused = e instanceof ConnectionsError && (e.status === 409 || e.status === 400);
      setKeyPhase(refused ? { s: 'refused', text } : { s: 'down', text });
      window.setTimeout(() => keyRef.current?.focus(), 0);
      return;
    }
    if (!alive.current) return;
    setKeyOpen(null);
    setKeyPhase({ s: 'idle' });
    await reloadOwn();
    void loadApps();
    if (slug && name && !replace) {
      void startSignIn(slug, tab);
      setNotice(`Composio is added. Finish signing in to ${name} in the new tab.`);
    } else {
      setNotice(replace ? 'Key saved.' : 'Composio is added. No bot has it until you choose bots.');
    }
    inputRef.current?.focus();
  };

  const runComposio = (app: ComposioRowApp) => {
    const st = appState[app.slug];
    if (keyOpen && keyOpen.slug === app.slug) return keyRef.current?.focus();
    if (app.connected) return p.onChooseBots({ kind: 'composio', slug: app.slug, name: app.name });
    if (st?.s === 'waiting') {
      if (st.url) window.open(st.url, '_blank', 'noopener,noreferrer');
      return;
    }
    connectApp(app);
  };

  /** One way to add an app: the flow its button starts (design §3 "Buttons"). */
  const runSource = (s: Source) => {
    if (s.kind === 'builtin') {
      if (s.added) return p.onChooseBots({ kind: 'server', connectionId: s.added.id, name: s.preset.label });
      return p.onAddPreset(s.preset);
    }
    if (s.kind === 'directory') {
      if (!s.entry.already_added) p.onAddDirectory?.(s.entry);
      return;
    }
    runComposio(s.app);
  };

  /** What Enter (and the option's button) does. A pressed row keeps its lead for this query. */
  const run = (o: Opt) => {
    if (o.t === 'more') return setShowAll(true);
    if (o.t === 'row') {
      const r = o.row;
      if (r.kind === 'builtin') {
        if (r.added) return p.onChooseBots({ kind: 'server', connectionId: r.added.id, name: r.preset.label });
        return p.onAddPreset(r.preset);
      }
      return runComposio(r.app);
    }
    setPins((m) => (m[o.row.appKey] === o.row.lead.id ? m : { ...m, [o.row.appKey]: o.row.lead.id }));
    runSource(o.t === 'app' ? o.row.lead : o.src);
  };

  const toggleOthers = (row: MergedRow, to?: boolean) => {
    const now = open.has(row.key);
    const next = to ?? !now;
    if (next === now) return false;
    setOpen((s) => {
      const n = new Set(s);
      if (next) n.add(row.key);
      else n.delete(row.key);
      return n;
    });
    // Closing while one of its other ways is highlighted: the highlight goes back to the row.
    if (!next && active?.t === 'other' && active.row.key === row.key) setHl(row.key);
    return true;
  };

  const move = (d: 1 | -1) => {
    if (!opts.length) return;
    const i = active ? opts.indexOf(active) : -1;
    const next = i + d;
    setGhostOff(false);
    if (next < 0) {
      setHl(null);
      setHlAuto(false);
      return;
    }
    if (next >= opts.length) return;
    setHl(opts[next].key);
  };

  const accept = () => {
    if (!ghost || !active) return false;
    setQuery(active.name);
    setHl(active.t === 'row' ? `app-${appKey(active.name)}` : active.key);
    setHlAuto(true);
    return true;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    const atEnd = e.currentTarget.selectionStart === e.currentTarget.value.length && e.currentTarget.selectionEnd === e.currentTarget.value.length;
    const owner = active && (active.t === 'app' || active.t === 'other') ? active.row : null;
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (!typed && !showEmptyGroups) setFocusIn(true);
        move(1);
        break;
      case 'ArrowUp':
        e.preventDefault();
        move(-1);
        break;
      case 'Tab':
        if (!e.shiftKey && ghost && accept()) {
          e.preventDefault();
          e.stopPropagation();
        }
        break;
      case 'ArrowRight':
        if (ghost && atEnd && accept()) e.preventDefault();
        else if (!ghost && atEnd && owner?.others.length && toggleOthers(owner, true)) e.preventDefault();
        break;
      case 'ArrowLeft':
        if (!ghost && atEnd && owner && toggleOthers(owner, false)) e.preventDefault();
        break;
      case 'Enter':
        if (active) {
          e.preventDefault();
          run(active);
        }
        break;
      case 'Escape':
        if (ghost) {
          e.preventDefault();
          e.stopPropagation();
          setGhostOff(true);
        } else if (query) {
          e.preventDefault();
          e.stopPropagation();
          change('');
        } else if (p.onEscapeEmpty) {
          e.preventDefault();
          e.stopPropagation();
          p.onEscapeEmpty();
        }
        break;
    }
  };

  const change = (v: string) => {
    setQuery(v);
    setHl(null);
    setHlAuto(true);
    setGhostOff(false);
    setCaretAtEnd(true);
    setShowAll(false);
    setPins({});
    setOpen(new Set());
  };

  // ── Pieces ──
  const enterCap = (isActive: boolean) =>
    isActive &&
    !phone && (
      <span className={keycap} aria-hidden="true" data-enter-keycap>
        ↵
      </span>
    );

  const mainButton = (name: string, label: React.ReactNode, onClick: () => void, o: { active: boolean; primary?: boolean; small?: boolean; attr?: string }) => (
    <span className={`flex items-center gap-2 ${phone ? 'w-full' : ''}`}>
      {!o.small && enterCap(o.active)}
      <button
        type="button"
        tabIndex={-1}
        className={`${o.active || o.primary ? primaryBtn : quietBtn} ${o.small ? 'sm:min-h-[28px] sm:h-7 sm:px-2.5' : ''} ${phone ? 'w-full' : ''}`}
        onMouseDown={(e) => e.preventDefault()}
        onClick={onClick}
        data-row-action={o.attr ?? 'main'}
      >
        {label}
        <span className="sr-only"> {name}</span>
      </button>
    </span>
  );

  /** The status of a way that is already in use, or null. */
  const inUse = (s: Source): { text: string; ok: boolean } | { waiting: true } | null => {
    if (s.kind === 'composio') {
      if (s.app.connected) {
        const who = composioUsers();
        return { text: who.length ? `Connected · ${listNames(who)} can use it` : 'Connected · No bot can use it yet.', ok: who.length > 0 };
      }
      if (appState[s.app.slug]?.s === 'waiting') return { waiting: true };
      return null;
    }
    if (s.kind === 'builtin' && s.added) {
      const who = s.added.used_by.map((u) => u.name);
      return { text: who.length ? `Connected · ${listNames(who)} can use it` : 'Connected · No bot can use it yet.', ok: who.length > 0 };
    }
    if (s.kind === 'directory' && s.connected) return { text: 'Added', ok: true };
    return null;
  };

  const howOf = (s: Source, row: MergedRow, other: boolean) => signInOf(s, { rowName: row.name, composioAdded, other });

  /** The How line: chip, then how you sign in (or the connected / waiting status). */
  const howLine = (s: Source, row: MergedRow, other: boolean) => {
    const status = inUse(s);
    const how = howOf(s, row, other);
    const Icon = SIGN_IN_ICON[how.icon];
    return (
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-xs text-zinc-300 min-w-0" data-how>
        <SourceChip s={s} />
        {status && 'waiting' in status ? (
          <span className="text-zinc-300">Waiting for you to finish signing in, in the new tab.</span>
        ) : status ? (
          <span className={status.ok ? 'text-zinc-300' : 'text-amber-300'} data-in-use>
            {status.ok && <Check size={13} className="inline-block align-[-2px] mr-1 text-emerald-300" aria-hidden="true" />}
            {status.text}
          </span>
        ) : (
          <span data-sign-in={how.icon}>
            <Icon size={13} className="inline-block align-[-2px] mr-1" aria-hidden="true" />
            {how.text}
          </span>
        )}
      </span>
    );
  };

  const sourceAction = (s: Source, row: MergedRow, o: { active: boolean; small?: boolean; onClick: () => void }) => {
    const status = inUse(s);
    if (s.kind === 'directory' && s.connected)
      return (
        <span className="inline-flex items-center gap-1 text-xs text-zinc-400 whitespace-nowrap">
          <Check size={12} aria-hidden="true" /> Added
        </span>
      );
    if (status && 'waiting' in status)
      return mainButton(
        row.name,
        <>
          Open sign-in again <ExternalLink size={12} aria-hidden="true" />
        </>,
        o.onClick,
        o,
      );
    if (status) return mainButton(row.name, 'Choose bots', o.onClick, { ...o, primary: !status.ok });
    return mainButton(row.name, howOf(s, row, !!o.small).button, o.onClick, o);
  };

  const statusLabel = (s: Source, row: MergedRow, other: boolean): string => {
    const status = inUse(s);
    if (status && 'waiting' in status) return 'waiting for you to finish signing in';
    if (status) return lowerFirst(status.text.replace(/\.$/, ''));
    return lowerFirst(howOf(s, row, other).text);
  };

  const keySetup = (app: { slug: string; name: string }) => (
    <div className={`mt-3 ${phone ? '' : 'pl-[42px]'} space-y-2`} data-composio-setup={app.slug}>
      <p className="text-sm font-medium text-zinc-100">{app.name} and hundreds more apps through Composio</p>
      <p className="text-[13px] text-zinc-300">
        Composio is a service with a free plan that signs your bots in to apps. Add your key once and your bots can use any app you connect.
      </p>
      <KeyField
        id={`${uid}-key`}
        app={app}
        replace={false}
        keyUrl={keyUrl}
        phone={phone}
        phase={keyPhase}
        inputRef={keyRef}
        onSubmit={submitKey}
        onCancel={() => (setKeyOpen(null), inputRef.current?.focus())}
      />
    </div>
  );

  // ── A typed app row (§1) ──
  const renderApp = (o: Extract<Opt, { t: 'app' }>) => {
    const row = o.row;
    const s = row.lead;
    const isActive = active?.key === o.key;
    const isOpen = open.has(row.key);
    const setupApp =
      keyOpen && !keyOpen.replace ? [s, ...row.others].find((x): x is Extract<Source, { kind: 'composio' }> => x.kind === 'composio' && x.app.slug === keyOpen.slug) : undefined;
    const description =
      s.kind === 'directory' ? markCutDescription(s.description) : s.kind === 'builtin' ? s.description.replace(/\s*Sign in after adding\.$/, '') : s.description;
    const slug = s.kind === 'composio' ? s.app.slug : s.kind === 'builtin' ? s.preset.id : row.appKey;
    const late = s.kind === 'directory';
    const linkId = `${optionId(o)}-ways`;
    const setupHere = !!(setupApp && setupApp.id === s.id);
    return (
      <li
        key={o.key}
        id={optionId(o)}
        role="option"
        aria-selected={isActive}
        aria-label={`${row.name}, ${sourceWords(s)}, ${statusLabel(s, row, false)}`}
        onMouseMove={() => hl !== o.key && setHl(o.key)}
        className={`px-3.5 py-3 ${isActive ? `${activeBg} ${activeBar}` : ''} ${late ? fade : ''}`}
        data-row="app"
        data-row-key={o.key}
        data-lead={s.id}
        data-active={isActive || undefined}
      >
        <div className={phone ? 'space-y-2.5' : 'flex items-start gap-3'}>
          <div className="flex items-start gap-2.5 flex-1 min-w-0">
            <Mark name={row.name} slug={slug} />
            <div className="min-w-0 flex-1">
              <Name name={row.name} bold={row.match?.bold ?? null} />
              {description && (
                <p className={`text-[13px] text-zinc-300 ${phone ? 'line-clamp-2' : 'truncate'}`} title={s.description}>
                  {description}
                </p>
              )}
              <div className="mt-1.5">{howLine(s, row, false)}</div>
              {phone && !setupHere && <div className="mt-2.5">{sourceAction(s, row, { active: isActive, onClick: () => run(o) })}</div>}
              {row.others.length > 0 && (
                <button
                  type="button"
                  tabIndex={-1}
                  aria-expanded={isOpen}
                  aria-controls={isOpen ? `${linkId}-list` : undefined}
                  className={`${linkBtn} mt-1.5 ${phone ? 'h-11' : 'sm:min-h-0 h-6'}`}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => toggleOthers(row)}
                  data-other-ways={row.key}
                >
                  {isOpen ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
                  {otherWaysLabel(row.others)}
                  <span className="sr-only"> for {row.name}</span>
                </button>
              )}
            </div>
          </div>
          {!phone && !setupHere && <div className="shrink-0">{sourceAction(s, row, { active: isActive, onClick: () => run(o) })}</div>}
        </div>
        {setupApp && keySetup(setupApp.app)}
      </li>
    );
  };

  // ── One other way to add the app, under its row (its own option, so ↑/↓ reach it) ──
  const renderOther = (o: Extract<Opt, { t: 'other' }>, i: number, all: Source[]) => {
    const { row, src } = o;
    const isActive = active?.key === o.key;
    const first = i === 0;
    const last = i === Math.min(all.length, OTHER_WAYS_SHOWN) - 1;
    const rest = all.length - OTHER_WAYS_SHOWN;
    return (
      <li
        key={o.key}
        id={optionId(o)}
        role="option"
        aria-selected={isActive}
        aria-label={`${row.name}, also ${sourceWords(src)}, ${statusLabel(src, row, true)}`}
        onMouseMove={() => hl !== o.key && setHl(o.key)}
        className={`!border-t-0 px-3.5 ${last ? 'pb-3' : 'pb-1'} ${isActive ? `${activeBg} ${activeBar}` : ''}`}
        data-row="other"
        data-row-key={o.key}
        data-active={isActive || undefined}
      >
        <div id={first ? `${optionId({ key: row.key })}-ways-list` : undefined} className={`ml-[42px] pl-3 border-l border-zinc-700 ${first ? 'pt-1' : 'pt-1'} ${phone ? 'space-y-2' : 'flex items-center gap-3'}`}>
          <div className="flex-1 min-w-0">{howLine(src, row, true)}</div>
          <div className={phone ? '' : 'shrink-0'}>{sourceAction(src, row, { active: isActive, small: true, onClick: () => run(o) })}</div>
        </div>
        {last && rest > 0 && <p className="ml-[42px] pl-3 pt-2 border-l border-zinc-700 text-xs text-zinc-400">{moreOtherWays(rest)}</p>}
      </li>
    );
  };

  const renderMore = (o: Extract<Opt, { t: 'more' }>) => {
    const isActive = active?.key === o.key;
    return (
      <li
        key={o.key}
        id={optionId(o)}
        role="option"
        aria-selected={isActive}
        onMouseMove={() => hl !== o.key && setHl(o.key)}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => run(o)}
        className={`px-3.5 ${phone ? 'py-3 pl-[56px]' : 'py-2.5 pl-[60px]'} cursor-pointer ${isActive ? `${activeBg} ${activeBar}` : ''}`}
        data-more="apps"
      >
        <span className="text-xs font-medium text-sky-300">{o.name}</span>
      </li>
    );
  };

  // ── An empty-field row (Connected, Popular): unchanged ──
  const rowAction = (r: Row, isActive: boolean) => {
    const main = (label: React.ReactNode, onClick: () => void, extra?: { primary?: boolean }) => mainButton(r.name, label, onClick, { active: isActive, primary: extra?.primary });
    const o: Opt = { t: 'row', key: r.key, name: r.name, match: null, row: r };
    if (r.kind === 'builtin') {
      if (r.added)
        return (
          <span className={`flex items-center gap-3 ${phone ? 'w-full' : ''}`}>
            <span className="inline-flex items-center gap-1 text-xs text-zinc-400 whitespace-nowrap">
              <Check size={12} aria-hidden="true" /> Added
            </span>
            {main('Choose bots', () => run(o), { primary: !r.added.used_by.length })}
          </span>
        );
      return main('Add', () => run(o));
    }
    const st = appState[r.app.slug];
    if (r.app.connected)
      return (
        <span className={`flex items-center gap-3 ${phone ? 'w-full' : ''}`}>
          <span className="inline-flex items-center gap-1 text-xs text-emerald-300 whitespace-nowrap">
            <Check size={12} aria-hidden="true" /> Connected
          </span>
          {main('Choose bots', () => run(o), { primary: !composioUsers().length })}
        </span>
      );
    if (st?.s === 'waiting')
      return main(
        <>
          Open sign-in again <ExternalLink size={12} aria-hidden="true" />
        </>,
        () => run(o),
      );
    return main('Connect', () => run(o));
  };

  const rowMeta = (r: Row): React.ReactNode => {
    if (r.kind === 'builtin') {
      if (r.added) {
        const who = r.added.used_by.map((u) => u.name);
        return who.length ? <span className="text-xs text-zinc-400">{listNames(who)} can use it</span> : <span className="text-xs text-amber-300">No bot can use it yet.</span>;
      }
      if (r.group === 'connected') return null;
      return (
        <>
          <span className={chip}>{r.preset.form.transport === 'http' ? 'Remote' : 'Runs on this computer'}</span>
          {r.preset.needs.includes('Sign-in') && <span className={amberChip}>Sign in after adding</span>}
        </>
      );
    }
    if (r.app.connected) {
      const who = composioUsers();
      return who.length ? <span className="text-xs text-zinc-400">{listNames(who)} can use it</span> : <span className="text-xs text-amber-300">No bot can use it yet.</span>;
    }
    if (appState[r.app.slug]?.s === 'waiting') return <span className="text-xs text-zinc-300">Waiting for you to finish signing in, in the new tab.</span>;
    return null;
  };

  const renderRow = (r: Row) => {
    const isActive = active?.key === r.key;
    const small = r.kind === 'builtin' ? 'built in' : 'through Composio';
    const meta = rowMeta(r);
    const description = r.kind === 'builtin' ? r.preset.blurb : r.app.description;
    const slug = r.kind === 'composio' ? r.app.slug : r.preset.id;
    const setupHere = r.kind === 'composio' && keyOpen && !keyOpen.replace && keyOpen.slug === r.app.slug;
    return (
      <li
        key={r.key}
        id={optionId(r)}
        role="option"
        aria-selected={isActive}
        aria-label={`${r.name}, ${small}`}
        onMouseMove={() => hl !== r.key && setHl(r.key)}
        className={`px-3.5 py-3 ${isActive ? `${activeBg} ${activeBar}` : ''}`}
        data-row={r.kind}
        data-row-key={r.key}
        data-active={isActive || undefined}
      >
        <div className={phone ? 'space-y-2.5' : 'flex items-center gap-3'}>
          <div className={`flex ${description || meta ? 'items-start' : 'items-center'} gap-2.5 flex-1 min-w-0`}>
            <Mark name={r.name} slug={slug} />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-baseline gap-x-1.5">
                <Name name={r.name} bold={null} />
                <span className="text-xs text-zinc-400">{small}</span>
              </div>
              {description && (
                <p className="text-[13px] text-zinc-300 truncate" title={description}>
                  {description}
                </p>
              )}
              {meta && <div className="mt-1 flex flex-wrap items-center gap-1.5">{meta}</div>}
            </div>
          </div>
          {!setupHere && <div className={phone ? 'pl-[42px]' : 'shrink-0'}>{rowAction(r, isActive)}</div>}
        </div>
        {setupHere && keySetup({ slug: r.app.slug, name: r.app.name })}
      </li>
    );
  };

  const retry = (what: { composio: boolean; dir: boolean }) => {
    if (what.composio) {
      setApps({ s: 'loading' });
      void loadApps();
    }
    if (what.dir) setDirTry((n) => n + 1);
  };

  // One quiet line at the end of the card: still loading, or a source that couldn't be reached.
  const statusLine = (): React.ReactNode => {
    if (!typed) return null;
    const tryAgain = (what: { composio: boolean; dir: boolean }) => (
      <button type="button" className={linkBtn} onClick={() => retry(what)} data-try-again>
        <RefreshCw size={12} aria-hidden="true" /> Try again
      </button>
    );
    const down = (text: string, what: { composio: boolean; dir: boolean }) => (
      <>
        <AlertTriangle size={14} className="shrink-0 text-amber-300" aria-hidden="true" />
        <span className="min-w-0 flex-1">{text}</span>
        {tryAgain(what)}
      </>
    );
    let body: React.ReactNode = null;
    let state = '';
    if (composioDown && dirDown) {
      state = 'down-both';
      body = down('Couldn’t reach Composio or the public list. Built-in tools still show.', { composio: true, dir: true });
    } else if (dirLoading || composioLoading) {
      state = 'loading';
      body = (
        <>
          <Spinner /> <span>Searching more apps…</span>
        </>
      );
    } else if (composioDown) {
      state = 'down-composio';
      body = down('Couldn’t reach Composio, so its apps aren’t in this list.', { composio: true, dir: false });
    } else if (dirDown) {
      state = 'down-directory';
      body = down('Couldn’t reach the public list of tool servers, so only built-in tools and Composio apps are shown.', { composio: false, dir: true });
    }
    if (!body) return null;
    return (
      <div role="status" className={`flex flex-wrap items-center gap-x-2 gap-y-1 px-3.5 ${phone ? 'py-1' : 'py-2'} text-xs text-zinc-300`} data-find-status={state}>
        {body}
      </div>
    );
  };

  const refusedAlert =
    typed && apps.s === 'refused' ? (
      <div role="alert" className="px-3 py-2.5 rounded-xl border border-rose-500/40 bg-rose-500/10 text-xs text-rose-200 space-y-2" data-composio-state="refused">
        <p className="flex items-start gap-2">
          <AlertTriangle size={14} className="mt-px shrink-0" aria-hidden="true" />
          Composio didn’t accept your saved key, so its apps can’t be listed.
        </p>
        {keyOpen?.replace ? (
          <div className="pl-[22px]">
            <KeyField id={`${uid}-key`} app={null} replace keyUrl={keyUrl} phone={phone} phase={keyPhase} inputRef={keyRef} onSubmit={submitKey} onCancel={() => setKeyOpen(null)} />
          </div>
        ) : (
          <button type="button" className={`${quietBtn} ml-[22px]`} onClick={() => openKey(null, true)} data-replace-key>
            <KeyRound size={12} aria-hidden="true" /> Replace key
          </button>
        )}
      </div>
    ) : null;

  const status = statusLine();
  const anyDown = composioDown || dirDown;
  const nothing = typed && merged.length === 0 && !dirLoading && !composioLoading && !anyDown && apps.s !== 'refused';
  const shownSources = rowsShown.flatMap((r) => [r.lead, ...(open.has(r.key) ? r.others.slice(0, OTHER_WAYS_SHOWN) : [])]);
  const notes = { composio: shownSources.some((s) => s.origin === 'composio'), community: shownSources.some((s) => s.origin === 'community') };
  const typedCard = typed && (opts.length > 0 || !!status);
  const listOpen = typed ? opts.length > 0 : showEmptyGroups && opts.length > 0;

  // The other ways of an open row, in order, so each line knows if it is first or last.
  const renderTyped = () =>
    opts.map((o) => {
      if (o.t === 'app') return renderApp(o);
      if (o.t === 'more') return renderMore(o);
      if (o.t === 'other') {
        const shown = o.row.others.slice(0, OTHER_WAYS_SHOWN);
        return renderOther(o, shown.indexOf(o.src), o.row.others);
      }
      return null;
    });

  return (
    <div
      ref={rootRef}
      className="space-y-3"
      data-find-tool
      onFocus={() => setFocusIn(true)}
      onBlur={(e) => {
        if (!rootRef.current?.contains(e.relatedTarget as Node | null)) setFocusIn(false);
      }}
    >
      <style>{'@keyframes findtool-in{from{opacity:0}to{opacity:1}}'}</style>
      <div className="relative">
        <Search size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400 pointer-events-none" aria-hidden="true" />
        {/* The ghost sits under the typed text: the typed part is invisible, the rest is zinc-500. */}
        {ghost && (
          <div
            aria-hidden="true"
            className={`absolute inset-y-0 left-0 right-0 flex items-center pl-[38px] pr-20 whitespace-pre overflow-hidden ${phone ? 'text-base' : 'text-sm'}`}
            onMouseDown={(e) => {
              e.preventDefault();
              accept();
              inputRef.current?.focus();
            }}
            data-ghost
          >
            <span className="invisible">{query}</span>
            <span className="text-zinc-500 pointer-events-auto cursor-text">{ghost}</span>
          </div>
        )}
        <input
          ref={inputRef}
          type="text"
          role="combobox"
          aria-label={p.label ?? 'Find a tool'}
          aria-autocomplete="both"
          aria-expanded={listOpen}
          aria-controls={`${uid}-results`}
          aria-activedescendant={active ? optionId(active) : undefined}
          aria-describedby={`${uid}-live`}
          enterKeyHint="go"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          autoFocus={p.autoFocus}
          value={query}
          placeholder={p.placeholder ?? 'Find a tool, like gmail, stripe or a database'}
          onChange={(e) => change(e.target.value)}
          onKeyDown={onKeyDown}
          onSelect={(e) => {
            const t = e.currentTarget;
            setCaretAtEnd(t.selectionStart === t.value.length && t.selectionEnd === t.value.length);
          }}
          className={`relative w-full ${phone ? 'h-11 text-base' : 'h-11 sm:h-10 text-base sm:text-sm'} pl-[38px] pr-20 rounded-[10px] bg-transparent border border-zinc-700 text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-400 focus:ring-[3px] focus:ring-sky-400/25`}
          style={{ backgroundColor: ghost ? 'transparent' : undefined }}
          data-find-input
        />
        <div className="absolute right-1.5 top-1/2 -translate-y-1/2 flex items-center gap-1.5">
          {ghost && !phone && (
            <span className={keycap} aria-hidden="true" data-tab-keycap>
              Tab
            </span>
          )}
          {query && (
            <button
              type="button"
              aria-label="Clear"
              onClick={() => (change(''), inputRef.current?.focus())}
              className={`inline-flex items-center justify-center ${phone ? 'w-11 h-11' : 'w-11 h-11 sm:w-7 sm:h-7'} rounded-full focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400`}
              data-find-clear
            >
              <span className="inline-flex items-center justify-center w-5 h-5 rounded-full bg-zinc-700 text-zinc-200">
                <X size={12} aria-hidden="true" />
              </span>
            </button>
          )}
        </div>
      </div>
      <p id={`${uid}-live`} className="sr-only" aria-live="polite" data-find-live>
        {live}
      </p>
      {notice && (
        <p role="status" className="flex items-start gap-2 px-3 py-2 rounded-lg border border-zinc-700 bg-zinc-900/60 text-xs text-zinc-200" data-find-notice>
          {notice}
        </p>
      )}

      <div id={`${uid}-results`} className="space-y-4" data-find-results>
        {showEmptyGroups &&
          groups.map((g) => (
            <section key={g.key} className="motion-safe:animate-[findtool-in_120ms_ease-out]" data-group={g.key}>
              <div className="flex items-baseline justify-between gap-2 px-0.5 mb-1.5">
                <h4 id={`${uid}-g-${g.key}`} className="text-[11px] font-semibold uppercase tracking-wider text-zinc-300">
                  {GROUP_LABEL[g.key]}
                </h4>
              </div>
              <ul role="listbox" aria-labelledby={`${uid}-g-${g.key}`} className="rounded-xl border border-zinc-800 bg-[#181335] divide-y divide-zinc-800 overflow-hidden">
                {g.rows.map(renderRow)}
                {g.rest ? (
                  <li role="presentation" className="px-3.5 py-2.5 pl-[60px] text-xs text-zinc-400" data-connected-rest>
                    {g.rest} more. Type to find them.
                  </li>
                ) : null}
              </ul>
            </section>
          ))}
        {refusedAlert}
        {typedCard && (
          <div className="rounded-xl border border-zinc-800 bg-[#181335] overflow-hidden" data-find-card>
            {opts.length > 0 && (
              <ul role="listbox" aria-label={p.label ?? 'Find a tool'} className="divide-y divide-zinc-800" data-find-list>
                {renderTyped()}
              </ul>
            )}
            {status && <div className={opts.length ? 'border-t border-zinc-800' : ''}>{status}</div>}
          </div>
        )}
        {typed && (notes.composio || notes.community) && (
          <div className="space-y-1 px-0.5 text-[11px] text-zinc-500" data-find-notes>
            {notes.composio && (
              <p className="flex items-start gap-1.5">
                <Lock size={11} className="mt-[2px] shrink-0" aria-hidden="true" /> Through Composio: sign-ins stay on Composio’s servers.
              </p>
            )}
            {notes.community && (
              <p className="flex items-start gap-1.5">
                <Users size={11} className="mt-[2px] shrink-0" aria-hidden="true" /> Community: listed by their makers, not checked by us.
              </p>
            )}
          </div>
        )}
        {nothing && (
          <div className="flex flex-col items-center gap-1.5 px-4 py-5 rounded-xl border border-zinc-800 bg-[#181335] text-center" data-find-nothing>
            <p className="text-sm text-zinc-100">Nothing found for “{query.trim()}”.</p>
            <p className="text-xs text-zinc-300">Check the spelling, or add a tool by its address.</p>
            {p.onAddByAddress && (
              <button type="button" className={`${quietBtn} mt-1.5`} onClick={p.onAddByAddress} data-add-by-address-inline>
                <Plus size={14} aria-hidden="true" /> Add by address
              </button>
            )}
            {composioAdded && (
              <p className="mt-1 text-xs text-zinc-400" data-not-listed>
                Not listed? Bots with Composio find its other apps themselves.
              </p>
            )}
          </div>
        )}
        {typed && !phone && optionCount > 0 && (
          <p className="flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500" aria-hidden="true" data-find-legend>
            <span className={keycap}>↑</span>
            <span className={keycap}>↓</span> move ·<span className={keycap}>↵</span> runs the button ·<span className={keycap}>Tab</span> fills in the name ·
            <span className={keycap}>→</span> other ways ·<span className={keycap}>Esc</span> clears
          </p>
        )}
      </div>

      {p.browseWhenEmpty && !typed && p.children && (
        <section aria-labelledby={`${uid}-all`} data-browse>
          <div className="flex items-baseline justify-between gap-2 px-0.5 mb-1.5">
            <h4 id={`${uid}-all`} className="text-[11px] font-semibold uppercase tracking-wider text-zinc-300">
              All built-in tools
            </h4>
          </div>
          {p.children}
        </section>
      )}
    </div>
  );
};
