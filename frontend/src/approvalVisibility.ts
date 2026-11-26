// Keeps a waiting approval card's Deny / Allow buttons on screen above the composer.

/** How close to the bottom (px) still counts as "following the chat". */
export const STICK_THRESHOLD = 80;

export const isNearBottom = (el: { scrollHeight: number; scrollTop: number; clientHeight: number }) =>
  el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD;

/**
 * Whether the chat keeps following its bottom after a scroll event.
 * - Following, it stays following unless the view moved up. Content that grows under a pinned view (a card's
 *   details arriving) can fire a scroll event that measures the taller page before it is re-pinned: no move, so it
 *   still follows. A move up while still exactly at the bottom is content shrinking under it (a clamp), not reading.
 * - chat-scroll-holds-place: any other move up stops following, also within STICK_THRESHOLD of the bottom (a 20 px
 *   wheel step used to count as following, so the next streamed word pulled the reader back down).
 * - Reading, it follows again only at the very bottom, or when the person's own scroll (`user`) moves down into
 *   STICK_THRESHOLD. A move made by the page (the chat holding the reader's place, the browser's scroll anchoring)
 *   never starts following.
 * `user` defaults to true (a scroll with no known source counts as the person's); `atBottom` to `gap <= 2`.
 */
export const nextFollowing = (
  wasFollowing: boolean,
  s: { nearBottom: boolean; scrollTop: number; lastScrollTop: number; user?: boolean; atBottom?: boolean },
): boolean => {
  const user = s.user ?? true;
  const atBottom = s.atBottom ?? false;
  if (s.scrollTop < s.lastScrollTop - 1) return wasFollowing && atBottom && !user;
  if (wasFollowing || atBottom) return true;
  return user && s.nearBottom && s.scrollTop > s.lastScrollTop + 1;
};

/** Distance (px) from the bottom that still counts as being at it (sub-pixel scroll positions, rounding). */
export const AT_BOTTOM = 2;

export const isAtBottom = (el: { scrollHeight: number; scrollTop: number; clientHeight: number }) =>
  el.scrollHeight - el.scrollTop - el.clientHeight <= AT_BOTTOM;

/** Keys that scroll the chat when focus is in it or on the page, and which way: 1 up (towards older), -1 down. */
const SCROLL_KEYS: Record<string, 1 | -1> = { PageUp: 1, ArrowUp: 1, Home: 1, PageDown: -1, ArrowDown: -1, End: -1, ' ': -1 };

/**
 * chat-scroll-holds-place: which way the person's own input scrolls the chat, before the scroll happens (a pin in
 * the same frame would otherwise undo it): 'up' to read older messages, 'down', or null (not a scroll).
 * - wheel/trackpad: the sign of deltaY;
 * - touch: the finger moving down the screen scrolls up (dy is this touchmove's clientY minus the last one's);
 * - keys: PageUp/ArrowUp/Home (and Shift+Space) up; PageDown/ArrowDown/End/Space down. Not while typing.
 */
export function scrollIntent(
  e: { kind: 'wheel'; deltaY: number } | { kind: 'touch'; dy: number } | { kind: 'key'; key: string; shiftKey?: boolean; editable?: boolean },
): 'up' | 'down' | null {
  if (e.kind === 'wheel') return e.deltaY < 0 ? 'up' : e.deltaY > 0 ? 'down' : null;
  if (e.kind === 'touch') return e.dy > 0 ? 'up' : e.dy < 0 ? 'down' : null;
  if (e.editable || e.key.length === 0) return null;
  const k = SCROLL_KEYS[e.key];
  if (!k) return null;
  if (e.key === ' ' && e.shiftKey) return 'up';
  return k === 1 ? 'up' : 'down';
}

/**
 * chat-scroll-holds-place: how far (px) to scroll so the message the reader is looking at stays where it was on
 * screen. `top` is its offset from the scroller's top when the place was taken, at scrollTop `atScrollTop`; `now`
 * is its offset now. 0 when the scroller itself moved since (a scroll the person made, or the browser's own
 * anchoring already did it: that scroll's event takes the place again), or when it moved less than half a pixel.
 */
export function holdPlaceBy(place: { top: number; atScrollTop: number }, now: { top: number; scrollTop: number }): number {
  if (Math.abs(now.scrollTop - place.atScrollTop) > 0.5) return 0;
  const d = now.top - place.top;
  return Math.abs(d) > 0.5 ? d : 0;
}

/**
 * chat-scroll-holds-place: whether 33q's lazy rows (content-visibility: auto) hold still in this engine. Not in WebKit
 * (Safari, the Mac app's WKWebView, iPhone and iPad browsers, WebKitGTK), even where it has scroll anchoring: a row
 * coming on screen is drawn a frame at its 160 px estimate before its real height, and the rows below jump by the
 * difference. Chromium (Chrome, Edge, the Windows app) and Firefox draw it at its real height.
 */
export const lazyRowsHold = (ua: string): boolean => !/AppleWebKit\//.test(ua) || /(Chrome|Chromium|Edg)\//.test(ua);

/** Ids of cards that weren't waiting before. */
export const newApprovalIds = (seen: ReadonlySet<string>, ids: readonly string[]): string[] =>
  ids.filter((id) => !seen.has(id));

/** Whether a box (e.g. the card's button row) lies fully inside the scroll area's visible part. */
export const fullyVisible = (box: { top: number; bottom: number }, view: { top: number; bottom: number }) =>
  box.top >= view.top - 1 && box.bottom <= view.bottom + 1;

/**
 * What to do when the chat's content or size changes while cards wait:
 * - `follow`: the person is following the chat, so keep the bottom (and the newest card's
 *   buttons) in view, also when a card grows after its details arrive or the composer grows;
 * - `notify`: they scrolled up to read, so leave the scroll alone and offer "Approval waiting";
 * - `none`: nothing waits, or its buttons are already on screen.
 */
export function approvalScrollAction(s: { waiting: number; following: boolean; buttonsVisible: boolean }): 'follow' | 'notify' | 'none' {
  if (s.following) return 'follow';
  if (s.waiting === 0 || s.buttonsVisible) return 'none';
  return 'notify';
}
