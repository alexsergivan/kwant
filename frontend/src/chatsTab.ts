// The phone's Chats tab lists everything you talk to: who needs you first, then who is working,
// then channels, team chats and every other bot's direct chat (spec 0.6.13 B1).
import type { AwayItem } from './away';

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
