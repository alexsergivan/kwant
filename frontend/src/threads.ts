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
  /** "decide": a consult or decide turn of the decide-first gate. */
  purpose?: string;
  /** 14a: an audit's hidden turn, with whose work it double-checks and why. */
  audit?: import('./auditor').AuditTurn;
  /** A running turn's steps so far (like `meta.activity`), so a page opened mid-turn can show them. */
  activity?: unknown;
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
  /** Continue in a new chat (continueChat.ts): the chat and message it was continued from; null otherwise. */
  forked_from_context?: string | null;
  forked_from_message_id?: string | null;
  forked_copied?: number | null;
  forked_truncated?: boolean | null;
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

/** Why a reply waits: its bot is busy, the server is full, or every VM spot is taken (#446). */
export type QueueReason = 'agent_busy' | 'server_busy' | 'vm_busy';

const queueReason = (r: unknown): QueueReason => (r === 'server_busy' || r === 'vm_busy' ? r : 'agent_busy');

export interface TrackedTurn extends ActiveTurn {
  /** Order seen; the queue is FIFO. */
  seq: number;
  reason?: QueueReason;
  /** vm_busy only: how many replies run inside the VM at once. */
  slots?: number;
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

// ---------------------------------------------------------------------------
// Bringing a finished reply onto the screen

/**
 * Whether `turn_ended` loads its chat again. Always: the model driver sends TurnComplete before
 * kwnt saves the reply, so the load TurnComplete starts can come back without it. `turn_ended`
 * is sent once the reply is saved, so its load is the one that is sure to show it.
 */
export function reloadOnTurnEnd(_turnId: string | null | undefined): boolean {
  return true;
}

/**
 * Loads of one chat can finish out of order (a slow first load landing after a later one). Each
 * load takes a number when it starts; only the newest started load for a chat may replace what's
 * on screen, so an older answer never hides a reply a newer one already showed.
 *
 * 33c: a message that arrives live while a load is running may be missing from that load's list (the
 * load read the chat just before the server saved it; the lead's hand-off card did this). `noteLive`
 * remembers it, and `keepLive` puts it back when that load lands, so it doesn't vanish until a reload.
 */
export class ChatLoads {
  private seq: Record<string, number> = {};
  private newest: Record<string, Promise<void>> = {};
  private running: Record<string, number> = {};
  /** Messages that came live while a load was running, each with the newest load started before it. */
  private live: Record<string, { after: number; message: { id: string } }[]> = {};

  start(ctx: string): number {
    this.seq[ctx] = (this.seq[ctx] ?? 0) + 1;
    return this.seq[ctx];
  }

  isLatest(ctx: string, n: number): boolean {
    return this.seq[ctx] === n;
  }

  /**
   * Runs one load of a chat; `load` may only touch the screen while `isLatest()` holds. The promise resolves
   * once the newest load of that chat has landed, never earlier: a load a newer one replaced waits for that one.
   * 22r: TurnComplete's load was replaced by turn_ended's, came back first, and its follow-up ended the turn
   * (dropped the working line) while the list without the reply was still on screen, so the asked-for OK was
   * the newest message again and "… is waiting for your OK" came back for a few frames.
   */
  run(
    ctx: string,
    load: (isLatest: () => boolean, keepLive: <M extends { id: string }>(data: M[]) => M[]) => Promise<void>,
  ): Promise<void> {
    const n = this.start(ctx);
    this.running[ctx] = (this.running[ctx] ?? 0) + 1;
    const p = load(
      () => this.isLatest(ctx, n),
      (data) => this.keepLive(ctx, n, data),
    ).finally(() => {
      this.running[ctx] -= 1;
      // The newest load has landed: every live message so far is in its list or was put back
      if (this.isLatest(ctx, n)) delete this.live[ctx];
    });
    this.newest[ctx] = p;
    return this.landed(ctx, p);
  }

  /** A message that just came live in `ctx` (33c): kept for the loads already running, which may have missed it. */
  noteLive(ctx: string, message: { id: string }): void {
    if (!this.running[ctx]) return;
    (this.live[ctx] ??= []).push({ after: this.seq[ctx] ?? 0, message });
  }

  /**
   * `data`, the list load `n` of `ctx` got, plus the messages that came live after that load started and aren't in
   * it, in the order they came. A message the load also has is never added twice.
   */
  keepLive<M extends { id: string }>(ctx: string, n: number, data: M[]): M[] {
    const missed = (this.live[ctx] ?? [])
      .filter((l) => l.after >= n && !data.some((m) => m.id === l.message.id))
      .map((l) => l.message as M);
    const once = missed.filter((m, i) => missed.findIndex((x) => x.id === m.id) === i);
    return once.length ? [...data, ...once] : data;
  }

  private async landed(ctx: string, p: Promise<void>): Promise<void> {
    await p.catch(() => undefined);
    const next = this.newest[ctx];
    if (next && next !== p) return this.landed(ctx, next);
  }
}

/**
 * Whether TurnComplete's load may settle the chat (clear the streamed text, drop the working line). Not while that
 * turn is still running: TurnComplete comes before kwnt saves the reply, so its load can come back without it, and
 * clearing the stream then blanks the reply for a few frames before it lands (22r). turn_ended settles it instead,
 * after the load that has the reply. A TurnComplete with no turn id, or one that arrives after turn_ended, settles.
 */
export function settlesOnTurnComplete(turns: TurnMap, turnId: unknown): boolean {
  return !(typeof turnId === 'string' && turnId in turns);
}

/**
 * Whether a load brought a saved bot reply the list on screen didn't have: its newest message is the bot's, saved
 * (not temp-) and new. Then TurnComplete may clear the streamed text even while the turn is still running, so the
 * saved reply and its streamed copy are never on screen together (22r).
 */
export function landedNewReply(
  before: readonly { id: string; sender_role?: string }[] | undefined,
  after: readonly { id: string; sender_role?: string }[] | undefined,
): boolean {
  const last = after?.[after.length - 1];
  if (!last || last.sender_role !== 'kwant' || last.id.startsWith('temp-')) return false;
  return !(before ?? []).some((m) => m.id === last.id);
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
          reason: queueReason(d.reason),
          slots: d.reason === 'vm_busy' && typeof d.slots === 'number' && d.slots > 0 ? d.slots : undefined,
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
          ...((d.audit ?? prev?.audit) ? { audit: d.audit ?? prev?.audit } : {}),
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
    if (t.reason === 'vm_busy' ? o.reason === 'vm_busy' : t.reason === 'server_busy' || o.kwant_id === t.kwant_id) ahead++;
  }
  const computed = ahead + 1;
  return t.reported ? Math.min(t.reported, computed) : computed;
}

/**
 * stuck-getting-started (0.9.51): the chats showing a reply as live (`live`, keyed by chat) that no
 * turn the server lists runs in. A missed `turn_ended` left one "getting started" for hours.
 */
export const unlistedLive = (live: Readonly<Record<string, unknown>>, turns: TurnMap): string[] => {
  const busy = new Set(Object.values(turns).map((t) => t.context_id));
  return Object.keys(live).filter((ctx) => !busy.has(ctx));
};

/** The least time between two re-reads of the running turns when the window comes back. */
export const REREAD_GAP_MS = 5_000;

/** Whether the window coming back (focus, or the tab shown again) re-reads the running turns now. */
export const rereadDue = (lastAt: number, now: number, hidden: boolean): boolean => !hidden && now - lastAt >= REREAD_GAP_MS;

export const turnsIn = (turns: TurnMap, contextId: string): TrackedTurn[] =>
  Object.values(turns)
    .filter((t) => t.context_id === contextId)
    .sort((a, b) => a.seq - b.seq);

/**
 * The open chat's waiting turns. `alone`: the first queued turn while nothing runs here, which takes
 * the live bubble's place ("In line."). `beside`: every turn queued while a reply runs here (P6 #467:
 * a second message to a bot still answering waits with `agent_busy`), oldest first, drawn under the
 * running reply so the person sees it was received.
 */
export function queuedInChat(turns: TurnMap, contextId: string): { alone: TrackedTurn | null; beside: TrackedTurn[] } {
  const here = turnsIn(turns, contextId);
  const queued = here.filter((t) => t.state === 'queued');
  if (!here.some((t) => t.state === 'running')) return { alone: queued[0] ?? null, beside: [] };
  return { alone: null, beside: queued };
}

/**
 * A turn waiting beside a running reply belongs to a different bot than any reply running in this
 * chat (a room: Scout waits behind its own work elsewhere while Marketer answers here). Its line then
 * names that bot instead of saying "this bot" under someone else's reply.
 */
export function waitsForOtherBot(turns: TurnMap, contextId: string, waiting: TrackedTurn): boolean {
  const running = turnsIn(turns, contextId).filter((t) => t.state === 'running');
  return running.length > 0 && !running.some((t) => t.kwant_id === waiting.kwant_id);
}

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

/**
 * 25a: the open chat's badge while its reply is still being written (its id is in App's `live`). The turn's
 * `turn_ended` can land a fetch before the saved reply, and the header must not say "done" while the body
 * still says "writing", so a missing or final badge reads "Working" until the live card goes. "Waiting for
 * you" and "Queued" stay as they are. Only for the conversation on screen: other rows keep `threadBadge`.
 */
export function writingBadge(badge: Badge | null, writing: boolean): Badge | null {
  if (!writing) return badge;
  if (badge && (badge.kind === 'working' || badge.kind === 'waiting' || badge.kind === 'queued')) return badge;
  return { kind: 'working', label: 'Working' };
}

export const isOpenDelegation = (d: Pick<Delegation, 'status'> | null | undefined): boolean =>
  !!d && (d.status === 'queued' || d.status === 'running');

/**
 * After the live stream drops and comes back, the hand-offs whose cards may have missed an update:
 * every open one held, plus ones a chat note named whose first fetch never landed. Sorted, no repeats.
 */
export function delegationsToRefresh(
  held: Record<string, Pick<Delegation, 'status'> | undefined>,
  named: Iterable<string> = [],
): string[] {
  const ids = new Set<string>();
  for (const [id, d] of Object.entries(held)) if (isOpenDelegation(d)) ids.add(id);
  // Array.from, not for…of or spread: the node tests transpile to ES5, where those skip a Set.
  for (const id of Array.from(named)) if (!held[id]) ids.add(id);
  return Array.from(ids).sort();
}

/**
 * Whether a refetched record is older than a live update for the same hand-off. `seq` is one counter
 * for both: a live update stores its number in `live[id]`, a fetch takes its own when it goes out.
 * The record has no updated_at, and a resumed hand-off goes back to queued, so the status can't tell.
 */
export const isStaleDelegationFetch = (live: Record<string, number>, id: string, sentSeq: number): boolean =>
  (live[id] ?? 0) > sentSeq;

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
  /** 32m: the last account switch in the teammate's run (result messages only; from #975). */
  failover?: DelegationFailover;
}

/** 32m: `meta.failover` on a `delegation_result`: the teammate ran out of usage or was signed out and switched account. */
export interface DelegationFailover {
  to_name: string;
  from_name: string;
  /** Why it switched: "quota" (out of usage) or "auth" (signed out); anything else is shown without a reason. */
  class: string;
  retry_at: string | null;
}

/** 32m: keeps a failover only when it is an object with a non-empty `to_name`; anything else means none. */
function parseFailover(raw: unknown): DelegationFailover | undefined {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return undefined;
  const f = raw as Record<string, unknown>;
  if (typeof f.to_name !== 'string' || !f.to_name.trim()) return undefined;
  return {
    to_name: f.to_name,
    from_name: typeof f.from_name === 'string' ? f.from_name : '',
    class: typeof f.class === 'string' ? f.class : '',
    retry_at: typeof f.retry_at === 'string' ? f.retry_at : null,
  };
}

/** 32m: the chip's full wording: "Writer switched to Work Claude after Personal ran out of usage". */
export function failoverText(name: string, f: DelegationFailover): string {
  const base = `${name} switched to ${f.to_name}`;
  if (!f.from_name.trim()) return base;
  if (f.class === 'quota') return `${base} after ${f.from_name} ran out of usage`;
  if (f.class === 'auth') return `${base} after ${f.from_name} was signed out`;
  return base;
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
  const failover = parseFailover(m.failover);
  return {
    ...(failover ? { failover } : {}),
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

/** 32j: what a running teammate is waiting on the person for, if anything. */
export type DelegationWaiting = 'approval' | 'question';

type WaitingApproval = { kwant_id: string | null; link: { thread_id?: string | null } | null; refused?: true };

/**
 * 32j: the pending approval that blocks the teammate on a delegation: one linked to its thread, or,
 * from the live event before /api/away links it, one with no link from that teammate. Returned
 * whole so App can name its action with the approval card's own words (approvalAction).
 */
export function delegationWaitingApproval<A extends WaitingApproval>(
  meta: Pick<DelegationMeta, 'thread_id' | 'specialist_id'>,
  approvals: ReadonlyArray<A>,
): A | null {
  return (
    approvals.find(
      (a) =>
        !a.refused &&
        (a.link ? !!a.link.thread_id && a.link.thread_id === meta.thread_id : !!meta.specialist_id && a.kwant_id === meta.specialist_id),
    ) ?? null
  );
}

/**
 * 32j: whether the teammate on a delegation is blocked on the person: a pending approval (above),
 * else an open question in its thread. Structural types so threads.ts stays import-free.
 */
export function delegationWaiting(
  meta: Pick<DelegationMeta, 'thread_id' | 'specialist_id'>,
  approvals: ReadonlyArray<WaitingApproval>,
  questions: Readonly<Record<string, { thread_id: string | null; status: string }>>,
): DelegationWaiting | null {
  if (delegationWaitingApproval(meta, approvals)) return 'approval';
  for (const q of Object.values(questions)) if (q.status === 'open' && q.thread_id && q.thread_id === meta.thread_id) return 'question';
  return null;
}

/** The one-line status under a delegation card, from the live record when there is one. */
export function delegationStatusLine(
  status: DelegationStatus,
  specialist: string,
  opts: { queuePos?: number | null; error?: string | null; waiting?: DelegationWaiting | null; action?: string | null } = {},
): string {
  switch (status) {
    case 'queued':
      return opts.queuePos ? `Waiting for ${specialist} · #${opts.queuePos} in line` : `Waiting for ${specialist} to start`;
    case 'running':
      // The action is the approval card's own words ("send an email"); without one, never guess.
      if (opts.waiting === 'approval') return opts.action ? `${specialist} needs your OK to ${opts.action}` : `${specialist} needs your OK`;
      if (opts.waiting === 'question') return `${specialist} asked you a question`;
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
    // The status rides along so callers can tell a 409 or 410 apart without matching wording.
    throw Object.assign(new Error(message), { status: res.status });
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
