import { useEffect } from 'react';

// Living avatars (autonomy/living-avatars/design.md, "Idle life" and "Speed"): ONE shared clock for every live avatar
// in the app. It never renders React: ModernAvatar draws the rest face plus hidden frame groups, and this module only
// sets `data-life` (which frame shows) and `data-hop` (the body up one cell) on the avatar's root. CSS does the rest.
//
//   - One `setTimeout`, armed for the soonest due step or idle event, for the whole app (hover delays included).
//   - One shared IntersectionObserver: avatars off screen get no idle events.
//   - The document hidden (or the Mac app's window hidden): the timer is cleared, any frame in flight ends at rest.
//   - Reduced motion: nothing registers, nothing is scheduled; faces still follow the mood (ModernAvatar).
//   - Caps: at most 3 avatars mid-event (moments count too, though they never wait), at least 400 ms between idle
//     event starts, one sidebar blink per 1.5 s, and each avatar rests 1 s after its own event before the next idle
//     one (so no avatar flips more than ~3 times a second; Mira's design check, Nov 26).
//
// The timings are life.js's (design/life.js), which Mira's mockup proved.

export type LifeZone = 'full' | 'header' | 'list';
export type LifeFrame = 'closed' | 'glanceL' | 'glanceR' | 'twitch' | 'perk' | 'happy';
/** How an idle creature fidgets: an accessory twitch, the bulb (once, held, rare), or glances only. */
export type LifeQuirk = 'twitch' | 'bulb' | 'none';

interface ZoneRates {
  blink: [number, number];
  double: number;
  quirk: [number, number] | null;
}
export const ZONES: Record<LifeZone, ZoneRates> = {
  full: { blink: [2800, 6000], double: 0.2, quirk: [8000, 15000] },
  header: { blink: [4000, 8000], double: 0.15, quirk: [12000, 20000] },
  list: { blink: [8000, 16000], double: 0, quirk: null },
};
export const BLINK = 120;
export const DOUBLE_GAP = 100;
export const SLOW_BLINK = 400;
export const GLANCE = 600;
export const TWITCH = 120;
export const BULB_HOLD = 600;
export const BULB_EVERY = 8000;
export const HOP = 120;
export const BLINK_THROUGH = 100;
export const NOTICE_BLINK = 160;
export const HOVER_DELAY = 150;
export const MAX_ACTIVE = 3;
export const MIN_GAP = 400;
export const LIST_GAP = 1500;
export const FIRST_MIN = 600;
/** An avatar rests this long after its own event ends before its next idle event or hover glance. */
export const REST_AFTER = 1000;
const HELLO_WAIT = 5000;

/** The root element a live avatar is drawn in (an HTMLElement in the app; any attribute holder in tests). */
export interface LifeEl {
  setAttribute(k: string, v: string): void;
  removeAttribute(k: string): void;
}

export interface LifeEnv {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(t: unknown): void;
  random(): number;
  reduced(): boolean;
}

interface Entry {
  el: LifeEl;
  zone: LifeZone;
  mood: string;
  kwantId?: string;
  quirk: LifeQuirk;
  busy: boolean;
  /** Counted in the 3-at-once cap. Idle events and hover wait for a free slot; moments start at once but count. */
  counted: boolean;
  /** When this avatar's last event ended (or was cut short): it rests REST_AFTER from here. */
  lastEnd: number;
  visible: boolean;
  nextBlink: number;
  nextQuirk: number;
  lastBulb: number;
}
type Step = [frame: LifeFrame | null, hop: boolean, ms: number];
interface Queued {
  t: number;
  e: Entry;
  fn: () => void;
}

export interface LifeOptions {
  zone: LifeZone;
  mood?: string;
  kwantId?: string;
  quirk?: LifeQuirk;
}

/** Moods that blink now and then (finished already has ^ ^ eyes; failed never moves). */
const BLINKS = new Set(['idle', 'thinking', 'working', 'needs_you', 'resting']);

export function createLife(env: LifeEnv) {
  const entries = new Map<LifeEl, Entry>();
  let queue: Queued[] = [];
  let timer: unknown = null;
  let active = 0;
  let lastStart = -Infinity;
  let lastList = -Infinity;
  let paused = false;
  let started = 0;
  const pendingHello = new Map<string, number>();
  let hoverEl: LifeEl | null = null;

  const between = ([a, b]: [number, number]) => a + env.random() * (b - a);

  const show = (e: Entry, frame: LifeFrame | null, hop: boolean) => {
    if (frame) e.el.setAttribute('data-life', frame);
    else e.el.removeAttribute('data-life');
    if (hop) e.el.setAttribute('data-hop', '');
    else e.el.removeAttribute('data-hop');
  };

  const push = (t: number, e: Entry, fn: () => void) => {
    queue.push({ t, e, fn });
    queue.sort((x, y) => x.t - y.t);
  };

  /** Drop an entry's queued steps and put its rest face back (a moment wins over an idle event). */
  const cancel = (e: Entry) => {
    queue = queue.filter((q) => q.e !== e);
    if (e.busy) {
      e.busy = false;
      if (e.counted) active--;
      e.counted = false;
      e.lastEnd = env.now();
    }
    show(e, null, false);
  };

  const play = (e: Entry, steps: Step[], counted: boolean, delay = 0) => {
    cancel(e);
    e.busy = true;
    e.counted = counted;
    if (counted) active++;
    let t = env.now() + delay;
    for (const [frame, hop, ms] of steps) {
      push(t, e, () => show(e, frame, hop));
      t += ms;
    }
    push(t, e, () => {
      show(e, null, false);
      e.busy = false;
      if (e.counted) active--;
      e.counted = false;
      e.lastEnd = env.now();
    });
    // A step due now shows now (before paint, for the blink-through), not on the next timer.
    if (delay <= 0) drain(env.now());
  };

  function drain(now: number) {
    while (queue.length && queue[0].t <= now) queue.shift()!.fn();
  }

  /** When this entry's next idle event may start, given the caps (Infinity: none due). */
  const idleDue = (e: Entry): number => {
    if (!e.visible || e.busy) return Infinity;
    const due = Math.min(e.nextBlink, e.nextQuirk);
    if (!Number.isFinite(due)) return Infinity;
    return Math.max(due, lastStart + MIN_GAP, e.lastEnd + REST_AFTER, e.zone === 'list' ? lastList + LIST_GAP : -Infinity);
  };

  function arm() {
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    if (paused || !entries.size) return;
    let soonest = queue.length ? queue[0].t : Infinity;
    if (active < MAX_ACTIVE) for (const e of entries.values()) soonest = Math.min(soonest, idleDue(e));
    if (soonest === Infinity) return;
    timer = env.setTimeout(tick, Math.max(0, soonest - env.now()));
  }

  function idleEvent(e: Entry, now: number) {
    const z = ZONES[e.zone];
    const blinkDue = e.nextBlink <= e.nextQuirk;
    if (blinkDue) {
      e.nextBlink = now + between(z.blink) * (e.mood === 'resting' ? 1.5 : 1);
      if (!BLINKS.has(e.mood)) return false;
      const resting = e.mood === 'resting';
      const steps: Step[] = !resting && env.random() < z.double
        ? [['closed', false, BLINK], [null, false, DOUBLE_GAP], ['closed', false, BLINK]]
        : [['closed', false, resting ? SLOW_BLINK : BLINK]];
      play(e, steps, true);
      if (e.zone === 'list') lastList = now;
      return true;
    }
    e.nextQuirk = z.quirk ? now + between(z.quirk) : Infinity;
    if (e.mood !== 'idle') return false;
    const bulbReady = e.quirk === 'bulb' && now - e.lastBulb >= BULB_EVERY;
    const twitch = (e.quirk === 'twitch' || bulbReady) && env.random() < 0.5;
    if (twitch && e.quirk === 'bulb') {
      e.lastBulb = now;
      play(e, [['twitch', false, BULB_HOLD]], true);
    } else if (twitch) play(e, [['twitch', false, TWITCH], [null, false, TWITCH], ['twitch', false, TWITCH]], true);
    else play(e, [[env.random() < 0.5 ? 'glanceL' : 'glanceR', false, GLANCE]], true);
    return true;
  }

  function tick() {
    timer = null;
    const now = env.now();
    drain(now);
    for (const e of entries.values()) {
      if (active >= MAX_ACTIVE) break;
      if (idleDue(e) > now) continue;
      if (idleEvent(e, now)) lastStart = now;
    }
    arm();
  }

  const firstEvents = (e: Entry, now: number) => {
    const z = ZONES[e.zone];
    // The first event lands at a random 0.6 s to the zone's max, so nothing syncs.
    e.nextBlink = now + between([FIRST_MIN, z.blink[1]]);
    e.nextQuirk = z.quirk ? now + between([FIRST_MIN, z.quirk[1]]) : Infinity;
  };

  return {
    /** Start idle life on an avatar root. Returns the unregister function. Under reduced motion: nothing. */
    register(el: LifeEl, o: LifeOptions): () => void {
      if (env.reduced()) return () => undefined;
      const e: Entry = { el, zone: o.zone, mood: o.mood ?? 'idle', kwantId: o.kwantId, quirk: o.quirk ?? 'none', busy: false, counted: false, lastEnd: -Infinity, visible: true, nextBlink: Infinity, nextQuirk: Infinity, lastBulb: -Infinity };
      firstEvents(e, env.now());
      entries.set(el, e);
      started++;
      const hello = o.kwantId ? pendingHello.get(o.kwantId) : undefined;
      if (o.kwantId && hello !== undefined && env.now() - hello < HELLO_WAIT) {
        pendingHello.delete(o.kwantId);
        playHello(e);
      }
      arm();
      return () => {
        const cur = entries.get(el);
        if (!cur) return;
        cancel(cur);
        entries.delete(el);
        arm();
      };
    },
    /** The avatar's mood changed (a mood re-render already happened): blink through, then the entry hops. */
    setMood(el: LifeEl, mood: string) {
      const e = entries.get(el);
      if (!e || e.mood === mood) return;
      const from = e.mood;
      e.mood = mood;
      if (paused || !e.visible) return cancel(e);
      const steps: Step[] = [];
      // Happy eyes are already closed carets; failed never moves: the face swaps at once.
      if (mood !== 'failed' && from !== 'finished') steps.push(['closed', false, BLINK_THROUGH]);
      if (mood === 'finished') steps.push([null, true, HOP], [null, false, HOP]);
      if (mood === 'needs_you') steps.push([null, true, HOP], [null, false, HOP], [null, true, HOP]);
      if (steps.length) play(e, steps, true);
      else cancel(e);
      arm();
    },
    setVisible(el: LifeEl, visible: boolean) {
      const e = entries.get(el);
      if (!e || e.visible === visible) return;
      e.visible = visible;
      if (!visible && e.busy && e.counted) cancel(e);
      if (visible) firstEvents(e, env.now());
      arm();
    },
    /** Document hidden or shown. Hidden: no timer, and anything mid-frame goes back to rest. */
    setHidden(hidden: boolean) {
      if (paused === hidden) return;
      paused = hidden;
      if (hidden) {
        for (const e of entries.values()) cancel(e);
        queue = [];
      } else {
        const now = env.now();
        for (const e of entries.values()) firstEvents(e, now);
      }
      arm();
    },
    /** Perk-up: you sent this bot a message. After today's 160 ms blink: wide eyes, "o" and one hop. */
    perk(kwantId: string) {
      if (paused) return;
      for (const e of entries.values()) if (e.kwantId === kwantId && e.visible) play(e, [['perk', true, HOP]], true, NOTICE_BLINK);
      arm();
    },
    /** Hello: a bot that just joined the space. Plays now on its live avatars, or on the next one to mount (5 s). */
    hello(kwantId: string) {
      if (env.reduced()) return;
      let any = false;
      for (const e of entries.values())
        if (e.kwantId === kwantId) {
          any = true;
          if (!paused) playHello(e);
        }
      if (!any) pendingHello.set(kwantId, env.now());
      arm();
    },
    /** The sidebar row under the pointer: after 150 ms it blinks, then (idle) glances. */
    hover(el: LifeEl | null) {
      for (const e of entries.values()) if (e.zone === 'list') queue = queue.filter((q) => !(q.e === e && q.fn === hoverStart));
      hoverEl = el;
      const e = el ? entries.get(el) : undefined;
      if (e && !paused) push(env.now() + HOVER_DELAY, e, hoverStart);
      arm();
    },
    stats() {
      return { timers: timer === null ? 0 : 1, registered: entries.size, active, queued: queue.length, started, paused };
    },
  };

  function playHello(e: Entry) {
    play(e, [['happy', true, HOP], ['happy', false, HOP], ['happy', true, HOP], ['happy', false, BULB_HOLD]], true);
  }

  // The hover step is one shared function, so a pointer moving on can find and drop it.
  function hoverStart() {
    const e = hoverEl ? entries.get(hoverEl) : undefined;
    if (!e || e.busy || !BLINKS.has(e.mood) || active >= MAX_ACTIVE) return;
    // Still resting after its own event: glance once the rest is over, if the pointer is still there.
    const restUntil = e.lastEnd + REST_AFTER;
    if (env.now() < restUntil) return push(restUntil, e, hoverStart);
    play(e, e.mood === 'idle' ? [['closed', false, BLINK], [null, false, 200], ['glanceR', false, GLANCE]] : [['closed', false, BLINK]], true);
    if (e.zone === 'list') lastList = env.now();
  }
}

export type Life = ReturnType<typeof createLife>;

// ---- The app's one clock ----

const reducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

let app: Life | null = null;
let io: IntersectionObserver | null = null;

function appLife(): Life {
  if (app) return app;
  const life = createLife({
    now: () => Date.now(),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (t) => window.clearTimeout(t as number),
    random: Math.random,
    reduced: reducedMotion,
  });
  app = life;
  if (typeof document !== 'undefined') {
    life.setHidden(document.hidden);
    document.addEventListener('visibilitychange', () => life.setHidden(document.hidden));
    // The row under the pointer (Sidebar marks its bot rows with data-life-row).
    let row: Element | null = null;
    document.addEventListener('pointerover', (ev) => {
      const next = (ev.target as Element | null)?.closest?.('[data-life-row]') ?? null;
      if (next === row) return;
      row = next;
      life.hover((row?.querySelector('[data-life-zone="list"]') as HTMLElement | null) ?? null);
    });
  }
  if (typeof IntersectionObserver !== 'undefined')
    io = new IntersectionObserver((es) => es.forEach((x) => life.setVisible(x.target as HTMLElement, x.isIntersecting)));
  return life;
}

/** Join the one shared clock from a component (no timer of its own): life on `ref`'s element while mounted. */
export function useLife(ref: { current: HTMLElement | null }, o: LifeOptions): void {
  const { zone, mood, kwantId, quirk } = o;
  useEffect(() => {
    if (!ref.current) return;
    return registerLife(ref.current, { zone, mood, kwantId, quirk });
  }, [ref, zone, mood, kwantId, quirk]);
}

/** ModernAvatar: start idle life on its root (when `life` is set). Returns the cleanup. */
export function registerLife(el: HTMLElement, o: LifeOptions): () => void {
  if (reducedMotion()) return () => undefined;
  const life = appLife();
  const off = life.register(el, o);
  io?.observe(el);
  return () => {
    io?.unobserve(el);
    off();
  };
}
export const lifeMood = (el: HTMLElement, mood: string) => app?.setMood(el, mood);
/** You sent this bot a message (ChatView): every live avatar of it perks up. */
export const perkBot = (kwantId: string) => app?.perk(kwantId);
/** A bot joined the space during this session. */
export const helloBot = (kwantId: string) => {
  if (!reducedMotion()) appLife().hello(kwantId);
};
/** For the perf budget and tests: the one timer, live avatars, and what is mid-event. */
export const lifeStats = () => app?.stats() ?? { timers: 0, registered: 0, active: 0, queued: 0, started: 0, paused: false };

// Hello: the bot ids each space has shown in this session. The first list a space loads fills its set; an id that
// shows up later (created or added) says hello once.
const seenBots = new Map<string, Set<string>>();
export function noteSpaceBots(spaceId: string, ids: string[]): string[] {
  const seen = seenBots.get(spaceId);
  if (!seen) {
    seenBots.set(spaceId, new Set(ids));
    return [];
  }
  const fresh = ids.filter((id) => !seen.has(id));
  fresh.forEach((id) => {
    seen.add(id);
    helloBot(id);
  });
  return fresh;
}
