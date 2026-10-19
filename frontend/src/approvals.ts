// Pending tool approvals: one list fed by the live ApprovalRequired event and, after a reload or
// reconnect, by `GET /api/away`'s waiting items, so a card never depends on the page having been
// open when the bot asked. Pure helpers, shared by the app and the node tests.

import type { PendingApproval } from './components/ModernApprovalCard';
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
  return { id: data.id, tool_name: data.tool_name, details, kwant_id: str(details.kwant_id), link: null };
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
  const link = item.link && (item.link.thread_id || item.link.channel_id || item.link.direct_kwant_id) ? item.link : null;
  return { id: item.id, tool_name: tool, details, kwant_id: item.kwant_id || str(details.kwant_id), link };
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
