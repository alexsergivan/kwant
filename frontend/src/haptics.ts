// Short phone vibrations for meaningful actions (send, answer, approve, puzzle steps) and for a
// bot reply arriving while the app is on screen. Uses navigator.vibrate, which Chrome on Android
// supports; iOS Safari and desktops have none, so every call there is a silent no-op.

export type HapticKind = 'tap' | 'success' | 'warning' | 'message';

/** Motor on/off times in ms. Kept short: a web app can only set how long the motor runs. */
export const HAPTIC_PATTERNS: Record<HapticKind, number | number[]> = {
  tap: 10,
  success: [15, 40, 15],
  warning: [30, 60, 30],
  // A light double buzz, different from the tap a button gives.
  message: [8, 80, 8],
};

export const HAPTICS_KEY = 'kwnt_haptics';

/** Several replies landing together give one buzz. */
export const MESSAGE_GAP_MS = 1500;

/** What the helpers read; the browser's globals by default, fakes in tests. */
export interface HapticsEnv {
  navigator?: {
    vibrate?: (pattern: number | number[]) => boolean;
    userActivation?: { hasBeenActive?: boolean };
  };
  matchMedia?: (query: string) => { matches: boolean };
  localStorage?: Pick<Storage, 'getItem' | 'setItem'>;
  document?: { visibilityState?: string };
}

const browserEnv = (): HapticsEnv => globalThis as unknown as HapticsEnv;

export function canVibrate(env: HapticsEnv = browserEnv()): boolean {
  return typeof env.navigator?.vibrate === 'function';
}

/** Whether to offer the Settings switch: desktop Chrome has vibrate too, but no motor. */
export function hapticsAvailable(env: HapticsEnv = browserEnv()): boolean {
  if (!canVibrate(env)) return false;
  try {
    return !!env.matchMedia?.('(pointer: coarse)').matches;
  } catch {
    return false;
  }
}

function prefersReducedMotion(env: HapticsEnv): boolean {
  try {
    return !!env.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}

/** The Settings switch. On by default, off by default when the phone asks for reduced motion. */
export function hapticsEnabled(env: HapticsEnv = browserEnv()): boolean {
  let stored: string | null = null;
  try {
    stored = env.localStorage?.getItem(HAPTICS_KEY) ?? null;
  } catch {
    // storage blocked: fall back to the default
  }
  if (stored === 'on') return true;
  if (stored === 'off') return false;
  return !prefersReducedMotion(env);
}

export function setHapticsEnabled(on: boolean, env: HapticsEnv = browserEnv()): void {
  try {
    env.localStorage?.setItem(HAPTICS_KEY, on ? 'on' : 'off');
  } catch {
    // storage blocked: the choice lasts until reload
  }
}

/** Vibrates once if the device can and the person allows it. Never throws; true if it buzzed. */
export function haptic(kind: HapticKind, env: HapticsEnv = browserEnv()): boolean {
  const nav = env.navigator;
  if (!nav || typeof nav.vibrate !== 'function') return false;
  if (!hapticsEnabled(env)) return false;
  // Chrome blocks (and logs) vibration before the first tap on the page.
  if (nav.userActivation && nav.userActivation.hasBeenActive === false) return false;
  try {
    return nav.vibrate.call(nav, HAPTIC_PATTERNS[kind]) !== false;
  } catch {
    return false;
  }
}

let lastMessageAt = -Infinity;

/** A bot's reply arrived: buzz only while the app is on screen, at most once per MESSAGE_GAP_MS. */
export function messageHaptic(env: HapticsEnv = browserEnv(), now: number = Date.now()): boolean {
  // In the background the push notification already vibrates.
  if (env.document && env.document.visibilityState !== 'visible') return false;
  if (now - lastMessageAt < MESSAGE_GAP_MS) return false;
  if (!haptic('message', env)) return false;
  lastMessageAt = now;
  return true;
}

/** Tests only. */
export function resetMessageHaptic(): void {
  lastMessageAt = -Infinity;
}
