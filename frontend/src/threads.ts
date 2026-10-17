// Threads, delegations and concurrent turns (batch 2 / A). Pure helpers, so App and the tests share
// them; the API calls at the bottom are thin wrappers over the routes in design-A.md. No imports, so
// the node tests can load it on its own.

export type DelegationStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' | 'interrupted';

export interface Delegation {
  id: string;
  thread_id: string;
  lead_kwant_id: string;
  specialist_kwant_id: string;
  origin_channel_id: string | null;
  origin_kwant_id: string | null;
  origin_thread_id: string | null;
  task: string;
  title: string;
  status: DelegationStatus;
  result_message_id: string | null;
  error: string | null;
  created_at: string;
  started_at: string | null;
  ended_at: string | null;
}

export interface ActiveTurn {
  turn_id: string;
  kwant_id: string;
  context_id: string;
  thread_id: string | null;
  state: 'queued' | 'running';
  since?: string;
}

export interface Thread {
  id: string;
  kwant_id: string;
  title: string;
  kind: 'chat' | 'delegation';
  team_id: string | null;
  /** 'user' or the id of the kwant that delegated into it. */
  created_by: string;
  archived: boolean;
  created_at: string;
  updated_at: string;
  delegation?: Delegation | null;
  active?: ActiveTurn[];
}

// ---------------------------------------------------------------------------
// Which chat an SSE event belongs to

/** The open chat's context id: a channel id, a kwant id (direct chat) or a thread id. */
export const contextOf = (open: { selectedId: string; threadId?: string | null }): string => open.threadId || open.selectedId;

/** The chat a turn event belongs to; turn events carry it at the top level, lifecycle events in `data`. */
export function eventContextId(ev: { context_id?: unknown; data?: unknown }): string | null {
  if (typeof ev.context_id === 'string' && ev.context_id) return ev.context_id;
  const d = ev.data as { context_id?: unknown } | null | undefined;
  if (d && typeof d === 'object' && typeof d.context_id === 'string' && d.context_id) return d.context_id;
  return null;
}

/**
 * Where a streamed event (TextDelta, ThinkingDelta, TurnComplete, Error…) should go. Tagged events
 * go to their own chat; untagged ones (older servers, team discussions) to the chat that started
 * the foreground stream.
 */
export const streamTarget = (ev: { context_id?: unknown; data?: unknown }, fallback: string | null): string | null =>
  eventContextId(ev) ?? fallback;

/** True when a streamed event should be drawn in the chat that's open. */
export const shouldRenderDelta = (
  ev: { context_id?: unknown; data?: unknown },
  openContextId: string,
  fallback: string | null,
): boolean => !!openContextId && streamTarget(ev, fallback) === openContextId;

// ---------------------------------------------------------------------------
// Turns: running and queued, from turn_queued / turn_started / turn_ended and GET /api/turns

export interface TrackedTurn extends ActiveTurn {
  /** Order seen; the queue is FIFO. */
  seq: number;
  reason?: 'agent_busy' | 'server_busy';
  /** Position reported by the server when it queued; only ever goes down. */
  reported?: number;
}

export type TurnMap = Readonly<Record<string, TrackedTurn>>;

let seqCounter = 0;

export function hydrateTurns(list: ActiveTurn[] | undefined): TurnMap {
  const out: Record<string, TrackedTurn> = {};
  const sorted = [...(list ?? [])].sort((a, b) => (a.since ?? '').localeCompare(b.since ?? ''));
  for (const t of sorted) {
    if (!t?.turn_id) continue;
    out[t.turn_id] = { ...t, thread_id: t.thread_id ?? null, seq: ++seqCounter };
  }
  return out;
}

/** Applies one SSE event; returns the same map when the event isn't about turns. */
export function applyTurnEvent(turns: TurnMap, ev: { type?: string; data?: any }): TurnMap {
  const d = ev.data ?? {};
  const id: unknown = d.turn_id;
  switch (ev.type) {
    case 'turn_queued':
    case 'TurnQueued': {
      if (typeof id !== 'string') return turns;
      return {
        ...turns,
        [id]: {
          turn_id: id,
          kwant_id: d.kwant_id,
          context_id: d.context_id ?? d.thread_id ?? d.kwant_id,
          thread_id: d.thread_id ?? null,
          state: 'queued',
          seq: turns[id]?.seq ?? ++seqCounter,
          reason: d.reason === 'server_busy' ? 'server_busy' : 'agent_busy',
          reported: typeof d.position === 'number' ? d.position : undefined,
          since: new Date().toISOString(),
        },
      };
    }
    case 'turn_started':
    case 'TurnStarted': {
      if (typeof id !== 'string') return turns;
      const prev = turns[id];
      return {
        ...turns,
        [id]: {
          turn_id: id,
          kwant_id: d.kwant_id,
          context_id: d.context_id ?? d.thread_id ?? d.kwant_id,
          thread_id: d.thread_id ?? null,
          state: 'running',
          seq: prev?.seq ?? ++seqCounter,
          since: prev?.since ?? new Date().toISOString(),
        },
      };
    }
    case 'turn_ended':
    case 'TurnEnded': {
      if (typeof id !== 'string' || !turns[id]) return turns;
      const next = { ...turns };
      delete next[id];
      return next;
    }
    default:
      return turns;
  }
}

/** 1 = next to start. Counts the queued turns ahead in the same line (the agent's, or the server's). */
export function queuePosition(turns: TurnMap, turnId: string): number | null {
  const t = turns[turnId];
  if (!t || t.state !== 'queued') return null;
  let ahead = 0;
  for (const o of Object.values(turns)) {
    if (o.state !== 'queued' || o.seq >= t.seq) continue;
    if (t.reason === 'server_busy' || o.kwant_id === t.kwant_id) ahead++;
  }
  const computed = ahead + 1;
  return t.reported ? Math.min(t.reported, computed) : computed;
}

export const turnsIn = (turns: TurnMap, contextId: string): TrackedTurn[] =>
  Object.values(turns)
    .filter((t) => t.context_id === contextId)
    .sort((a, b) => a.seq - b.seq);

/** The kwant's turns outside its direct chat (threads, delegations, team channels), oldest first. */
export const backgroundTurns = (turns: TurnMap, kwantId: string): TrackedTurn[] =>
  Object.values(turns)
    .filter((t) => t.kwant_id === kwantId && t.context_id !== kwantId)
    .sort((a, b) => a.seq - b.seq);

// ---------------------------------------------------------------------------
// Thread list and status badges

export type BadgeKind = 'working' | 'waiting' | 'queued' | 'done' | 'failed' | 'interrupted' | 'cancelled';

export interface Badge {
  kind: BadgeKind;
  /** Short visible text, e.g. "Queued · #2". */
  label: string;
}

const FINAL_LABEL: Record<string, Badge> = {
  completed: { kind: 'done', label: 'Done' },
  failed: { kind: 'failed', label: 'Failed' },
  cancelled: { kind: 'cancelled', label: 'Cancelled' },
  interrupted: { kind: 'interrupted', label: 'Interrupted' },
};

/**
 * A thread's badge: live turns win over the delegation record; idle user threads have none.
 * A turn paused on the person's answer (`awaiting`) reads "Waiting for you", not "Working".
 */
export function threadBadge(thread: Pick<Thread, 'id' | 'delegation'>, turns: TurnMap, awaiting?: ReadonlySet<string>): Badge | null {
  const live = turnsIn(turns, thread.id);
  const running = live.filter((t) => t.state === 'running');
  if (running.some((t) => !awaiting?.has(t.turn_id))) return { kind: 'working', label: 'Working' };
  if (running.length) return { kind: 'waiting', label: 'Waiting for you' };
  const queued = live.find((t) => t.state === 'queued');
  if (queued) {
    const pos = queuePosition(turns, queued.turn_id);
    return { kind: 'queued', label: pos ? `Queued · #${pos}` : 'Queued' };
  }
  const st = thread.delegation?.status;
  if (!st) return null;
  if (st === 'running') return { kind: 'working', label: 'Working' };
  if (st === 'queued') return { kind: 'queued', label: 'Queued' };
  return FINAL_LABEL[st] ?? null;
}

export const isOpenDelegation = (d: Pick<Delegation, 'status'> | null | undefined): boolean =>
  !!d && (d.status === 'queued' || d.status === 'running');

/** Visible threads: delegated ones first, then the person's, newest activity first within each. */
export function sortThreads(threads: Thread[], includeArchived = false): Thread[] {
  return threads
    .filter((t) => includeArchived || !t.archived)
    .sort((a, b) => {
      if (a.archived !== b.archived) return a.archived ? 1 : -1;
      if (a.kind !== b.kind) return a.kind === 'delegation' ? -1 : 1;
      return (b.updated_at ?? '').localeCompare(a.updated_at ?? '');
    });
}

/** Inserts or replaces a thread, keeping fields only the list route sends (delegation, active). */
export function upsertThread(list: Thread[] | undefined, t: Thread): Thread[] {
  const prev = list ?? [];
  const i = prev.findIndex((x) => x.id === t.id);
  if (i < 0) return [t, ...prev];
  const merged = { ...prev[i], ...t, delegation: t.delegation ?? prev[i].delegation, active: t.active ?? prev[i].active };
  return prev.map((x, j) => (j === i ? merged : x));
}

/** Puts a delegation update on its thread; unknown threads are left alone (the list fetch brings them). */
export function applyDelegation(list: Thread[] | undefined, d: Delegation): Thread[] | undefined {
  if (!list || !list.some((t) => t.id === d.thread_id)) return list;
  return list.map((t) => (t.id === d.thread_id ? { ...t, delegation: d } : t));
}

// ---------------------------------------------------------------------------
// Messages kwnt writes about delegations (`meta` is a JSON string on the wire)

export type DelegationMetaKind = 'delegation_started' | 'delegation_result' | 'delegation_stalled';

export interface DelegationMeta {
  kind: DelegationMetaKind;
  delegation_id: string;
  thread_id: string;
  specialist_id: string;
  specialist_name: string;
  lead_id?: string;
  status: DelegationStatus;
  title: string;
}

const META_KINDS: DelegationMetaKind[] = ['delegation_started', 'delegation_result', 'delegation_stalled'];

/** Parses `message.meta`; null for anything that isn't a well-formed delegation note. Never throws. */
export function parseDelegationMeta(raw: unknown): DelegationMeta | null {
  let v: unknown = raw;
  if (typeof raw === 'string') {
    if (!raw.trim()) return null;
    try {
      v = JSON.parse(raw);
    } catch {
      return null;
    }
  }
  if (!v || typeof v !== 'object') return null;
  const m = v as Record<string, unknown>;
  if (!META_KINDS.includes(m.kind as DelegationMetaKind)) return null;
  if (typeof m.delegation_id !== 'string' || typeof m.thread_id !== 'string') return null;
  const str = (x: unknown, d = '') => (typeof x === 'string' ? x : d);
  return {
    kind: m.kind as DelegationMetaKind,
    delegation_id: m.delegation_id,
    thread_id: m.thread_id,
    specialist_id: str(m.specialist_id),
    specialist_name: str(m.specialist_name, 'The specialist'),
    lead_id: typeof m.lead_id === 'string' ? m.lead_id : undefined,
    status: (str(m.status, 'queued') as DelegationStatus),
    title: str(m.title, 'Delegated task'),
  };
}

/** The one-line status under a delegation card, from the live record when there is one. */
export function delegationStatusLine(
  status: DelegationStatus,
  specialist: string,
  opts: { queuePos?: number | null; error?: string | null } = {},
): string {
  switch (status) {
    case 'queued':
      return opts.queuePos ? `Waiting for ${specialist} · #${opts.queuePos} in line` : `Waiting for ${specialist} to start`;
    case 'running':
      return `${specialist} is working on it`;
    case 'completed':
      return `${specialist} finished`;
    case 'failed':
      return opts.error ? `${specialist} couldn't finish: ${opts.error}` : `${specialist} couldn't finish`;
    case 'cancelled':
      return 'Stopped';
    case 'interrupted':
      return 'Interrupted by a restart';
    default:
      return '';
  }
}

/** Where a message belongs, for `message_added`. */
export const messageContextId = (m: { thread_id?: string | null; channel_id?: string | null; direct_kwant_id?: string | null }) =>
  m.thread_id || m.channel_id || m.direct_kwant_id || null;

// ---------------------------------------------------------------------------
// API

export async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let message = res.status === 404 ? 'It no longer exists.' : `Request failed (HTTP ${res.status}).`;
    try {
      const data = await res.json();
      if (data && typeof data.error === 'string' && data.error) message = data.error;
    } catch {
      // no JSON body
    }
    throw new Error(message);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

const enc = encodeURIComponent;

export const threadsApi = {
  list: (kwantId: string, includeArchived = false) =>
    request<Thread[]>('GET', `/api/kwants/${enc(kwantId)}/threads${includeArchived ? '?include_archived=1' : ''}`),
  create: (kwantId: string, title?: string) => request<Thread>('POST', `/api/kwants/${enc(kwantId)}/threads`, title ? { title } : {}),
  patch: (id: string, body: { title?: string; archived?: boolean }) => request<Thread>('PATCH', `/api/threads/${enc(id)}`, body),
  messages: <M>(id: string) => request<M[]>('GET', `/api/threads/${enc(id)}/messages`),
  cancel: (id: string) => request<{ cancelled: number }>('POST', `/api/threads/${enc(id)}/cancel`),
  turns: () => request<{ turns: ActiveTurn[] }>('GET', '/api/turns'),
  delegation: (id: string) => request<Delegation>('GET', `/api/delegations/${enc(id)}`),
  cancelDelegation: (id: string) => request<Delegation>('POST', `/api/delegations/${enc(id)}/cancel`),
};

// ---------------------------------------------------------------------------
// Settings

export const TURN_LIMITS = {
  max_turns_per_agent: { min: 1, max: 16 },
  max_turns_global: { min: 1, max: 64 },
  delegation_follow_up_minutes: { min: 0, max: 1440 },
} as const;

export type TurnLimitKey = keyof typeof TURN_LIMITS;

/** Error text for one limit field, or null when it's a whole number in range. */
export function limitError(key: TurnLimitKey, raw: string): string | null {
  const { min, max } = TURN_LIMITS[key];
  const s = raw.trim();
  if (!/^\d+$/.test(s)) return `Enter a whole number from ${min} to ${max}.`;
  const n = Number(s);
  if (n < min || n > max) return `Use ${min}–${max}.`;
  return null;
}
