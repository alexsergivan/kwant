// reply-chrome part 2 (autonomy/reply-chrome/design.md): a bot reply that ends by asking for the person's OK
// ("once you approve the draft.", "Shall I send it?") gets Approve / Change… under it. Only the last two
// sentences count, so an OK asked for early on and then settled doesn't light the row up.

const ASKS: RegExp[] = [
  /\b(once|when|after|if) you(?:'ve|’ve| have)? (approve|confirm(?:ed)?|ok|okay|sign off on|give (me )?the go[- ]ahead)\b/i,
  /\bwaiting (for|on) your (approval|ok|okay|go[- ]ahead|sign[- ]off)\b/i,
  /\b(shall|should|can) i (go ahead|send|schedule|publish|post|launch)\b[^.?!]*\?/i,
  /\b(ok|okay) to (send|schedule|publish|post|go ahead)\b[^.?!]*\?/i,
];
const SETTLED: RegExp[] = [/\b(no need to|don't need to|don’t need to|already) (approve|approved|confirm)/i, /\bapproved\b/i];

/** The text without code, links' targets and markdown marks, so a sentence reads as words. */
function plain(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/`[^`]*`/g, ' ')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[*_>#~]+/g, ' ')
    .replace(/[ \t]+/g, ' ')
    .trim();
}

/** The last two sentences: split after . ! ? (or a line break) followed by a space. */
export function lastSentences(text: string, n = 2): string {
  const parts = plain(text)
    .split(/(?<=[.!?…])\s+|\s*\n\s*/)
    .map((s) => s.trim())
    .filter(Boolean);
  return parts.slice(-n).join(' ');
}

/**
 * True when a reply's meta rules out the Approve / Change… row: a question or a card of its own (`question_id`, `kind`),
 * or a Blocked reply (`blocked`), which shows as Blocked instead. Except a failed step's Blocked (`from: "failed_step"`,
 * 13a): it ends asking in the bot's own words ("If you approve it, I'll run it"), so the one-tap OK stays (Ada, Nov 14).
 */
export function metaHidesApprove(meta: Record<string, unknown> | null | undefined): boolean {
  if (!meta) return false;
  if (meta.question_id || meta.kind) return true;
  const blocked = meta.blocked as { from?: unknown } | null | undefined;
  return !!blocked && blocked.from !== 'failed_step';
}

/** True when the reply's last two sentences ask the person for an OK and nothing there says it's settled. */
export function asksApproval(text: string | null | undefined): boolean {
  if (!text) return false;
  const tail = lastSentences(text);
  if (!tail) return false;
  if (SETTLED.some((r) => r.test(tail))) return false;
  return ASKS.some((r) => r.test(tail));
}

/**
 * The reply that gets Approve / Change…: the newest message, when it's a saved bot reply asking for an OK, nothing
 * else is open (`busy`: a reply running, a question card, an approval card) and the person hasn't already answered
 * it with a 👍 (`answered`, kept by id, so a turn-end reload with fresh copies of the messages doesn't bring it back, 22r).
 */
export function approveRowId(
  last: { id: string; sender_role?: string | null; text?: string | null } | undefined,
  meta: Record<string, unknown> | null | undefined,
  o: { busy: boolean; answered: ReadonlySet<string> },
): string | null {
  if (!last || last.sender_role !== 'kwant' || last.id.startsWith('temp-') || o.busy || o.answered.has(last.id)) return null;
  if (metaHidesApprove(meta)) return null;
  return asksApproval(last.text) ? last.id : null;
}

/** What Approve sends, as an ordinary reply to that message. */
export const APPROVE_TEXT = 'Approved, go ahead.';
/** The composer's placeholder after Change…. */
export const changePlaceholder = (name: string): string => `What should ${name} change?`;
/** The row's caption and group name. */
export const waitingCaption = (name: string): string => `${name} is waiting for your OK`;
