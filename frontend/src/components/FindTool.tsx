// Find a tool (composio-simple/design.md §1, §2 and "Autocomplete"): one field that finds a tool
// wherever it comes from (built in, through Composio, the public directory), from the first letter.
// Used in Add a tool (browseWhenEmpty) and on Apps & accounts. ARIA 1.2 combobox: focus stays in the
// field, ↑/↓ move a highlight across every group's rows, Enter runs the highlighted row's main button.
//
// Text from Composio and the directory is shown as plain text, never as HTML.

import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { AlertTriangle, Check, ExternalLink, Eye, EyeOff, KeyRound, Loader2, Lock, Plus, RefreshCw, Search, X } from 'lucide-react';
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
import { ToolMatch, ghostRest, normalize, rankTools, suggestionsLine } from '../toolSearch';
import { usePhoneLayout } from '../usePhoneLayout';

// ── Data ─────────────────────────────────────────────────────────────────────────────────────

export interface FindToolApi {
  list: () => Promise<ConnectionsList>;
  apps: () => Promise<ComposioAppList>;
  addKey: (key: string) => Promise<unknown>;
  replaceKey: (key: string) => Promise<unknown>;
  connect: (slug: string) => Promise<{ redirect_url: string }>;
  /** null: this server has no public directory yet, so the group hides. */
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
  /** The browse grid. */
  children?: React.ReactNode;
}

type GroupKey = 'connected' | 'popular' | 'builtin' | 'composio' | 'directory';

const GROUP_LABEL: Record<GroupKey, string> = {
  connected: 'Connected',
  popular: 'Popular',
  builtin: 'Built in',
  composio: 'Through Composio',
  directory: 'From the public directory',
};
const MORE_LABEL: Partial<Record<GroupKey, string>> = {
  builtin: 'built in',
  composio: 'through Composio',
  directory: 'in the public directory',
};

interface ComposioRowApp {
  slug: string;
  name: string;
  description: string;
  tools: number | null;
  connected: boolean;
}

type Row =
  | { key: string; group: GroupKey; kind: 'builtin'; name: string; match: ToolMatch | null; preset: McpPreset; added: Connection | null }
  | { key: string; group: GroupKey; kind: 'composio'; name: string; match: ToolMatch | null; app: ComposioRowApp }
  | { key: string; group: GroupKey; kind: 'directory'; name: string; match: ToolMatch | null; entry: DirectoryEntry }
  | { key: string; group: GroupKey; kind: 'more'; name: string; match: null; count: number; of: GroupKey };

interface Group {
  key: GroupKey;
  rows: Row[];
  /** "{n} more. Type to find them." in Connected; not an option. */
  rest?: number;
}

const POPULAR = ['gmail', 'googlecalendar', 'slack', 'notion', 'github', 'googledrive'];
const PER_GROUP = 5;

/** What an app does, for the Composio rows we know; Composio's lists carry no description. */
const KNOWN_BLURBS: Record<string, string> = Object.fromEntries([
  ...COMPOSIO_ONLY_APPS.map((a) => [a.toolkit, a.blurb]),
  ...APP_TILES.map((t) => [t.slug, `${t.blurb}.`]),
]);
const TINT: Record<string, string> = Object.fromEntries(APP_TILES.map((t) => [t.slug, t.tint]));

const FALLBACK_APPS: ComposioListedApp[] = COMPOSIO_ONLY_APPS.map((a) => ({ slug: a.toolkit, name: a.label }));

const presetUrl = (p: McpPreset) => (p.form.transport === 'http' ? p.form.url.trim().replace(/\/+$/, '').toLowerCase() : '');
const PRESET_URLS = new Set(MCP_PRESETS.map(presetUrl).filter(Boolean));

/** "by notion.com" for a remote built-in tool: the last two labels of its address. */
export function presetPublisher(p: McpPreset): string | null {
  if (p.form.transport !== 'http') return null;
  try {
    const host = new URL(p.form.url).hostname.split('.');
    return host.slice(-2).join('.');
  } catch {
    return null;
  }
}

/** The directory's key chips: "API key", "2 keys" or "Sign-in". */
export function directoryNeeds(e: DirectoryEntry): string | null {
  const keys = (e.inputs ?? []).filter((i) => i.secret || /key|token|secret/i.test(i.name)).length;
  if (keys === 1) return 'API key';
  if (keys > 1) return `${keys} keys`;
  return null;
}

const addedServer = (p: McpPreset, connections: Connection[]): Connection | null =>
  connections.find(
    (c) => c.kind === 'mcp' && (c.name.toLowerCase() === p.form.name.toLowerCase() || c.name.toLowerCase() === p.label.toLowerCase()),
  ) ?? null;

/** Every group for `query`, in order, before the Show more rows are cut. */
export function buildGroups(opts: {
  query: string;
  composioApps: ComposioRowApp[];
  composioAdded: boolean;
  directory: DirectoryEntry[] | null;
  connections: Connection[];
  expanded: Set<GroupKey>;
}): Group[] {
  const { query, composioApps, directory, connections, expanded } = opts;
  const q = normalize(query);
  const cut = (key: GroupKey, rows: Row[]): Group => {
    if (rows.length <= PER_GROUP || expanded.has(key)) return { key, rows };
    const more: Row = { key: `more-${key}`, group: key, kind: 'more', name: `Show ${rows.length - PER_GROUP} more ${MORE_LABEL[key]}`, match: null, count: rows.length - PER_GROUP, of: key };
    return { key, rows: [...rows.slice(0, PER_GROUP), more] };
  };
  const builtinRow = (group: GroupKey, preset: McpPreset, match: ToolMatch | null): Row => ({
    key: `${group}-b-${preset.id}`,
    group,
    kind: 'builtin',
    name: preset.label,
    match,
    preset,
    added: addedServer(preset, connections),
  });
  const composioRow = (group: GroupKey, app: ComposioRowApp, match: ToolMatch | null): Row => ({ key: `${group}-c-${app.slug}`, group, kind: 'composio', name: app.name, match, app });

  if (!q) {
    const connected: Row[] = [];
    MCP_PRESETS.forEach((p) => addedServer(p, connections) && connected.push(builtinRow('connected', p, null)));
    composioApps.filter((a) => a.connected).forEach((a) => connected.push(composioRow('connected', a, null)));
    const taken = new Set(connected.map((r) => (r.kind === 'builtin' ? r.preset.id : r.kind === 'composio' ? r.app.slug : '')));
    const popular: Row[] = [];
    POPULAR.forEach((slug) => {
      if (taken.has(slug)) return;
      const preset = MCP_PRESETS.find((p) => p.id === slug);
      if (preset) return void popular.push(builtinRow('popular', preset, null));
      const app = composioApps.find((a) => a.slug === slug);
      if (app) popular.push(composioRow('popular', app, null));
    });
    const groups: Group[] = [];
    if (connected.length) groups.push({ key: 'connected', rows: connected.slice(0, 4), rest: Math.max(0, connected.length - 4) || undefined });
    if (popular.length) groups.push({ key: 'popular', rows: popular });
    return groups;
  }

  const builtin = rankTools(query, MCP_PRESETS.map((p) => ({ name: p.label, slug: p.id, p })), (e) => (addedServer(e.p, connections) ? 1 : 0)).map((r) =>
    builtinRow('builtin', r.entry.p, r.match),
  );
  const composio = rankTools(query, composioApps, (a) => (a.connected ? 1e6 : 0) + (a.tools ?? 0)).map((r) => composioRow('composio', r.entry, r.match));
  const groups: Group[] = [];
  if (builtin.length) groups.push(cut('builtin', builtin));
  groups.push(cut('composio', composio));
  if (directory) {
    const seen = new Set<string>();
    const rows: Row[] = directory
      .filter((e) => !(e.url && PRESET_URLS.has(e.url.trim().replace(/\/+$/, '').toLowerCase())))
      .filter((e) => (seen.has(e.registry_name) ? false : (seen.add(e.registry_name), true)))
      .map((entry) => {
        const name = entry.title?.trim() || entry.registry_name;
        return { key: `directory-${entry.registry_name}`, group: 'directory' as GroupKey, kind: 'directory' as const, name, match: rankTools(query, [{ name }])[0]?.match ?? null, entry };
      });
    groups.push(cut('directory', rows));
  }
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

const openKeyPage = (url: string) => window.open(url, '_blank', 'noopener,noreferrer');

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
  const [expanded, setExpanded] = useState<Set<GroupKey>>(new Set());
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

  // ── Composio's apps as rows ──
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

  // ── The public directory: from 2 letters, 120 ms after the last key, newer cancels older ──
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

  const groups = useMemo(
    () =>
      buildGroups({
        query,
        composioApps,
        composioAdded,
        directory: dirMissing || !q2 ? null : dir?.entries ?? [],
        connections,
        expanded,
      }),
    [query, composioApps, composioAdded, dirMissing, q2, dir, connections, expanded],
  );
  const rows = useMemo(() => groups.flatMap((g) => g.rows), [groups]);
  const typed = query.trim().length > 0;
  const showEmptyGroups = !typed && (p.browseWhenEmpty || focusIn);
  const visibleRows = typed || showEmptyGroups ? rows : [];
  const active = visibleRows.find((r) => r.key === hl) ?? (typed && hlAuto ? visibleRows[0] : undefined) ?? null;
  const ghost = active && typed && !ghostOff && caretAtEnd && active.kind !== 'more' && active.match?.kind === 'start' ? ghostRest(query, active.name) : '';
  const optionId = (r: Row) => `${uid}-opt-${r.key}`;
  const optionCount = visibleRows.filter((r) => r.kind !== 'more').length;

  // The polite live line, 400 ms after typing stops.
  useEffect(() => {
    const t = window.setTimeout(() => setLive(typed || showEmptyGroups ? suggestionsLine(optionCount) : ''), 400);
    return () => window.clearTimeout(t);
  }, [optionCount, typed, showEmptyGroups]);

  // The highlighted row scrolls into view.
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
      if (!/^https:\/\//.test(redirect_url)) throw new Error('Composio sent an address kwant can’t open.');
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

  /** What Enter (and the row's main button) does. */
  const run = (r: Row) => {
    if (r.kind === 'more') {
      setExpanded((s) => new Set(s).add(r.of));
      setHl(null);
      setHlAuto(true);
      return;
    }
    if (r.kind === 'builtin') {
      if (r.added) return p.onChooseBots({ kind: 'server', connectionId: r.added.id, name: r.preset.label });
      return p.onAddPreset(r.preset);
    }
    if (r.kind === 'directory') {
      if (!r.entry.already_added) p.onAddDirectory?.(r.entry);
      return;
    }
    const st = appState[r.app.slug];
    if (keyOpen && keyOpen.slug === r.app.slug) return keyRef.current?.focus();
    if (r.app.connected) return p.onChooseBots({ kind: 'composio', slug: r.app.slug, name: r.app.name });
    if (st?.s === 'waiting') {
      if (st.url) window.open(st.url, '_blank', 'noopener,noreferrer');
      return;
    }
    connectApp(r.app);
  };

  const move = (dir: 1 | -1) => {
    if (!visibleRows.length) return;
    const i = active ? visibleRows.indexOf(active) : -1;
    const next = i + dir;
    setGhostOff(false);
    if (next < 0) {
      setHl(null);
      setHlAuto(false);
      return;
    }
    if (next >= visibleRows.length) return;
    setHl(visibleRows[next].key);
  };

  const accept = () => {
    if (!ghost || !active) return false;
    setQuery(active.name);
    setHl(active.key.replace(/^(connected|popular)-/, active.kind === 'builtin' ? 'builtin-' : 'composio-'));
    setHlAuto(true);
    return true;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    const atEnd = e.currentTarget.selectionStart === e.currentTarget.value.length && e.currentTarget.selectionEnd === e.currentTarget.value.length;
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
    if (!v.trim()) setExpanded(new Set());
  };

  // ── Rows ──
  const rowAction = (r: Row, isActive: boolean) => {
    const enterCap = isActive && !phone && (
      <span className={keycap} aria-hidden="true" data-enter-keycap>
        ↵
      </span>
    );
    const main = (label: React.ReactNode, onClick: () => void, extra?: { primary?: boolean; attr?: string }) => (
      <span className={`flex items-center gap-2 ${phone ? 'w-full' : ''}`}>
        {enterCap}
        <button
          type="button"
          tabIndex={-1}
          className={`${isActive || extra?.primary ? primaryBtn : quietBtn} ${phone ? 'w-full' : ''}`}
          onMouseDown={(e) => e.preventDefault()}
          onClick={onClick}
          data-row-action={extra?.attr ?? 'main'}
        >
          {label}
          <span className="sr-only"> {r.name}</span>
        </button>
      </span>
    );
    if (r.kind === 'builtin') {
      if (r.added)
        return (
          <span className={`flex items-center gap-3 ${phone ? 'w-full' : ''}`}>
            <span className="inline-flex items-center gap-1 text-xs text-zinc-400 whitespace-nowrap">
              <Check size={12} aria-hidden="true" /> Added
            </span>
            {main('Choose bots', () => run(r), { primary: !r.added.used_by.length })}
          </span>
        );
      return main('Add', () => run(r));
    }
    if (r.kind === 'directory') {
      if (r.entry.already_added)
        return (
          <span className="inline-flex items-center gap-1 text-xs text-zinc-400 whitespace-nowrap">
            <Check size={12} aria-hidden="true" /> Added
          </span>
        );
      return main('Add', () => run(r));
    }
    if (r.kind === 'composio') {
      const st = appState[r.app.slug];
      if (r.app.connected) {
        const users = composioUsers();
        return (
          <span className={`flex items-center gap-3 ${phone ? 'w-full' : ''}`}>
            <span className="inline-flex items-center gap-1 text-xs text-emerald-300 whitespace-nowrap">
              <Check size={12} aria-hidden="true" /> Connected
            </span>
            {main('Choose bots', () => run(r), { primary: !users.length })}
          </span>
        );
      }
      if (st?.s === 'waiting')
        return main(
          <>
            Open sign-in again <ExternalLink size={12} aria-hidden="true" />
          </>,
          () => run(r),
        );
      return main('Connect', () => run(r));
    }
    return null;
  };

  const rowSmall = (r: Row): string | null => {
    if (r.group === 'connected' || r.group === 'popular') return r.kind === 'builtin' ? 'built in' : r.kind === 'composio' ? 'through Composio' : null;
    if (r.kind === 'builtin') return presetPublisher(r.preset) ? `by ${presetPublisher(r.preset)}` : null;
    if (r.kind === 'composio') return r.app.tools ? `${r.app.tools} tools` : null;
    if (r.kind === 'directory') return r.entry.publisher?.label ? `by ${r.entry.publisher.label}` : null;
    return null;
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
    if (r.kind === 'directory') {
      const needs = directoryNeeds(r.entry);
      return (
        <>
          <span className={chip}>{r.entry.kind === 'remote' ? 'Remote' : 'Runs on this computer'}</span>
          {needs && <span className={chip}>{needs}</span>}
        </>
      );
    }
    if (r.kind === 'composio') {
      const st = appState[r.app.slug];
      if (r.app.connected) {
        const who = composioUsers();
        return who.length ? <span className="text-xs text-zinc-400">{listNames(who)} can use it</span> : <span className="text-xs text-amber-300">No bot can use it yet.</span>;
      }
      if (st?.s === 'waiting') return <span className="text-xs text-zinc-300">Waiting for you to finish signing in, in the new tab.</span>;
    }
    return null;
  };

  const renderRow = (r: Row) => {
    const isActive = active?.key === r.key;
    const activate = () => {
      if (hl !== r.key) setHl(r.key);
    };
    if (r.kind === 'more')
      return (
        <li
          key={r.key}
          id={optionId(r)}
          role="option"
          aria-selected={isActive}
          onMouseMove={activate}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => run(r)}
          className={`px-3.5 py-2.5 cursor-pointer ${phone ? 'pl-[56px]' : 'pl-[60px]'} ${isActive ? 'bg-[#272050] shadow-[inset_2px_0_0_#9d8fff]' : ''}`}
          data-more={r.of}
        >
          <span className="text-xs font-medium text-sky-300">{r.name}</span>
        </li>
      );
    const small = rowSmall(r);
    const meta = rowMeta(r);
    const description = r.kind === 'builtin' ? r.preset.blurb : r.kind === 'composio' ? r.app.description : r.entry.description;
    const slug = r.kind === 'composio' ? r.app.slug : r.kind === 'builtin' ? r.preset.id : undefined;
    const setupHere = r.kind === 'composio' && keyOpen && !keyOpen.replace && keyOpen.slug === r.app.slug;
    return (
      <li
        key={r.key}
        id={optionId(r)}
        role="option"
        aria-selected={isActive}
        aria-label={small ? `${r.name}, ${small}` : r.name}
        onMouseMove={activate}
        className={`px-3.5 py-3 ${isActive ? 'bg-[#272050] shadow-[inset_2px_0_0_#9d8fff]' : ''}`}
        data-row={r.kind}
        data-row-key={r.key}
        data-active={isActive || undefined}
      >
        <div className={phone ? 'space-y-2.5' : 'flex items-center gap-3'}>
          <div className="flex items-start gap-2.5 flex-1 min-w-0">
            <Mark name={r.name} slug={slug} />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-baseline gap-x-1.5">
                <Name name={r.name} bold={r.match?.bold ?? null} />
                {small && <span className={`text-xs ${isActive ? 'text-zinc-400' : 'text-zinc-400'}`}>{small}</span>}
              </div>
              {description && (
                <p className={`text-[13px] text-zinc-300 ${r.kind === 'directory' ? 'line-clamp-2' : 'truncate'}`} title={description}>
                  {description}
                </p>
              )}
              {meta && <div className="mt-1 flex flex-wrap items-center gap-1.5">{meta}</div>}
            </div>
          </div>
          {!setupHere && <div className={phone ? 'pl-[42px]' : 'shrink-0'}>{rowAction(r, isActive)}</div>}
        </div>
        {setupHere && (
          <div className={`mt-3 ${phone ? '' : 'pl-[42px]'} space-y-2`} data-composio-setup={r.app.slug}>
            <p className="text-sm font-medium text-zinc-100">{r.app.name} and hundreds more apps through Composio</p>
            <p className="text-[13px] text-zinc-300">
              Composio is a service with a free plan that signs your bots in to apps. Add your key once and your bots can use any app you connect.
            </p>
            <KeyField
              id={`${uid}-key`}
              app={{ slug: r.app.slug, name: r.app.name }}
              replace={false}
              keyUrl={keyUrl}
              phone={phone}
              phase={keyPhase}
              inputRef={keyRef}
              onSubmit={submitKey}
              onCancel={() => (setKeyOpen(null), inputRef.current?.focus())}
            />
          </div>
        )}
      </li>
    );
  };

  const groupHeader = (g: Group) => (
    <div className={`flex ${phone ? 'flex-col gap-0.5' : 'items-baseline justify-between gap-2'} px-0.5 mb-1.5`}>
      <h4 id={`${uid}-g-${g.key}`} className="text-[11px] font-semibold uppercase tracking-wider text-zinc-300">
        {GROUP_LABEL[g.key]}
      </h4>
      {g.key === 'composio' && (
        <span className="inline-flex items-center gap-1 text-xs text-zinc-400">
          <Lock size={12} aria-hidden="true" /> Sign-ins stay on Composio’s servers
        </span>
      )}
      {g.key === 'directory' && <span className="text-xs text-zinc-400">Listed by their makers, not checked by kwant</span>}
    </div>
  );

  const composioState = (g: Group): React.ReactNode => {
    if (apps.s === 'refused')
      return (
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
      );
    if (apps.s === 'down' && composioAdded)
      return (
        <p className="flex flex-wrap items-center gap-x-2 text-xs text-zinc-300" data-composio-state="down">
          Couldn’t reach Composio, so its apps aren’t listed.
          <button type="button" className={linkBtn} onClick={() => (setApps({ s: 'loading' }), void loadApps())}>
            <RefreshCw size={12} aria-hidden="true" /> Try again
          </button>
        </p>
      );
    const hasRows = g.rows.length > 0;
    return (
      <>
        {apps.s === 'loading' && composioAdded && (
          <p className="flex items-center gap-2 text-xs text-zinc-300 py-1" data-composio-state="loading">
            <Spinner /> Loading Composio’s apps…
          </p>
        )}
        {!hasRows && normalize(query).length >= 2 && (
          <p className="text-xs text-zinc-300 py-1" data-not-listed>
            Not listed? Bots with Composio find its other apps themselves.
          </p>
        )}
      </>
    );
  };

  const directoryState = (g: Group): React.ReactNode => {
    if (dir?.unavailable && !g.rows.length)
      return (
        <p className="flex flex-wrap items-center gap-x-2 text-xs text-zinc-300" data-directory-state="down">
          Couldn’t reach the public directory. The tools above still work.
          <button type="button" className={linkBtn} onClick={() => setDirTry((n) => n + 1)}>
            <RefreshCw size={12} aria-hidden="true" /> Try again
          </button>
        </p>
      );
    if ((!dir || dir.loading) && !g.rows.length)
      return (
        <p className="flex items-center gap-2 text-xs text-zinc-300 py-1" data-directory-state="loading">
          <Spinner /> Searching the public directory…
        </p>
      );
    return null;
  };

  // A group with no rows and nothing to say is hidden.
  const shown = (g: Group): boolean => {
    if (g.rows.length) return true;
    if (g.key === 'composio') return apps.s === 'refused' || (apps.s === 'down' && composioAdded) || (apps.s === 'loading' && composioAdded) || normalize(query).length >= 2;
    if (g.key === 'directory') return !!dir?.unavailable || !dir || dir.loading;
    return false;
  };
  const visibleGroups = typed || showEmptyGroups ? groups.filter(shown) : [];
  const nothing = typed && optionCount === 0 && !(apps.s === 'refused') && !(dir?.loading || (!dir && q2 && !dirMissing)) && !dir?.unavailable;

  const expandedOpen = visibleGroups.length > 0;
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
          aria-label="Find a tool"
          aria-autocomplete="both"
          aria-expanded={expandedOpen}
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
          placeholder="Find a tool, like gmail, stripe or a database"
          onChange={(e) => change(e.target.value)}
          onKeyDown={onKeyDown}
          onSelect={(e) => {
            const t = e.currentTarget;
            setCaretAtEnd(t.selectionStart === t.value.length && t.selectionEnd === t.value.length);
          }}
          className={`relative w-full ${phone ? 'h-11 text-base' : 'h-10 text-sm'} pl-[38px] pr-20 rounded-[10px] bg-transparent border border-zinc-700 text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-400 focus:ring-[3px] focus:ring-sky-400/25`}
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
              className={`inline-flex items-center justify-center ${phone ? 'w-11 h-11' : 'w-7 h-7'} rounded-full focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400`}
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
        {visibleGroups.map((g) => (
          <section key={g.key} className={showEmptyGroups ? 'motion-safe:animate-[findtool-in_120ms_ease-out]' : g.key === 'directory' ? fade : ''} data-group={g.key}>
            {groupHeader(g)}
            {g.key === 'composio' && composioState(g)}
            {g.rows.length > 0 && (
              <ul
                role="listbox"
                aria-labelledby={`${uid}-g-${g.key}`}
                className="rounded-xl border border-zinc-800 bg-[#181335] divide-y divide-zinc-800 overflow-hidden"
              >
                {g.rows.map(renderRow)}
                {g.rest ? (
                  <li role="presentation" className="px-3.5 py-2.5 pl-[60px] text-xs text-zinc-400" data-connected-rest>
                    {g.rest} more. Type to find them.
                  </li>
                ) : null}
              </ul>
            )}
            {g.key === 'directory' && directoryState(g)}
          </section>
        ))}
        {nothing && (
          <div className="flex flex-col items-center gap-1.5 px-4 py-5 rounded-xl border border-zinc-800 bg-[#181335] text-center" data-find-nothing>
            <p className="text-sm text-zinc-100">Nothing found for “{query.trim()}”.</p>
            <p className="text-xs text-zinc-300">Check the spelling, or add a tool by its address.</p>
            {p.onAddByAddress && (
              <button type="button" className={`${quietBtn} mt-1.5`} onClick={p.onAddByAddress} data-add-by-address-inline>
                <Plus size={14} aria-hidden="true" /> Add by address
              </button>
            )}
          </div>
        )}
        {typed && !phone && optionCount > 0 && (
          <p className="flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500" aria-hidden="true" data-find-legend>
            <span className={keycap}>↑</span>
            <span className={keycap}>↓</span> move ·<span className={keycap}>↵</span> runs the button ·<span className={keycap}>Tab</span> fills in the name ·
            <span className={keycap}>Esc</span> clears
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
