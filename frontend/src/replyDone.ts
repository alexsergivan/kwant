// reply-chrome part 1 (autonomy/reply-chrome/design.md, Mira): whether a bot reply is a job's final answer, so
// its name line gets a quiet "Done" marker, and the done-check state when the Inbox item carries one. Nothing
// here computes a number; it reads what the server stamped (`meta.chain_done`, the `Done:` line, the Inbox item).

import { type CheckState, knownChecked, type Checked } from './doneCheck';

export interface ReplyDone {
  /** The done text ("Compared both shops' 1 kg price and shipping"), for the marker's tooltip; null when none. */
  text: string | null;
  /** The Inbox item's check (#566/#577), when it has a known state. */
  checked: (Checked & { state: CheckState }) | null;
}

/** The Inbox done items this chat can match on, by the reply they point at. */
export type DoneItems = ReadonlyMap<string, { checked?: unknown; answer_excerpt?: string | null }>;

/**
 * What follows `Done:` on the reply's last non-empty line, the way the server reads it (keep_going::done_line):
 * leading `* _ - > \`` and spaces stripped, the head compared case-insensitively. Null when it doesn't end so.
 */
export function doneLineOf(text: string | null | undefined): string | null {
  if (!text) return null;
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean);
  const last = lines[lines.length - 1];
  if (!last) return null;
  const bare = last.replace(/^[*_\->` ]+/, '');
  if (bare.slice(0, 5).toLowerCase() !== 'done:') return null;
  const what = bare.slice(5).trim().replace(/^[*_` ]+|[*_` ]+$/g, '').trim();
  return what || null;
}

function metaOf(meta: string | null | undefined): Record<string, unknown> | null {
  if (!meta) return null;
  try {
    const v = JSON.parse(meta);
    return v && typeof v === 'object' ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

export interface DoneMsg {
  id: string;
  sender_role: string;
  text: string;
  meta?: string | null;
}

/**
 * The reply's done state, first match wins: the server's `chain_done` stamp, a `Done:` last line, or an Inbox done
 * item whose result is this message. Never on a user message, a temp or streaming one, a blocked reply, or one
 * that asks (`asks`: the Approve row or an open question card is on it).
 */
export function doneOf(msg: DoneMsg, doneItems: DoneItems | null | undefined, opts: { asks?: boolean; streaming?: boolean; failed?: boolean } = {}): ReplyDone | null {
  if (msg.sender_role !== 'kwant' || msg.id.startsWith('temp-') || opts.asks || opts.streaming || opts.failed) return null;
  const meta = metaOf(msg.meta);
  if (meta?.blocked || meta?.interrupted === true || meta?.error) return null;
  const item = doneItems?.get(msg.id);
  const checked = item ? knownChecked(item.checked) : null;
  const stamp = meta?.chain_done;
  if (stamp && typeof stamp === 'object') {
    const t = (stamp as { text?: unknown }).text;
    return { text: typeof t === 'string' && t.trim() ? t.trim() : doneLineOf(msg.text), checked };
  }
  const line = doneLineOf(msg.text);
  if (line) return { text: line, checked };
  if (item) return { text: null, checked };
  return null;
}

/** The Inbox items whose result is a reply, keyed by that reply's id: finished and to-review only. */
export function doneItemsOf(items: ReadonlyArray<{ group: string; result_message_id?: string | null; checked?: unknown; answer_excerpt?: string | null }>): Map<string, { checked?: unknown; answer_excerpt?: string | null }> {
  const out = new Map<string, { checked?: unknown; answer_excerpt?: string | null }>();
  for (const i of items) {
    if ((i.group === 'finished' || i.group === 'to_review') && i.result_message_id && !out.has(i.result_message_id)) out.set(i.result_message_id, i);
  }
  return out;
}

/** The marker's words per check state (the Inbox list's words, done-check design section 2); plain "Done" without one. */
export const DONE_WORD: Record<CheckState | 'done', string> = {
  done: 'Done',
  checked: 'Checked',
  partly: 'Partly checked',
  failed: 'Check failed',
  not_checked: 'Not checked',
};

/** The marker's screen-reader name. */
export function doneLabel(d: ReplyDone): string {
  switch (d.checked?.state) {
    case 'checked':
      return 'Checked: this job is finished and every check passed';
    case 'partly':
      return 'Partly checked: this job is finished and some checks passed';
    case 'failed':
      return 'Check failed: this job is finished but a check failed';
    case 'not_checked':
      return 'Done: this job is finished, nothing verified it';
    default:
      return 'Done: this job is finished';
  }
}
