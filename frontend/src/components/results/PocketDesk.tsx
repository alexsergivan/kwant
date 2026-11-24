// Pocket apps (pa13): the front desk (Guest Guide) Pocket card in chat, its Allow card and its Needs-you item
// (pa13/design.md §1, §3, §4, "For Pixel" 1-10; Lin's API in pa13/impl.md wins where they differ).
// States: 1 set-up, 2 needs your own address, 3 ready, 4 live, 5 a guest's chat, 6 answer a hand-off, 7 answered,
// 8 daily limit reached, 9 stopped, plus the Inbox item (10) and the Done row. A guest's words show only here and in
// the host's Inbox item, always as plain text in a bubble or a labelled quote; the answering bot's id is never kept.
// No holo: answering a guest is the job, not a result a check earned. Hand-off, limits, Stopped and errors stay plain.

import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  BellRing,
  Check,
  ChevronLeft,
  ConciergeBell,
  Eye,
  FileText,
  Globe,
  Lock,
  MessageSquare,
  Minus,
  Pencil,
  Plus,
  Reply,
  RotateCcw,
  Send,
  Share2,
  ShieldCheck,
  Square,
  Trash2,
  Undo2,
  Wifi,
  X,
  Zap,
} from 'lucide-react';
import type { DeskFrom, DeskGuest, DeskInfo, DeskMessage, PocketBlock } from '../../smartResults';
import { isIndex, isObj, isText, optText } from '../../smartResults';
import { ModernAvatar } from '../ModernAvatar';
import type { Mood } from '../../kwantMood';
import { pocketApi, PocketApiError, type PocketRead } from '../../pocketApi';
import { Bar, Callout, GHOST, HALF, ICON, LATER_CLS, LaterButton, LinkField, PRIMARY, Qr, Row, STOP } from './pocketParts';
import { GuestCreature, HowTo, NoQuickLink, portOf, validSantaJoin } from './PocketSanta';

// ---------------------------------------------------------------------------------------------------
// Validation. A guest's words may hold anything a guest typed ("<b>hi</b>" included): they are kept as plain
// strings with a length cap and only ever rendered as text, so they skip isText's no-HTML rule.

export const DESK_CAP_MIN = 10;
export const DESK_CAP_MAX = 200;
export const DESK_CAP_STEP = 10;
const MAX_GUESTS = 60;
const MAX_MESSAGES = 200;
const FROM: DeskFrom[] = ['guest', 'bot', 'host', 'note'];
const NOTES = ['handoff', 'host_away', 'daily_cap'] as const;
const isCount = (v: unknown): v is number => isIndex(v) && v <= 1_000_000;
const words = (v: unknown, max: number): v is string => typeof v === 'string' && v.length <= max;
const optCount = (v: unknown) => (v === undefined || v === null ? null : isCount(v) ? v : undefined);
const optWords = (v: unknown, max: number) => (v === undefined || v === null || v === '' ? null : words(v, max) ? v : undefined);
const creature = (v: unknown) => (v === undefined || v === null || v === '' ? null : typeof v === 'string' && /^[a-z0-9_-]{1,32}$/.test(v) ? v : undefined);
const guestId = (v: unknown): v is string => typeof v === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(v);
const DAY = /^\d{4}-\d{2}-\d{2}$/;
const optDay = (v: unknown) => (v === undefined || v === null ? null : typeof v === 'string' && (DAY.test(v) || isText(v, 40)) ? v : undefined);
const optTime = (v: unknown) => optText(v, 60);

function validLast(v: unknown): DeskGuest['last'] | undefined {
  if (v === undefined || v === null) return null;
  if (!isObj(v) || !FROM.includes(v.from as DeskFrom) || !words(v.text, 400) || !isText(v.at, 60)) return undefined;
  return { from: v.from as DeskFrom, text: v.text, at: v.at };
}

export function validDeskGuest(v: unknown): DeskGuest | undefined {
  if (!isObj(v) || !guestId(v.id) || !isText(v.name, 40) || typeof v.waiting !== 'boolean') return undefined;
  const c = creature(v.creature);
  const reason = optWords(v.handoff_reason, 200);
  const last = validLast(v.last);
  const unread = v.unread === undefined || v.unread === null ? 0 : isCount(v.unread) ? v.unread : undefined;
  const msgs = v.msgs === undefined || v.msgs === null ? 0 : isCount(v.msgs) ? v.msgs : undefined;
  const del = optDay(v.delete_on);
  if ([c, reason, last, unread, msgs, del].includes(undefined)) return undefined;
  return { id: v.id, name: v.name, creature: c!, waiting: v.waiting, handoff_reason: v.waiting ? reason! : null, last: last!, unread: unread!, msgs: msgs!, delete_on: del! };
}

export function validDeskMessage(v: unknown): DeskMessage | undefined {
  if (!isObj(v) || typeof v.id !== 'string' || v.id.length > 40 || !FROM.includes(v.from as DeskFrom) || !words(v.text, 4000) || !isText(v.at, 60)) return undefined;
  const note = v.note === undefined || v.note === null ? null : NOTES.includes(v.note as (typeof NOTES)[number]) ? (v.note as DeskMessage['note']) : undefined;
  if (note === undefined) return undefined;
  // A note carries no words of its own (impl.md: `text: ""`); one without a known kind reads as a hand-off.
  return { id: v.id, from: v.from as DeskFrom, text: v.from === 'note' ? '' : v.text, at: v.at, note: v.from === 'note' ? note ?? 'handoff' : null };
}

/** One guest's conversation (`read.conversation`); null when it's missing or malformed. */
export function validConversation(v: unknown): { guest: DeskGuest; messages: DeskMessage[] } | null {
  if (!isObj(v) || !isObj(v.guest) || !Array.isArray(v.messages) || v.messages.length > MAX_MESSAGES) return null;
  const guest = validDeskGuest({ msgs: v.messages.length, ...v.guest });
  const messages = v.messages.map(validDeskMessage);
  if (!guest || messages.some((m) => !m)) return null;
  return { guest, messages: messages as DeskMessage[] };
}

const START_ERRORS = new Set(['no_quick_tunnel', 'needs_stable_address']);

/**
 * block.desk on the wire: Lin's host summary plus the face (`title`, `place`, `host_name`) and the desk's own `join`.
 * Missing counts fall back to 0 and the cap to 50; a wrong type anywhere rejects the block. `answer_bot.id` is read
 * past and never kept: only its name and model reach the card.
 */
export function validDesk(v: unknown, fallbackTitle?: string): DeskInfo | undefined {
  if (!isObj(v)) return undefined;
  const title = v.title === undefined || v.title === null ? fallbackTitle : v.title;
  if (!isText(title, 80)) return undefined;
  const place = optText(v.place, 80);
  const host = optText(v.host_name, 40);
  const c = v.counts === undefined || v.counts === null ? {} : v.counts;
  if (!isObj(c)) return undefined;
  const count = (k: string, d: number) => (c[k] === undefined || c[k] === null ? d : isCount(c[k]) ? (c[k] as number) : undefined);
  const counts = { guests: count('guests', 0), waiting: count('waiting', 0), replies_today: count('replies_today', 0), daily_cap: count('daily_cap', 50), messages_today: count('messages_today', 0) };
  if (Object.values(counts).includes(undefined)) return undefined;
  if (counts.daily_cap! < DESK_CAP_MIN || counts.daily_cap! > DESK_CAP_MAX) return undefined;
  const ab = v.answer_bot === undefined || v.answer_bot === null ? v.bot : v.answer_bot;
  if (!isObj(ab) || !isText(ab.name, 40)) return undefined;
  const model = optText(ab.model, 60);
  const cost = v.cost_per_reply_usd === undefined || v.cost_per_reply_usd === null ? null : typeof v.cost_per_reply_usd === 'number' && v.cost_per_reply_usd >= 0 && v.cost_per_reply_usd < 10 ? v.cost_per_reply_usd : undefined;
  const chars = optCount(v.guide_chars);
  const facts = optCount(v.guide_facts);
  const preview = optWords(v.guide_preview, 600);
  const resets = optTime(v.cap_resets_at);
  if (!Array.isArray(v.guests ?? []) || ((v.guests as unknown[] | undefined)?.length ?? 0) > MAX_GUESTS) return undefined;
  const guests = ((v.guests as unknown[] | undefined) ?? []).map(validDeskGuest);
  if (guests.some((g) => !g) || new Set(guests.map((g) => g!.id)).size !== guests.length) return undefined;
  const join = v.join === undefined || v.join === null ? null : validSantaJoin(v.join);
  const startError = v.start_error === undefined || v.start_error === null ? null : typeof v.start_error === 'string' ? v.start_error : undefined;
  let ended: DeskInfo['ended'] | undefined = null;
  if (v.ended !== undefined && v.ended !== null) {
    const e = v.ended;
    if (!isObj(e)) return undefined;
    const parts = { at: optTime(e.at), deletes_on: optDay(e.deletes_on), days: optCount(e.days), bot_replies: optCount(e.bot_replies), host_replies: optCount(e.host_replies), messages: optCount(e.messages) };
    ended = Object.values(parts).includes(undefined) ? undefined : (parts as NonNullable<DeskInfo['ended']>);
  }
  if ([place, host, model, cost, chars, facts, preview, resets, join, startError, ended].includes(undefined)) return undefined;
  // More waiting than guests, or a list that disagrees with its own counts, is junk.
  if (counts.waiting! > counts.guests! && guests.length > 0) return undefined;
  return {
    title,
    place: place!,
    host_name: host!,
    bot: { name: ab.name, model: model! },
    counts: counts as DeskInfo['counts'],
    cap_resets_at: resets!,
    cost_per_reply_usd: cost!,
    guide_chars: chars!,
    guide_facts: facts!,
    guide_preview: preview!,
    guests: guests as DeskGuest[],
    join: join!,
    needs_address: !!startError && START_ERRORS.has(startError),
    ended: ended!,
  };
}

/** A fresh read (`GET /api/pockets/:code` or an action's answer) laid over the card's desk. Null: keep what we had. */
export function deskFromRead(prev: DeskInfo, read: unknown): { desk: DeskInfo; ended: boolean } | null {
  if (!isObj(read) || !isObj(read.summary)) return null;
  const s = read.summary as Record<string, unknown>;
  const pocket = isObj(read.pocket) ? read.pocket : {};
  const desk = validDesk({ ...prevWire(prev), ...s, join: read.join === undefined ? prevWire(prev).join : read.join }, prev.title);
  if (!desk) return null;
  const ended = pocket.state === 'ended' || pocket.state === 'closed';
  return { desk, ended };
}
const prevWire = (d: DeskInfo) => ({ title: d.title, place: d.place, host_name: d.host_name, answer_bot: d.bot, guide_preview: d.guide_preview, join: d.join, ended: d.ended });

// ---------------------------------------------------------------------------------------------------
// The Needs-you item (`GET /api/waiting`, kind pocket_desk): one per waiting guest.

export interface DeskItem {
  id: string;
  pocket_code: string;
  pocket_title: string;
  guest_id: string;
  guest_name: string;
  /** The hand-off reason or a short label: the Allow card's title. */
  text: string;
  /** The guest's last words, up to 200 characters, host only. */
  quote: string | null;
  asked_at: string;
  /** The spec's bot name ("Bell"), even after the bot was deleted. */
  kwant_name: string;
  thread_id: string | null;
}

export function validDeskItems(body: unknown): DeskItem[] {
  const items = isObj(body) && Array.isArray(body.items) ? body.items : [];
  const out: DeskItem[] = [];
  const seen = new Set<string>();
  for (const it of items) {
    if (!isObj(it) || it.kind !== 'pocket_desk') continue;
    const code = it.pocket_code;
    if (typeof code !== 'string' || !/^[A-Za-z0-9_-]{3,32}$/.test(code) || !guestId(it.guest_id) || !isText(it.guest_name, 40) || !isText(it.pocket_title, 80)) continue;
    if (!words(it.text, 200) || !isText(it.asked_at, 60) || !isText(it.kwant_name, 40)) continue;
    const id = `desk:${code}:${it.guest_id}`;
    // One item per waiting guest, never one per message.
    if (seen.has(id)) continue;
    seen.add(id);
    out.push({
      id,
      pocket_code: code,
      pocket_title: it.pocket_title,
      guest_id: it.guest_id,
      guest_name: it.guest_name,
      text: it.text.trim() || `${it.kwant_name} asked you to reply`,
      quote: words(it.quote, 400) && it.quote.trim() ? it.quote.slice(0, 200) : null,
      asked_at: it.asked_at,
      kwant_name: it.kwant_name,
      thread_id: typeof it.thread_id === 'string' && it.thread_id ? it.thread_id : null,
    });
  }
  return out;
}

// ---------------------------------------------------------------------------------------------------
// Answers: one store for the card and the Inbox, so answering in either place closes both. Yes / Sorry are held
// for 5 s with Undo, then sent; leaving the screen sends at once. Every answer is optimistic and rolls back on failure.

export const DESK_UNDO_MS = 5000;
export const DESK_YES = 'Yes, that’s fine. Enjoy your last morning!';
export const DESK_SORRY = 'Sorry, that’s not possible this time.';
export const DESK_FAILED = 'Couldn’t reach the box. Try again';

export type DeskAnswer = {
  phase: 'holding' | 'sending' | 'sent' | 'failed';
  kind: 'reply' | 'resolve';
  /** The host's words (reply); null for "No reply needed". */
  text: string | null;
  /** Date.now() when the host answered: a hand-off asked after this is a new one. */
  at: number;
};

const answers = new Map<string, DeskAnswer>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();
const pending = new Map<string, { code: string; guest: string }>();
const listeners = new Set<() => void>();
const readListeners = new Set<(code: string, read: PocketRead) => void>();
let holdMs = DESK_UNDO_MS;
let version = 0;

export const deskKey = (code: string, guest: string) => `${code}:${guest}`;
const emit = () => {
  version++;
  listeners.forEach((f) => f());
};

/** True while the host's answer is held, on its way or sent: the guest no longer waits in this app. */
export const isAnswered = (a: DeskAnswer | undefined, askedAt?: number | null) => !!a && a.phase !== 'failed' && (askedAt === undefined || askedAt === null || askedAt <= a.at);

async function send(k: string) {
  const a = answers.get(k);
  const p = pending.get(k);
  if (!a || !p) return;
  const sending: DeskAnswer = { ...a, phase: 'sending' };
  answers.set(k, sending);
  emit();
  // A newer answer (or a reset) since this one left wins: a late reply never overwrites it.
  const still = () => answers.get(k) === sending;
  try {
    const read = a.kind === 'reply' ? await pocketApi.deskReply(p.code, p.guest, a.text ?? '') : await pocketApi.deskResolve(p.code, p.guest);
    if (!still()) return;
    answers.set(k, { ...a, phase: 'sent' });
    if (read) readListeners.forEach((f) => f(p.code, read));
  } catch {
    if (!still()) return;
    answers.set(k, { ...a, phase: 'failed' });
  }
  emit();
}

export const deskAnswers = {
  get: (k: string) => answers.get(k),
  version: () => version,
  subscribe(f: () => void) {
    listeners.add(f);
    return () => void listeners.delete(f);
  },
  /** Fresh reads that came back from an answer, for the card that shows that pocket. */
  onRead(f: (code: string, read: PocketRead) => void) {
    readListeners.add(f);
    return () => void readListeners.delete(f);
  },
  /** Yes / Sorry: held for 5 s with Undo, then sent. */
  hold(code: string, guest: string, text: string) {
    const k = deskKey(code, guest);
    deskAnswers.undo(k, false);
    pending.set(k, { code, guest });
    answers.set(k, { phase: 'holding', kind: 'reply', text, at: Date.now() });
    timers.set(
      k,
      setTimeout(() => {
        timers.delete(k);
        void send(k);
      }, holdMs),
    );
    emit();
  },
  /** A written reply or "No reply needed": sent now. */
  now(code: string, guest: string, kind: 'reply' | 'resolve', text: string | null) {
    const k = deskKey(code, guest);
    deskAnswers.undo(k, false);
    pending.set(k, { code, guest });
    answers.set(k, { phase: 'sending', kind, text, at: Date.now() });
    return send(k);
  },
  undo(k: string, notify = true) {
    const t = timers.get(k);
    if (t) clearTimeout(t);
    timers.delete(k);
    const a = answers.get(k);
    if (a && a.phase !== 'sent') answers.delete(k);
    if (notify) emit();
  },
  /** Leaving the screen: a held answer goes now. */
  flush(k: string) {
    const t = timers.get(k);
    if (!t) return;
    clearTimeout(t);
    timers.delete(k);
    void send(k);
  },
  flushAll() {
    [...timers.keys()].forEach((k) => deskAnswers.flush(k));
  },
  /** Tests only. */
  reset(ms = DESK_UNDO_MS) {
    timers.forEach((t) => clearTimeout(t));
    timers.clear();
    answers.clear();
    pending.clear();
    holdMs = ms;
    emit();
  },
};

if (typeof window !== 'undefined') window.addEventListener('pagehide', () => deskAnswers.flushAll());

export function useDeskAnswers(): number {
  const [, setV] = useState(version);
  useEffect(() => deskAnswers.subscribe(() => setV(version)), []);
  return version;
}

const stamp = (s: string | null | undefined) => {
  if (!s) return null;
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
};

/** The guests as the host should see them now: an answered guest no longer waits, until they ask again. */
export function withAnswers(code: string, d: DeskInfo): DeskInfo {
  let cleared = 0;
  const guests = d.guests.map((g) => {
    if (!g.waiting) return g;
    const a = answers.get(deskKey(code, g.id));
    if (!isAnswered(a, stamp(g.last?.at))) return g;
    cleared++;
    return { ...g, waiting: false, handoff_reason: null, unread: 0 };
  });
  if (!cleared) return d;
  return { ...d, guests, counts: { ...d.counts, waiting: Math.max(0, d.counts.waiting - cleared) } };
}

// ---------------------------------------------------------------------------------------------------
// States, mood and words

export type DeskStage = 'setup' | 'address' | 'ready' | 'live' | 'cap' | 'stopped';

/** For Pixel 2: set-up until started (address refused → 2), ended → 9, at the cap → 8, no guests → 3, else live. */
export function deskStage(b: Pick<PocketBlock, 'state'>, d: DeskInfo): DeskStage {
  if (b.state === 'ended') return 'stopped';
  if (b.state !== 'live') return d.needs_address ? 'address' : 'setup';
  if (d.counts.replies_today >= d.counts.daily_cap) return 'cap';
  return d.guests.length === 0 && d.counts.guests === 0 ? 'ready' : 'live';
}

const BOT_DOWN = /(couldn['’]t answer this message|was deleted, so no bot is answering|had no reply it could send)$/;

/** §4 moods: thinking at set-up, failed when the engine can't answer, needs_you while a guest waits, finished when stopped. */
export function deskMood(stage: DeskStage, d: DeskInfo): Mood {
  if (stage === 'stopped') return 'finished';
  if (stage === 'setup' || stage === 'address') return 'thinking';
  if (d.guests.some((g) => g.waiting && g.handoff_reason && BOT_DOWN.test(g.handoff_reason))) return 'failed';
  if (d.counts.waiting > 0) return 'needs_you';
  return stage === 'ready' ? 'idle' : 'working';
}

/** Actions Lin's API doesn't have yet: focusable, say why, do nothing (card-impl.md "LATER"). */
export const DESK_LATER = {
  paste: 'Pasting from a doc comes later. Paste the guide into the chat for now.',
  noLink: 'The link exists once the desk starts.',
  address: 'Start needs an address that stays the same: turn on Tailscale Funnel, or use the flat Wi-Fi only.',
};

const NUM = new Intl.NumberFormat('en');
const num = (n: number) => NUM.format(n);
const usd = (n: number) => `$${n > 0 && n < 0.01 ? n.toFixed(3) : n.toFixed(2)}`;
const plural = (n: number, one: string, many = `${one}s`) => `${num(n)} ${n === 1 ? one : many}`;
const clock = (d: Date) => d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });

/** "2:14 PM" today, "Tue" this week, "Nov 12" before; text that isn't a time shows as it is. */
export function shortWhen(s: string, now = new Date()): string {
  const t = stamp(s);
  if (t === null) return s;
  const d = new Date(t);
  if (d.toDateString() === now.toDateString()) return clock(d);
  if (now.getTime() - t < 6 * 86_400_000 && t <= now.getTime()) return d.toLocaleDateString('en-US', { weekday: 'short' });
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}
/** "2026-12-14" → "Dec 14"; display text stays. */
export function dayLabel(s: string): string {
  if (!DAY.test(s)) {
    const t = stamp(s);
    return t === null ? s : new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  }
  const [y, m, d] = s.split('-').map(Number);
  return new Date(y, m - 1, d).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}
const resets = (d: DeskInfo) => {
  const t = stamp(d.cap_resets_at);
  if (t === null) return 'midnight';
  const at = new Date(t);
  return at.getHours() === 0 && at.getMinutes() === 0 ? 'midnight' : clock(at);
};

export const deskCostLine = (d: DeskInfo, cap = d.counts.daily_cap) =>
  d.cost_per_reply_usd !== null && d.bot.model
    ? `${d.bot.name} uses ${d.bot.model} with your API key: about ${usd(d.cost_per_reply_usd)} a reply, so at most ${usd(d.cost_per_reply_usd * cap)} a day at ${num(cap)} replies. Anything ${d.bot.name} wants to do besides answering asks you first.`
    : `${d.bot.name} writes at most ${num(cap)} replies a day.`;
export const deskFootLine = (d: DeskInfo) =>
  `${d.bot.name} writes one reply per guest message (a model call), up to ${num(d.counts.daily_cap)} a day. Your answers and the guest page use none.`;
export const DESK_KEEP = 'Guest chats stay on this Mac. Each guest’s messages are deleted 7 days after their last one.';
export const deskNeverLine = (d: DeskInfo) => `${d.bot.name} never says yes to a late check-out, a refund or an extra night: those come to you.`;

/** The Done row and stopped totals: "6 guests, Bell answered 41, you answered 3". */
export function deskTotals(d: DeskInfo): string {
  const e = d.ended;
  const parts = [plural(d.counts.guests, 'guest')];
  if (e?.bot_replies !== null && e?.bot_replies !== undefined) parts.push(`${d.bot.name} answered ${num(e.bot_replies)}`);
  if (e?.host_replies !== null && e?.host_replies !== undefined) parts.push(`you answered ${num(e.host_replies)}`);
  return parts.join(', ');
}
export function deskDoneLine(d: DeskInfo): string {
  const del = d.ended?.deletes_on ? ` · chats deleted ${dayLabel(d.ended.deletes_on)}` : '';
  return `${d.place ?? d.title} · ${deskTotals(d)}${del}`;
}

// ---------------------------------------------------------------------------------------------------
// Small parts

const P = 'text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5';
const SMALL = 'text-[13px] leading-5 text-zinc-400 sm:text-2xs';
const GRID = 'grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center';
const FULL = 'col-span-2 sm:col-span-1';
const HALF_BTN = `kw-btn-ghost ${HALF}`;

const DeskChip: React.FC<{ stage: DeskStage; d: DeskInfo }> = ({ stage, d }) => {
  switch (stage) {
    case 'setup':
      return (
        <span className="kw-chip kw-chip-info" data-chip="setup">
          <Pencil size={12} aria-hidden="true" />
          Set up · not shared yet
        </span>
      );
    case 'address':
      return (
        <span className="kw-chip kw-chip-warn" data-chip="address">
          <AlertTriangle size={12} aria-hidden="true" />
          Needs your own address
        </span>
      );
    case 'ready':
      return (
        <span className="kw-chip kw-chip-info" data-chip="ready">
          <ConciergeBell size={12} aria-hidden="true" />
          Ready · no guests yet
        </span>
      );
    case 'cap':
      return (
        <span className="kw-chip kw-chip-warn" data-chip="cap">
          <BellRing size={12} aria-hidden="true" />
          Daily limit reached
        </span>
      );
    case 'stopped':
      return (
        <span className="kw-chip" data-chip="stopped">
          <Check size={12} aria-hidden="true" />
          Stopped{d.ended?.at ? ` · ${dayLabel(d.ended.at)}` : ''}
        </span>
      );
    default:
      // The live state the host glances for, like the quiz's Live chip. White on sky-600 is 5.1:1.
      return (
        <span className="kw-chip border-sky-400 bg-sky-600 font-semibold text-white" data-chip="live">
          <span className="kw-status-dot bg-white" aria-hidden="true" />
          {d.counts.waiting > 0 ? `Live · ${num(d.counts.waiting)} waiting` : 'Live'}
        </span>
      );
  }
};

const Tiles: React.FC<{ d: DeskInfo; stage: DeskStage }> = ({ d, stage }) => {
  const c = d.counts;
  const stopped = stage === 'stopped';
  const tile = (key: string, label: string, value: React.ReactNode, sub: string, hot = false) => (
    <div className={`min-w-0 rounded-xl border bg-surface-main px-3 py-2 ${hot ? 'border-sky-600' : 'border-hairline-strong'}`} data-count={key}>
      <div className="truncate text-2xs font-semibold text-zinc-400">{label}</div>
      <div className="text-[22px] font-extrabold leading-7 text-zinc-50 tabular-nums">{value}</div>
      <div className="truncate text-2xs font-medium text-zinc-300">{sub}</div>
    </div>
  );
  const replies = stopped ? d.ended?.bot_replies ?? null : c.replies_today;
  return (
    <div className="grid grid-cols-3 gap-2" role="group" aria-label="Today at the desk">
      {tile('guests', 'Guests', num(c.guests), stopped ? (d.ended?.days ? `in ${plural(d.ended.days, 'day')}` : 'in all') : c.guests ? 'chatting' : 'none yet')}
      {tile('waiting', 'Waiting', num(c.waiting), c.waiting ? 'for you' : 'none', c.waiting > 0)}
      {stopped
        ? tile('replies', `${d.bot.name} replies`, replies === null ? '–' : num(replies), 'in all')
        : tile(
            'replies',
            'Replies',
            <>
              {num(c.replies_today)}
              <span className="text-[14px] font-semibold text-zinc-400"> / {num(c.daily_cap)}</span>
            </>,
            'today',
          )}
    </div>
  );
};

const CapBar: React.FC<{ d: DeskInfo; full: boolean }> = ({ d, full }) => {
  const c = d.counts;
  const label = full
    ? `${num(c.replies_today)} of ${num(c.daily_cap)} replies ${d.bot.name} can write today · until ${resets(d)}, new messages wait for you`
    : `${num(c.replies_today)} of ${num(c.daily_cap)} replies ${d.bot.name} can write today · resets at ${resets(d)}`;
  return (
    <div className="flex flex-col gap-1.5" data-cap-bar={full ? 'full' : 'ok'}>
      <p className={`flex items-center gap-1.5 text-[13px] leading-5 sm:text-2xs ${full ? 'text-warn' : 'text-zinc-300'}`}>
        <Zap size={13} className="shrink-0" aria-hidden="true" />
        {label}
      </p>
      <Bar value={c.replies_today / c.daily_cap} label={`Replies today: ${num(c.replies_today)} of ${num(c.daily_cap)}`} tone={full ? 'warn' : 'accent'} />
    </div>
  );
};

/** For Pixel 4: 10-200 in steps of 10; 44 px sides on a phone (`.phone .mini`). */
export const CapStepper: React.FC<{ value: number; onChange: (n: number) => void; label?: string }> = ({ value, onChange, label = 'Replies a day' }) => {
  const set = (n: number) => onChange(Math.min(DESK_CAP_MAX, Math.max(DESK_CAP_MIN, n)));
  const SIDE = 'flex h-full w-11 items-center justify-center text-zinc-200 hover:bg-white/[0.05] disabled:cursor-not-allowed disabled:text-zinc-600 kw-focus sm:w-9';
  return (
    <div className="inline-grid h-[46px] shrink-0 grid-cols-[auto_64px_auto] items-center overflow-hidden rounded-[10px] border border-hairline-strong sm:h-9" role="group" aria-label={label} data-stepper>
      <button type="button" className={SIDE} aria-label={`Fewer: ${value - DESK_CAP_STEP}`} disabled={value <= DESK_CAP_MIN} onClick={() => set(value - DESK_CAP_STEP)} data-step="down">
        <Minus size={15} aria-hidden="true" />
      </button>
      <output className="border-x border-hairline text-center text-[15px] font-semibold leading-[44px] text-zinc-50 tabular-nums sm:leading-[34px]" aria-live="polite">
        {value}
      </output>
      <button type="button" className={SIDE} aria-label={`More: ${value + DESK_CAP_STEP}`} disabled={value >= DESK_CAP_MAX} onClick={() => set(value + DESK_CAP_STEP)} data-step="up">
        <Plus size={15} aria-hidden="true" />
      </button>
    </div>
  );
};

const joinUrl = (d: DeskInfo) => d.join?.anywhere_url ?? d.join?.wifi_url ?? null;

const AddressRow: React.FC<{ d: DeskInfo; future?: boolean }> = ({ d, future }) => {
  const url = joinUrl(d);
  if (!url) return null;
  const anywhere = !!d.join?.anywhere_url;
  return (
    <Row icon={anywhere ? <Globe size={15} aria-hidden="true" /> : <Wifi size={15} aria-hidden="true" />}>
      Guests {future ? 'will use' : 'use'} <code className="break-all font-mono text-[13px] text-zinc-100 sm:text-2xs">{url.replace(/^https?:\/\//, '')}</code>
      {anywhere ? (d.join?.anywhere_kind === 'funnel' ? ' (Tailscale Funnel)' : '') + '. It stays the same, so the QR can go on the fridge.' : ': guests on the flat Wi-Fi only.'}
    </Row>
  );
};

const KeepRows: React.FC<{ d: DeskInfo }> = ({ d }) => (
  <>
    <Row icon={<Lock size={15} aria-hidden="true" />}>{DESK_KEEP}</Row>
    <Row icon={<ShieldCheck size={15} aria-hidden="true" />}>{deskNeverLine(d)}</Row>
  </>
);

const TryAsGuest: React.FC<{ d: DeskInfo; half?: boolean }> = ({ d, half = true }) => {
  const url = joinUrl(d);
  if (!url)
    return (
      <LaterButton icon={<Eye size={14} aria-hidden="true" />} why={DESK_LATER.noLink} half={half}>
        Try as a guest
      </LaterButton>
    );
  return (
    <a href={url} target="_blank" rel="noreferrer" className={HALF_BTN} data-action="try">
      <Eye size={14} aria-hidden="true" />
      Try as a guest
    </a>
  );
};

const ShareLink: React.FC<{ d: DeskInfo }> = ({ d }) => {
  const url = joinUrl(d);
  const [state, setState] = useState<'copied' | 'failed' | null>(null);
  useEffect(() => {
    if (!state) return;
    const t = setTimeout(() => setState(null), 2000);
    return () => clearTimeout(t);
  }, [state]);
  if (!url)
    return (
      <button type="button" aria-disabled="true" title={DESK_LATER.noLink} aria-description={DESK_LATER.noLink} onClick={(e) => e.preventDefault()} className={`${PRIMARY} ${FULL} ${LATER_CLS}`}>
        <Share2 size={14} aria-hidden="true" />
        Share link
      </button>
    );
  const go = async () => {
    if (typeof navigator !== 'undefined' && typeof navigator.share === 'function') {
      try {
        await navigator.share({ title: d.title, url });
        return;
      } catch (e) {
        if ((e as Error)?.name === 'AbortError') return;
      }
    }
    try {
      await navigator.clipboard.writeText(url);
      setState('copied');
    } catch {
      setState('failed');
    }
  };
  return (
    <button type="button" onClick={go} className={`${PRIMARY} ${FULL}`} data-action="share-link">
      {state === 'copied' ? <Check size={14} aria-hidden="true" /> : <Share2 size={14} aria-hidden="true" />}
      <span aria-live="polite">{state === 'copied' ? 'Link copied' : state === 'failed' ? "Couldn't copy" : 'Share link'}</span>
    </button>
  );
};

const JoinBox: React.FC<{ d: DeskInfo }> = ({ d }) => {
  const url = joinUrl(d);
  if (!url) return null;
  const anywhere = !!d.join?.anywhere_url;
  return (
    <div className="flex flex-col gap-4 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5 sm:flex-row" data-join="full" data-join-mode={anywhere ? 'anywhere' : 'wifi'}>
      <Qr url={url} size="full" name={d.title} />
      <div className="flex min-w-0 flex-1 flex-col gap-2.5">
        <div className="text-[13px] font-semibold text-zinc-200 sm:text-meta">How guests open it</div>
        <p className="inline-flex items-center gap-1.5 self-start rounded-lg bg-sky-600 px-2.5 py-1 text-meta font-semibold text-white">
          {anywhere ? <Globe size={14} aria-hidden="true" /> : <Wifi size={14} aria-hidden="true" />}
          {anywhere ? 'From anywhere' : 'Flat Wi-Fi only'}
        </p>
        <p className="text-[15px] leading-[22px] text-zinc-100 sm:text-body">Print the QR for the fridge or send the link before guests arrive. It stays the same for every stay.</p>
        <p className={SMALL}>Each guest picks a first name and a PIN, so they can come back on another phone.</p>
        <LinkField url={url} />
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// Guest list (For Pixel 5): rows are buttons, newest activity first, the last line cut by CSS.

const lastLine = (g: DeskGuest, bot: string) => {
  if (!g.last) return null;
  const who = g.last.from === 'guest' ? '' : g.last.from === 'bot' ? `${bot}: ` : g.last.from === 'host' ? 'You: ' : '';
  return g.last.from === 'note' ? null : `${who}${g.last.text}`;
};

const GuestRow: React.FC<{ g: DeskGuest; bot: string; onOpen: (id: string) => void }> = ({ g, bot, onOpen }) => {
  const line = lastLine(g, bot);
  return (
    <li className="border-b border-hairline last:border-b-0">
      <button
        type="button"
        onClick={() => onOpen(g.id)}
        className={`grid min-h-[64px] w-full grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-2.5 px-3 py-2 text-left hover:bg-white/[0.03] kw-focus ${g.waiting ? 'bg-sky-500/[0.08]' : ''}`}
        data-guest={g.id}
        data-waiting={g.waiting || undefined}
        aria-label={`${g.name}${g.waiting ? ', needs you' : ''}: open the chat`}
      >
        <GuestCreature avatar={g.creature} size={28} />
        <span className="min-w-0">
          <span className="flex min-w-0 items-baseline gap-1.5">
            <span className="max-w-[60%] shrink-0 truncate text-[15px] font-semibold text-zinc-100 sm:text-[13.5px]">{g.name}</span>
            <small className="min-w-0 truncate text-2xs text-zinc-400">
              {g.last ? `${shortWhen(g.last.at)} · ${plural(g.msgs, 'message')}` : 'Hasn’t written yet'}
            </small>
          </span>
          {g.waiting ? (
            <span className="flex min-w-0 items-center gap-1 text-[13px] text-zinc-200 sm:text-2xs" data-reason>
              <BellRing size={12} className="shrink-0 text-sky-300" aria-hidden="true" />
              <span className="truncate">
                <b className="font-semibold text-zinc-50">Needs you:</b> {g.handoff_reason ?? `${bot} asked you to reply`}
              </span>
            </span>
          ) : line ? (
            <span className="block truncate text-[13px] text-zinc-300 sm:text-2xs">{line}</span>
          ) : null}
        </span>
        {g.waiting ? (
          <span className="rounded-full border border-sky-600 bg-sky-500/15 px-2.5 py-0.5 text-2xs font-semibold text-sky-100" data-answer-chip>
            Answer
          </span>
        ) : g.unread > 0 ? (
          <span className="grid h-[22px] min-w-[22px] place-items-center rounded-full bg-sky-600 px-1.5 text-2xs font-bold text-white" aria-label={`${g.unread} unread`}>
            {g.unread}
          </span>
        ) : g.last ? (
          <span className="text-2xs text-zinc-400">{shortWhen(g.last.at)}</span>
        ) : (
          <span />
        )}
      </button>
    </li>
  );
};

const GuestList: React.FC<{ d: DeskInfo; only?: 'waiting'; onOpen: (id: string) => void }> = ({ d, only, onOpen }) => {
  const list = only === 'waiting' ? d.guests.filter((g) => g.waiting) : d.guests;
  if (list.length === 0) return null;
  return (
    <section className="overflow-hidden rounded-xl border border-hairline-strong bg-surface-main" aria-label="Guests" data-guests>
      <header className="flex items-center justify-between gap-2 border-b border-hairline px-3 py-2 text-2xs font-bold text-zinc-300">
        <span>Guests · newest first</span>
        <span className="font-medium text-zinc-400">only you see this list</span>
      </header>
      <ul>
        {list.map((g) => (
          <GuestRow key={g.id} g={g} bot={d.bot.name} onOpen={onOpen} />
        ))}
      </ul>
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// The Allow card (§3), in the card's state 6 and inline in the Inbox (state 10): the same component, the same store.

export const AllowCard: React.FC<{
  code: string;
  guest: string;
  name: string;
  reason: string;
  quote: string | null;
  bot: string;
  /** Hand-off time (RFC 3339): an answer older than this one doesn't count. */
  askedAt?: string | null;
}> = ({ code, guest, name, reason, quote, bot, askedAt }) => {
  useDeskAnswers();
  const k = deskKey(code, guest);
  const a = deskAnswers.get(k);
  const answered = isAnswered(a, stamp(askedAt ?? null));
  const [writing, setWriting] = useState(false);
  const [text, setText] = useState('');
  const field = useRef<HTMLTextAreaElement | null>(null);
  // Leaving the screen sends a held answer at once.
  useEffect(() => () => deskAnswers.flush(k), [k]);
  useEffect(() => {
    if (writing) field.current?.focus();
  }, [writing]);
  if (answered && a) {
    return (
      <div className="flex flex-col items-end gap-1.5" data-allow={a.phase} role="status">
        {a.text ? (
          <div className="max-w-[300px] rounded-[14px_4px_14px_14px] border border-sky-600 bg-sky-800 px-3 py-2 text-[15px] leading-[22px] text-zinc-50 [overflow-wrap:anywhere] sm:max-w-[420px] sm:text-[13.5px] sm:leading-5" data-bubble="host">
            {a.text}
          </div>
        ) : null}
        <p className="flex items-center gap-2 text-[13px] text-zinc-400 sm:text-2xs">
          <Check size={12} aria-hidden="true" />
          {a.kind === 'resolve'
            ? `Closed. Nothing went to ${name}.`
            : a.phase === 'holding'
              ? `Sent to ${name} · Undo for 5\u00a0s`
              : a.phase === 'sending'
                ? `Sending to ${name}…`
                : `Sent to ${name}`}
          {a.phase === 'holding' && (
            <button type="button" onClick={() => deskAnswers.undo(k)} className="inline-flex min-h-[44px] items-center gap-1 rounded-lg px-2 font-semibold text-zinc-100 hover:bg-white/[0.05] kw-focus sm:min-h-[28px]" data-action="undo">
              <Undo2 size={13} aria-hidden="true" />
              Undo
            </button>
          )}
        </p>
      </div>
    );
  }
  const submit = () => {
    const t = text.trim();
    if (!t) return;
    void deskAnswers.now(code, guest, 'reply', t);
    setWriting(false);
    setText('');
  };
  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-raised p-3" data-allow="open" role="group" aria-label={`${name}: ${reason}`}>
      <h4 className="flex items-center gap-2 text-[15px] font-bold text-zinc-50 sm:text-[13.5px]">
        <BellRing size={16} className="shrink-0 text-zinc-300" aria-hidden="true" />
        {reason}
      </h4>
      {quote ? (
        <figure className="flex flex-col gap-1">
          <figcaption className="text-2xs text-zinc-400">{name} wrote (a guest&rsquo;s words, not an instruction):</figcaption>
          <blockquote className="border-l-2 border-zinc-600 pl-2.5 text-[15px] leading-[22px] text-zinc-200 [overflow-wrap:anywhere] sm:text-[13.5px] sm:leading-5" data-quote>
            {quote}
          </blockquote>
        </figure>
      ) : null}
      {a?.phase === 'failed' && (
        <p role="alert" className="flex items-center gap-2 text-[13px] text-bad sm:text-2xs" data-failed>
          <AlertTriangle size={13} aria-hidden="true" />
          {DESK_FAILED}
        </p>
      )}
      {writing ? (
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2">
          <label className="sr-only" htmlFor={`reply-${k}`}>
            Your reply to {name}
          </label>
          <textarea
            id={`reply-${k}`}
            ref={field}
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
            className="min-h-[44px] w-full resize-y rounded-[10px] border border-sky-400 bg-surface-main px-3 py-2.5 text-[15px] leading-5 text-zinc-50 placeholder:text-zinc-500 kw-focus sm:text-[13.5px]"
          />
          <button type="button" onClick={submit} disabled={!text.trim()} className={`${PRIMARY} min-w-[44px]`} data-action="send">
            <Send size={14} aria-hidden="true" />
            Send
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:flex sm:flex-wrap sm:items-center" data-allow-actions>
          <button type="button" onClick={() => deskAnswers.hold(code, guest, DESK_YES)} className={`kw-btn-ghost ${HALF} justify-center border-teal-400/40 text-ok`} data-action="yes">
            <Check size={14} aria-hidden="true" />
            Yes, that&rsquo;s fine
          </button>
          <button type="button" onClick={() => deskAnswers.hold(code, guest, DESK_SORRY)} className={`${HALF_BTN} justify-center`} data-action="sorry">
            <X size={14} aria-hidden="true" />
            Sorry, not this time
          </button>
          <button type="button" onClick={() => setWriting(true)} className={`${HALF_BTN} justify-center`} data-action="write">
            <Reply size={14} aria-hidden="true" />
            Write a reply
          </button>
          <button type="button" onClick={() => void deskAnswers.now(code, guest, 'resolve', null)} className="min-h-[44px] rounded-xl px-3 text-[14px] font-medium text-zinc-300 hover:bg-white/[0.04] kw-focus sm:min-h-[36px] sm:text-meta" data-action="resolve">
            No reply needed
          </button>
        </div>
      )}
      <p className={SMALL}>
        Your answer goes to {name} as you, marked &ldquo;host&rdquo;. {bot} never answers this for you. &ldquo;No reply needed&rdquo; sends nothing.
      </p>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// One guest's chat inside the card (states 5-7)

const noteWords = (m: DeskMessage, bot: string, host: string) =>
  m.note === 'daily_cap' ? `Daily limit reached: this waits for you` : m.note === 'host_away' ? `${bot} couldn’t answer, so this came to ${host}` : `${bot} handed this to ${host}`;

const Bubble: React.FC<{ m: DeskMessage; guest: DeskGuest; bot: string }> = ({ m, guest, bot }) => {
  if (m.from === 'note')
    return (
      <p className="flex items-center justify-center gap-1.5 text-2xs text-zinc-400" data-note={m.note}>
        <BellRing size={12} aria-hidden="true" />
        {noteWords(m, bot, 'you')} · {shortWhen(m.at)}
      </p>
    );
  const mine = m.from !== 'guest';
  const who = m.from === 'guest' ? guest.name : m.from === 'bot' ? bot : 'You';
  return (
    <div className={`flex items-end gap-2 ${mine ? 'justify-end' : ''}`} data-bubble={m.from}>
      {!mine && <GuestCreature avatar={guest.creature} size={22} />}
      <div className={`flex min-w-0 flex-col gap-1 ${mine ? 'items-end' : ''}`}>
        <span className="text-2xs text-zinc-400">
          <b className="font-semibold text-zinc-200">{who}</b> {shortWhen(m.at)}
          {m.from === 'guest' ? <span className="ml-1 text-zinc-500">guest</span> : null}
        </span>
        <div
          className={`max-w-[300px] px-3 py-2 text-[15px] leading-[22px] text-zinc-50 [overflow-wrap:anywhere] sm:max-w-[420px] sm:text-[13.5px] sm:leading-5 ${
            m.from === 'guest' ? 'rounded-[4px_14px_14px_14px] bg-surface-raised' : m.from === 'host' ? 'rounded-[14px_4px_14px_14px] border border-sky-600 bg-sky-800' : 'rounded-[14px_4px_14px_14px] bg-sky-500/10'
          }`}
        >
          {m.text}
        </div>
      </div>
      {m.from === 'bot' && <ModernAvatar role="desk" name={bot} size={22} modelBadge={false} />}
    </div>
  );
};

type ConvState = { status: 'loading' } | { status: 'error'; text: string } | { status: 'ok'; guest: DeskGuest; messages: DeskMessage[] };

/** Load one conversation; fixtures and tests can pass it in. */
export const deskConversationCache = new Map<string, { guest: DeskGuest; messages: DeskMessage[] }>();

const Conversation: React.FC<{ code: string; d: DeskInfo; guestId: string; onBack: () => void; focus: boolean }> = ({ code, d, guestId, onBack, focus }) => {
  useDeskAnswers();
  const k = deskKey(code, guestId);
  const cached = deskConversationCache.get(k);
  const [conv, setConv] = useState<ConvState>(cached ? { status: 'ok', ...cached } : { status: 'loading' });
  const [writing, setWriting] = useState(false);
  const back = useRef<HTMLButtonElement | null>(null);
  const load = () => {
    pocketApi
      .deskGuest(code, guestId)
      .then((r) => {
        const c = validConversation(r?.conversation);
        if (c) {
          deskConversationCache.set(k, c);
          setConv({ status: 'ok', ...c });
        } else if (!cached) setConv({ status: 'error', text: 'This chat couldn’t be read. Try again.' });
      })
      .catch((e) => {
        if (!cached) setConv({ status: 'error', text: e instanceof PocketApiError ? e.message : DESK_FAILED });
      });
  };
  useEffect(() => {
    if (focus) back.current?.focus();
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k]);
  const fromList = d.guests.find((g) => g.id === guestId);
  const guest = conv.status === 'ok' ? { ...conv.guest, ...(fromList ? { waiting: fromList.waiting, handoff_reason: fromList.handoff_reason } : {}) } : fromList;
  const a = deskAnswers.get(k);
  const handoffAt = conv.status === 'ok' ? [...conv.messages].reverse().find((m) => m.from === 'note')?.at ?? null : null;
  const answered = isAnswered(a, stamp(handoffAt)) && a?.phase !== 'failed';
  const waiting = !!guest?.waiting || (!!a && a.phase !== 'sent' && conv.status === 'ok' && conv.guest.waiting);
  const sub = !guest
    ? ''
    : answered && a?.kind === 'reply'
      ? `${guest.name}’s question is answered`
      : waiting
        ? `${plural(guest.msgs, 'message')} · waiting for you`
        : `${plural(guest.msgs, 'message')}${guest.delete_on ? ` · deleted ${dayLabel(guest.delete_on)} unless ${guest.name} writes again` : ''}`;
  return (
    <section className="overflow-hidden rounded-xl border border-hairline-strong bg-surface-main" aria-label={guest ? `Chat with ${guest.name}` : 'Chat'} data-conversation={guestId}>
      <header className="flex min-h-[48px] items-center gap-2.5 border-b border-hairline px-2 py-1.5 text-[13px] text-zinc-300">
        <button ref={back} type="button" onClick={onBack} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg text-zinc-300 hover:bg-white/[0.05] kw-focus sm:h-8 sm:w-8" aria-label="All guests" data-action="back">
          <ChevronLeft size={16} aria-hidden="true" />
        </button>
        {guest && <GuestCreature avatar={guest.creature} size={24} />}
        <span className="min-w-0">
          <b className="block truncate text-[15px] font-semibold text-zinc-50 sm:text-[14px]">{guest?.name ?? 'Guest'}</b>
          <span className="block truncate text-2xs text-zinc-400" data-conv-sub>
            {sub}
          </span>
        </span>
      </header>
      <div className="flex flex-col gap-2.5 p-3">
        {conv.status === 'loading' && <p className={SMALL}>Reading the chat&hellip;</p>}
        {conv.status === 'error' && (
          <p role="alert" className="flex items-center gap-2 text-[13px] text-bad sm:text-2xs">
            <AlertTriangle size={13} aria-hidden="true" />
            {conv.text}
            <button type="button" onClick={load} className="ml-1 inline-flex min-h-[44px] items-center gap-1 rounded-lg px-2 font-semibold text-zinc-100 hover:bg-white/[0.05] kw-focus sm:min-h-[28px]">
              <RotateCcw size={12} aria-hidden="true" />
              Try again
            </button>
          </p>
        )}
        {conv.status === 'ok' && guest && conv.messages.map((m) => <Bubble key={m.id} m={m} guest={guest} bot={d.bot.name} />)}
        {guest && (waiting || (a && a.phase !== 'sent' && isAnswered(a))) ? (
          <AllowCard code={code} guest={guestId} name={guest.name} reason={guest.handoff_reason ?? `${d.bot.name} asked you to reply`} quote={lastGuestWords(conv)} bot={d.bot.name} askedAt={handoffAt} />
        ) : guest && a?.phase === 'sent' && a.text ? (
          <AllowCard code={code} guest={guestId} name={guest.name} reason="" quote={null} bot={d.bot.name} />
        ) : guest && writing ? (
          <AllowWrite code={code} guest={guestId} name={guest.name} onDone={() => setWriting(false)} />
        ) : guest ? (
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={() => setWriting(true)} className={HALF_BTN} data-action="write-to">
              <Reply size={14} aria-hidden="true" />
              Write to {guest.name}
            </button>
          </div>
        ) : null}
      </div>
    </section>
  );
};

const lastGuestWords = (c: ConvState) => {
  if (c.status !== 'ok') return null;
  const m = [...c.messages].reverse().find((x) => x.from === 'guest');
  return m ? (m.text.length > 200 ? `${m.text.slice(0, 199)}…` : m.text) : null;
};

/** "Write to Tom" outside a hand-off: the same reply action, sent now. */
const AllowWrite: React.FC<{ code: string; guest: string; name: string; onDone: () => void }> = ({ code, guest, name, onDone }) => {
  const [text, setText] = useState('');
  const ref = useRef<HTMLTextAreaElement | null>(null);
  useEffect(() => ref.current?.focus(), []);
  const submit = () => {
    const t = text.trim();
    if (!t) return;
    void deskAnswers.now(code, guest, 'reply', t);
    onDone();
  };
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-2" data-write>
      <label className="sr-only" htmlFor={`write-${code}-${guest}`}>
        Your message to {name}
      </label>
      <textarea
        id={`write-${code}-${guest}`}
        ref={ref}
        rows={2}
        maxLength={4000}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            submit();
          }
          if (e.key === 'Escape') onDone();
        }}
        placeholder={`Write to ${name}`}
        className="min-h-[44px] w-full resize-y rounded-[10px] border border-sky-400 bg-surface-raised px-3 py-2.5 text-[15px] leading-5 text-zinc-50 placeholder:text-zinc-500 kw-focus sm:text-[13.5px]"
      />
      <button type="button" onClick={submit} disabled={!text.trim()} className={`${PRIMARY} min-w-[44px]`} data-action="send">
        <Send size={14} aria-hidden="true" />
        Send
      </button>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card

type Confirm = 'stop' | 'delete' | null;

export const DeskCard: React.FC<{ block: PocketBlock; ask: (t: string) => void; /** Fixtures: open a guest's chat. */ openGuest?: string | null }> = ({ block, ask, openGuest = null }) => {
  useDeskAnswers();
  const [fresh, setFresh] = useState<{ desk: DeskInfo; ended: boolean } | null>(null);
  const [open, setOpenRaw] = useState<string | null>(openGuest);
  // The host opened it (not a chat drawn open): focus goes to Back, and back to the list after.
  const [moved, setMoved] = useState(false);
  const setOpen = (id: string | null) => {
    setMoved(true);
    setOpenRaw(id);
  };
  const [cap, setCap] = useState(block.desk!.counts.daily_cap);
  const [raising, setRaising] = useState(false);
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ kind: 'ok' | 'error'; text: string } | null>(null);
  const [stopped, setStopped] = useState(false);
  const listRef = useRef<HTMLDivElement | null>(null);
  const base = fresh?.desk ?? block.desk!;
  const d = useMemo(() => withAnswers(block.code, base), [block.code, base, deskAnswers.version()]); // eslint-disable-line react-hooks/exhaustive-deps
  const state = stopped || fresh?.ended ? 'ended' : block.state;
  const stage = deskStage({ state }, d);
  // One read when a started desk shows (no polling), and the read every answer returns.
  useEffect(() => {
    if (block.state !== 'live') return;
    let gone = false;
    pocketApi
      .read(block.code)
      .then((r) => {
        const f = !gone && deskFromRead(block.desk!, r);
        if (f) setFresh(f);
      })
      .catch(() => undefined);
    const off = deskAnswers.onRead((code, r) => {
      if (code !== block.code) return;
      const f = deskFromRead(block.desk!, r);
      if (f) setFresh(f);
    });
    return () => {
      gone = true;
      off();
    };
  }, [block.code, block.state, block.desk]);

  const run = async (what: () => Promise<unknown>, ok: string, final: boolean) => {
    setBusy(true);
    try {
      await what();
      setDone({ kind: 'ok', text: ok });
      setConfirm(null);
      if (final) setStopped(true);
    } catch (e) {
      setDone({ kind: 'error', text: `Couldn't do that: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.` });
    } finally {
      setBusy(false);
    }
  };
  const bot = d.bot.name;
  const where = [d.place, `guests ask ${bot}, the rest comes to you`].filter(Boolean).join(' · ');
  const conv = open && (stage === 'live' || stage === 'cap' || stage === 'stopped' || stage === 'ready');
  const firstWaiting = d.guests.find((g) => g.waiting);

  const stopButton = (
    <button type="button" onClick={() => setConfirm('stop')} className={`${STOP} col-span-2 justify-self-center sm:ml-auto`} data-action="stop">
      <Square size={13} aria-hidden="true" />
      Stop
    </button>
  );
  const editGuide = (
    <button type="button" onClick={() => ask(`I want to change the house guide for ${d.title}.`)} className={HALF_BTN} data-action="edit-guide">
      <Pencil size={14} aria-hidden="true" />
      Edit guide
    </button>
  );

  let body: React.ReactNode = null;
  let actions: React.ReactNode = null;
  if (stage === 'setup' || stage === 'address') {
    const address = stage === 'address';
    body = (
      <>
        {address ? (
          <div className="flex flex-col gap-2.5 rounded-xl border border-amber-300/35 bg-amber-300/[0.07] p-3" data-callout="no-stable-address">
            <p className={`flex items-start gap-2 ${P}`}>
              <AlertTriangle size={15} className={`${ICON} text-warn`} aria-hidden="true" />
              <span>
                <strong className="font-semibold text-zinc-50">Guests come back over days, so the link must never change.</strong> This Mac has no address that stays the same yet, and a quick tunnel gets a new one each restart.
              </span>
            </p>
            <div className="flex flex-wrap items-start gap-2">
              <HowTo port={portOf(d.join?.wifi_url ?? null)} bot={bot} chat="your guests" />
              <button type="button" onClick={() => ask(`Start ${d.title} for guests on the flat Wi-Fi only, with ${cap} replies a day.`)} className={HALF_BTN} data-action="wifi-only">
                <Wifi size={14} aria-hidden="true" />
                Use flat Wi-Fi only
              </button>
            </div>
            <p className={SMALL}>Funnel is free on your own Tailscale account; guests need nothing. Flat Wi-Fi only works for guests on the flat&rsquo;s network.</p>
            <NoQuickLink />
          </div>
        ) : (
          <SetupParts d={d} cap={cap} setCap={setCap} ask={ask} />
        )}
        <ul className="flex flex-col gap-2">
          <KeepRows d={d} />
          {!address && <AddressRow d={d} future />}
        </ul>
      </>
    );
    actions = (
      <div className={GRID}>
        {address ? (
          <button type="button" aria-disabled="true" title={DESK_LATER.address} aria-description={DESK_LATER.address} onClick={(e) => e.preventDefault()} className={`${PRIMARY} ${FULL} ${LATER_CLS}`} data-action="start">
            Start and get the link
          </button>
        ) : (
          <button type="button" onClick={() => ask(`Start ${d.title} with ${cap} replies a day and give me the link.`)} className={`${PRIMARY} ${FULL}`} data-action="start">
            Start and get the link
          </button>
        )}
        <TryAsGuest d={{ ...d, join: null }} />
        <button type="button" onClick={() => ask(`Discard the ${d.title} front desk set-up.`)} className={`${STOP} sm:ml-auto`} data-action="discard">
          <Trash2 size={13} aria-hidden="true" />
          Discard
        </button>
      </div>
    );
  } else if (stage === 'stopped') {
    const e = d.ended;
    body = (
      <>
        <Tiles d={d} stage={stage} />
        <ul className="flex flex-col gap-2">
          <Row icon={<MessageSquare size={15} aria-hidden="true" />}>
            {deskTotals(d)}
            {e?.messages !== null && e?.messages !== undefined ? `, ${plural(e.messages, 'message')}` : ''}.
          </Row>
          <Row icon={<Lock size={15} aria-hidden="true" />}>
            The guest page is closed.{' '}
            {e?.deletes_on ? (
              <>
                The last chat is deleted on <b className="font-semibold text-zinc-50">{dayLabel(e.deletes_on)}</b> (7 days after the last message).
              </>
            ) : (
              'Each chat is deleted 7 days after its last message.'
            )}
          </Row>
        </ul>
        {open ? null : raising ? <GuestList d={d} onOpen={setOpen} /> : null}
      </>
    );
    actions = (
      <div className={GRID}>
        <button type="button" onClick={() => ask(`Open ${d.title} again for the next guests.`)} className={`${PRIMARY} ${FULL}`} data-action="open-again">
          <RotateCcw size={14} aria-hidden="true" />
          Open again for the next guests
        </button>
        {d.guests.length > 0 && (
          <button type="button" onClick={() => setRaising(!raising)} aria-expanded={raising} className={HALF_BTN} data-action="read-chats">
            <MessageSquare size={14} aria-hidden="true" />
            Read the chats
          </button>
        )}
        <button type="button" onClick={() => setConfirm('delete')} className={`${STOP} sm:ml-auto`} data-action="delete">
          <Trash2 size={13} aria-hidden="true" />
          Delete chats now
        </button>
      </div>
    );
  } else {
    const full = stage === 'cap';
    const waiting = d.guests.filter((g) => g.waiting).length;
    body = (
      <>
        <Tiles d={d} stage={stage} />
        {stage !== 'ready' && <CapBar d={d} full={full} />}
        {full && !conv && (
          <Callout tone="warn">
            <div className="flex flex-col gap-2.5" data-callout="cap">
              <p className="flex items-start gap-2">
                <AlertTriangle size={15} className={`${ICON} text-warn`} aria-hidden="true" />
                <span>
                  {bot} wrote {num(d.counts.replies_today)} replies today, its limit. Until {resets(d)} guests read &ldquo;{bot} has answered all it can today. {d.host_name ?? 'The host'} will reply here.&rdquo; Their messages wait for you
                  {waiting ? `: ${num(waiting)} now.` : '.'}
                </span>
              </p>
              <div className="flex flex-wrap items-center gap-2">
                {waiting > 0 && firstWaiting && (
                  <button type="button" onClick={() => setOpen(firstWaiting.id)} className={PRIMARY} data-action="answer-waiting">
                    Answer the {num(waiting)} waiting
                  </button>
                )}
                <button type="button" onClick={() => setRaising(!raising)} aria-expanded={raising} className={HALF_BTN} data-action="raise">
                  <BellRing size={14} aria-hidden="true" />
                  Raise the limit
                </button>
              </div>
              {raising && (
                <div className="flex flex-wrap items-center gap-2.5" data-raise>
                  <CapStepper value={cap} onChange={setCap} label="New daily limit" />
                  <button
                    type="button"
                    disabled={cap === d.counts.daily_cap}
                    onClick={() => {
                      ask(`Raise ${bot}'s daily limit for ${d.title} to ${cap} replies a day.`);
                      setRaising(false);
                    }}
                    className={HALF_BTN}
                    data-action="save-cap"
                  >
                    Ask {bot} to change it
                  </button>
                  <span className={SMALL}>{deskCostLine(d, cap)}</span>
                </div>
              )}
            </div>
          </Callout>
        )}
        {conv ? (
          <Conversation code={block.code} d={d} guestId={open!} onBack={() => setOpen(null)} focus={moved} />
        ) : stage === 'ready' ? (
          <>
            <ul className="flex flex-col gap-2">
              <KeepRows d={d} />
            </ul>
            <JoinBox d={d} />
          </>
        ) : (
          <div ref={listRef}>
            <GuestList d={d} only={full ? 'waiting' : undefined} onOpen={setOpen} />
          </div>
        )}
        {stage !== 'ready' && !conv && (
          <ul className="flex flex-col gap-2">
            <KeepRows d={d} />
          </ul>
        )}
      </>
    );
    actions = (
      <div className={GRID}>
        <ShareLink d={d} />
        {stage === 'ready' ? <TryAsGuest d={d} /> : editGuide}
        {stage === 'ready' ? editGuide : <TryAsGuest d={d} />}
        {stopButton}
      </div>
    );
  }

  return (
    <section
      className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card sm:p-[18px]"
      data-pocket={state}
      data-desk={stage}
      data-mood={deskMood(stage, d)}
      aria-label={`${d.title}, pocket app Guest Guide`}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950 text-sky-200" aria-hidden="true">
          <ConciergeBell size={22} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">Pocket app · Guest Guide</div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{d.title}</h3>
          <p className="text-[15px] text-zinc-300 sm:text-[13px]">{where}</p>
          <span className="mt-1.5 block sm:hidden">
            <DeskChip stage={stage} d={d} />
          </span>
        </div>
        <span className="hidden shrink-0 sm:block">
          <DeskChip stage={stage} d={d} />
        </span>
      </header>
      <div className="relative flex flex-col gap-4">
        {body}
        {done ? (
          <p role="status" className={`flex items-start gap-2 text-[15px] sm:text-[13px] ${done.kind === 'ok' ? 'text-ok' : 'text-bad'}`} data-done={done.kind}>
            {done.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
            {done.text}
          </p>
        ) : null}
        {confirm ? (
          <div className="flex flex-col gap-2.5 rounded-xl border border-hairline-strong bg-surface-rail/60 p-3" role="group" aria-label={confirm === 'stop' ? 'Stop the desk?' : 'Delete the chats now?'} data-confirm={confirm}>
            <p className={P}>
              {confirm === 'stop'
                ? `Stop ${d.title}? The guest page closes and ${bot} stops answering. Each chat is still deleted 7 days after its last message.`
                : `Delete every guest chat on ${d.title} now? This can't be undone.`}
            </p>
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  confirm === 'stop'
                    ? run(() => pocketApi.end(block.code), `${d.title} is stopped. Guests see that it's closed.`, true)
                    : run(() => pocketApi.remove(block.code), 'The chats are deleted.', true)
                }
                className={STOP}
                data-action={`confirm-${confirm}`}
              >
                {confirm === 'stop' ? 'Stop the desk' : 'Delete chats now'}
              </button>
              <button type="button" disabled={busy} onClick={() => setConfirm(null)} className={GHOST}>
                {confirm === 'stop' ? 'Keep it open' : 'Keep them'}
              </button>
            </div>
          </div>
        ) : stopped && done?.kind === 'ok' ? null : (
          actions
        )}
      </div>
      <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]" data-budget>
        <Zap size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
        <span>{deskFootLine(d)}</span>
      </footer>
    </section>
  );
};

const SetupParts: React.FC<{ d: DeskInfo; cap: number; setCap: (n: number) => void; ask: (t: string) => void }> = ({ d, cap, setCap, ask }) => {
  const H = 'flex items-center justify-between gap-2 text-2xs font-bold text-zinc-300';
  const guideSize = [d.guide_facts !== null ? plural(d.guide_facts, 'fact') : null, d.guide_chars !== null ? plural(d.guide_chars, 'character') : null].filter(Boolean).join(' · ');
  return (
    <div className="flex flex-col gap-3" data-setup>
      <div className="flex flex-col gap-2">
        <div className={H}>
          <span>House guide</span>
          {guideSize && <small className="font-medium text-zinc-400">{guideSize}</small>}
        </div>
        {d.guide_preview && (
          <div className="relative max-h-[118px] overflow-hidden whitespace-pre-line rounded-[10px] border border-zinc-700 bg-surface-main px-3 py-2.5 font-mono text-[12.5px] leading-[19px] text-zinc-200" data-guide>
            {d.guide_preview}
            <span className="pointer-events-none absolute inset-x-0 bottom-0 h-7 bg-gradient-to-t from-surface-main to-transparent" aria-hidden="true" />
          </div>
        )}
        <div className="flex flex-wrap gap-2">
          <button type="button" onClick={() => ask(`I want to change the house guide for ${d.title}.`)} className={HALF_BTN} data-action="edit-guide">
            <Pencil size={14} aria-hidden="true" />
            Edit guide
          </button>
          <LaterButton icon={<FileText size={14} aria-hidden="true" />} why={DESK_LATER.paste} half>
            Paste from a doc
          </LaterButton>
        </div>
      </div>
      <div className="flex flex-col gap-2">
        <div className={H}>Who answers</div>
        <div className="flex min-h-[44px] items-center gap-2.5 rounded-[10px] border border-hairline-strong bg-surface-main px-2.5 py-1.5">
          <ModernAvatar role="desk" name={d.bot.name} size={28} modelBadge={false} mood="thinking" moodText={`${d.bot.name}: reading the guide`} />
          <span className="min-w-0 flex-1 text-[15px] sm:text-[13.5px]">
            <b className="font-semibold text-zinc-50">{d.bot.name}</b>
            <small className="block text-2xs text-zinc-400">Answers only from this guide</small>
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
          <span className="min-w-0 text-[13px] leading-5 text-zinc-300 sm:text-2xs">
            After {num(cap)}, guests read &ldquo;{d.host_name ?? 'The host'} will reply&rdquo; and their messages wait in your Inbox.
          </span>
        </div>
      </div>
      <p className="flex items-start gap-2 rounded-[10px] border border-sky-800 bg-sky-500/[0.08] px-3 py-2.5 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-[19px]" data-cost>
        <Zap size={15} className={`${ICON} text-sky-300`} aria-hidden="true" />
        <span>{deskCostLine(d, cap)}</span>
      </p>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// Inbox (state 10): one row per waiting guest with the Allow card inline; the Done row.

export const DeskNeedsRow: React.FC<{ item: DeskItem }> = ({ item }) => {
  useDeskAnswers();
  const a = deskAnswers.get(deskKey(item.pocket_code, item.guest_id));
  // Sent: the row goes; held or sending: it stays, so Undo is still in reach.
  if (a?.phase === 'sent' && isAnswered(a, stamp(item.asked_at))) return null;
  return (
    <li className="overflow-hidden rounded-[14px] border border-hairline-strong bg-surface-raised" data-desk-item={item.id}>
      <div className="flex items-center gap-2.5 px-3.5 pb-1.5 pt-3">
        <ModernAvatar role="desk" name={item.kwant_name} size={28} modelBadge={false} mood="needs_you" moodText={`${item.kwant_name}: ${item.guest_name} needs you`} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] font-semibold text-zinc-100 sm:text-[14px]">{item.guest_name} needs you</div>
          <div className="truncate text-[13px] text-zinc-400 sm:text-2xs">
            {item.pocket_title} · Guest Guide · {shortWhen(item.asked_at)}
          </div>
        </div>
        <span className="kw-chip shrink-0" data-chip="needs-you">
          <BellRing size={12} aria-hidden="true" />
          Needs you
        </span>
      </div>
      <div className="px-3 pb-3">
        <AllowCard code={item.pocket_code} guest={item.guest_id} name={item.guest_name} reason={item.text} quote={item.quote} bot={item.kwant_name} askedAt={item.asked_at} />
      </div>
    </li>
  );
};

/**
 * The Inbox's desk items: read `/api/waiting` when the Inbox loads or refreshes (`reload` changes), never polled.
 * `initial` (tests, shots) skips the read; `skip` (the Inbox's noLoad) too.
 */
export function useDeskItems(reload: unknown, skip = false, initial?: DeskItem[]): DeskItem[] {
  const [items, setItems] = useState<DeskItem[]>(initial ?? []);
  useEffect(() => {
    if (initial || skip) return;
    let gone = false;
    pocketApi
      .waiting()
      .then((b) => !gone && setItems(validDeskItems(b)))
      .catch(() => undefined);
    return () => {
      gone = true;
    };
  }, [initial, skip, reload]);
  return initial ?? items;
}

/** Rows still open in this app: a sent answer closes its row (a held one stays, so Undo is in reach). */
export const openDeskItems = (items: DeskItem[]) =>
  items.filter((it) => {
    const a = deskAnswers.get(deskKey(it.pocket_code, it.guest_id));
    return !(a?.phase === 'sent' && isAnswered(a, stamp(it.asked_at)));
  });

export const DeskDoneRow: React.FC<{ d: DeskInfo; onOpen?: () => void }> = ({ d, onOpen }) => (
  <li className="grid grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-2.5 border-b border-hairline px-3.5 py-2.5 last:border-b-0" data-done-row="desk">
    <ModernAvatar role="desk" name={d.bot.name} size={28} modelBadge={false} mood="finished" moodText={`${d.bot.name}: the desk is stopped`} />
    <div className="min-w-0">
      <div className="truncate text-[15px] font-semibold text-zinc-100 sm:text-[13.5px]">{d.title} · Guest Guide stopped</div>
      <div className="truncate text-[13px] text-zinc-400 sm:text-2xs">{deskDoneLine(d)}</div>
    </div>
    {onOpen ? (
      <button type="button" onClick={onOpen} className="min-h-[44px] rounded-lg px-2 text-[13px] font-medium text-zinc-300 hover:bg-white/[0.04] kw-focus sm:min-h-[32px] sm:text-2xs">
        Open
      </button>
    ) : (
      <span />
    )}
  </li>
);
