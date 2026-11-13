/**
 * Waiting checks (0.9.7, #564): a Mission Control check whose command exited 75, or whose http
 * reply said "not ready yet", waits on an outside service. The task card and the task detail show
 * it as one line, "Waiting on <label> since 09:12 · next look 09:27", plus the last line the
 * script printed. Past its deadline it stops and tells you once in Needs you.
 *
 * Everything here reads the server's fields as they are: no new numbers, only clock times and the
 * hours between the server's own `waiting_since` and `wait_until`.
 */
import type { Check, Task } from './missions';
import { clockTime } from './failover';

type WaitFields = Pick<Check, 'kind' | 'spec' | 'status' | 'output_excerpt' | 'waiting_since' | 'next_run_at' | 'wait_until' | 'timed_out_at'>;

/** Whether the check waits on an outside service now, or stopped waiting at its deadline. */
export const isWaiting = (c: Pick<Check, 'status' | 'timed_out_at'>): boolean => c.status === 'waiting' || (c.status === 'error' && !!c.timed_out_at);

const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);

/**
 * What the check waits on, as the server names it in Needs you (`check_label`): the command, the
 * address, or the file. The spec in task JSON is already masked by the server.
 */
export function waitLabel(c: Pick<Check, 'kind' | 'spec'>): string {
  const s = c.spec ?? {};
  const what = c.kind === 'command' ? str(s.command) : c.kind === 'http' ? str(s.url) : str(s.path);
  return what ?? c.kind;
}

/** A label cut to `max` characters with an ellipsis, so the line never runs off a phone card. */
export function shortLabel(label: string, max = 48): string {
  const chars = [...label];
  return chars.length > max ? `${chars.slice(0, max - 1).join('')}…` : label;
}

/**
 * The last line the script printed. The server stores a waiting run's output as
 * "run N: not ready yet\n<output>", so that first line is kwant's, not the script's.
 */
export function lastPrinted(excerpt: string | null | undefined): string | null {
  if (!excerpt) return null;
  const lines = excerpt.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  if (lines.length && /^run \d+: not ready yet$/.test(lines[0])) lines.shift();
  return lines.length ? lines[lines.length - 1] : null;
}

const at = (iso: string | null | undefined): Date | null => {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : new Date(t);
};

export interface WaitLine {
  /** Full label, for the title and screen readers. */
  label: string;
  /** "since 09:12 · next look 09:27" (waiting), or "didn't answer in 72 h. Verify the task by hand." (stopped). */
  rest: string;
  /** Waiting: "If it isn't ready by Sun 09:12, it stops and tells you." Stopped: null. */
  deadline: string | null;
  /** The last line the script printed, or null. */
  last: string | null;
  timedOut: boolean;
}

/** The one line for a waiting check, or null when it isn't waiting. */
export function waitLine(c: WaitFields, now: number = Date.now()): WaitLine | null {
  if (!isWaiting(c)) return null;
  const label = waitLabel(c);
  const last = lastPrinted(c.output_excerpt);
  const since = at(c.waiting_since);
  const until = at(c.wait_until);
  if (c.status === 'error') {
    const hours = since && until ? Math.max(1, Math.round((until.getTime() - since.getTime()) / 3_600_000)) : null;
    return { label, rest: `${hours ? `didn’t answer in ${hours} h` : 'didn’t answer in time'}. Verify the task by hand.`, deadline: null, last, timedOut: true };
  }
  const next = at(c.next_run_at);
  const parts = [since ? `since ${clockTime(since, now)}` : null, next ? `next look ${next.getTime() <= now ? 'soon' : clockTime(next, now)}` : null].filter(Boolean);
  return {
    label,
    rest: parts.join(' · '),
    deadline: until ? `If it isn’t ready by ${clockTime(until, now)}, it stops and tells you.` : null,
    last,
    timedOut: false,
  };
}

/** The whole line as one sentence, for screen readers and tests. */
export function waitLineText(w: WaitLine): string {
  if (w.timedOut) return `${w.label} ${w.rest}`;
  return `Waiting on ${w.label}${w.rest ? ` ${w.rest}` : ''}`;
}

/** Whether the card should look up its waiting checks: it is in review and the summary counts some. */
export const cardWaits = (t: Pick<Task, 'state' | 'checks_summary'>): boolean => t.state === 'review' && (t.checks_summary.waiting ?? 0) > 0;

/** Before the card has its checks: "Waiting on an outside service" (or "on 2 outside services"). */
export function waitingFallback(n: number): string {
  return n > 1 ? `Waiting on ${n} outside services` : 'Waiting on an outside service';
}

/** When to look the card's checks up again: shortly after the earliest next look, or null. */
export function nextRefresh(checks: Pick<Check, 'status' | 'next_run_at'>[], now: number = Date.now()): number | null {
  const due = checks
    .filter((c) => c.status === 'waiting')
    .map((c) => Date.parse(c.next_run_at ?? ''))
    .filter((t) => !Number.isNaN(t));
  if (!due.length) return null;
  // A minute after the run is due (the missions tick runs it then), and never sooner than 30 s.
  return Math.max(30_000, Math.min(...due) + 60_000 - now);
}
