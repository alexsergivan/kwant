// Folds the back-and-forth of a finished team discussion into one collapsible thread, so the chat
// shows the team's summary instead of every intermediate turn. Labels come from src/server/discussion.rs.

export const TURN_LABEL = 'Team Discussion';
export const SUMMARY_LABEL = 'Team Summary';

interface LabeledMessage {
  id: string;
  sender_id: string;
  reasoning_content?: string | null;
}

export type ChatItem<M> =
  | { kind: 'message'; message: M }
  | { kind: 'thread'; id: string; turns: M[]; speakers: string[] };

const isTurn = (m: LabeledMessage) => !!m.reasoning_content?.startsWith(TURN_LABEL);
const isSummary = (m: LabeledMessage) => !!m.reasoning_content?.startsWith(SUMMARY_LABEL);

/** Groups runs of discussion turns that end in a team summary; turns still in progress stay inline. */
export function groupDiscussionThreads<M extends LabeledMessage>(messages: M[]): ChatItem<M>[] {
  const items: ChatItem<M>[] = [];
  let i = 0;
  while (i < messages.length) {
    if (!isTurn(messages[i])) {
      items.push({ kind: 'message', message: messages[i] });
      i += 1;
      continue;
    }
    let j = i;
    while (j < messages.length && isTurn(messages[j])) j += 1;
    const turns = messages.slice(i, j);
    if (j < messages.length && isSummary(messages[j])) {
      const speakers = Array.from(new Set(turns.map((t) => t.sender_id)));
      items.push({ kind: 'thread', id: turns[0].id, turns, speakers });
    } else {
      turns.forEach((message) => items.push({ kind: 'message', message }));
    }
    i = j;
  }
  return items;
}

/** Hides the reply markers kwants use to pass or to ask the user first. */
export function stripDiscussionMarkers(text: string): string {
  return text.replace(/\[(PASS|ASK_USER)\]/gi, '').trimStart();
}
