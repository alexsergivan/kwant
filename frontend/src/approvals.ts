// Pending tool approvals: one list fed by the live ApprovalRequired event and, after a reload or
// reconnect, by `GET /api/away`'s waiting items, so a card never depends on the page having been
// open when the bot asked. Pure helpers, shared by the app and the node tests.

import { approvalAction, type PendingApproval } from './components/ModernApprovalCard';
import type { AwayItem, AwayLink } from './away';

/** A card plus where it belongs: the asking bot and, when the server knows it, the conversation. */
export interface ChatApproval extends PendingApproval {
  kwant_id: string | null;
  link: AwayLink | null;
}

const str = (v: unknown) => (typeof v === 'string' && v ? v : null);

/** From the live SSE event's data. */
export function approvalFromEvent(data: { id: string; tool_name: string; details?: Record<string, unknown> }): ChatApproval {
  const details = data.details ?? {};
  return { id: data.id, tool_name: data.tool_name, details, kwant_id: str(details.kwant_id), link: null, asked_at: new Date().toISOString() };
}

/**
 * From a "While you were away" approval item. Servers that send `tool_name`/`details` give the full
 * card; older ones only a title ("Use Bash") and a flat summary, which the card shows as text.
 */
export function approvalFromAway(item: AwayItem): ChatApproval {
  const hasDetails = !!item.details && typeof item.details === 'object' && !Array.isArray(item.details);
  const tool = item.tool_name || item.title.replace(/^Use\s+/, '').replace(/\s+\(.*\)$/, '');
  const details: Record<string, unknown> = hasDetails
    ? { kwant_name: item.kwant_name, ...item.details }
    : { source: 'summary', tool, summary: item.summary, kwant_id: item.kwant_id, kwant_name: item.kwant_name };
  // A payment keeps its reason (and so loses its Always button) whichever form the server sent.
  if (str(item.payment_reason) && details.payment_reason === undefined) details.payment_reason = item.payment_reason;
  const link = item.link && (item.link.thread_id || item.link.channel_id || item.link.direct_kwant_id) ? item.link : null;
  const approval: ChatApproval = { id: item.id, tool_name: tool, details, kwant_id: item.kwant_id || str(details.kwant_id), link };
  if (item.status === 'waiting') approval.waiting = true;
  if (str(item.timestamp)) approval.asked_at = item.timestamp;
  return approval;
}

/**
 * An Inbox item's title in words. An approval reads like its card ("Open a pull request on
 * GitHub"), never the server's raw "Use create_pull_request (github)"; other items keep theirs.
 */
export function awayTitle(item: AwayItem): string {
  if (item.kind !== 'approval') return item.title;
  const action = approvalAction(approvalFromAway(item));
  return action.charAt(0).toUpperCase() + action.slice(1);
}

/**
 * What to tell the person after they answered a card. A late Allow on a still-open card starts
 * the bot again (`resumed: "new_turn"`); otherwise null (nothing worth saying).
 */
export function lateAnswerText(reply: unknown, bot: string | null): string | null {
  if (!reply || typeof reply !== 'object') return null;
  const r = reply as { resumed?: unknown; reason?: unknown };
  const who = bot ?? 'The bot';
  if (r.resumed === 'new_turn') return `Allowed. ${who} is picking the work back up.`;
  if (typeof r.reason === 'string' && r.reason) return `Allowed, but ${who} couldn't continue: ${r.reason}.`;
  return null;
}

/** The server's waiting approvals, newest first, as cards. */
export const approvalsFromAway = (waiting: AwayItem[]): ChatApproval[] =>
  waiting.filter((i) => i.kind === 'approval').map(approvalFromAway);

/** Adds or refreshes one card, keeping the richer details when the live event already brought them. */
export function upsertApproval(list: ChatApproval[], next: ChatApproval): ChatApproval[] {
  const i = list.findIndex((a) => a.id === next.id);
  if (i < 0) return [...list, next];
  const prev = list[i];
  const keepPrev = prev.details.source !== 'summary' && next.details.source === 'summary';
  const merged: ChatApproval = keepPrev ? { ...prev, link: next.link ?? prev.link } : { ...next, link: next.link ?? prev.link };
  // Whether the bot still waits on it comes from the newer copy (the server's, on a sync).
  if (next.waiting) merged.waiting = true;
  else delete merged.waiting;
  return list.map((a, j) => (j === i ? merged : a));
}

/**
 * The server's list replaces ours (answered or expired ones drop out), but a card that arrived
 * live after the fetch started stays until the next sync, and live details beat a summary.
 */
export function syncApprovals(list: ChatApproval[], fromServer: ChatApproval[], liveSince: Set<string> = new Set()): ChatApproval[] {
  const live = list.filter((a) => liveSince.has(a.id) && !fromServer.some((s) => s.id === a.id));
  const synced = fromServer.map((s) => {
    const prev = list.find((a) => a.id === s.id);
    return prev ? upsertApproval([prev], s)[0] : s;
  });
  return [...synced, ...live];
}

export const withoutApproval = (list: ChatApproval[], id: string): ChatApproval[] => list.filter((a) => a.id !== id);

/** The open chat: a bot's direct chat (or one of its conversations), or a channel and its members. */
export type OpenChat =
  | { kind: 'kwant'; kwantId: string; threadId?: string | null }
  | { kind: 'channel'; channelId: string; memberIds: string[] };

/**
 * The cards that belong in the open chat. With a conversation link, only there; else the asking
 * bot's direct chat or any channel it's in. A card with no bot at all shows wherever you are.
 */
export function approvalsForChat(list: ChatApproval[], chat: OpenChat | null): ChatApproval[] {
  if (!chat) return [];
  return list.filter((a) => {
    const l = a.link;
    if (l?.channel_id) return chat.kind === 'channel' && chat.channelId === l.channel_id;
    if (l?.thread_id) return chat.kind === 'kwant' && chat.threadId === l.thread_id;
    if (l?.direct_kwant_id) return chat.kind === 'kwant' && chat.kwantId === l.direct_kwant_id;
    if (!a.kwant_id) return true;
    return chat.kind === 'kwant' ? chat.kwantId === a.kwant_id : chat.memberIds.includes(a.kwant_id);
  });
}

// ---- Approvals answered in Telegram (0.7.8, telegram-api.md §3) ----

/** What closed a card, from `ApprovalWithdrawn` or a `GET /api/approvals` row. Old events are `{id}` only. */
export interface ApprovalClosed {
  answered_via?: string | null;
  outcome?: string | null;
}

/** Whether a closed card was allowed in Telegram. Only Allow once is offered there (A22). */
export const allowedInTelegram = (c: ApprovalClosed | null | undefined): boolean =>
  !!c && c.answered_via === 'telegram' && (c.outcome == null || c.outcome === 'allowed_once');

/** A row of `GET /api/approvals`. */
export interface ApprovalRecord {
  id: string;
  kwant_id?: string | null;
  tool_name?: string | null;
  status?: string | null;
  answered_via?: string | null;
  created_at?: string | null;
  details?: unknown;
}

/** How long after it was asked a Telegram-allowed approval is still drawn after a reload. */
export const SETTLED_KEEP_MS = 24 * 60 * 60 * 1000;

/**
 * The rows to draw again after a reload: allowed once in Telegram, asked in the last day. The
 * card comes from the row's own (masked) details, so it reads like the live one.
 */
export function settledFromRecords(rows: unknown, now = Date.now()): ChatApproval[] {
  if (!Array.isArray(rows)) return [];
  const out: ChatApproval[] = [];
  for (const r of rows as ApprovalRecord[]) {
    if (!r || typeof r.id !== 'string' || !allowedInTelegram({ answered_via: r.answered_via, outcome: r.status })) continue;
    const at = str(r.created_at) ? Date.parse(r.created_at as string) : NaN;
    if (!Number.isFinite(at) || now - at > SETTLED_KEEP_MS) continue;
    const details = r.details && typeof r.details === 'object' && !Array.isArray(r.details) ? (r.details as Record<string, unknown>) : {};
    const kwantId = str(r.kwant_id) ?? str(details.kwant_id);
    if (!kwantId) continue; // no chat to put it in
    out.push({ id: r.id, tool_name: str(r.tool_name) ?? 'tool', details, kwant_id: kwantId, link: null, asked_at: r.created_at as string });
  }
  return out.reverse(); // the server sends newest first; the chat reads oldest first
}

/** Adds settled cards once each, keeping the order they came in. */
export function addSettled(list: ChatApproval[], more: ChatApproval[]): ChatApproval[] {
  const seen = new Set(list.map((a) => a.id));
  const add = more.filter((a) => !seen.has(a.id) && (seen.add(a.id), true));
  return add.length ? [...list, ...add] : list;
}

/**
 * `ApprovalWithdrawn {id, answered_via?, outcome?}`: the card always closes; when it was allowed
 * in Telegram it is kept as a settled line. Old `{id}`-only events only close it.
 */
export function withdrawApproval(pending: ChatApproval[], settled: ChatApproval[], data: unknown): { pending: ChatApproval[]; settled: ChatApproval[] } {
  const d = (data && typeof data === 'object' ? data : {}) as { id?: unknown } & ApprovalClosed;
  if (typeof d.id !== 'string') return { pending, settled };
  const card = pending.find((a) => a.id === d.id);
  return {
    pending: card ? withoutApproval(pending, d.id) : pending,
    settled: card && allowedInTelegram(d) ? addSettled(settled, [card]) : settled,
  };
}
