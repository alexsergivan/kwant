// The page for a link whose pocket is gone (deleted with Delete now, or past its expiry), or never existed. The server
// sends the same bundle with a boot of exactly {"gone": true} and status 404, and nothing else: the code is unknown, so
// this screen names no kit, host or title. Static: no fetch, no socket, no wake listeners (santa-gone-page).
import { h, quizmaster } from './dom';

export const GONE_TITLE = 'This page has ended';
// A no-break space keeps "part!" off a line of its own at 390 px (Mira D1).
export const GONE_LEAD = 'Everything in it was deleted. Thanks for taking\u00a0part!';
/** For the guest who mistyped a link or got one for something never made (Mira D2). */
export const GONE_SMALL = 'Expected something here? Check the link with whoever sent it.';

/** True for the server's gone boot: an object whose `gone` is exactly `true`. */
export const isGone = (b: unknown): boolean => !!b && typeof b === 'object' && (b as { gone?: unknown }).gone === true;

/** Draws the ended screen into `root`, in the same frame as the other guest screens. */
export function renderGone(root: HTMLElement, doc: Document = document): void {
  doc.title = GONE_TITLE;
  const page = h(
    'main',
    { class: 'g s-gone', 'data-screen': 'gone' },
    h('header', { class: 'bar' }, quizmaster(28), h('span', { class: 'sp' })),
    h('div', { class: 'body mid' }, h('div', { class: 'qmwrap' }, quizmaster(96, 'finished')), h('h2', null, GONE_TITLE), h('p', { class: 'lead' }, GONE_LEAD), h('p', { class: 'small' }, GONE_SMALL)),
    h('footer', { class: 'madewith' }, 'made with ', h('a', { href: 'https://kwant.si', rel: 'noopener noreferrer', target: '_blank' }, 'kwant')),
  );
  root.replaceChildren(page);
}
