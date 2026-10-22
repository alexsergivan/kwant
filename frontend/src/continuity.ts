// Agent continuity: pinned instructions, chat search, kwant memory, routines and notifications.

import type { DeletedSkill, Skill } from './skills';

export const MAX_INSTRUCTIONS = 4000;
export const MAX_MEMORY = 500;
export const MAX_MEMORIES = 100;

export interface SearchHit {
  message_id: string;
  /** null: a direct chat with `kwant_id`. */
  channel_id: string | null;
  channel_name: string | null;
  kwant_id: string | null;
  author: string;
  /** Raw message text with `<mark>…</mark>` around hits. Not HTML-escaped. */
  snippet: string;
  created_at: string | null;
}

export interface Memory {
  id: string;
  text: string;
  source: 'user' | 'agent';
  created_at: string;
  updated_at: string;
}

export type Weekday = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun';
export const WEEKDAYS: Array<{ id: Weekday; label: string }> = [
  { id: 'mon', label: 'Mon' },
  { id: 'tue', label: 'Tue' },
  { id: 'wed', label: 'Wed' },
  { id: 'thu', label: 'Thu' },
  { id: 'fri', label: 'Fri' },
  { id: 'sat', label: 'Sat' },
  { id: 'sun', label: 'Sun' },
];

export type Schedule =
  | { type: 'once'; at: string }
  | { type: 'daily'; time: string; tz: string }
  | { type: 'weekly'; time: string; weekdays: Weekday[]; tz: string }
  | { type: 'interval'; every_minutes: number }
  /** Days of the month (1-31, or `'last'`); a day a month lacks runs on its last day. */
  | { type: 'monthly'; time: string; days: MonthDay[]; tz: string }
  /** Five fields: minute hour day-of-month month weekday. The server checks the rest. */
  | { type: 'cron'; expression: string; tz: string };

export type MonthDay = number | 'last';

export type RunStatus = 'running' | 'ok' | 'error' | 'skipped';

export interface Routine {
  id: string;
  kwant_id: string;
  /** null: the kwant's direct chat. */
  channel_id: string | null;
  name: string;
  instructions: string;
  schedule: Schedule;
  enabled: boolean;
  next_run_at: string | null;
  last_run_at: string | null;
  last_status: RunStatus | null;
  created_at: string;
  /** The schedule in the server's words, e.g. "monthly on day 1 at 09:00 Europe/Berlin". */
  schedule_text?: string;
  /** Set when it delivers into one of its bot's threads. */
  thread_id?: string | null;
  /** The message a follow-up checks back on. */
  context_message_id?: string | null;
}

export interface RoutineRun {
  id: string;
  routine_id: string;
  trigger: 'schedule' | 'manual';
  started_at: string;
  ended_at: string | null;
  status: RunStatus;
  error: string | null;
  message_id: string | null;
}

export interface RoutineInput {
  kwant_id: string;
  channel_id: string | null;
  name: string;
  instructions: string;
  schedule: Schedule;
  enabled: boolean;
}

export type NotifyKind =
  | 'turn_finished'
  | 'approval_needed'
  | 'routine_failed'
  | 'delegation_finished'
  | 'delegation_stalled'
  | 'skill_proposed'
  | 'question_asked'
  | 'human_step';
export const NOTIFY_KINDS: Array<{ id: NotifyKind; label: string }> = [
  { id: 'turn_finished', label: 'A bot finished replying' },
  { id: 'approval_needed', label: 'A bot needs your approval' },
  { id: 'routine_failed', label: 'A routine failed' },
  { id: 'delegation_finished', label: 'A delegated task finished' },
  { id: 'delegation_stalled', label: 'A delegated task is taking long' },
  { id: 'skill_proposed', label: 'A bot proposed a skill to review' },
  { id: 'question_asked', label: 'A bot has a question for you' },
  { id: 'human_step', label: 'A bot needs you in the browser' },
];

/** Kinds also shown as an in-app toast while kwnt is visible (the others only matter in the background). */
export const TOAST_KINDS: NotifyKind[] = ['routine_failed', 'delegation_finished', 'delegation_stalled', 'skill_proposed', 'question_asked', 'human_step'];

export interface NotifyEvent {
  kind: NotifyKind;
  kwant_id: string;
  kwant_name: string;
  channel_id: string | null;
  title: string;
  body: string;
  /** The space it happened in (servers with spaces). */
  space_id?: string;
  space_name?: string;
}

// ---------------------------------------------------------------------------
// Search snippets

export interface SnippetPart {
  text: string;
  hit: boolean;
}

/**
 * Splits a snippet into plain-text parts, marking the ones the server wrapped in `<mark>`.
 * Everything is rendered as text, so any other markup in a message stays inert; only `<mark>`
 * survives, as the `hit` flag.
 */
export function snippetParts(snippet: string): SnippetPart[] {
  const parts: SnippetPart[] = [];
  let hit = false;
  for (const piece of snippet.split(/(<\/?mark>)/)) {
    if (piece === '<mark>') hit = true;
    else if (piece === '</mark>') hit = false;
    else if (piece) {
      const last = parts[parts.length - 1];
      if (last && last.hit === hit) last.text += piece;
      else parts.push({ text: piece, hit });
    }
  }
  return parts;
}

const HTML_ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

/** The same rule as an HTML string: everything escaped, then only `<mark>`/`</mark>` re-allowed. */
export function sanitizeSnippet(snippet: string): string {
  return snippetParts(snippet)
    .map((p) => {
      const text = p.text.replace(/[&<>"']/g, (c) => HTML_ESCAPES[c]);
      return p.hit ? `<mark>${text}</mark>` : text;
    })
    .join('');
}

export interface HitGroup {
  key: string;
  channel_id: string | null;
  kwant_id: string | null;
  label: string;
  hits: SearchHit[];
}

/** Groups hits by where they were said, keeping the server's ranking (best group first). */
export function groupHits(hits: SearchHit[], kwantName: (id: string) => string | undefined): HitGroup[] {
  const groups = new Map<string, HitGroup>();
  for (const h of hits) {
    const key = h.channel_id ? `c:${h.channel_id}` : `k:${h.kwant_id ?? ''}`;
    let g = groups.get(key);
    if (!g) {
      const label = h.channel_id
        ? channelLabel(h.channel_name ?? h.channel_id)
        : `Direct chat · ${(h.kwant_id && kwantName(h.kwant_id)) || 'bot'}`;
      g = { key, channel_id: h.channel_id, kwant_id: h.channel_id ? null : h.kwant_id, label, hits: [] };
      groups.set(key, g);
    }
    g.hits.push(h);
  }
  return Array.from(groups.values());
}

export const channelLabel = (name: string) => (name.startsWith('team-') ? `@${name.slice(5)}` : `#${name}`);

// ---------------------------------------------------------------------------
// Routine schedules

export interface ScheduleForm {
  type: Schedule['type'];
  /** `datetime-local` value, in the browser's time zone. */
  onceAt: string;
  time: string;
  weekdays: Weekday[];
  tz: string;
  everyMinutes: string;
  monthDays: MonthDay[];
  /** A raw five-field rule, for the "Custom" choice. */
  cron: string;
}

/**
 * Browsers (ICU) still report some zones by their old names, which newer system tz databases
 * (e.g. Ubuntu 24.04 without tzdata-legacy) no longer ship, so the server would reject them.
 */
const LEGACY_TZ: Record<string, string> = {
  'Africa/Asmera': 'Africa/Asmara',
  'America/Buenos_Aires': 'America/Argentina/Buenos_Aires',
  'America/Catamarca': 'America/Argentina/Catamarca',
  'America/Cordoba': 'America/Argentina/Cordoba',
  'America/Godthab': 'America/Nuuk',
  'America/Indianapolis': 'America/Indiana/Indianapolis',
  'America/Jujuy': 'America/Argentina/Jujuy',
  'America/Louisville': 'America/Kentucky/Louisville',
  'America/Mendoza': 'America/Argentina/Mendoza',
  'Asia/Calcutta': 'Asia/Kolkata',
  'Asia/Katmandu': 'Asia/Kathmandu',
  'Asia/Rangoon': 'Asia/Yangon',
  'Asia/Saigon': 'Asia/Ho_Chi_Minh',
  'Atlantic/Faeroe': 'Atlantic/Faroe',
  'Europe/Kiev': 'Europe/Kyiv',
  'Pacific/Enderbury': 'Pacific/Kanton',
  'Pacific/Ponape': 'Pacific/Pohnpei',
  'Pacific/Truk': 'Pacific/Chuuk',
};

/** The current IANA name for a zone the browser may report by an old one. */
export const canonicalTimeZone = (tz: string) => LEGACY_TZ[tz] ?? tz;

/** The browser's IANA time zone, or UTC when it can't tell. */
export function defaultTimeZone(): string {
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return tz ? canonicalTimeZone(tz) : 'UTC';
  } catch {
    return 'UTC';
  }
}

const pad = (n: number) => String(n).padStart(2, '0');

/** A `datetime-local` value for a Date, in local time. */
export function toLocalInput(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function emptyScheduleForm(now = new Date(), tz = defaultTimeZone()): ScheduleForm {
  const soon = new Date(now.getTime() + 60 * 60 * 1000);
  soon.setMinutes(0, 0, 0);
  return {
    type: 'daily',
    onceAt: toLocalInput(soon),
    time: '09:00',
    weekdays: ['mon', 'tue', 'wed', 'thu', 'fri'],
    tz,
    everyMinutes: '60',
    monthDays: [1],
    cron: '0 9 * * 1-5',
  };
}

export function formFromSchedule(s: Schedule, now = new Date()): ScheduleForm {
  const base = emptyScheduleForm(now, 'tz' in s ? s.tz : defaultTimeZone());
  switch (s.type) {
    case 'once': {
      const at = new Date(s.at);
      return { ...base, type: 'once', onceAt: Number.isNaN(at.getTime()) ? base.onceAt : toLocalInput(at) };
    }
    case 'daily':
      return { ...base, type: 'daily', time: s.time };
    case 'weekly':
      return { ...base, type: 'weekly', time: s.time, weekdays: s.weekdays };
    case 'interval':
      return { ...base, type: 'interval', everyMinutes: String(s.every_minutes) };
    case 'monthly':
      return { ...base, type: 'monthly', time: s.time, monthDays: s.days.length ? sortMonthDays(s.days) : base.monthDays };
    case 'cron':
      return { ...base, type: 'cron', cron: s.expression };
    default:
      return base;
  }
}

/** 1, 2, … 31, then "last". */
export const sortMonthDays = (days: MonthDay[]): MonthDay[] =>
  Array.from(new Set(days)).sort((a, b) => (a === 'last' ? 32 : a) - (b === 'last' ? 32 : b));

/** The JSON the API takes, or a message for the first thing to fix. Mirrors the server's checks. */
export function scheduleFromForm(f: ScheduleForm, now = Date.now()): { schedule: Schedule } | { error: string } {
  switch (f.type) {
    case 'once': {
      const at = new Date(f.onceAt);
      if (!f.onceAt || Number.isNaN(at.getTime())) return { error: 'Pick a date and time.' };
      if (at.getTime() <= now) return { error: 'Pick a time in the future.' };
      return { schedule: { type: 'once', at: at.toISOString() } };
    }
    case 'daily':
    case 'weekly':
    case 'monthly': {
      if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(f.time)) return { error: 'Use a time like 09:00.' };
      const tz = canonicalTimeZone(f.tz.trim());
      if (!tz) return { error: 'Pick a time zone.' };
      if (f.type === 'daily') return { schedule: { type: 'daily', time: f.time, tz } };
      if (f.type === 'monthly') {
        const days = sortMonthDays(f.monthDays.filter((d) => d === 'last' || (Number.isInteger(d) && d >= 1 && d <= 31)));
        if (days.length === 0) return { error: 'Pick at least one day of the month.' };
        return { schedule: { type: 'monthly', time: f.time, days, tz } };
      }
      if (f.weekdays.length === 0) return { error: 'Pick at least one day.' };
      const days = WEEKDAYS.map((d) => d.id).filter((d) => f.weekdays.includes(d));
      return { schedule: { type: 'weekly', time: f.time, weekdays: days, tz } };
    }
    case 'interval': {
      const n = Number(f.everyMinutes);
      if (!Number.isInteger(n) || n < 5 || n > 10080) return { error: 'Repeat every 5 to 10080 minutes.' };
      return { schedule: { type: 'interval', every_minutes: n } };
    }
    case 'cron': {
      const expression = f.cron.trim().replace(/\s+/g, ' ');
      if (!expression) return { error: 'Type a rule, like 0 9 * * MON#2.' };
      if (expression.startsWith('@')) return { error: 'Shortcuts like @daily aren’t supported. Write the five parts instead.' };
      if (expression.split(' ').length !== 5) return { error: 'A rule has five parts: minute, hour, day of month, month and weekday.' };
      const tz = canonicalTimeZone(f.tz.trim());
      if (!tz) return { error: 'Pick a time zone.' };
      return { schedule: { type: 'cron', expression, tz } };
    }
  }
}

const ordinal = (n: number) => {
  const t = n % 100;
  if (t >= 11 && t <= 13) return `${n}th`;
  return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`;
};

/** "the 1st, 15th and last day". */
export function monthDaysText(days: MonthDay[]): string {
  const words = sortMonthDays(days).map((d) => (d === 'last' ? 'last day' : ordinal(d)));
  if (words.length === 0) return 'no day';
  const list = words.length > 1 ? `${words.slice(0, -1).join(', ')} and ${words[words.length - 1]}` : words[0];
  return `the ${list}`;
}

export function describeSchedule(s: Schedule): string {
  switch (s.type) {
    case 'once':
      return `Once, ${formatWhen(s.at)}`;
    case 'daily':
      return `Every day at ${s.time} (${s.tz})`;
    case 'weekly': {
      const days = s.weekdays.length === 7 ? 'every day' : s.weekdays.map((d) => WEEKDAYS.find((w) => w.id === d)?.label ?? d).join(', ');
      return `${days} at ${s.time} (${s.tz})`;
    }
    case 'interval':
      return s.every_minutes % 60 === 0
        ? `Every ${s.every_minutes / 60} h`
        : `Every ${s.every_minutes} min`;
    case 'monthly':
      return `Every month on ${monthDaysText(s.days)} at ${s.time} (${s.tz})`;
    case 'cron':
      return `Custom rule ${s.expression} (${s.tz})`;
    default:
      return 'Unknown schedule';
  }
}

export function formatWhen(iso: string | null): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '—';
  return d.toLocaleString(undefined, { weekday: 'short', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/** Time zones for the picker: the browser's list when it has one, with the current value kept. */
export function timeZoneOptions(current: string): string[] {
  let list: string[] = [];
  try {
    const intl = Intl as unknown as { supportedValuesOf?: (k: string) => string[] };
    list = intl.supportedValuesOf?.('timeZone') ?? [];
  } catch {
    list = [];
  }
  const all = new Set(['UTC'].concat(list.map(canonicalTimeZone)));
  if (current) all.add(current);
  return Array.from(all);
}

// ---------------------------------------------------------------------------
// Notifications

export const BROWSER_NOTIFY_KEY = 'kwnt.notify.browser';
export const MASK = '••••••••';

/** Only https webhooks; the saved mask (unchanged) and "" (remove) are fine too. */
export function webhookUrlError(url: string): string | null {
  const v = url.trim();
  if (!v || v === MASK) return null;
  let u: URL;
  try {
    u = new URL(v);
  } catch {
    return 'Enter a full URL, like https://ntfy.sh/your-topic.';
  }
  if (u.protocol !== 'https:') return 'Use an https:// URL.';
  if (u.username || u.password) return 'Leave the user name and password out of the URL.';
  return null;
}

/**
 * OS notifications only when the person turned them on here, granted permission, and isn't
 * looking at kwnt. The server already drops kinds unticked in `notify_on`.
 */
export function shouldShowOsNotification(opts: {
  enabled: boolean;
  permission: NotificationPermission | 'unsupported';
  hidden: boolean;
  kind: string;
}): boolean {
  return opts.enabled && opts.permission === 'granted' && opts.hidden && NOTIFY_KINDS.some((k) => k.id === opts.kind);
}

/** Whether this browser has OS notifications switched on for kwnt (stored per browser). */
export function browserNotifyEnabled(storage: Pick<Storage, 'getItem'> | undefined = globalThis.localStorage): boolean {
  try {
    return storage?.getItem(BROWSER_NOTIFY_KEY) === 'on';
  } catch {
    return false;
  }
}

export function notificationPermission(): NotificationPermission | 'unsupported' {
  return typeof Notification === 'undefined' ? 'unsupported' : Notification.permission;
}

// ---------------------------------------------------------------------------
// Live updates: App's single EventSource re-broadcasts these for whoever is open.

export type ContinuityEvent =
  | { type: 'memory_changed'; data: { kwant_id: string } }
  | { type: 'routine_run'; data: { routine_id: string; run_id: string; status: RunStatus } }
  | { type: 'skill_updated'; data: Skill | DeletedSkill };

const bus = typeof EventTarget !== 'undefined' ? new EventTarget() : null;

export function emitContinuity(e: ContinuityEvent) {
  bus?.dispatchEvent(new CustomEvent(e.type, { detail: e.data }));
}

export function onContinuity<T extends ContinuityEvent['type']>(
  type: T,
  fn: (data: Extract<ContinuityEvent, { type: T }>['data']) => void,
): () => void {
  const listener = (ev: Event) => fn((ev as CustomEvent).detail);
  bus?.addEventListener(type, listener);
  return () => bus?.removeEventListener(type, listener);
}

// ---------------------------------------------------------------------------
// API

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let message = res.status === 404 ? 'It no longer exists.' : `Request failed (HTTP ${res.status}).`;
    try {
      const data = await res.json();
      if (data && typeof data.error === 'string' && data.error) message = data.error;
    } catch {
      // no JSON body
    }
    throw new Error(message);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

const enc = encodeURIComponent;

export const continuityApi = {
  setTeamInstructions: (id: string, instructions: string) =>
    request<{ instructions: string | null }>('PATCH', `/api/teams/${enc(id)}`, { instructions }),
  setChannelInstructions: (id: string, instructions: string) =>
    request<{ instructions: string | null }>('PATCH', `/api/channels/${enc(id)}`, { instructions }),

  search: (q: string, opts: { channel_id?: string; kwant_id?: string; limit?: number } = {}) => {
    const p = new URLSearchParams({ q, limit: String(opts.limit ?? 30) });
    if (opts.channel_id) p.set('channel_id', opts.channel_id);
    if (opts.kwant_id) p.set('kwant_id', opts.kwant_id);
    return request<SearchHit[]>('GET', `/api/search?${p}`);
  },

  memories: (kwantId: string) => request<Memory[]>('GET', `/api/kwants/${enc(kwantId)}/memories`),
  addMemory: (kwantId: string, text: string) => request<Memory>('POST', `/api/kwants/${enc(kwantId)}/memories`, { text }),
  updateMemory: (kwantId: string, id: string, text: string) =>
    request<Memory>('PATCH', `/api/kwants/${enc(kwantId)}/memories/${enc(id)}`, { text }),
  removeMemory: (kwantId: string, id: string) => request<void>('DELETE', `/api/kwants/${enc(kwantId)}/memories/${enc(id)}`),

  routines: () => request<Routine[]>('GET', '/api/routines'),
  createRoutine: (body: RoutineInput) => request<Routine>('POST', '/api/routines', body),
  updateRoutine: (id: string, body: Partial<RoutineInput>) => request<Routine>('PATCH', `/api/routines/${enc(id)}`, body),
  removeRoutine: (id: string) => request<void>('DELETE', `/api/routines/${enc(id)}`),
  runRoutine: (id: string) => request<RoutineRun>('POST', `/api/routines/${enc(id)}/run`),
  routineRuns: (id: string, limit = 20) => request<RoutineRun[]>('GET', `/api/routines/${enc(id)}/runs?limit=${limit}`),
};
