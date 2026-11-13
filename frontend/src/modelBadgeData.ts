// What the avatar's model badge needs besides the live turn: each bot's pick and last reply's engine
// (from the app's bot list), and which engines can run (GET /api/engines). The app fills it, like botLooks.
import type { EngineInfo, RunsOn } from './engines';

export interface BadgeBot {
  id: string;
  preferred_model?: string | null;
  /** From GET /api/kwants; missing on servers before 0.9.6, null for a bot that never replied. */
  last_ran_on?: RunsOn | null;
}

let bots = new Map<string, BadgeBot>();
let engines: EngineInfo[] | null = null;
let rev = 0;
const listeners = new Set<() => void>();
const bump = () => {
  rev += 1;
  listeners.forEach((fn) => fn());
};

/** Replaces the known bots (the app calls it whenever its bot list changes). */
export function setBadgeBots(list: readonly BadgeBot[]): void {
  bots = new Map(list.map((b) => [b.id, b]));
  bump();
}

export function setBadgeEngines(list: EngineInfo[] | null): void {
  if (list === engines) return;
  engines = list;
  bump();
}

export const badgeBotOf = (id: string | undefined): BadgeBot | undefined => (id ? bots.get(id) : undefined);
export const badgeEngines = (): EngineInfo[] | null => engines;
export const badgeDataRev = (): number => rev;
export function subscribeBadgeData(fn: () => void): () => void {
  listeners.add(fn);
  return () => void listeners.delete(fn);
}
