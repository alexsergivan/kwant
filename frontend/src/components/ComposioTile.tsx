// The Composio tile (composio-simple/design.md §3): status and actions only; Find a tool does the
// searching. One key field adds it, Replace key and Switch fix it in place, and Remove asks inline.
// Every error sits next to the control that fixes it. The main path never says MCP, header or URL;
// only "Use a Composio link instead (advanced)" does.

import React, { useEffect, useId, useRef, useState } from 'react';
import { ChevronDown, Info, KeyRound, Loader2, Lock, Plus } from 'lucide-react';
import { CatalogEntry, ConnectionsError, connectionsApi } from '../connections';
import { listNames, removeServer, serverIdOf, setServerEnabled } from '../appsAccounts';
import { editorProps } from '../escape';
import { KeyField, KeyPhase } from './FindTool';

export interface ComposioTileApi {
  addKey: (key: string) => Promise<unknown>;
  replaceKey: (key: string) => Promise<unknown>;
  switchTo: (key?: string) => Promise<unknown>;
  setEnabled: (serverId: string, enabled: boolean) => Promise<void>;
  remove: (serverId: string) => Promise<void>;
}

export const defaultComposioTileApi: ComposioTileApi = {
  addKey: (k) => connectionsApi.addComposioKey(k),
  replaceKey: (k) => connectionsApi.replaceComposioKey(k),
  switchTo: (k) => connectionsApi.switchComposio(k),
  setEnabled: (id, on) => setServerEnabled(id, on),
  remove: (id) => removeServer(id),
};

type Open = 'none' | 'add' | 'replace' | 'remove' | 'switchKey';

export interface ComposioTileProps {
  entry: CatalogEntry;
  /** Names of the bots that have Composio. */
  usedBy: string[];
  /** Apps & accounts: picking bots turns it on (the Connect sheet). */
  onChooseBots?: () => void;
  /** The old Connections page: on and off live in Tools. */
  onOpenTools?: () => void;
  /** The old link form ("Use a Composio link instead"): an error to show, or null. */
  onAddLink: (url: string, apiKey: string, enabled: boolean) => Promise<string | null>;
  /** Load the page again after a change. */
  onChanged?: () => void;
  api?: ComposioTileApi;
  phone?: boolean;
  /** Tests and shots: start with this open. */
  initialOpen?: Open;
  /** Goes up by one each time the page asks for Composio: scroll here (and open the key field if not added). */
  openRequest?: number;
}

const KEY_DEFAULT = 'https://dashboard.composio.dev';

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[30px] px-3 py-1 rounded-lg text-xs font-medium whitespace-nowrap transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400 motion-reduce:transition-none';
const quietBtn = `${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const primaryBtn = `${btn} border border-sky-600 bg-sky-600 text-white hover:bg-sky-500`;
const roseBtn = `${btn} border border-zinc-700 bg-transparent text-rose-300 hover:bg-rose-500/10 hover:border-rose-500/40`;
const roseFill = `${btn} border border-rose-500 bg-rose-600 text-white hover:bg-rose-500`;
const field =
  'w-full min-h-[40px] px-3 py-1.5 bg-zinc-950 border border-zinc-700 rounded-[10px] text-sm text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-400 focus:ring-[3px] focus:ring-sky-400/25';

/** "Ann and Ben", or "Ann, Ben and 3 more" from three names on. */
export function shortList(names: string[], joiner = 'and'): string {
  if (names.length <= 2) return joiner === 'and' ? listNames(names) : names.join(` ${joiner} `);
  return `${names[0]}, ${names[1]} and ${names.length - 2} more`;
}

/** The Remove question, with who and what it takes away. */
export function removeQuestion(bots: string[], apps: string[]): string {
  const lose = bots.length && apps.length ? ` ${shortList(bots)} ${bots.length === 1 ? 'loses' : 'lose'} ${shortList(apps)}.` : bots.length ? ` ${shortList(bots)} ${bots.length === 1 ? 'loses' : 'lose'} it.` : '';
  return `Remove Composio?${lose} Your sign-ins stay on Composio, so you can add it back later.`;
}

export const ComposioTile: React.FC<ComposioTileProps> = (p) => {
  const { entry, usedBy } = p;
  const api = p.api ?? defaultComposioTileApi;
  const id = useId().replace(/:/g, '');
  const cardRef = useRef<HTMLLIElement>(null);
  const keyRef = useRef<HTMLInputElement>(null);
  const keepRef = useRef<HTMLButtonElement>(null);

  const refused = entry.added && entry.problem?.action === 'replace_key';
  const down = entry.added && entry.problem?.action === 'try_again';
  const urlMode = entry.added && entry.mode === 'url';
  const keyUrl = entry.key_url && /^https:\/\//.test(entry.key_url) ? entry.key_url : KEY_DEFAULT;
  const apps = (entry.apps_connected ?? []).map((a) => a.name);
  const serverId = serverIdOf(entry.connection_id);

  const [open, setOpen] = useState<Open>(p.initialOpen ?? 'none');
  const [phase, setPhase] = useState<KeyPhase>({ s: 'idle' });
  const [busy, setBusy] = useState<null | 'off' | 'remove' | 'switch'>(null);
  const [actionError, setActionError] = useState<{ at: 'off' | 'remove' | 'switch'; text: string } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [linkOpen, setLinkOpen] = useState(false);
  const [url, setUrl] = useState('');
  const [linkKey, setLinkKey] = useState('');
  const [linkError, setLinkError] = useState<string | null>(null);
  const [linkSaving, setLinkSaving] = useState(false);

  useEffect(() => {
    if (!p.openRequest) return;
    cardRef.current?.scrollIntoView?.({ behavior: 'smooth', block: 'start' });
    if (!entry.added) openKey('add');
    else cardRef.current?.querySelector<HTMLElement>('button')?.focus({ preventScroll: true });
  }, [p.openRequest]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (open === 'remove') keepRef.current?.focus();
  }, [open]);

  const openKey = (what: Open) => {
    setOpen(what);
    setPhase({ s: 'idle' });
    setActionError(null);
    window.setTimeout(() => keyRef.current?.focus(), 0);
  };
  const close = () => {
    setOpen('none');
    setPhase({ s: 'idle' });
  };

  const submitKey = async (key: string) => {
    setPhase({ s: 'checking' });
    try {
      if (open === 'add') await api.addKey(key);
      else if (open === 'switchKey') await api.switchTo(key);
      else await api.replaceKey(key);
    } catch (e) {
      const text = e instanceof Error ? e.message : 'Something went wrong.';
      const isRefused = e instanceof ConnectionsError && (e.status === 409 || e.status === 400);
      setPhase(isRefused ? { s: 'refused', text } : { s: 'down', text });
      window.setTimeout(() => keyRef.current?.focus(), 0);
      return;
    }
    const was = open;
    close();
    setNotice(
      was === 'add'
        ? 'Composio is added. No bot has it until you choose bots.'
        : was === 'switchKey'
          ? switchedLine(apps)
          : 'Key saved.',
    );
    p.onChanged?.();
  };

  const doSwitch = async () => {
    setBusy('switch');
    setActionError(null);
    try {
      await api.switchTo();
      setNotice(switchedLine(apps));
      p.onChanged?.();
    } catch (e) {
      // No saved key to switch with: ask for one, right here.
      if (e instanceof ConnectionsError && (e.status === 400 || e.status === 409 || e.status === 422)) openKey('switchKey');
      else setActionError({ at: 'switch', text: e instanceof Error ? e.message : 'Couldn’t switch. Try again in a minute.' });
    } finally {
      setBusy(null);
    }
  };

  const turnOff = async () => {
    if (!serverId) return;
    setBusy('off');
    setActionError(null);
    try {
      await api.setEnabled(serverId, false);
      setNotice('Composio is off. No bot can use it until you turn it on again.');
      p.onChanged?.();
    } catch (e) {
      setActionError({ at: 'off', text: e instanceof Error ? e.message : 'Couldn’t turn it off.' });
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!serverId) return;
    setBusy('remove');
    setActionError(null);
    try {
      await api.remove(serverId);
      setOpen('none');
      setNotice(null);
      p.onChanged?.();
    } catch (e) {
      setActionError({ at: 'remove', text: e instanceof Error ? e.message : 'Couldn’t remove it.' });
    } finally {
      setBusy(null);
    }
  };

  const submitLink = async (e: React.FormEvent) => {
    e.preventDefault();
    setLinkSaving(true);
    setLinkError(null);
    const problem = await p.onAddLink(url.trim(), linkKey.trim(), false);
    setLinkSaving(false);
    if (problem) setLinkError(problem);
    else (setLinkOpen(false), setUrl(''), setLinkKey(''), close());
  };

  // ── Pieces ──
  const chip = refused ? (
    <span className="px-1.5 py-px rounded-md border border-rose-500/40 bg-rose-500/10 text-[11px] text-rose-200" data-composio-chip="refused">
      Key not accepted
    </span>
  ) : entry.added ? (
    <span
      className={`px-1.5 py-px rounded-md border text-[11px] ${entry.enabled ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200' : 'border-zinc-700 text-zinc-300'}`}
      data-composio-chip={entry.enabled ? 'on' : 'off'}
    >
      {entry.enabled ? 'Added · on' : 'Added · off'}
    </span>
  ) : null;

  const fact = (k: string, v: React.ReactNode, attr: string) => (
    <div className={p.phone ? 'space-y-0.5' : 'grid grid-cols-[120px_1fr] gap-3'} data-fact={attr}>
      <dt className="text-xs text-zinc-400">{k}</dt>
      <dd className="text-xs text-zinc-200 min-w-0">{v}</dd>
    </div>
  );

  const keyField = (what: Open) => {
    const replace = what === 'replace' || what === 'switchKey' || refused;
    return (
      <KeyField
        id={`${id}-key`}
        app={null}
        replace={replace && what !== 'switchKey'}
        keyUrl={keyUrl}
        phone={!!p.phone}
        phase={phase}
        inputRef={keyRef}
        onSubmit={submitKey}
        onCancel={close}
        noCancel={refused && what === 'replace'}
        label={what === 'switchKey' ? 'Your Composio API key' : undefined}
        action={what === 'switchKey' ? 'Switch' : undefined}
        helper={
          what === 'add'
            ? undefined
            : what === 'switchKey'
              ? 'Free plan: 10,000 tool calls a month. Kept on this computer and never shown again.'
              : refused
                ? 'From your Composio dashboard.'
                : apps.length
                  ? `A key from another Composio project starts fresh there, so you’d connect ${shortList(apps)} again.`
                  : undefined
        }
      />
    );
  };

  const errorNext = (at: 'off' | 'remove' | 'switch') =>
    actionError?.at === at ? (
      <p role="alert" className="text-xs text-rose-300" data-tile-error={at}>
        {actionError.text}
      </p>
    ) : null;

  const actionsGrid = p.phone ? 'grid grid-cols-2 gap-2' : 'flex flex-wrap items-center gap-2';
  const span2 = p.phone ? 'col-span-2' : '';
  const spin = <Loader2 size={12} className="animate-spin motion-reduce:hidden" aria-hidden="true" />;

  const chooseBots = (primary: boolean) =>
    p.onChooseBots ? (
      <button type="button" className={primary ? primaryBtn : quietBtn} onClick={p.onChooseBots} data-composio-choose-bots>
        {entry.enabled ? 'Choose bots' : 'Choose bots to turn it on'}
      </button>
    ) : p.onOpenTools ? (
      <button type="button" className={quietBtn} onClick={p.onOpenTools}>
        {entry.enabled ? 'Manage in Tools' : 'Turn on in Tools'}
      </button>
    ) : null;
  const replaceBtn = (
    <button type="button" className={quietBtn} onClick={() => openKey('replace')} data-composio-replace>
      <KeyRound size={12} aria-hidden="true" /> Replace key
    </button>
  );
  const offBtn =
    entry.enabled && serverId ? (
      <button type="button" className={quietBtn} onClick={turnOff} disabled={busy === 'off'} data-composio-off>
        {busy === 'off' && spin} Turn off
      </button>
    ) : null;
  const removeBtn = serverId ? (
    <button type="button" className={roseBtn} onClick={() => (setOpen('remove'), setActionError(null))} data-composio-remove>
      Remove
    </button>
  ) : null;

  let body: React.ReactNode;
  if (!entry.added) {
    body =
      open === 'add' ? (
        <div className="space-y-3">
          {keyField('add')}
          <div>
            <button
              type="button"
              className="inline-flex items-center gap-1 min-h-[44px] sm:min-h-[28px] text-xs text-zinc-400 hover:text-zinc-200 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
              aria-expanded={linkOpen}
              onClick={() => setLinkOpen((v) => !v)}
              data-composio-link-toggle
            >
              <ChevronDown size={12} className={`transition-transform motion-reduce:transition-none ${linkOpen ? 'rotate-180' : ''}`} aria-hidden="true" />
              Use a Composio link instead (advanced)
            </button>
            {linkOpen && (
              <form onSubmit={submitLink} {...editorProps(!!(url.trim() || linkKey.trim()))} className="mt-2 space-y-3" aria-label="Add Composio with a link" noValidate data-composio-link-form>
                <div className="space-y-1">
                  <label htmlFor={`${id}-url`} className="block text-xs font-medium text-zinc-200">
                    Composio MCP URL
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
                    aria-invalid={linkError ? true : undefined}
                    aria-describedby={`${id}-url-h${linkError ? ` ${id}-url-err` : ''}`}
                  />
                  <p id={`${id}-url-h`} className="text-[11px] text-zinc-400">
                    From your Composio dashboard. It has to be an https address on composio.dev.
                  </p>
                </div>
                <div className="space-y-1">
                  <label htmlFor={`${id}-lkey`} className="block text-xs font-medium text-zinc-200">
                    API key <span className="font-normal text-zinc-400">(optional)</span>
                  </label>
                  <input id={`${id}-lkey`} className={field} type="password" autoComplete="off" value={linkKey} onChange={(e) => setLinkKey(e.target.value)} />
                </div>
                {linkError && (
                  <p id={`${id}-url-err`} role="alert" className="px-3 py-2 rounded-lg border border-rose-500/40 bg-rose-500/10 text-xs text-rose-200">
                    {linkError}
                  </p>
                )}
                <button type="submit" className={quietBtn} disabled={linkSaving || !url.trim()}>
                  {linkSaving && spin} Add with this link
                </button>
              </form>
            )}
          </div>
        </div>
      ) : (
        <div className={actionsGrid}>
          <button type="button" className={`${primaryBtn} ${span2}`} onClick={() => openKey('add')} data-composio-add>
            <Plus size={12} aria-hidden="true" /> Add Composio
          </button>
        </div>
      );
  } else if (open === 'remove') {
    body = (
      <div role="alert" className="px-3 py-2.5 rounded-xl border border-rose-500/40 bg-rose-500/10 space-y-2.5" data-composio-remove-ask>
        <p className="text-xs text-rose-100">{removeQuestion(usedBy, apps)}</p>
        <div className={actionsGrid}>
          <button type="button" className={roseFill} onClick={remove} disabled={busy === 'remove'} data-composio-remove-yes>
            {busy === 'remove' && spin} Remove Composio
          </button>
          <button ref={keepRef} type="button" className={quietBtn} onClick={() => setOpen('none')} data-composio-keep>
            Keep it
          </button>
        </div>
        {errorNext('remove')}
      </div>
    );
  } else if (refused) {
    const who = usedBy.length ? shortList(usedBy) : 'your bots';
    const what = apps.length ? shortList(apps, 'or') : 'its apps';
    body = (
      <div className="space-y-3">
        <p role="alert" className="px-3 py-2 rounded-lg border border-rose-500/40 bg-rose-500/10 text-xs text-rose-200" data-composio-refused>
          Composio didn’t accept the saved key, so {who} can’t use {what} right now. Paste a new key to fix it.
        </p>
        {keyField('replace')}
        <div className={actionsGrid}>
          {offBtn}
          {removeBtn}
        </div>
        {errorNext('off')}
      </div>
    );
  } else if (open === 'replace' || open === 'switchKey') {
    body = (
      <div className="space-y-3">
        {open === 'switchKey' && (
          <p className="flex items-start gap-2 px-3 py-2 rounded-lg border border-violet-400/30 bg-violet-500/10 text-xs text-violet-100" data-composio-switch-needs-key>
            <Info size={14} className="mt-px shrink-0" aria-hidden="true" />
            The one-key setup needs your Composio API key. The apps you already connected stay connected.
          </p>
        )}
        {keyField(open)}
      </div>
    );
  } else {
    body = (
      <div className="space-y-2">
        {urlMode && entry.can_switch && (
          <div className="px-3 py-2.5 rounded-xl border border-violet-400/30 bg-violet-500/10 space-y-2" data-composio-switch>
            <p className="flex items-start gap-2 text-xs text-violet-100">
              <Info size={14} className="mt-px shrink-0" aria-hidden="true" />
              Switch to the one-key setup to connect apps right here. Your bots then find any app by themselves, and the apps you already connected stay connected.
            </p>
            <div className={actionsGrid}>
              <button type="button" className={`${primaryBtn} ${span2}`} onClick={doSwitch} disabled={busy === 'switch'} data-composio-switch-go>
                {busy === 'switch' && spin} Switch to the one-key setup
              </button>
            </div>
            {errorNext('switch')}
          </div>
        )}
        <div className={actionsGrid}>
          {chooseBots(!entry.enabled || !usedBy.length)}
          {!urlMode && replaceBtn}
          {offBtn}
          {removeBtn}
        </div>
        {errorNext('off')}
      </div>
    );
  }

  return (
    <li ref={cardRef} className="rounded-xl border border-zinc-800 bg-[#181335] p-4 space-y-3 scroll-mt-4" aria-labelledby={`${id}-n`} data-catalog={entry.id} data-composio-tile={open !== 'none' ? open : refused ? 'refused' : urlMode ? 'url' : entry.added ? (entry.enabled ? 'on' : 'off') : 'new'}>
      <div className="space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <h4 id={`${id}-n`} className="text-[15px] font-semibold text-zinc-100">
            Composio
          </h4>
          {chip}
        </div>
        <p className="text-xs text-zinc-300">Gmail and hundreds more apps. Your bots find the app they need by themselves.</p>
      </div>

      {entry.added && (
        <dl className="space-y-1.5">
          {urlMode
            ? fact('Set up with', 'a Composio link', 'setup')
            : fact(
                'Apps connected',
                down ? (
                  <span className="flex flex-wrap items-center gap-x-2">
                    Couldn’t reach Composio to list them.
                    <button type="button" className="inline-flex items-center min-h-[44px] sm:min-h-[20px] text-xs font-medium text-sky-300 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400" onClick={() => p.onChanged?.()} data-composio-retry>
                      Try again
                    </button>
                  </span>
                ) : apps.length ? (
                  shortList(apps)
                ) : (
                  <span className="text-zinc-300">None yet. Find one with Find a tool above.</span>
                ),
                'apps',
              )}
          {fact('Used by', usedBy.length ? shortList(usedBy) : <span className="text-zinc-300">No bot yet</span>, 'used-by')}
        </dl>
      )}

      {notice && (
        <p role="status" className="px-3 py-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 text-xs text-emerald-100" data-composio-notice>
          {notice}
        </p>
      )}

      {body}

      <p className="flex items-start gap-2 text-[11px] text-zinc-400" data-composio-footer>
        <Lock size={12} className="mt-px shrink-0" aria-hidden="true" />
        Your app sign-ins stay on Composio’s servers, not in kwant’s vault.
      </p>
    </li>
  );
};

const switchedLine = (apps: string[]) =>
  apps.length ? `Switched to the one-key setup. ${shortList(apps)} ${apps.length === 1 ? 'is' : 'are'} still connected.` : 'Switched to the one-key setup.';
