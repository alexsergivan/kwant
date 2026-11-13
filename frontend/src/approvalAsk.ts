// reply-chrome part 2 (autonomy/reply-chrome/design.md): a bot reply that ends by asking for the person's OK
// ("once you approve the draft.", "Shall I send it?") gets Approve / Change… under it. Only the last two
// sentences count, so an OK asked for early on and then settled doesn't light the row up.

const ASKS: RegExp[] = [
  /\b(once|when|after|if) you(?:'ve|’ve| have)? (approve|confirm|ok|okay|sign off on|give (me )?the go[- ]ahead)\b/i,
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

/** True when the reply's last two sentences ask the person for an OK and nothing there says it's settled. */
export function asksApproval(text: string | null | undefined): boolean {
  if (!text) return false;
  const tail = lastSentences(text);
  if (!tail) return false;
  if (SETTLED.some((r) => r.test(tail))) return false;
  return ASKS.some((r) => r.test(tail));
}

/** What Approve sends, as an ordinary reply to that message. */
export const APPROVE_TEXT = 'Approved, go ahead.';
/** The composer's placeholder after Change…. */
export const changePlaceholder = (name: string): string => `What should ${name} change?`;
/** The row's caption and group name. */
export const waitingCaption = (name: string): string => `${name} is waiting for your OK`;
