// Pocket apps (pa11b): the queue ticket host card in chat (pa11/queue-design.md §1 and "For Pixel"; Vega's API in
// pa11/queue-impl.md wins where they differ). States: 1 set-up, 2 open and empty, 3 waiting list, 4 a number called,
// 5 a number skipped, 6 paused, 7 close-for-today confirm, 8 closed, 9 ended, plus the A6 counter sign (print only).
// Customers' first names show only here, always as plain text. Never holo: calling a number is the job, not a result
// a check earned. Paused, closed and errors stay plain chips.

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  AlertTriangle,
  Check,
  CheckCheck,
  Clock,
  Eye,
  Globe,
  Lock,
  Megaphone,
  Minus,
  Monitor,
  Pause,
  Pencil,
  Play,
  Plus,
  Printer,
  RotateCcw,
  ShieldCheck,
  SkipForward,
  Square,
  Ticket,
  Trash2,
  Undo2,
  Wifi,
  X,
  Zap,
} from 'lucide-react';
import type { PocketBlock, QueueInfo, QueueTicket } from '../../smartResults';
import { isIndex, isObj, isText, optText } from '../../smartResults';
import type { Mood } from '../../kwantMood';
import { linkUrl, pocketApi, PocketApiError } from '../../pocketApi';
import { QrImage } from '../DeviceSignIn';
import { Bar, Callout, CopyLink, GHOST, HALF, ICON, LaterButton, LinkField, PRIMARY, Qr, Row, STOP } from './pocketParts';
import { GuestCreature, validSantaJoin } from './PocketSanta';

// ---------------------------------------------------------------------------------------------------
// Wire shape

export const QUEUE_CLOSE_DEFAULT = "Sorry, we've closed for today. Thanks for waiting, and see you next time!";
export const QUEUE_PAUSE_PRESETS = ['Back in 10 minutes', 'Back after lunch'] as const;
export const QUEUE_PAUSE_MAX = 60;
export const QUEUE_CLOSE_MAX = 140;
export const QUEUE_LINE_MAX = 80;
const CALL_MIN = 1;
const CALL_MAX = 10;
const MAX_TICKETS = 260;
/** Remove waits this long for Undo before it reaches the Mac. */
export const QUEUE_UNDO_MS = 5000;
/** While open or paused, the card reads the queue again this often (only while the page is visible). */
export const QUEUE_POLL_MS = 10_000;
/** The guest page's creatures, so a number looks the same here as on the customer's phone. */
const AVATARS = ['antenna', 'ears', 'visor', 'spikes', 'beret', 'goggles', 'bulb', 'horns'];
export const ticketCreature = (n: number) => AVATARS[n % AVATARS.length];

const isCount = (v: unknown): v is number => isIndex(v) && v <= 1_000_000;
const count = (v: unknown, d = 0) => (v === undefined || v === null ? d : isCount(v) ? v : undefined);
const optCount = (v: unknown) => (v === undefined || v === null ? null : isCount(v) ? v : undefined);
const optTime = (v: unknown) => (v === undefined || v === null ? null : typeof v === 'string' && v.length <= 60 && !Number.isNaN(Date.parse(v)) ? v : undefined);
const optHour = (v: unknown) => (v === undefined || v === null ? null : typeof v === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(v) ? v : undefined);
/** Customers' and the host's own words: any plain string within its length, shown as text only. */
const optPlain = (v: unknown, max: number) => (v === undefined || v === null || v === '' ? null : typeof v === 'string' && v.length <= max && !/[\u0000-\u0008\u000b-\u001f]/.test(v) ? v : undefined);

function validTicket(v: unknown): QueueTicket | undefined {
  if (!isObj(v) || !isIndex(v.number) || v.number < 1 || v.number > 100_000) return undefined;
  if (v.state !== 'waiting' && v.state !== 'called' && v.state !== 'skipped') return undefined;
  const name = optPlain(v.name, 40);
  const t = { taken_at: optTime(v.taken_at), called_at: optTime(v.called_at), call_ends_at: optTime(v.call_ends_at), recall_until: optTime(v.recall_until) };
  if (name === undefined || Object.values(t).includes(undefined)) return undefined;
  return { number: v.number, name, state: v.state, ...(t as Pick<QueueTicket, 'taken_at' | 'called_at' | 'call_ends_at' | 'recall_until'>) };
}

/**
 * block.queue on the wire: Vega's host view plus the pocket's own `join` (and `ended_at`). Missing counts fall back to
 * 0, call time to 3 minutes, the close text to the default; a wrong type anywhere rejects the block (it shows as
 * markdown). Block state "writing" is set-up (spec only), "ended" is ended, whatever the view says.
 */
export function validQueue(v: unknown, fallbackTitle?: string, blockState?: string): QueueInfo | undefined {
  if (!isObj(v)) return undefined;
  const sp = v.spec === undefined || v.spec === null ? {} : v.spec;
  if (!isObj(sp)) return undefined;
  const title = sp.title === undefined || sp.title === null || sp.title === '' ? v.title ?? fallbackTitle : sp.title;
  if (!isText(title, 80)) return undefined;
  const line = optText(sp.line ?? v.line, QUEUE_LINE_MAX);
  const hostName = optText(sp.host_name, 40);
  const hoursObj = sp.hours;
  const specUntil = hoursObj === undefined || hoursObj === null ? null : isObj(hoursObj) ? optHour(hoursObj.until) : undefined;
  const call = count(sp.call_minutes, 3);
  const maxWaiting = count(sp.max_waiting, 200);
  const askName = sp.ask_name === undefined || sp.ask_name === null ? false : typeof sp.ask_name === 'boolean' ? sp.ask_name : undefined;
  if ([line, hostName, specUntil, call, maxWaiting, askName].includes(undefined)) return undefined;
  if (call! < CALL_MIN || call! > CALL_MAX) return undefined;
  const raw = v.state === undefined || v.state === null ? (blockState === 'writing' ? 'setup' : undefined) : v.state;
  if (raw !== 'setup' && raw !== 'open' && raw !== 'paused' && raw !== 'closed' && raw !== 'ended') return undefined;
  const state: QueueInfo['state'] = blockState === 'writing' ? 'setup' : blockState === 'ended' ? 'ended' : raw;
  const hours = optHour(v.hours_until);
  const text = { pause_text: optPlain(v.pause_text, QUEUE_PAUSE_MAX), close_text: optPlain(v.close_text, QUEUE_CLOSE_MAX) };
  const times = { paused_at: optTime(v.paused_at), closed_at: optTime(v.closed_at), now: optTime(v.now), ended_at: optTime(v.ended_at) };
  const autoClosed = v.auto_closed === undefined || v.auto_closed === null ? false : typeof v.auto_closed === 'boolean' ? v.auto_closed : undefined;
  const closedWaiting = count(v.closed_waiting);
  const nowServing = optCount(v.now_serving);
  const next = count(v.next_number, 1);
  if ([hours, autoClosed, closedWaiting, nowServing, next].includes(undefined) || Object.values(text).includes(undefined) || Object.values(times).includes(undefined)) return undefined;
  const list = v.tickets === undefined || v.tickets === null ? [] : v.tickets;
  if (!Array.isArray(list) || list.length > MAX_TICKETS) return undefined;
  const tickets = list.map(validTicket);
  if (tickets.some((t) => !t) || new Set(tickets.map((t) => t!.number)).size !== tickets.length) return undefined;
  if (tickets.filter((t) => t!.state === 'called').length > 1) return undefined;
  const s = v.summary === undefined || v.summary === null ? {} : v.summary;
  if (!isObj(s)) return undefined;
  const summary = {
    served_today: count(s.served_today),
    skipped_today: count(s.skipped_today),
    left_today: count(s.left_today),
    waiting: count(s.waiting, tickets.filter((t) => t!.state === 'waiting').length),
    avg_minutes: optCount(s.avg_minutes),
    longest_wait_minutes: optCount(s.longest_wait_minutes),
    days: count(s.days),
    served_total: count(s.served_total),
    avg_total_minutes: optCount(s.avg_total_minutes),
  };
  if (Object.values(summary).includes(undefined)) return undefined;
  const join = v.join === undefined || v.join === null ? null : validSantaJoin(v.join);
  if (join === undefined) return undefined;
  return {
    state,
    spec: { title, line: line!, hours_until: specUntil!, call_minutes: call!, ask_name: askName!, max_waiting: maxWaiting!, host_name: hostName! },
    ...(text as Pick<QueueInfo, 'pause_text' | 'close_text'>),
    paused_at: times.paused_at!,
    closed_at: times.closed_at!,
    auto_closed: autoClosed!,
    closed_waiting: closedWaiting!,
    hours_until: hours === null ? specUntil! : hours!,
    now_serving: nowServing!,
    next_number: next!,
    tickets: (tickets as QueueTicket[]).slice().sort((a, b) => a.number - b.number),
    summary: summary as QueueInfo['summary'],
    now: times.now!,
    join,
    ended_at: times.ended_at!,
  };
}

/** A host read (`{pocket, host_view, join}`) laid over what the card shows; null when it doesn't parse. */
export function queueFromRead(prev: QueueInfo, read: unknown): QueueInfo | null {
  if (!isObj(read) || !isObj(read.host_view)) return null;
  const pocket = isObj(read.pocket) ? read.pocket : {};
  const ended = pocket.state === 'ended' || pocket.state === 'closed';
  const join = read.join === undefined ? prev.join : read.join;
  const q = validQueue({ ...read.host_view, join, ended_at: pocket.ended_at ?? read.host_view.ended_at ?? null }, prev.spec.title, ended ? 'ended' : 'live');
  return q ?? null;
}

/** Ben's mood beside the card: idle when open and empty, working while serving, thinking paused, finished closed. */
export function queueMood(q: QueueInfo): Mood {
  switch (q.state) {
    case 'setup':
    case 'paused':
      return 'thinking';
    case 'closed':
    case 'ended':
      return 'finished';
    default:
      return q.tickets.some((t) => t.state === 'waiting' || t.state === 'called') ? 'working' : 'idle';
  }
}

// ---------------------------------------------------------------------------------------------------
// Words and times

const P = 'text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5';
const SMALL = 'text-[13px] leading-5 text-zinc-400 sm:text-2xs';
const GRID = 'grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center';
const FULL = 'col-span-2 sm:col-span-1';
const HALF_BTN = `kw-btn-ghost ${HALF}`;
const num = (n: number) => n.toLocaleString('en-US');
const people = (n: number) => (n === 1 ? '1 person' : `${num(n)} people`);
/** "10:31" (the day's own clock, no AM/PM, as on the ticket). */
export const clock = (iso: string | null) => {
  if (!iso) return '';
  const d = new Date(iso);
  return `${d.getHours() % 12 || 12}:${String(d.getMinutes()).padStart(2, '0')}`;
};
const clockAmPm = (iso: string | null) => {
  if (!iso) return '';
  const d = new Date(iso);
  return `${clock(iso)} ${d.getHours() < 12 ? 'AM' : 'PM'}`;
};
/** "13:00" → "1 PM", "13:30" → "1:30 PM". */
export const hourLabel = (hm: string | null) => {
  if (!hm) return '';
  const [h, m] = hm.split(':').map(Number);
  return `${h % 12 || 12}${m ? `:${String(m).padStart(2, '0')}` : ''} ${h < 12 ? 'AM' : 'PM'}`;
};
const dayLabel = (iso: string | null) => (iso ? new Date(iso).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) : '');
const mmss = (ms: number) => {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};
const minsSince = (iso: string | null, now: number) => (iso ? Math.max(0, Math.floor((now - Date.parse(iso)) / 60_000)) : null);

const errorLine = (e: unknown) => {
  const code = e instanceof PocketApiError ? e.code : null;
  switch (code) {
    case 'paused':
      return 'The queue is paused. Resume it first.';
    case 'empty':
      return 'Nobody is waiting right now.';
    case 'not_now':
      return "That can't be done right now. The card shows the queue as it is.";
    case 'closed':
      return 'The queue is closed for today.';
    case 'too_late':
      return "It's too late to bring that number back.";
    case 'ended':
      return 'This queue has ended.';
    case 'no_such_ticket':
      return 'That ticket is gone already.';
    default:
      return `Couldn't do that: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`;
  }
};

const joinUrlOf = (q: QueueInfo) => q.join?.anywhere_url ?? q.join?.wifi_url ?? null;
const shortUrl = (u: string) => u.replace(/^https?:\/\//, '').replace(/\/$/, '');

// ---------------------------------------------------------------------------------------------------
// Small parts

const QueueChip: React.FC<{ q: QueueInfo; called: QueueTicket | null }> = ({ q, called }) => {
  const w = q.summary.waiting;
  if (q.state === 'setup')
    return (
      <span className="kw-chip kw-chip-info" data-chip="setup">
        <Pencil size={12} aria-hidden="true" />
        Set up · not open yet
      </span>
    );
  if (q.state === 'open')
    return (
      <span className="kw-chip border-sky-400 bg-sky-600 font-semibold text-white" data-chip="open">
        <span className="kw-status-dot bg-white" aria-hidden="true" />
        {called ? `Open · #${called.number} called` : w ? `Open · ${num(w)} waiting` : 'Open · nobody waiting'}
      </span>
    );
  if (q.state === 'paused')
    return (
      <span className="kw-chip kw-chip-warn" data-chip="paused">
        <Pause size={12} aria-hidden="true" />
        {w ? `Paused · ${num(w)} keep${w === 1 ? 's' : ''} their place` : 'Paused'}
      </span>
    );
  return (
    <span className="kw-chip" data-chip={q.state}>
      {q.state === 'closed' ? <Lock size={12} aria-hidden="true" /> : <Square size={12} aria-hidden="true" />}
      {q.state === 'closed' ? `Closed for today${q.closed_at ? ` · ${clockAmPm(q.closed_at)}` : ''}` : `Ended${q.ended_at ? ` · ${dayLabel(q.ended_at)}` : ''}`}
    </span>
  );
};

const Tile: React.FC<{ k: string; label: string; value: React.ReactNode; sub: string; hot?: boolean }> = ({ k, label, value, sub, hot }) => (
  <div className={`min-w-0 rounded-xl border bg-surface-main px-3 py-2 ${hot ? 'border-sky-600' : 'border-hairline-strong'}`} data-count={k}>
    <div className="text-2xs font-semibold leading-4 text-zinc-400 sm:truncate">{label}</div>
    <div className="text-[22px] font-extrabold leading-7 text-zinc-50 tabular-nums">{value}</div>
    <div className="text-2xs font-medium leading-4 text-zinc-300 sm:truncate">{sub}</div>
  </div>
);

const avg = (n: number | null) => (n === null ? '—' : `${num(n)} min`);

const Tiles: React.FC<{ q: QueueInfo }> = ({ q }) => {
  const s = q.summary;
  if (q.state === 'ended')
    return (
      <div className="grid grid-cols-3 gap-2" role="group" aria-label="The queue in all">
        <Tile k="days" label="Days open" value={num(s.days)} sub={s.days === 1 ? 'day' : 'days'} />
        <Tile k="served" label="Served" value={num(s.served_total)} sub="in all" />
        <Tile k="avg" label="Average wait" value={avg(s.avg_total_minutes)} sub={s.avg_total_minutes === null ? 'too few calls' : 'per turn'} />
      </div>
    );
  if (q.state === 'closed')
    return (
      <div className="grid grid-cols-3 gap-2" role="group" aria-label="Today at the counter">
        <Tile k="served" label="Served" value={num(s.served_today)} sub="today" />
        <Tile k="avg" label="Average wait" value={avg(s.avg_minutes)} sub={s.avg_minutes === null ? 'too few calls' : 'per turn'} />
        <Tile k="skipped-left" label="Skipped · left" value={`${num(s.skipped_today)} · ${num(s.left_today)}`} sub="no-shows, gave up" />
      </div>
    );
  return (
    <div className="grid grid-cols-3 gap-2" role="group" aria-label="Today at the counter">
      <Tile k="served" label="Served today" value={num(s.served_today)} sub="today" />
      <Tile k="avg" label="Average wait" value={avg(s.avg_minutes)} sub={s.avg_minutes === null ? 'measured after 3 calls' : 'per turn, measured today'} />
      <Tile k="waiting" label="Waiting" value={num(s.waiting)} sub={s.waiting && s.longest_wait_minutes !== null ? `longest ${num(s.longest_wait_minutes)} min` : 'nobody now'} hot={s.waiting > 0} />
    </div>
  );
};

const CallStepper: React.FC<{ value: number; onChange: (n: number) => void }> = ({ value, onChange }) => {
  const set = (n: number) => onChange(Math.min(CALL_MAX, Math.max(CALL_MIN, n)));
  const SIDE = 'flex h-full w-11 items-center justify-center text-zinc-200 hover:bg-white/[0.05] disabled:cursor-not-allowed disabled:text-zinc-600 kw-focus sm:w-9';
  return (
    <div className="inline-grid h-[46px] shrink-0 grid-cols-[auto_72px_auto] items-center overflow-hidden rounded-[10px] border border-hairline-strong sm:h-9" role="group" aria-label="Call time in minutes" data-stepper>
      <button type="button" className={SIDE} aria-label={`Shorter: ${value - 1} min`} disabled={value <= CALL_MIN} onClick={() => set(value - 1)} data-step="down">
        <Minus size={15} aria-hidden="true" />
      </button>
      <output className="border-x border-hairline text-center text-[15px] font-semibold leading-[44px] text-zinc-50 tabular-nums sm:leading-[34px]" aria-live="polite">
        {value} min
      </output>
      <button type="button" className={SIDE} aria-label={`Longer: ${value + 1} min`} disabled={value >= CALL_MAX} onClick={() => set(value + 1)} data-step="up">
        <Plus size={15} aria-hidden="true" />
      </button>
    </div>
  );
};

const Switch: React.FC<{ id: string; on: boolean; set: (b: boolean) => void; label: string; hint: string }> = ({ id, on, set, label, hint }) => (
  <div className="flex items-start gap-3">
    <div className="min-w-0 flex-1">
      <p id={`${id}-label`} className="text-[15px] font-semibold text-zinc-100 sm:text-meta">
        {label}
      </p>
      <p id={`${id}-hint`} className={SMALL}>
        {hint}
      </p>
    </div>
    <button type="button" role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={`${id}-hint`} onClick={() => set(!on)} className="-m-1.5 shrink-0 rounded-full p-3 kw-focus sm:p-1.5" data-switch={id}>
      <span className={`relative block h-5 w-9 rounded-full transition-colors ${on ? 'bg-sky-600' : 'bg-zinc-700'}`}>
        <span className={`absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-white transition-transform motion-reduce:transition-none ${on ? 'translate-x-4' : ''}`} />
      </span>
    </button>
  </div>
);

/** "Open until [1:00 PM]" or "Until I close it": two 44 px choices and a time field. */
const HoursPick: React.FC<{ until: string | null; set: (u: string | null) => void; last: React.MutableRefObject<string> }> = ({ until, set, last }) => {
  const opt = (on: boolean) => `inline-flex min-h-[44px] items-center gap-2 rounded-[10px] border px-3 text-[15px] kw-focus sm:min-h-[36px] sm:text-meta ${on ? 'border-sky-500 bg-sky-950 text-sky-100' : 'border-hairline-strong text-zinc-200 hover:bg-white/[0.04]'}`;
  return (
    <div className="flex flex-wrap items-center gap-2" role="radiogroup" aria-label="Open until">
      <label className={opt(until !== null)}>
        <input type="radio" className="sr-only" name="queue-until" checked={until !== null} onChange={() => set(last.current)} />
        Until
        <input
          type="time"
          value={until ?? last.current}
          aria-label="Closing time"
          onChange={(e) => {
            if (!/^\d\d:\d\d$/.test(e.target.value)) return;
            last.current = e.target.value;
            set(e.target.value);
          }}
          className="h-8 rounded-md border border-hairline bg-surface-main px-1.5 text-[15px] text-zinc-50 tabular-nums kw-focus sm:h-7 sm:text-meta"
        />
      </label>
      <label className={opt(until === null)}>
        <input type="radio" className="sr-only" name="queue-until" checked={until === null} onChange={() => set(null)} />
        Until I close it
      </label>
    </div>
  );
};

const KeepRows: React.FC = () => (
  <>
    <Row icon={<Lock size={15} aria-hidden="true" />}>Tickets stay on this Mac. A ticket is deleted when it's served, skipped or left, or when you close for the day.</Row>
    <Row icon={<ShieldCheck size={15} aria-hidden="true" />}>No phone numbers, no accounts. Customers see only their own ticket and how many are ahead.</Row>
  </>
);

const AddressRow: React.FC<{ q: QueueInfo }> = ({ q }) => {
  const url = joinUrlOf(q);
  if (!url) return null;
  const anywhere = !!q.join?.anywhere_url;
  return (
    <Row icon={anywhere ? <Globe size={15} aria-hidden="true" /> : <Wifi size={15} aria-hidden="true" />}>
      Customers use <code className="break-all font-mono text-[13px] text-zinc-100 sm:text-2xs">{shortUrl(url)}</code>
      {anywhere ? `${q.join?.anywhere_kind === 'funnel' ? ' (Tailscale Funnel)' : ''}, from their own phone data or your Wi-Fi.` : ', on your Wi-Fi only.'}
    </Row>
  );
};

// ---------------------------------------------------------------------------------------------------
// The counter sign (print only, A6): the same QR as the card, black on white.

export const CounterSign: React.FC<{ q: QueueInfo; url: string; preview?: boolean }> = ({ q, url, preview }) => (
  <div className={`kw-print-sign${preview ? ' kw-print-sign--preview' : ''}`} data-counter-sign aria-hidden={preview ? undefined : true}>
    <p className="kw-sign-name">{q.spec.title}</p>
    <p className="kw-sign-head">Take a ticket</p>
    <div className="kw-sign-qr">
      <QrImage text={url} label={`QR code to take a ticket at ${q.spec.title}`} />
    </div>
    <p className="kw-sign-how">Scan with your phone camera. We'll call your number on the page; keep it open.</p>
    {q.spec.line && <p className="kw-sign-line">{q.spec.line}</p>}
    <p className="kw-sign-foot">No app, no sign-up · {shortUrl(url)}</p>
  </div>
);

/** Print the sign: mount it in <body>, print, then take it away again. */
const usePrintSign = () => {
  const [sign, setSign] = useState<{ q: QueueInfo; url: string } | null>(null);
  useEffect(() => {
    if (!sign) return;
    const done = () => setSign(null);
    window.addEventListener('afterprint', done);
    const t = window.setTimeout(() => window.print(), 50);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener('afterprint', done);
    };
  }, [sign]);
  const node = sign && typeof document !== 'undefined' ? createPortal(<CounterSign q={sign.q} url={sign.url} />, document.body) : null;
  return { print: (q: QueueInfo, url: string) => setSign({ q, url }), node };
};

// ---------------------------------------------------------------------------------------------------
// The card

type Confirm = null | 'close' | 'end';
type Done = { kind: 'ok' | 'error'; text: string } | null;

export const QueueCard: React.FC<{ block: PocketBlock; ask: (t: string) => void }> = ({ block, ask }) => {
  const [fresh, setFresh] = useState<QueueInfo | null>(null);
  const q = fresh ?? block.queue!;
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<Done>(null);
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [closeText, setCloseText] = useState(q.close_text ?? QUEUE_CLOSE_DEFAULT);
  const [picking, setPicking] = useState(false);
  const [own, setOwn] = useState<string | null>(null);
  const [hoursEdit, setHoursEdit] = useState(false);
  const lastHour = useRef(q.hours_until ?? q.spec.hours_until ?? '13:00');
  const [hours, setHours] = useState<string | null>(q.hours_until);
  const [screenErr, setScreenErr] = useState<string | null>(null);
  const sign = usePrintSign();
  const live = block.state === 'live';
  const code = block.code;

  // The box's clock: countdowns run from `now` in the view, moved on by the browser's own seconds since.
  const skew = useMemo(() => (q.now ? Date.parse(q.now) - Date.now() : 0), [q]);
  const [, tick] = useState(0);
  const serverNow = Date.now() + skew;

  // Remove with a 5 s Undo: the row hides at once, the Mac hears about it after 5 s (or when the card goes away).
  const [hidden, setHidden] = useState<number[]>([]);
  const pending = useRef<{ n: number; t: number } | null>(null);
  const [undo, setUndo] = useState<number | null>(null);
  // Numbers this card skipped by hand (so the callout doesn't say they didn't come).
  const skippedByHost = useRef(new Map<number, string>());

  const refresh = useCallback(async () => {
    try {
      const next = queueFromRead(q, await pocketApi.read(code));
      if (next) setFresh(next);
    } catch {
      /* the card keeps what it shows */
    }
  }, [code, q]);

  const act = async (name: string, body?: Record<string, unknown>, ok?: string) => {
    setBusy(true);
    setDone(null);
    try {
      const next = queueFromRead(q, await pocketApi.queueAction(code, name, body));
      if (next) setFresh(next);
      if (ok) setDone({ kind: 'ok', text: ok });
      return true;
    } catch (e) {
      setDone({ kind: 'error', text: errorLine(e) });
      void refresh();
      return false;
    } finally {
      setBusy(false);
    }
  };

  const sendRemove = useCallback(
    async (n: number) => {
      try {
        const next = queueFromRead(q, await pocketApi.queueAction(code, 'remove', { ticket: n }));
        if (next) setFresh(next);
      } catch (e) {
        if (!(e instanceof PocketApiError && e.code === 'no_such_ticket')) {
          setHidden((h) => h.filter((x) => x !== n));
          setDone({ kind: 'error', text: `Couldn't remove #${n}: ${errorLine(e)}` });
        }
      } finally {
        setHidden((h) => h.filter((x) => x !== n));
      }
    },
    [code, q],
  );
  const flush = useCallback(() => {
    const p = pending.current;
    if (!p) return;
    window.clearTimeout(p.t);
    pending.current = null;
    setUndo(null);
    void sendRemove(p.n);
  }, [sendRemove]);
  const remove = (n: number) => {
    flush();
    setHidden((h) => [...h, n]);
    setUndo(n);
    pending.current = { n, t: window.setTimeout(flush, QUEUE_UNDO_MS) };
  };
  const undoRemove = () => {
    const p = pending.current;
    if (!p) return;
    window.clearTimeout(p.t);
    pending.current = null;
    setHidden((h) => h.filter((x) => x !== p.n));
    setUndo(null);
  };
  const flushRef = useRef(flush);
  flushRef.current = flush;
  useEffect(() => () => flushRef.current(), []);

  // Read the queue on show, then every 10 s while open or paused and the page is visible.
  const running = live && (q.state === 'open' || q.state === 'paused');
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (live) void refreshRef.current();
  }, [live]);
  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => {
      if (typeof document === 'undefined' || document.visibilityState === 'visible') void refreshRef.current();
    }, QUEUE_POLL_MS);
    return () => window.clearInterval(t);
  }, [live, running]);

  const visible = q.tickets.filter((t) => !hidden.includes(t.number));
  const called = visible.find((t) => t.state === 'called') ?? null;
  const waiting = visible.filter((t) => t.state === 'waiting');
  const skipped = visible
    .filter((t) => t.state === 'skipped' && t.recall_until && Date.parse(t.recall_until) > serverNow)
    .sort((a, b) => b.number - a.number)[0] ?? null;
  const waitingCount = Math.max(0, q.summary.waiting - hidden.filter((n) => q.tickets.some((t) => t.number === n && t.state === 'waiting')).length);
  const nextUp = waiting[0] ?? null;

  // The countdown: tick each second while a number is called; read the queue once when it runs out.
  const endsAt = called?.call_ends_at ? Date.parse(called.call_ends_at) : null;
  const left = endsAt === null ? null : endsAt - serverNow;
  useEffect(() => {
    if (!called || q.state !== 'open') return;
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [called, q.state]);
  const readAtZero = useRef<number | null>(null);
  useEffect(() => {
    if (called && left !== null && left <= 0 && live && readAtZero.current !== called.number) {
      readAtZero.current = called.number;
      void refresh();
    }
  }, [called, left, live, refresh]);

  // The main call button, and N for it while the card has focus.
  const canCall = q.state === 'open' && !busy;
  const primary = (): (() => void) | null => {
    if (!canCall) return null;
    if (called && nextUp) return () => void act('call_next', { served: called.number });
    if (called) return () => void act('serve', { ticket: called.number });
    if (nextUp) return () => void act('call_next');
    return null;
  };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key !== 'n' && e.key !== 'N') return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const el = e.target as HTMLElement;
    if (el.closest('input, textarea, select, [contenteditable="true"]')) return;
    const go = primary();
    if (!go) return;
    e.preventDefault();
    go();
  };

  const url = joinUrlOf(q);
  const bigScreen = async () => {
    if (!url) return;
    setScreenErr(null);
    const win = window.open('about:blank', '_blank');
    try {
      const link = await pocketApi.mintLink(code, 'screen');
      const to = linkUrl(url, 'screen', link.token);
      if (win) win.location.href = to;
      else window.open(to, '_blank', 'noopener');
    } catch (e) {
      win?.close();
      setScreenErr(`Couldn't open the big screen: ${e instanceof PocketApiError ? e.message : 'something went wrong.'}`);
    }
  };

  const title = q.spec.title;
  const where =
    q.state === 'setup' || q.state === 'ended'
      ? q.spec.line
      : [
          q.spec.line,
          q.state === 'closed'
            ? 'closed until you open it again'
            : q.state === 'paused'
              ? 'paused'
              : q.hours_until
                ? `open until ${hourLabel(q.hours_until)}`
                : 'open until you close it',
        ]
          .filter(Boolean)
          .join(' · ');

  // ---- buttons
  const bigScreenBtn = (cls: string) =>
    url ? (
      <button type="button" onClick={bigScreen} className={cls} data-action="big-screen">
        <Monitor size={14} aria-hidden="true" />
        Big screen
      </button>
    ) : (
      <LaterButton icon={<Monitor size={14} aria-hidden="true" />} why="The big screen works once the queue has its link." half>
        Big screen
      </LaterButton>
    );
  const printBtn = (cls: string, label = 'Print counter sign') =>
    url ? (
      <button type="button" onClick={() => sign.print(q, url)} className={cls} data-action="print-sign">
        <Printer size={14} aria-hidden="true" />
        {label}
      </button>
    ) : (
      <LaterButton icon={<Printer size={14} aria-hidden="true" />} why="The sign prints once the queue has its link." half>
        {label}
      </LaterButton>
    );
  const closeBtn = (
    <button
      type="button"
      onClick={() => {
        setCloseText(q.close_text ?? QUEUE_CLOSE_DEFAULT);
        setConfirm('close');
        setPicking(false);
      }}
      className={`${STOP} col-span-2 justify-self-center sm:ml-auto`}
      data-action="close"
    >
      <Lock size={13} aria-hidden="true" />
      Close for today
    </button>
  );

  // ---- the serving block (open and paused)
  const callCls = 'inline-flex min-h-[52px] w-full items-center justify-center gap-2 rounded-xl bg-sky-600 px-4 text-[16px] font-bold text-white shadow-[0_3px_0_theme(colors.sky.800)] transition-transform hover:bg-sky-700 active:translate-y-[2px] active:shadow-[0_1px_0_theme(colors.sky.800)] kw-focus motion-reduce:transition-none aria-disabled:cursor-not-allowed aria-disabled:bg-sky-900 aria-disabled:text-sky-300 aria-disabled:shadow-none';
  const go = primary();
  const callLabel = called ? (nextUp ? `Served · call #${nextUp.number}` : `Served #${called.number}`) : nextUp ? `Call next · #${nextUp.number}` : 'Call next';
  const skippedWhen = (t: QueueTicket) => skippedByHost.current.get(t.number) ?? (t.called_at ? new Date(Date.parse(t.called_at) + q.spec.call_minutes * 60_000).toISOString() : null);

  const serving = (
    <div className="flex flex-col gap-3 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5" data-serving={called ? 'called' : q.now_serving ? 'serving' : 'none'}>
      <div className="flex items-end justify-between gap-3">
        <div className="min-w-0">
          <div className="text-2xs font-semibold text-sky-300">{called ? 'Called' : skipped && !q.now_serving ? 'Skipped' : 'Now serving'}</div>
          <div className="flex items-baseline gap-2">
            <span className="text-[40px] font-extrabold leading-[44px] text-zinc-50 tabular-nums" data-number>
              {called ? `#${called.number}` : q.now_serving ? `#${q.now_serving}` : '—'}
            </span>
            {called?.called_at ? <span className={SMALL}>called {clock(called.called_at)}</span> : null}
            {called?.name ? <span className="truncate text-[15px] font-semibold text-zinc-200 sm:text-meta">{called.name}</span> : null}
          </div>
        </div>
        <div className="shrink-0 text-right text-[15px] font-semibold text-zinc-200 sm:text-meta" data-waiting-count>
          {called ? `${num(waitingCount)} waiting after` : waitingCount ? `${num(waitingCount)} waiting` : ''}
        </div>
      </div>
      {!called && !q.now_serving && !waitingCount ? <p className={P}>Nobody waiting yet. The first ticket is #{q.next_number}.</p> : null}
      {called && left !== null && q.state === 'open' ? (
        <div className="flex flex-col gap-1.5" data-countdown>
          <p className="flex items-center justify-between gap-2 text-[13px] leading-5 text-zinc-300 sm:text-2xs">
            <span className="flex items-center gap-1.5">
              <Clock size={13} className="shrink-0" aria-hidden="true" />
              <span role="timer" aria-live="off" aria-label={`#${called.number} has ${mmss(left)} to reach the counter, then it's skipped`}>
                has <b className="font-semibold tabular-nums text-zinc-50">{mmss(left)}</b> to reach the counter
              </span>
            </span>
            <span className="text-zinc-400">then skipped</span>
          </p>
          <Bar value={left / (q.spec.call_minutes * 60_000)} label={`Time left for #${called.number}`} />
        </div>
      ) : null}
      {q.state === 'open' ? (
        <div className="grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center">
          <button
            type="button"
            className={`${callCls} col-span-2 sm:w-auto sm:flex-1`}
            aria-disabled={go ? undefined : 'true'}
            aria-keyshortcuts="N"
            title={go ? 'Shortcut: N' : 'Nobody is waiting'}
            onClick={() => go?.()}
            data-action={called ? (nextUp ? 'served-call-next' : 'serve') : 'call-next'}
          >
            {called ? <CheckCheck size={18} aria-hidden="true" /> : <Megaphone size={18} aria-hidden="true" />}
            {callLabel}
          </button>
          {called && nextUp ? (
            <button type="button" disabled={busy} onClick={() => void act('serve', { ticket: called.number })} className={HALF_BTN} data-action="serve">
              <Check size={14} aria-hidden="true" />
              Served
            </button>
          ) : null}
          {called ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                skippedByHost.current.set(called.number, new Date(serverNow).toISOString());
                void act('skip', { ticket: called.number });
              }}
              className={`${HALF_BTN} ${nextUp ? '' : 'col-span-2 sm:col-span-1'}`}
              data-action="skip"
            >
              <SkipForward size={14} aria-hidden="true" />
              Skip #{called.number}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );

  const skippedCallout = skipped ? (
    <Callout tone="warn">
      <div className="flex flex-col gap-2.5 sm:flex-row sm:items-center" data-skipped={skipped.number}>
        <div className="min-w-0 flex-1">
          <p>
            {skippedByHost.current.has(skipped.number)
              ? `You skipped #${skipped.number}. Their page says so and offers a new ticket.`
              : `#${skipped.number} didn't come in ${q.spec.call_minutes} minute${q.spec.call_minutes === 1 ? '' : 's'}, so it was skipped. Their page says so and offers a new ticket.`}
          </p>
          <p className={`${SMALL} mt-1`}>
            {skippedWhen(skipped) ? `Skipped ${clock(skippedWhen(skipped))}. ` : ''}You can bring it back until {clock(skipped.recall_until)}. It goes to the front, called again.
          </p>
        </div>
        <button type="button" disabled={busy || q.state !== 'open'} onClick={() => void act('recall', { ticket: skipped.number })} className={`${HALF_BTN} shrink-0`} data-action="recall">
          <RotateCcw size={14} aria-hidden="true" />
          Bring back #{skipped.number}
        </button>
      </div>
    </Callout>
  ) : null;

  // ---- next 3
  const shown = waiting.slice(0, 3);
  const more = waiting.slice(3);
  const nextList = waiting.length ? (
    <div className="flex flex-col gap-1.5" data-next>
      <div className="flex items-baseline justify-between text-2xs font-bold text-zinc-300">
        <span>
          Next {shown.length} · {num(waitingCount)} waiting
        </span>
        <span className="font-medium text-zinc-400">oldest first</span>
      </div>
      <ul className="flex flex-col overflow-hidden rounded-xl border border-hairline-strong" aria-label="Next in the queue">
        {shown.map((t) => {
          const m = minsSince(t.taken_at, serverNow);
          return (
            <li key={t.number} className="flex min-h-[52px] items-center gap-3 border-b border-hairline px-3 last:border-b-0 sm:min-h-[44px]" data-ticket={t.number}>
              <GuestCreature avatar={ticketCreature(t.number)} size={24} />
              <span className="w-11 shrink-0 text-[17px] font-extrabold text-zinc-50 tabular-nums sm:text-[15px]">#{t.number}</span>
              <span className="min-w-0 flex-1 truncate text-[14px] text-zinc-300 sm:text-meta">
                {t.name ? <b className="font-semibold text-zinc-100">{t.name}</b> : null}
                {t.name ? ' · ' : ''}
                {t.taken_at ? (
                  <>
                    <span className="hidden sm:inline">took a ticket </span>at {clock(t.taken_at)}
                  </>
                ) : (
                  'waiting'
                )}
              </span>
              <span className="shrink-0 text-[14px] font-semibold text-zinc-200 tabular-nums sm:text-meta">{m === null ? '' : `${m} min`}</span>
              <button
                type="button"
                onClick={() => remove(t.number)}
                className="-mr-1.5 flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-zinc-400 hover:bg-white/[0.05] hover:text-zinc-100 kw-focus sm:h-8 sm:w-8"
                aria-label={`Remove #${t.number} from the queue`}
                data-action="remove"
              >
                <X size={16} aria-hidden="true" />
              </button>
            </li>
          );
        })}
      </ul>
      {more.length ? (
        <p className={SMALL} data-more>
          + {num(more.length)} more: {more.slice(0, 6).map((t) => `#${t.number}`).join(', ')}
          {more.length > 6 ? ', …' : ''}
        </p>
      ) : null}
    </div>
  ) : null;

  const undoBar =
    undo !== null ? (
      <div className="flex items-center gap-3 rounded-xl border border-hairline-strong bg-surface-rail/80 px-3 py-1.5" role="status" data-undo={undo}>
        <Trash2 size={14} className="shrink-0 text-zinc-400" aria-hidden="true" />
        <span className={`${P} flex-1`}>#{undo} removed from the queue.</span>
        <button type="button" onClick={undoRemove} className="inline-flex min-h-[44px] items-center gap-1.5 rounded-lg px-2.5 text-[15px] font-semibold text-sky-200 hover:bg-white/[0.04] kw-focus sm:min-h-[32px] sm:text-meta" data-action="undo">
          <Undo2 size={14} aria-hidden="true" />
          Undo
        </button>
      </div>
    ) : null;

  // ---- join: the full QR box while nobody has come yet, a compact row after (hidden on a phone)
  const fresh0 = q.state === 'open' && !q.summary.served_today && !q.tickets.length && q.now_serving === null;
  const join = url ? (
    fresh0 ? (
      <div className="flex flex-col gap-4 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5 sm:flex-row" data-join="full">
        <Qr url={url} size="full" name={title} />
        <div className="flex min-w-0 flex-1 flex-col gap-2.5">
          <div className="text-[13px] font-semibold text-zinc-200 sm:text-meta">Customers scan this to take a ticket</div>
          <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">Put the counter sign by the till. Each phone gets one ticket; nobody signs up.</p>
          <div className="grid grid-cols-2 gap-2 sm:flex sm:flex-wrap">
            {printBtn(HALF_BTN)}
            {bigScreenBtn(HALF_BTN)}
          </div>
          <LinkField url={url} />
        </div>
      </div>
    ) : (
      <div className="hidden items-center gap-3 rounded-xl border border-hairline bg-surface-rail/40 p-2.5 sm:flex" data-join="compact">
        <div className="w-14 shrink-0 rounded-md bg-white p-1">
          <QrImage text={url} label={`QR code to take a ticket at ${title}`} />
        </div>
        <p className={`${P} flex-1`}>Customers scan the counter sign to join.</p>
        {printBtn(HALF_BTN, 'Print sign')}
        <CopyLink url={url} />
      </div>
    )
  ) : null;

  // ---- set-up form
  const [name, setName] = useState(title);
  const [line, setLine] = useState(q.spec.line ?? '');
  const [until, setUntil] = useState<string | null>(q.spec.hours_until ?? '13:00');
  const setupHour = useRef(q.spec.hours_until ?? '13:00');
  const [callMin, setCallMin] = useState(q.spec.call_minutes);
  const [askName, setAskName] = useState(q.spec.ask_name);
  const FIELD = 'w-full rounded-[10px] border border-hairline-strong bg-surface-main px-3 py-2 text-[16px] text-zinc-50 placeholder:text-zinc-500 kw-focus sm:text-body';
  const LABEL = 'flex items-baseline justify-between gap-2 text-[15px] font-semibold text-zinc-100 sm:text-meta';
  const today = q.now ? new Date(q.now).toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' }) : 'today';
  const setupBody = (
    <div className="flex flex-col gap-4" data-setup>
      <label className="flex flex-col gap-1.5">
        <span className={LABEL}>
          Name <span className="text-2xs font-medium text-zinc-400">on the ticket and the counter sign</span>
        </span>
        <input className={FIELD} value={name} maxLength={80} onChange={(e) => setName(e.target.value)} />
      </label>
      <label className="flex flex-col gap-1.5">
        <span className={LABEL}>
          A line for customers <span className="text-2xs font-medium text-zinc-400">optional · up to 80 characters</span>
        </span>
        <input className={FIELD} value={line} maxLength={QUEUE_LINE_MAX} placeholder="Pick up at the side counter" onChange={(e) => setLine(e.target.value)} />
      </label>
      <div className="flex flex-col gap-1.5">
        <span className={LABEL}>
          Open <span className="text-2xs font-medium text-zinc-400">today, {today}</span>
        </span>
        <HoursPick until={until} set={setUntil} last={setupHour} />
      </div>
      <div className="flex flex-col gap-1.5">
        <span className={LABEL}>Call time</span>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
          <CallStepper value={callMin} onChange={setCallMin} />
          <p className={SMALL}>
            A called number has {callMin} minute{callMin === 1 ? '' : 's'} to reach the counter, then it's skipped. You can bring it back.
          </p>
        </div>
      </div>
      <Switch id={`queue-name-${code}`} on={askName} set={setAskName} label="Ask for a first name" hint={askName ? 'On: customers type a first name; you see it next to their number.' : 'Off: numbers only. Turn on to see a first name next to each number.'} />
      <ul className="flex flex-col gap-2">
        <KeepRows />
        <AddressRow q={q} />
      </ul>
    </div>
  );
  const openAsk = () =>
    ask(
      `Open ${name.trim() || title}${line.trim() ? ` with the line "${line.trim()}"` : ''}, ${until ? `open until ${hourLabel(until)} today` : 'open until I close it'}, ${callMin} minute${callMin === 1 ? '' : 's'} to reach the counter, ${askName ? 'asking for a first name' : 'numbers only'}.`,
    );

  // ---- pause
  const pauseWith = async (text: string) => {
    setPicking(false);
    setOwn(null);
    if (q.state === 'paused') {
      if (!(await act('resume'))) return;
    }
    await act('pause', { text });
  };
  const presetCls = (on: boolean) =>
    `inline-flex min-h-[44px] items-center justify-center gap-1.5 rounded-[10px] border px-3 text-[15px] kw-focus sm:min-h-[34px] sm:text-meta ${on ? 'border-amber-300/60 bg-amber-300/[0.08] font-semibold text-zinc-50' : 'border-hairline-strong text-zinc-200 hover:bg-white/[0.04]'}`;
  const presets = (
    <div className="flex flex-col gap-2" data-presets>
      <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap" role="group" aria-label="What customers read while you're paused">
        {QUEUE_PAUSE_PRESETS.map((p) => (
          <button key={p} type="button" disabled={busy} aria-pressed={q.state === 'paused' ? q.pause_text === p : undefined} onClick={() => void pauseWith(p)} className={presetCls(q.state === 'paused' && q.pause_text === p)} data-preset={p}>
            {p}
          </button>
        ))}
        <button
          type="button"
          disabled={busy}
          aria-expanded={own !== null}
          aria-pressed={q.state === 'paused' ? !!q.pause_text && !QUEUE_PAUSE_PRESETS.includes(q.pause_text as (typeof QUEUE_PAUSE_PRESETS)[number]) : undefined}
          onClick={() => setOwn(own === null ? (q.pause_text && !QUEUE_PAUSE_PRESETS.includes(q.pause_text as (typeof QUEUE_PAUSE_PRESETS)[number]) ? q.pause_text : '') : null)}
          className={presetCls(q.state === 'paused' && !!q.pause_text && !QUEUE_PAUSE_PRESETS.includes(q.pause_text as (typeof QUEUE_PAUSE_PRESETS)[number]))}
          data-preset="own"
        >
          <Pencil size={13} aria-hidden="true" />
          Write your own
        </button>
      </div>
      {own !== null ? (
        <form
          className="flex flex-col gap-2 sm:flex-row sm:items-center"
          onSubmit={(e) => {
            e.preventDefault();
            if (own.trim()) void pauseWith(own.trim());
          }}
        >
          <input className={`${FIELD} sm:flex-1`} value={own} maxLength={QUEUE_PAUSE_MAX} autoFocus aria-label="Your pause message" placeholder="Back at 2 PM" onChange={(e) => setOwn(e.target.value)} />
          <button type="submit" disabled={busy || !own.trim()} className={`${PRIMARY} px-4`} data-action="pause-own">
            <Pause size={14} aria-hidden="true" />
            {q.state === 'paused' ? 'Show this' : 'Pause'}
          </button>
        </form>
      ) : null}
    </div>
  );

  // ---- body per state
  let body: React.ReactNode;
  let actions: React.ReactNode = null;
  if (q.state === 'setup') {
    body = setupBody;
    actions = (
      <div className={GRID}>
        <button type="button" onClick={openAsk} disabled={!name.trim()} className={`${PRIMARY} ${FULL}`} data-action="open-queue">
          <Ticket size={14} aria-hidden="true" />
          Open the queue
        </button>
        {url ? (
          <a href={url} target="_blank" rel="noreferrer" className={HALF_BTN} data-action="try">
            <Eye size={14} aria-hidden="true" />
            Try as a customer
          </a>
        ) : (
          <LaterButton icon={<Eye size={14} aria-hidden="true" />} why="The customer page opens once the queue is open." half>
            Try as a customer
          </LaterButton>
        )}
        <button type="button" onClick={() => ask(`Discard the ${title} queue set-up.`)} className={`${STOP} sm:ml-auto`} data-action="discard">
          <Trash2 size={13} aria-hidden="true" />
          Discard
        </button>
      </div>
    );
  } else if (q.state === 'open') {
    body = (
      <>
        {serving}
        {skippedCallout}
        {undoBar}
        {nextList}
        {fresh0 ? null : <Tiles q={q} />}
        {join}
        {fresh0 ? (
          <ul className="flex flex-col gap-2">
            <KeepRows />
          </ul>
        ) : null}
        {picking ? presets : null}
      </>
    );
    actions = (
      <div className={GRID}>
        <button type="button" onClick={() => setPicking(!picking)} aria-expanded={picking} className={HALF_BTN} data-action="pause">
          <Pause size={14} aria-hidden="true" />
          Pause
        </button>
        {bigScreenBtn(HALF_BTN)}
        {closeBtn}
      </div>
    );
  } else if (q.state === 'paused') {
    body = (
      <>
        <div className="flex flex-col gap-2 rounded-2xl border border-amber-300/35 bg-amber-300/[0.05] p-3.5" data-paused>
          <div className="text-2xs font-semibold text-warn">Paused{q.paused_at ? ` since ${clock(q.paused_at)}` : ''}</div>
          <p className="text-[22px] font-extrabold leading-7 text-zinc-50" data-pause-text>
            {q.pause_text ?? 'Paused'}
          </p>
          <p className={P}>
            Customers read this. {waitingCount ? `The ${people(waitingCount)} waiting keep their place; ` : 'Nobody is waiting; '}new tickets wait until you resume.
          </p>
          {presets}
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              if ((await act('resume')) && nextUp && !called) await act('call_next');
            }}
            className={`${callCls} mt-1`}
            data-action="resume"
          >
            <Play size={18} aria-hidden="true" />
            {nextUp && !called ? `Resume · call #${nextUp.number} next` : 'Resume'}
          </button>
        </div>
        {undoBar}
        {nextList}
        <Tiles q={q} />
      </>
    );
    actions = (
      <div className={GRID}>
        {bigScreenBtn(HALF_BTN)}
        {closeBtn}
      </div>
    );
  } else if (q.state === 'closed') {
    const closedLine = q.auto_closed
      ? `Closed by itself at ${q.hours_until ? hourLabel(q.hours_until) : clockAmPm(q.closed_at)}, as set.`
      : `You closed it${q.closed_at ? ` at ${clockAmPm(q.closed_at)}` : ''}.`;
    body = (
      <>
        <Tiles q={q} />
        <ul className="flex flex-col gap-2">
          <Row icon={<Lock size={15} aria-hidden="true" />}>
            {closedLine}
            {q.closed_waiting ? ` ${q.closed_waiting === 1 ? '1 person was' : `${num(q.closed_waiting)} people were`} still waiting and saw your closing message.` : ''}
          </Row>
          <Row icon={<ShieldCheck size={15} aria-hidden="true" />} muted>
            All tickets are deleted. Only these counts stay.
          </Row>
        </ul>
        {hoursEdit ? (
          <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-rail/60 p-3" role="group" aria-label="Change hours" data-hours-edit>
            <p className={P}>When should it close by itself next time?</p>
            <HoursPick until={hours} set={setHours} last={lastHour} />
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                disabled={busy}
                onClick={async () => {
                  if (await act('set_hours', { until: hours }, hours ? `It closes at ${hourLabel(hours)} from now on.` : 'It stays open until you close it.')) setHoursEdit(false);
                }}
                className={PRIMARY}
                data-action="save-hours"
              >
                Save hours
              </button>
              <button type="button" onClick={() => setHoursEdit(false)} className={GHOST}>
                Cancel
              </button>
            </div>
          </div>
        ) : null}
      </>
    );
    actions = (
      <div className={GRID}>
        <button type="button" disabled={busy} onClick={() => void act('reopen', undefined, 'Open again. The next ticket is #1.')} className={`${PRIMARY} ${FULL}`} data-action="reopen">
          <Ticket size={14} aria-hidden="true" />
          Open again · starts at #1
        </button>
        <button
          type="button"
          onClick={() => {
            setHours(q.hours_until);
            setHoursEdit(!hoursEdit);
          }}
          aria-expanded={hoursEdit}
          className={HALF_BTN}
          data-action="change-hours"
        >
          <Clock size={14} aria-hidden="true" />
          Change hours
        </button>
        <button type="button" onClick={() => setConfirm('end')} className={`${STOP} sm:ml-auto`} data-action="end">
          <Square size={13} aria-hidden="true" />
          End this queue
        </button>
      </div>
    );
  } else {
    body = (
      <>
        <Tiles q={q} />
        <ul className="flex flex-col gap-2">
          <Row icon={<ShieldCheck size={15} aria-hidden="true" />}>The link and the counter sign no longer work. Nothing about customers was kept.</Row>
        </ul>
      </>
    );
    actions = (
      <div className={GRID}>
        <button type="button" onClick={() => ask(`Set up a new queue like ${title}.`)} className={`${PRIMARY} ${FULL}`} data-action="new-like-this">
          <Ticket size={14} aria-hidden="true" />
          Set up a new queue like this
        </button>
      </div>
    );
  }

  const confirmBox =
    confirm === 'close' ? (
      <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-rail/60 p-3" role="group" aria-label="Close for today?" data-confirm="close">
        <p className={P}>
          <b className="font-semibold text-zinc-50">Close for today?</b>{' '}
          {waitingCount ? `${people(waitingCount)} ${waitingCount === 1 ? 'is' : 'are'} still waiting. Their page shows your message and their tickets are deleted.` : 'Nobody is waiting. Anyone who opens the page sees your message.'}
        </p>
        <label className="flex flex-col gap-1.5">
          <span className="flex items-baseline justify-between text-2xs font-bold text-zinc-300">
            What customers read
            <span className="font-medium text-zinc-400 tabular-nums">
              {closeText.length} / {QUEUE_CLOSE_MAX}
            </span>
          </span>
          <textarea className={`${FIELD} min-h-[72px] resize-y`} value={closeText} maxLength={QUEUE_CLOSE_MAX} onChange={(e) => setCloseText(e.target.value)} />
        </label>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              if (await act('close', { text: closeText.trim() || QUEUE_CLOSE_DEFAULT })) setConfirm(null);
            }}
            className={STOP}
            data-action="confirm-close"
          >
            <Lock size={13} aria-hidden="true" />
            Close and tell them
          </button>
          <button type="button" disabled={busy} onClick={() => setConfirm(null)} className={GHOST}>
            Keep open
          </button>
        </div>
      </div>
    ) : confirm === 'end' ? (
      <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-rail/60 p-3" role="group" aria-label="End this queue?" data-confirm="end">
        <p className={P}>End {title}? The link and the counter sign stop working for good. The counts stay here.</p>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await pocketApi.end(code);
                setFresh({ ...q, state: 'ended', ended_at: new Date(serverNow).toISOString() });
                setConfirm(null);
                setDone({ kind: 'ok', text: `${title} has ended.` });
              } catch (e) {
                setDone({ kind: 'error', text: errorLine(e) });
              } finally {
                setBusy(false);
              }
            }}
            className={STOP}
            data-action="confirm-end"
          >
            End this queue
          </button>
          <button type="button" disabled={busy} onClick={() => setConfirm(null)} className={GHOST}>
            Keep it
          </button>
        </div>
      </div>
    ) : null;

  const foot =
    q.state === 'setup'
      ? 'No model calls after set-up. Calling, tickets and the big screen run on this Mac, past your daily limit.'
      : q.state === 'ended'
        ? 'No model calls were used to run it.'
        : 'No model calls: calling, tickets and the big screen run on this Mac, past your daily limit.';

  return (
    <section
      className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card outline-none sm:p-[18px]"
      data-pocket={block.state}
      data-queue={q.state}
      data-mood={queueMood(q)}
      aria-label={`${title}, pocket app Queue ticket`}
      tabIndex={-1}
      onKeyDown={onKey}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950 text-sky-200" aria-hidden="true">
          <Ticket size={22} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">Pocket app · Queue ticket</div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{q.state === 'setup' ? name.trim() || title : title}</h3>
          {where ? <p className="text-[15px] text-zinc-300 sm:text-[13px]">{where}</p> : null}
          <span className="mt-1.5 block sm:hidden">
            <QueueChip q={q} called={called} />
          </span>
        </div>
        <span className="hidden shrink-0 sm:block">
          <QueueChip q={q} called={called} />
        </span>
      </header>
      <div className="relative flex flex-col gap-4">
        {body}
        {screenErr ? (
          <p role="status" className="flex items-start gap-2 text-[15px] text-bad sm:text-[13px]">
            <AlertTriangle size={15} className={ICON} aria-hidden="true" />
            {screenErr}
          </p>
        ) : null}
        {done ? (
          <p role="status" className={`flex items-start gap-2 text-[15px] sm:text-[13px] ${done.kind === 'ok' ? 'text-ok' : 'text-bad'}`} data-done={done.kind}>
            {done.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
            {done.text}
          </p>
        ) : null}
        {confirmBox ?? actions}
      </div>
      <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]" data-budget>
        <Zap size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
        <span>{foot}</span>
      </footer>
      {sign.node}
    </section>
  );
};

