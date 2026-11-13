import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  Plug,
  Terminal,
  Globe,
  Plus,
  Trash2,
  Pencil,
  Loader2,
  CheckCircle2,
  AlertCircle,
  X,
  ShieldAlert,
  Eye,
  EyeOff,
  ChevronLeft,
  Wrench,
  CircleDashed,
  PauseCircle,
  Zap,
  KeyRound,
  ShieldCheck,
  LogOut,
  Users,
  ExternalLink,
} from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import {
  kwantUsage,
  McpServer,
  McpFormState,
  McpTestResult,
  McpTransport,
  SecretRow,
  StoredStatus,
  McpPreset,
  OpenAddTool,
  MCP_PRESETS,
  groupPresets,
  emptyMcpForm,
  formToPayload,
  serverToForm,
  serverTarget,
  splitCommandLine,
  validateMcpForm,
  loadStatuses,
  saveStatuses,
  toStoredStatus,
  humanizeToolName,
  serverStatusKind,
  isSignedIn,
  readOAuthMessage,
  isBlankRequired,
} from '../mcp';
import { inDesktopApp, leaveForSignIn, pollUntil, signInChanged } from '../externalLinks';
import { useEscapeLayer } from '../escape';
import { ChooseBotsTarget, FindTool, FindToolApi } from './FindTool';
import { DirectoryEntry, connectionsApi } from '../connections';

interface McpServersModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** For "Off for: …" on each card. */
  kwants?: Array<{ id: string; name: string }>;
  /** Opens the Connections page, where Composio is added (for apps that go through it). */
  onOpenConnections?: () => void;
  /** Opens straight on "Add a tool server" instead of the list. */
  startAdding?: boolean;
  /** Tests and shots: Find a tool's data, and text already typed. */
  findApi?: FindToolApi;
  initialQuery?: string;
  /** Opened from Apps & accounts' Find a tool: straight on this tool's form. */
  startWith?: OpenAddTool | null;
}

const inputCls =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';

const TRANSPORTS: Array<{ id: McpTransport; title: string; blurb: string; icon: React.ReactNode }> = [
  { id: 'stdio', title: 'Runs on this machine', blurb: 'The server starts a command, like npx or uvx', icon: <Terminal size={18} /> },
  { id: 'http', title: 'Connects to a URL', blurb: 'A server already running somewhere else', icon: <Globe size={18} /> },
];

const readError = async (res: Response) => {
  try {
    const body = await res.json();
    if (body?.error) return String(body.error);
  } catch {
    // not JSON
  }
  return `Something went wrong (HTTP ${res.status}).`;
};

const StatusPill: React.FC<{ server: McpServer; status?: StoredStatus; testing: boolean }> = ({ server, status, testing }) => {
  const kind = serverStatusKind(server, status, testing);
  let cls = 'bg-zinc-800/60 text-zinc-400 border-zinc-700/60';
  let icon = <CircleDashed size={11} />;
  let label = 'Not tested yet';
  if (kind === 'testing') {
    cls = 'bg-sky-500/10 text-sky-300 border-sky-500/20';
    icon = <Loader2 size={11} className="animate-spin" />;
    label = 'Testing…';
  } else if (kind === 'paused') {
    icon = <PauseCircle size={11} />;
    label = 'Paused';
  } else if (kind === 'ok' && status) {
    cls = 'bg-emerald-500/10 text-emerald-300 border-emerald-500/20';
    icon = <CheckCircle2 size={11} />;
    label = `Working · ${status.tools.length} ${status.tools.length === 1 ? 'tool' : 'tools'}`;
  } else if (kind === 'needs_sign_in') {
    cls = 'bg-amber-500/10 text-amber-300 border-amber-500/25';
    icon = <KeyRound size={11} />;
    label = 'Needs sign-in';
  } else if (kind === 'error') {
    cls = 'bg-rose-500/10 text-rose-300 border-rose-500/20';
    icon = <AlertCircle size={11} />;
    label = 'Couldn’t connect';
  }
  return (
    <span className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full border text-[10px] font-medium whitespace-nowrap ${cls}`}>
      {icon}
      {label}
    </span>
  );
};

const SecretRows: React.FC<{
  rows: SecretRow[];
  onChange: (rows: SecretRow[]) => void;
  keyPlaceholder: string;
  keyLabel: string;
  addLabel: string;
  /** Prefix for the ids that tie a row's warning to its value field. */
  idPrefix: string;
}> = ({ rows, onChange, keyPlaceholder, keyLabel, addLabel, idPrefix }) => {
  const [shown, setShown] = useState<Record<number, boolean>>({});
  const update = (i: number, patch: Partial<SecretRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  return (
    <div className="space-y-1.5">
      {rows.map((row, i) => {
        // A preset's key that is still empty: say so, but saving stays allowed.
        const missing = isBlankRequired(row);
        const warnId = `${idPrefix}-missing-${i}`;
        return (
          <div key={i} className="space-y-1">
            <div className="flex flex-col sm:flex-row gap-1.5">
              <input
                aria-label={keyLabel}
                value={row.key}
                disabled={row.saved}
                onChange={(e) => update(i, { key: e.target.value })}
                placeholder={keyPlaceholder}
                className={`${inputCls} sm:w-2/5 font-mono disabled:text-zinc-400 disabled:bg-zinc-950/60`}
              />
              <div className="flex gap-1.5 flex-1 min-w-0">
                <div className="relative flex-1 min-w-0">
                  <input
                    aria-label={`Value for ${row.key || 'this entry'}`}
                    aria-describedby={missing ? warnId : undefined}
                    type={shown[i] ? 'text' : 'password'}
                    autoComplete="off"
                    value={row.value}
                    onChange={(e) => update(i, { value: e.target.value })}
                    placeholder={row.saved ? '•••••••• saved, leave blank to keep' : row.hint || 'Secret value'}
                    className={`${inputCls} pr-8 font-mono ${missing ? 'border-amber-500/40' : ''}`}
                  />
                  <button
                    type="button"
                    onClick={() => setShown((s) => ({ ...s, [i]: !s[i] }))}
                    aria-label={shown[i] ? 'Hide value' : 'Show value'}
                    className="absolute right-1.5 top-1/2 -translate-y-1/2 p-1 rounded text-zinc-500 hover:text-zinc-200"
                  >
                    {shown[i] ? <EyeOff size={13} /> : <Eye size={13} />}
                  </button>
                </div>
                <button
                  type="button"
                  onClick={() => onChange(rows.filter((_, j) => j !== i))}
                  aria-label={`Remove ${row.key || 'entry'}`}
                  className="p-2 rounded-lg text-zinc-500 hover:text-rose-300 hover:bg-rose-500/10 transition-colors"
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </div>
            {missing && (
              <p id={warnId} className="flex items-start gap-1.5 text-[11px] text-amber-300/90">
                <KeyRound size={12} className="flex-shrink-0 mt-px text-amber-400" aria-hidden="true" />
                <span>This server needs a key to work. Add it now, or save and edit later.</span>
              </p>
            )}
          </div>
        );
      })}
      <button
        type="button"
        onClick={() => onChange([...rows, { key: '', value: '', saved: false }])}
        className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium text-sky-300 hover:bg-sky-500/10 transition-colors"
      >
        <Plus size={12} />
        {addLabel}
      </button>
    </div>
  );
};

const ToolChips: React.FC<{ tools: Array<{ name: string; description?: string }>; max?: number }> = ({ tools, max = 8 }) => {
  if (tools.length === 0) return <p className="text-[11px] text-zinc-500">Connected, but it didn’t offer any tools.</p>;
  const extra = tools.length - max;
  return (
    <div className="flex flex-wrap gap-1">
      {tools.slice(0, max).map((t) => (
        <span
          key={t.name}
          title={t.description || t.name}
          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-zinc-950/70 border border-zinc-800 text-[11px] text-zinc-300"
        >
          <Wrench size={10} className="text-sky-400/80" />
          {humanizeToolName(t.name)}
        </span>
      ))}
      {extra > 0 && <span className="px-2 py-0.5 text-[11px] text-zinc-500">+{extra} more</span>}
    </div>
  );
};

/** A Composio-only app picked in the catalog: why it goes through Composio, and where to set it up. */
/** The built-in tools by category, under "All built-in tools" while Find a tool is empty. */
export const BuiltInGrid: React.FC<{ onPick: (preset: McpPreset) => void }> = ({ onPick }) => {
  const groups = groupPresets(MCP_PRESETS, []);
  return (
    <div className="space-y-4 text-left" data-builtin-grid>
      {groups.map((g) => (
        <section key={g.id} aria-labelledby={`mcp-cat-${g.id}`}>
          <h5 id={`mcp-cat-${g.id}`} className="mb-1.5 text-[11px] font-medium text-zinc-400">
            {g.label}
          </h5>
          <ul className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {g.presets.map((p) => (
              <li key={p.id}>
                <button
                  type="button"
                  onClick={() => onPick(p)}
                  data-preset={p.id}
                  className="group w-full h-full p-3 rounded-xl border border-zinc-800 bg-zinc-950/60 hover:border-sky-500/40 hover:bg-sky-500/5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 text-left transition-colors motion-reduce:transition-none"
                >
                  <span className="flex items-center gap-1.5 text-xs font-medium text-zinc-200 group-hover:text-sky-200">
                    <span className="text-sky-400" aria-hidden="true">
                      {p.form.transport === 'stdio' ? <Terminal size={13} /> : <Globe size={13} />}
                    </span>
                    {p.label}
                  </span>
                  <span className="mt-1 block text-[11px] text-zinc-400 leading-snug">{p.blurb}</span>
                  <span className="mt-2 flex flex-wrap gap-1">
                    <span className="px-1.5 py-px rounded border text-[10px] border-zinc-700/70 text-zinc-400">{p.form.transport === 'http' ? 'Remote' : 'Runs on this computer'}</span>
                    {p.needs.includes('Sign-in') && <span className="px-1.5 py-px rounded border text-[10px] border-amber-500/25 text-amber-300/90">Sign in after adding</span>}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
};

/** Add a tool: Find a tool, the built-in grid while it's empty, and Add by address. */
export const AddToolPicker: React.FC<{
  onPick: (preset: McpPreset) => void;
  onCustom: () => void;
  onDirectory: (entry: DirectoryEntry) => void;
  onChooseBots: (target: ChooseBotsTarget) => void;
  onOpenConnections?: () => void;
  autoFocus?: boolean;
  findApi?: FindToolApi;
  initialQuery?: string;
}> = ({ onPick, onCustom, onDirectory, onChooseBots, onOpenConnections, autoFocus, findApi, initialQuery }) => (
  <div className="space-y-4 text-left" data-add-tool>
    <FindTool
      browseWhenEmpty
      autoFocus={autoFocus}
      api={findApi}
      initialQuery={initialQuery}
      onAddPreset={onPick}
      onAddDirectory={onDirectory}
      onAddByAddress={onCustom}
      onChooseBots={onChooseBots}
      onComposioElsewhere={() => onOpenConnections?.()}
    >
      <BuiltInGrid onPick={onPick} />
    </FindTool>
    <button
      type="button"
      onClick={onCustom}
      data-add-by-address
      className="w-full flex items-center gap-2.5 p-3 rounded-xl border border-dashed border-zinc-700 hover:border-sky-500/40 hover:bg-sky-500/5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 text-left transition-colors motion-reduce:transition-none"
    >
      <Plus size={16} className="flex-shrink-0 text-sky-400" aria-hidden="true" />
      <span>
        <span className="block text-xs font-medium text-zinc-200">Add by address</span>
        <span className="block text-[11px] text-zinc-400">Paste a command or web address from any tool’s instructions.</span>
      </span>
    </button>
  </div>
);

export const McpServersModal: React.FC<McpServersModalProps> = ({ isOpen, onClose, kwants = [], onOpenConnections, startAdding = false, findApi, initialQuery, startWith }) => {
  const [servers, setServers] = useState<McpServer[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<Record<string, StoredStatus>>({});
  const [testing, setTesting] = useState<Record<string, boolean>>({});
  const [testResults, setTestResults] = useState<Record<string, McpTestResult>>({});
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  // Form view: null = list, 'new' = adding, otherwise the id being edited.
  const [editing, setEditing] = useState<string | null>(null);
  const [form, setForm] = useState<McpFormState>(emptyMcpForm());
  const [formStart, setFormStart] = useState<McpFormState | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // Find a tool before the form when adding, and the preset the form came from.
  const [picking, setPicking] = useState(false);
  const [presetId, setPresetId] = useState<string | null>(null);
  // Added from the public directory: Save installs it by its directory name.
  const [fromDirectory, setFromDirectory] = useState<DirectoryEntry | null>(null);
  // Sign-in: the server whose popup is open, sign-in errors per server, and a pending disconnect.
  const [connecting, setConnecting] = useState<string | null>(null);
  const [oauthErrors, setOauthErrors] = useState<Record<string, string>>({});
  const [confirmDisconnect, setConfirmDisconnect] = useState<string | null>(null);
  const popupRef = useRef<Window | null>(null);
  const popupWatch = useRef<number | null>(null);
  /** Mac app: stops watching the server for a sign-in finished in the browser. */
  const signInWatch = useRef<(() => void) | null>(null);
  const macApp = inDesktopApp();

  const refresh = useCallback(async (): Promise<McpServer[] | null> => {
    try {
      const res = await fetch('/api/mcp/servers');
      if (!res.ok) throw new Error(await readError(res));
      const list: McpServer[] = await res.json();
      setServers(list);
      setLoadError(null);
      return list;
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : 'Could not reach the server.');
      setServers((prev) => prev ?? []);
      return null;
    }
  }, []);

  const stopWatchingPopup = useCallback(() => {
    if (popupWatch.current !== null) window.clearInterval(popupWatch.current);
    popupWatch.current = null;
    popupRef.current = null;
    signInWatch.current?.();
    signInWatch.current = null;
  }, []);

  const setStatus = useCallback((id: string, status: StoredStatus | null) => {
    setStatuses((prev) => {
      const next = { ...prev };
      if (status) next[id] = status;
      else delete next[id];
      saveStatuses(next);
      return next;
    });
  }, []);

  const runTest = useCallback(async (id: string) => {
    setTesting((t) => ({ ...t, [id]: true }));
    let result: McpTestResult;
    try {
      const res = await fetch(`/api/mcp/servers/${id}/test`, { method: 'POST' });
      result = res.ok ? await res.json() : { ok: false, error: await readError(res) };
    } catch {
      result = { ok: false, error: 'Could not reach the server.' };
    }
    setTesting((t) => ({ ...t, [id]: false }));
    setTestResults((r) => ({ ...r, [id]: result }));
    setStatus(id, toStoredStatus(result, new Date().toISOString()));
    playSfx(result.ok ? 'coin' : 'alert');
  }, [setStatus]);

  // Re-tests servers whose last test asked for sign-in but that kwnt now holds a sign-in for.
  const checkNewSignIns = useCallback(async () => {
    const stored = loadStatuses();
    if (!Object.values(stored).some((s) => s.auth_required)) return;
    const list = await refresh();
    list?.filter((s) => s.enabled && isSignedIn(s) && stored[s.id]?.auth_required).forEach((s) => runTest(s.id));
  }, [refresh, runTest]);

  useEffect(() => {
    if (!isOpen) return;
    setEditing(null);
    setPicking(startAdding);
    setConfirmDelete(null);
    setConfirmDisconnect(null);
    setTestResults({});
    setOauthErrors({});
    setStatuses(loadStatuses());
    // Also catches a sign-in finished elsewhere, like the same-tab fallback when popups are blocked.
    refresh().then(() => checkNewSignIns());
  }, [isOpen, startAdding, refresh, checkNewSignIns]);

  useEffect(() => {
    if (!isOpen || !startWith) return;
    const preset = startWith.preset ? MCP_PRESETS.find((p) => p.id === startWith.preset) : undefined;
    if (preset) openForm('new', preset);
    else if (startWith.directory) openDirectoryForm(startWith.directory);
    else if (startWith.address) openForm('new');
  }, [isOpen, startWith]); // eslint-disable-line react-hooks/exhaustive-deps

  // If a provider cuts the popup's link back to this window, no message arrives: coming back to
  // this window is the cue to look again.
  useEffect(() => {
    if (!isOpen) return;
    window.addEventListener('focus', checkNewSignIns);
    return () => window.removeEventListener('focus', checkNewSignIns);
  }, [isOpen, checkNewSignIns]);

  // The sign-in popup reports back with a postMessage from kwnt's callback page.
  useEffect(() => {
    if (!isOpen) return;
    const onMessage = (event: MessageEvent) => {
      const msg = readOAuthMessage(event, window.location.origin);
      if (!msg) return;
      const id = msg.server_id ?? connecting;
      stopWatchingPopup();
      setConnecting(null);
      if (!id) return;
      if (msg.ok) {
        setOauthErrors(({ [id]: _, ...rest }) => rest);
        refresh();
        runTest(id);
      } else {
        setOauthErrors((e) => ({ ...e, [id]: msg.error || 'Sign-in didn’t finish. Try again.' }));
        playSfx('alert');
      }
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [isOpen, connecting, refresh, runTest, stopWatchingPopup]);

  useEffect(() => stopWatchingPopup, [stopWatchingPopup]);

  // Escape steps back (form or catalog to the list), then closes; it keeps a form with changes.
  const rootRef = useRef<HTMLDivElement>(null);
  const formDirty = editing !== null && JSON.stringify(form) !== JSON.stringify(formStart);
  useEscapeLayer(rootRef, isOpen, () => (editing ? setEditing(null) : picking ? setPicking(false) : onClose()), { modal: true, unsaved: formDirty });

  if (!isOpen) return null;

  const connect = async (server: McpServer) => {
    playSfx('click');
    setOauthErrors(({ [server.id]: _, ...rest }) => rest);
    const before = server.oauth;
    // Open the window inside the click so popup blockers allow it; the address comes after the fetch.
    // The Mac app never opens a second window: its sign-ins go to the default browser.
    const popup = macApp ? null : window.open('', 'kwnt-mcp-oauth', 'popup,width=520,height=720');
    if (popup) {
      try {
        popup.document.title = 'Signing in…';
        popup.document.body.style.cssText = 'background:#18181b;color:#a1a1aa;font:14px system-ui;display:grid;place-items:center;height:100vh;margin:0';
        popup.document.body.textContent = `Opening sign-in for ${server.name}…`;
      } catch {
        // not ours to style; harmless
      }
    }
    setConnecting(server.id);
    let url: string;
    try {
      const res = await fetch(`/api/mcp/servers/${server.id}/oauth/start`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ origin: window.location.origin }),
      });
      if (!res.ok) throw new Error(await readError(res));
      const body = await res.json();
      if (typeof body?.authorize_url !== 'string' || !/^https?:\/\//.test(body.authorize_url)) throw new Error('The server didn’t return a sign-in page.');
      url = body.authorize_url;
    } catch (err) {
      popup?.close();
      setConnecting(null);
      setOauthErrors((e) => ({ ...e, [server.id]: err instanceof Error ? err.message : 'Could not reach the server.' }));
      playSfx('alert');
      return;
    }
    if (!popup || popup.closed) {
      // Blocked anyway: sign in in this tab. kwnt brings you back to the app afterwards.
      leaveForSignIn(url);
      if (macApp) {
        // The browser can't report back to the app: watch the server for the new sign-in.
        stopWatchingPopup();
        const stopWaiting = () => {
          signInWatch.current = null;
          setConnecting((c) => (c === server.id ? null : c));
        };
        signInWatch.current = pollUntil(
          async () => {
            const list = await refresh();
            return signInChanged(before, list?.find((s) => s.id === server.id)?.oauth);
          },
          () => {
            stopWaiting();
            setOauthErrors(({ [server.id]: _, ...rest }) => rest);
            runTest(server.id);
          },
          { onTimeout: stopWaiting },
        );
      }
      return;
    }
    popup.location.href = url;
    popup.focus();
    stopWatchingPopup();
    popupRef.current = popup;
    // Closing the window without finishing just stops waiting.
    popupWatch.current = window.setInterval(() => {
      if (popupRef.current?.closed) {
        stopWatchingPopup();
        setConnecting((c) => (c === server.id ? null : c));
        checkNewSignIns();
      }
    }, 500);
  };

  const disconnect = async (server: McpServer) => {
    playSfx('click');
    setConfirmDisconnect(null);
    const res = await fetch(`/api/mcp/servers/${server.id}/oauth`, { method: 'DELETE' }).catch(() => null);
    if (!res || !res.ok) {
      setLoadError(res ? await readError(res) : 'Could not reach the server.');
      return;
    }
    setTestResults(({ [server.id]: _, ...rest }) => rest);
    setStatus(server.id, null);
    await refresh();
    // Shows "Needs sign-in" again if the server requires it.
    if (server.enabled) runTest(server.id);
  };

  const openCatalog = () => {
    playSfx('click');
    setEditing(null);
    setPicking(true);
  };

  const openForm = (target: 'new' | McpServer, preset?: McpPreset) => {
    playSfx('click');
    setFormError(null);
    setPicking(false);
    setPresetId(preset?.id ?? null);
    setFromDirectory(null);
    const f = preset?.form;
    const start = target !== 'new' ? serverToForm(target) : f ? { ...f, env: f.env.map((r) => ({ ...r })), headers: f.headers.map((r) => ({ ...r })) } : emptyMcpForm();
    setEditing(target === 'new' ? 'new' : target.id);
    setForm(start);
    setFormStart(start);
  };

  /** A tool from the public directory: the form filled in from its listing, its keys still to add. */
  const openDirectoryForm = (entry: DirectoryEntry) => {
    playSfx('click');
    setFormError(null);
    setPicking(false);
    setPresetId(null);
    const remote = entry.kind === 'remote';
    const rows: SecretRow[] = (entry.inputs ?? []).map((i) => ({ key: i.name, value: '', saved: false, hint: i.description, required: !!i.required }));
    const start: McpFormState = {
      ...emptyMcpForm(remote ? 'http' : 'stdio'),
      name: (entry.title || entry.registry_name.split('/').pop() || entry.registry_name).trim(),
      url: remote ? entry.url ?? '' : '',
      commandLine: remote ? '' : entry.command ?? '',
      env: remote ? [] : rows,
      headers: remote ? rows : [],
    };
    setFromDirectory(entry);
    setEditing('new');
    setForm(start);
    setFormStart(start);
  };

  /** "Choose bots" from Find a tool: a tool server's card shows who has it; Composio's bots are on Apps & accounts. */
  const chooseBots = (target: ChooseBotsTarget) => {
    if (target.kind === 'composio') return onOpenConnections?.();
    setPicking(false);
    setEditing(null);
    window.setTimeout(() => document.querySelector<HTMLElement>(`[data-server-card="${CSS.escape(target.connectionId.replace(/^mcp:/, ''))}"]`)?.scrollIntoView({ block: 'nearest' }), 50);
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    const problem = validateMcpForm(form);
    if (problem) {
      setFormError(problem);
      return;
    }
    setSaving(true);
    setFormError(null);
    if (editing === 'new' && fromDirectory) {
      const pairs = (rows: SecretRow[]) => Object.fromEntries(rows.filter((r) => r.key.trim()).map((r) => [r.key.trim(), r.value]));
      try {
        const r = await connectionsApi.installFromDirectory(fromDirectory.registry_name, pairs(form.env), pairs(form.headers));
        setEditing(null);
        setFromDirectory(null);
        await refresh();
        if (r?.server?.id) {
          setStatus(r.server.id, null);
          runTest(r.server.id);
        }
      } catch (err) {
        setFormError(err instanceof Error ? err.message : 'Could not reach the server.');
      } finally {
        setSaving(false);
      }
      return;
    }
    try {
      const isNew = editing === 'new';
      const res = await fetch(isNew ? '/api/mcp/servers' : `/api/mcp/servers/${editing}`, {
        method: isNew ? 'POST' : 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(formToPayload(form)),
      });
      if (res.status === 409) {
        setFormError(`A server named "${form.name.trim()}" already exists.`);
        return;
      }
      if (!res.ok) {
        setFormError(await readError(res));
        return;
      }
      const saved: McpServer = await res.json();
      setEditing(null);
      await refresh();
      // Config changed, so the old status no longer says anything: test right away.
      setStatus(saved.id, null);
      runTest(saved.id);
    } catch {
      setFormError('Could not reach the server.');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (server: McpServer) => {
    playSfx('click');
    const res = await fetch(`/api/mcp/servers/${server.id}`, { method: 'DELETE' }).catch(() => null);
    setConfirmDelete(null);
    if (!res || !res.ok) {
      setLoadError(res ? await readError(res) : 'Could not reach the server.');
      return;
    }
    setStatus(server.id, null);
    refresh();
  };

  const toggleEnabled = async (server: McpServer) => {
    playSfx('click');
    // Sending the masks back keeps every stored secret.
    const res = await fetch(`/api/mcp/servers/${server.id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...formToPayload(serverToForm(server)), enabled: !server.enabled }),
    }).catch(() => null);
    if (!res || !res.ok) setLoadError(res ? await readError(res) : 'Could not reach the server.');
    refresh();
  };

  const close = () => {
    playSfx('click');
    onClose();
  };

  const parsed = splitCommandLine(form.commandLine);
  const preset = editing === 'new' && presetId ? MCP_PRESETS.find((p) => p.id === presetId) : undefined;

  const picker = (autoFocus: boolean) => (
    <AddToolPicker
      onPick={(p) => openForm('new', p)}
      onCustom={() => openForm('new')}
      onDirectory={openDirectoryForm}
      onChooseBots={chooseBots}
      onOpenConnections={onOpenConnections}
      autoFocus={autoFocus}
      findApi={findApi}
      initialQuery={initialQuery}
    />
  );
  const renderPicker = () => picker(true);

  const renderEmpty = () => (
    <div className="text-center py-6 px-2 space-y-5">
      <div className="mx-auto w-14 h-14 rounded-2xl bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-300">
        <Plug size={26} />
      </div>
      <div className="space-y-1.5 max-w-md mx-auto">
        <h3 className="text-sm font-semibold text-zinc-100">Give your bots new abilities</h3>
        <p className="text-xs text-zinc-400 leading-relaxed">
          Tools give your bots new abilities, like reading mail, browsing the web or working with GitHub.
          Add one here and every bot in this space can use it; switch it off for a bot in that bot’s edit screen. They always ask you before using a tool.
        </p>
      </div>
      {picker(false)}
    </div>
  );

  const renderCard = (server: McpServer) => {
    const status = statuses[server.id];
    const result = testResults[server.id];
    const isTesting = !!testing[server.id];
    const secretCount = Object.keys(server.env).length + Object.keys(server.headers).length;
    const tools = result?.ok ? result.tools ?? [] : status?.ok ? status.tools.map((name) => ({ name })) : [];
    const kind = serverStatusKind(server, status, isTesting);
    const signedIn = isSignedIn(server);
    const isConnecting = connecting === server.id;
    const oauthError = oauthErrors[server.id];
    const usage = kwantUsage(server, kwants);
    return (
      <li
        key={server.id}
        data-server-card={server.id}
        className={`p-3.5 rounded-xl border bg-zinc-950/50 transition-colors ${server.enabled ? 'border-zinc-800' : 'border-zinc-800/60 opacity-75'}`}
      >
        <div className="flex items-start gap-3">
          <div className="w-9 h-9 flex-shrink-0 rounded-lg bg-zinc-900 border border-zinc-800 flex items-center justify-center text-sky-300">
            {server.transport === 'stdio' ? <Terminal size={16} /> : <Globe size={16} />}
          </div>
          <div className="flex-1 min-w-0 space-y-1">
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span className="text-sm font-semibold text-zinc-100 truncate">{server.name}</span>
              <StatusPill server={server} status={status} testing={isTesting} />
            </div>
            <p className="text-[11px] text-zinc-500">
              {server.transport === 'stdio' ? 'Runs a command on this machine' : 'Connects to a URL'}
              {secretCount > 0 && <span> · {secretCount} hidden {secretCount === 1 ? 'secret' : 'secrets'}</span>}
              {signedIn && (
                <span className="text-emerald-300/80">
                  {' · '}
                  <ShieldCheck size={11} className="inline -mt-px mr-0.5" aria-hidden="true" />
                  Signed in
                </span>
              )}
            </p>
            <p className="text-[11px] font-mono text-zinc-400 truncate" title={serverTarget(server)}>
              {serverTarget(server)}
            </p>
            {kwants.length > 0 && (
              <p className="flex items-start gap-1 text-[11px] text-zinc-400">
                <Users size={11} className="flex-shrink-0 mt-[3px] text-zinc-500" aria-hidden="true" />
                <span className="min-w-0 break-words">
                  {usage.allKwants ? (
                    server.enabled ? 'Used by every bot in this space' : 'Every bot here gets it once it’s on'
                  ) : (
                    <>
                      {server.enabled ? 'Off for: ' : 'Once it’s on, off for: '}
                      <span className="text-zinc-300">{usage.offFor.join(', ')}</span>
                    </>
                  )}
                </span>
              </p>
            )}
          </div>
        </div>

        {tools.length > 0 && (
          <div className="mt-2.5 pl-12">
            <ToolChips tools={tools} />
          </div>
        )}
        {result?.ok && tools.length === 0 && (
          <div className="mt-2.5 pl-12">
            <ToolChips tools={[]} />
          </div>
        )}
        {(kind === 'needs_sign_in' || oauthError) && server.transport === 'http' && server.enabled && (
          <div className="mt-2.5 ml-12 p-2.5 rounded-lg bg-amber-500/5 border border-amber-500/20 text-[11px] space-y-2">
            <div className="flex items-start gap-2 text-amber-100/90">
              <KeyRound size={13} className="flex-shrink-0 mt-px text-amber-400" />
              <span>
                {isConnecting
                  ? macApp
                    ? 'Finish signing in in your browser. This updates by itself when you’re done.'
                    : 'Waiting for you to approve access in the sign-in window…'
                  : 'This server asks you to sign in and approve access.'}
              </span>
            </div>
            {oauthError && !isConnecting && (
              <p role="alert" className="flex items-start gap-1.5 text-rose-300">
                <AlertCircle size={12} className="flex-shrink-0 mt-px" />
                <span>{oauthError}</span>
              </p>
            )}
            <button
              type="button"
              onClick={() => connect(server)}
              disabled={isConnecting && !macApp}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-60"
            >
              {isConnecting && !macApp ? <Loader2 size={12} className="animate-spin" /> : <KeyRound size={12} />}
              {isConnecting ? (macApp ? 'Open the sign-in again' : 'Waiting…') : oauthError ? 'Retry' : 'Connect'}
            </button>
          </div>
        )}
        {result && !result.ok && kind !== 'needs_sign_in' && (
          <div className="mt-2.5 ml-12 p-2.5 rounded-lg bg-rose-500/5 border border-rose-500/20 text-[11px] text-rose-200 space-y-1">
            <p>{result.error || 'The server didn’t answer.'}</p>
            {result.stderr_tail && (
              <details>
                <summary className="cursor-pointer text-rose-300/80">What the server printed</summary>
                <pre className="mt-1 whitespace-pre-wrap break-all font-mono text-[10px] text-zinc-400 max-h-32 overflow-y-auto">{result.stderr_tail}</pre>
              </details>
            )}
          </div>
        )}

        <div className="mt-3 pt-2.5 border-t border-zinc-800/70 flex flex-wrap items-center justify-between gap-2">
          {confirmDisconnect === server.id ? (
            <div className="flex flex-wrap items-center gap-2 text-[11px] text-zinc-300">
              <span>Sign out of {server.name}? Bots can’t use its tools until you connect again.</span>
              <button
                type="button"
                onClick={() => setConfirmDisconnect(null)}
                className="px-2.5 py-1 rounded-md font-medium text-zinc-300 hover:bg-zinc-800 transition-colors"
              >
                Stay signed in
              </button>
              <button
                type="button"
                onClick={() => disconnect(server)}
                className="px-2.5 py-1 rounded-md font-medium text-white bg-rose-600 hover:bg-rose-500 transition-colors"
              >
                Disconnect
              </button>
            </div>
          ) : confirmDelete === server.id ? (
            <div className="flex flex-wrap items-center gap-2 text-[11px] text-zinc-300">
              <span>Delete {server.name}? Bots using it lose these tools.</span>
              <button
                type="button"
                onClick={() => setConfirmDelete(null)}
                className="px-2.5 py-1 rounded-md font-medium text-zinc-300 hover:bg-zinc-800 transition-colors"
              >
                Keep it
              </button>
              <button
                type="button"
                onClick={() => handleDelete(server)}
                className="px-2.5 py-1 rounded-md font-medium text-white bg-rose-600 hover:bg-rose-500 transition-colors"
              >
                Delete
              </button>
            </div>
          ) : (
            <>
              <label className="flex items-center gap-2 text-[11px] text-zinc-400 cursor-pointer select-none">
                <button
                  type="button"
                  role="switch"
                  aria-checked={server.enabled}
                  aria-label={`${server.enabled ? 'Pause' : 'Turn on'} ${server.name}`}
                  onClick={() => toggleEnabled(server)}
                  className={`relative w-8 h-[18px] rounded-full transition-colors ${server.enabled ? 'bg-sky-600' : 'bg-zinc-700'}`}
                >
                  <span className={`absolute top-0.5 w-3.5 h-3.5 rounded-full bg-white shadow transition-all ${server.enabled ? 'left-[16px]' : 'left-0.5'}`} />
                </button>
                {server.enabled ? 'On' : 'Paused'}
              </label>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => runTest(server.id)}
                  disabled={isTesting}
                  className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-[11px] font-medium text-sky-200 bg-sky-500/10 hover:bg-sky-500/20 border border-sky-500/20 transition-colors disabled:opacity-60"
                >
                  {isTesting ? <Loader2 size={12} className="animate-spin" /> : <Zap size={12} />}
                  Test connection
                </button>
                {signedIn && (
                  <button
                    type="button"
                    onClick={() => {
                      playSfx('click');
                      setConfirmDelete(null);
                      setConfirmDisconnect(server.id);
                    }}
                    aria-label={`Disconnect ${server.name}`}
                    title="Disconnect (sign out)"
                    className="p-1.5 rounded-md text-zinc-400 hover:text-rose-300 hover:bg-rose-500/10 transition-colors"
                  >
                    <LogOut size={13} />
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => openForm(server)}
                  aria-label={`Edit ${server.name}`}
                  className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
                >
                  <Pencil size={13} />
                </button>
                <button
                  type="button"
                  onClick={() => {
                    playSfx('click');
                    setConfirmDisconnect(null);
                    setConfirmDelete(server.id);
                  }}
                  aria-label={`Delete ${server.name}`}
                  className="p-1.5 rounded-md text-zinc-400 hover:text-rose-300 hover:bg-rose-500/10 transition-colors"
                >
                  <Trash2 size={13} />
                </button>
              </div>
            </>
          )}
        </div>
      </li>
    );
  };

  const renderList = () => {
    if (servers === null) {
      return (
        <div className="py-12 flex justify-center text-zinc-500">
          <Loader2 size={20} className="animate-spin" aria-label="Loading" />
        </div>
      );
    }
    if (servers.length === 0) return renderEmpty();
    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-2">
          <p className="text-[11px] text-zinc-500">
            {servers.length} {servers.length === 1 ? 'server' : 'servers'}
          </p>
          <button
            type="button"
            onClick={openCatalog}
            className="flex-shrink-0 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm whitespace-nowrap"
          >
            <Plus size={14} />
            Add server
          </button>
        </div>
        <ul className="space-y-2.5">{servers.map(renderCard)}</ul>
      </div>
    );
  };

  const renderForm = () => (
    <form onSubmit={handleSave} className="space-y-5" noValidate>
      <fieldset>
        <legend className="text-xs font-medium text-zinc-300 mb-2">
          <span className="text-sky-400 mr-1.5">1</span>How does it connect?
        </legend>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {TRANSPORTS.map((t) => {
            const active = form.transport === t.id;
            return (
              <button
                key={t.id}
                type="button"
                aria-pressed={active}
                onClick={() => setForm((f) => ({ ...f, transport: t.id }))}
                className={`flex items-start gap-2.5 p-3 rounded-xl border text-left transition-all ${
                  active ? 'border-sky-500/60 bg-sky-500/10' : 'border-zinc-800 bg-zinc-950/60 hover:border-zinc-700'
                }`}
              >
                <span className={active ? 'text-sky-300' : 'text-zinc-500'}>{t.icon}</span>
                <span>
                  <span className={`block text-xs font-medium ${active ? 'text-sky-100' : 'text-zinc-200'}`}>{t.title}</span>
                  <span className="block text-[11px] text-zinc-500">{t.blurb}</span>
                </span>
              </button>
            );
          })}
        </div>
        {editing === 'new' && (
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
            {fromDirectory ? (
              <span className="text-zinc-400" data-from-directory>
                Filled in from the public directory, listed by {fromDirectory.publisher?.label || 'its maker'}. Add any keys it needs below.
              </span>
            ) : preset ? (
              <span className="text-zinc-400">
                Filled in for <span className="text-zinc-200">{preset.label}</span>.{' '}
                {preset.needs.includes('Sign-in')
                  ? 'Add it, then use Connect on its card to sign in.'
                  : preset.form.env.length + preset.form.headers.length > 0
                    ? 'Add your key below.'
                    : preset.form.transport === 'stdio'
                      ? 'Check the command, then add it.'
                      : 'Just add it.'}{' '}
                <a
                  href={preset.source}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-0.5 text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline"
                >
                  Setup guide
                  <ExternalLink size={10} aria-hidden="true" />
                  <span className="sr-only">(opens in a new tab)</span>
                </a>
              </span>
            ) : null}
            <button type="button" onClick={openCatalog} className="text-sky-300 hover:text-sky-200 hover:underline underline-offset-2">
              {preset || fromDirectory ? 'Pick a different one' : 'Or find a tool by name'}
            </button>
          </div>
        )}
      </fieldset>

      <fieldset className="space-y-3">
        <legend className="text-xs font-medium text-zinc-300 mb-2">
          <span className="text-sky-400 mr-1.5">2</span>Details
        </legend>
        <div>
          <label htmlFor="mcp-name" className="block text-xs font-medium text-zinc-400 mb-1">Name</label>
          <input
            id="mcp-name"
            value={form.name}
            onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
            placeholder="e.g. files, github"
            className={`${inputCls} font-mono`}
          />
          <p className="mt-1 text-[11px] text-zinc-500">
            Short and simple: letters, numbers, - or _. Bots see its tools as <span className="font-mono text-zinc-400">{form.name.trim() || 'name'} · tool</span>.
          </p>
        </div>

        {form.transport === 'stdio' ? (
          <>
            <div>
              <label htmlFor="mcp-command" className="block text-xs font-medium text-zinc-400 mb-1">Command</label>
              <input
                id="mcp-command"
                value={form.commandLine}
                onChange={(e) => setForm((f) => ({ ...f, commandLine: e.target.value }))}
                placeholder="npx -y @modelcontextprotocol/server-filesystem /home/me/projects"
                spellCheck={false}
                className={`${inputCls} font-mono`}
              />
              <p className="mt-1 text-[11px] text-zinc-500">
                {parsed.length > 0 ? (
                  <>
                    Runs <span className="font-mono text-zinc-300">{parsed[0]}</span>
                    {parsed.length > 1 ? ` with ${parsed.length - 1} ${parsed.length === 2 ? 'argument' : 'arguments'}` : ''}. Use quotes around paths with spaces.
                  </>
                ) : (
                  'Paste it the way the server’s README shows it.'
                )}
              </p>
            </div>
            <div className="flex items-start gap-2 p-2.5 rounded-lg bg-amber-500/5 border border-amber-500/20 text-[11px] text-amber-200/90">
              <ShieldAlert size={14} className="flex-shrink-0 mt-px text-amber-400" />
              <span>This command runs on the server’s machine with your user’s permissions. Only add servers you trust.</span>
            </div>
            <div>
              <p className="text-xs font-medium text-zinc-400 mb-1">
                Secrets & settings <span className="text-zinc-600 font-normal">· environment variables, always hidden</span>
              </p>
              <SecretRows
                rows={form.env}
                onChange={(env) => setForm((f) => ({ ...f, env }))}
                keyPlaceholder="API_TOKEN"
                keyLabel="Variable name"
                addLabel="Add variable"
                idPrefix="mcp-env"
              />
            </div>
          </>
        ) : (
          <>
            <div>
              <label htmlFor="mcp-url" className="block text-xs font-medium text-zinc-400 mb-1">Server URL</label>
              <input
                id="mcp-url"
                type="url"
                value={form.url}
                onChange={(e) => setForm((f) => ({ ...f, url: e.target.value }))}
                placeholder="https://example.com/mcp"
                spellCheck={false}
                className={`${inputCls} font-mono`}
              />
            </div>
            <div>
              <p className="text-xs font-medium text-zinc-400 mb-1">
                Headers <span className="text-zinc-600 font-normal">· e.g. Authorization, always hidden</span>
              </p>
              <SecretRows
                rows={form.headers}
                onChange={(headers) => setForm((f) => ({ ...f, headers }))}
                keyPlaceholder="Authorization"
                keyLabel="Header name"
                addLabel="Add header"
                idPrefix="mcp-header"
              />
              <p className="mt-1.5 text-[11px] text-zinc-500">
                Servers that need sign-in (like Notion) don’t need a header. Use <span className="text-zinc-300">Connect</span> on the server after saving.
              </p>
            </div>
          </>
        )}

        <details className="group">
          <summary className="cursor-pointer text-[11px] text-zinc-500 hover:text-zinc-300 select-none">More options</summary>
          <div className="mt-2 flex flex-wrap items-center gap-4">
            <label className="flex items-center gap-2 text-[11px] text-zinc-400">
              Give up after
              <input
                type="number"
                min={1}
                max={600}
                value={form.timeout_secs}
                onChange={(e) => setForm((f) => ({ ...f, timeout_secs: Number(e.target.value) }))}
                className={`${inputCls} w-20`}
              />
              seconds
            </label>
            <label className="flex items-center gap-2 text-[11px] text-zinc-400">
              <input
                type="checkbox"
                checked={form.enabled}
                onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
                className="accent-sky-500"
              />
              Turned on
            </label>
          </div>
        </details>
      </fieldset>

      {formError && (
        <p role="alert" className="flex items-center gap-1.5 text-[11px] text-rose-300">
          <AlertCircle size={13} />
          {formError}
        </p>
      )}

      <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-zinc-800">
        <button
          type="button"
          onClick={() => setEditing(null)}
          className="px-3.5 py-2 rounded-lg text-xs font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={saving}
          className="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-60"
        >
          {saving && <Loader2 size={13} className="animate-spin" />}
          {editing === 'new' ? 'Add & test' : 'Save & test'}
        </button>
      </div>
    </form>
  );

  return (
    <div className="kw-dialog fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-2 sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        ref={rootRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="mcp-title"
        className="kw-dialog-panel outline-none bg-zinc-900 border border-zinc-800 rounded-2xl max-w-2xl w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5"
      >
        <div className="flex items-center justify-between gap-3 border-b border-zinc-800 pb-4">
          <div className="flex items-center gap-2.5 min-w-0">
            {editing || picking ? (
              <button
                type="button"
                onClick={() => {
                  setEditing(null);
                  setPicking(false);
                }}
                aria-label="Back to servers"
                className="w-8 h-8 flex-shrink-0 rounded-lg bg-zinc-800/60 border border-zinc-700/60 flex items-center justify-center text-zinc-300 hover:text-zinc-100"
              >
                <ChevronLeft size={17} />
              </button>
            ) : (
              <div className="w-8 h-8 flex-shrink-0 rounded-lg bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-400">
                <Plug size={16} />
              </div>
            )}
            <div className="min-w-0">
              <h2 id="mcp-title" className="text-sm font-semibold text-zinc-100 truncate">
                {editing === 'new' || picking ? 'Add a tool' : editing ? `Edit ${form.name || 'server'}` : 'Tools for your kwant bots'}
              </h2>
              <p className="text-xs text-zinc-400 truncate">
                {picking
                  ? 'Find one by name, or add one by its address'
                  : editing === 'new'
                  ? 'Pick how it connects, then fill in the details'
                  : editing
                    ? 'Saved secrets stay hidden and are kept unless you change them'
                    : 'MCP servers your bots can use, with your approval'}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label="Close"
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {loadError && !editing && !picking && (
          <p role="alert" className="flex items-center gap-1.5 px-3 py-2 rounded-lg bg-rose-500/5 border border-rose-500/20 text-[11px] text-rose-300">
            <AlertCircle size={13} />
            {loadError}
          </p>
        )}

        {editing ? renderForm() : picking ? renderPicker() : renderList()}
      </div>
    </div>
  );
};
