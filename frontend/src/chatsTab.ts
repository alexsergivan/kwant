// The phone's Chats tab lists everything you talk to: who needs you first, then who is working,
// then channels, team chats and every other bot's direct chat (spec 0.6.13 B1).
import type { AwayItem } from './away';
import type { InboxItem } from './api/inbox';

/** One bot waiting on you: the newest approval or question it asked, or an open browser step. */
export interface NeedsYouRow {
  kwant_id: string;
  /** The amber status line: "Wants to open a pull request", "Asked you a question", "Needs you in the browser". */
  line: string;
  /** Opens the chat with this card; none for a browser step (its chat shows the step). */
  item?: AwayItem;
}

// Tools people approve most, in words; anything else falls back to "Wants your approval".
const TOOL_WORDS: Record<string, string> = {
  create_pull_request: 'open a pull request',
  merge_pull_request: 'merge a pull request',
  create_issue: 'open an issue',
  push_files: 'push files',
  create_branch: 'create a branch',
  send_email: 'send an email',
  send_message: 'send a message',
  create_event: 'create a calendar event',
  delete_file: 'delete a file',
  write_file: 'write a file',
  bash: 'run a command',
};

/** "mcp__github__create_pull_request" → "Wants to open a pull request". */
export const approvalLine = (tool?: string | null): string => {
  const bare = (tool ?? '').split('__').pop()!.toLowerCase();
  const words = TOOL_WORDS[bare];
  return words ? `Wants to ${words}` : 'Wants your approval';
};

/**
 * Bots that need you, in the order they asked, one row each. An approval wins over a question,
 * a question over a browser step: the line names the thing that blocks the bot hardest.
 */
export function needsYouRows(waiting: AwayItem[], openSteps: Record<string, number>, kwantIds: Set<string>): NeedsYouRow[] {
  const rank = (r: NeedsYouRow) => (r.item?.kind === 'approval' ? 0 : r.item ? 1 : 2);
  const rows = new Map<string, NeedsYouRow>();
  const put = (r: NeedsYouRow) => {
    if (!kwantIds.has(r.kwant_id)) return;
    const had = rows.get(r.kwant_id);
    if (!had || rank(r) < rank(had)) rows.set(r.kwant_id, r);
  };
  for (const item of waiting) {
    if (item.kind === 'approval') put({ kwant_id: item.kwant_id, line: approvalLine(item.tool_name), item });
    else if (item.kind === 'question') put({ kwant_id: item.kwant_id, line: 'Asked you a question', item });
  }
  for (const [id, n] of Object.entries(openSteps)) if (n > 0) put({ kwant_id: id, line: 'Needs you in the browser' });
  return [...rows.values()];
}

/** Working: bots with a live turn that are not already under Needs you, by name. */
export const workingIds = <K extends { id: string; name: string }>(kwants: K[], busy: Record<string, string>, needs: Set<string>): K[] =>
  kwants.filter((k) => busy[k.id] && !needs.has(k.id)).sort((a, b) => a.name.localeCompare(b.name));

/** Direct chats: every other bot, by name (the client can't tell which direct chats have messages yet). */
export const directIds = <K extends { id: string; name: string }>(kwants: K[], shown: Set<string>): K[] =>
  kwants.filter((k) => !shown.has(k.id)).sort((a, b) => a.name.localeCompare(b.name));

// ---- H5: the one attention order, used by the sidebar block, the phone sections and the Inbox ----

/**
 * 1 Needs you · 2 Stuck or failed · 3 To review (0.9 V1) · 4 Finished, not read · 5 Working · 6 Waiting in
 * line · 7 everyone else. A bot sits in the highest group it qualifies for.
 */
export type AttentionRank = 1 | 2 | 3 | 4 | 5 | 6 | 7;

/** The ranks pinned in "Needs attention" on desktop, and given a dot on team rollups. */
export const PINNED_RANK = 4;

export interface AttentionRow<K> {
  kwant: K;
  rank: AttentionRank;
  /** The status line for ranks 1–4 ("Asked you a question", "Stuck: …", "To review: …", "Finished: …"). */
  line?: string;
  /** Ranks 1–4: the ask or Inbox item it comes from (none for a browser step). */
  item?: AwayItem;
  /** Set when the row comes from a review item: clicking it opens that item in the Inbox. */
  review?: InboxItem;
  needs?: NeedsYouRow;
  /** When it asked, failed or finished, for the order inside the group. */
  at?: number;
}

export interface AttentionInput<K> {
  kwants: K[];
  /** `needsYouRows()`: already oldest first. */
  needs: NeedsYouRow[];
  /** Inbox `failed` items (until H4 ships its `stuck` status, the only source of rank 2). */
  failed?: AwayItem[];
  /** Inbox `finished` items not seen yet. */
  finished?: AwayItem[];
  /** 0.9 V1: the Inbox's review items (any group), including the Low-risk card's. */
  reviews?: InboxItem[];
  /** Live turns per bot (`busyLines`), and each bot's dot status (queued vs working). */
  busy?: Record<string, string>;
  status?: Record<string, string>;
  /** When the person last opened each bot's chat: a finished item older than that has been read. */
  openedAt?: Record<string, number>;
  now?: number;
}

const FAILED_WINDOW_MS = 24 * 3600_000;
const ts = (i?: AwayItem) => (i ? Date.parse(i.timestamp) || 0 : 0);

/** Rank 2's line: what broke, in the Inbox's words. */
export const stuckLine = (i: AwayItem): string => {
  const what = i.status === 'stuck' ? 'Stuck' : i.kind === 'routine_run' ? 'Routine failed' : i.status === 'interrupted' ? 'Interrupted' : 'Failed';
  return i.title ? `${what}: ${i.title}` : what;
};

export const finishedLine = (i: AwayItem): string => (i.title ? `Finished: ${i.title}` : 'Finished');

/** A review item's rank and line, from the Inbox group it sits in. */
export function reviewRank(i: InboxItem): { rank: 1 | 2 | 3; line: string } {
  if (i.group === 'waiting_on_you') {
    const yourCall = i.verdict?.result === 'fail' && !!i.round && i.round.n >= i.round.max;
    return { rank: 1, line: `${yourCall ? 'Your call' : 'Needs you'}: ${i.title}` };
  }
  if (i.group === 'failed_or_stuck') return { rank: 2, line: `Couldn’t approve: ${i.title}` };
  return { rank: 3, line: `To review: ${i.title}` };
}

/** One rank for one bot (the rules; `attentionRows` applies them to everyone). */
export function attentionRank(bot: { needs?: boolean; failed?: boolean; review?: boolean; finished?: boolean; working?: boolean; queued?: boolean }): AttentionRank {
  if (bot.needs) return 1;
  if (bot.failed) return 2;
  if (bot.review) return 3;
  if (bot.finished) return 4;
  if (bot.working) return 5;
  if (bot.queued) return 6;
  return 7;
}

/**
 * The order inside and across groups: rank first; then oldest first for 1–3, newest first for 4;
 * 5 and 6 by name (the sidebar has no turn start times); ties by name A→Z. Rank 7 keeps its input order.
 */
export function attentionSort<K extends { name: string }>(rows: AttentionRow<K>[]): AttentionRow<K>[] {
  const pos = new Map(rows.map((r, i) => [r, i]));
  return [...rows].sort((a, b) => {
    if (a.rank !== b.rank) return a.rank - b.rank;
    if (a.rank === 7) return pos.get(a)! - pos.get(b)!;
    if (a.rank <= 3 && a.at !== b.at) return (a.at ?? 0) - (b.at ?? 0);
    if (a.rank === 4 && a.at !== b.at) return (b.at ?? 0) - (a.at ?? 0);
    return a.kwant.name.localeCompare(b.kwant.name);
  });
}

/** Every bot with its rank and line, in the one order. */
export function attentionRows<K extends { id: string; name: string }>(input: AttentionInput<K>): AttentionRow<K>[] {
  const { kwants, needs, failed = [], finished = [], reviews = [], busy = {}, status = {}, openedAt = {}, now = Date.now() } = input;
  const needsBy = new Map(needs.map((r, i) => [r.kwant_id, { r, i }]));
  // Oldest failure per bot from the last 24 h; newest finished item per bot not read since.
  const failedBy = new Map<string, AwayItem>();
  for (const i of failed) {
    const t = ts(i);
    if (now - t > FAILED_WINDOW_MS) continue;
    const had = failedBy.get(i.kwant_id);
    if (!had || t < ts(had)) failedBy.set(i.kwant_id, i);
  }
  const finishedBy = new Map<string, AwayItem>();
  for (const i of finished) {
    const t = ts(i);
    if ((openedAt[i.kwant_id] ?? 0) >= t) continue;
    const had = finishedBy.get(i.kwant_id);
    if (!had || t > ts(had)) finishedBy.set(i.kwant_id, i);
  }
  // Review items: each bot's highest-ranked one, oldest first inside a rank.
  const reviewBy = new Map<string, { i: InboxItem; rank: 1 | 2 | 3; line: string }>();
  for (const i of reviews) {
    if (i.kind !== 'review') continue;
    const r = reviewRank(i);
    const had = reviewBy.get(i.kwant_id);
    if (!had || r.rank < had.rank || (r.rank === had.rank && ts(i as AwayItem) < ts(had.i as AwayItem))) reviewBy.set(i.kwant_id, { i, ...r });
  }
  const rows = kwants.map((k): AttentionRow<K> => {
    const n = needsBy.get(k.id);
    const f = failedBy.get(k.id);
    const d = finishedBy.get(k.id);
    const v = reviewBy.get(k.id);
    const queued = status[k.id] === 'queued';
    const rank = attentionRank({
      needs: !!n || v?.rank === 1,
      failed: !!f || v?.rank === 2,
      review: v?.rank === 3,
      finished: !!d,
      working: !!busy[k.id] && !queued,
      queued: !!busy[k.id] && queued,
    });
    const fromReview = (x: { i: InboxItem; line: string }): AttentionRow<K> => ({ kwant: k, rank, line: x.line, item: x.i as AwayItem, review: x.i, at: ts(x.i as AwayItem) });
    if (rank === 1) return n ? { kwant: k, rank, line: n.r.line, item: n.r.item, needs: n.r, at: n.r.item ? ts(n.r.item) : n.i } : fromReview(v!);
    if (rank === 2) return f && !(v?.rank === 2 && ts(v.i as AwayItem) < ts(f)) ? { kwant: k, rank, line: stuckLine(f), item: f, at: ts(f) } : fromReview(v!);
    if (rank === 3) return fromReview(v!);
    if (rank === 4) return { kwant: k, rank, line: finishedLine(d!), item: d, at: ts(d) };
    return { kwant: k, rank };
  });
  // Needs-you rows keep needsYouRows' own order (oldest ask first; a browser step has no time); a review
  // that needs you comes after them, oldest first.
  const sorted = attentionSort(rows);
  const ones = sorted.filter((r) => r.rank === 1);
  const asks = ones.filter((r) => !r.review).sort((a, b) => needsBy.get(a.kwant.id)!.i - needsBy.get(b.kwant.id)!.i);
  return [...asks, ...ones.filter((r) => r.review), ...sorted.filter((r) => r.rank !== 1)];
}

/** "2 need you, 1 stuck, 1 to review, 1 finished" (zeros left out). */
export function rollupLabel(counts: { needs: number; stuck: number; review?: number; finished: number; working?: number }): string {
  const parts: string[] = [];
  if (counts.needs) parts.push(`${counts.needs} ${counts.needs === 1 ? 'needs' : 'need'} you`);
  if (counts.stuck) parts.push(`${counts.stuck} stuck`);
  if (counts.review) parts.push(`${counts.review} to review`);
  if (counts.finished) parts.push(`${counts.finished} finished`);
  if (!parts.length && counts.working) parts.push(`${counts.working} working`);
  return parts.join(', ');
}
