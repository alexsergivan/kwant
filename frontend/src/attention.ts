// "Needs you" nudges for the web app and the PWA (0.6.9): an in-app pill when a NEW question, approval
// or browser step arrives for a chat the person isn't looking at, the "!" favicon while something
// waits, and the installed app's icon badge. Pure rules plus small browser helpers that tests drive
// with fakes. The counts come from /api/away (`unseen.waiting`); nothing here invents a number.

import type { AwayItem } from './away';
import { awayTarget } from './away';
import { dotGeometry } from './tabSignal';

export type NudgeKind = 'question' | 'approval' | 'human_step';

/** One thing the pill can show. `away` (questions, approvals) or `kwantId`/`channelId` (browser steps) say where a click goes. */
export interface Nudge {
  key: string;
  kind: NudgeKind;
  kwantId: string;
  kwantName: string;
  title: string;
  /** The chat it belongs to: a thread id, a channel id or the bot's direct chat id. */
  ctx: string;
  away?: AwayItem;
  channelId?: string | null;
}

/** How long the pill stays when nobody hovers or focuses it (Mira's pill: 5 s, then 2 s after the pointer leaves). */
export const PILL_MS = 5000;
export const PILL_AFTER_HOVER_MS = 2000;
/** Hard cap from when the pill first appeared: a stream of arrivals can't pin it (hover and focus still hold it). */
export const PILL_CAP_MS = 12000;

/**
 * How long the pill waits before it goes, counted from now. `elapsed` is the time since it first appeared;
 * `held` means the pointer or focus has been on it and left. After a hold it goes 2 s later, whatever the
 * cap says, so nobody is cut off mid-read. Otherwise 5 s from the last arrival, but never past the cap.
 */
export function pillHoldMs(elapsed: number, held: boolean): number {
  if (held) return PILL_AFTER_HOVER_MS;
  return Math.max(0, Math.min(PILL_MS, PILL_CAP_MS - elapsed));
}
/** The leave animation before it unmounts. */
export const PILL_LEAVE_MS = 160;
/** The same browser step announced twice within this window is one arrival. */
export const STEP_DEDUPE_MS = 60_000;

export const NUDGES_KEY = 'kwnt_nudges';
export const NUDGES_SETTING_EVENT = 'kwnt-nudges-setting';

/** The chat an away item lives in, the same id the app uses for the open chat. */
export function awayCtx(item: Pick<AwayItem, 'kwant_id' | 'link'>): string {
  const t = awayTarget(item);
  return 'thread' in t ? t.thread : 'channel' in t ? t.channel : t.direct;
}

export function nudgeFromAway(item: AwayItem): Nudge | null {
  if (item.kind !== 'question' && item.kind !== 'approval') return null;
  return {
    key: `${item.kind}:${item.id}`,
    kind: item.kind,
    kwantId: item.kwant_id,
    kwantName: item.kwant_name,
    title: item.title,
    ctx: awayCtx(item),
    away: item,
  };
}

/** What the person is looking at right now. `looking` is the open chat id, or 'inbox' on the Inbox. */
export interface AttentionView {
  /** The open chat (thread, channel or direct chat id) while a chat is on screen, else null. */
  openCtx: string | null;
  /** On the Inbox every waiting item is already in front of them. */
  onInbox: boolean;
  /** The tab is in the background: the OS notification or push covers it. */
  hidden: boolean;
  /** The "Show nudges" setting. */
  enabled: boolean;
}

/** Whether a new arrival gets the pill. It is marked seen either way, so it never pops later. */
export function shouldPill(n: Pick<Nudge, 'ctx'>, view: AttentionView): boolean {
  if (!view.enabled || view.hidden || view.onInbox) return false;
  return view.openCtx !== n.ctx;
}

/**
 * Remembers which waiting ids it has seen. The first list primes it (what already waits when the
 * app opens is shown by the badge, title and Inbox, not by a pill); after that only ids it has
 * never seen are arrivals.
 */
export function createArrivals() {
  let primed = false;
  const seen = new Set<string>();
  const steps = new Map<string, number>();
  return {
    /** The waiting list from /api/away: the new items, in the server's order (newest first). */
    fromAway(items: readonly AwayItem[]): Nudge[] {
      const nudges = items.map(nudgeFromAway).filter((n): n is Nudge => !!n);
      const fresh = primed ? nudges.filter((n) => !seen.has(n.key)) : [];
      for (const n of nudges) seen.add(n.key);
      primed = true;
      // Keep the set small: ids that stopped waiting can't come back under the same id.
      if (seen.size > 500) {
        const live = new Set(nudges.map((n) => n.key));
        for (const k of seen) if (!live.has(k)) seen.delete(k);
      }
      return fresh;
    },
    /** A browser step the bot handed over (not in /api/away): one arrival per bot and title a minute. */
    fromHumanStep(ev: { kwant_id: string; kwant_name: string; channel_id: string | null; title: string; body?: string }, now = Date.now()): Nudge | null {
      const title = ev.body || ev.title;
      const key = `human_step:${ev.kwant_id}:${title}`;
      const last = steps.get(key);
      steps.set(key, now);
      if (last !== undefined && now - last < STEP_DEDUPE_MS) return null;
      return { key, kind: 'human_step', kwantId: ev.kwant_id, kwantName: ev.kwant_name, title, ctx: ev.channel_id ?? ev.kwant_id, channelId: ev.channel_id };
    },
    get primed() {
      return primed;
    },
  };
}

/** The pill's queue: arrivals while it shows coalesce into one "N things need you". */
export interface PillState {
  items: Nudge[];
  /** Bumped on every change, so the 5 s timer restarts when more arrive. */
  rev: number;
}

export const EMPTY_PILL: PillState = { items: [], rev: 0 };

export function pillAdd(state: PillState, arrivals: readonly Nudge[]): PillState {
  const keys = new Set(state.items.map((n) => n.key));
  const add = arrivals.filter((n) => !keys.has(n.key) && (keys.add(n.key), true));
  if (!add.length) return state;
  return { items: [...add, ...state.items], rev: state.rev + 1 };
}

/** Drops questions and approvals that stopped waiting (answered elsewhere) while the pill shows. */
export function pillPrune(state: PillState, waitingKeys: ReadonlySet<string>): PillState {
  const items = state.items.filter((n) => n.kind === 'human_step' || waitingKeys.has(n.key));
  return items.length === state.items.length ? state : { items, rev: state.rev + 1 };
}

/** Who several items are from (Mira's pill, line 2): "All from Juno", "Juno and Lin", "Juno, Lin and 2 more". */
export function fromNames(items: readonly Pick<Nudge, 'kwantName'>[]): string {
  const u = [...new Set(items.map((i) => i.kwantName || 'A bot'))];
  if (u.length === 1) return `All from ${u[0]}`;
  if (u.length <= 3) return `${u.slice(0, -1).join(', ')} and ${u[u.length - 1]}`;
  return `${u.slice(0, 2).join(', ')} and ${u.length - 2} more`;
}

const VERB: Record<NudgeKind, string> = { approval: 'needs your OK', question: 'asked you something', human_step: 'needs you to do a step' };

/** The pill's two lines. One item: who and what; several: how many and from whom, and it opens the Inbox. */
export function pillText(items: readonly Pick<Nudge, 'kind' | 'kwantName' | 'title'>[]): { headline: string; detail: string; action: string } {
  if (items.length > 1) return { headline: `${items.length} things need you`, detail: fromNames(items), action: 'Open the Inbox' };
  const n = items[0];
  if (!n) return { headline: '', detail: '', action: '' };
  return { headline: `${n.kwantName || 'A bot'} ${VERB[n.kind]}`, detail: n.title, action: 'Open the chat' };
}

// ---------------------------------------------------------------------------
// Browser glue. Each helper takes an env (the browser's globals by default) and never throws.

export interface AttentionEnv {
  navigator?: {
    setAppBadge?: (n?: number) => Promise<void>;
    clearAppBadge?: () => Promise<void>;
  };
  matchMedia?: (query: string) => { matches: boolean };
  localStorage?: Pick<Storage, 'getItem' | 'setItem'>;
  dispatchEvent?: (e: Event) => boolean;
}

const browserEnv = (): AttentionEnv => globalThis as unknown as AttentionEnv;

/** "Show nudges": on unless the person switched it off. */
export function nudgesEnabled(env: AttentionEnv = browserEnv()): boolean {
  try {
    return env.localStorage?.getItem(NUDGES_KEY) !== 'off';
  } catch {
    return true;
  }
}

export function setNudgesEnabled(on: boolean, env: AttentionEnv = browserEnv()): void {
  try {
    env.localStorage?.setItem(NUDGES_KEY, on ? 'on' : 'off');
  } catch {
    // storage blocked: the choice lasts until reload
  }
  try {
    if (typeof CustomEvent !== 'undefined') env.dispatchEvent?.(new CustomEvent(NUDGES_SETTING_EVENT, { detail: on }));
  } catch {
    // no event target
  }
}

/** The pill's entrance: a drop with a small spring, or a plain fade when the device asks for less motion. */
export function pillMotion(env: AttentionEnv = browserEnv()): 'drop' | 'fade' {
  try {
    return env.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'fade' : 'drop';
  } catch {
    return 'drop';
  }
}

/** Whether this browser can badge the installed app's icon (Chrome/Edge desktop, Android, iOS 16.4+ home-screen apps). */
export function canBadge(env: AttentionEnv = browserEnv()): boolean {
  return typeof env.navigator?.setAppBadge === 'function';
}

/** The app icon shows how many things wait; cleared at 0. Information, not motion: the setting doesn't turn it off. */
export function setAppBadgeCount(waiting: number, env: AttentionEnv = browserEnv()): 'set' | 'cleared' | 'unsupported' {
  const nav = env.navigator;
  if (!nav || typeof nav.setAppBadge !== 'function') return 'unsupported';
  try {
    if (waiting > 0) {
      nav.setAppBadge.call(nav, waiting)?.catch?.(() => undefined);
      return 'set';
    }
    if (typeof nav.clearAppBadge === 'function') nav.clearAppBadge.call(nav)?.catch?.(() => undefined);
    else nav.setAppBadge.call(nav, 0)?.catch?.(() => undefined);
    return 'cleared';
  } catch {
    return 'unsupported';
  }
}

// ---------------------------------------------------------------------------
// The "!" favicon. Pixel's tab signal (tabSignal.ts) owns the violet "working" dot; while something
// waits the "!" wins and the app passes working=false to it.

/** Fallback drawing colours (the AvatarMood "needs you" amber) for a page whose icon has no "!" variant. */
export const NUDGE_ICON = { fill: '#ffb020', ring: '#120e26', ink: '#1b1033' };

interface LinkLike { href: string; type?: string }
export interface IconDoc {
  iconLink(): LinkLike | null;
  /** The icon with the "!" badge as an image URL, or null when it can't be drawn. */
  drawMarked(originalHref: string): Promise<string | null>;
}

export function createNudgeIcon(doc: IconDoc) {
  const first = doc.iconLink();
  const original = first ? { href: first.href, type: first.type } : null;
  let want = false;
  let ticket = 0;
  let marked: string | null = null;
  return {
    async set(on: boolean) {
      const link = doc.iconLink();
      if (!link || !original || want === on) return;
      want = on;
      const mine = ++ticket;
      if (on) {
        const url = marked ?? (await doc.drawMarked(original.href));
        if (url) marked = url;
        if (url && want && mine === ticket) {
          link.href = url;
          link.type = /\.svg(\?|$)/.test(url) ? 'image/svg+xml' : 'image/png';
        }
      } else if (marked && link.href === marked) {
        link.href = original.href;
        if (original.type !== undefined) link.type = original.type;
      }
    },
  };
}

/** Mira's "!" favicon (design/web/icon-needs-you.svg), shipped next to icon.svg. */
export const NEEDS_YOU_ICON = '/icon-needs-you.svg';

/** The "!" variant of a page icon: Mira's SVG for kwant's own icon, else null (draw one). */
export function markedIconFor(href: string): string | null {
  try {
    const u = new URL(href, 'http://x');
    return u.pathname === '/icon.svg' ? NEEDS_YOU_ICON : null;
  } catch {
    return null;
  }
}

/** The real document: Mira's "!" icon, or the page's icon on a canvas with an amber "!" badge. */
export function browserIconDoc(): IconDoc {
  return {
    iconLink: () => document.querySelector<HTMLLinkElement>('link[rel~="icon"]'),
    drawMarked: (href) =>
      markedIconFor(href) ? Promise.resolve(markedIconFor(href)) : new Promise((resolve) => {
        const img = new Image();
        img.onload = () => {
          try {
            const size = 64;
            const c = document.createElement('canvas');
            c.width = c.height = size;
            const g = c.getContext('2d');
            if (!g) return resolve(null);
            g.drawImage(img, 0, 0, size, size);
            const { cx, cy, ring } = dotGeometry(size);
            const r = Math.round(size * 0.24);
            const x = Math.min(cx, size - r - ring - 1);
            const y = Math.min(cy, size - r - ring - 1);
            g.fillStyle = NUDGE_ICON.ring;
            g.beginPath(); g.arc(x, y, r + ring, 0, Math.PI * 2); g.fill();
            g.fillStyle = NUDGE_ICON.fill;
            g.beginPath(); g.arc(x, y, r, 0, Math.PI * 2); g.fill();
            // "!" in whole pixels: a bar and a dot.
            const w = Math.max(2, Math.round(r * 0.32));
            g.fillStyle = NUDGE_ICON.ink;
            g.fillRect(Math.round(x - w / 2), Math.round(y - r * 0.62), w, Math.round(r * 0.78));
            g.fillRect(Math.round(x - w / 2), Math.round(y + r * 0.3), w, w);
            resolve(c.toDataURL('image/png'));
          } catch {
            resolve(null);
          }
        };
        img.onerror = () => resolve(null);
        img.src = href;
      }),
  };
}
