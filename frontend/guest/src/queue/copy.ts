// The Queue ticket's words: queue-design.md "Copy table", with Vega's codes from queue-impl.md. The server sends codes
// and the page owns the words; the code of a refusal is never shown. Numbers are the server's (place, ahead, waiting,
// now_serving, avg_minutes, call_minutes, max_waiting). The one figure the page works out is the design's {min_total},
// "about N minutes": the server's avg_minutes times the turns before this one, and only once avg_minutes is set
// (3 calls today). Before that no minute figure is shown anywhere.
import { clock } from '../app';
import { fmt } from '../dom';
import type { Phase, QueueView, Ticket } from './view';

/** A bold lead and the rest of the sentence, drawn as <b>lead</b> rest. */
export type Line = [string, string];

export const DEFAULT_CLOSE = "Sorry, we've closed for today. Thanks for waiting, and see you next time!";

/** "1st", "2nd", "3rd", "11th", "22nd". */
export function ordinal(n: number): string {
  const t = n % 100;
  if (t >= 11 && t <= 13) return `${n}th`;
  return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`;
}

/** "13:00" → "1 PM", "12:30" → "12:30 PM" (the box's own clock, shown as it is: no time zone sums). */
export function untilText(hhmm: string): string {
  const [hh, mm] = hhmm.split(':').map(Number);
  const h12 = hh % 12 || 12;
  return `${h12}${mm ? `:${String(mm).padStart(2, '0')}` : ''} ${hh < 12 ? 'AM' : 'PM'}`;
}

/** {min_total}: the turns before you times the measured average, or null while there is no average. */
export function minutesFor(turns: number, avg: number | null): number | null {
  return avg == null || !(turns > 0) ? null : turns * avg;
}

const minutes = (n: number): string => `${fmt(n)} ${n === 1 ? 'minute' : 'minutes'}`;

/** "1 person", "3 people". */
export const people = (n: number): string => `${fmt(n)} ${n === 1 ? 'person' : 'people'}`;

/** The host by name, or "the host". */
export const hostOf = (v: Pick<QueueView, 'host_name'>): string => v.host_name || 'the host';

export const keepLine = (v: Pick<QueueView, 'ask_name'>): string =>
  v.ask_name
    ? "Your ticket is a number and, if you like, a first name. Both are deleted when you're served, skipped or leave, or when the queue closes. No phone number, no account."
    : "Your ticket is just a number. It's deleted when you're served, skipped or leave, or when the queue closes. No phone number, no account.";

/** The bar's right side: "open until 1 PM", "open", "paused", "closed". */
export function barRight(v: QueueView): string {
  if (v.state === 'paused') return 'paused';
  if (v.state === 'closed') return 'closed';
  if (v.state === 'ended') return '';
  return v.hours_until ? `open until ${untilText(v.hours_until)}` : 'open';
}

/** The three Take facts: [label, value]. "—" stands in for what isn't measured or set yet. */
export function facts(v: QueueView): [string, string][] {
  return [
    ['Now serving', v.now_serving ? `#${v.now_serving}` : '—'],
    ['Waiting', fmt(v.waiting)],
    ['Each turn', v.avg_minutes ? `~${fmt(v.avg_minutes)} min` : '—'],
  ];
}

/** `would_be`: the next ticket would be after everyone waiting now. */
export function wouldBe(v: QueueView): Line {
  const place = ordinal(v.waiting + 1);
  const m = minutesFor(v.waiting + 1, v.avg_minutes);
  return [place, m == null ? '.' : `, about ${minutes(m)}.`];
}

export const BUZZ_HINT = "We'll buzz this page when it's your turn. Keep it open; nothing is sent to your phone.";
export const nameLabel = (v: QueueView): string => `First name · optional · only ${hostOf(v)} sees it`;

/** The ticket's `eta` pill, or null until avg_minutes is set. */
export function eta(t: Ticket, v: QueueView): string | null {
  const m = t.place ? minutesFor(t.place, v.avg_minutes) : null;
  return m == null ? null : `about ${minutes(m)}`;
}

export const backLine = (t: Ticket): string => {
  const at = t.taken_at ? clock(t.taken_at) : '';
  return at ? `Welcome back. This is your ticket from ${at}.` : 'Welcome back. This is your ticket.';
};

export const BUZZ_HEAD = "When it's your turn";
export const NO_VIBRATE = "iPhone can't vibrate from a web page";
export const HONEST: Line = ['Keep this page open.', "It can only buzz you while it's open: there's no app, and nothing is sent to your phone."];
export const HONEST_IOS: Line = ['Keep this page open, screen on.', "On iPhone the page goes quiet when the screen locks, so leave it unlocked. Nothing is sent to your phone: there's no app."];

/** "the side counter" from the host's line when it names a place to go, else "the counter". */
export function where(v: QueueView): string {
  const m = /\b(?:at|to)\s+(the\s+[^.,;:!?]{1,40})/i.exec(v.line);
  return m ? m[1].trim() : 'the counter';
}

export const nextLines = (v: QueueView): [string, string] => [`Head over to ${where(v)}.`, "We buzzed once; we'll buzz again when it's your turn."];

/** `turn`: "Come to the side counter and show this number." A host line that names no place: "{line}. Show this number." */
export function turnLine(v: QueueView): string {
  const w = where(v);
  if (w === 'the counter' && v.line) return `${v.line.replace(/[.!]+$/, '')}. Show this number.`;
  return `Come to ${w} and show this number.`;
}

export const turnNote = (v: QueueView): string =>
  `After ${minutes(v.call_minutes)} your number is skipped. Running late? ${cap(hostOf(v))} can still bring it back for a few minutes.`;

/** m:ss for the countdown, from whole seconds left. */
export function mss(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** The countdown's screen-reader name: "2 minutes 41 seconds left". */
export function leftLabel(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const m = Math.floor(s / 60);
  const r = s % 60;
  return [m ? `${m} ${m === 1 ? 'minute' : 'minutes'}` : '', r || !m ? `${r} ${r === 1 ? 'second' : 'seconds'}` : ''].filter(Boolean).join(' ') + ' left';
}

export const GIVE_UP = "I can't come · give up my turn";
export const LEAVE = 'Leave the queue';
export const leaveConfirm = (t: Ticket): Line => ['Leave the queue?', `Number #${t.number} is given up. To come back you'd take a new ticket at the end.`];

export const pausedNote = (v: QueueView): Line => [`${v.title} paused the queue:`, `${v.pause_text ? `“${v.pause_text}” ` : ''}You keep your place.`];

export function skippedLines(t: Ticket, v: QueueView): { lead: string; after: string } {
  const at = t.called_at ? clock(t.called_at) : '';
  const m = minutesFor(v.waiting, v.avg_minutes);
  return {
    lead: `Number #${t.number} was called${at ? ` at ${at}` : ''} and nobody came in ${minutes(v.call_minutes)}.`,
    after: `Now ${fmt(v.waiting)} waiting${m == null ? '' : `, about ${minutes(m)}`}. Or ask at the counter.`,
  };
}

export const closeQuote = (v: QueueView): string => `“${v.close_text || DEFAULT_CLOSE}”`;

/** The refusals (r_*): a bold lead and the rest. `closed` and `paused` also show when the view says so before a tap. */
export type Refusal = 'paused' | 'closed' | 'full' | 'slow' | 'slow_net' | 'name';

export function refusalCopy(r: Refusal, v: QueueView, message?: string): Line {
  switch (r) {
    case 'paused':
      return [v.pause_text ? `Paused: “${v.pause_text}”` : 'Paused.', 'New tickets open when the queue resumes. Keep this page open; it updates by itself.'];
    case 'closed':
      return [`${v.title || 'This queue'} is closed for today.`, 'See you next time!'];
    case 'full':
      return [`The queue is full right now${v.max_waiting ? `: ${fmt(v.max_waiting)} waiting` : ''}.`, 'Try again in a few minutes, or ask at the counter.'];
    case 'slow':
      return ['This phone has taken 3 tickets in the last hour.', 'Wait a bit, or ask at the counter.'];
    case 'slow_net':
      return ['Lots of tickets have come from this network in the last hour.', 'Wait a bit, or ask at the counter.'];
    case 'name':
      return [message || 'Type a first name, or leave it empty.', ''];
  }
}

export const GONE = 'This queue has ended.';
export const OFFLINE: Line = ['No connection:', "your ticket is safe. This page will update when it's back."];

/** The tab title for each phase (`title_*`). `blink` is the once-a-second "#27" swap while called. */
export function tabTitle(p: Phase, v: QueueView, blink = false): string {
  const t = v.my_ticket;
  const title = v.title || 'Queue';
  if (!t) return title;
  switch (p) {
    case 'waiting':
      return t.place ? `(${ordinal(t.place)}) ${title} · #${t.number}` : `${title} · #${t.number}`;
    case 'next':
      return `(Next) ${title} · #${t.number}`;
    case 'turn':
      return blink ? `#${t.number}` : `(!) Your turn: #${t.number} · ${title}`;
    case 'skipped':
      return `Skipped · ${title}`;
    case 'served':
      return `Served · ${title}`;
    case 'closed':
      return `Closed · ${title}`;
    default:
      return title;
  }
}

const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);
