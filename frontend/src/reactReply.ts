// Reactions and replies on bot messages (react-reply contract, section 3).
// Pure helpers, so the toggle, the excerpt and the save-with-rollback are testable without a DOM.

export type Reaction = 'up' | 'down';

/** What a reply points at: the server's shape, or a target that was deleted. */
export interface ReplyRef {
  id: string;
  sender_role?: string;
  sender_id?: string;
  excerpt?: string;
  missing?: boolean;
}

/** Older servers (before the backend PR) leave both fields out; anything unexpected reads as none. */
export function reactionOf(value: unknown): Reaction | null {
  return value === 'up' || value === 'down' ? value : null;
}

export function replyRefOf(value: unknown): ReplyRef | null {
  if (!value || typeof value !== 'object') return null;
  const v = value as Record<string, unknown>;
  if (typeof v.id !== 'string' || !v.id) return null;
  if (v.missing === true) return { id: v.id, missing: true };
  return {
    id: v.id,
    sender_role: typeof v.sender_role === 'string' ? v.sender_role : undefined,
    sender_id: typeof v.sender_id === 'string' ? v.sender_id : undefined,
    excerpt: typeof v.excerpt === 'string' ? v.excerpt : '',
  };
}

/** Pressing the selected reaction clears it; pressing the other one switches. */
export function nextReaction(current: Reaction | null, pressed: Reaction): Reaction | null {
  return current === pressed ? null : pressed;
}

/**
 * One line of a message for the reply chip and the optimistic quote: markdown marks dropped,
 * whitespace folded, cut at `max` characters with an ellipsis. Display only; the server's excerpt wins once it arrives.
 */
export function excerptOf(text: string, max = 120): string {
  const plain = text
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+\.)\s+/gm, '')
    .replace(/(\*\*|__|\*|_|~~)(?=\S)([^*_~]*?\S)\1/g, '$2')
    .replace(/\s+/g, ' ')
    .trim();
  if (plain.length <= max) return plain;
  return `${plain.slice(0, max - 1).trimEnd()}…`;
}

/** Who a message or quote is from, as the chat shows it: long ids (uuids) fall back to the chat's title. */
export function senderLabel(senderRole: string | undefined, senderId: string | undefined, fallback: string): string {
  if (senderRole === 'user') return 'You';
  if (!senderId) return fallback;
  return senderId.length > 20 ? fallback : senderId;
}

/** The reply a just-sent message carries until the server's own copy replaces it. */
export function optimisticReplyRef(target: { id: string; sender_role: string; sender_id: string; text: string }): ReplyRef {
  return { id: target.id, sender_role: target.sender_role, sender_id: target.sender_id, excerpt: excerptOf(target.text, 200) };
}

export type FetchLike = (url: string, init: { method: string; headers: Record<string, string>; body: string }) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

/** What a reaction save came to; `answered` means it started a turn as the person's yes or no. */
export interface ReactResult {
  ok: boolean;
  answered: boolean;
  error?: string;
}

/**
 * Saves a reaction. Resolves to the server's value and whether it answered the bot's question
 * (servers before the backend PR leave `answered` out: read as false), or throws with a short reason.
 */
export async function saveReaction(
  fetcher: FetchLike,
  messageId: string,
  reaction: Reaction | null,
): Promise<{ reaction: Reaction | null; answered: boolean }> {
  const res = await fetcher(`/api/messages/${encodeURIComponent(messageId)}/reaction`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ reaction }),
  });
  if (!res.ok) {
    let reason = res.status === 404 ? 'That message is gone.' : `HTTP ${res.status}`;
    try {
      const data = (await res.json()) as { error?: unknown };
      if (typeof data?.error === 'string' && data.error) reason = data.error;
    } catch {
      // no JSON body
    }
    throw new Error(reason);
  }
  const data = (await res.json().catch(() => null)) as { reaction?: unknown; answered?: unknown } | null;
  return {
    reaction: data && 'reaction' in data ? reactionOf(data.reaction) : reaction,
    answered: data?.answered === true,
  };
}

/** The last paragraph has a question mark: the server's own test for "this message asks something". */
export function endsWithQuestion(text: string): boolean {
  const paras = text
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter(Boolean);
  return (paras[paras.length - 1] ?? '').includes('?');
}

/**
 * The id of the bot message a thumb would answer: the newest message from a bot, with nothing from the
 * person after it, whose last paragraph asks something. Null while a reply is running.
 */
export function questionMessageId(
  messages: ReadonlyArray<{ id: string; sender_role: string; text: string }>,
  running: boolean,
): string | null {
  if (running) return null;
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.sender_role === 'user') return null;
    if (m.sender_role === 'kwant') return !m.id.startsWith('temp-') && endsWithQuestion(m.text) ? m.id : null;
  }
  return null;
}

/** The two thumbs' words: a yes or no on the bot's open question, otherwise feedback. */
export function reactionLabels(asks: boolean): Record<Reaction, string> {
  return asks ? { up: 'Yes, go ahead', down: 'No' } : { up: 'Good reply', down: 'Not helpful' };
}

/** A bot's name for a quote or the chip: from the bot list by id or name, else a short sender id, else "a bot". */
export function botNameOf(senderId: string | undefined, bots: ReadonlyArray<{ id: string; name: string }>): string {
  if (!senderId) return 'a bot';
  const known = bots.find((b) => b.id === senderId || b.name === senderId);
  if (known) return known.name;
  return senderId.length > 20 ? 'a bot' : senderId;
}

/** The message id and reaction a live update carries, whatever the event names them. */
export function reactionEventOf(data: unknown): { id: string; reaction: Reaction | null } | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  const id = typeof d.message_id === 'string' ? d.message_id : typeof d.id === 'string' ? d.id : null;
  if (!id || !('reaction' in d)) return null;
  return { id, reaction: reactionOf(d.reaction) };
}
