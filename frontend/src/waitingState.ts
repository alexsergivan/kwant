// Waiting state (UX audit #2, 0.9.7): while the only open thing in a chat is a bot's question for the
// person, the chat reads as "waiting for you", not as working. Pure helpers; ChatView draws them.
import type { WorkingState } from './workingStrip';

export interface AskOnlyState extends WorkingState {
  /** A reply is running in this chat (its turn hasn't ended). */
  streaming?: boolean;
  /** The running turn is paused on an approval card (shell command, production floor). */
  approvalOpen?: boolean;
}

/**
 * Whether the only open thing here is a question the person has to answer: a turn is running, it is
 * paused on a question (not a browser step or an approval), nothing is queued and no words are streaming.
 * Then the working card, the "Paused on the question above" box and the red Stop hide, Send stays, and
 * Stop moves to the banner's ghost button.
 */
export function onlyAskOpen(s: AskOnlyState): boolean {
  return !!s.streaming && !!s.awaitingAnswer && !s.awaitingBrowser && !s.approvalOpen && !s.queued && !s.writing;
}

/** One line for each thing the chat waits on the person for. Later lines (waiting checks) add a kind. */
export interface WaitingLine {
  kind: 'answer';
  bot: string;
}

/** The banner's words for one waiting line. */
export function waitingBannerText(line: WaitingLine): string {
  const bot = line.bot.trim() || 'A bot';
  return `Waiting for you: ${bot} needs one answer to finish`;
}

/** The banner line on a phone (below sm): the same words without "to finish". */
export function waitingBannerShortText(line: WaitingLine): string {
  const bot = line.bot.trim() || 'A bot';
  return `Waiting for you: ${bot} needs one answer`;
}

/** A team turn (a team chat, or a discussion running in a channel) gets "Stop team" and the rail; a
 *  single bot asking in a channel or its own chat does not. */
export function isTeamTurn(homeKind: string | undefined, discussionInProgress: boolean): boolean {
  return homeKind === 'team' || discussionInProgress;
}

/** The composer's placeholder while the chat waits on a bot's answer: a message sent here answers it. */
export function answerPlaceholder(bot: string | undefined): string {
  return `Type your answer to ${bot?.trim() || 'the bot'}…`;
}

/** The banner's Stop button: a team turn stops the team, a single bot's turn stops that reply. */
export function waitingStopLabel(team: boolean): string {
  return team ? 'Stop team' : 'Stop';
}

/** "asked just now", "asked 1 min ago", "asked 12 min ago", "asked 2 h ago". Null without a valid time. */
export function askedAgo(createdAt: string | null | undefined, now: number): string | null {
  if (!createdAt) return null;
  // The server writes UTC without a zone in some places ("2026-11-13 07:01:02").
  const iso = /[zZ]|[+-]\d\d:?\d\d$/.test(createdAt) ? createdAt : `${createdAt.replace(' ', 'T')}Z`;
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return null;
  const mins = Math.floor(Math.max(0, now - t) / 60000);
  if (mins < 1) return 'asked just now';
  if (mins < 60) return `asked ${mins} min ago`;
  return `asked ${Math.floor(mins / 60)} h ago`;
}

export interface RailStep {
  name: string;
  state: 'done' | 'you' | 'next';
}

/**
 * The team turn rail under the banner, from messages the chat already has: each teammate who replied
 * since the person's last message (done), then "You pick" (you), then the asker wrapping up (next).
 * Empty when no teammate replied yet (a single bot's ask has no rail).
 */
export function turnRail(
  messages: { sender_role: string; sender_id: string }[],
  askerId: string | null | undefined,
  botOf: (senderId: string) => { id: string; name: string } | null | undefined,
): RailStep[] {
  const done = repliedSinceYou(messages, askerId, botOf);
  if (!done.length) return [];
  const asker = (askerId && botOf(askerId)?.name) || 'The lead';
  return [...done.map((b) => ({ name: b.name, state: 'done' as const })), { name: 'You pick', state: 'you' }, { name: `${asker} wraps up`, state: 'next' }];
}

/** The bots other than the asker that replied since the person's last message, in reply order. */
export function repliedSinceYou(
  messages: { sender_role: string; sender_id: string }[],
  askerId: string | null | undefined,
  botOf: (senderId: string) => { id: string; name: string } | null | undefined,
): { id: string; name: string }[] {
  let start = 0;
  for (let i = messages.length - 1; i >= 0; i--)
    if (messages[i].sender_role === 'user') {
      start = i + 1;
      break;
    }
  const done: { id: string; name: string }[] = [];
  for (const m of messages.slice(start)) {
    if (m.sender_role !== 'kwant') continue;
    const bot = botOf(m.sender_id);
    if (bot && bot.id !== askerId && !done.some((d) => d.id === bot.id)) done.push(bot);
  }
  return done;
}

/** The sidebar's line for a teammate that finished its part of a team turn now waiting on the person. */
export const DONE_HERE_LINE = 'Done · replied here';

/** Sidebar "Done" lines, by bot id, for a team chat waiting on one bot's question; empty otherwise. */
export function doneLines(
  messages: { sender_role: string; sender_id: string }[],
  asker: { id: string } | null | undefined,
  team: boolean,
  botOf: (senderId: string) => { id: string; name: string } | null | undefined,
): Record<string, string> {
  if (!asker || !team) return {};
  return Object.fromEntries(repliedSinceYou(messages, asker.id, botOf).map((b) => [b.id, DONE_HERE_LINE]));
}
