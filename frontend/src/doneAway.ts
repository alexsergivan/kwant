// 0.9.5 F: Inbox "Done while you were away" (release-095/inbox-away/brief.md, Mira's ux-audit-090 §1 and
// mockup 2). The queue splits into Needs you and Done while you were away; a done item's pane says what ran,
// when and why, shows the answer inline, what the bot did, and the routine's own words. Everything here is
// read from today's API (inbox items, /api/routines, a routine's runs, the result message). Fields the API
// lacks are listed in release-095/inbox-away/impl.md for Vega's part B; nothing here invents a number.

import { parseActivityMeta, plainTitle } from './activity';
import type { InboxGroup, InboxItem, InboxResponse, ReviewItem } from './api/inbox';
import { isReview } from './api/inbox';
import { type Routine, type RoutineRun, type Schedule, WEEKDAYS, monthDaysText } from './continuity';
import { parseMetaObject } from './questions';

export const NEEDS_GROUPS: InboxGroup[] = ['waiting_on_you', 'failed_or_stuck'];
export const DONE_GROUPS: InboxGroup[] = ['to_review', 'finished'];

/** How long done items stay; the footer says so and "Earlier this week" loads this window. */
export const DONE_DAYS = 7;
export const DONE_FOOTER = `Done items stay here for ${DONE_DAYS} days.`;
export const EARLIER_LABEL = 'Earlier this week';

export const doneSince = (now: number = Date.now()): string => new Date(now - DONE_DAYS * 86400_000).toISOString();

export const isDoneGroup = (g: InboxGroup | undefined): boolean => !!g && DONE_GROUPS.includes(g);

const ROUTINE_PREFIX = /^Routine:\s*/;

/** A routine run's own row, or the review task a routine run made (`Routine: <name>`). */
export const isRoutineItem = (i: InboxItem): boolean => i.kind === 'routine_run' || (isReview(i) && ROUTINE_PREFIX.test(i.title));

/** The routine's name: a review task's title without its `Routine: ` prefix. */
export const routineName = (i: InboxItem): string => (isReview(i) ? i.title.replace(ROUTINE_PREFIX, '') : i.title);

/** How close a routine run's end and its review task's time must be to read as the same run. */
const SAME_RUN_MS = 15 * 60_000;

export interface Sections {
  needs: InboxItem[];
  done: InboxItem[];
  /**
   * A routine's review task and its finished run are one thing done: the run's row is folded into the review
   * (same bot, same routine name, ended within 15 minutes). Keyed by the review item's id.
   */
  runFor: Record<string, InboxItem>;
}

/** Needs you (waiting, then failed or stuck) and Done (to review, then finished), in the server's order. */
export function sectionItems(r: Pick<InboxResponse, 'items'>): Sections {
  const by: Record<InboxGroup, InboxItem[]> = {
    waiting_on_you: [],
    failed_or_stuck: [],
    to_review: [],
    finished: [],
  };
  for (const i of r.items) (by[i.group] ?? by.finished).push(i);
  const runFor: Record<string, InboxItem> = {};
  const folded = new Set<string>();
  const runs = r.items.filter((i) => i.kind === 'routine_run' && isDoneGroup(i.group));
  for (const rev of r.items) {
    if (!isReview(rev) || !ROUTINE_PREFIX.test(rev.title) || !isDoneGroup(rev.group)) continue;
    const name = routineName(rev);
    const at = Date.parse(rev.timestamp);
    const run = runs
      .filter((x) => !folded.has(x.id) && x.kwant_id === rev.kwant_id && x.title === name)
      .map((x) => ({ x, d: Math.abs(Date.parse(x.timestamp) - at) }))
      .filter(({ d }) => Number.isFinite(d) && d <= SAME_RUN_MS)
      .sort((a, b) => a.d - b.d)[0]?.x;
    if (run) {
      runFor[rev.id] = run;
      folded.add(run.id);
    }
  }
  const keep = (i: InboxItem) => !folded.has(i.id);
  return {
    needs: [...by.waiting_on_you, ...by.failed_or_stuck],
    done: [...by.to_review, ...by.finished].filter(keep),
    runFor,
  };
}

/** The row's first line before the bot's name: "Routine" for a routine's work, else nothing. */
export const rowKicker = (i: InboxItem): string | null => (isRoutineItem(i) ? 'Routine' : null);

/** The row's title: a routine's own name, without the `Routine: ` its review task carries. */
export const doneTitle = (i: InboxItem): string => (isRoutineItem(i) ? routineName(i) : i.title);

// ---------- Times ----------

const pad = (n: number) => String(n).padStart(2, '0');

/** "22:51", local time; '' for a bad stamp. */
export function hm(ts: string | null | undefined): string {
  const t = Date.parse(ts ?? '');
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** "22:51:02", local time; '' for a bad stamp. */
export function hms(ts: string | null | undefined): string {
  const t = Date.parse(ts ?? '');
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "Nov 11", local date; '' for a bad stamp. */
export function monthDay(ts: string | null | undefined): string {
  const t = Date.parse(ts ?? '');
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
}

// ---------- Meta line ----------

/** The schedule as the meta line says it: "every Mon 09:00", "every day 08:30", "every 2 h". */
export function scheduleShort(s: Schedule | null | undefined): string {
  if (!s || typeof s !== 'object') return '';
  switch (s.type) {
    case 'once':
      return 'once';
    case 'daily':
      return `every day ${s.time}`;
    case 'weekly': {
      const days = s.weekdays.length === 7 ? 'day' : s.weekdays.map((d) => WEEKDAYS.find((w) => w.id === d)?.label ?? d).join(', ');
      return `every ${days} ${s.time}`;
    }
    case 'interval':
      return s.every_minutes % 60 === 0 ? `every ${s.every_minutes / 60} h` : `every ${s.every_minutes} min`;
    case 'monthly':
      return `monthly on ${monthDaysText(s.days)} ${s.time}`;
    case 'cron':
      return 'on a custom rule';
    default:
      return '';
  }
}

/**
 * "Ben · routine, every Mon 09:00 · ran 22:51 because you pressed Run now". Without the routine it says what
 * the item says ("Ben · finished 22:45"); without the run it leaves the why out rather than guess it.
 */
export function metaLine(item: InboxItem, routine: Routine | null, run: RoutineRun | null): string {
  const bot = item.kwant_name;
  if (!isRoutineItem(item)) {
    const at = hm(item.timestamp);
    return at ? `${bot} · finished ${at}` : bot;
  }
  const sched = routine ? scheduleShort(routine.schedule) : '';
  const parts = [bot, sched ? `routine, ${sched}` : 'routine'];
  const ranAt = hm(run?.started_at ?? null) || hm(item.timestamp);
  if (ranAt) {
    const why = run ? (run.trigger === 'manual' ? ' because you pressed Run now' : ' on schedule') : '';
    parts.push(`ran ${ranAt}${why}`);
  }
  return parts.join(' · ');
}

// ---------- The result message ----------

/** The fields of a chat message this pane reads (GET /api/channels/:id/messages). */
export interface DoneMessage {
  id: string;
  text: string;
  created_at: string;
  meta?: unknown;
  sender_id?: string | null;
}

export interface TrailRow {
  at: string;
  /** Plain text: a step's title without its markdown backticks. */
  what: string;
  detail: string | null;
  failed: boolean;
}

/**
 * "What <bot> did": the steps the reply saved in `meta.activity` (tools, approvals, human steps; thinking and
 * notes left out), each at the time it started, then the answer itself. Empty when the message saved none.
 */
export function trailFrom(msg: DoneMessage | null, place: string | null): TrailRow[] {
  if (!msg) return [];
  const a = parseActivityMeta(msg.meta);
  const rows: TrailRow[] = (a?.steps ?? [])
    .filter((s) => s.kind !== 'thinking' && s.kind !== 'note')
    .map((s) => ({
      at: hms(s.started_at),
      what: plainTitle(s.title),
      detail: s.detail ? plainTitle(s.detail) : null,
      failed: s.status === 'failed',
    }));
  if (!rows.length) return [];
  rows.push({
    at: hms(msg.created_at),
    what: place ? `Wrote the answer in ${place}` : 'Wrote the answer',
    detail: null,
    failed: false,
  });
  return rows;
}

/** Steps the saved activity left out (it keeps the last ones), so the trail can say so. */
export const trailDropped = (msg: DoneMessage | null): number => parseActivityMeta(msg?.meta)?.dropped ?? 0;

/** A message's own text, or null when there is none to show. */
export const answerText = (msg: DoneMessage | null): string | null => (msg && typeof msg.text === 'string' && msg.text.trim() ? msg.text : null);

/** `meta.changed` on a result message, when a server sends it (Vega B); else unknown. */
export function changedFromMeta(msg: DoneMessage | null): boolean | null {
  const v = parseMetaObject(msg?.meta)?.changed;
  return typeof v === 'boolean' ? v : null;
}

// ---------- Chips and actions ----------

/**
 * Whether the work changed anything: the item's `changed` when the server sends it (Vega B), else the
 * message's `meta.changed`. For a review, `rollback_ok` true means there is something to roll back.
 * Null means unknown, and then no "Read only" chip is drawn.
 */
export function changedState(item: InboxItem, msg: DoneMessage | null): boolean | null {
  const own = (item as { changed?: unknown }).changed;
  if (typeof own === 'boolean') return own;
  const m = changedFromMeta(msg);
  if (m !== null) return m;
  if (isReview(item) && item.rollback_ok) return true;
  return null;
}

/** Roll back shows only when something changed and the server says it can be rolled back. */
export const rollbackShown = (item: ReviewItem, changed: boolean | null): boolean => !!item.rollback_ok && changed !== false;

/** "No checks set": a review with no checks and no criteria. Unknown for other items (no chip). */
export function noChecksSet(item: InboxItem): boolean {
  if (!isReview(item)) return false;
  return item.evidence.checks.length === 0 && item.criteria.length === 0 && item.verdict.result === 'none';
}

/** The send-back button and its panel's title. */
export const redoLabel = (bot: string): string => `Ask ${bot} to redo…`;
export const redoTitle = (bot: string): string => `Ask ${bot} to redo`;
export const LOOKS_GOOD = 'Looks good';

// ---------- Finding the routine and its run ----------

/** The routine an item came from: same bot and name. Null when it was renamed or deleted. */
export function findRoutine(routines: Routine[], item: InboxItem): Routine | null {
  if (!isRoutineItem(item)) return null;
  const name = routineName(item);
  return routines.find((r) => r.kwant_id === item.kwant_id && r.name === name) ?? null;
}

/** The run: by id for a routine-run row, else the finished run that ended closest to the item's time. */
export function findRun(runs: RoutineRun[], runId: string | null, at: string): RoutineRun | null {
  if (runId) {
    const own = runs.find((r) => r.id === runId);
    if (own) return own;
  }
  const t = Date.parse(at);
  if (Number.isNaN(t)) return null;
  return (
    runs
      .filter((r) => r.status === 'ok' || r.status === 'error')
      .map((r) => ({
        r,
        d: Math.abs(Date.parse(r.ended_at ?? r.started_at) - t),
      }))
      .filter(({ d }) => Number.isFinite(d) && d <= SAME_RUN_MS)
      .sort((a, b) => a.d - b.d)[0]?.r ?? null
  );
}

/** Where the answer was written, for the trail's last line: the routine's channel, else the direct chat. */
export const placeLabel = (channelName: string | null | undefined): string | null => (channelName ? channelName : null);
