// H4 (release-090/h4-contract.md, Vega's #499): the server's status object per bot. The frontend
// here only reads what H5 and H3 need: `stuck` for the attention order's rank 2, the turn start for
// "running longest first", `held` for the line under a held message, and Try again.
import { personFetch } from './auth';

export type H4StatusKind = 'working' | 'queued' | 'needs_you' | 'quiet' | 'stuck' | 'paused' | 'available';

export interface H4Held {
  kind: 'message' | 'handoff';
  id: string;
  since?: string;
  reason_code?: string;
  context_id?: string;
}

export interface H4Status {
  kwant_id: string;
  status: H4StatusKind;
  reason_code: string;
  since?: string;
  waits_on?: { kind: string; label?: string };
  turn?: { started_at?: string; turn_id?: string; context_id?: string; thread_id?: string | null };
  retry?: { at?: string; try?: number; of?: number };
  stuck?: { gave_up_at?: string; tries?: number };
  held: H4Held[];
}

const rec = (v: unknown): Record<string, unknown> | null => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
const KINDS: readonly H4StatusKind[] = ['working', 'queued', 'needs_you', 'quiet', 'stuck', 'paused', 'available'];

/** One status object, or null when it isn't one (unknown statuses are dropped, not guessed). */
export function parseH4Status(v: unknown): H4Status | null {
  const o = rec(v);
  const kwant_id = str(o?.kwant_id);
  const status = str(o?.status) as H4StatusKind | undefined;
  if (!o || !kwant_id || !status || !KINDS.includes(status)) return null;
  const w = rec(o.waits_on);
  const t = rec(o.turn);
  const r = rec(o.retry);
  const s = rec(o.stuck);
  const held = (Array.isArray(o.held) ? o.held : []).flatMap((h): H4Held[] => {
    const x = rec(h);
    const id = str(x?.id);
    const kind = x?.kind === 'handoff' ? 'handoff' : x?.kind === 'message' ? 'message' : null;
    return x && id && kind ? [{ kind, id, since: str(x.since), reason_code: str(x.reason_code), context_id: str(x.context_id) }] : [];
  });
  return {
    kwant_id,
    status,
    reason_code: str(o.reason_code) ?? '',
    since: str(o.since),
    waits_on: w && str(w.kind) ? { kind: str(w.kind)!, label: str(w.label) } : undefined,
    turn: t ? { started_at: str(t.started_at), turn_id: str(t.turn_id), context_id: str(t.context_id), thread_id: str(t.thread_id) ?? null } : undefined,
    retry: r ? { at: str(r.at), try: num(r.try), of: num(r.of) } : undefined,
    stuck: s ? { gave_up_at: str(s.gave_up_at), tries: num(s.tries) } : undefined,
    held,
  };
}

/** `GET /api/bots/status` body → map by bot. */
export function parseH4List(v: unknown): Record<string, H4Status> {
  const out: Record<string, H4Status> = {};
  for (const b of Array.isArray(rec(v)?.bots) ? (rec(v)!.bots as unknown[]) : []) {
    const s = parseH4Status(b);
    if (s) out[s.kwant_id] = s;
  }
  return out;
}

/** A `bot_status` event (`{kwant_id, status:{…}}`) applied to the map; the same map when it isn't one. */
export function applyBotStatusEvent(map: Record<string, H4Status>, data: unknown): Record<string, H4Status> {
  const d = rec(data);
  const s = parseH4Status(d?.status);
  if (!s || s.kwant_id !== str(d?.kwant_id)) return map;
  return { ...map, [s.kwant_id]: s };
}

export const h4Api = {
  /** Every bot's status, or null on an older server (404) so callers keep today's lines. */
  list: async (space: string): Promise<Record<string, H4Status> | null> => {
    const res = await fetch(`/api/bots/status?space=${encodeURIComponent(space)}`);
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return parseH4List(await res.json());
  },
  /** "Try again" after the stall check gave up: releases the bot's held messages and hand-offs. */
  retry: async (kwantId: string): Promise<{ released: number; status: H4Status | null }> => {
    const res = await personFetch(`/api/kwants/${encodeURIComponent(kwantId)}/status/retry`, { method: 'POST' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const o = rec(await res.json().catch(() => null));
    return { released: num(o?.released) ?? 0, status: parseH4Status(o?.status) };
  },
};

const hhmm = (iso?: string): string | null => {
  const t = iso ? Date.parse(iso) : NaN;
  return Number.isFinite(t) ? new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }) : null;
};

export const isGaveUp = (s?: H4Status | null): boolean => s?.status === 'stuck' && s.reason_code === 'stalled_gave_up';

/** The sidebar's rank-2 line for an H4 stuck bot (H5 copy "Stuck: Claude stopped answering"; tries per §4). */
export function h4StuckLine(s: H4Status): string {
  if (isGaveUp(s)) {
    const at = hhmm(s.stuck?.gave_up_at ?? s.since);
    return `Stuck: stopped after ${s.stuck?.tries ?? 2} tries${at ? ` · ${at}` : ''}`;
  }
  return `Stuck: ${s.waits_on?.label ?? 'the AI'} stopped answering`;
}

/** When the bot became stuck, for "oldest first" inside rank 2. */
export const h4StuckAt = (s: H4Status): number => Date.parse(s.stuck?.gave_up_at ?? s.since ?? '') || 0;

/** When the bot's primary turn started running, for "running longest first" inside rank 4. */
export const h4StartedAt = (s?: H4Status): number | undefined => {
  const t = Date.parse(s?.turn?.started_at ?? '');
  return Number.isFinite(t) ? t : undefined;
};

// ---- H3: a message the server held instead of starting a turn ----

/** `POST /api/chat/send` answered `held`: what the line under the message needs. */
export interface HeldSend {
  /** The optimistic bubble's id, then the saved message's id. */
  tempId: string;
  messageId: string;
  kwantId: string;
  name: string;
  reason: string;
  questionAt?: string;
}

/** A `held` send reply → HeldSend, or null when the send wasn't held. */
export function heldFromReply(reply: unknown, tempId: string): HeldSend | null {
  const o = rec(reply);
  const h = rec(o?.held);
  if (o?.status !== 'held' || !h) return null;
  const kwantId = str(h.kwant_id);
  if (!kwantId) return null;
  return {
    tempId,
    messageId: str(o.user_message_id) ?? tempId,
    kwantId,
    name: str(h.kwant_name) ?? 'The bot',
    reason: str(h.reason_code) ?? '',
    questionAt: str(rec(h.question)?.asked_at),
  };
}

/** Still held: the bot's status lists the message (or no status has arrived since the send). */
export function stillHeld(h: HeldSend, status?: H4Status): boolean {
  if (!status) return true;
  return status.held.some((x) => x.kind === 'message' && (x.id === h.messageId || x.id === h.tempId));
}

/** The line under a held message (Mira's H3 copy) and whether it offers Try again (N3). */
export function heldLine(h: HeldSend, status?: H4Status): { text: string; tryAgain: boolean } {
  const n = h.name;
  if (h.reason === 'waits_on_you') {
    const at = hhmm(h.questionAt);
    return {
      text: h.questionAt
        ? `Queued for ${n}: ${n} is waiting for your answer to its question${at ? ` (${at})` : ''}. It goes through when you answer.`
        : `Queued for ${n}: ${n} is waiting for you in another chat. It goes through when you’re done there.`,
      tryAgain: false,
    };
  }
  if (h.reason === 'paused') return { text: `Waiting for ${n}: ${n} is paused at its monthly budget.`, tryAgain: false };
  // Stuck. Retrying: the retry is already running, so there's nothing to press. Gave up (or not known yet): Try again.
  if (status?.status === 'stuck' && status.reason_code === 'stalled_retrying') {
    return { text: `Waiting for ${n}: ${n} is stuck (${status.waits_on?.label ?? 'the AI'} stopped answering). Your message waits until ${n} runs again.`, tryAgain: false };
  }
  const at = hhmm(status?.stuck?.gave_up_at);
  return {
    text: `Waiting for ${n}: ${n} stopped after ${status?.stuck?.tries ?? 2} tries${at ? ` at ${at}` : ''}. Your message is kept and goes through when ${n} runs again.`,
    tryAgain: true,
  };
}
