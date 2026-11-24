// Pocket apps (pa13b): the Booking Desk's React parts (pa13/pa13b-design.md §1, §3, "For Pixel"): the week of times
// (SlotWeek), QuickFill, the booking-request Allow card (BookingAllow), the odd-request card (OddAllow), the host's
// "Book a time for Ben" picker (BookFor) and the booking Needs-you row. PocketDesk.tsx draws the card frame and
// branches on spec.mode; guide mode never reaches this file.
// No holo: "Booked" is a person's yes, a plain ok chip. Requests, hand-offs, refusals and errors stay plain.
// Only Tailwind classes and --kw-* tokens; never a hex. No pronouns for a guest or the host: the name repeats.

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { AlertTriangle, BellRing, CalendarClock, CalendarPlus, Check, ChevronLeft, ChevronRight, Clock, LayoutGrid, List, Lock, Minus, Pencil, Plus, Reply, Send, Undo2, X, Zap } from 'lucide-react';
import type { DeskBooking, DeskInfo, DeskSlot } from '../../smartResults';
import { ModernAvatar } from '../ModernAvatar';
import { pocketApi } from '../../pocketApi';
import { parseSpacePath, spacePath } from '../../spaces';
import { GuestCreature } from './PocketSanta';
import { HALF, PRIMARY } from './pocketParts';
import { AllowCard, DESK_FAILED, DESK_SORRY, deskAnswers, deskKey, isAnswered, shortWhen, useDeskAnswers, validDesk, type DeskItem } from './PocketDesk';
import {
  addDays,
  applyFill,
  bookingClock,
  bookingKey,
  DOW_LABEL,
  fillWeek,
  holdingOf,
  inUse,
  isOpen,
  lengthLabel,
  MAX_SEATS,
  MAX_SLOTS,
  monthDay,
  nextSlotFor,
  nowKey,
  requestsOf,
  slotDay,
  slotTime,
  slotWhen,
  sortSlots,
  timesLine,
  todayKey,
  weekday,
  weekDays,
  weekOf,
  weekRange,
  type BookingRequest,
  type FillRule,
} from './deskBooking';

// Kept local (not imported from PocketDesk) so this module reads nothing from PocketDesk while the two load.
const SMALL = 'text-[13px] leading-5 text-zinc-400 sm:text-2xs';
const HALF_BTN = `kw-btn-ghost ${HALF}`;
const OK_BTN = `kw-btn-ghost ${HALF} justify-center border-teal-400/40 text-ok`;
const QUIET = 'min-h-[44px] rounded-xl px-3 text-[14px] font-medium text-zinc-300 hover:bg-white/[0.04] kw-focus sm:min-h-[36px] sm:text-meta';
const FIELD = 'min-h-[44px] rounded-lg border border-zinc-700 bg-surface-raised px-2.5 text-[15px] font-semibold text-zinc-50 tabular-nums kw-focus sm:h-8 sm:min-h-0 sm:text-[13px]';
const BOX = 'min-h-[44px] w-full resize-y rounded-[10px] border border-sky-400 bg-surface-main px-3 py-2.5 text-[15px] leading-5 text-zinc-50 placeholder:text-zinc-500 kw-focus sm:text-[13.5px]';
export const LENGTHS = [15, 20, 30, 45, 60, 90, 120];

const stamp = (s: string | null | undefined) => {
  if (!s) return null;
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
};
/** "1 hour", "6 min", "2 days": how long ago, for "asked 11:32 AM · 1 hour". */
function ago(at: string | null): string | null {
  const t = stamp(at);
  if (t === null) return null;
  const m = Math.max(0, Math.round((bookingClock.now().getTime() - t) / 60_000));
  if (m < 1) return 'just now';
  if (m < 60) return `${m} min`;
  const h = Math.round(m / 60);
  if (h < 24) return h === 1 ? '1 hour' : `${h} hours`;
  const d = Math.round(h / 24);
  return d === 1 ? '1 day' : `${d} days`;
}

// ---------------------------------------------------------------------------------------------------
// Features the server may not have yet (design "Data the app doesn't have yet")

let bookOff = false;
const BOOK_MISSING = new Set(['bad_action', 'unknown_action', 'no_such_action', 'not_supported']);
export const bookingFeatures = {
  /** False once the server said it has no `book` action: "Book a time for Ben" hides. */
  canBook: () => !bookOff,
  /** Tests and fixtures. */
  setBook(on: boolean) {
    bookOff = !on;
  },
};

/** Opens the Inbox (Needs you) in this space, the way Shell's Back/Forward do. */
export function openInbox() {
  if (typeof window === 'undefined') return;
  const route = parseSpacePath(window.location.pathname);
  if (!route.spaceId) return;
  window.history.pushState(null, '', spacePath(route.spaceId, null, { kind: 'inbox', tab: 'new' }));
  window.dispatchEvent(new PopStateEvent('popstate'));
}

// ---------------------------------------------------------------------------------------------------
// Words

export const bookingKeepLine = 'Chats stay on this Mac. A guest’s messages are deleted 7 days after their last one, or the day after their booking if that’s later.';
export const bookingNeverLine = (bot: string) => `${bot} never books anyone. It holds the time a guest picks; only your Yes books it. No payments here.`;
export const bookingFootLine = (bot: string, cap: number) =>
  `${bot} writes one reply per guest message (a model call), up to ${new Intl.NumberFormat('en').format(cap)} a day. Picking a time, your Yes and the guest page use none.`;
export const BOOKING_CAP_LINE = 'Booking still works: guests can pick a time and you can say yes, with no model call.';
/** The refusal line for a time in use (a Lock tap or a 409 slot_in_use). */
export function refusalLine(start: string, slot: DeskSlot | null, name?: string | null): { head: string; rest: string } {
  const when = `${weekday(start.slice(0, 10))} ${slotTime(start)}`;
  const req = slot?.requested[0];
  const conf = slot?.confirmed[0];
  if (req || (!conf && name)) {
    const who = req?.name ?? name ?? 'A guest';
    return { head: `${when} stays: ${who} asked for it.`, rest: `Answer ${who}’s request first. A time someone asked for or booked has a lock; to free it, say Sorry to the request or cancel the booking.` };
  }
  const who = conf?.name ?? name ?? null;
  return {
    head: who ? `${when} is booked by ${who}.` : `${when} is booked.`,
    rest: 'A time someone asked for or booked has a lock; to free it, say Sorry to the request or cancel the booking.',
  };
}

// ---------------------------------------------------------------------------------------------------
// One time: a chip (design "Chip sizes"; booking.css .sl)

const CHIP = 'relative flex min-h-[44px] min-w-0 items-center gap-1.5 rounded-lg border px-2 text-[14px] font-semibold tabular-nums transition-shadow duration-120 sm:min-h-[32px] sm:text-[13px]';
const TONE = {
  open: 'border-hairline-strong bg-surface-main text-zinc-100',
  past: 'border-transparent bg-transparent font-medium text-zinc-500',
  req: 'border-dashed border-sky-400 bg-sky-500/[0.14] text-zinc-100',
  ok: 'border-[color-mix(in_srgb,var(--kw-ok)_45%,transparent)] bg-[color-mix(in_srgb,var(--kw-ok)_8%,var(--kw-surface-main))] text-zinc-100',
};
const REFUSE = 'border-[color-mix(in_srgb,var(--kw-warn)_70%,transparent)] shadow-[0_0_0_3px_color-mix(in_srgb,var(--kw-warn)_18%,transparent)]';
const CTRL = '-mr-2 ml-auto inline-grid h-11 w-11 shrink-0 place-items-center rounded-md text-zinc-400 hover:bg-white/[0.06] hover:text-zinc-100 kw-focus sm:mr-0 sm:h-5 sm:w-5';

export type SlotState = 'open' | 'past' | 'req' | 'ok';
export function slotState(s: DeskSlot, now = nowKey()): SlotState {
  if (s.start <= now) return 'past';
  if (s.requested.length) return 'req';
  if (s.confirmed.length) return 'ok';
  return 'open';
}

export const SlotChip: React.FC<{
  slot: DeskSlot;
  editing?: boolean;
  refused?: boolean;
  showDay?: boolean;
  onRemove?: (s: DeskSlot) => void;
  onLocked?: (s: DeskSlot) => void;
}> = ({ slot, editing = false, refused = false, showDay = false, onRemove, onLocked }) => {
  const now = nowKey();
  const st = slotState(slot, now);
  const people = [...slot.requested, ...slot.confirmed];
  const first = people[0];
  const name = first ? first.name ?? 'chat deleted' : null;
  const more = people.length > 1 ? ` +${people.length - 1}` : '';
  const left = slot.seats - people.length;
  const time = slotTime(slot.start);
  const label = [
    showDay ? slotWhen(slot.start) : time,
    st === 'past' ? 'past' : st === 'req' ? `requested by ${first?.name ?? 'a guest'}` : st === 'ok' ? `booked${first?.name ? ` by ${first.name}` : ''}` : 'open',
    slot.seats > 1 ? `${left} of ${slot.seats} left` : null,
  ]
    .filter(Boolean)
    .join(', ');
  return (
    <div className={`${CHIP} ${TONE[st]} ${refused ? REFUSE : ''}`} data-slot={slot.start} data-slot-state={st} data-refused={refused || undefined} role="listitem" aria-label={label}>
      {st === 'req' && !editing && <BellRing size={13} className="shrink-0 text-sky-200 sm:hidden" aria-hidden="true" />}
      <span className="shrink-0">{showDay ? `${slotDay(slot.start)}, ${time}` : time}</span>
      {name ? (
        <span className={`min-w-0 flex-1 truncate font-medium ${st === 'past' ? 'text-zinc-500' : 'text-zinc-200'} ${editing ? 'hidden sm:inline' : ''}`}>
          {name}
          {more}
        </span>
      ) : null}
      {name && editing ? <span className="flex-1 sm:hidden" /> : null}
      {name ? null : (
        <span className="flex-1" />
      )}
      {slot.seats > 1 && st !== 'past' && !editing && <span className="ml-auto shrink-0 text-[11.5px] font-semibold text-zinc-400">{left} left</span>}
      {editing && st !== 'past' ? (
        inUse(slot) ? (
          <button type="button" className={CTRL} onClick={() => onLocked?.(slot)} aria-label={`${time} is in use: why it can’t go`} data-action="locked">
            <Lock size={13} aria-hidden="true" />
          </button>
        ) : (
          <button type="button" className={CTRL} onClick={() => onRemove?.(slot)} aria-label={`Take out ${slotWhen(slot.start)}`} data-action="remove">
            <X size={13} aria-hidden="true" />
          </button>
        )
      ) : null}
    </div>
  );
};

const AddChip: React.FC<{ day: string; disabled: boolean; onAdd: (day: string) => void }> = ({ day, disabled, onAdd }) => (
  <button
    type="button"
    onClick={() => onAdd(day)}
    disabled={disabled}
    title={disabled ? `${MAX_SLOTS} times is the most for one desk.` : undefined}
    className={`${CHIP} justify-center border-dashed border-hairline-strong bg-transparent text-sky-300 hover:bg-white/[0.03] kw-focus disabled:cursor-not-allowed disabled:opacity-50`}
    aria-label={`Add a time on ${slotDay(day)}`}
    data-action="add-time"
  >
    <Plus size={13} aria-hidden="true" />
    Add
  </button>
);

// ---------------------------------------------------------------------------------------------------
// The week (SlotWeek): columns at sm+, day tabs + a 2-column list on a phone; or one list with dates.

export interface WeekProps {
  slots: DeskSlot[];
  monday: string;
  view: 'week' | 'list';
  editing?: boolean;
  /** The start of a chip in the warn ring. */
  refused?: string | null;
  /** Days to show even without times (QuickFill's days while setting up). */
  extraDays?: number[];
  onRemove?: (s: DeskSlot) => void;
  onLocked?: (s: DeskSlot) => void;
  onAdd?: (day: string) => void;
}

export function shownDays(slots: DeskSlot[], monday: string, extra: number[] = []): string[] {
  const days = weekDays(monday);
  const has = new Set(slots.map((s) => s.start.slice(0, 10)));
  const today = todayKey();
  // A past day with no times has nothing to show or add, so it drops out (Dec 1 – 5, not Nov 30).
  const pick = days.filter((d, i) => has.has(d) || (extra.includes(i) && d >= today));
  return pick.length ? pick : days.slice(0, 5);
}

export const SlotWeek: React.FC<WeekProps> = ({ slots, monday, view, editing = false, refused = null, extraDays = [], onRemove, onLocked, onAdd }) => {
  const days = shownDays(slots, monday, editing ? extraDays : []);
  const today = todayKey();
  const byDay = (day: string) => slots.filter((s) => s.start.slice(0, 10) === day);
  const full = slots.length >= MAX_SLOTS;
  const canAdd = (day: string) => editing && !!onAdd && day >= today;
  const [tab, setTab] = useState<string>(() => (days.includes(today) ? today : days[0]));
  const shownTab = days.includes(tab) ? tab : days[0];
  const chip = (s: DeskSlot, showDay = false) => <SlotChip key={s.start} slot={s} editing={editing} refused={refused === s.start} onRemove={onRemove} onLocked={onLocked} showDay={showDay} />;

  if (view === 'list') {
    const listDays = days.filter((d) => byDay(d).length || canAdd(d));
    return (
      <div className="flex flex-col gap-3" data-week="list">
        {listDays.map((day) => (
          <section key={day} className="flex flex-col gap-1.5" aria-label={slotDay(day)}>
            <h5 className="text-[13px] font-bold text-zinc-200 sm:text-2xs">
              {slotDay(day)}
              {day === today ? <span className="font-medium text-zinc-400"> · today</span> : null}
            </h5>
            <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-4" role="list">
              {byDay(day).map((s) => chip(s))}
              {canAdd(day) && <AddChip day={day} disabled={full} onAdd={onAdd!} />}
            </div>
          </section>
        ))}
        {listDays.length === 0 && <p className={SMALL}>No times this week.</p>}
      </div>
    );
  }

  return (
    <div data-week="week">
      {/* sm+: one column per day */}
      <div className="hidden gap-1.5 sm:grid" style={{ gridTemplateColumns: `repeat(${days.length}, minmax(0, 1fr))` }}>
        {days.map((day) => (
          <section key={day} className="flex min-w-0 flex-col gap-1.5" aria-label={slotDay(day)} data-day={day}>
            <h5 className="border-b border-hairline pb-1">
              <span className="block text-[13px] font-bold text-zinc-100">{weekday(day)}</span>
              <span className="block text-2xs text-zinc-400">
                {monthDay(day)}
                {day === today ? <b className="font-semibold text-zinc-200"> · today</b> : null}
              </span>
            </h5>
            <div className="flex flex-col gap-1.5" role="list">
              {byDay(day).map((s) => chip(s))}
            </div>
            {canAdd(day) && <AddChip day={day} disabled={full} onAdd={onAdd!} />}
          </section>
        ))}
      </div>
      {/* phone: day tabs + a 2-column list */}
      <div className="flex flex-col gap-2.5 sm:hidden">
        <div className="grid gap-1" style={{ gridTemplateColumns: `repeat(${days.length}, minmax(0, 1fr))` }} role="tablist" aria-label="Days">
          {days.map((day) => {
            const on = day === shownTab;
            const asks = byDay(day).some((s) => s.requested.length > 0);
            return (
              <button
                key={day}
                type="button"
                role="tab"
                aria-selected={on}
                onClick={() => setTab(day)}
                className={`grid min-h-[52px] min-w-0 justify-items-center rounded-[10px] border py-1.5 text-[11.5px] font-semibold kw-focus ${on ? 'border-sky-500 bg-sky-800 text-sky-100' : 'border-hairline-strong bg-surface-main text-zinc-400'}`}
                data-tab={day}
                aria-label={`${slotDay(day)}${asks ? ', has a request' : ''}`}
              >
                <span>{weekday(day)}</span>
                <b className={`text-[17px] leading-[22px] tabular-nums ${on ? 'text-white' : 'text-zinc-100'}`}>{Number(day.slice(8))}</b>
                {asks ? <i className="mt-0.5 block h-[5px] w-[5px] rounded-sm bg-sky-300" aria-hidden="true" /> : <i className="mt-0.5 block h-[5px]" aria-hidden="true" />}
              </button>
            );
          })}
        </div>
        <div className="grid grid-cols-2 gap-2" role="list" aria-label={slotDay(shownTab)} data-day-list={shownTab}>
          {byDay(shownTab).map((s) => chip(s))}
          {canAdd(shownTab) && <AddChip day={shownTab} disabled={full} onAdd={onAdd!} />}
        </div>
        {byDay(shownTab).length === 0 && !canAdd(shownTab) && <p className={SMALL}>No times on {slotDay(shownTab)}.</p>}
      </div>
    </div>
  );
};

/** Week / List toggle and ‹ Dec 1 – 5 › (B1, B7). */
export const WeekBar: React.FC<{
  view: 'week' | 'list';
  setView: (v: 'week' | 'list') => void;
  monday: string;
  setMonday: (m: string) => void;
  days: string[];
}> = ({ view, setView, monday, setMonday, days }) => {
  const thisWeek = weekOf(todayKey());
  const seg = (v: 'week' | 'list', icon: React.ReactNode, label: string) => (
    <button
      type="button"
      onClick={() => setView(v)}
      aria-pressed={view === v}
      className={`inline-flex min-h-[44px] items-center gap-1.5 rounded-lg px-3 text-[14px] font-semibold kw-focus sm:min-h-[30px] sm:text-meta ${view === v ? 'bg-sky-800 text-white' : 'text-zinc-300 hover:bg-white/[0.04]'}`}
      data-view={v}
    >
      {icon}
      {label}
    </button>
  );
  const nav = 'inline-grid h-11 w-11 place-items-center rounded-lg text-zinc-300 hover:bg-white/[0.05] disabled:cursor-not-allowed disabled:text-zinc-600 kw-focus sm:h-8 sm:w-8';
  return (
    <div className="flex flex-wrap items-center justify-between gap-2">
      <div className="inline-flex gap-1 rounded-[10px] border border-hairline-strong bg-surface-main p-1" role="group" aria-label="Show times as">
        {seg('week', <LayoutGrid size={14} aria-hidden="true" />, 'Week')}
        {seg('list', <List size={14} aria-hidden="true" />, 'List')}
      </div>
      <WeekNav monday={monday} setMonday={setMonday} days={days} thisWeek={thisWeek} cls={nav} />
    </div>
  );
};

const WeekNav: React.FC<{ monday: string; setMonday: (m: string) => void; days: string[]; thisWeek: string; cls: string }> = ({ monday, setMonday, days, thisWeek, cls }) => (
  <div className="inline-flex items-center gap-1" role="group" aria-label="Week">
    <button type="button" className={cls} onClick={() => setMonday(addDays(monday, -7))} disabled={monday <= thisWeek} aria-label="Week before" data-action="week-prev">
      <ChevronLeft size={16} aria-hidden="true" />
    </button>
    <span className="min-w-[92px] text-center text-[14px] font-semibold text-zinc-100 tabular-nums sm:text-meta" aria-live="polite" data-week-range>
      {weekRange(days)}
    </span>
    <button type="button" className={cls} onClick={() => setMonday(addDays(monday, 7))} aria-label="Next week" data-action="week-next">
      <ChevronRight size={16} aria-hidden="true" />
    </button>
  </div>
);

/** The read-only week's head on a live desk (B2, B3): "This week" · names line, with ‹ › to look ahead. */
export const WeekHead: React.FC<{ monday: string; setMonday: (m: string) => void; days: string[] }> = ({ monday, setMonday, days }) => {
  const thisWeek = weekOf(todayKey());
  return (
    <div className="flex items-center justify-between gap-2 rounded-lg bg-surface-rail/60 px-2 py-1 text-2xs font-bold text-zinc-300">
      <span className="inline-flex items-center gap-1">
        {monday === thisWeek ? 'This week' : 'Week of'}
        <WeekNav monday={monday} setMonday={setMonday} days={days} thisWeek={thisWeek} cls="inline-grid h-11 w-11 place-items-center rounded-lg text-zinc-300 hover:bg-white/[0.05] disabled:cursor-not-allowed disabled:text-zinc-600 kw-focus sm:h-7 sm:w-7" />
      </span>
      <span className="hidden font-medium text-zinc-400 sm:inline">names: only you see them</span>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// QuickFill (B1): days, hours, length, guests per time → "Fill these days" makes the shown week's times.

export const QuickFill: React.FC<{ rule: FillRule; setRule: (r: FillRule) => void; monday: string; count: number; onFill: () => void; note?: string | null }> = ({
  rule,
  setRule,
  monday,
  count,
  onFill,
  note,
}) => {
  const makes = fillWeek(rule, monday).length;
  const full = count >= MAX_SLOTS;
  const K = 'w-full shrink-0 text-[13px] font-semibold text-zinc-300 sm:w-16 sm:text-[12.5px]';
  const R = 'flex flex-wrap items-center gap-2 text-[14px] text-zinc-300 sm:text-[13px]';
  const toggle = (i: number) => setRule({ ...rule, days: rule.days.includes(i) ? rule.days.filter((x) => x !== i) : [...rule.days, i].sort() });
  const seats = (n: number) => setRule({ ...rule, seats: Math.min(MAX_SEATS, Math.max(1, n)) });
  const SIDE = 'flex h-full w-11 items-center justify-center text-zinc-200 hover:bg-white/[0.05] disabled:cursor-not-allowed disabled:text-zinc-600 kw-focus sm:w-9';
  return (
    <div className="grid gap-2.5 rounded-xl border border-hairline-strong bg-surface-main p-3" data-quickfill>
      <div className={R}>
        <span className={K} id="qf-days">
          Days
        </span>
        <div className="flex flex-wrap gap-1" role="group" aria-labelledby="qf-days">
          {DOW_LABEL.map((d, i) => (
            <button
              key={d}
              type="button"
              aria-pressed={rule.days.includes(i)}
              onClick={() => toggle(i)}
              className={`inline-grid h-11 min-w-[44px] place-items-center rounded-lg border px-1.5 text-[13px] font-semibold kw-focus sm:h-8 sm:min-w-[38px] sm:text-[12.5px] ${
                rule.days.includes(i) ? 'border-sky-600 bg-sky-800 text-white' : 'border-hairline-strong text-zinc-400 hover:bg-white/[0.03]'
              }`}
              data-day-toggle={i}
            >
              {d}
            </button>
          ))}
        </div>
      </div>
      <div className={R}>
        <span className={K}>Hours</span>
        <label className="sr-only" htmlFor="qf-from">
          From
        </label>
        <input id="qf-from" type="time" step={900} value={rule.from} onChange={(e) => e.target.value && setRule({ ...rule, from: e.target.value })} className={FIELD} data-field="from" />
        <span>to</span>
        <label className="sr-only" htmlFor="qf-to">
          To
        </label>
        <input id="qf-to" type="time" step={900} value={rule.to} onChange={(e) => e.target.value && setRule({ ...rule, to: e.target.value })} className={FIELD} data-field="to" />
        <label htmlFor="qf-len">each</label>
        <select id="qf-len" value={rule.minutes} onChange={(e) => setRule({ ...rule, minutes: Number(e.target.value) })} className={FIELD} data-field="length">
          {LENGTHS.map((m) => (
            <option key={m} value={m}>
              {lengthLabel(m)}
            </option>
          ))}
        </select>
      </div>
      <div className={R}>
        <span className={K}>Per time</span>
        <div className="inline-grid h-[46px] shrink-0 grid-cols-[auto_64px_auto] items-center overflow-hidden rounded-[10px] border border-hairline-strong sm:h-9" role="group" aria-label="Guests per time" data-stepper="seats">
          <button type="button" className={SIDE} aria-label={`Fewer: ${rule.seats - 1}`} disabled={rule.seats <= 1} onClick={() => seats(rule.seats - 1)}>
            <Minus size={15} aria-hidden="true" />
          </button>
          <output className="border-x border-hairline text-center text-[15px] font-semibold leading-[44px] text-zinc-50 tabular-nums sm:leading-[34px]" aria-live="polite">
            {rule.seats}
          </output>
          <button type="button" className={SIDE} aria-label={`More: ${rule.seats + 1}`} disabled={rule.seats >= MAX_SEATS} onClick={() => seats(rule.seats + 1)}>
            <Plus size={15} aria-hidden="true" />
          </button>
        </div>
        <span>guests (more for a group class)</span>
      </div>
      <p className="text-[13px] text-zinc-400 sm:text-[12.5px]" aria-live="polite" data-makes={makes}>
        {makes ? (
          <>
            Makes <b className="font-semibold text-zinc-100">{makes} times</b>. Tap a time to take it out, like lunch or Saturday afternoon.
          </>
        ) : (
          'Pick at least one day and hours long enough for one time.'
        )}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={onFill} disabled={!makes || full} className={HALF_BTN} data-action="fill">
          <CalendarPlus size={14} aria-hidden="true" />
          Fill these days
        </button>
        {full ? <span className={SMALL}>{MAX_SLOTS} times is the most for one desk.</span> : note ? <span className={SMALL}>{note}</span> : null}
      </div>
    </div>
  );
};

/** The refusal line above the week (B7): a Lock tap or a 409 slot_in_use. */
export const Refusal: React.FC<{ head: string; rest: string }> = ({ head, rest }) => (
  <p
    role="alert"
    className="flex items-start gap-2 rounded-[10px] border border-[color-mix(in_srgb,var(--kw-warn)_40%,transparent)] bg-[color-mix(in_srgb,var(--kw-warn)_8%,var(--kw-surface-main))] px-3 py-2.5 text-[15px] leading-[22px] text-zinc-100 sm:text-[13px] sm:leading-[19px]"
    data-refusal
  >
    <AlertTriangle size={15} className="mt-0.5 shrink-0 text-warn" aria-hidden="true" />
    <span>
      <b className="font-semibold text-zinc-50">{head}</b> {rest}
    </span>
  </p>
);

/**
 * The editor both set-up (B1) and Edit times (B7) use: QuickFill (set-up, or an empty week), the bar, the refusal,
 * the week with × and Lock. The draft lives in the card; this only changes it.
 */
export const TimesEditor: React.FC<{
  draft: DeskSlot[];
  setDraft: (s: DeskSlot[]) => void;
  monday: string;
  setMonday: (m: string) => void;
  rule: FillRule;
  setRule: (r: FillRule) => void;
  quickFill: boolean;
  refusal: { start: string; head: string; rest: string } | null;
  setRefusal: (r: { start: string; head: string; rest: string } | null) => void;
}> = ({ draft, setDraft, monday, setMonday, rule, setRule, quickFill, refusal, setRefusal }) => {
  const [view, setView] = useState<'week' | 'list'>('week');
  const [note, setNote] = useState<string | null>(null);
  const days = shownDays(draft, monday, rule.days);
  const inWeek = draft.filter((s) => s.start.slice(0, 10) >= monday && s.start.slice(0, 10) <= addDays(monday, 6));
  const showFill = quickFill || inWeek.length === 0;
  const fill = () => {
    const r = applyFill(draft, rule, monday);
    setDraft(r.slots);
    setRefusal(null);
    setNote(r.capped ? `${MAX_SLOTS} times is the most for one desk.` : `Made ${r.made} times.`);
  };
  const remove = (s: DeskSlot) => {
    setRefusal(null);
    setDraft(draft.filter((x) => x.start !== s.start));
  };
  const add = (day: string) => {
    const n = nextSlotFor(draft, day, rule.minutes);
    if (!n) {
      setNote(draft.length >= MAX_SLOTS ? `${MAX_SLOTS} times is the most for one desk.` : `${slotDay(day)} is full.`);
      return;
    }
    setDraft(sortSlots([...draft, { ...n, seats: rule.seats }]));
  };
  const locked = (s: DeskSlot) => setRefusal({ start: s.start, ...refusalLine(s.start, s) });
  return (
    <div className="flex flex-col gap-3" data-times-editor>
      <div className="flex items-center justify-between gap-2 text-2xs font-bold text-zinc-300">
        <span>Times</span>
        <small className="font-medium text-zinc-400" data-times-line>
          {timesLine(draft)}
        </small>
      </div>
      <WeekBar view={view} setView={setView} monday={monday} setMonday={setMonday} days={days} />
      {showFill && <QuickFill rule={rule} setRule={setRule} monday={monday} count={draft.length} onFill={fill} note={note} />}
      {refusal && <Refusal head={refusal.head} rest={refusal.rest} />}
      <SlotWeek slots={inWeek} monday={monday} view={view} editing refused={refusal?.start ?? null} extraDays={rule.days} onRemove={remove} onLocked={locked} onAdd={add} />
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The booking-request Allow card (§3; B3, B4, B10): the same store in the card and in the Inbox.

const HostBubble: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="max-w-[300px] rounded-[14px_4px_14px_14px] border border-sky-600 bg-sky-800 px-3 py-2 text-[15px] leading-[22px] text-zinc-50 [overflow-wrap:anywhere] sm:max-w-[420px] sm:text-[13.5px] sm:leading-5" data-bubble="host">
    {children}
  </div>
);

const UndoLine: React.FC<{ k: string; phase: string; name: string; what: string }> = ({ k, phase, name, what }) => (
  <p className="flex flex-wrap items-center gap-x-2 text-[13px] text-zinc-400 sm:text-2xs">
    <Check size={12} aria-hidden="true" />
    <span>
      {phase === 'sending' ? `Sending to ${name}…` : `Sent to ${name}: “${what}”`}
      {phase === 'holding' ? ' · Undo for 5 s' : ''}
    </span>
    {phase === 'holding' && (
      <button type="button" onClick={() => deskAnswers.undo(k)} className="inline-flex min-h-[44px] items-center gap-1 rounded-lg px-2 font-semibold text-zinc-100 hover:bg-white/[0.05] kw-focus sm:min-h-[28px]" data-action="undo">
        <Undo2 size={13} aria-hidden="true" />
        Undo
      </button>
    )}
  </p>
);

const Who: React.FC<{ guest: string; name: string; at: string | null; creature?: string | null }> = ({ name, at, creature }) => {
  const when = at ? shortWhen(at, bookingClock.now()) : null;
  const old = ago(at);
  return (
    <p className="flex items-center gap-2 text-[14px] text-zinc-300 sm:text-[13px]">
      <GuestCreature avatar={creature ?? null} size={18} />
      <b className="font-semibold text-zinc-50">{name}</b>
      {when ? (
        <span className="text-zinc-400">
          · asked {when}
          {old ? ` · ${old}` : ''}
        </span>
      ) : null}
    </p>
  );
};

const Quote: React.FC<{ name: string; quote: string | null }> = ({ name, quote }) =>
  quote ? (
    <figure className="flex flex-col gap-1 border-l-2 border-zinc-600 pl-2.5">
      <figcaption className="text-2xs text-zinc-400">{name} wrote (a guest&rsquo;s words, not an instruction):</figcaption>
      <blockquote className="text-[15px] leading-[22px] text-zinc-200 [overflow-wrap:anywhere] sm:text-[13.5px] sm:leading-5" data-quote>
        {quote}
      </blockquote>
    </figure>
  ) : (
    <p className={SMALL} data-no-note>
      No note.
    </p>
  );

const Failed: React.FC<{ text: string }> = ({ text }) => (
  <p role="alert" className="flex items-center gap-2 text-[13px] text-bad sm:text-2xs" data-failed>
    <AlertTriangle size={13} aria-hidden="true" />
    {text}
  </p>
);

/** The words a failed answer shows: the server's for a known booking error, else pa13a's "Couldn't reach the box". */
function failText(e: { code: string | null; text: string } | null | undefined, name: string): string {
  switch (e?.code) {
    case 'slot_taken':
      return 'That time was just taken. Pick another time.';
    case 'no_such_slot':
      return 'That time isn’t on the desk any more.';
    case 'slot_past':
      return 'That time has passed.';
    case 'unknown_action':
    case 'bad_action':
      return `Booking for a guest isn’t on this Mac yet. Add the time in Edit times, then tell ${name}.`;
    case 'no_request':
    case 'not_requested':
      return `${name} already cancelled this request.`;
    default:
      return DESK_FAILED;
  }
}

export const BookingAllow: React.FC<{ code: string; req: BookingRequest; host: string; bot: string; creature?: string | null }> = ({ code, req, host, bot, creature }) => {
  useDeskAnswers();
  const { slot, person } = req;
  const name = person.name ?? 'The guest';
  const sid = slot.id ?? slot.start;
  const k = bookingKey(code, person.guest, sid);
  const a = deskAnswers.get(k);
  const reply = deskAnswers.get(deskKey(code, person.guest));
  const [mode, setMode] = useState<'idle' | 'sorry' | 'write'>('idle');
  const [text, setText] = useState('');
  const field = useRef<HTMLTextAreaElement | null>(null);
  const when = slotWhen(slot.start);
  useEffect(() => () => deskAnswers.flush(k), [k]);
  useEffect(() => {
    if (mode !== 'idle') field.current?.focus();
  }, [mode]);

  if (a && a.phase !== 'failed' && (a.kind === 'confirm' || a.kind === 'decline')) {
    const yes = a.kind === 'confirm';
    return (
      <div className="flex flex-col gap-1.5 rounded-xl border border-hairline-strong bg-surface-raised p-3 motion-safe:animate-[kw-fade-in_160ms_var(--kw-ease)]" data-booking-allow={a.phase} data-answer={a.kind} role="status">
        <p className="flex flex-wrap items-center gap-2 text-[15px] text-zinc-100 sm:text-[13.5px]">
          {yes ? (
            <span className="kw-chip kw-chip-ok" data-chip="booked">
              <Check size={12} aria-hidden="true" />
              Booked
            </span>
          ) : (
            <span className="kw-chip" data-chip="freed">
              <X size={12} aria-hidden="true" />
              Time freed
            </span>
          )}
          <span>
            <b className="font-semibold">{name}</b> · {when}
          </span>
        </p>
        {!yes && a.text ? (
          <div className="flex justify-end">
            <HostBubble>{a.text}</HostBubble>
          </div>
        ) : null}
        <UndoLine k={k} phase={a.phase} name={name} what={yes ? (host ? `${host} confirmed your time.` : 'Your time is confirmed.') : host ? `${host} can’t do ${when}.` : `${when} isn’t possible.`} />
      </div>
    );
  }

  const hold = (kind: 'confirm' | 'decline', t: string | null) => {
    deskAnswers.holdAs(k, code, person.guest, { kind, text: t, slot: sid });
    setMode('idle');
    setText('');
  };
  const sendReply = () => {
    const t = text.trim();
    if (!t) return;
    void deskAnswers.now(code, person.guest, 'reply', t);
    setMode('idle');
    setText('');
  };
  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-raised p-3" data-booking-allow="open" role="group" aria-label={`Booking request from ${name}: ${when}`}>
      <h4 className="flex items-center gap-2 text-[15px] font-bold text-zinc-50 sm:text-[13.5px]">
        <BellRing size={16} className="shrink-0 text-zinc-300" aria-hidden="true" />
        Booking request: {when}
      </h4>
      <Who guest={person.guest} name={name} at={person.at} creature={creature} />
      <Quote name={name} quote={person.note} />
      {a?.phase === 'failed' && <Failed text={failText(a.error, name)} />}
      {reply && reply.kind === 'reply' && reply.phase !== 'failed' && (
        <div className="flex flex-col items-end gap-1">
          <HostBubble>{reply.text}</HostBubble>
          <span className="text-2xs text-zinc-400">{reply.phase === 'sent' ? `Sent to ${name}` : `Sending to ${name}…`}</span>
        </div>
      )}
      {mode !== 'idle' ? (
        <div className="flex flex-col gap-2">
          <label className="sr-only" htmlFor={`bk-${k}`}>
            {mode === 'sorry' ? `A line for ${name}` : `Your reply to ${name}`}
          </label>
          <textarea
            id={`bk-${k}`}
            ref={field}
            rows={2}
            maxLength={1000}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setMode('idle');
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                if (mode === 'sorry') hold('decline', text.trim() || null);
                else sendReply();
              }
            }}
            placeholder={mode === 'sorry' ? `Add a line for ${name} (optional)` : `Write to ${name}`}
            className={BOX}
          />
          <div className="flex flex-wrap gap-2">
            {mode === 'sorry' ? (
              <button type="button" onClick={() => hold('decline', text.trim() || null)} className={`${PRIMARY}`} data-action="decline-send">
                <Send size={14} aria-hidden="true" />
                Send and free the time
              </button>
            ) : (
              <button type="button" onClick={sendReply} disabled={!text.trim()} className={`${PRIMARY}`} data-action="send">
                <Send size={14} aria-hidden="true" />
                Send
              </button>
            )}
            <button type="button" onClick={() => setMode('idle')} className={QUIET} data-action="back">
              Back
            </button>
          </div>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap sm:items-center" data-allow-actions>
          <button type="button" onClick={() => hold('confirm', null)} className={OK_BTN} data-action="confirm">
            <Check size={14} aria-hidden="true" />
            Yes, book it
          </button>
          <button type="button" onClick={() => setMode('sorry')} className={`${HALF_BTN} justify-center`} data-action="decline">
            <X size={14} aria-hidden="true" />
            Sorry, can&rsquo;t
          </button>
          <button type="button" onClick={() => setMode('write')} className={`${HALF_BTN} justify-center`} data-action="write">
            <Reply size={14} aria-hidden="true" />
            Write a reply
          </button>
        </div>
      )}
      <p className={SMALL}>
        Yes books {name} and tells them. Sorry frees the time and tells them, with your line if you write one. {bot} never books for you.
      </p>
    </div>
  );
};

/** Requests above the week (B3): oldest first, at most 3, then "N more requests" opens the Inbox. */
export const BookingRequests: React.FC<{ code: string; slots: DeskSlot[]; host: string; bot: string; creatures?: Record<string, string | null>; onMore?: () => void }> = ({
  code,
  slots,
  host,
  bot,
  creatures = {},
  onMore = openInbox,
}) => {
  useDeskAnswers();
  // A sent answer leaves; a held or sending one stays so Undo is in reach.
  const list = requestsOf(slots).filter(({ slot, person }) => deskAnswers.get(bookingKey(code, person.guest, slot.id ?? slot.start))?.phase !== 'sent');
  if (!list.length) return null;
  const shown = list.slice(0, 3);
  const more = list.length - shown.length;
  return (
    <div className="flex flex-col gap-2.5" data-requests={list.length}>
      {shown.map((r) => (
        <BookingAllow key={`${r.person.guest}@${r.slot.start}`} code={code} req={r} host={host} bot={bot} creature={creatures[r.person.guest] ?? null} />
      ))}
      {more > 0 && (
        <button type="button" onClick={onMore} className={`${HALF_BTN} self-start`} data-action="more-requests">
          <BellRing size={14} aria-hidden="true" />
          {more} more {more === 1 ? 'request' : 'requests'}
        </button>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The odd-request card (§3, B5) and "Book a time for Ben" (B6)

export const OddAllow: React.FC<{
  code: string;
  guest: string;
  name: string;
  reason: string;
  quote: string | null;
  bot: string;
  host: string;
  askedAt?: string | null;
  askStart?: string | null;
  /** The desk's times; the Inbox has none and BookFor reads them. */
  slots?: DeskSlot[] | null;
}> = ({ code, guest, name, reason, quote, bot, host, askedAt, askStart, slots }) => {
  useDeskAnswers();
  const k = deskKey(code, guest);
  const a = deskAnswers.get(k);
  const [picking, setPicking] = useState(false);
  const [writing, setWriting] = useState(false);
  const [text, setText] = useState('');
  const answered = isAnswered(a, stamp(askedAt ?? null));
  useEffect(() => () => deskAnswers.flush(k), [k]);
  // `book` came back unknown: the server has no such action yet. Hide the button and say so once.
  const missing = a?.phase === 'failed' && a.kind === 'book' && !!a.error?.code && BOOK_MISSING.has(a.error.code);
  useEffect(() => {
    if (missing) bookingFeatures.setBook(false);
  }, [missing]);

  if (answered && a?.kind === 'book') {
    const start = 'slot' in (a.to ?? {}) ? slots?.find((s) => s.id === (a.to as { slot: string }).slot)?.start ?? null : (a.to as { new_slot: { start: string } } | null)?.new_slot.start ?? null;
    const when = start ? slotWhen(start) : 'the time you picked';
    return (
      <div className="flex flex-col gap-1.5 rounded-xl border border-hairline-strong bg-surface-raised p-3" data-odd-allow={a.phase} data-answer="book" role="status">
        <p className="flex flex-wrap items-center gap-2 text-[15px] text-zinc-100 sm:text-[13.5px]">
          <span className="kw-chip kw-chip-ok" data-chip="booked">
            <Check size={12} aria-hidden="true" />
            Booked
          </span>
          <span>
            <b className="font-semibold">{name}</b> · {when}
          </span>
        </p>
        <p className="flex items-center gap-2 text-[13px] text-zinc-400 sm:text-2xs">
          <Check size={12} aria-hidden="true" />
          {a.phase === 'sending' ? `Booking ${name}…` : `Sent to ${name}: “${host ? `${host} booked you for` : 'You’re booked for'} ${when}.”`}
        </p>
      </div>
    );
  }
  // Every other answer (Sorry, a reply, No reply needed) is pa13a's: its answered view is the same.
  if (answered) return <AllowCard code={code} guest={guest} name={name} reason={reason} quote={quote} bot={bot} askedAt={askedAt} />;

  if (picking) return <BookFor code={code} guest={guest} name={name} host={host} slots={slots ?? null} askStart={askStart ?? null} onBack={() => setPicking(false)} />;

  const head = askStart ? `${name} asks for a time you haven’t listed: ${slotWhen(askStart)}` : reason || `${name} needs you`;
  const canBook = bookingFeatures.canBook();
  const submit = () => {
    const t = text.trim();
    if (!t) return;
    void deskAnswers.now(code, guest, 'reply', t);
    setWriting(false);
    setText('');
  };
  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-raised p-3" data-odd-allow="open" role="group" aria-label={head}>
      <h4 className="flex items-center gap-2 text-[15px] font-bold text-zinc-50 sm:text-[13.5px]">
        <BellRing size={16} className="shrink-0 text-zinc-300" aria-hidden="true" />
        {head}
      </h4>
      {quote ? <Quote name={name} quote={quote} /> : null}
      {a?.phase === 'failed' && <Failed text={missing ? `Booking for a guest needs a newer kwant on this Mac. Write ${name} a reply instead.` : failText(a.error, name)} />}
      {writing ? (
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
          <label className="sr-only" htmlFor={`odd-${k}`}>
            Your reply to {name}
          </label>
          <textarea
            id={`odd-${k}`}
            autoFocus
            rows={2}
            maxLength={4000}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                submit();
              }
              if (e.key === 'Escape') setWriting(false);
            }}
            placeholder={`Write to ${name}`}
            className={BOX}
          />
          <button type="button" onClick={submit} disabled={!text.trim()} className={`${PRIMARY} min-w-[44px]`} data-action="send">
            <Send size={14} aria-hidden="true" />
            Send
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap sm:items-center" data-allow-actions>
          {canBook && (
            <button type="button" onClick={() => setPicking(true)} className={OK_BTN} data-action="book-for">
              <CalendarClock size={14} aria-hidden="true" />
              Book a time for {name}
            </button>
          )}
          <button type="button" onClick={() => deskAnswers.hold(code, guest, DESK_SORRY)} className={`${HALF_BTN} justify-center`} data-action="sorry">
            <X size={14} aria-hidden="true" />
            Sorry, not this time
          </button>
          <button type="button" onClick={() => setWriting(true)} className={`${HALF_BTN} justify-center`} data-action="write">
            <Reply size={14} aria-hidden="true" />
            Write a reply
          </button>
          <button type="button" onClick={() => void deskAnswers.now(code, guest, 'resolve', null)} className={QUIET} data-action="resolve">
            No reply needed
          </button>
        </div>
      )}
      <p className={SMALL}>
        {canBook
          ? `There’s no plain “Yes” here: a yes to a time is a booking. “Book a time” lets you pick an open time or add one, and ${name} is told it’s booked.`
          : `There’s no plain “Yes” here: a yes to a time is a booking. Write ${name} a reply asking them to pick a time, or add the time in Edit times first.`}
      </p>
    </div>
  );
};

type Pick = { kind: 'slot'; slot: DeskSlot } | { kind: 'new'; day: string; time: string; minutes: number };

export const BookFor: React.FC<{ code: string; guest: string; name: string; host: string; slots: DeskSlot[] | null; askStart: string | null; onBack: () => void }> = ({
  code,
  guest,
  name,
  host,
  slots: given,
  askStart,
  onBack,
}) => {
  const [loaded, setLoaded] = useState<DeskSlot[] | null>(given);
  const [loadError, setLoadError] = useState<string | null>(null);
  // The Inbox has no times: read them once from the desk.
  useEffect(() => {
    if (given) return;
    let gone = false;
    pocketApi
      .read(code)
      .then((r) => {
        if (gone) return;
        const s = r && typeof r === 'object' ? (r as unknown as Record<string, unknown>) : {};
        const summary = s.summary && typeof s.summary === 'object' ? (s.summary as Record<string, unknown>) : {};
        const d = validDesk({ title: 'Desk', answer_bot: { name: 'Bot' }, ...summary, mode: 'booking', slots: s.slots ?? summary.slots });
        if (d?.booking) setLoaded(d.booking.slots);
        else setLoadError('The times couldn’t be read. Try again.');
      })
      .catch(() => !gone && setLoadError(DESK_FAILED));
    return () => {
      gone = true;
    };
  }, [code, given]);
  const slots = useMemo(() => loaded ?? [], [loaded]);
  const now = nowKey();
  const open = slots.filter((s) => isOpen(s, now));
  const openDays = [...new Set(open.map((s) => s.start.slice(0, 10)))];
  const askDay = askStart?.slice(0, 10) ?? null;
  const askOpen = askStart ? open.find((s) => s.start === askStart) ?? null : null;
  const commonLen = slots.length ? slots[slots.length - 1].minutes : 60;
  const [day, setDay] = useState<string>(() => (askDay && askDay >= todayKey() ? askDay : openDays[0] ?? todayKey()));
  const [others, setOthers] = useState(false);
  const [pick, setPick] = useState<Pick | null>(() =>
    askOpen ? { kind: 'slot', slot: askOpen } : askStart && askStart > now ? { kind: 'new', day: askStart.slice(0, 10), time: askStart.slice(11), minutes: commonLen } : null,
  );
  const [newDay, setNewDay] = useState(askStart?.slice(0, 10) ?? day);
  const [newTime, setNewTime] = useState(askStart && !askOpen ? askStart.slice(11) : '');
  const [newLen, setNewLen] = useState(commonLen);
  const [err, setErr] = useState<string | null>(null);
  const holding = holdingOf(slots, guest);
  const dayOpen = open.filter((s) => s.start.slice(0, 10) === day);

  const setNew = (patch: Partial<{ day: string; time: string; minutes: number }>) => {
    const next = { day: patch.day ?? newDay, time: patch.time ?? newTime, minutes: patch.minutes ?? newLen };
    setNewDay(next.day);
    setNewTime(next.time);
    setNewLen(next.minutes);
    setErr(null);
    setPick(next.time ? { kind: 'new', ...next } : null);
  };
  const label = !pick
    ? `Book ${name}`
    : pick.kind === 'slot'
      ? `Book ${name} for ${slotTime(pick.slot.start)}${pick.slot.start.slice(0, 10) === todayKey() ? '' : ` ${weekday(pick.slot.start.slice(0, 10))}`}`
      : `Add ${slotTime(`${pick.day}T${pick.time}`)} and book ${name}`;
  const go = () => {
    if (!pick) return;
    let to: { slot: string } | { new_slot: { start: string; minutes: number } };
    if (pick.kind === 'slot') to = { slot: pick.slot.id ?? pick.slot.start };
    else {
      const start = `${pick.day}T${pick.time}`;
      if (start <= nowKey()) return setErr('That time has passed. Pick a later time.');
      const same = slots.find((s) => s.start === start);
      if (same && !isOpen(same)) return setErr(`${slotWhen(start)} is taken. Pick another time.`);
      to = same?.id ? { slot: same.id } : { new_slot: { start, minutes: pick.minutes } };
    }
    void deskAnswers.nowAs(deskKey(code, guest), code, guest, { kind: 'book', text: null, to });
    onBack();
  };
  const TIME = 'inline-flex min-h-[44px] items-center justify-center rounded-lg border px-3 text-[15px] font-semibold tabular-nums kw-focus sm:min-h-[34px] sm:text-[13px]';
  return (
    <div className="flex flex-col gap-3 rounded-xl border border-hairline-strong bg-surface-raised p-3" data-book-for={guest} role="group" aria-label={`Book a time for ${name}`}>
      <h4 className="flex items-center gap-2 text-[15px] font-bold text-zinc-50 sm:text-[13.5px]">
        <CalendarClock size={16} className="shrink-0 text-zinc-300" aria-hidden="true" />
        Book a time for {name}
      </h4>
      <p className={SMALL}>
        Pick an open time, or add one just for {name}. {name} will read &ldquo;{host ? `${host} booked you for` : 'You’re booked for'} &hellip;&rdquo;, marked &ldquo;host&rdquo;.
      </p>
      {holding && (
        <p className="flex items-start gap-2 text-[13px] text-zinc-300 sm:text-2xs" data-moves>
          <Clock size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
          {name}&rsquo;s {holding.state === 'requested' ? 'request' : 'booking'} for {weekday(holding.slot.start.slice(0, 10))} {slotTime(holding.slot.start)} moves to the time you pick.
        </p>
      )}
      {!loaded && !loadError && <p className={SMALL}>Reading the times&hellip;</p>}
      {loadError && <Failed text={loadError} />}
      {loaded && (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <b className="text-[14px] font-bold text-zinc-100 sm:text-[13px]">
              {slotDay(day)}
              {day === todayKey() ? ' · today' : ''}
            </b>
            <span className="text-2xs text-zinc-400">
              open times only ·{' '}
              <button type="button" onClick={() => setOthers(!others)} aria-expanded={others} className="inline-flex min-h-[44px] items-center font-semibold text-sky-200 hover:underline kw-focus sm:min-h-0" data-action="other-days">
                Other days
              </button>
            </span>
          </div>
          {others && (
            <div className="flex flex-wrap gap-1.5" role="group" aria-label="Days with open times">
              {openDays.length === 0 && <span className={SMALL}>No open times on any day. Add one below.</span>}
              {openDays.map((d) => (
                <button
                  key={d}
                  type="button"
                  aria-pressed={d === day}
                  onClick={() => {
                    setDay(d);
                    setOthers(false);
                  }}
                  className={`${TIME} ${d === day ? 'border-sky-500 bg-sky-800 text-white' : 'border-hairline-strong bg-surface-main text-zinc-200'}`}
                >
                  {slotDay(d)}
                </button>
              ))}
            </div>
          )}
          <div className="grid grid-cols-3 gap-1.5 sm:grid-cols-4" role="radiogroup" aria-label={`Open times on ${slotDay(day)}`}>
            {dayOpen.map((s) => {
              const on = pick?.kind === 'slot' && pick.slot.start === s.start;
              return (
                <button
                  key={s.start}
                  type="button"
                  role="radio"
                  aria-checked={on}
                  onClick={() => {
                    setErr(null);
                    setPick({ kind: 'slot', slot: s });
                  }}
                  className={`${TIME} ${on ? 'border-sky-400 bg-sky-800 text-white' : 'border-hairline-strong bg-surface-main text-zinc-100 hover:bg-white/[0.03]'}`}
                  data-pick={s.start}
                >
                  {slotTime(s.start)}
                </button>
              );
            })}
            {dayOpen.length === 0 && <span className={`${SMALL} col-span-3`}>No open times on {slotDay(day)}.</span>}
          </div>
          <div className="flex flex-wrap items-center gap-2 border-t border-hairline pt-3" data-add-time>
            <span className="text-[13px] font-semibold text-zinc-300 sm:text-[12.5px]">Add a time</span>
            <label className="sr-only" htmlFor={`bf-day-${guest}`}>
              Day
            </label>
            <input id={`bf-day-${guest}`} type="date" min={todayKey()} value={newDay} onChange={(e) => e.target.value && setNew({ day: e.target.value })} className={FIELD} data-field="new-day" />
            <label className="sr-only" htmlFor={`bf-time-${guest}`}>
              Time
            </label>
            <input
              id={`bf-time-${guest}`}
              type="time"
              step={900}
              value={newTime}
              onChange={(e) => setNew({ time: e.target.value })}
              className={`${FIELD} ${pick?.kind === 'new' ? 'border-sky-400' : ''}`}
              data-field="new-time"
            />
            <label className="sr-only" htmlFor={`bf-len-${guest}`}>
              Length
            </label>
            <select id={`bf-len-${guest}`} value={newLen} onChange={(e) => setNew({ minutes: Number(e.target.value) })} className={FIELD} data-field="new-length">
              {LENGTHS.map((m) => (
                <option key={m} value={m}>
                  {lengthLabel(m)}
                </option>
              ))}
            </select>
          </div>
        </>
      )}
      {err && <Failed text={err} />}
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={go} disabled={!pick} className={OK_BTN} data-action="book">
          <Check size={14} aria-hidden="true" />
          {label}
        </button>
        <button type="button" onClick={onBack} className={QUIET} data-action="back">
          Back
        </button>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// Booked list on a stopped desk (B9) and the Inbox row (B10)

export const UpcomingList: React.FC<{ slots: DeskSlot[]; creatures?: Record<string, string | null> }> = ({ slots, creatures = {} }) => {
  const now = nowKey();
  const rows = slots.filter((s) => s.start > now).flatMap((s) => s.confirmed.map((p) => ({ s, p })));
  if (!rows.length) return null;
  return (
    <ul className="overflow-hidden rounded-xl border border-hairline-strong bg-surface-main" aria-label="Bookings still coming up" data-upcoming={rows.length}>
      {rows.map(({ s, p }) => (
        <li key={`${p.guest || 'gone'}@${s.start}@${p.at ?? ''}`} className="grid min-h-[52px] grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-2.5 border-b border-hairline px-3 py-2 text-[14px] last:border-b-0 sm:text-[13.5px]">
          <GuestCreature avatar={p.guest ? (creatures[p.guest] ?? null) : null} size={26} />
          <span className="min-w-0 truncate">
            <b className="font-semibold text-zinc-50 tabular-nums">{slotWhen(s.start)}</b> <span className="text-zinc-300">{p.name ?? 'chat deleted'}</span>
          </span>
          <span className="kw-chip kw-chip-ok" data-chip="booked">
            <Check size={12} aria-hidden="true" />
            Booked
          </span>
        </li>
      ))}
    </ul>
  );
};

/** The Inbox item on a booking desk: "Emma asks for a time" (ask: booking) or the odd-request card (ask: handoff). */
export const BookingNeedsRow: React.FC<{ item: DeskItem }> = ({ item }) => {
  useDeskAnswers();
  const booking = item.ask === 'booking' && !!item.slot_start;
  const k = booking ? bookingKey(item.pocket_code, item.guest_id, item.slot ?? item.slot_start!) : deskKey(item.pocket_code, item.guest_id);
  const a = deskAnswers.get(k);
  if (a?.phase === 'sent' && isAnswered(a, stamp(item.asked_at))) return null;
  // The item has no host name (pa13b-impl.md): the guest-facing lines then go without one.
  const host = item.host_name ?? '';
  const req: BookingRequest | null = booking
    ? {
        slot: { id: item.slot ?? null, start: item.slot_start!, minutes: 60, label: null, seats: 1, requested: [], confirmed: [] },
        person: { guest: item.guest_id, name: item.guest_name, note: item.quote, at: item.asked_at, by: null },
      }
    : null;
  return (
    <li className="overflow-hidden rounded-[14px] border border-hairline-strong bg-surface-raised" data-desk-item={item.id} data-ask={item.ask}>
      <div className="flex items-center gap-2.5 px-3.5 pb-1.5 pt-3">
        <ModernAvatar role="desk" name={item.kwant_name} size={28} modelBadge={false} mood="needs_you" moodText={`${item.kwant_name}: ${item.guest_name} needs you`} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] font-semibold text-zinc-100 sm:text-[14px]">{booking ? `${item.guest_name} asks for a time` : `${item.guest_name} needs you`}</div>
          <div className="truncate text-[13px] text-zinc-400 sm:text-2xs">
            {item.pocket_title} · Booking Desk · {shortWhen(item.asked_at)}
          </div>
        </div>
        <span className="kw-chip shrink-0" data-chip="needs-you">
          <BellRing size={12} aria-hidden="true" />
          Needs you
        </span>
      </div>
      <div className="px-3 pb-3">
        {req ? (
          <BookingAllow code={item.pocket_code} req={req} host={host} bot={item.kwant_name} />
        ) : (
          <OddAllow
            code={item.pocket_code}
            guest={item.guest_id}
            name={item.guest_name}
            reason={item.text}
            quote={item.quote}
            bot={item.kwant_name}
            host={host}
            askedAt={item.asked_at}
            askStart={item.ask_slot_start ?? null}
            slots={null}
          />
        )}
      </div>
    </li>
  );
};

// ---------------------------------------------------------------------------------------------------
// Card parts PocketDesk's DeskCard draws in booking mode (B1, B3, B9)

const NUM = new Intl.NumberFormat('en');

/** B3: Open N times left / Requests N for you / Booked N coming up. B9: the stopped totals. */
export const BookingTiles: React.FC<{ b: DeskBooking; handoffs: number; stopped?: boolean }> = ({ b, handoffs, stopped = false }) => {
  const asks = b.requests + handoffs;
  const tile = (key: string, label: string, value: number, sub: string, hot = false) => (
    <div className={`min-w-0 rounded-xl border bg-surface-main px-3 py-2 ${hot ? 'border-sky-600' : 'border-hairline-strong'}`} data-count={key}>
      <div className="truncate text-2xs font-semibold text-zinc-400">{label}</div>
      <div className="text-[22px] font-extrabold leading-7 text-zinc-50 tabular-nums">{NUM.format(value)}</div>
      <div className="truncate text-2xs font-medium text-zinc-300">{sub}</div>
    </div>
  );
  return (
    <div className="grid grid-cols-3 gap-2" role="group" aria-label="Times at the desk">
      {stopped ? tile('open', 'Times', b.slots.length, 'on the desk') : tile('open', 'Open', b.open_left, b.open_left === 1 ? 'time left' : 'times left')}
      {tile('requests', 'Requests', asks, asks ? 'for you' : 'none', asks > 0 && !stopped)}
      {stopped ? tile('booked', 'Booked', b.booked_total, 'in all') : tile('booked', 'Booked', b.booked_upcoming, 'coming up')}
    </div>
  );
};

/** B1's "What Bell knows" and "Who answers", the cap and the cost line; the times editor sits above them. */
export const BookingSetup: React.FC<{
  d: DeskInfo;
  cap: number;
  setCap: (n: number) => void;
  ask: (t: string) => void;
  editor: React.ReactNode;
  costLine: string;
  CapStepper: React.FC<{ value: number; onChange: (n: number) => void; label?: string }>;
}> = ({ d, cap, setCap, ask, editor, costLine, CapStepper }) => {
  const H = 'flex items-center justify-between gap-2 text-2xs font-bold text-zinc-300';
  return (
    <div className="flex flex-col gap-3" data-setup="booking">
      {editor}
      <div className="flex flex-col gap-2">
        <div className={H}>
          <span>What {d.bot.name} knows</span>
          <small className="font-medium text-zinc-400">optional</small>
        </div>
        {d.guide_preview ? (
          <div className="relative max-h-[80px] overflow-hidden whitespace-pre-line rounded-[10px] border border-zinc-700 bg-surface-main px-3 py-2.5 font-mono text-[12.5px] leading-[19px] text-zinc-200" data-guide>
            {d.guide_preview}
            <span className="pointer-events-none absolute inset-x-0 bottom-0 h-7 bg-gradient-to-t from-surface-main to-transparent" aria-hidden="true" />
          </div>
        ) : (
          <p className={SMALL}>Prices, where to find you, what to bring: {d.bot.name} answers questions from this. Without it, {d.bot.name} only helps guests pick a time.</p>
        )}
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={() => ask(`I want to change what ${d.bot.name} knows for ${d.title}.`)} className={HALF_BTN} data-action="edit-guide">
            <Pencil size={14} aria-hidden="true" />
            {d.guide_preview ? 'Edit' : 'Add notes'}
          </button>
        </div>
      </div>
      <div className="flex flex-col gap-2">
        <div className={H}>Who answers</div>
        <div className="flex min-h-[44px] items-center gap-2.5 rounded-[10px] border border-hairline-strong bg-surface-main px-2.5 py-1.5">
          <ModernAvatar role="desk" name={d.bot.name} size={28} modelBadge={false} mood="thinking" moodText={`${d.bot.name}: learning the times`} />
          <span className="min-w-0 flex-1 text-[15px] sm:text-[13.5px]">
            <b className="font-semibold text-zinc-50">{d.bot.name}</b>
            <small className="block text-2xs text-zinc-400">Helps guests pick a time; only you book</small>
          </span>
          <button type="button" onClick={() => ask(`Change which bot answers guests on ${d.title}.`)} className="min-h-[44px] rounded-lg px-2 text-[14px] font-semibold text-sky-200 hover:bg-white/[0.04] kw-focus sm:min-h-[32px] sm:text-meta" data-action="change-bot">
            Change
          </button>
        </div>
      </div>
      <div className="flex flex-col gap-2">
        <div className={H}>
          <span>Replies a day</span>
          <small className="font-medium text-zinc-400">all guests together</small>
        </div>
        <div className="flex items-center gap-3 rounded-[10px] border border-hairline-strong bg-surface-main px-2.5 py-1.5">
          <CapStepper value={cap} onChange={setCap} />
          <span className="min-w-0 text-[13px] leading-5 text-zinc-300 sm:text-2xs">After {NUM.format(cap)}, guests can still pick a time; their messages wait in your Inbox.</span>
        </div>
      </div>
      <p className="flex items-start gap-2 rounded-[10px] border border-sky-800 bg-sky-500/[0.08] px-3 py-2.5 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-[19px]" data-cost>
        <Zap size={15} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
        <span>{costLine}</span>
      </p>
    </div>
  );
};

/** The read-only week on a live desk: head with ‹ ›, then the days. */
export const LiveWeek: React.FC<{ slots: DeskSlot[] }> = ({ slots }) => {
  const now = nowKey();
  const first = slots.find((s) => s.start > now)?.start.slice(0, 10) ?? todayKey();
  const [monday, setMonday] = useState(() => weekOf(first < todayKey() ? todayKey() : first));
  const inWeek = slots.filter((s) => s.start.slice(0, 10) >= monday && s.start.slice(0, 10) <= addDays(monday, 6));
  const days = shownDays(inWeek, monday);
  return (
    <div className="flex flex-col gap-2" data-live-week>
      <WeekHead monday={monday} setMonday={setMonday} days={days} />
      <SlotWeek slots={inWeek} monday={monday} view="week" />
    </div>
  );
};

/** B9: "N bookings are still coming up…" */
export const stoppedLine = (upcoming: number) =>
  upcoming > 0
    ? `${upcoming} ${upcoming === 1 ? 'booking is' : 'bookings are'} still coming up. ${upcoming === 1 ? 'It stays' : 'They stay'} here until ${upcoming === 1 ? 'its' : 'their'} day has passed. Nobody new can ask.`
    : 'The guest page is closed. Nobody new can ask.';
