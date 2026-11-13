import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Bot, ChevronLeft, ChevronRight, ExternalLink, Github, KeyRound, Loader2, Lock, Mail, Plug, Plus, RefreshCw, Search, Server, StickyNote, UserRound } from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { relativeTime } from '../away';
import { quotaLine } from '../clock';
import { UnlockDialog, usePasswordStore } from './PasswordStorePanel';
import { devApi, devStore } from '../devMode';
import { useDevModeOn } from './DevModeSwitch';
import {
  CatalogEntry,
  ComposioApp,
  Connection,
  ConnectionsError,
  ConnectionsList,
  Tone,
  composioBody,
  composioErrorText,
  connectionsApi,
  countsLine,
  groupConnections,
  isRawCommand,
  canMarkProduction,
  mcpServerId,
  statusBadge,
  storedLabel,
  usedByText,
} from '../connections';
import { inDesktopApp } from '../externalLinks';
import { editorProps } from '../escape';
import { TelegramConnection } from './TelegramCard';
import { ToolNotesSection, ToolNotesSectionProps } from './ToolNotesSection';
import { notesChipText } from '../toolNotes';
import { usePhoneLayout } from '../usePhoneLayout';
import { ComposioTile, ComposioTileApi } from './ComposioTile';

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
  /** Opens Tools straight on adding a server; falls back to onOpenTools. */
  onAddTool?: () => void;
  onOpenAccounts?: () => void;
  onOpenLooseEnds?: () => void;
}

const manageFor = (kind: string, links: ManageLinks): { label: string; run: () => void } | null => {
  if (kind === 'mcp' && links.onOpenTools) return { label: 'Manage tools', run: links.onOpenTools };
  if (kind === 'account' && links.onOpenAccounts) return { label: 'Manage accounts', run: links.onOpenAccounts };
  if ((kind === 'email' || kind === 'github') && links.onOpenLooseEnds) return { label: 'Open Loose ends', run: links.onOpenLooseEnds };
  return null;
};

export interface ConnectionRowProps {
  c: Connection;
  now?: number;
  checking?: boolean;
  links?: ManageLinks;
  onCheck?: (c: Connection) => void;
  /** Dev mode is on in this space: MCP servers and Composio apps get a Production mark (D10). */
  onProduction?: (c: Connection, production: boolean) => void;
  markingProduction?: boolean;
  /** 0.9.0 P5: the "2 notes" chip filters Notes from bots to this connection. */
  onNotes?: (name: string) => void;
}

export const ConnectionRow: React.FC<ConnectionRowProps> = ({ c, now = Date.now(), checking = false, links = {}, onCheck, onProduction, markingProduction = false, onNotes }) => {
  const id = useId();
  const badge = statusBadge(c.status);
  const manage = manageFor(c.kind, links);
  const canCheck = onCheck && mcpServerId(c) && c.status !== 'off';
  const composio = c.logins_stored === 'composio';
  const status = quotaLine(c.status_detail, c.limited_until, now);
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
            {c.production && (
              <span className="inline-flex items-center px-1.5 py-px rounded border border-amber-400/40 bg-amber-500/10 text-[11px] font-medium text-amber-200" data-production-badge>
                Production
              </span>
            )}
            {c.kind === 'mcp' && c.notes && c.notes.open > 0 && (
              <button
                type="button"
                onClick={() => onNotes?.(c.name)}
                disabled={!onNotes}
                aria-label={`${notesChipText(c.notes.open)} about ${c.name}${c.notes.broke ? `, ${c.notes.broke} didn’t work` : ''}: show them`}
                className={`inline-flex items-center gap-1 px-1.5 py-px rounded-md border text-[11px] font-medium hover:bg-zinc-800 disabled:hover:bg-transparent focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                  c.notes.broke > 0 ? 'border-rose-300/40 text-rose-200' : 'border-zinc-700 text-zinc-300'
                }`}
                data-notes-chip={c.notes.broke > 0 ? 'broke' : 'open'}
              >
                <StickyNote size={11} aria-hidden="true" />
                {notesChipText(c.notes.open)}
              </button>
            )}
          </div>
          {c.detail &&
            (isRawCommand(c) ? (
              // A command or path says little to most people: it waits behind Details.
              <details className="mt-0.5 text-xs text-zinc-400 group">
                <summary className="inline-flex items-center gap-1 cursor-pointer select-none text-[11px] text-zinc-500 hover:text-zinc-300 list-none [&::-webkit-details-marker]:hidden">
                  <ChevronRight size={11} className="transition-transform group-open:rotate-90 motion-reduce:transition-none" aria-hidden="true" />
                  Details
                </summary>
                <code className="mt-1 block font-mono text-[11px] text-zinc-400 break-all">{c.detail}</code>
              </details>
            ) : (
              <p className="mt-0.5 text-xs text-zinc-400 break-all">{c.detail}</p>
            ))}
          {status && <p className={`mt-1 text-xs ${c.status === 'error' ? 'text-rose-300' : 'text-amber-200'} break-words`}>{status}</p>}
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
        {(canCheck || manage || (onProduction && canMarkProduction(c))) && (
          <div className="flex flex-wrap gap-2 shrink-0">
            {onProduction && canMarkProduction(c) && (
              <label className={`${quietBtn} cursor-pointer has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-sky-500`} title="Steps that use a production connection always ask you while Dev mode is on.">
                <input
                  type="checkbox"
                  className="accent-amber-400"
                  checked={!!c.production}
                  disabled={markingProduction}
                  onChange={(e) => onProduction(c, e.target.checked)}
                  aria-label={`${c.name} is production`}
                  data-production-toggle={c.id}
                />
                Production
              </label>
            )}
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

export interface ComposioSearchApi {
  search: (q: string) => Promise<{ apps: ComposioApp[]; can_connect: boolean }>;
  connect: (slug: string) => Promise<{ redirect_url: string }>;
}

const errorText = (e: unknown) => (e instanceof ConnectionsError || e instanceof Error ? e.message : 'Something went wrong.');

/** "Find an app": searches Composio's catalogue through kwnt, with the person's own Composio key. */
export const ComposioAppSearch: React.FC<{
  entry: CatalogEntry;
  api?: ComposioSearchApi;
  /** Tests show results without searching. */
  initialApps?: ComposioApp[];
  /** Apps & accounts: "Connect" opens the Connect sheet for this app instead of its sign-in. */
  onPick?: (app: ComposioApp) => void;
}> = ({ entry, api = { search: connectionsApi.searchComposioApps, connect: connectionsApi.connectComposioApp }, initialApps, onPick }) => {
  const id = useId();
  const [q, setQ] = useState('');
  const [apps, setApps] = useState<ComposioApp[] | null>(initialApps ?? null);
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [opened, setOpened] = useState<string | null>(null);
  const canConnect = entry.can_connect === true;

  if (!entry.can_search) {
    return (
      <p className="text-[11px] text-zinc-400" data-composio-search="needs-key">
        To search apps here, add your Composio API key to the Composio server under Advanced › Manage tools (as the header <code className="text-zinc-300">x-api-key</code>).
      </p>
    );
  }

  const search = async (e: React.FormEvent) => {
    e.preventDefault();
    const query = q.trim();
    if (!query) return setError('Type an app name, like gmail.');
    setBusy(true);
    setError(null);
    setOpened(null);
    try {
      setApps((await api.search(query)).apps);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const connect = async (app: ComposioApp) => {
    // open the tab now, while the click still counts as the person's, then point it at Composio
    // The Mac app opens no second window: its links go to the default browser.
    const tab = typeof window !== 'undefined' && !inDesktopApp() ? window.open('', '_blank') : null;
    if (tab) tab.opener = null;
    setConnecting(app.slug);
    setError(null);
    try {
      const { redirect_url } = await api.connect(app.slug);
      if (tab) tab.location.href = redirect_url;
      else if (inDesktopApp()) window.location.assign(redirect_url);
      else window.open(redirect_url, '_blank', 'noopener,noreferrer');
      setOpened(app.name);
    } catch (err) {
      tab?.close();
      setError(errorText(err));
    } finally {
      setConnecting(null);
    }
  };

  return (
    <div className="space-y-2" data-composio-search>
      <form onSubmit={search} role="search" aria-label="Find an app on Composio" className="flex gap-2">
        <label htmlFor={`${id}-q`} className="sr-only">
          Find an app
        </label>
        <input
          id={`${id}-q`}
          className={field}
          type="search"
          autoComplete="off"
          spellCheck={false}
          maxLength={60}
          placeholder="Find an app, like gmail or figma"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <button type="submit" disabled={busy} className={quietBtn}>
          {busy ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Search size={12} aria-hidden="true" />}
          Find
        </button>
      </form>
      {!canConnect && !onPick && (
        <p className="text-[11px] text-zinc-400">
          Your Composio URL has no <code className="text-zinc-300">user_id</code>, so connect apps on composio.dev. If your Composio server lists fixed apps, add the app to it there too.
        </p>
      )}
      <div role="status" aria-live="polite">
        {error && <p className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 text-xs text-rose-300">{error}</p>}
        {opened && (
          <p className="text-xs text-emerald-200">
            Opened {opened}’s sign-in on Composio in {inDesktopApp() ? 'your browser' : 'a new tab'}. Its tools reach your bots through the Composio server once you finish there.
          </p>
        )}
        {apps && apps.length === 0 && <p className="text-xs text-zinc-400">No Composio apps match.</p>}
      </div>
      {apps && apps.length > 0 && (
        <ul className="divide-y divide-zinc-800 rounded-lg border border-zinc-800" aria-label="Composio apps">
          {apps.map((a) => (
            <li key={a.slug} className="flex flex-wrap items-center gap-2 px-3 py-2" data-composio-app={a.slug}>
              <div className="min-w-0 flex-1 basis-48">
                <p className="text-xs font-medium text-zinc-100">
                  {a.name}
                  <span className="ml-1.5 font-normal text-zinc-500">{a.slug}</span>
                </p>
                {a.description && <p className="text-[11px] text-zinc-400 line-clamp-2">{a.description}</p>}
                <p className="text-[11px] text-zinc-500">
                  {a.tools_count != null && `${a.tools_count} tools · `}
                  {a.no_auth ? 'No sign-in needed' : 'Sign-in kept on Composio'}
                </p>
              </div>
              {onPick ? (
                <button type="button" className={quietBtn} onClick={() => onPick(a)} data-composio-pick={a.slug}>
                  Connect<span className="sr-only"> {a.name}</span>
                </button>
              ) : canConnect && !a.no_auth && (
                <button type="button" className={quietBtn} disabled={connecting !== null} onClick={() => connect(a)}>
                  {connecting === a.slug ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ExternalLink size={12} aria-hidden="true" />}
                  Connect<span className="sr-only"> {a.name} on Composio (opens in a new tab)</span>
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export interface ComposioCardProps {
  entry: CatalogEntry;
  /** Tests open the form. */
  initialOpen?: boolean;
  onAdd: (url: string, apiKey: string, enabled: boolean) => Promise<string | null>;
  onOpenTools?: () => void;
  /** Apps & accounts: "Choose bots" opens the Connect sheet (picking bots turns it on). */
  onChooseBots?: () => void;
  /** Tests: the search box's API and starting results. */
  searchApi?: ComposioSearchApi;
  initialApps?: ComposioApp[];
  /** Goes up by one each time "+ Add connection" picks Composio: scroll here and open the form. */
  openRequest?: number;
}

/** The optional Composio connector: what it is, where it keeps sign-ins, and the add form. */
export const ComposioCard: React.FC<ComposioCardProps> = ({ entry, initialOpen = false, onAdd, onOpenTools, onChooseBots, searchApi, initialApps, openRequest = 0 }) => {
  const id = useId();
  const [open, setOpen] = useState(initialOpen);
  const cardRef = useRef<HTMLLIElement>(null);

  useEffect(() => {
    if (!openRequest) return;
    if (!entry.added) setOpen(true);
    const card = cardRef.current;
    card?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    // An added card has no form: focus its first control, usually "Find an app".
    if (entry.added) card?.querySelector<HTMLElement>('input, button')?.focus({ preventScroll: true });
  }, [openRequest, entry.added]);
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
    <li ref={cardRef} className="rounded-xl border border-zinc-800 bg-zinc-900/60 p-3 sm:p-4 space-y-3 scroll-mt-4" aria-labelledby={`${id}-n`} data-catalog={entry.id}>
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
        {entry.added && onChooseBots && (
          <button type="button" className={quietBtn} onClick={onChooseBots} data-composio-choose-bots>
            {entry.enabled ? 'Choose bots' : 'Choose bots to turn it on'}
          </button>
        )}
        {entry.added && !onChooseBots && onOpenTools && (
          <button type="button" className={quietBtn} onClick={onOpenTools}>
            {entry.enabled ? 'Manage in Tools' : 'Turn on in Tools'}
          </button>
        )}
      </div>

      <p className="flex items-start gap-2 px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100" data-warning>
        <AlertTriangle size={14} className="mt-px shrink-0 text-amber-300" aria-hidden="true" />
        <span>{entry.warning}</span>
      </p>

      {entry.added && <ComposioAppSearch entry={entry} api={searchApi} initialApps={initialApps} />}

      {open && !entry.added && (
        <form onSubmit={submit} {...editorProps(!!(url.trim() || key.trim() || enabled))} className="space-y-3" aria-label={`Add ${entry.name}`} noValidate>
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

/** One way to add something, as offered by "+ Add connection" and the empty groups. */
export interface AddChoice {
  key: 'mcp' | 'account' | 'composio' | 'sources';
  label: string;
  hint: string;
  run: () => void;
}

/** The ways to add a connection that this page can actually open, in menu order. */
export const addChoices = (links: ManageLinks, onComposio?: () => void): AddChoice[] => {
  const out: AddChoice[] = [];
  const tool = links.onAddTool ?? links.onOpenTools;
  if (tool) out.push({ key: 'mcp', label: 'Tool server (MCP)', hint: 'A command on this computer or a server URL', run: tool });
  if (links.onOpenAccounts) out.push({ key: 'account', label: 'Engine account', hint: 'Claude, Codex, Copilot and other engines', run: links.onOpenAccounts });
  if (onComposio) out.push({ key: 'composio', label: 'App via Composio', hint: 'Gmail, Notion, Slack and hundreds more', run: onComposio });
  return out;
};

const ADD_ICON: Record<AddChoice['key'], React.ReactNode> = {
  mcp: <Server size={14} aria-hidden="true" />,
  account: <UserRound size={14} aria-hidden="true" />,
  composio: <Plug size={14} aria-hidden="true" />,
  sources: <Mail size={14} aria-hidden="true" />,
};

/** "+ Add connection" in the header: a small list of what can be added. */
export const AddConnectionMenu: React.FC<{ choices: AddChoice[]; disabled?: boolean; initialOpen?: boolean }> = ({ choices, disabled = false, initialOpen = false }) => {
  const id = useId();
  const [open, setOpen] = useState(initialOpen);
  const wrapRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    wrapRef.current?.querySelector<HTMLButtonElement>('[data-add-choice]')?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      // Closes the menu only, not Settings around it.
      e.preventDefault();
      setOpen(false);
      buttonRef.current?.focus();
    };
    const onDown = (e: PointerEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('pointerdown', onDown);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('pointerdown', onDown);
    };
  }, [open]);

  if (choices.length === 0) return null;
  return (
    <div ref={wrapRef} className="relative shrink-0">
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setOpen((o) => !o)}
        disabled={disabled}
        aria-expanded={open}
        aria-controls={`${id}-list`}
        className="inline-flex items-center gap-1.5 min-h-[32px] px-2.5 py-1 rounded-lg text-xs font-medium text-white bg-sky-600 hover:bg-sky-500 disabled:opacity-60 shadow-sm whitespace-nowrap focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-500"
      >
        <Plus size={14} aria-hidden="true" />
        Add connection
      </button>
      {open && (
        <div
          id={`${id}-list`}
          role="group"
          aria-label="Add a connection"
          className="absolute right-0 top-full mt-2 z-30 w-72 max-w-[calc(100vw-1rem)] rounded-xl border border-zinc-700 bg-zinc-900 p-1.5 shadow-xl shadow-black/40"
        >
          <ul className="space-y-0.5">
            {choices.map((c) => (
              <li key={c.key}>
                <button
                  type="button"
                  data-add-choice={c.key}
                  onClick={() => {
                    setOpen(false);
                    c.run();
                  }}
                  className="w-full flex items-start gap-2.5 px-2.5 py-2 rounded-lg text-left hover:bg-zinc-800 focus-visible:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                >
                  <span className="mt-0.5 shrink-0 text-zinc-400">{ADD_ICON[c.key]}</span>
                  <span className="min-w-0">
                    <span className="block text-xs font-medium text-zinc-100">{c.label}</span>
                    <span className="block text-[11px] text-zinc-400">{c.hint}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
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
  /** Inside Settings › This space: the screen has the heading and the scrolling. */
  embedded?: boolean;
  onProduction?: (c: Connection, production: boolean) => void;
  marking?: string | null;
  /** Tests open the Composio form. */
  composioOpen?: boolean;
  /** Tests and screenshots open the "+ Add connection" list. */
  addMenuOpen?: boolean;
  /** 0.9.0 P5: "See the note" from a chat shows just that note. */
  focusNote?: string | null;
  onClearFocusNote?: () => void;
  /** Refetches the connections so each row's note chip recounts. */
  onNotesChanged?: () => void;
  onOpenNoteWhere?: ToolNotesSectionProps['onOpenWhere'];
  /** Tests and screenshots: the Notes from bots section's api and starting state. */
  notes?: Partial<ToolNotesSectionProps>;
  phone?: boolean;
  /** Apps & accounts' Advanced: AI accounts have their own section there, so their group is left out. */
  hideAccounts?: boolean;
  /** Apps & accounts' Advanced: Composio's "Choose bots" (instead of "Turn on in Tools"). */
  onChooseComposioBots?: () => void;
  /** Tests and screenshots: the Composio tile's api. */
  composioTileApi?: ComposioTileApi;
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
  embedded = false,
  onProduction,
  marking = null,
  composioOpen = false,
  addMenuOpen = false,
  focusNote = null,
  onClearFocusNote,
  onNotesChanged,
  onOpenNoteWhere,
  notes: notesProps = {},
  phone,
  hideAccounts = false,
  onChooseComposioBots,
  composioTileApi,
  ...links
}) => {
  const phoneLayout = usePhoneLayout();
  const [noteConnection, setNoteConnection] = useState<string | null>(notesProps.connection ?? null);
  const [noteScroll, setNoteScroll] = useState(0);
  const filterNotes = (name: string | null) => {
    setNoteConnection(name);
    if (name) {
      onClearFocusNote?.();
      setNoteScroll((n) => n + 1);
    }
  };
  const groups = data ? groupConnections(data.connections).filter((g) => !(hideAccounts && g.key === 'account')) : [];
  const vault = data?.vault;
  const [composioRequest, setComposioRequest] = useState(0);
  const hasComposio = !!data?.catalog.some((e) => e.id === 'composio');
  const choices = addChoices(links, hasComposio ? () => setComposioRequest((n) => n + 1) : undefined);
  // An empty group links to the same place "+ Add connection" goes.
  const emptyAdd: Record<string, AddChoice | undefined> = {
    mcp: choices.find((c) => c.key === 'mcp'),
    account: choices.find((c) => c.key === 'account'),
    sources: links.onOpenLooseEnds ? { key: 'sources', label: 'Loose ends', hint: '', run: links.onOpenLooseEnds } : undefined,
  };
  return (
    <div className={embedded ? 'space-y-4' : 'flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden'}>
      <div
        className={
          embedded
            ? 'flex items-center justify-between gap-3 select-none'
            : `h-14 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`
        }
      >
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
          {!embedded && <h2 className="text-sm font-semibold text-zinc-100 truncate">Connections</h2>}
          <p className={`text-[11px] text-zinc-400 truncate ${embedded ? '' : 'mt-0.5'}`}>{data ? countsLine(data.connections) : 'Everything your bots reach outside the app'}</p>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading || needsToken}
          aria-label="Refresh"
          className="shrink-0 inline-flex items-center gap-1.5 min-h-[32px] px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <RefreshCw size={12} className={loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          <span className="hidden sm:inline">Refresh</span>
        </button>
        <AddConnectionMenu choices={choices} disabled={needsToken} initialOpen={addMenuOpen} />
      </div>

      <div className={embedded ? '' : 'flex-1 overflow-y-auto p-4 sm:p-6'} aria-busy={loading}>
        <div className={embedded ? 'space-y-5' : 'max-w-3xl mx-auto space-y-5'}>
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

          {data && !needsToken && (
            <ToolNotesSection
              connections={data.connections.filter((c) => c.kind === 'mcp').map((c) => c.name)}
              now={now}
              phone={phone ?? phoneLayout}
              onChanged={onNotesChanged}
              onOpenWhere={onOpenNoteWhere}
              focusNote={focusNote}
              onClearFocus={onClearFocusNote}
              scrollRequest={noteScroll}
              {...notesProps}
              connection={noteConnection}
              onConnection={filterNotes}
            />
          )}

          {data &&
            groups.map((g) => {
              const lockedLogins = g.key === 'browser_login' && vault && vault.status !== 'ok';
              return (
                <section key={g.key} aria-labelledby={`conn-${g.key}`} className="space-y-2">
                  <h3 id={`conn-${g.key}`} className="text-xs font-semibold text-zinc-300 uppercase tracking-wider">
                    {g.label} <span className="tabular-nums font-normal text-zinc-400">{g.items.length}</span>
                  </h3>
                  {g.items.length > 0 ? (
                    <ul className="rounded-xl border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800">
                      {g.items.map((c) => (
                        <ConnectionRow key={c.id} c={c} now={now} checking={checking === c.id} links={links} onCheck={onCheck} onProduction={onProduction} markingProduction={marking === c.id} onNotes={filterNotes} />
                      ))}
                    </ul>
                  ) : (
                    <p className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-3 rounded-xl border border-dashed border-zinc-800 text-xs text-zinc-400" data-empty={g.key}>
                      <span>{lockedLogins ? 'Hidden while Bitwarden is locked.' : g.empty}</span>
                      {!lockedLogins && emptyAdd[g.key] && (
                        <button
                          type="button"
                          onClick={emptyAdd[g.key]!.run}
                          data-empty-add={g.key}
                          className="inline-flex items-center gap-1 min-h-[28px] font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                        >
                          <Plus size={12} aria-hidden="true" />
                          {g.key === 'sources' ? 'Add one in Loose ends' : 'Add one'}
                          {g.key !== 'sources' && <span className="sr-only"> {g.label.toLowerCase()}</span>}
                        </button>
                      )}
                    </p>
                  )}
                </section>
              );
            })}

          {data && (
            <section aria-labelledby="conn-catalog" className="space-y-2">
              <h3 id="conn-catalog" className="text-xs font-semibold text-zinc-300 uppercase tracking-wider">
                Optional connectors
              </h3>
              <ul className="space-y-2">
                <TelegramConnection />
                {data.catalog.map((entry) => (
                  entry.id === 'composio' ? (
                    <ComposioTile
                      key={entry.id}
                      entry={entry}
                      usedBy={data.connections.find((c) => c.id === entry.connection_id)?.used_by.map((b) => b.name) ?? []}
                      onAddLink={onAddComposio}
                      onOpenTools={onChooseComposioBots ? undefined : links.onOpenTools}
                      onChooseBots={onChooseComposioBots}
                      onChanged={onRefresh}
                      phone={phone ?? phoneLayout}
                      openRequest={composioRequest}
                      api={composioTileApi}
                      initialOpen={composioOpen && !entry.added ? 'add' : undefined}
                    />
                  ) : (
                  <ComposioCard
                    key={entry.id}
                    entry={entry}
                    initialOpen={composioOpen}
                    onAdd={onAddComposio}
                    onOpenTools={links.onOpenTools}
                    onChooseBots={entry.id === 'composio' ? onChooseComposioBots : undefined}
                    openRequest={entry.id === 'composio' ? composioRequest : 0}
                  />
                  ),
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
export interface ConnectionsPageProps extends ManageLinks {
  onBack?: () => void;
  embedded?: boolean;
  /** 0.9.0 P5: open on one note ("See the note" in a chat). */
  focusNote?: string | null;
  onClearFocusNote?: () => void;
  onOpenNoteWhere?: ToolNotesSectionProps['onOpenWhere'];
  hideAccounts?: boolean;
  onChooseComposioBots?: () => void;
}

export const ConnectionsPage: React.FC<ConnectionsPageProps> = ({ onBack, embedded, focusNote, onClearFocusNote, onOpenNoteWhere, hideAccounts, onChooseComposioBots, ...links }) => {
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

  // A note changed: recount the row chips without the page's loading state.
  const quietLoad = useCallback(() => {
    connectionsApi.list().then((r) => alive.current && setData(r), () => undefined);
  }, []);

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

  // Dev mode D10: the open space's production marks. Shown only while Dev mode is on there.
  const devOn = useDevModeOn();
  const [marking, setMarking] = useState<string | null>(null);
  const markProduction = async (c: Connection, production: boolean) => {
    const space = devStore.current();
    if (!space || marking) return;
    setMarking(c.id);
    setNotice(null);
    try {
      await devApi.production(c.id, space, production);
      if (alive.current) setData((d) => (d ? { ...d, connections: d.connections.map((x) => (x.id === c.id ? { ...x, production } : x)) } : d));
    } catch (e) {
      if (alive.current) setNotice({ text: `Couldn’t change ${c.name}: ${e instanceof Error ? e.message : 'something went wrong.'}`, error: true });
    } finally {
      if (alive.current) setMarking(null);
    }
  };

  const addComposio = async (url: string, apiKey: string, enabled: boolean): Promise<string | null> => {
    try {
      await connectionsApi.addComposio(apiKey ? { url, api_key: apiKey, enabled } : { url, enabled });
      setNotice({
        text: enabled
          ? 'Composio is added and on.'
          : onChooseComposioBots
            ? 'Composio is added. Choose the bots that get it to turn it on.'
            : 'Composio is added and off. Turn it on in Tools when you want bots to use it.',
        error: false,
      });
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
        embedded={embedded}
        onProduction={devOn ? markProduction : undefined}
        marking={marking}
        focusNote={focusNote}
        onClearFocusNote={onClearFocusNote}
        onOpenNoteWhere={onOpenNoteWhere}
        onNotesChanged={quietLoad}
        hideAccounts={hideAccounts}
        onChooseComposioBots={onChooseComposioBots}
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
