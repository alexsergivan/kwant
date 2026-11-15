// What a bot's avatar shows about it besides "working": thinking, just finished, failed, needs you,
// resting (out of quota). Worked out on the client from events and data it already has: turn start/end
// (with the outcome), the first streamed output, open questions and browser steps, and the engine
// accounts' limited_until. Pure selection plus a small per-kwant store, like kwantActivity.

import { isKwantWorking, subscribeKwant } from './kwantActivity';
import { parsePick } from './engines';
import { activeLimit, type Profile } from './profiles';
import { timeText } from './clock';

export type Mood = 'resting' | 'needs_you' | 'thinking' | 'working' | 'failed' | 'finished' | 'idle';

/** How long "just finished" and "failed" stay on the avatar. */
export const FINISHED_MS = 3000;
export const FAILED_MS = 10 * 60_000;

export type TurnOutcome = 'completed' | 'failed' | 'cancelled';

export interface MoodInputs {
  /** A turn is running (kwantActivity). */
  working: boolean;
  /** A turn started and nothing has streamed yet. */
  thinking?: boolean;
  /** A question or browser step waits for the person. */
  needsYou?: boolean;
  /** The bot's engine accounts are all out of usage until then. */
  restingUntil?: Date | null;
  /** How the last turn ended, and when (ms). */
  last?: { outcome: TurnOutcome; at: number } | null;
  now: number;
}

/**
 * One mood per bot. Something that needs the person beats everything; a running turn beats the
 * resting look (the bot is running, so it isn't out); a failure stays until the next turn; "just
 * finished" fades after a moment. A cancelled turn shows nothing.
 */
export function selectMood(i: MoodInputs): Mood {
  if (i.needsYou) return 'needs_you';
  if (i.working) return i.thinking ? 'thinking' : 'working';
  if (i.restingUntil && i.restingUntil.getTime() > i.now) return 'resting';
  if (i.last?.outcome === 'failed' && i.now - i.last.at < FAILED_MS) return 'failed';
  if (i.last?.outcome === 'completed' && i.now - i.last.at < FINISHED_MS) return 'finished';
  return 'idle';
}

/** "3:00 AM" / "03:00" on the device's clock, with the day when it isn't today. */
export function restingClock(at: Date, now = Date.now()): string {
  const time = timeText(at) ?? '';
  return at.toDateString() === new Date(now).toDateString() ? time : `${at.toLocaleDateString(undefined, { weekday: 'short' })} ${time}`;
}

/** Words for the avatar's tooltip and screen readers; null when the mood adds nothing. */
export function moodLabel(mood: Mood, name: string | undefined, until?: Date | null, now = Date.now()): string | null {
  const who = name || 'This bot';
  switch (mood) {
    case 'resting': return until ? `Resting until ${restingClock(until, now)}` : `${who} is resting`;
    case 'needs_you': return `${who} needs you`;
    case 'thinking': return `${who} is thinking`;
    case 'failed': return `${who}'s last turn failed`;
    case 'finished': return `${who} just finished`;
    default: return null;
  }
}

/**
 * When a bot's engine accounts come back, or null while any of them can run. A bot that picked an
 * engine family rests when every switched-on account of that family is out; a bot on "auto" rests
 * only when every switched-on account is. The earliest return time wins.
 */
export function restingUntil(preferredModel: string | null | undefined, profiles: Profile[] | null | undefined, now = Date.now()): Date | null {
  if (!profiles?.length) return null;
  const { family } = parsePick(preferredModel);
  const scope = profiles.filter((p) => p.enabled && (family && family !== 'auto' ? p.engine === family : true));
  if (!scope.length) return null;
  let earliest: Date | null = null;
  for (const p of scope) {
    const limit = activeLimit(p.limited_until, now);
    if (!limit) return null;
    if (!earliest || limit < earliest) earliest = limit;
  }
  return earliest;
}

// ---- The store ----

interface BotMood {
  thinking: boolean;
  needsYou: boolean;
  until: Date | null;
  last: { outcome: TurnOutcome; at: number } | null;
  /** Bumps each time the person sends this bot a message: the avatar blinks once. */
  notices: number;
}

const state = new Map<string, BotMood>();
const listeners = new Map<string, Set<() => void>>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();

const get = (id: string): BotMood => state.get(id) ?? { thinking: false, needsYou: false, until: null, last: null, notices: 0 };
const notify = (id: string) => listeners.get(id)?.forEach((l) => l());

function patch(id: string, next: Partial<BotMood>): void {
  const before = get(id);
  const after = { ...before, ...next };
  const changed = (Object.keys(next) as Array<keyof BotMood>).some((k) => {
    const a = before[k];
    const b = after[k];
    return a instanceof Date || b instanceof Date ? (a as Date | null)?.getTime() !== (b as Date | null)?.getTime() : JSON.stringify(a) !== JSON.stringify(b);
  });
  if (!changed) return;
  state.set(id, after);
  notify(id);
}

/** Re-render when "just finished" or "failed" runs out. */
function expireLater(id: string, ms: number) {
  clearTimeout(timers.get(id));
  timers.set(id, setTimeout(() => notify(id), ms + 20));
}

export const moodTurnStarted = (id: string) => id && patch(id, { thinking: true, last: null });
/** First streamed text or step of a turn: no longer "thinking". */
export const moodOutput = (id: string) => id && patch(id, { thinking: false });
export function moodTurnEnded(id: string, outcome: string | undefined, now = Date.now()): void {
  if (!id) return;
  const o: TurnOutcome = outcome === 'failed' ? 'failed' : outcome === 'cancelled' ? 'cancelled' : 'completed';
  patch(id, { thinking: false, last: { outcome: o, at: now } });
  if (o !== 'cancelled') expireLater(id, o === 'failed' ? FAILED_MS : FINISHED_MS);
}
/** Which bots wait for the person right now (replaces the previous set). */
export function setMoodNeedsYou(ids: Iterable<string>): void {
  const want = new Set(ids);
  for (const id of new Set([...state.keys(), ...want])) patch(id, { needsYou: want.has(id) });
}
/** Each bot's return time while resting (replaces the previous map). */
export function setMoodResting(untilByBot: Record<string, Date | null>): void {
  for (const id of new Set([...state.keys(), ...Object.keys(untilByBot)])) patch(id, { until: untilByBot[id] ?? null });
}
/** The person just sent this bot a message. */
export const moodNoticed = (id: string) => id && patch(id, { notices: get(id).notices + 1 });
export const noticesOf = (id: string | undefined): number => (id ? get(id).notices : 0);
export const restingUntilOf = (id: string | undefined): Date | null => (id ? get(id).until : null);
export function clearMoods(): void {
  const ids = [...state.keys()];
  state.clear();
  timers.forEach(clearTimeout);
  timers.clear();
  ids.forEach(notify);
}

export function moodOf(id: string | undefined, now = Date.now()): Mood {
  if (!id) return 'idle';
  const s = get(id);
  return selectMood({ working: isKwantWorking(id), thinking: s.thinking, needsYou: s.needsYou, restingUntil: s.until, last: s.last, now });
}

export function subscribeMood(id: string, listener: () => void): () => void {
  const set = listeners.get(id) ?? new Set();
  set.add(listener);
  listeners.set(id, set);
  const offActivity = subscribeKwant(id, listener);
  return () => {
    set.delete(listener);
    if (!set.size) listeners.delete(id);
    offActivity();
  };
}
