import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Bot, ChevronLeft, Github, KeyRound, Loader2, Lock, Mail, Plug, Plus, RefreshCw, Server, UserRound } from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { relativeTime } from '../away';
import { UnlockDialog, usePasswordStore } from './PasswordStorePanel';
import {
  CatalogEntry,
  Connection,
  ConnectionsError,
  ConnectionsList,
  Tone,
  composioBody,
  composioErrorText,
  connectionsApi,
  countsLine,
  groupConnections,
  mcpServerId,
  statusBadge,
  storedLabel,
  usedByText,
} from '../connections';

const TONES: Record<Tone, string> = {
  amber: 'border-amber-500/30 bg-amber-500/10 text-amber-200',
  emerald: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  rose: 'border-rose-500/30 bg-rose-500/10 text-rose-200',
  zinc: 'border-zinc-700 bg-zinc-800 text-zinc-300',
};

const KIND_ICON: Record<string, React.ReactNode> = {
  mcp: <Server size={14} aria-hidden="true" />,
  account: <UserRound size={14} aria-hidden="true" />,
  browser_login: <KeyRound size={14} aria-hidden="true" />,
  email: <Mail size={14} aria-hidden="true" />,
  github: <Github size={14} aria-hidden="true" />,
};

const actionBtn =
  'inline-flex items-center gap-1.5 min-h-[36px] sm:min-h-[30px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const field =
  'w-full min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-500';

/** Where each kind is changed in the app. */
export interface ManageLinks {
  onOpenTools?: () => void;
  onOpenAccounts?: () => void;
  onOpenLooseEnds?: () => void;
}

const manageFor = (kind: string, links: ManageLinks): { label: string; run: () => void } | null => {
  if (kind === 'mcp' && links.onOpenTools) return { label: 'Manage', run: links.onOpenTools };
  if (kind === 'account' && links.onOpenAccounts) return { label: 'Manage', run: links.onOpenAccounts };
  if ((kind === 'email' || kind === 'github') && links.onOpenLooseEnds) return { label: 'Open Loose ends', run: links.onOpenLooseEnds };
  return null;
};

export interface ConnectionRowProps {
  c: Connection;
  now?: number;
  checking?: boolean;
  links?: ManageLinks;
  onCheck?: (c: Connection) => void;
}

export const ConnectionRow: React.FC<ConnectionRowProps> = ({ c, now = Date.now(), checking = false, links = {}, onCheck }) => {
  const id = useId();
  const badge = statusBadge(c.status);
  const manage = manageFor(c.kind, links);
  const canCheck = onCheck && mcpServerId(c) && c.status !== 'off';
  const composio = c.logins_stored === 'composio';
  return (
    <li className="px-3 py-3 sm:px-4" aria-labelledby={`${id}-n`} data-connection={c.id}>
      <div className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <span className="mt-0.5 shrink-0 text-zinc-400">{KIND_ICON[c.kind] ?? <Plug size={14} aria-hidden="true" />}</span>
        <div className="min-w-0 flex-1 basis-48">
          <div className="flex flex-wrap items-center gap-2">
            <h4 id={`${id}-n`} className="text-sm font-medium text-zinc-100 break-all">
              {c.name}
            </h4>
            <span className={`inline-flex items-center px-1.5 py-px rounded border text-[11px] font-medium ${TONES[badge.tone]}`} data-status={c.status}>
              {badge.label}
            </span>
          </div>
          {c.detail && <p className="mt-0.5 text-xs text-zinc-400 break-all">{c.detail}</p>}
          {c.status_detail && (
            <p className={`mt-1 text-xs ${c.status === 'error' ? 'text-rose-300' : 'text-amber-200'} break-words`}>{c.status_detail}</p>
          )}
          <dl className="mt-1.5 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-zinc-400">
            <div className="flex items-center gap-1">
              <dt className="sr-only">Used by</dt>
              <Bot size={11} aria-hidden="true" />
              <dd>{usedByText(c.used_by, c.status)}</dd>
            </div>
            <div>
              <dt className="sr-only">Last check</dt>
              <dd>{c.last_check ? `Checked ${relativeTime(c.last_check, now)}` : 'Never checked'}</dd>
            </div>
            <div className={composio ? 'text-amber-200' : undefined}>
              <dt className="sr-only">Where the login is stored</dt>
              <dd className="flex items-center gap-1">
                {composio ? <AlertTriangle size={11} aria-hidden="true" /> : <Lock size={11} aria-hidden="true" />}
                {storedLabel(c.logins_stored, c.kind)}
              </dd>
            </div>
          </dl>
        </div>
        {(canCheck || manage) && (
          <div className="flex gap-2 shrink-0">
            {canCheck && (
              <button type="button" className={quietBtn} disabled={checking} onClick={() => onCheck!(c)} aria-label={`Check ${c.name} now`}>
                <RefreshCw size={12} className={checking ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
                {checking ? 'Checking…' : 'Check now'}
              </button>
            )}
            {manage && (
              <button type="button" className={quietBtn} onClick={manage.run} aria-label={`${manage.label}: ${c.name}`}>
                {manage.label}
              </button>
            )}
          </div>
        )}
      </div>
    </li>
  );
};

export interface ComposioCardProps {
  entry: CatalogEntry;
  /** Tests open the form. */
  initialOpen?: boolean;
  onAdd: (url: string, apiKey: string, enabled: boolean) => Promise<string | null>;
  onOpenTools?: () => void;
}

/** The optional Composio connector: what it is, where it keeps sign-ins, and the add form. */
export const ComposioCard: React.FC<ComposioCardProps> = ({ entry, initialOpen = false, onAdd, onOpenTools }) => {
  const id = useId();
  const [open, setOpen] = useState(initialOpen);
  const [url, setUrl] = useState('');
  const [key, setKey] = useState('');
  const [enabled, setEnabled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const body = composioBody(url, key, enabled);
    if ('error' in body) return setError(body.error);
    setSaving(true);
    setError(null);
    const problem = await onAdd(body.url, body.api_key ?? '', body.enabled);
    setSaving(false);
    if (problem) setError(problem);
    else (setOpen(false), setUrl(''), setKey(''), setEnabled(false));
  };

  return (
    <li className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3" aria-labelledby={`${id}-n`} data-catalog={entry.id}>
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1 basis-56">
          <div className="flex flex-wrap items-center gap-2">
            <h4 id={`${id}-n`} className="text-sm font-medium text-zinc-100">
              {entry.name}
            </h4>
            <span className={`px-1.5 py-px rounded border text-[11px] ${TONES.zinc}`}>Optional</span>
            {entry.added && (
              <span className={`px-1.5 py-px rounded border text-[11px] ${entry.enabled ? TONES.emerald : TONES.zinc}`}>{entry.enabled ? 'Added · on' : 'Added · off'}</span>
            )}
          </div>
          <p className="mt-1 text-xs text-zinc-400">{entry.description}</p>
        </div>
        {!entry.added && !open && (
          <button type="button" className={quietBtn} onClick={() => setOpen(true)} aria-expanded={false}>
            <Plus size={12} aria-hidden="true" /> Add {entry.name}
          </button>
        )}
        {entry.added && onOpenTools && (
          <button type="button" className={quietBtn} onClick={onOpenTools}>
            {entry.enabled ? 'Manage in Tools' : 'Turn on in Tools'}
          </button>
        )}
      </div>

      <p className="flex items-start gap-2 px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100" data-warning>
        <AlertTriangle size={14} className="mt-px shrink-0 text-amber-300" aria-hidden="true" />
        <span>{entry.warning}</span>
      </p>

      {open && !entry.added && (
        <form onSubmit={submit} className="space-y-3" aria-label={`Add ${entry.name}`} noValidate>
          <div className="space-y-1">
            <label htmlFor={`${id}-url`} className="block text-xs font-medium text-zinc-200">
              MCP URL
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
              onChange={(e) => setUrl(e.target.value)}
              aria-describedby={`${id}-url-h`}
              aria-invalid={error ? true : undefined}
              autoFocus
            />
            <p id={`${id}-url-h`} className="text-[11px] text-zinc-400">
              From your Composio dashboard. It has to be an https address on composio.dev.
            </p>
          </div>
          <div className="space-y-1">
            <label htmlFor={`${id}-key`} className="block text-xs font-medium text-zinc-200">
              API key <span className="font-normal text-zinc-400">(optional)</span>
            </label>
            <input id={`${id}-key`} className={field} type="password" autoComplete="off" value={key} onChange={(e) => setKey(e.target.value)} aria-describedby={`${id}-key-h`} />
            <p id={`${id}-key-h`} className="text-[11px] text-zinc-400">
              Stored like any tool-server header and never shown again.
            </p>
          </div>
          <label className="flex items-start gap-2 text-xs text-zinc-200">
            <input type="checkbox" className="mt-0.5 accent-sky-500" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
            <span>
              Turn it on for my bots now
              <span className="block text-[11px] text-zinc-400">Off by default. Until it’s on, no bot can use it.</span>
            </span>
          </label>
          <div role="status" aria-live="polite">
            {error && <p className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 text-xs text-rose-300">{error}</p>}
          </div>
          <div className="flex flex-wrap gap-2">
            <button type="submit" disabled={saving} className={`${actionBtn} bg-sky-500 text-zinc-950 hover:bg-sky-400`}>
              {saving && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
              Add {entry.name}
            </button>
            <button type="button" className={quietBtn} onClick={() => (setOpen(false), setError(null))}>
              Cancel
            </button>
          </div>
        </form>
      )}
    </li>
  );
};

export interface ConnectionsViewProps extends ManageLinks {
  data: ConnectionsList | null;
  loading: boolean;
  error: string | null;
  needsToken?: boolean;
  notice?: { text: string; error: boolean } | null;
  checking?: string | null;
  now?: number;
  onRefresh: () => void;
  onCheck?: (c: Connection) => void;
  onUnlockVault?: () => void;
  onAddComposio: ComposioCardProps['onAdd'];
  onBack?: () => void;
  /** Tests open the Composio form. */
  composioOpen?: boolean;
}

export const ConnectionsView: React.FC<ConnectionsViewProps> = ({
  data,
  loading,
  error,
  needsToken = false,
  notice = null,
  checking = null,
  now = Date.now(),
  onRefresh,
  onCheck,
  onUnlockVault,
  onAddComposio,
  onBack,
  composioOpen = false,
  ...links
}) => {
  const groups = data ? groupConnections(data.connections) : [];
  const vault = data?.vault;
  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      <div className={`h-14 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`}>
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            aria-label="Back to chats"
            className="shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <ChevronLeft size={18} />
          </button>
        )}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">Connections</h2>
          <p className="text-[11px] text-zinc-400 truncate mt-0.5">{data ? countsLine(data.connections) : 'Everything your bots reach outside the app'}</p>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading || needsToken}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <RefreshCw size={12} className={loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          Refresh
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-4 sm:p-6" aria-busy={loading}>
        <div className="max-w-3xl mx-auto space-y-5">
          <p className="text-[11px] text-zinc-400">Tool servers, accounts and logins your bots use. Logins stay in your own vault on this computer unless a row says otherwise. Nothing secret is shown here.</p>

          {needsToken && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 flex flex-wrap items-center gap-2 text-xs text-amber-100">
              <span>Only you can see connections. Confirm it’s you to open them.</span>
              <button type="button" onClick={requestPersonToken} className="ml-auto px-2 py-0.5 rounded border border-amber-500/40 hover:bg-amber-500/10">
                Confirm it’s you
              </button>
            </div>
          )}

          {error && !needsToken && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
              <span>Couldn’t load connections: {error}</span>
              <button type="button" onClick={onRefresh} className="ml-auto px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10">
                Retry
              </button>
            </div>
          )}

          <div role="status" aria-live="polite">
            {notice && (
              <p className={`px-3 py-2 rounded-lg border text-xs ${notice.error ? 'border-amber-500/30 bg-amber-500/10 text-amber-200' : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'}`}>
                {notice.text}
              </p>
            )}
          </div>

          {!data && loading && (
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
            </p>
          )}

          {vault && vault.status !== 'ok' && (
            <div className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 flex flex-wrap items-center gap-2 text-xs text-amber-100" data-vault={vault.status}>
              <Lock size={13} aria-hidden="true" />
              <span>{vault.status === 'locked' ? 'Unlock Bitwarden to list saved logins.' : 'Saved logins can’t be read right now, so they aren’t listed.'}</span>
              {vault.status === 'locked' && onUnlockVault && (
                <button type="button" onClick={onUnlockVault} className="ml-auto px-2 py-0.5 rounded border border-amber-500/40 hover:bg-amber-500/10">
                  Unlock Bitwarden
                </button>
              )}
            </div>
          )}

          {data &&
            groups.map((g) => {
              const lockedLogins = g.key === 'browser_login' && vault && vault.status !== 'ok';
              return (
                <section key={g.key} aria-labelledby={`conn-${g.key}`} className="space-y-2">
                  <h3 id={`conn-${g.key}`} className="text-xs font-semibold text-zinc-300 uppercase tracking-wider">
                    {g.label} <span className="font-mono font-normal text-zinc-400">{g.items.length}</span>
                  </h3>
                  {g.items.length > 0 ? (
                    <ul className="rounded-xl border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800">
                      {g.items.map((c) => (
                        <ConnectionRow key={c.id} c={c} now={now} checking={checking === c.id} links={links} onCheck={onCheck} />
                      ))}
                    </ul>
                  ) : (
                    <p className="px-3 py-3 rounded-xl border border-dashed border-zinc-800 text-xs text-zinc-400" data-empty={g.key}>
                      {lockedLogins ? 'Hidden while Bitwarden is locked.' : g.empty}
                    </p>
                  )}
                </section>
              );
            })}

          {data && data.catalog.length > 0 && (
            <section aria-labelledby="conn-catalog" className="space-y-2">
              <h3 id="conn-catalog" className="text-xs font-semibold text-zinc-300 uppercase tracking-wider">
                Optional connectors
              </h3>
              <ul className="space-y-2">
                {data.catalog.map((entry) => (
                  <ComposioCard key={entry.id} entry={entry} initialOpen={composioOpen} onAdd={onAddComposio} onOpenTools={links.onOpenTools} />
                ))}
              </ul>
            </section>
          )}
        </div>
      </div>
    </div>
  );
};

/** The page: loads the list, checks MCP servers, unlocks the vault and adds Composio. */
export const ConnectionsPage: React.FC<ManageLinks & { onBack?: () => void }> = ({ onBack, ...links }) => {
  const [data, setData] = useState<ConnectionsList | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [needsToken, setNeedsToken] = useState(false);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [checking, setChecking] = useState<string | null>(null);
  const [unlocking, setUnlocking] = useState(false);
  const store = usePasswordStore(unlocking, true);
  const alive = useRef(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await connectionsApi.list();
      if (!alive.current) return;
      setData(r);
      setError(null);
      setNeedsToken(false);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ConnectionsError && e.status === 401) setNeedsToken(true);
      else setError(e instanceof Error ? e.message : 'Something went wrong.');
    } finally {
      if (alive.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    load();
    window.addEventListener(PERSON_SIGNED_IN_EVENT, load);
    return () => {
      alive.current = false;
      window.removeEventListener(PERSON_SIGNED_IN_EVENT, load);
    };
  }, [load]);

  const check = async (c: Connection) => {
    const sid = mcpServerId(c);
    if (!sid || checking) return;
    setChecking(c.id);
    setNotice(null);
    try {
      const r = await connectionsApi.testMcp(sid);
      if (alive.current) setNotice(r.ok === false ? { text: `${c.name}: ${r.error || 'the check failed.'}`, error: true } : { text: `${c.name} answered.`, error: false });
    } catch (e) {
      if (alive.current) setNotice({ text: `Couldn’t check ${c.name}: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
    } finally {
      if (alive.current) setChecking(null);
      load();
    }
  };

  const addComposio = async (url: string, apiKey: string, enabled: boolean): Promise<string | null> => {
    try {
      await connectionsApi.addComposio(apiKey ? { url, api_key: apiKey, enabled } : { url, enabled });
      setNotice({ text: enabled ? 'Composio is added and on.' : 'Composio is added and off. Turn it on in Tools when you want bots to use it.', error: false });
      load();
      return null;
    } catch (e) {
      return composioErrorText(e);
    }
  };

  return (
    <>
      <ConnectionsView
        data={data}
        loading={loading}
        error={error}
        needsToken={needsToken}
        notice={notice}
        checking={checking}
        onRefresh={load}
        onCheck={check}
        onUnlockVault={() => setUnlocking(true)}
        onAddComposio={addComposio}
        onBack={onBack}
        {...links}
      />
      {unlocking && (
        <UnlockDialog
          status={store.status}
          onClose={() => setUnlocking(false)}
          onUnlocked={(s) => {
            store.set(s);
            load();
          }}
        />
      )}
    </>
  );
};
