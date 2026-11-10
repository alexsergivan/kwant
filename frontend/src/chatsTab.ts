// The phone's Chats tab lists everything you talk to: who needs you first, then who is working,
// then channels, team chats and every other bot's direct chat (spec 0.6.13 B1).
import type { AwayItem } from './away';
import { type H4Status, h4StuckAt, h4StuckLine, h4StartedAt } from './h4Status';

/** One bot waiting on you: the newest approval or question it asked, or an open browser step. */
export interface NeedsYouRow {
  kwant_id: string;
  /** The amber status line: "Wants to open a pull request", "Asked you a question", "Needs you in the browser". */
  line: string;
  /** Opens the chat with this card; none for a browser step (its chat shows the step). */
  item?: AwayItem;
  /** When this bot's oldest open ask began (any kind); none for a browser step alone. */
  at?: number;
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
 * Bots that need you, oldest ask first (S1: by how long the ask has waited, whatever its kind), one row
 * each. The line names the thing that blocks the bot hardest (an approval over a question over a browser
 * step), but the row's time is the bot's oldest open ask. A browser step alone has no time and goes last.
 */
export function needsYouRows(waiting: AwayItem[], openSteps: Record<string, number>, kwantIds: Set<string>): NeedsYouRow[] {
  const rank = (r: NeedsYouRow) => (r.item?.kind === 'approval' ? 0 : r.item ? 1 : 2);
  const rows = new Map<string, NeedsYouRow>();
  const put = (r: NeedsYouRow) => {
    if (!kwantIds.has(r.kwant_id)) return;
    const had = rows.get(r.kwant_id);
    const t = r.item ? Date.parse(r.item.timestamp) : NaN;
    const at = Number.isFinite(t) ? (had?.at !== undefined ? Math.min(had.at, t) : t) : had?.at;
    rows.set(r.kwant_id, { ...(!had || rank(r) < rank(had) ? r : had), ...(at !== undefined ? { at } : {}) });
  };
  for (const item of waiting) {
    if (item.kind === 'approval') put({ kwant_id: item.kwant_id, line: approvalLine(item.tool_name), item });
    else if (item.kind === 'question') put({ kwant_id: item.kwant_id, line: 'Asked you a question', item });
  }
  for (const [id, n] of Object.entries(openSteps)) if (n > 0) put({ kwant_id: id, line: 'Needs you in the browser' });
  const last = Number.POSITIVE_INFINITY;
  return [...rows.values()].sort((a, b) => (a.at ?? last) - (b.at ?? last));
}

/** Working: bots with a live turn that are not already under Needs you, by name. */
export const workingIds = <K extends { id: string; name: string }>(kwants: K[], busy: Record<string, string>, needs: Set<string>): K[] =>
  kwants.filter((k) => busy[k.id] && !needs.has(k.id)).sort((a, b) => a.name.localeCompare(b.name));

/** Direct chats: every other bot, by name (the client can't tell which direct chats have messages yet). */
export const directIds = <K extends { id: string; name: string }>(kwants: K[], shown: Set<string>): K[] =>
  kwants.filter((k) => !shown.has(k.id)).sort((a, b) => a.name.localeCompare(b.name));

// ---- H5: the one attention order, used by the sidebar block, the phone sections and the Inbox ----

/**
 * 1 Needs you · 2 Stuck or failed · 3 Finished, not read · 4 Working · 5 Waiting in line · 6 everyone else.
 * A bot sits in the highest group it qualifies for.
 */
export type AttentionRank = 1 | 2 | 3 | 4 | 5 | 6;

export interface AttentionRow<K> {
  kwant: K;
  rank: AttentionRank;
  /** The status line for ranks 1–3 ("Asked you a question", "Stuck: …", "Finished: …"). */
  line?: string;
  /** Ranks 1–3: the ask or Inbox item it comes from (none for a browser step). */
  item?: AwayItem;
  needs?: NeedsYouRow;
  /** When it asked, failed or finished, for the order inside the group. */
  at?: number;
}

export interface AttentionInput<K> {
  kwants: K[];
  /** `needsYouRows()`: each row carries its oldest ask's time. */
  needs: NeedsYouRow[];
  /** Inbox `failed` items from the last 24 h: rank 2 with H4's `stuck`. */
  failed?: AwayItem[];
  /** H4 status per bot (#499): `stuck` is rank 2 without an Inbox item; a turn's start orders rank 4. */
  h4?: Record<string, H4Status>;
  /** Inbox `finished` items not seen yet. */
  finished?: AwayItem[];
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

/** One rank for one bot (the rules; `attentionRows` applies them to everyone). */
export function attentionRank(bot: { needs?: boolean; failed?: boolean; finished?: boolean; working?: boolean; queued?: boolean }): AttentionRank {
  if (bot.needs) return 1;
  if (bot.failed) return 2;
  if (bot.finished) return 3;
  if (bot.working) return 4;
  if (bot.queued) return 5;
  return 6;
}

/**
 * The order inside and across groups: rank first; then oldest first for 1 and 2 and running longest
 * first for 4 (a row without a time after those with one), newest first for 3; 5 by name; ties by
 * name A→Z. Rank 6 keeps its input order.
 */
export function attentionSort<K extends { name: string }>(rows: AttentionRow<K>[]): AttentionRow<K>[] {
  const pos = new Map(rows.map((r, i) => [r, i]));
  const last = Number.POSITIVE_INFINITY;
  return [...rows].sort((a, b) => {
    if (a.rank !== b.rank) return a.rank - b.rank;
    if (a.rank === 6) return pos.get(a)! - pos.get(b)!;
    if ((a.rank <= 2 || a.rank === 4) && a.at !== b.at) return (a.at ?? last) - (b.at ?? last);
    if (a.rank === 3 && a.at !== b.at) return (b.at ?? 0) - (a.at ?? 0);
    return a.kwant.name.localeCompare(b.kwant.name);
  });
}

/** Every bot with its rank and line, in the one order. */
export function attentionRows<K extends { id: string; name: string }>(input: AttentionInput<K>): AttentionRow<K>[] {
  const { kwants, needs, failed = [], finished = [], busy = {}, status = {}, h4 = {}, openedAt = {}, now = Date.now() } = input;
  const needsBy = new Map(needs.map((r) => [r.kwant_id, r]));
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
  const rows = kwants.map((k): AttentionRow<K> => {
    const n = needsBy.get(k.id);
    const f = failedBy.get(k.id);
    const d = finishedBy.get(k.id);
    const s = h4[k.id];
    const stuck = s?.status === 'stuck' ? s : undefined;
    const queued = status[k.id] === 'queued';
    const rank = attentionRank({ needs: !!n, failed: !!f || !!stuck, finished: !!d, working: !!busy[k.id] && !queued, queued: !!busy[k.id] && queued });
    if (rank === 1) return { kwant: k, rank, line: n!.line, item: n!.item, needs: n, at: n!.at };
    if (rank === 2) {
      // A live stuck status says more than an Inbox failure; the group order uses the older of the two.
      const at = Math.min(stuck ? h4StuckAt(stuck) : Infinity, f ? ts(f) : Infinity);
      return { kwant: k, rank, line: stuck ? h4StuckLine(stuck) : stuckLine(f!), item: f, at };
    }
    if (rank === 3) return { kwant: k, rank, line: finishedLine(d!), item: d, at: ts(d) };
    if (rank === 4) return { kwant: k, rank, at: h4StartedAt(s) };
    return { kwant: k, rank };
  });
  return attentionSort(rows);
}

/** "2 need you, 1 stuck, 1 finished" (zeros left out). */
export function rollupLabel(counts: { needs: number; stuck: number; finished: number; working?: number }): string {
  const parts: string[] = [];
  if (counts.needs) parts.push(`${counts.needs} ${counts.needs === 1 ? 'needs' : 'need'} you`);
  if (counts.stuck) parts.push(`${counts.stuck} stuck`);
  if (counts.finished) parts.push(`${counts.finished} finished`);
  if (!parts.length && counts.working) parts.push(`${counts.working} working`);
  return parts.join(', ');
}
