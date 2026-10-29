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
