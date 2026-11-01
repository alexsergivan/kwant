// Times as people read them, in this device's time zone: "8:15 PM" today, "Nov 3, 8:15 PM" on
// another day this year, with the year when it isn't this year.

const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** "8:15 PM" when `at` is today, else the date too. Null for a missing or unreadable time. */
export function localClock(at: string | Date | null | undefined, now: number = Date.now(), locale?: string): string | null {
  if (!at) return null;
  const d = at instanceof Date ? at : new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  const today = new Date(now);
  const time = d.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' });
  if (sameDay(d, today)) return time;
  const date = d.toLocaleDateString(locale, {
    month: 'short',
    day: 'numeric',
    ...(d.getFullYear() === today.getFullYear() ? {} : { year: 'numeric' }),
  });
  return `${date}, ${time}`;
}

/** A Connections line: "Out of quota until 8:15 PM", or the server's words when there's no time. */
export function quotaLine(statusDetail: string | null, limitedUntil: string | null | undefined, now: number = Date.now(), locale?: string): string | null {
  const until = limitedUntil && new Date(limitedUntil).getTime() > now ? localClock(limitedUntil, now, locale) : null;
  return until ? `Out of quota until ${until}` : statusDetail;
}
