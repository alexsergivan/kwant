import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, CheckCircle2, ExternalLink, Eye, EyeOff, House, Loader2, Lock, Plug, WifiOff, X } from 'lucide-react';
import { useEscapeLayer } from '../escape';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { ModernAvatar, renderCreature, useCreatureFor } from './ModernAvatar';
import { Sticker } from './Sticker';
import { markNewSeen } from '../whatsNew';
import { SECRET_MASK, loadStatuses, saveStatuses, toStoredStatus, type McpServer, type McpTestResult } from '../mcp';
import { HA_CHANGED_EVENT, actionsText, findHaServer, freeHaName, haResultTitle, mapHaTest, normaliseHaAddress, type HaResult } from '../homeAssistant';
import { useReadyTeams } from './ReadyTeams';
import type { Mood } from '../kwantMood';

// Connect Home Assistant (26e, design.md §1): an address and an access token, nothing else. kwant
// builds `<origin>/api/mcp` and the Bearer header, saves the server in Tools, tests it and says
// what happened in plain words. From the Home team it also hands the server to the team's bots.

export interface HomeAssistantRequest {
  /** Where it was opened: Tools (Add a tool / Fix) or the Home team's needs card. */
  door: 'tools' | 'team';
  teamId?: string;
  /** Fix mode: the Home Assistant server already in Tools. Cancel never deletes it. */
  serverId?: string;
}

const HA_EVENT = 'kwant:connect-home-assistant';

export function openHomeAssistant(req: HomeAssistantRequest): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new window.CustomEvent<HomeAssistantRequest>(HA_EVENT, { detail: req }));
}

const readError = async (res: Response) => {
  try {
    const body = await res.json();
    if (body?.error) return String(body.error);
  } catch {
    // not JSON
  }
  return `Something went wrong (HTTP ${res.status}).`;
};

// ── Look (the Add a tool dialog's classes) ───────────────────────────────────────────────────────

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3.5 py-1 rounded-lg text-[13px] font-medium whitespace-nowrap transition-colors disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-900 motion-reduce:transition-none';
const quietBtn = `${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const primaryBtn = `${btn} border border-sky-600 bg-sky-600 text-white hover:bg-sky-500 disabled:hover:bg-sky-600`;
const linkCls =
  'inline-flex items-center gap-1 min-h-[44px] sm:min-h-0 text-xs font-medium text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 aria-disabled:text-zinc-500 aria-disabled:no-underline aria-disabled:pointer-events-none';
const inputCls = (phone: boolean, bad: boolean) =>
  `w-full ${phone ? 'h-11 text-base' : 'h-10 text-[13px]'} px-3 rounded-[10px] bg-zinc-950 border text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-400 focus:ring-[3px] focus:ring-sky-400/25 read-only:opacity-70 ${
    bad ? 'border-rose-400' : 'border-zinc-700'
  }`;

const Spinner: React.FC = () => (
  <>
    <Loader2 size={14} className="shrink-0 animate-spin motion-reduce:hidden" style={{ animationDuration: '700ms' }} aria-hidden="true" />
    <span className="hidden motion-reduce:inline-block w-1.5 h-1.5 rounded-full bg-zinc-400" aria-hidden="true" />
  </>
);

/** House peeking up from the empty strip's bottom edge: the creature alone, no frame, lower rows cut. */
export const HousePeek: React.FC = () => {
  const c = useCreatureFor('home', 'House');
  return (
    <svg
      viewBox="5 1.5 16 10.5"
      width={56}
      height={37}
      className="shrink-0 self-end"
      style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
      aria-hidden="true"
      data-house-peek
    >
      {renderCreature(c).filter((r) => r.key !== 'shadow')}
    </svg>
  );
};

type Phase = { s: 'edit' } | { s: 'testing' } | { s: 'done'; r: HaResult };

const MOOD: Record<Phase['s'] | 'bad', Mood | undefined> = { edit: undefined, testing: 'working', done: 'finished', bad: 'failed' };

export interface HomeAssistantSheetProps {
  req: HomeAssistantRequest;
  onClose: () => void;
  /** The bots in the team this was opened from; they get the server once it tests OK. */
  teamKwantIds?: string[];
  /** Hides "Start a Home team" when one exists already. */
  homeTeamExists?: boolean;
  onStartHomeTeam?: () => void;
  /** For tests: the clock and the network. */
  fetcher?: typeof fetch;
}

export const HomeAssistantSheet: React.FC<HomeAssistantSheetProps> = ({ req, onClose, teamKwantIds = [], homeTeamExists, onStartHomeTeam, fetcher }) => {
  const f = fetcher ?? ((...a: Parameters<typeof fetch>) => fetch(...a));
  const phone = useMedia(PHONE_QUERY);
  const root = useRef<HTMLDivElement>(null);
  const addressRef = useRef<HTMLInputElement>(null);
  const tokenRef = useRef<HTMLInputElement>(null);
  const titleId = React.useId();
  const addrId = React.useId();
  const tokenId = React.useId();
  const [address, setAddress] = useState('');
  const [addrTouched, setAddrTouched] = useState(false);
  const [token, setToken] = useState('');
  const [tokenSaved, setTokenSaved] = useState(false);
  /** A token whose save failed: kept in memory for Try again, never in the field (QA D1). */
  const heldToken = useRef('');
  const [tokenHeld, setTokenHeld] = useState(false);
  const [shown, setShown] = useState(false);
  const [phase, setPhase] = useState<Phase>({ s: 'edit' });
  /** The server this sheet saved (or is fixing); Try again updates it. */
  const [serverId, setServerId] = useState<string | null>(req.serverId ?? null);
  const createdHere = useRef(false);

  const parsed = normaliseHaAddress(address);
  const testing = phase.s === 'testing';
  const result = phase.s === 'done' ? phase.r : null;
  const ok = result?.kind === 'ok';
  const canConnect = !!parsed && (token.trim() !== '' || tokenSaved || tokenHeld) && !testing;

  useEffect(() => markNewSeen('home-assistant'), []);

  // Fix (or a second visit): start from the saved server's address; the token stays saved unless
  // a new one is pasted. One Home Assistant in Tools, never a copy per visit.
  const [fixing, setFixing] = useState(!!req.serverId);
  useEffect(() => {
    let live = true;
    f('/api/mcp/servers')
      .then((r) => (r.ok ? (r.json() as Promise<McpServer[]>) : []))
      .then((list) => {
        const s = req.serverId ? list.find((x) => x.id === req.serverId) : findHaServer(list);
        if (!live || !s || !Array.isArray(list)) return;
        setFixing(true);
        setServerId(s.id);
        setAddress((s.url ?? '').replace(/\/api\/mcp\/?$/, ''));
        setTokenSaved(Object.keys(s.headers ?? {}).some((k) => k.toLowerCase() === 'authorization'));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [req.serverId]);

  useEffect(() => {
    if (!phone) addressRef.current?.focus();
  }, [phone]);

  /** Leaving after a failed try removes the server this sheet made, so Tools keeps no broken entry. */
  const close = () => {
    if (testing) return;
    if (createdHere.current && serverId && !ok) {
      void f(`/api/mcp/servers/${serverId}`, { method: 'DELETE' })
        .catch(() => null)
        .then(() => window.dispatchEvent(new window.Event(HA_CHANGED_EVENT)));
    }
    onClose();
  };
  useEscapeLayer(root, true, close, { modal: true });

  const connect = async () => {
    if (!parsed || !canConnect) return;
    setPhase({ s: 'testing' });
    // A pasted token wins; else one held from a failed save; else the saved one (the mask tells the
    // server to keep the header it has).
    const typed = token.trim().replace(/^Bearer\s+/i, '') || heldToken.current;
    const headers = { Authorization: typed ? `Bearer ${typed}` : SECRET_MASK };
    // Every save attempt clears the field, so the token never stays in the page (QA D1).
    setToken('');
    setShown(false);
    const payload = { transport: 'http', command: null, args: [], url: parsed.url, env: {}, headers, enabled: true, timeout_secs: 30 };
    let id = serverId;
    try {
      if (id) {
        const list = await f('/api/mcp/servers').then((r) => (r.ok ? (r.json() as Promise<McpServer[]>) : []));
        const name = list.find((s) => s.id === id)?.name ?? freeHaName(list.map((s) => s.name));
        const res = await f(`/api/mcp/servers/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...payload, name }) });
        if (!res.ok) throw new Error(await readError(res));
      } else {
        const list = await f('/api/mcp/servers').then((r) => (r.ok ? (r.json() as Promise<McpServer[]>) : []));
        const res = await f('/api/mcp/servers', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ ...payload, name: freeHaName(list.map((s) => s.name)) }),
        });
        if (!res.ok) throw new Error(await readError(res));
        const saved: McpServer = await res.json();
        id = saved.id;
        createdHere.current = true;
        setServerId(id);
      }
      heldToken.current = '';
      setTokenHeld(false);
      setTokenSaved(true);
    } catch (e) {
      if (typed) {
        heldToken.current = typed;
        setTokenHeld(true);
      }
      setPhase({ s: 'done', r: { kind: 'other', error: e instanceof Error ? e.message : 'Couldn’t save it in Tools.' } });
      return;
    }
    let test: McpTestResult;
    try {
      const res = await f(`/api/mcp/servers/${id}/test`, { method: 'POST' });
      test = res.ok ? await res.json() : { ok: false, error: await readError(res) };
    } catch {
      test = { ok: false, error: 'The check didn’t finish. Try again.' };
    }
    const statuses = loadStatuses();
    statuses[id!] = toStoredStatus(test, new Date().toISOString());
    saveStatuses(statuses);
    const r = mapHaTest(test);
    if (r.kind === 'ok') {
      if (req.door === 'team') await giveToTeam(id!);
    }
    setPhase({ s: 'done', r });
    window.dispatchEvent(new window.Event(HA_CHANGED_EVENT));
  };

  /** Adds the server to each bot in the team, keeping the tools they already have. */
  const giveToTeam = async (id: string) => {
    for (const k of teamKwantIds) {
      try {
        const cur = await f(`/api/kwants/${k}/mcp-servers`).then((r) => (r.ok ? (r.json() as Promise<{ server_ids?: string[] }>) : null));
        if (!cur) continue;
        const ids = cur.server_ids ?? [];
        if (ids.includes(id)) continue;
        await f(`/api/kwants/${k}/mcp-servers`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ server_ids: [...ids, id] }) });
      } catch {
        // The server is in Tools either way; the bot's settings can still turn it on.
      }
    }
  };

  const bad = result && result.kind !== 'ok';
  const mood = MOOD[bad ? 'bad' : phase.s];
  const moodText = testing ? 'House is checking Home Assistant' : ok ? 'House, connected' : bad ? 'House, connection failed' : 'House';
  const addrError = addrTouched && address.trim() !== '' && !parsed;
  const showEmpty = !fixing && address.trim() === '' && phase.s === 'edit';

  return (
    <div className={`fixed inset-0 z-[70] ${phone ? '' : 'grid place-items-center p-6'} bg-[rgb(6_3_20/.62)]`} onClick={close} data-ha-sheet-scrim>
      <div
        ref={root}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        data-ha-sheet={phase.s === 'done' ? phase.r.kind : phase.s}
        data-door={req.door}
        onClick={(e) => e.stopPropagation()}
        className={`[--kw-avatar-cut:theme(colors.zinc.900)] ${
          phone
            ? 'absolute inset-x-0 bottom-0 max-h-[92%] overflow-y-auto rounded-t-2xl bg-zinc-900 border-t border-zinc-800 px-4 pt-3 pb-[max(16px,env(safe-area-inset-bottom))] outline-none animate-[t-sheet_200ms_ease-out] motion-reduce:animate-none'
            : 'w-[480px] max-w-[calc(100vw-32px)] max-h-[calc(100vh-48px)] overflow-y-auto rounded-2xl bg-zinc-900 border border-zinc-800 p-5 shadow-[0_25px_50px_-12px_rgb(0_0_0/.6)] outline-none animate-[t-rise_180ms_ease-out] motion-reduce:animate-none'
        }`}
      >
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (ok) close();
            else void connect();
          }}
        >
          <div className="flex items-start gap-3 mb-4">
            {req.door === 'team' ? (
              <ModernAvatar role="home" name="House" size={32} mood={mood} moodText={moodText} modelBadge={false} />
            ) : (
              <span aria-hidden="true" className="w-8 h-8 rounded-[9px] shrink-0 grid place-items-center bg-sky-400/15 text-sky-400 ring-1 ring-inset ring-sky-300/20">
                <House size={16} />
              </span>
            )}
            <div className="flex-1 min-w-0">
              <h3 id={titleId} className="text-[15px] font-semibold text-zinc-50 leading-8">
                Connect Home Assistant
              </h3>
              <p className="text-xs text-zinc-400 leading-relaxed">Your bots can then see and control the devices you share.</p>
            </div>
            <button
              type="button"
              aria-label="Close"
              onClick={close}
              disabled={testing}
              className={`${phone ? 'w-11 h-11' : 'w-8 h-8'} shrink-0 grid place-items-center rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-white/5 disabled:opacity-40 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400`}
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>

          {showEmpty && (
            <div role="note" className="mb-4 flex items-end gap-3 px-3 pt-2 rounded-[10px] border border-dashed border-zinc-700 overflow-hidden" data-ha-empty>
              <HousePeek />
              <div className="flex-1 min-w-0 pb-2.5">
                <p className="text-[13px] font-medium text-zinc-100">Nothing connected yet</p>
                <p className="text-xs text-zinc-400 leading-relaxed">Add the address and an access token below, then press Connect.</p>
              </div>
            </div>
          )}

          <div className="space-y-4">
            <div className="space-y-1.5">
              <label htmlFor={addrId} className="block text-xs font-medium text-zinc-200">
                Address
              </label>
              <input
                ref={addressRef}
                id={addrId}
                value={address}
                readOnly={testing}
                onChange={(e) => setAddress(e.target.value)}
                onBlur={() => setAddrTouched(true)}
                placeholder="homeassistant.local:8123"
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
                inputMode="url"
                aria-invalid={addrError || result?.kind === 'unreachable' || undefined}
                aria-describedby={`${addrId}-help`}
                className={inputCls(phone, addrError || result?.kind === 'unreachable')}
                data-ha-address
              />
              <p id={`${addrId}-help`} className={`text-[11px] leading-relaxed ${addrError ? 'text-rose-300' : 'text-zinc-400'}`} data-ha-address-help>
                {addrError ? (
                  'That doesn’t look like an address. Try homeassistant.local:8123.'
                ) : parsed ? (
                  <>
                    kwant will connect to <span className="font-mono text-zinc-300 break-all">{parsed.url}</span>
                  </>
                ) : (
                  <>
                    The address you use to open Home Assistant, like <b className="font-medium text-zinc-300">homeassistant.local:8123</b> or{' '}
                    <b className="font-medium text-zinc-300">http://192.168.1.20:8123</b>.
                  </>
                )}
              </p>
            </div>

            <div className="space-y-1.5">
              <div className="flex items-center justify-between gap-2">
                <label htmlFor={tokenId} className="text-xs font-medium text-zinc-200">
                  Access token
                </label>
                <a
                  href={parsed ? `${parsed.origin}/profile/security` : undefined}
                  target="_blank"
                  rel="noreferrer"
                  aria-disabled={!parsed || undefined}
                  className={linkCls}
                  data-ha-token-link
                >
                  Make one in Home Assistant <ExternalLink size={12} aria-hidden="true" />
                  <span className="sr-only"> (opens a new tab)</span>
                </a>
              </div>
              <div className="relative">
                <input
                  ref={tokenRef}
                  id={tokenId}
                  type={shown ? 'text' : 'password'}
                  value={token}
                  readOnly={testing}
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="Paste the access token"
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={result?.kind === 'bad_token' || undefined}
                  aria-describedby={`${tokenId}-help`}
                  className={`${inputCls(phone, result?.kind === 'bad_token')} pr-11`}
                  data-ha-token
                />
                <button
                  type="button"
                  onClick={() => setShown((v) => !v)}
                  aria-label={shown ? 'Hide access token' : 'Show access token'}
                  aria-pressed={shown}
                  className="absolute right-0 top-0 h-full w-11 inline-flex items-center justify-center text-zinc-400 hover:text-zinc-200 rounded-r-[10px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
                >
                  {shown ? <EyeOff size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}
                </button>
              </div>
              <p id={`${tokenId}-help`} className="text-[11px] leading-relaxed text-zinc-400">
                {tokenSaved
                  ? 'Saved. Paste a new one to replace it. Leaving it empty keeps the saved one.'
                  : tokenHeld
                    ? 'Kept for the next try. Paste a new one to replace it.'
                    : 'A long code that lets kwant into your Home Assistant. Make it in your profile › Security › Long-lived access tokens › Create token.'}
              </p>
            </div>

            <ul className="space-y-1.5 text-[11px] leading-relaxed text-zinc-400" data-ha-notes>
              <li className="flex items-start gap-2">
                <Eye size={13} className="mt-px shrink-0 text-zinc-500" aria-hidden="true" />
                <span>Bots see only the devices you share with Assist in Home Assistant (Settings › Voice assistants › Expose).</span>
              </li>
              <li className="flex items-start gap-2">
                <Lock size={13} className="mt-px shrink-0 text-zinc-500" aria-hidden="true" />
                <span>Locks, doors, garage doors, gates and alarms always ask you first.</span>
              </li>
            </ul>
          </div>

          {(testing || result) && <ResultBox phase={phase} host={parsed?.host} origin={parsed?.origin} door={req.door} />}

          <div className={`mt-5 flex ${phone ? 'flex-col-reverse' : 'justify-end'} gap-2`}>
            {ok ? (
              <>
                {req.door === 'tools' && !homeTeamExists && onStartHomeTeam && (
                  <button
                    type="button"
                    className={quietBtn}
                    onClick={() => {
                      onClose();
                      onStartHomeTeam();
                    }}
                    data-ha-start-team
                  >
                    Start a Home team
                  </button>
                )}
                <button type="submit" className={primaryBtn} data-ha-done>
                  Done
                </button>
              </>
            ) : (
              <>
                <button type="button" className={quietBtn} onClick={close} disabled={testing}>
                  Cancel
                </button>
                <button type="submit" className={primaryBtn} disabled={!canConnect} data-ha-connect>
                  {testing ? (
                    <>
                      <Spinner /> Connecting…
                    </>
                  ) : result ? (
                    'Try again'
                  ) : (
                    'Connect'
                  )}
                </button>
              </>
            )}
          </div>
        </form>
      </div>
    </div>
  );
};

const Box: React.FC<{ tone: 'info' | 'ok' | 'bad' | 'warn'; icon: React.ReactNode; title: React.ReactNode; children?: React.ReactNode; kind: string }> = ({
  tone,
  icon,
  title,
  children,
  kind,
}) => {
  const cls = {
    info: 'border-sky-400/30 bg-sky-400/10 text-sky-200',
    ok: 'border-emerald-500/40 bg-emerald-500/10 text-emerald-200',
    bad: 'border-rose-500/40 bg-rose-500/10 text-rose-200',
    warn: 'border-amber-500/40 bg-amber-500/10 text-amber-200',
  }[tone];
  return (
    <div role={tone === 'bad' || tone === 'warn' ? 'alert' : 'status'} className={`mt-4 flex items-start gap-2.5 px-3 py-2.5 rounded-[10px] border ${cls}`} data-ha-result={kind}>
      <span className="mt-0.5 shrink-0">{icon}</span>
      <div className="flex-1 min-w-0 space-y-1">
        <div className="text-[13px] font-medium">{title}</div>
        {children && <div className="text-xs leading-relaxed opacity-90">{children}</div>}
      </div>
    </div>
  );
};

const ResultBox: React.FC<{ phase: Phase; host?: string; origin?: string; door: 'tools' | 'team' }> = ({ phase, host, origin, door }) => {
  if (phase.s === 'testing')
    return (
      <Box tone="info" kind="testing" icon={<Loader2 size={16} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />} title="Checking Home Assistant…">
        This takes a few seconds.
      </Box>
    );
  if (phase.s !== 'done') return null;
  const r = phase.r;
  switch (r.kind) {
    case 'ok':
      return (
        <Box
          tone="ok"
          kind="ok"
          icon={<CheckCircle2 size={16} aria-hidden="true" />}
          title={
            <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
              Connected · {actionsText(r.actions)}
              {/* Earned: kwant reached Home Assistant and listed what it can do (design: kwant feel). */}
              {r.actions > 0 && (
                <Sticker tone="holo" size="sm" icon={<CheckCircle2 />} title={`kwant reached Home Assistant and found ${actionsText(r.actions)}`}>
                  Tested
                </Sticker>
              )}
            </span>
          }
        >
          {door === 'team' ? 'House can use it now.' : 'It’s saved in Tools. Start a Home team, or turn it on for any bot in that bot’s settings.'}
        </Box>
      );
    case 'bad_token':
      return (
        <Box tone="bad" kind={r.kind} icon={<AlertTriangle size={16} aria-hidden="true" />} title={haResultTitle(r)}>
          Make a new one in Home Assistant and paste the whole thing here.
        </Box>
      );
    case 'mcp_off':
      return (
        <Box tone="warn" kind={r.kind} icon={<Plug size={16} aria-hidden="true" />} title={haResultTitle(r)}>
          <p>
            MCP Server is the part of Home Assistant that lets apps like kwant control it. In Home Assistant, open Settings › Devices &amp; services › Add
            integration, add “Model Context Protocol Server”, then press Try again.
          </p>
          {origin && (
            <a href={`${origin}/config/integrations/dashboard`} target="_blank" rel="noreferrer" className={`${linkCls} mt-1 text-amber-200 hover:text-amber-100`}>
              Open integrations in Home Assistant <ExternalLink size={12} aria-hidden="true" />
              <span className="sr-only"> (opens a new tab)</span>
            </a>
          )}
        </Box>
      );
    case 'unreachable':
      return (
        <Box
          tone="bad"
          kind={r.kind}
          icon={<WifiOff size={16} aria-hidden="true" />}
          title={
            <>
              kwant can’t reach <span className="font-mono break-all">{host ?? 'Home Assistant'}</span>.
            </>
          }
        >
          Check that Home Assistant is on and the address is right. If kwant runs on another computer or in Docker, use Home Assistant’s IP address instead, like
          http://192.168.1.20:8123.
        </Box>
      );
    case 'not_ha':
      return (
        <Box tone="warn" kind={r.kind} icon={<AlertTriangle size={16} aria-hidden="true" />} title={haResultTitle(r)}>
          Check the address. It’s the one you open Home Assistant at.
        </Box>
      );
    default:
      return (
        <Box tone="bad" kind={r.kind} icon={<AlertTriangle size={16} aria-hidden="true" />} title={haResultTitle(r)}>
          <p data-ha-next>Check that the address opens your Home Assistant in a browser, then try again.</p>
          {r.error && (
            <p className="mt-1 font-mono text-[11px] text-rose-200/70 line-clamp-3 break-words" data-ha-detail>
              {r.error}
            </p>
          )}
        </Box>
      );
  }
};

/** Lives once in the app; `openHomeAssistant(...)` from Tools or the Home team opens it. */
export const HomeAssistantHost: React.FC<{
  /** The bots of a team, for the team door. */
  teamKwants: (teamId: string) => string[];
  /** Team ids in this space, to tell whether a Home team exists. */
  teamIds: string[];
}> = ({ teamKwants, teamIds }) => {
  const [req, setReq] = useState<HomeAssistantRequest | null>(null);
  const rt = useReadyTeams();
  useEffect(() => {
    const on = (e: Event) => setReq((e as CustomEvent<HomeAssistantRequest>).detail ?? { door: 'tools' });
    window.addEventListener(HA_EVENT, on);
    return () => window.removeEventListener(HA_EVENT, on);
  }, []);
  if (!req) return null;
  const homeTeamExists = teamIds.some((id) => rt?.made(id)?.templateId === 'home');
  return (
    <HomeAssistantSheet
      key={`${req.door}:${req.serverId ?? ''}:${req.teamId ?? ''}`}
      req={req}
      onClose={() => setReq(null)}
      teamKwantIds={req.teamId ? teamKwants(req.teamId) : []}
      homeTeamExists={homeTeamExists}
      onStartHomeTeam={rt?.available ? () => rt.open({ kind: 'template', templateId: 'home' }) : undefined}
    />
  );
};
