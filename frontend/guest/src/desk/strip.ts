// pa13b K4 / K5 / K7 / K11: the booking strip under the bar (pa13b-design.md "For Iris": --kw-surface-rail, 14 px,
// role=status; the booked one in the ok tint, with a plain "Booked" chip: a person's yes earns no holo).
// Requested has Cancel, which turns the strip into an inline confirm (K7, role=alertdialog, no modal). Booked has no
// Cancel: changing a booked time is a message to the host.
import { h } from '../dom';
import { book, Names } from './copy';
import { dIcon } from './look';
import { whenLabel } from './slots';
import type { Booking, Slot } from './view';

export interface StripDo {
  cancel: () => void;
  keep: () => void;
  confirm: () => void;
}

/** "Wed Dec 2, 10 AM" for the guest's own booking: from its start, else the slot's, else the box's label. */
export function bookingWhen(b: Booking, slots: Slot[]): string {
  const s = slots.find((x) => x.id === b.slot);
  return whenLabel({ id: b.slot, start: b.start ?? s?.start ?? '', label: b.label ?? s?.label });
}

export function strip(b: Booking, when: string, n: Names, o: { asking: boolean; busy: boolean; ended?: boolean }, act: StripDo): HTMLElement {
  const ok = b.state === 'confirmed';
  if (ok || o.ended) {
    return h(
      'div',
      { class: 'bks ok', role: 'status', 'data-strip': 'booked' },
      dIcon('tick'),
      h('div', { class: 'sp' }, h('b', null, when), h('small', null, o.ended ? book.ended_booked(n) : book.strip_booked(n))),
      h('span', { class: 'gchip ok' }, book.chip_booked),
    );
  }
  if (o.asking) {
    const keep = h('button', { type: 'button', class: 'gsmall', 'data-k': 'bk-keep', 'data-cancel-keep': '' }, book.cancel_keep);
    const yes = h('button', { type: 'button', class: 'gsmall bad', 'data-k': 'bk-yes', 'data-cancel-yes': '', 'aria-busy': o.busy ? 'true' : null, disabled: o.busy }, book.cancel_yes);
    keep.addEventListener('click', act.keep);
    yes.addEventListener('click', act.confirm);
    const [q, rest] = book.cancel_q(when);
    const el = h(
      'div',
      { class: 'bks ask', role: 'alertdialog', 'aria-labelledby': 'bk-q', 'aria-describedby': 'bk-q-rest', 'data-strip': 'cancel' },
      dIcon('cal'),
      h('div', { class: 'sp' }, h('b', { id: 'bk-q' }, q), h('small', { id: 'bk-q-rest' }, rest)),
      h('div', { class: 'acts' }, keep, yes),
    );
    el.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        act.keep();
      }
    });
    return el;
  }
  const cancel = h('button', { type: 'button', class: 'gsmall', 'data-k': 'bk-cancel', 'data-cancel': '' }, book.strip_cancel);
  cancel.addEventListener('click', act.cancel);
  return h(
    'div',
    { class: 'bks', role: 'status', 'data-strip': 'requested' },
    dIcon('cal'),
    h('div', { class: 'sp' }, h('b', null, when), h('small', null, book.strip_requested(n))),
    cancel,
  );
}
