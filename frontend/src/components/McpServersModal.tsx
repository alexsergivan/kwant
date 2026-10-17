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
  Search,
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
  MCP_PRESETS,
  searchPresets,
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
} from '../mcp';

interface McpServersModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** For "Off for: …" on each card. */
  kwants?: Array<{ id: string; name: string }>;
}

const inputCls =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors';

const TRANSPORTS: Array<{ id: McpTransport; title: string; blurb: string; icon: React.ReactNode }> = [
  { id: 'stdio', title: 'Runs on this machine', blurb: 'kwnt starts a command, like npx or uvx', icon: <Terminal size={18} /> },
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
}> = ({ rows, onChange, keyPlaceholder, keyLabel, addLabel }) => {
  const [shown, setShown] = useState<Record<number, boolean>>({});
  const update = (i: number, patch: Partial<SecretRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  return (
    <div className="space-y-1.5">
      {rows.map((row, i) => (
        <div key={i} className="flex flex-col sm:flex-row gap-1.5">
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
                type={shown[i] ? 'text' : 'password'}
                autoComplete="off"
                value={row.value}
                onChange={(e) => update(i, { value: e.target.value })}
                placeholder={row.saved ? '•••••••• saved, leave blank to keep' : row.hint || 'Secret value'}
                className={`${inputCls} pr-8 font-mono`}
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
      ))}
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

const PresetCatalog: React.FC<{
  query: string;
  onQuery: (q: string) => void;
  onPick: (preset: McpPreset) => void;
  onCustom: () => void;
  autoFocus?: boolean;
}> = ({ query, onQuery, onPick, onCustom, autoFocus }) => {
  const groups = groupPresets(searchPresets(query));
  return (
    <div className="space-y-4 text-left">
      <div className="relative">
        <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500 pointer-events-none" aria-hidden="true" />
        <input
          type="search"
          aria-label="Search servers"
          value={query}
          autoFocus={autoFocus}
          onChange={(e) => onQuery(e.target.value)}
          placeholder={`Search ${MCP_PRESETS.length} servers, like notion or database`}
          className={`${inputCls} pl-8`}
        />
      </div>
      {groups.length === 0 && (
        <p className="py-4 text-center text-xs text-zinc-500">
          Nothing matches “{query.trim()}”. You can still add it as a custom server.
        </p>
      )}
      {groups.map((g) => (
        <section key={g.id} aria-labelledby={`mcp-cat-${g.id}`}>
          <h4 id={`mcp-cat-${g.id}`} className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-zinc-500">
            {g.label}
          </h4>
          <ul className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {g.presets.map((p) => (
              <li key={p.id}>
                <button
                  type="button"
                  onClick={() => onPick(p)}
                  className="group w-full h-full p-3 rounded-xl border border-zinc-800 bg-zinc-950/60 hover:border-sky-500/40 hover:bg-sky-500/5 focus-visible:border-sky-500/60 focus-visible:outline-none text-left transition-all"
                >
                  <span className="flex items-center gap-1.5 text-xs font-medium text-zinc-200 group-hover:text-sky-200">
                    <span className="text-sky-400" aria-hidden="true">
                      {p.form.transport === 'stdio' ? <Terminal size={13} /> : <Globe size={13} />}
                    </span>
                    {p.label}
                  </span>
                  <span className="mt-1 block text-[11px] text-zinc-500 leading-snug">{p.blurb}</span>
                  <span className="mt-2 flex flex-wrap gap-1">
                    {(p.needs.length ? p.needs : ['Nothing to install']).map((n) => (
                      <span
                        key={n}
                        className={`px-1.5 py-px rounded border text-[10px] ${
                          n === 'Sign-in' || n === 'Token' || n === 'API key'
                            ? 'border-amber-500/25 text-amber-300/90'
                            : 'border-zinc-700/70 text-zinc-400'
                        }`}
                      >
                        {n}
                      </span>
                    ))}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
      <button
        type="button"
        onClick={onCustom}
        className="w-full flex items-center gap-2.5 p-3 rounded-xl border border-dashed border-zinc-700 hover:border-sky-500/40 hover:bg-sky-500/5 focus-visible:border-sky-500/60 focus-visible:outline-none text-left transition-all"
      >
        <Plus size={16} className="flex-shrink-0 text-sky-400" />
        <span>
          <span className="block text-xs font-medium text-zinc-200">Custom server</span>
          <span className="block text-[11px] text-zinc-500">Paste a command or URL from any server’s README.</span>
        </span>
      </button>
    </div>
  );
};

export const McpServersModal: React.FC<McpServersModalProps> = ({ isOpen, onClose, kwants = [] }) => {
  const [servers, setServers] = useState<McpServer[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [statuses, setStatuses] = useState<Record<string, StoredStatus>>({});
  const [testing, setTesting] = useState<Record<string, boolean>>({});
  const [testResults, setTestResults] = useState<Record<string, McpTestResult>>({});
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  // Form view: null = list, 'new' = adding, otherwise the id being edited.
  const [editing, setEditing] = useState<string | null>(null);
  const [form, setForm] = useState<McpFormState>(emptyMcpForm());
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // Catalog step before the form when adding, its search, and the preset the form came from.
  const [picking, setPicking] = useState(false);
  const [query, setQuery] = useState('');
  const [presetId, setPresetId] = useState<string | null>(null);
  // Sign-in: the server whose popup is open, sign-in errors per server, and a pending disconnect.
  const [connecting, setConnecting] = useState<string | null>(null);
  const [oauthErrors, setOauthErrors] = useState<Record<string, string>>({});
  const [confirmDisconnect, setConfirmDisconnect] = useState<string | null>(null);
  const popupRef = useRef<Window | null>(null);
  const popupWatch = useRef<number | null>(null);

  const refresh = useCallback(async (): Promise<McpServer[] | null> => {
    try {
      const res = await fetch('/api/mcp/servers');
      if (!res.ok) throw new Error(await readError(res));
      const list: McpServer[] = await res.json();
      setServers(list);
      setLoadError(null);
      return list;
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : 'Could not reach kwnt.');
      setServers((prev) => prev ?? []);
      return null;
    }
  }, []);

  const stopWatchingPopup = useCallback(() => {
    if (popupWatch.current !== null) window.clearInterval(popupWatch.current);
    popupWatch.current = null;
    popupRef.current = null;
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
      result = { ok: false, error: 'Could not reach kwnt.' };
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
    setPicking(false);
    setQuery('');
    setConfirmDelete(null);
    setConfirmDisconnect(null);
    setTestResults({});
    setOauthErrors({});
    setStatuses(loadStatuses());
    // Also catches a sign-in finished elsewhere, like the same-tab fallback when popups are blocked.
    refresh().then(() => checkNewSignIns());
  }, [isOpen, refresh, checkNewSignIns]);

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

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      if (editing) setEditing(null);
      else if (picking) setPicking(false);
      else onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [isOpen, editing, picking, onClose]);

  if (!isOpen) return null;

  const connect = async (server: McpServer) => {
    playSfx('click');
    setOauthErrors(({ [server.id]: _, ...rest }) => rest);
    // Open the window inside the click so popup blockers allow it; the address comes after the fetch.
    const popup = window.open('', 'kwnt-mcp-oauth', 'popup,width=520,height=720');
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
      if (typeof body?.authorize_url !== 'string' || !/^https?:\/\//.test(body.authorize_url)) throw new Error('kwnt didn’t return a sign-in page.');
      url = body.authorize_url;
    } catch (err) {
      popup?.close();
      setConnecting(null);
      setOauthErrors((e) => ({ ...e, [server.id]: err instanceof Error ? err.message : 'Could not reach kwnt.' }));
      playSfx('alert');
      return;
    }
    if (!popup || popup.closed) {
      // Blocked anyway: sign in in this tab. kwnt brings you back to the app afterwards.
      window.location.assign(url);
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
      setLoadError(res ? await readError(res) : 'Could not reach kwnt.');
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
    setQuery('');
    setPicking(true);
  };

  const openForm = (target: 'new' | McpServer, preset?: McpPreset) => {
    playSfx('click');
    setFormError(null);
    setPicking(false);
    setPresetId(preset?.id ?? null);
    if (target === 'new') {
      setEditing('new');
      const f = preset?.form;
      setForm(f ? { ...f, env: f.env.map((r) => ({ ...r })), headers: f.headers.map((r) => ({ ...r })) } : emptyMcpForm());
    } else {
      setEditing(target.id);
      setForm(serverToForm(target));
    }
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
      setFormError('Could not reach kwnt.');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (server: McpServer) => {
    playSfx('click');
    const res = await fetch(`/api/mcp/servers/${server.id}`, { method: 'DELETE' }).catch(() => null);
    setConfirmDelete(null);
    if (!res || !res.ok) {
      setLoadError(res ? await readError(res) : 'Could not reach kwnt.');
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
    if (!res || !res.ok) setLoadError(res ? await readError(res) : 'Could not reach kwnt.');
    refresh();
  };

  const close = () => {
    playSfx('click');
    onClose();
  };

  const parsed = splitCommandLine(form.commandLine);
  const preset = editing === 'new' && presetId ? MCP_PRESETS.find((p) => p.id === presetId) : undefined;

  const renderPicker = () => (
    <PresetCatalog query={query} onQuery={setQuery} onPick={(p) => openForm('new', p)} onCustom={() => openForm('new')} autoFocus />
  );

  const renderEmpty = () => (
    <div className="text-center py-6 px-2 space-y-5">
      <div className="mx-auto w-14 h-14 rounded-2xl bg-sky-500/10 border border-sky-500/20 flex items-center justify-center text-sky-300">
        <Plug size={26} />
      </div>
      <div className="space-y-1.5 max-w-md mx-auto">
        <h3 className="text-sm font-semibold text-zinc-100">Give your kwants new abilities</h3>
        <p className="text-xs text-zinc-400 leading-relaxed">
          MCP servers are small add-ons that hand your kwants tools, like reading files, browsing the web or working with GitHub.
          Add one here and every kwant can use it; switch it off for a kwant in that kwant’s edit screen. They always ask you before using a tool.
          Pick a popular one below and just add your key or sign in.
        </p>
      </div>
      <PresetCatalog query={query} onQuery={setQuery} onPick={(p) => openForm('new', p)} onCustom={() => openForm('new')} />
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
                    server.enabled ? 'Used by all kwants' : 'All kwants get it once it’s on'
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
                  ? 'Waiting for you to approve access in the sign-in window…'
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
              disabled={isConnecting}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] font-medium text-white bg-sky-600 hover:bg-sky-500 transition-colors shadow-sm disabled:opacity-60"
            >
              {isConnecting ? <Loader2 size={12} className="animate-spin" /> : <KeyRound size={12} />}
              {isConnecting ? 'Waiting…' : oauthError ? 'Retry' : 'Connect'}
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
              <span>Sign out of {server.name}? Kwants can’t use its tools until you connect again.</span>
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
              <span>Delete {server.name}? Kwants using it lose these tools.</span>
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
            {preset ? (
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
              {preset ? 'Pick a different one' : 'Or pick a popular server'}
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
            Short and simple: letters, numbers, - or _. Kwants see its tools as <span className="font-mono text-zinc-400">{form.name.trim() || 'name'} · tool</span>.
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
              <span>This command runs on the kwnt machine with your user’s permissions. Only add servers you trust.</span>
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
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-2 sm:p-4" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="mcp-title"
        className="bg-zinc-900 border border-zinc-800 rounded-2xl max-w-2xl w-full max-h-[92vh] overflow-y-auto p-4 sm:p-6 text-zinc-100 shadow-2xl space-y-5"
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
                {editing === 'new' || picking ? 'Add a tool server' : editing ? `Edit ${form.name || 'server'}` : 'Tools for your kwants'}
              </h2>
              <p className="text-xs text-zinc-400 truncate">
                {picking
                  ? 'Pick a popular server or add your own'
                  : editing === 'new'
                  ? 'Pick how it connects, then fill in the details'
                  : editing
                    ? 'Saved secrets stay hidden and are kept unless you change them'
                    : 'MCP servers your kwants can use, with your approval'}
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
