// Keeps a waiting approval card's Deny / Allow buttons on screen above the composer.

/** How close to the bottom (px) still counts as "following the chat". */
export const STICK_THRESHOLD = 80;

export const isNearBottom = (el: { scrollHeight: number; scrollTop: number; clientHeight: number }) =>
  el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD;

/**
 * Whether the chat keeps following its bottom after a scroll event. Only scrolling up stops it:
 * content that grows under a pinned view (a card's details arriving) can fire a scroll event that
 * measures the taller page before it is re-pinned, and that must not count as reading history.
 */
export const nextFollowing = (wasFollowing: boolean, s: { nearBottom: boolean; scrollTop: number; lastScrollTop: number }) =>
  s.nearBottom || (wasFollowing && s.scrollTop >= s.lastScrollTop - 1);

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
