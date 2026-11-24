// pa13b K3 / K8 / K10: the "Pick a time" sheet (pa13b-design.md "For Iris").
// A bottom sheet: role=dialog with the title focused on open, Esc and the close button shut it and focus goes back to
// whatever opened it. Day tabs are a tablist (arrows move and pick), times a radiogroup in 3 columns (arrows move and
// pick). The page draws it again on every paint like the rest of the chat, so its state lives in `Pick` (kept by the
// app) and every control carries a data-k for the app's focus keeping. Only open / not open is ever shown: no names,
// no counts of other guests, no payment fields.
import { fmt, h, Kid } from '../dom';
import { book, Line, Names } from './copy';
import { dIcon } from './look';
import { dayMinutes, Day, groupDays, lengthLabel, openSlots, timeLabel, whenLabel } from './slots';
import type { DeskView, Slot } from './view';

export interface Pick {
  open: boolean;
  /** The day tab shown (YYYY-MM-DD). */
  day: string;
  /** The picked slot id. */
  slot: string;
  note: string;
  /** K10 and the other refusals, in the page's words. */
  notice: { kind: 'taken'; time: string } | { kind: 'say'; text: string } | null;
  busy: boolean;
  /** The opener's data-k, so focus goes back to it. */
  from: string;
  /** Only the first paint after opening slides in. */
  fresh: boolean;
}

export const closedPick = (): Pick => ({ open: false, day: '', slot: '', note: '', notice: null, busy: false, from: '', fresh: false });

export interface PickerDo {
  close: () => void;
  day: (key: string) => void;
  pick: (slot: string) => void;
  note: (text: string) => void;
  ask: () => void;
  write: () => void;
}

const chars = (s: string): number => [...s].length;

/** The day to show: the picked slot's, else the one asked for if it is still there, else the first with an open time. */
export function dayFor(days: Day[], p: Pick, slots: Slot[]): string {
  const picked = slots.find((s) => s.id === p.slot);
  if (picked && days.some((d) => d.key === picked.start.slice(0, 10))) return picked.start.slice(0, 10);
  if (p.day && days.some((d) => d.key === p.day)) return p.day;
  return (days.find((d) => d.open.length) ?? days[0])?.key ?? '';
}

/** Arrow keys inside a tablist or radiogroup: move focus and pick, like native radios. */
function arrows(group: HTMLElement, sel: string, cols: number, on: (el: HTMLElement) => void): void {
  group.addEventListener('keydown', (e) => {
    const items = [...group.querySelectorAll<HTMLElement>(sel)];
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (i < 0) return;
    const step: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols };
    let j = -1;
    if (e.key in step) j = Math.min(items.length - 1, Math.max(0, i + step[e.key]));
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = items.length - 1;
    if (j < 0) return;
    e.preventDefault();
    if (j !== i) on(items[j]);
  });
}

function bold(line: Line): Kid[] {
  return [h('b', null, line[0]), line[1] ? ` ${line[1]}` : null];
}

/** The scrim and the sheet, or nothing while closed. */
export function picker(v: DeskView, p: Pick, n: Names, now: number, act: PickerDo): HTMLElement[] {
  if (!p.open) return [];
  const days = groupDays(v.slots, now);
  const anyOpen = openSlots(v.slots, now).length > 0;
  const scrim = h('div', { class: `scrim${p.fresh ? ' new' : ''}`, 'aria-hidden': 'true', 'data-scrim': '' });
  scrim.addEventListener('click', act.close);
  const close = h('button', { type: 'button', class: 'close', 'aria-label': 'Close', 'data-k': 'pk-close', 'data-pick-close': '' }, dIcon('x'));
  close.addEventListener('click', act.close);
  const head = h('div', { class: 'sh' }, h('h2', { id: 'pk-title', tabindex: -1, 'data-k': 'pk-title' }, book.sheet_title), close);
  let inner: Kid[];
  if (!anyOpen) {
    // K8: nothing open. Write to the host closes the sheet and puts the cursor in the composer.
    const write = h('button', { type: 'button', class: 'gbtn ghost', 'data-k': 'pk-write', 'data-pick-write': '' }, dIcon('chat'), book.none_button(n));
    write.addEventListener('click', act.write);
    inner = [h('div', { class: 'gempty', 'data-pick-none': '' }, dIcon('cal', 'pi big'), h('b', null, book.none_title), h('span', null, book.none_body(n))), write];
  } else {
    const key = dayFor(days, p, v.slots);
    const d = days.find((x) => x.key === key)!;
    const tabs = h(
      'div',
      { class: 'gdays', role: 'tablist', 'aria-label': 'Days' },
      days.map((x) => {
        const on = x.key === key;
        const t = h(
          'button',
          {
            type: 'button',
            role: 'tab',
            id: `pk-day-${x.key}`,
            'aria-selected': on ? 'true' : 'false',
            'aria-controls': 'pk-times',
            tabindex: on ? 0 : -1,
            class: `${on ? 'on' : ''}${x.open.length ? '' : ' none'}`.trim() || null,
            'data-k': `pk-day-${x.key}`,
            'data-day': x.key,
          },
          h('span', null, x.weekday),
          h('b', null, x.date),
          h('em', null, x.open.length ? book.day_open(x.open.length) : book.day_full),
        );
        t.addEventListener('click', () => act.day(x.key));
        return t;
      }),
    );
    arrows(tabs, '[role=tab]', 1, (el) => act.day(el.getAttribute('data-day')!));
    const m = dayMinutes(d);
    const line = h('p', { class: 'dayline', 'data-dayline': '' }, book.day_line(d.long, m ? lengthLabel(m) : undefined));
    const picked = d.open.find((s) => s.id === p.slot);
    const times = h(
      'div',
      { class: 'gtimes', role: 'radiogroup', id: 'pk-times', 'aria-label': `Open times, ${d.long}` },
      d.open.map((s, i) => {
        const on = s.id === p.slot;
        const b = h(
          'button',
          {
            type: 'button',
            role: 'radio',
            'aria-checked': on ? 'true' : 'false',
            // Roving focus: the picked time, else the first, is the one Tab reaches.
            tabindex: on || (!picked && i === 0) ? 0 : -1,
            class: on ? 'on' : null,
            'data-k': `pk-t-${s.id}`,
            'data-slot': s.id,
          },
          timeLabel(s),
        );
        b.addEventListener('click', () => act.pick(s.id));
        return b;
      }),
    );
    arrows(times, '[role=radio]', 3, (el) => act.pick(el.getAttribute('data-slot')!));
    if (!d.open.length) times.append(h('p', { class: 'soft dayfull' }, book.day_full));
    const notice = p.notice
      ? p.notice.kind === 'taken'
        ? h('div', { class: 'gnote warn', role: 'alert', 'data-taken': '' }, dIcon('warn'), h('span', null, bold(book.taken(p.notice.time))))
        : h('div', { class: 'gnote warn', role: 'alert', 'data-pick-say': '' }, dIcon('warn'), h('span', null, p.notice.text))
      : null;
    const max = v.limits.note_max_chars;
    const over = chars(p.note) > max;
    const note = h('textarea', {
      id: 'pk-note',
      'data-k': 'pk-note',
      class: `in${over ? ' over' : ''}`,
      rows: 2,
      'aria-describedby': over ? 'pk-note-hint' : null,
      'aria-invalid': over ? 'true' : 'false',
      autocomplete: 'off',
    }) as HTMLTextAreaElement;
    note.value = p.note;
    const hint = h('p', { class: 'hint over', id: 'pk-note-hint', 'aria-live': 'polite', hidden: !over }, book.note_long(max), ` ${fmt(chars(p.note))} / ${fmt(max)}`);
    const ask = h(
      'button',
      { type: 'button', class: `gbtn${picked ? '' : ' off'}`, 'data-k': 'pk-ask', 'data-ask': '', 'aria-busy': p.busy ? 'true' : null },
      picked ? book.ask_button(whenLabel(picked)) : book.ask_button_off,
    ) as HTMLButtonElement;
    const ready = () => {
      const long = chars(p.note) > max;
      ask.disabled = p.busy || !picked || long;
      note.classList.toggle('over', long);
      note.setAttribute('aria-invalid', long ? 'true' : 'false');
      if (long) note.setAttribute('aria-describedby', 'pk-note-hint');
      else note.removeAttribute('aria-describedby');
      hint.hidden = !long;
      hint.replaceChildren(book.note_long(max), ` ${fmt(chars(p.note))} / ${fmt(max)}`);
    };
    note.addEventListener('input', () => {
      act.note(note.value);
      ready();
    });
    ask.addEventListener('click', act.ask);
    ready();
    inner = [
      tabs,
      line,
      notice,
      times,
      h('div', { class: 'gnotefield' }, h('label', { class: 'lbl', for: 'pk-note' }, book.note_label(n)), note, hint),
      ask,
      h('p', { class: 'fine' }, dIcon('person'), h('span', null, book.sheet_fine(n))),
    ];
  }
  const sheet = h(
    'div',
    { class: `sheet${p.fresh ? ' new' : ''}`, role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'pk-title', 'data-sheet': '' },
    h('span', { class: 'grab', 'aria-hidden': 'true' }),
    head,
    ...inner,
  );
  // Esc closes; Tab stays inside the sheet (the chat behind is inert while it is open).
  sheet.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      act.close();
      return;
    }
    if (e.key !== 'Tab') return;
    const f = [...sheet.querySelectorAll<HTMLElement>('button:not([disabled]):not([tabindex="-1"]), textarea, [tabindex="0"]')];
    if (!f.length) return;
    const first = f[0];
    const last = f[f.length - 1];
    const at = document.activeElement;
    if (e.shiftKey && (at === first || at === sheet.querySelector('#pk-title'))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && at === last) {
      e.preventDefault();
      first.focus();
    }
  });
  return [scrim, sheet];
}
