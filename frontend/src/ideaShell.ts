// Idea jar, the shell's side (0.7, Pixel): the wobble signal, the /idea send answer, the drop toast's
// steps and the Settings switch. Pure where it can be, so tests reach it without a DOM.
import { useEffect, useState } from 'react';
import { parseIdea, type Idea } from './ideas';

export const JAR_SEND_FAILED = 'Couldn’t drop it in the jar. Try again.';
export const JAR_PERSON_ONLY = 'Only you can add ideas.';
export const JAR_DROPPED = 'Dropped in the jar.';

/** What POST /api/chat/send answered to a `/idea` line. */
export type IdeaSendOutcome =
  | { kind: 'dropped'; idea: Idea }
  /** An older server ran it as a chat message: fall back to the normal path. */
  | { kind: 'chat' }
  | { kind: 'error'; line: string };

/** Reads the send's answer (spec §2.3). `body` is the parsed JSON, or null. */
export function ideaSendOutcome(status: number, body: unknown): IdeaSendOutcome {
  const o = body && typeof body === 'object' ? (body as Record<string, unknown>) : null;
  if (status === 403) return { kind: 'error', line: JAR_PERSON_ONLY };
  if (status === 400) {
    const m = typeof o?.message === 'string' && o.message.trim() ? o.message.trim() : null;
    return { kind: 'error', line: m ? (/[.!?]$/.test(m) ? m : `${m}.`) : JAR_SEND_FAILED };
  }
  if (status < 200 || status >= 300) return { kind: 'error', line: JAR_SEND_FAILED };
  if (o?.handled === 'idea') {
    const idea = parseIdea(o.idea);
    return idea ? { kind: 'dropped', idea } : { kind: 'error', line: JAR_SEND_FAILED };
  }
  return { kind: 'chat' };
}

/** The composer's blocked-send line for a `/idea` draft, or null when it may go (spec §2.3). */
export function ideaBlockedLine(text: string, max: number, hasFiles = false): string | null {
  const body = text.trimStart().replace(/^\/idea/i, '').trim();
  if (!body) return 'Type your idea after /idea';
  if (body.length > max) return `${body.length.toLocaleString('en-US')} / ${max.toLocaleString('en-US')} · too long for the jar`;
  if (hasFiles) return 'Ideas are text only. Remove the file to drop it in.';
  return null;
}

/** The length the counter shows: the idea itself, without `/idea `. */
export const ideaLength = (text: string) => text.trimStart().replace(/^\/idea/i, '').trim().length;

// ---- Settings › "Free bots peek in the idea jar" (spec §6). One number; 0 is off.

export const IDEA_CAP_DEFAULT = 3;
export const IDEA_CAP_MIN = 1;
export const IDEA_CAP_MAX = 10;

/** The cap to save when the switch flips: off writes 0, on brings back the last non-zero value or 3. */
export function ideaCapForSwitch(on: boolean, last: number | null | undefined): number {
  if (!on) return 0;
  const n = typeof last === 'number' && Number.isFinite(last) ? Math.round(last) : 0;
  return n >= IDEA_CAP_MIN ? Math.min(IDEA_CAP_MAX, n) : IDEA_CAP_DEFAULT;
}

/** The settings value, clamped 0–10 (0 allowed here, unlike work-ahead); 3 when missing. */
export function parseIdeaCap(raw: unknown): number {
  const n = typeof raw === 'number' ? raw : typeof raw === 'string' && raw.trim() ? Number(raw) : NaN;
  if (!Number.isFinite(n)) return IDEA_CAP_DEFAULT;
  return Math.max(0, Math.min(IDEA_CAP_MAX, Math.round(n)));
}

// ---- The wobble: one signal, so a drop anywhere (sidebar, /idea, the phone) wobbles the jar icon.

let wobbleSeq = 0;
const wobblers = new Set<(n: number) => void>();
export function wobbleJar() {
  wobbleSeq += 1;
  wobblers.forEach((f) => f(wobbleSeq));
}
/** A key that changes on every drop; put it on the icon's wrapper so the animation restarts. 0 = never. */
export function useJarWobble(): number {
  const [n, setN] = useState(0);
  useEffect(() => {
    wobblers.add(setN);
    return () => void wobblers.delete(setN);
  }, []);
  return n;
}

// The last non-zero cap, so turning the switch back on brings it back even after a reload.
const LAST_CAP_KEY = 'kwant.ideaJarCap.last';
export function lastIdeaCap(): number | null {
  try {
    const v = Number(globalThis.localStorage?.getItem(LAST_CAP_KEY));
    return Number.isInteger(v) && v >= IDEA_CAP_MIN && v <= IDEA_CAP_MAX ? v : null;
  } catch {
    return null;
  }
}
export function rememberIdeaCap(n: number) {
  if (!(Number.isInteger(n) && n >= IDEA_CAP_MIN && n <= IDEA_CAP_MAX)) return;
  try {
    globalThis.localStorage?.setItem(LAST_CAP_KEY, String(n));
  } catch {
    // private mode: the switch falls back to 3
  }
}
