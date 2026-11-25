// "We'll buzz your phone", honestly (queue-design.md section 3). Nothing leaves the box: no push, no SMS, no email. The
// page reacts only while it is open, with what this browser really has: vibrate, a chime made in code (WebAudio, no
// audio file) and the screen wake lock. Each is feature-detected, and a missing one is hidden. The iPhone check only
// picks the honest line and the "can't vibrate" row; it never switches a feature off.

type Vibrate = (pattern: number[]) => boolean;
type Sentinel = { release: () => Promise<void>; released?: boolean; addEventListener?: (t: string, f: () => void) => void };
type WakeLock = { request: (type: 'screen') => Promise<Sentinel> };
type AudioCtor = new () => AudioContext;

/** The parts of `window` the buzz uses: tests hand in their own. */
export interface BuzzWin {
  navigator: { vibrate?: Vibrate; wakeLock?: WakeLock; userAgent?: string; maxTouchPoints?: number; platform?: string };
  AudioContext?: AudioCtor;
  webkitAudioContext?: AudioCtor;
  document?: { visibilityState?: string; addEventListener?: (t: string, f: () => void) => void };
  setTimeout?: (f: () => void, ms: number) => unknown;
}

export interface Can {
  vibrate: boolean;
  chime: boolean;
  wake: boolean;
  /** iPhone, iPad or iPod: only for the honest line and the "can't vibrate" row. */
  ios: boolean;
}

export type Cue = 'next' | 'turn';

/** design: [200,100,200] when you're next, [400,150,400,150,400] on your turn. */
export const VIBRATE: Record<Cue, number[]> = { next: [200, 100, 200], turn: [400, 150, 400, 150, 400] };
/** The chime plays once when you're next and three times on your turn. */
export const CHIMES: Record<Cue, number> = { next: 1, turn: 3 };

export function detect(w: BuzzWin): Can {
  const n = w.navigator ?? {};
  const ua = `${n.userAgent ?? ''} ${n.platform ?? ''}`;
  // An iPad says "Macintosh" now: a Mac with a touch screen is one.
  const ios = /iP(hone|ad|od)/.test(ua) || (/Mac/.test(ua) && (n.maxTouchPoints ?? 0) > 1);
  return {
    vibrate: typeof n.vibrate === 'function',
    chime: typeof (w.AudioContext ?? w.webkitAudioContext) === 'function',
    wake: !!n.wakeLock && typeof n.wakeLock.request === 'function',
    ios,
  };
}

export class Buzz {
  readonly can: Can;
  /** Sound is on: a tap unlocked audio (phones allow sound only after one). */
  sound = false;
  /** The guest asked to keep the screen on; the lock is asked for again when the page comes back. */
  wakeWanted = false;
  wakeOn = false;
  private ctx: AudioContext | null = null;
  private lock: Sentinel | null = null;
  onChange: () => void = () => {};

  constructor(private w: BuzzWin) {
    this.can = detect(w);
    w.document?.addEventListener?.('visibilitychange', () => {
      if (w.document?.visibilityState === 'visible' && this.wakeWanted && !this.wakeOn) void this.keepScreenOn();
    });
  }

  /** Turn on sound: unlocks audio and plays the chime once as a test. */
  turnOnSound(): void {
    const A = this.w.AudioContext ?? this.w.webkitAudioContext;
    if (!A) return;
    try {
      this.ctx ??= new A();
      void this.ctx.resume?.();
      this.sound = true;
      this.chime(1);
    } catch {
      this.sound = false;
    }
    this.onChange();
  }

  /** Keep screen on: the Wake Lock API. A refusal (low battery, a hidden page) leaves the button there to try again. */
  async keepScreenOn(): Promise<void> {
    const wl = this.w.navigator.wakeLock;
    if (!wl) return;
    this.wakeWanted = true;
    try {
      this.lock = await wl.request('screen');
      this.wakeOn = true;
      this.lock.addEventListener?.('release', () => {
        this.wakeOn = false;
        this.onChange();
      });
    } catch {
      this.wakeOn = false;
    }
    this.onChange();
  }

  /** The ticket is done or the page is closing: the screen may sleep again. */
  release(): void {
    this.wakeWanted = false;
    this.wakeOn = false;
    void this.lock?.release().catch(() => {});
    this.lock = null;
  }

  /** One cue for one state change: the vibrate pattern and the chime, each only where it works. */
  cue(c: Cue): void {
    if (this.can.vibrate) {
      try {
        this.w.navigator.vibrate?.(VIBRATE[c]);
      } catch {
        /* not allowed right now */
      }
    }
    if (this.sound) this.chime(CHIMES[c]);
  }

  /** A soft two-note chime (E6 then A5), `times` times, 0.7 s apart. */
  private chime(times: number): void {
    const ctx = this.ctx;
    if (!ctx) return;
    const t0 = ctx.currentTime + 0.02;
    for (let i = 0; i < times; i++) {
      const at = t0 + i * 0.7;
      this.note(ctx, 1318.5, at, 0.22);
      this.note(ctx, 880, at + 0.2, 0.34);
    }
  }

  private note(ctx: AudioContext, hz: number, at: number, len: number): void {
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.type = 'sine';
    o.frequency.value = hz;
    g.gain.setValueAtTime(0.0001, at);
    g.gain.exponentialRampToValueAtTime(0.3, at + 0.02);
    g.gain.exponentialRampToValueAtTime(0.0001, at + len);
    o.connect(g).connect(ctx.destination);
    o.start(at);
    o.stop(at + len + 0.05);
  }
}

/** The page's one buzz, made from this window. */
export const useBuzz = (w: BuzzWin): Buzz => new Buzz(w);
