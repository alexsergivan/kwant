/** Where "Answer in chat", a search hit or a notification wants to go: a channel, or a bot's direct chat. */
export interface ChatTarget {
  channel_id: string | null;
  kwant_id: string | null;
}

/** What the chat pane shows right now. */
export interface ShownChat {
  selectedId: string | null;
  isChannel: boolean;
  openThreadId: string | null;
}

/**
 * How to open a target: `gone` when it no longer exists, `select` to switch the chat pane to it,
 * `reveal` when it's already the selected chat and only the page on top (board, Loose ends…) has to close.
 */
export type OpenTargetPlan = 'gone' | 'select' | 'reveal';

export function planOpenTarget(
  t: ChatTarget,
  shown: ShownChat,
  exists: { channel: (id: string) => boolean; kwant: (id: string) => boolean },
): OpenTargetPlan {
  if (t.channel_id) {
    if (!exists.channel(t.channel_id)) return 'gone';
    return shown.isChannel && shown.selectedId === t.channel_id ? 'reveal' : 'select';
  }
  if (!t.kwant_id || !exists.kwant(t.kwant_id)) return 'gone';
  return !shown.isChannel && shown.selectedId === t.kwant_id && !shown.openThreadId ? 'reveal' : 'select';
}

/** What a needs-you notification names (0.8.1 F6): the chat it was asked in, and the card to show there. */
export interface NotifiedChat {
  channel_id?: string | null;
  kwant_id?: string | null;
  thread_id?: string | null;
  card_id?: string | null;
}

/** Where a tap on a notification lands: one of the bot's conversations, a chat, or nowhere. */
export type NotifiedPlace =
  | { kind: 'thread'; kwant_id: string; thread_id: string }
  | { kind: 'chat'; target: ChatTarget }
  | { kind: 'gone' };

/**
 * 0.8.1 F6: the exact place a notification names, else the closest one still there. A channel (a
 * team chat is one) or the bot's conversation first; when that place is gone, the bot's direct
 * chat, so a tap never does nothing while the bot exists. `thread` answers false only when the
 * bot's conversations are known and this one isn't among them (unknown means try it).
 */
export function notifiedPlace(
  n: NotifiedChat,
  exists: { channel: (id: string) => boolean; kwant: (id: string) => boolean; thread?: (kwantId: string, threadId: string) => boolean },
): NotifiedPlace {
  const bot = n.kwant_id && exists.kwant(n.kwant_id) ? n.kwant_id : null;
  if (n.channel_id && exists.channel(n.channel_id)) return { kind: 'chat', target: { channel_id: n.channel_id, kwant_id: null } };
  if (!n.channel_id && bot && n.thread_id && (exists.thread?.(bot, n.thread_id) ?? true)) {
    return { kind: 'thread', kwant_id: bot, thread_id: n.thread_id };
  }
  return bot ? { kind: 'chat', target: { channel_id: null, kwant_id: bot } } : { kind: 'gone' };
}
