// Continue in a new chat from here (roadmap Next 12d; autonomy/continue-chat/brief.md, "Frontend (Iris)").
// A message's overflow offers it in a chat with one bot; the server copies every message up to the pressed one
// into a new thread of the same bot (POST /api/messages/:id/continue, Lin) and the app opens it.
// Pure helpers plus one fetch, so the node tests can load it on its own.

/** The fields a continued thread carries (Lin's thread_view); all null or absent on any other thread. */
export interface ForkFields {
  /** The chat it came from: a thread id, or the bot's id for its main chat. */
  forked_from_context?: string | null;
  /** The message pressed, in that chat. */
  forked_from_message_id?: string | null;
  /** How many messages were copied. */
  forked_copied?: number | null;
  /** True when only the newest 1,000 were copied. */
  forked_truncated?: boolean | null;
}

/** The 201: the new thread plus `copied` and `truncated` at the top level. */
export type ContinueResult<T> = T & ForkFields & { id: string; kwant_id: string; copied?: number; truncated?: boolean };

/** The newest messages the server copies; more are cut and the line says so. */
export const CONTINUE_CAP = 1000;

export const CONTINUE_LABEL = 'Continue in a new chat from here';

/**
 * Where the item shows. Only a chat with one bot: its main chat or one of its `chat` threads (not a delegation,
 * a team or any channel). Not on a message still being written (a `temp-` id, or the newest one while a reply
 * streams), not on a system line, and not once the server has said this session is a bot's (403).
 */
export function canContinueFrom(o: {
  /** The open chat: 'bot' for a bot's own chat, anything else hides it. */
  chatKind: string | null | undefined;
  /** The open thread's kind; null or undefined in the bot's main chat. */
  threadKind?: string | null;
  /** A team chat shown in the bot's pane. */
  inTeam?: boolean;
  message: { id: string; sender_role: string };
  /** True for the newest message of the chat. */
  newest: boolean;
  streaming: boolean;
  /** A bot-token session: the route answered 403 once. */
  botSession: boolean;
}): boolean {
  if (o.chatKind !== 'bot' || o.inTeam || o.botSession) return false;
  if (o.threadKind && o.threadKind !== 'chat') return false;
  const m = o.message;
  if (!m?.id || m.id.startsWith('temp-')) return false;
  if (m.sender_role !== 'user' && m.sender_role !== 'kwant') return false;
  if (o.streaming && o.newest) return false;
  return true;
}

/** The quiet first line of a continued chat. `count` unknown (an older server, a reload before Lin's fields): no count. */
export function continuedLine(title: string, count: number | null | undefined, truncated?: boolean | null): string {
  const t = title.trim() || 'the earlier chat';
  const n = typeof count === 'number' && Number.isFinite(count) && count >= 0 ? Math.floor(count) : null;
  const words = n === null ? '' : ` · ${n.toLocaleString('en-US')} ${n === 1 ? 'message' : 'messages'}`;
  return `Continued from "${t}"${words}${truncated ? ` (the newest ${CONTINUE_CAP.toLocaleString('en-US')})` : ''}`;
}

/** The fork a thread records, or null when it wasn't continued from another chat. */
export function forkOf(t: ForkFields | null | undefined): { context: string; messageId: string; copied: number | null; truncated: boolean } | null {
  if (!t || !t.forked_from_context || !t.forked_from_message_id) return null;
  return {
    context: t.forked_from_context,
    messageId: t.forked_from_message_id,
    copied: typeof t.forked_copied === 'number' ? t.forked_copied : null,
    truncated: t.forked_truncated === true,
  };
}

/** The one plain line by the message when the request fails. */
export const continueErrorLine = (err: unknown): string => {
  const raw = err instanceof Error ? err.message : typeof err === 'string' ? err : '';
  const why = raw.trim().replace(/[.\s]+$/, '') || 'something went wrong';
  return `Couldn't start a new chat: ${why}.`;
};

/** Thrown with the server's words and status; a 403 means this session belongs to a bot. */
export class ContinueError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

/** The failure's words: the JSON `{error}` when there is one, else the status in plain words. */
export async function continueFailure(res: Response): Promise<ContinueError> {
  let message =
    res.status === 404 ? 'That message is no longer there' : res.status === 403 ? 'Only you can do this, not a bot' : `the server answered ${res.status}`;
  try {
    const data = await res.json();
    if (data && typeof data.error === 'string' && data.error.trim()) message = data.error.trim();
  } catch {
    // no JSON body
  }
  return new ContinueError(message, res.status);
}

/** POST /api/messages/:id/continue. `send` is `personFetch` in the app (it asks "Confirm it's you" on a 401). */
export async function continueFromMessage<T = object>(id: string, send: (url: string, init?: RequestInit) => Promise<Response>): Promise<ContinueResult<T>> {
  let res: Response;
  try {
    res = await send(`/api/messages/${encodeURIComponent(id)}/continue`, { method: 'POST' });
  } catch (err) {
    if (err instanceof Error && err.name !== 'TypeError') throw err;
    throw new ContinueError("the server didn't answer", 0);
  }
  if (res.status !== 201 && res.status !== 200) throw await continueFailure(res);
  const body = (await res.json()) as ContinueResult<T>;
  if (!body || typeof body.id !== 'string' || typeof body.kwant_id !== 'string') throw new ContinueError('the server sent back no chat', res.status);
  // The thread view carries the fork fields on newer builds; the top-level copy keeps older ones working.
  return {
    ...body,
    forked_copied: typeof body.forked_copied === 'number' ? body.forked_copied : typeof body.copied === 'number' ? body.copied : null,
    forked_truncated: typeof body.forked_truncated === 'boolean' ? body.forked_truncated : body.truncated === true,
  };
}

/** Next to `threadsApi`; kept here so threads.ts stays import-free for its tests. */
export const continueChatApi = {
  fromMessage: <T = object>(id: string, send: (url: string, init?: RequestInit) => Promise<Response> = (u, i) => fetch(u, i)) => continueFromMessage<T>(id, send),
};
