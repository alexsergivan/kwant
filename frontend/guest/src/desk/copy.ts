// The front desk's words: every server answer the guest can meet, as the page's own copy (design.md "For Iris" 5,
// with Lin's codes from impl.md). {host} and {bot} come from the view's host_name and bot.name; an empty host_name
// reads "the host". The code or the cause of a refusal is never shown.
import type { Fail } from '../api';
import { clock } from '../app';
import { fmt } from '../dom';
import { cap, pinCopy, Problem } from '../santa/pin';
import type { DeskIcon } from './look';
import type { BookingNote, DeskView, NoteKind } from './view';

export interface Names {
  /** "Marta", or "the host" (sentence case where it starts a sentence: use `Host`). */
  host: string;
  Host: string;
  /** "Marta's", "the host's". */
  hosts: string;
  bot: string;
  /** host_name was left out: the host's tile shows the kit bell icon instead of an initial. */
  anon: boolean;
}

export function names(v: Pick<DeskView, 'host_name' | 'bot'> | null | undefined): Names {
  const name = v?.host_name ?? '';
  const host = name || 'the host';
  const bot = v?.bot || 'the bot';
  return { host, Host: cap(host), hosts: `${host}'s`, bot: cap(bot), anon: !name };
}

/** A line with a bold lead: `[bold, rest]`, drawn as <b>bold</b> rest. */
export type Line = [string, string];

export function noteCopy(kind: NoteKind, n: Names, when = ''): { line: Line; icon: DeskIcon; ok?: boolean } {
  if (kind.startsWith('book')) return bookNote(kind as BookingNote, n, when);
  if (kind === 'handoff') return { line: [`${n.Host} will reply here.`, "You can close this page; your chat stays, and it's here when you come back."], icon: 'bell' };
  if (kind === 'daily_cap')
    return { line: [`${n.bot} has answered all it can for today.`, `${n.Host} will read your message and reply here. ${n.bot} is back tomorrow morning.`], icon: 'moon' };
  return { line: [`${n.bot} can't answer right now.`, `${n.Host} has your message and will reply here.`], icon: 'moon' };
}

/** G6, 429 slow_down: the open time is the server's retry_at in this phone's own time. */
export const slowCopy = (retryAt: string): { lead: string; time: string; rest: string; hint: string } => {
  const time = clock(retryAt);
  return {
    lead: "That's a lot of messages for one hour.",
    time,
    rest: 'Your message is still here, ready to send.',
    hint: time ? `You can send it at ${time}` : 'You can send it soon',
  };
};

/** G8, 413 too_long, checked on the phone first from limits.max_chars. */
export const longCopy = (max: number): Line => ["That's a long one.", `Messages can be up to ${fmt(max)} characters. Try two.`];

/** G9: a message that didn't reach the box stays on this phone. */
export const offCopy: Line = ['No connection.', 'Your message is saved on this phone.'];

/** A refused join or rejoin. 409 guest_max (and pa1's own full) is the desk's; the rest is pa1's PIN flow. */
export function joinCopy(r: Fail, n: Names, joining: boolean): Problem {
  if (r.error === 'guest_max' || r.error === 'full') return { title: 'This guide is full right now.', sub: `Ask ${n.host} to make room.` };
  return pinCopy(r, n.host, joining);
}

/** A refused message other than 429 / 413 / 410: plain words, never the code. */
export function sayCopy(r: Fail): string {
  if (r.error === 'empty') return 'Write a message first.';
  return "That message didn't go through. Try again.";
}

// ---------- pa13b Booking Desk (pa13b-design.md copy table; the keys are the table's) ----------
// {when} is "Wed Dec 2, 10 AM" from slots.ts whenLabel, in the box's own wall-clock time.

export const book = {
  join_say: (n: Names, what?: string): string =>
    what ? `Pick a time for ${what} and ${n.host} will confirm it here. Ask me anything else too.` : `Pick a time and ${n.host} will confirm it here. Ask me anything too.`,
  join_button: 'See open times',
  join_nobook: (n: Names): string => `${n.bot} can't book you. You ask for a time and ${n.host} confirms it. Nothing to pay here.`,
  /** `days` is the box's keep_days (7 in the design). */
  keep_booking: (n: Names, days = 7): string =>
    `${n.Host} and ${n.hosts} bot, ${n.bot}, read this chat. Messages are deleted ${fmt(days)} day${days === 1 ? '' : 's'} after your last one, or the day after your booking if that's later.`,
  hello: (name: string, n: Names): string => `Hi ${name}! I'm ${n.bot}, ${n.hosts} bot. Pick a time and ${n.host} will confirm it, or ask me anything else.`,
  starter_pick: 'Pick a time',
  sheet_title: 'Pick a time',
  day_open: (count: number): string => `${fmt(count)} open`,
  day_full: 'full',
  day_line: (day: string, length?: string): string => (length ? `${day} · each ${length} · only open times show` : `${day} · only open times show`),
  note_label: (n: Names): string => `A note for ${n.host} · optional`,
  ask_button: (when: string): string => `Ask for ${when}`,
  ask_button_off: 'Pick a time first',
  sheet_fine: (n: Names): string => `${n.Host} confirms every booking. Until then the time is held for you. Nothing to pay here.`,
  strip_requested: (n: Names): string => `Requested · ${n.host} will confirm`,
  strip_cancel: 'Cancel',
  cancel_q: (when: string): Line => [`Cancel ${when}?`, 'The time goes back to others.'],
  cancel_keep: 'Keep it',
  cancel_yes: 'Cancel my request',
  strip_booked: (n: Names): string => `Booked · confirmed by ${n.host}`,
  chip_booked: 'Booked',
  pick_again: 'Pick another time',
  none_title: 'No open times right now',
  none_body: (n: Names): string => `${n.Host} adds new times each week. Write here if you'd like a time, and ${n.host} will reply.`,
  none_button: (n: Names): string => `Write to ${n.host}`,
  taken: (time: string): Line => [time ? `Someone just took ${time}.` : 'Someone just took that time.', 'Pick another time.'],
  has_request: (when: string): string => (when ? `You already asked for ${when}. Cancel it first to pick another time.` : 'You already asked for a time. Cancel it first to pick another time.'),
  slot_past: 'That time has passed. Pick another time.',
  no_slot: (n: Names): string => `${n.Host} changed the times. Pick again.`,
  ended_title: (n: Names): string => (n.anon ? 'Bookings are closed' : `${n.Host} closed bookings`),
  ended_booked: (n: Names): string => `Your booking stays · confirmed by ${n.host}`,
  ended_change: (n: Names): string => `To change it, write to ${n.host} the way you found this page.`,
  /** Not in the copy table: K11 for a guest with no booking. */
  ended_none: (n: Names): string => `If ${n.host} confirmed a time for you, it stays. For anything else, write to ${n.host} the way you found this page.`,
  ended_keep: (date: string): string => `Your messages are deleted on ${date}, the day after your booking.`,
  offers_label: (n: Names): string => `Times ${n.bot} found`,
  /** Not in the copy table (Vega's request limit, 429 slow_down): flagged for Mira in pa13b-guest-impl.md. */
  slow: (time: string): string => (time ? `That's a lot of requests this hour. You can ask again at ${time}.` : "That's a lot of requests this hour. You can ask again soon."),
  /** Not in the copy table: a note over limits.note_max_chars (the page checks first; 413 too_long says the same). */
  note_long: (max: number): string => `A note can be up to ${fmt(max)} characters.`,
  /** Not in the copy table: any other refusal of a request or a cancel. */
  failed: "That didn't go through. Try again.",
};

function bookNote(kind: BookingNote, n: Names, when: string): { line: Line; icon: DeskIcon; ok?: boolean } {
  // A note without its time (Vega's draft sends none) still says what happened, without a time.
  const t = when || 'a time';
  switch (kind) {
    case 'booking_requested':
      return { line: [`You asked for ${t}.`, `${n.Host} will confirm it here. The time is held for you until then.`], icon: 'cal' };
    case 'booking_confirmed':
      return { line: [when ? `${n.Host} confirmed your time: ${when}.` : `${n.Host} confirmed your time.`, 'See you then! To change it, write here.'], icon: 'tick', ok: true };
    case 'booked_by_host':
      return { line: [when ? `${n.Host} booked you for ${when}.` : `${n.Host} booked you a time.`, 'See you then! To change it, write here.'], icon: 'tick', ok: true };
    case 'booking_declined':
      return { line: [when ? `${n.Host} can't do ${when}.` : `${n.Host} can't do that time.`, 'The time is free again.'], icon: 'x' };
    case 'booking_cancelled':
      return { line: [when ? `You cancelled ${when}.` : 'You cancelled your request.', ''], icon: 'cal' };
  }
}
