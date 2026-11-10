// The slim "Bot is <step> · 0:42" strip above the composer. Pure helpers; the component only draws them.
import { ActivityStep, LiveActivity, formatClock, isOpenStep, plainTitle } from './activity';
import { readableStepTitle } from './toolLabel';

export interface WorkingState {
  live?: LiveActivity | null;
  queued?: boolean;
  awaitingAnswer?: boolean;
  awaitingBrowser?: boolean;
  /** The reply's first words have started streaming. */
  writing?: boolean;
}

const lower = (t: string) => (t ? t.charAt(0).toLowerCase() + t.slice(1) : t);

/** What the bot is doing right now, in a few lowercase words that follow "<Bot> is". */
export function workingLabel(s: WorkingState): string {
  if (s.queued) return 'waiting in line';
  if (s.awaitingAnswer) return s.awaitingBrowser ? 'waiting for you in the browser' : 'waiting for your answer';
  const open: ActivityStep | undefined = [...(s.live?.steps ?? [])].reverse().find(isOpenStep);
  // An approval: the card says what it is for; the strip only says whose move it is.
  if (open?.kind === 'approval' && open.status === 'waiting') return 'waiting for your OK';
  if (open) {
    const t = readableStepTitle(plainTitle(open.title).trim());
    if (t) return lower(t);
  }
  if (s.writing) return 'writing the reply';
  // Steps already ran (a tool finished, the next one isn't open yet): still working, not starting.
  if (s.live?.steps.length) return 'working on it';
  return 'getting started';
}

/**
 * What the bot waits on the person for, so the phone strip can offer "Answer ↑" (spec A3): a question,
 * an approval card, or null (working, in line, or a browser step, which has its own card and button).
 */
export function waitingOn(s: WorkingState): 'question' | 'approval' | null {
  if (s.queued) return null;
  if (s.awaitingAnswer) return s.awaitingBrowser ? null : 'question';
  const open = [...(s.live?.steps ?? [])].reverse().find(isOpenStep);
  return open?.kind === 'approval' && open.status === 'waiting' ? 'approval' : null;
}

/**
 * What the live reply bubble holds before its first words. A turn paused on an approval card (the
 * production floor, a shell command) is waiting for the person, not loading, so it gets a line that
 * points at the card instead of the "first words" skeleton (0.7.1, D6 footage).
 */
export type LiveBubble = 'queued' | 'text' | 'question' | 'browser' | 'approval' | 'starting';

export function liveBubble(s: WorkingState): LiveBubble {
  if (s.queued) return 'queued';
  if (s.writing) return 'text';
  if (s.awaitingAnswer) return s.awaitingBrowser ? 'browser' : 'question';
  return waitingOn(s) === 'approval' ? 'approval' : 'starting';
}

/** The strip's clock; the saved card header uses the same one. */
export const formatElapsed = formatClock;

/** When the turn began: the earliest step start, else the moment the strip appeared. */
export function turnStart(live: LiveActivity | null | undefined, mountedAt: number): number {
  let start = mountedAt;
  for (const s of live?.steps ?? []) {
    const t = Date.parse(s.started_at);
    if (Number.isFinite(t) && t < start) start = t;
  }
  return start;
}

/**
 * 0.8.1 F4: live chats whose running bot has no name yet but can be named now: [chat, name] pairs.
 * `live` maps a chat to the name its strip shows ('' when the bot list hadn't loaded), `kwantIds` maps
 * a chat to its running bot's id. A chat that already shows a name keeps it.
 */
export function namesToFill(
  live: Record<string, string>,
  kwantIds: Record<string, string>,
  nameOf: (id: string) => string | undefined,
): [string, string][] {
  const out: [string, string][] = [];
  for (const [ctx, name] of Object.entries(live)) {
    if (name || !kwantIds[ctx]) continue;
    const n = nameOf(kwantIds[ctx]);
    if (n) out.push([ctx, n]);
  }
  return out;
}

/**
 * 0.8.1 F4: the order a page opened mid-turn names its live chats in. Each chat keeps the last name it
 * is given, so queued turns go first and a running turn in the same chat names the strip, never a bot
 * that is only waiting in line.
 */
export function liveNamingOrder<T extends { state?: string }>(turns: T[]): T[] {
  return [...turns.filter((t) => t.state !== 'running'), ...turns.filter((t) => t.state === 'running')];
}
