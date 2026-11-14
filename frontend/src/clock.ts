// Times as people read them, in this device's locale and time zone: "8:15 PM" today, "Nov 3, 8:15 PM" on
// another day this year, with the year when it isn't this year. Every time the app shows goes through here
// (One clock, roadmap 13c): no forced 24 h, no forced en-GB, no hour cycle. The device decides.
// `locale` is for tests only; the app always passes nothing, which means the device's own.

type When = string | number | Date | null | undefined;

const toDate = (at: When): Date | null => {
  if (at === null || at === undefined || at === '') return null;
  const d = at instanceof Date ? at : new Date(at);
  return Number.isNaN(d.getTime()) ? null : d;
};

const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

// The locale's own short time: "8:15 PM" in en-US, "20:15" and "08:30" in en-GB. (`hour: 'numeric'` would write
// en-GB as "8:30", which is not how a 24-hour device shows it.)
const TIME = { timeStyle: 'short' } as const;

export interface ClockOpts {
  locale?: string;
  /** Only where a screen shows another zone on purpose (missionWatch tests, UTC day buckets). */
  timeZone?: string;
}

/** "8:15 PM" in en-US, "20:15" in en-GB. Null for a missing or unreadable time. */
export function timeText(at: When, opts: ClockOpts = {}): string | null {
  const d = toDate(at);
  if (!d) return null;
  // Some ICU builds (Node 20) put a narrow no-break space before AM/PM; browsers use a plain
  // space. One space everywhere keeps copy and tests the same on every runtime.
  return d
    .toLocaleTimeString(opts.locale, { ...TIME, ...(opts.timeZone ? { timeZone: opts.timeZone } : {}) })
    .replace(/\u202f/g, ' ');
}

/** "8:15:02 PM" / "20:15:02": the device's clock with seconds, for step-by-step logs. */
export function secondsText(at: When, opts: ClockOpts = {}): string | null {
  const d = toDate(at);
  if (!d) return null;
  return d
    .toLocaleTimeString(opts.locale, { timeStyle: 'medium', ...(opts.timeZone ? { timeZone: opts.timeZone } : {}) })
    .replace(/\u202f/g, ' ');
}

/** "Nov 3" / "3 Nov", with the weekday ("Tue, Nov 3") or year ("Nov 3, 2026") when asked. */
export function dateText(at: When, opts: ClockOpts & { weekday?: boolean; year?: boolean } = {}): string | null {
  const d = toDate(at);
  if (!d) return null;
  return d.toLocaleDateString(opts.locale, {
    ...(opts.weekday ? { weekday: 'short' } : {}),
    month: 'short',
    day: 'numeric',
    ...(opts.year ? { year: 'numeric' } : {}),
    ...(opts.timeZone ? { timeZone: opts.timeZone } : {}),
  });
}

/** "Mon, Nov 16, 8:30 AM" in en-US, "Mon 16 Nov, 08:30" in en-GB: a day and time with its weekday. */
export function weekdayClock(at: When, opts: ClockOpts & { year?: boolean } = {}): string | null {
  const d = toDate(at);
  if (!d) return null;
  return `${dateText(d, { ...opts, weekday: true })}, ${timeText(d, opts)}`;
}

/** "Nov 3, 8:15 PM" / "3 Nov, 20:15": a date and time, with the year when asked. */
export function dateClock(at: When, opts: ClockOpts & { year?: boolean } = {}): string | null {
  const d = toDate(at);
  if (!d) return null;
  return `${dateText(d, opts)}, ${timeText(d, opts)}`;
}

/** "8:15 PM" when `at` is today, else the date too. Null for a missing or unreadable time. */
export function localClock(at: When, now: number = Date.now(), locale?: string): string | null {
  const d = toDate(at);
  if (!d) return null;
  const today = new Date(now);
  const time = timeText(d, { locale })!;
  if (sameDay(d, today)) return time;
  return `${dateText(d, { locale, year: d.getFullYear() !== today.getFullYear() })}, ${time}`;
}

/** A stored wall-clock "HH:MM" (a routine's `time`) the way this device writes it: "8:30 AM" in en-US,
 *  "08:30" in en-GB. Anything that isn't HH:MM comes back as it was. */
export function wallClock(hhmm: string, locale?: string): string {
  const m = /^(\d{1,2}):(\d{2})$/.exec((hhmm ?? '').trim());
  if (!m) return hhmm;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return hhmm;
  return timeText(new Date(Date.UTC(2000, 0, 3, h, min)), { locale, timeZone: 'UTC' })!;
}

/** A Connections line: "Out of quota until 8:15 PM", or the server's words when there's no time. */
export function quotaLine(statusDetail: string | null, limitedUntil: string | null | undefined, now: number = Date.now(), locale?: string): string | null {
  const until = limitedUntil && new Date(limitedUntil).getTime() > now ? localClock(limitedUntil, now, locale) : null;
  return until ? `Out of quota until ${until}` : statusDetail;
}
