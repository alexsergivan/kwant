// 22i "a Needs-attention row always shows what it needs": where a row (or its Inbox card) lands, the
// one action that resolves it, and which open asks belong in the chat on screen (the "Needs you"
// strip above the composer). Pure, so the node tests cover them without a DOM.
import { awayTarget, type AwayItem } from './away';
import { parseAttachments } from './attachments';
import type { ChatMessage } from './components/ChatView';
import { retryTargets, type RetryTarget } from './interruptions';

/** What resolves an item. Servers with Lin's 22i part send it as `action`; older ones get it inferred. */
export type AskAction = 'retry' | 'continue' | 'ask_again' | 'answer' | 'decide' | 'browser';

const ACTIONS: ReadonlySet<string> = new Set(['retry', 'continue', 'ask_again', 'answer', 'decide', 'browser']);

/** The kinds an ask in a chat can be: these get a strip and land on their message. */
const ASK_KINDS: ReadonlySet<string> = new Set(['question', 'approval', 'stopped']);

export const isAsk = (item: Pick<AwayItem, 'kind'>): boolean => ASK_KINDS.has(item.kind);

/** Shown in the strip: every Needs-you item that lives in a chat, except a sign-in to redo (its fix is in the Inbox). */
export const onStrip = (item: Pick<AwayItem, 'kind'>): boolean => item.kind !== 'connection';

/**
 * The one action for an item: the server's `action` when it sends a known one, else from its kind.
 * A stopped item resumes by Ask again (an approval that expired), Continue (a paused chain), the
 * browser step it waits on, or Retry (resend the person's message).
 */
export function askAction(item: Pick<AwayItem, 'kind' | 'retry' | 'human_step_id'> & { action?: unknown; task_id?: string | null }): AskAction | null {
  // A failed hand-off or run retries only through its board task (#700, Ada): no task_id, no Retry,
  // whatever the server says; the person asks the bot again in the thread.
  if (isHandOff(item) && !item.task_id && (item.action === 'retry' || item.action === undefined)) return null;
  if (typeof item.action === 'string' && ACTIONS.has(item.action)) return item.action as AskAction;
  // A 22i server says "nothing resolves it" with an explicit null: only Done, clear it.
  if (item.action === null) return null;
  switch (item.kind) {
    case 'question':
      return 'answer';
    case 'approval':
      return 'decide';
    case 'stopped': {
      const r = item.retry;
      if (r && r.type === 'approval' && r.approval_id) return 'ask_again';
      if (r && r.type === 'continue') return 'continue';
      if (item.human_step_id) return 'browser';
      return 'retry';
    }
  }
  return null;
}

/** A hand-off or routine run (a failed one is pinned as a rank-2 row and shown on its thread's strip). */
export const isHandOff = (item: Pick<AwayItem, 'kind'>): boolean => item.kind === 'delegation' || item.kind === 'routine_run';

/** The action's button words (design.md §2, final copy). */
export const ACTION_LABEL: Record<AskAction, string> = {
  retry: 'Retry',
  continue: 'Continue',
  ask_again: 'Ask again',
  answer: 'Go to the question',
  decide: 'Go to the approval',
  browser: 'Open the browser step',
};

/** The pressed button's words while its request runs (only the actions that resolve the ask). */
export const BUSY_LABEL: Partial<Record<AskAction, string>> = { retry: 'Retrying…', continue: 'Continuing…', ask_again: 'Asking…' };

/** Actions that resolve the ask (the strip waits on the server); the rest only take the person to it. */
export const RESOLVES: ReadonlySet<AskAction> = new Set(['retry', 'continue', 'ask_again']);

const failed = (item: Pick<AwayItem, 'status'>) => item.status === 'failed' || item.status === 'interrupted' || item.status === 'error';

/** The strip's kind word, the same words as the sidebar tooltip (design.md §2, §4). */
export function askKind(item: Pick<AwayItem, 'kind' | 'status'>, action: AskAction | null): string {
  switch (action) {
    case 'retry':
      return failed(item) ? 'Failed' : 'Stopped';
    case 'continue':
      return 'Paused';
    case 'ask_again':
      return 'Approval ran out';
    case 'answer':
      return 'Question';
    case 'decide':
      return 'Approval';
    case 'browser':
      return 'Browser step';
  }
  return failed(item) ? 'Failed' : 'Stuck';
}

/** The strip's hint under the line: what the button does, so pressing it is never a guess. */
export function askHint(item: Pick<AwayItem, 'kind' | 'status'>, action: AskAction | null, bot: string, clearable = true): string {
  // No button and nothing to clear it with: it clears on its own once handled (Mira's fix 4).
  if (!action && !clearable) return `Write to ${bot} in the chat. This clears once it’s handled.`;
  // design.md §2 "no action", for a hand-off: the bot is asked again where the strip shows (its thread, else its chat).
  // A failed routine run never lives in a thread, and asking the bot again doesn't re-run it (Mira, round 2).
  if (!action && item.kind === 'routine_run' && failed(item)) return 'There’s no button for this one. Press Run now on it in Routines, or clear it once it’s handled.';
  if (!action && isHandOff(item) && failed(item)) return `There’s no button for this one. Ask ${bot} again here, or clear it once it’s handled.`;
  switch (action) {
    case 'retry':
      return failed(item) ? 'Retry runs it again from the start.' : `Retry sends your last message to ${bot} again.`;
    case 'continue':
      return `Continue picks up where ${bot} stopped. Nothing is sent again.`;
    case 'ask_again':
      return 'Ask again starts a new turn. Nothing runs until you say yes.';
    case 'answer':
      return 'Answer it on the card in the chat.';
    case 'decide':
      return 'Allow or Deny it on the card in the chat.';
    case 'browser':
      return `Do it in ${bot}’s browser, then press Done there.`;
  }
  return `There’s no button for this one. Write to ${bot}, or clear it once it’s handled.`;
}

/** The action button's accessible name: its words plus whose ask it is (design.md §5). */
export function actionName(action: AskAction, bot: string): string {
  switch (action) {
    case 'retry':
      return `Retry ${bot}’s reply`;
    case 'continue':
      return `Continue ${bot}’s reply`;
    case 'ask_again':
      return `Ask ${bot} again`;
    case 'answer':
      return `Go to ${bot}’s question`;
    case 'decide':
      return `Go to ${bot}’s approval`;
    case 'browser':
      return `Open ${bot}’s browser step`;
  }
}

/** A failed press, in the server's own words when it gave some (design.md §2, error lines). */
export function failLine(what: AskAction | 'dismiss', reason: string | null, bot: string): string {
  const why = (reason ?? '').trim().replace(/[.\s]+$/, '');
  if (what === 'dismiss') return why ? `Couldn’t clear it: ${why}.` : 'Couldn’t clear it.';
  const verb = what === 'continue' ? 'continue' : what === 'ask_again' ? 'ask again' : 'retry';
  return why ? `Couldn’t ${verb}: ${why}. Try again, or write to ${bot}.` : `Couldn’t ${verb}. Try again, or write to ${bot}.`;
}

/** The ask's full line: the item's own title, whole; its summary when the title is empty or was cut for a row. */
export function askLine(item: Pick<AwayItem, 'kind' | 'title' | 'summary' | 'question'>): string {
  const t = (s?: string | null) => (s ?? '').trim();
  const title = t(item.title);
  const summary = t(item.summary);
  if (item.kind === 'approval' && t(item.question)) return t(item.question);
  // The kind word sits right above the line, so the line doesn't say it again (Mira's fix 1).
  const bare = (x: string) => {
    const rest = x.replace(/^(Failed|Stuck|Stopped|Paused)\s*:\s*/i, '');
    return rest === x || !rest ? x : rest.charAt(0).toUpperCase() + rest.slice(1);
  };
  if (!title) return bare(summary);
  // A title cut for one row ("The model sent nothing for 120 s, so…") gives way to the whole sentence.
  const cut = /(…|\.\.\.)$/.test(title);
  if (cut && summary && summary.length > title.length) return bare(summary);
  return bare(title);
}

/** The place a chat is: what the pane shows now, or where an item lives. */
export type Here = { thread: string; kwant: string } | { channel: string } | { direct: string };

/** Lookups for the message an item's card sits in, when the item doesn't name it. */
export interface OpenLookups {
  /** An open question's card message (`questions[id].message_id`). */
  questionMessage?: (questionId: string) => string | null | undefined;
  /** An open browser step's card message (`humanSteps[id].message_id`). */
  stepMessage?: (stepId: string) => string | null | undefined;
}

/** Where a row opens and what it rings there. `focus` is a message id, or an approval's card id. */
export interface OpenTarget {
  channel_id: string | null;
  kwant_id: string;
  thread_id: string | null;
  focus: string | null;
  focusKind: 'approval' | 'question' | 'browser_step' | 'message' | null;
}

/**
 * Every Needs-attention row and its Inbox card: the exact conversation (thread, channel or direct)
 * and the item's own message to scroll to and ring. Before 22i only approvals named their card.
 */
export function openTarget(item: AwayItem, look: OpenLookups = {}): OpenTarget {
  const t = awayTarget(item);
  const place = {
    channel_id: 'channel' in t ? t.channel : null,
    kwant_id: 'thread' in t ? t.kwant : 'direct' in t ? t.direct : item.kwant_id,
    thread_id: 'thread' in t ? t.thread : null,
  };
  const msg = item.message_id || null;
  if (item.kind === 'approval') return { ...place, focus: item.id, focusKind: 'approval' };
  if (item.kind === 'question') {
    const m = msg ?? look.questionMessage?.(item.id) ?? null;
    return { ...place, focus: m, focusKind: m ? 'question' : null };
  }
  if (item.kind === 'stopped') {
    const m = msg ?? (item.human_step_id ? look.stepMessage?.(item.human_step_id) : null) ?? null;
    return { ...place, focus: m, focusKind: m ? (item.human_step_id ? 'browser_step' : 'message') : null };
  }
  return { ...place, focus: msg, focusKind: msg ? 'message' : null };
}

/**
 * 22x: where to load the messages around an ask that is older than the chat's latest page. A thread
 * has its own route (it answers `around` like the chat route since 22x); a channel or direct chat
 * uses the chat route. `ctx` is the chat the window belongs to (the open thread's id, or the
 * channel's or bot's), so it shows only while that chat is on screen.
 */
export function askWindowUrl(t: Pick<OpenTarget, 'channel_id' | 'kwant_id' | 'thread_id'>, focus: string): { ctx: string; url: string } {
  const around = `around=${encodeURIComponent(focus)}`;
  if (t.thread_id) return { ctx: t.thread_id, url: `/api/threads/${encodeURIComponent(t.thread_id)}/messages?${around}` };
  const ctx = t.channel_id ?? t.kwant_id;
  return { ctx, url: `/api/channels/${encodeURIComponent(ctx)}/messages?${around}` };
}

/**
 * Loads the window around an ask's message. `null` when it can't be shown: the load failed, the
 * server said it isn't in that chat, or (an older server's thread route ignores `around`) the
 * window it sent doesn't hold the message.
 */
export async function loadAskWindow<M extends { id: string }>(
  t: Pick<OpenTarget, 'channel_id' | 'kwant_id' | 'thread_id'>,
  focus: string,
  get: (url: string) => Promise<Pick<Response, 'ok' | 'json'>> = (url) => fetch(url),
): Promise<{ ctx: string; messages: M[] } | null> {
  const { ctx, url } = askWindowUrl(t, focus);
  try {
    const res = await get(url);
    if (!res.ok) return null;
    const messages: unknown = await res.json();
    if (!Array.isArray(messages) || !messages.some((m: M) => m?.id === focus)) return null;
    return { ctx, messages: messages as M[] };
  } catch {
    return null;
  }
}

/** Whether an item lives in the chat on screen. */
export function isHere(item: Pick<AwayItem, 'kwant_id' | 'link'>, here: Here | null): boolean {
  if (!here) return false;
  const t = awayTarget(item);
  if ('thread' in here) return 'thread' in t && t.thread === here.thread;
  if ('channel' in here) return 'channel' in t && t.channel === here.channel;
  return 'direct' in t && t.direct === here.direct;
}

/** The open asks in the chat on screen, oldest first (the order the strip steps through), minus any just cleared. */
export function asksHere(waiting: AwayItem[], here: Here | null, cleared: ReadonlySet<string> = new Set()): AwayItem[] {
  const ts = (i: AwayItem) => Date.parse(i.timestamp) || 0;
  return waiting.filter((i) => onStrip(i) && !cleared.has(i.id) && isHere(i, here)).sort((a, b) => ts(a) - ts(b) || a.id.localeCompare(b.id));
}

/** The strip's eyebrow and its name: "Quant needs you" for one, "2 things need you" for more. */
export const stripHeading = (asks: Pick<AwayItem, 'kwant_name'>[]): string =>
  asks.length > 1 ? `${asks.length} things need you` : `${asks[0]?.kwant_name || 'A bot'} needs you`;

/** Which ask the strip shows after the list changed: the same id if it is still there, else the same place. */
export function stripIndex(asks: Pick<AwayItem, 'id'>[], shownId: string | null, prevIndex: number): number {
  if (!asks.length) return 0;
  const at = shownId ? asks.findIndex((a) => a.id === shownId) : -1;
  if (at >= 0) return at;
  return Math.min(Math.max(0, prevIndex), asks.length - 1);
}

/**
 * Whether the strip offers "Done, clear it": the server's `dismissable` (Lin, #700) when it sends one;
 * older servers leave it out and every ask keeps the button (its 404 says the version can't clear it).
 */
export const canClear = (item: Pick<AwayItem, 'dismissable'>): boolean => (typeof item.dismissable === 'boolean' ? item.dismissable : true);

/**
 * A dismiss refused with 409 because the ask has its own answer, which tells the bot (#700): the button
 * the strip switches to instead. Anything else is an error.
 */
export function switchFor(code: string | null | undefined): AskAction | null {
  switch (code) {
    case 'use_question_answer':
      return 'answer';
    case 'use_approval_decide':
    case 'use_watch_decide':
      return 'decide';
    case 'use_step_decline':
      return 'browser';
  }
  return null;
}

/** The kind the dismiss body names (#700 takes the item's kind from /api/away; a browser step is "browser"). */
export const dismissKind = (item: Pick<AwayItem, 'kind' | 'human_step_id'>): string => (item.kind === 'stopped' && item.human_step_id ? 'browser' : item.kind);

/**
 * Where Retry goes. There is no `POST /api/waiting/:id/retry` (#700 won't add one):
 * - an approval that ran out asks again (`POST /api/approvals/:id/retry`);
 * - a failed task or routine run that names its task retries it (`POST /api/tasks/:id/retry`);
 * - a stopped turn or a reply sends the person's last message again, as the server's notice says, which
 *   closes the item with reason `person_wrote`;
 * - a failed hand-off or run with no task to retry: null (the strip says to write to the bot).
 */
export type RetryRoute = { via: 'approval'; id: string } | { via: 'task'; id: string } | { via: 'resend' };
export function retryRoute(item: Pick<AwayItem, 'kind' | 'retry' | 'task_id'>): RetryRoute | null {
  if (item.retry && item.retry.type === 'approval' && item.retry.approval_id) return { via: 'approval', id: item.retry.approval_id };
  if (item.kind === 'delegation' || item.kind === 'routine_run') return item.task_id ? { via: 'task', id: item.task_id } : null;
  return { via: 'resend' };
}

/**
 * SSE `waiting_updated` (#700): an item that closed leaves Needs attention and the strip at once,
 * without waiting for the refetch. `closedId` is the closed item's id, or null when the event isn't a
 * close (an open item: the refetch picks it up); `dropClosed` returns the same list when it has no such item.
 */
export function closedId(data: unknown): string | null {
  const item = (data as { item?: { id?: unknown; status?: unknown } } | null)?.item;
  return item && item.status === 'closed' && typeof item.id === 'string' && item.id ? item.id : null;
}
export function dropClosed<T extends { id: string }>(list: T[], id: string | null): T[] {
  if (!id || !list.some((i) => i.id === id)) return list;
  return list.filter((i) => i.id !== id);
}

/**
 * What the strip's Retry sends again for a stopped turn or a reply: the notice's
 * own Retry target when the notice is loaded (none when a step may already have gone out), else the
 * person's newest message in this chat, which is what Retry in the chat resends.
 */
export function resendTarget(messages: ChatMessage[], noticeId: string | null | undefined): RetryTarget | null {
  if (noticeId && messages.some((m) => m.id === noticeId)) return retryTargets(messages).get(noticeId) ?? null;
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.sender_role !== 'user') continue;
    const attachments = parseAttachments(m.meta);
    if (m.text.trim() || attachments.length) return { text: m.text, attachments };
  }
  return null;
}
