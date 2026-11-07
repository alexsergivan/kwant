import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Check, ChevronDown, Copy, Loader2, Plus, RotateCcw, TerminalSquare } from 'lucide-react';
import {
  AgentCall,
  AgentConnection,
  ConnectionError,
  Created,
  limitText,
  NAME_MAX,
  NewConnection,
  SETUP_TOOLS,
  SetupTool,
  TOKEN_WARNING,
  agentConnectionsApi,
  botsLine,
  callLine,
  callTime,
  createBody,
  lastUsed,
  nameError,
  serverUrl,
  setupSnippets,
  shortDate,
  spacesLine,
} from '../agentConnections';

export interface ScopeSpace {
  id: string;
  name: string;
  bots: Array<{ id: string; name: string }>;
}

const errText = (e: unknown, fallback: string) => (e instanceof ConnectionError || e instanceof Error ? e.message : fallback);

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[36px] px-3 rounded-lg text-xs font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 disabled:opacity-50';
const quiet = `${btn} text-zinc-300 border border-zinc-800 hover:bg-zinc-800 hover:text-zinc-100`;
const primary = `${btn} text-white bg-sky-600 hover:bg-sky-500`;

/** A Copy button that says Copied for two seconds. Copies only on click; nothing is kept. */
export const CopyButton: React.FC<{ text: string; label: string }> = ({ text, label }) => {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(t);
  }, [copied]);
  return (
    <button
      type="button"
      aria-label={copied ? `${label} copied` : `Copy ${label}`}
      onClick={() => {
        navigator.clipboard?.writeText(text).catch(() => undefined);
        setCopied(true);
      }}
      className={`${quiet} shrink-0`}
    >
      {copied ? <Check size={13} className="text-emerald-400" aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
      {copied ? 'Copied' : 'Copy'}
    </button>
  );
};

const CodeBox: React.FC<{ text: string; label: string }> = ({ text, label }) => (
  <div className="flex items-start gap-2 rounded-lg border border-zinc-800 bg-zinc-950 p-2">
    <pre
      tabIndex={0}
      aria-label={label}
      className="flex-1 min-w-0 overflow-x-auto whitespace-pre text-[11px] leading-relaxed font-mono text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 rounded"
    >
      {text}
    </pre>
    <CopyButton text={text} label={label} />
  </div>
);

/** The one view that shows the token: copy it, set up a tool, then Done forgets it. */
export const TokenReveal: React.FC<{ name: string; token: string; origin: string | null; onDone: () => void }> = ({ name, token, origin, onDone }) => {
  const [tool, setTool] = useState<SetupTool>('claude');
  const head = useRef<HTMLHeadingElement>(null);
  useEffect(() => head.current?.focus(), []);
  const server = serverUrl(origin);
  const picked = SETUP_TOOLS.find((t) => t.id === tool) ?? SETUP_TOOLS[0];
  const snippets = setupSnippets(tool, server, token);
  return (
    <section aria-labelledby="agent-token-h" className="space-y-3 rounded-xl border border-sky-500/30 bg-sky-500/5 p-3" data-agent-token-view>
      <h4 id="agent-token-h" ref={head} tabIndex={-1} className="text-xs font-semibold text-zinc-100 focus:outline-none">
        {name} is connected. Copy its token now.
      </h4>
      <div className="flex items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-950 p-2">
        <code className="flex-1 min-w-0 break-all font-mono text-[11px] text-zinc-100" data-agent-token aria-label="Token">
          {token}
        </code>
        <CopyButton text={token} label="token" />
      </div>
      <p className="flex items-start gap-1.5 text-[11px] text-amber-300">
        <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
        <span>{TOKEN_WARNING}</span>
      </p>
      <div className="space-y-2">
        <p id="agent-setup-label" className="text-[11px] font-medium text-zinc-300">
          Set up your tool
        </p>
        <div role="radiogroup" aria-labelledby="agent-setup-label" className="flex flex-wrap gap-1.5">
          {SETUP_TOOLS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="radio"
              aria-checked={tool === t.id}
              onClick={() => setTool(t.id)}
              className={`${btn} border ${tool === t.id ? 'border-sky-500/60 bg-sky-500/10 text-sky-200' : 'border-zinc-800 text-zinc-400 hover:text-zinc-200'}`}
            >
              {t.label}
            </button>
          ))}
        </div>
        <p className="text-[11px] text-zinc-500">{picked.where}</p>
        {snippets.map((s, i) => (
          <CodeBox key={`${tool}-${i}`} text={s} label={i === 0 ? `${picked.label} setup` : `${picked.label} token line`} />
        ))}
      </div>
      <div className="flex justify-end">
        <button type="button" onClick={onDone} className={primary} data-agent-token-done>
          Done, I’ve saved it
        </button>
      </div>
    </section>
  );
};

/** Name, spaces, and all of their bots or some. */
export const NewConnectionForm: React.FC<{
  spaces: ScopeSpace[];
  busy: boolean;
  error: string | null;
  onCreate: (body: NewConnection) => void;
  onCancel: () => void;
  initialSpace?: string | null;
}> = ({ spaces, busy, error, onCreate, onCancel, initialSpace }) => {
  const uid = useId();
  const [name, setName] = useState('');
  const [touched, setTouched] = useState(false);
  const [spaceIds, setSpaceIds] = useState<string[]>(() => {
    const first = spaces.find((s) => s.id === initialSpace) ?? spaces[0];
    return first ? [first.id] : [];
  });
  const [allBots, setAllBots] = useState(true);
  const [botIds, setBotIds] = useState<string[]>([]);
  const nameRef = useRef<HTMLInputElement>(null);
  useEffect(() => nameRef.current?.focus(), []);
  const inScope = spaces.filter((s) => spaceIds.includes(s.id));
  const pickedBots = botIds.filter((id) => inScope.some((s) => s.bots.some((b) => b.id === id)));
  const body = createBody(name, spaceIds, allBots, pickedBots);
  const nErr = touched ? nameError(name) : null;
  const toggle = (list: string[], id: string) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]);
  return (
    <form
      className="space-y-3 rounded-xl border border-zinc-800 bg-zinc-950/60 p-3"
      aria-label="Connect a coding agent"
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (body && !busy) onCreate(body);
      }}
      data-agent-new
    >
      <div className="space-y-1">
        <label htmlFor={`${uid}-name`} className="block text-[11px] font-medium text-zinc-300">
          Name
        </label>
        <input
          id={`${uid}-name`}
          ref={nameRef}
          value={name}
          maxLength={NAME_MAX + 10}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => setTouched(true)}
          placeholder="Claude Code on my laptop"
          autoComplete="off"
          spellCheck={false}
          aria-invalid={!!nErr}
          aria-describedby={`${uid}-name-hint`}
          className={`w-full bg-zinc-950 border rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:outline-none ${
            nErr ? 'border-rose-500/60 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'
          }`}
        />
        <p id={`${uid}-name-hint`} className={`text-[11px] ${nErr ? 'text-rose-300' : 'text-zinc-500'}`}>
          {nErr ?? 'One per tool, so you can revoke each on its own.'}
        </p>
      </div>
      <fieldset className="min-w-0 space-y-1">
        <legend className="text-[11px] font-medium text-zinc-300">Spaces it can reach</legend>
        {spaces.length ? (
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            {spaces.map((s) => (
              <label key={s.id} className="flex items-center gap-2 min-h-[32px] text-[11px] text-zinc-200 cursor-pointer">
                <input type="checkbox" className="accent-sky-500" checked={spaceIds.includes(s.id)} onChange={() => setSpaceIds((l) => toggle(l, s.id))} />
                {s.name}
              </label>
            ))}
          </div>
        ) : (
          <p className="text-[11px] text-zinc-500">No spaces to connect yet.</p>
        )}
      </fieldset>
      <fieldset className="min-w-0 space-y-1">
        <legend className="text-[11px] font-medium text-zinc-300">Bots it can ask</legend>
        <label className="flex items-center gap-2 min-h-[32px] text-[11px] text-zinc-200 cursor-pointer">
          <input type="radio" name={`${uid}-bots`} className="accent-sky-500" checked={allBots} onChange={() => setAllBots(true)} />
          All bots in these spaces
        </label>
        <label className="flex items-center gap-2 min-h-[32px] text-[11px] text-zinc-200 cursor-pointer">
          <input type="radio" name={`${uid}-bots`} className="accent-sky-500" checked={!allBots} onChange={() => setAllBots(false)} />
          Only the bots I pick
        </label>
        {!allBots && (
          <div className="ml-6 space-y-2" data-agent-bot-picks>
            {inScope.map((s) => (
              <div key={s.id} role="group" aria-label={`Bots in ${s.name}`}>
                {inScope.length > 1 && <p className="text-[11px] text-zinc-500">{s.name}</p>}
                {s.bots.length ? (
                  <div className="flex flex-wrap gap-x-4 gap-y-1">
                    {s.bots.map((b) => (
                      <label key={b.id} className="flex items-center gap-2 min-h-[32px] text-[11px] text-zinc-200 cursor-pointer">
                        <input type="checkbox" className="accent-sky-500" checked={botIds.includes(b.id)} onChange={() => setBotIds((l) => toggle(l, b.id))} />
                        {b.name}
                      </label>
                    ))}
                  </div>
                ) : (
                  <p className="text-[11px] text-zinc-500">No bots in {s.name} yet.</p>
                )}
              </div>
            ))}
            {!inScope.length && <p className="text-[11px] text-zinc-500">Pick a space first.</p>}
          </div>
        )}
      </fieldset>
      {error && (
        <p role="alert" className="flex items-start gap-1.5 text-[11px] text-rose-300">
          <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
          <span>{error}</span>
        </p>
      )}
      <div className="flex flex-wrap justify-end gap-2">
        <button type="button" onClick={onCancel} className={quiet}>
          Cancel
        </button>
        <button type="submit" disabled={!body || busy} aria-busy={busy} className={primary}>
          {busy && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          Create
        </button>
      </div>
    </form>
  );
};

type Activity = { kind: 'loading' } | { kind: 'error'; message: string } | { kind: 'ready'; calls: AgentCall[] };

export const ActivityList: React.FC<{ state: Activity; onRetry: () => void; now?: Date }> = ({ state, onRetry, now }) => {
  if (state.kind === 'loading')
    return (
      <p role="status" className="flex items-center gap-2 text-[11px] text-zinc-500">
        <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
      </p>
    );
  if (state.kind === 'error')
    return (
      <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
        <span className="flex-1">{state.message}</span>
        <button type="button" onClick={onRetry} className="inline-flex items-center gap-1 text-zinc-300 underline underline-offset-2">
          <RotateCcw size={11} aria-hidden="true" /> Retry
        </button>
      </p>
    );
  if (!state.calls.length) return <p className="text-[11px] text-zinc-500">Nothing yet.</p>;
  return (
    <ul className="space-y-0.5" aria-label="Recent activity">
      {state.calls.slice(0, 20).map((c, i) => (
        <li key={`${c.at}-${i}`} className="flex items-baseline gap-2 text-[11px]">
          <span className="w-24 shrink-0 font-mono tabular-nums text-zinc-500">{callTime(c.at, now)}</span>
          <span className={`flex-1 min-w-0 break-words ${c.outcome === 'refused' ? 'text-amber-300' : 'text-zinc-300'}`}>{callLine(c)}</span>
        </li>
      ))}
    </ul>
  );
};

/** One live connection: what it reaches, when it was used, its activity, and Revoke with one confirm. */
export const ConnectionRow: React.FC<{
  c: AgentConnection;
  spaceName: (id: string) => string | undefined;
  botName: (id: string) => string | undefined;
  confirming: boolean;
  revoking: boolean;
  onAskRevoke: () => void;
  onKeep: () => void;
  onRevoke: () => void;
  activity: Activity | null;
  onToggleActivity: () => void;
  onRetryActivity: () => void;
  now?: number;
}> = ({ c, spaceName, botName, confirming, revoking, onAskRevoke, onKeep, onRevoke, activity, onToggleActivity, onRetryActivity, now }) => {
  const actId = `agent-activity-${c.id}`;
  return (
    <li className="rounded-xl border border-zinc-800 bg-zinc-950/60 p-3 space-y-2" data-agent-row={c.id}>
      <div className="flex flex-wrap items-start gap-2">
        <div className="flex-1 min-w-[12rem]">
          <p className="text-xs font-medium text-zinc-100 break-words">{c.name}</p>
          <p className="text-[11px] text-zinc-400 break-words">
            {spacesLine(c.space_ids, spaceName)} · {botsLine(c.bots, botName)}
          </p>
          <p className="text-[11px] text-zinc-500">
            Added {shortDate(c.created_at)} · Last used {lastUsed(c.last_used_at, now)}
          </p>
        </div>
        {!confirming && (
          <button type="button" onClick={onAskRevoke} className={`${quiet} hover:text-rose-200`} aria-label={`Revoke ${c.name}`}>
            Revoke
          </button>
        )}
      </div>
      {confirming && (
        <div role="group" aria-label={`Revoke ${c.name}?`} className="flex flex-wrap items-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/5 p-2">
          <p className="flex-1 min-w-[12rem] text-[11px] text-rose-200">Revoke {c.name}? Its token stops working at once. Its threads stay.</p>
          <button type="button" onClick={onKeep} className={quiet} data-agent-keep>
            Keep
          </button>
          <button type="button" onClick={onRevoke} disabled={revoking} aria-busy={revoking} className={`${btn} text-white bg-rose-600 hover:bg-rose-500`} data-agent-revoke>
            {revoking && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            Revoke
          </button>
        </div>
      )}
      <button
        type="button"
        aria-expanded={!!activity}
        aria-controls={actId}
        onClick={onToggleActivity}
        className="inline-flex items-center gap-1 min-h-[32px] text-[11px] text-zinc-400 hover:text-zinc-200 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
      >
        <ChevronDown size={13} className={`transition-transform motion-reduce:transition-none ${activity ? 'rotate-180' : ''}`} aria-hidden="true" />
        Recent activity
      </button>
      {activity && (
        <div id={actId}>
          <ActivityList state={activity} onRetry={onRetryActivity} now={now === undefined ? undefined : new Date(now)} />
        </div>
      )}
    </li>
  );
};

type ListState = { kind: 'loading' } | { kind: 'error'; message: string } | { kind: 'ready'; connections: AgentConnection[]; max: number };

async function loadScope(send: typeof fetch): Promise<ScopeSpace[]> {
  const r = await send('/api/spaces', { cache: 'no-store' });
  if (!r.ok) throw new Error('Couldn’t load your spaces.');
  const spaces = ((await r.json()) as Array<{ id: string; name: string; archived_at?: string | null }>).filter((s) => !s.archived_at);
  return Promise.all(
    spaces.map(async (s) => {
      const b = await send(`/api/kwants?space=${encodeURIComponent(s.id)}`, { cache: 'no-store' }).catch(() => null);
      const bots = b && b.ok ? ((await b.json()) as Array<{ id: string; name: string }>) : [];
      return { id: s.id, name: s.name, bots: bots.filter((x) => x && typeof x.id === 'string').map((x) => ({ id: x.id, name: x.name })) };
    }),
  );
}

/** Settings → Coding agents. Self-contained: it saves at once, outside the Settings form. */
export const CodingAgents: React.FC<{ send?: typeof fetch; currentSpace?: string | null; origin?: string | null }> = ({
  send = (...a) => fetch(...a),
  currentSpace = null,
  origin = typeof window === 'undefined' ? null : window.location.origin,
}) => {
  const api = useRef(agentConnectionsApi(send)).current;
  const [list, setList] = useState<ListState>({ kind: 'loading' });
  const [scope, setScope] = useState<ScopeSpace[]>([]);
  const [adding, setAdding] = useState(false);
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  // The token lives here, only while the copy view is open.
  const [created, setCreated] = useState<Created | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);
  const [activity, setActivity] = useState<Record<string, Activity>>({});
  const addRef = useRef<HTMLButtonElement>(null);

  const load = useCallback(async () => {
    try {
      const l = await api.list();
      setList({ kind: 'ready', ...l });
    } catch (e) {
      setList({ kind: 'error', message: errText(e, 'Couldn’t load coding agents.') });
    }
  }, [api]);
  useEffect(() => {
    load();
    loadScope(send)
      .then(setScope)
      .catch(() => setScope([]));
    // `send` is stable for the card's life.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [load]);
  // Leaving the card (closing Settings) drops the token with the component's state.
  useEffect(() => () => setCreated(null), []);

  const spaceName = (id: string) => scope.find((s) => s.id === id)?.name;
  const botName = (id: string) => scope.flatMap((s) => s.bots).find((b) => b.id === id)?.name;

  const fetchActivity = async (id: string) => {
    setActivity((a) => ({ ...a, [id]: { kind: 'loading' } }));
    try {
      const calls = await api.activity(id);
      setActivity((a) => (a[id] ? { ...a, [id]: { kind: 'ready', calls } } : a));
    } catch (e) {
      setActivity((a) => (a[id] ? { ...a, [id]: { kind: 'error', message: errText(e, 'Couldn’t load recent activity.') } } : a));
    }
  };

  const full = list.kind === 'ready' && list.connections.length >= list.max;
  const done = () => {
    setCreated(null);
    requestAnimationFrame(() => addRef.current?.focus());
  };

  return (
    <fieldset className="min-w-0 space-y-3 pt-2 border-t border-zinc-800" data-coding-agents>
      <legend className="flex items-center gap-2 text-xs font-medium text-zinc-300 pt-3">
        <TerminalSquare size={14} className="text-sky-300" aria-hidden="true" />
        <span>Coding agents</span>
      </legend>
      <p className="text-[11px] text-zinc-500 leading-relaxed">
        Let Claude Code, Codex or Cursor ask your bots for work and get the reply where you’re coding. Each connection reaches only the spaces and
        bots you pick, can’t approve anything, and can be revoked at once.
      </p>

      {created ? (
        <TokenReveal name={created.connection.name} token={created.token} origin={origin} onDone={done} />
      ) : adding ? (
        <NewConnectionForm
          spaces={scope}
          busy={busy}
          error={formError}
          initialSpace={currentSpace}
          onCancel={() => {
            setAdding(false);
            setFormError(null);
            requestAnimationFrame(() => addRef.current?.focus());
          }}
          onCreate={async (body) => {
            setBusy(true);
            setFormError(null);
            try {
              const c = await api.create(body);
              setAdding(false);
              setCreated(c);
              setList((l) => (l.kind === 'ready' ? { ...l, connections: [c.connection, ...l.connections.filter((x) => x.id !== c.connection.id)] } : l));
            } catch (e) {
              setFormError(errText(e, 'Couldn’t connect it. Try again.'));
            } finally {
              setBusy(false);
            }
          }}
        />
      ) : (
        <div className="space-y-1">
          <button
            ref={addRef}
            type="button"
            disabled={list.kind !== 'ready' || full}
            onClick={() => {
              setFormError(null);
              setAdding(true);
            }}
            className={quiet}
            data-agent-add
          >
            <Plus size={13} aria-hidden="true" /> Connect a coding agent
          </button>
          {full && <p className="text-[11px] text-amber-300">{limitText(list.max)}</p>}
        </div>
      )}

      {list.kind === 'loading' && (
        <p role="status" className="flex items-center gap-2 text-[11px] text-zinc-500">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading coding agents…
        </p>
      )}
      {list.kind === 'error' && (
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <span className="flex-1">{list.message}</span>
          <button
            type="button"
            onClick={() => {
              setList({ kind: 'loading' });
              load();
            }}
            className="inline-flex items-center gap-1 text-zinc-300 underline underline-offset-2"
          >
            <RotateCcw size={11} aria-hidden="true" /> Retry
          </button>
        </p>
      )}
      {rowError && (
        <p role="alert" className="text-[11px] text-rose-300">
          {rowError}
        </p>
      )}
      {list.kind === 'ready' && list.connections.length > 0 && (
        <ul className="space-y-2" aria-label="Connected coding agents">
          {list.connections.map((c) => (
            <ConnectionRow
              key={c.id}
              c={c}
              spaceName={spaceName}
              botName={botName}
              confirming={confirm === c.id}
              revoking={revoking === c.id}
              onAskRevoke={() => {
                setRowError(null);
                setConfirm(c.id);
              }}
              onKeep={() => setConfirm(null)}
              onRevoke={async () => {
                setRevoking(c.id);
                setRowError(null);
                try {
                  await api.revoke(c.id);
                } catch (e) {
                  if (!(e instanceof ConnectionError && e.status === 404)) {
                    setRowError(errText(e, 'Couldn’t revoke it. Try again.'));
                    setRevoking(null);
                    return;
                  }
                }
                setRevoking(null);
                setConfirm(null);
                if (created?.connection.id === c.id) setCreated(null);
                setList((l) => (l.kind === 'ready' ? { ...l, connections: l.connections.filter((x) => x.id !== c.id) } : l));
                requestAnimationFrame(() => addRef.current?.focus());
              }}
              activity={activity[c.id] ?? null}
              onToggleActivity={() => {
                if (activity[c.id]) setActivity(({ [c.id]: _, ...rest }) => rest);
                else fetchActivity(c.id);
              }}
              onRetryActivity={() => fetchActivity(c.id)}
            />
          ))}
        </ul>
      )}
      {list.kind === 'ready' && !list.connections.length && !created && !adding && (
        <p className="text-[11px] text-zinc-500">No coding agents connected.</p>
      )}
    </fieldset>
  );
};
