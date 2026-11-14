// Dictation: the mic in the message box (Mira, autonomy/dictation/design.md; Juno's #610 routes).
// The server turns a recording into text with the space's OpenAI key; nothing goes to a bot before Send.

/** `GET /api/spaces/:id/transcribe`. */
export interface Availability {
  available: boolean;
  source: string | null;
  why: string | null;
}

/** The answer to a recording: the words, or the server's refusal as it sent it. */
export type TranscribeResult =
  | { ok: true; text: string }
  | { ok: false; status: number; code: string | null; error: string; why?: string | null };

/** A network failure (status 0): nothing came back from kwant. */
export const NETWORK_ERROR = 'Couldn’t reach the server to turn this into text.';
export const NO_WORDS = 'Didn’t catch any words. Try again a little closer to the microphone.';
export const INSECURE_WHY = 'Your browser only lets pages on https, or on this computer, use the microphone. Open kwant bots that way to dictate.';
/** The longest recording, in seconds (about 2.4 MB of opus, far under the server's 25 MB). */
export const MAX_SECONDS = 600;
/** From here the timer says how much is left, in amber. */
export const WARN_SECONDS = 570;
/** No sound for this long: "Can't hear anything." */
export const SILENCE_MS = 4000;
export const SILENCE_LEVEL = 0.02;
/** The "Added n words" line goes away after this. */
export const ADDED_MS = 8000;
/** The tab hidden this long while dictating stops and discards it. */
export const HIDDEN_MS = 30000;

// ---------------------------------------------------------------------------------------------------
// Availability, cached per space

type Cached = Availability | 'error';
const cache = new Map<string, Cached>();
const inflight = new Map<string, Promise<Cached>>();
const listeners = new Set<() => void>();

const hiddenKey = (spaceId: string) => `kwnt:dictation-hidden:${spaceId}`;

export function micHidden(spaceId: string): boolean {
  try {
    return localStorage.getItem(hiddenKey(spaceId)) === '1';
  } catch {
    return false;
  }
}

export function setMicHidden(spaceId: string, hidden: boolean): void {
  try {
    if (hidden) localStorage.setItem(hiddenKey(spaceId), '1');
    else localStorage.removeItem(hiddenKey(spaceId));
  } catch {
    /* private mode: the mic just stays */
  }
  emit();
}

function emit() {
  for (const l of [...listeners]) l();
}

/** Called when the cache or the hidden flag changes. */
export function onDictationChange(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

/** What's known for a space now: an answer, 'error' (fetch failed or 404), or undefined (not asked yet). */
export function cachedAvailability(spaceId: string): Cached | undefined {
  return cache.get(spaceId);
}

/** Sets a space's availability (a 409 `no_transcriber` from the POST flips it to not set up). */
export function setAvailability(spaceId: string, a: Availability): void {
  cache.set(spaceId, a);
  if (a.available && globalThis.isSecureContext !== false && micHidden(spaceId)) setMicHidden(spaceId, false);
  emit();
}

export function parseAvailability(raw: unknown): Availability | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  if (typeof r.available !== 'boolean') return null;
  return {
    available: r.available,
    source: typeof r.source === 'string' ? r.source : null,
    why: typeof r.why === 'string' && r.why.trim() ? r.why : null,
  };
}

/** Reads `GET /api/spaces/:id/transcribe`, once per space unless `fresh`. */
export function getAvailability(spaceId: string, opts: { fresh?: boolean } = {}): Promise<Cached> {
  const have = cache.get(spaceId);
  if (have !== undefined && !opts.fresh) return Promise.resolve(have);
  const going = inflight.get(spaceId);
  if (going) return going;
  const p = fetch(`/api/spaces/${encodeURIComponent(spaceId)}/transcribe`, { credentials: 'same-origin' })
    .then(async (res) => (res.ok ? parseAvailability(await res.json().catch(() => null)) ?? 'error' : 'error'))
    .catch((): Cached => 'error')
    .then((a) => {
      inflight.delete(spaceId);
      cache.set(spaceId, a);
      // A key was added: "Hide the mic" was about not having one, so the mic comes back.
      if (a !== 'error' && a.available && micHidden(spaceId) && globalThis.isSecureContext !== false) setMicHidden(spaceId, false);
      emit();
      return a;
    });
  inflight.set(spaceId, p);
  return p;
}

/** Forget every answer and ask again (Space settings closed, or the window got focus back). */
export function refreshDictation(): void {
  const spaces = [...cache.keys()];
  cache.clear();
  for (const s of spaces) void getAvailability(s);
  emit();
}

/** `POST /api/spaces/:id/transcribe?lang=xx` with the recording as the body. Throws only on abort. */
export async function transcribe(spaceId: string, blob: Blob, lang: string | null, signal?: AbortSignal): Promise<TranscribeResult> {
  const q = lang ? `?lang=${encodeURIComponent(lang)}` : '';
  let res: Response;
  try {
    res = await fetch(`/api/spaces/${encodeURIComponent(spaceId)}/transcribe${q}`, {
      method: 'POST',
      body: blob,
      headers: { 'Content-Type': blob.type || 'audio/webm' },
      credentials: 'same-origin',
      signal,
    });
  } catch (err) {
    if (signal?.aborted || (err as { name?: string })?.name === 'AbortError') throw err;
    return { ok: false, status: 0, code: 'network', error: NETWORK_ERROR };
  }
  let body: Record<string, unknown> | null = null;
  try {
    body = (await res.json()) as Record<string, unknown>;
  } catch {
    if (signal?.aborted) throw new DOMException('Aborted', 'AbortError');
  }
  if (res.ok && body && typeof body.text === 'string') return { ok: true, text: body.text };
  const error = body && typeof body.error === 'string' && body.error.trim() ? body.error : res.ok ? 'The answer couldn’t be read.' : `Dictation failed (${res.status}).`;
  return {
    ok: false,
    status: res.ok ? 502 : res.status,
    code: body && typeof body.code === 'string' ? body.code : null,
    error,
    why: body && typeof body.why === 'string' ? body.why : null,
  };
}

// ---------------------------------------------------------------------------------------------------
// Pure helpers

export type MicView = 'ready' | 'muted' | 'hidden';

/**
 * Whether the mic shows, and how (design §2). `supported`: the browser has MediaRecorder (and
 * getUserMedia where it's allowed); `secure`: window.isSecureContext; `availability`: the GET's answer,
 * 'error' when it failed or 404'd, undefined while it's asked; `hidden`: "Hide the mic" in this space.
 */
export function micVisible(s: { supported: boolean; secure: boolean; availability: Cached | undefined; hidden: boolean }): MicView {
  if (!s.supported) return 'hidden';
  if (s.availability === undefined || s.availability === 'error') return 'hidden';
  if (s.secure && s.availability.available) return 'ready';
  if (s.hidden) return 'hidden';
  return 'muted';
}

/** Whether this browser can record at all. Over plain http getUserMedia is missing; that's `secure`'s job. */
export function browserSupports(w: { MediaRecorder?: unknown; isSecureContext?: boolean; navigator?: { mediaDevices?: { getUserMedia?: unknown } } }): boolean {
  if (typeof w.MediaRecorder !== 'function') return false;
  if (w.isSecureContext === false) return true;
  return typeof w.navigator?.mediaDevices?.getUserMedia === 'function';
}

export interface Inserted {
  value: string;
  /** Where the inserted piece (with any added spaces) starts and ends in `value`. */
  from: number;
  to: number;
  piece: string;
  /** The caret: right after the words. */
  caret: number;
}

/**
 * Puts dictated words into the box at `at` (clamped to the text): a space before unless it's the start
 * or there's whitespace there, a space after unless it's the end or the next character is whitespace or
 * punctuation. Nothing typed is replaced.
 */
export function insertDictated(value: string, at: number, text: string): Inserted {
  const words = text.trim();
  const pos = Math.max(0, Math.min(Number.isFinite(at) ? at : value.length, value.length));
  const before = value.slice(0, pos);
  const after = value.slice(pos);
  const lead = pos > 0 && !/\s$/.test(before) ? ' ' : '';
  const trail = after.length > 0 && !/^[\s.,;:!?…)\]}"'’”»]/.test(after) ? ' ' : '';
  const piece = `${lead}${words}${trail}`;
  return { value: before + piece + after, from: pos, to: pos + piece.length, piece, caret: pos + lead.length + words.length };
}

/** Takes the words back out, only if they are still there unchanged; null otherwise. */
export function undoDictated(value: string, ins: Pick<Inserted, 'from' | 'to' | 'piece'>): string | null {
  if (value.slice(ins.from, ins.to) !== ins.piece) return null;
  return value.slice(0, ins.from) + value.slice(ins.to);
}

export function wordCount(text: string): number {
  const t = text.trim();
  return t ? t.split(/\s+/).length : 0;
}

export function wordsLabel(n: number): string {
  return n === 1 ? '1 word' : `${n.toLocaleString('en-US')} words`;
}

/** The browser's language as a two-letter hint, or null. */
export function langHint(navigatorLanguage: string | null | undefined): string | null {
  const l = (navigatorLanguage ?? '').slice(0, 2).toLowerCase();
  return /^[a-z]{2}$/.test(l) ? l : null;
}

/** What to say when the browser refuses the microphone (design §7), by the error's `name`. */
export function micErrorText(name: string | null | undefined, phone: boolean): string {
  switch (name) {
    case 'NotAllowedError':
    case 'SecurityError':
      return phone
        ? 'The microphone is blocked for this site. Allow it in your browser’s site settings, then press the mic again.'
        : 'The microphone is blocked for this site. Allow it from the icon at the left of the address bar, then press the mic again.';
    case 'NotFoundError':
    case 'OverconstrainedError':
      return phone
        ? 'No microphone found. Check your phone’s microphone settings for this browser, then press the mic again.'
        : 'No microphone found. Plug one in or pick one in your system’s sound settings, then press the mic again.';
    case 'NotReadableError':
    case 'AbortError':
      return 'The microphone is in use by another app. Close it there, then press the mic again.';
    default:
      return 'Couldn’t start the microphone. Try again, or reload the page.';
  }
}

/** Try again (same recording) only where saying it again wouldn't help: busy, OpenAI failed, no network. */
export function canRetry(status: number, code: string | null | undefined): boolean {
  if (status === 0 || code === 'network') return true;
  if (status === 429 && code === 'busy') return true;
  if (status === 502) return true;
  return false;
}

/** m:ss */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** The level bars: the centre is the level, its neighbours 0.7x and 0.35x, never under 0.2. */
export function barScales(level: number): number[] {
  const l = Math.max(0, Math.min(1, level));
  return [0.35, 0.7, 1, 0.7, 0.35].map((k) => Math.max(0.2, Math.min(1, l * k)));
}

/** RMS of a time-domain buffer (-1..1), scaled so normal speech reaches about 1. */
export function rmsLevel(buf: ArrayLike<number>): number {
  if (!buf.length) return 0;
  let sum = 0;
  for (let i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
  return Math.min(1, Math.sqrt(sum / buf.length) * 4);
}
