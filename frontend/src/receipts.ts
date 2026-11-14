// "Download receipts" (Roadmap Next 7): the space's finished tasks as a CSV, from Lin's
// GET /api/tasks/receipts.csv (#611). The app's fetch wrapper adds `?space=`, so this file never does.
// The download steps copy downloadExport in imports.ts on purpose (Ada's brief: don't refactor it).

import { downloadName } from './imports';

/** The ranges the select offers, in days back from today (UTC). Default first. */
export const RECEIPT_RANGES = [
  { days: 30, label: 'Last 30 days' },
  { days: 90, label: 'Last 90 days' },
  { days: 365, label: 'Last 12 months' },
] as const;

export type ReceiptDays = (typeof RECEIPT_RANGES)[number]['days'];

export const RECEIPTS_FALLBACK_NAME = 'kwant-receipts.csv';

export const RECEIPTS_NETWORK_ERROR = 'Couldn’t reach the server. Check your connection and try again.';
export const RECEIPTS_OTHER_ERROR = 'The download didn’t work. Try again in a moment.';
export const RECEIPTS_TRUNCATED = 'Only the oldest 10,000 tasks fit. Pick a shorter range for the rest.';

const ymd = (d: Date) => d.toISOString().slice(0, 10);

/**
 * `from` and `to` as UTC days, both included by the server. `to` is today in UTC and `from` is `to`
 * minus `days`, the same window the route gives with no dates (Lin: "from defaults to to − 30 days").
 */
export function receiptsWindow(days: number, now: Date = new Date()): { from: string; to: string } {
  const to = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const from = new Date(to.getTime() - days * 86_400_000);
  return { from: ymd(from), to: ymd(to) };
}

export function receiptsUrl(days: number, now?: Date): string {
  const { from, to } = receiptsWindow(days, now);
  return `/api/tasks/receipts.csv?from=${from}&to=${to}&state=closed`;
}

export class ReceiptsError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
    this.name = 'ReceiptsError';
  }
}

/** What a refusal reads as: a 400 carries the server's own words inside the retry line (Mira, round 1). */
export function receiptsErrorText(status: number, serverText?: string | null): string {
  if (status === 0) return RECEIPTS_NETWORK_ERROR;
  if (status === 400 && serverText && serverText.trim()) return `The download didn’t work (${serverText.trim()}). Try again in a moment.`;
  return RECEIPTS_OTHER_ERROR;
}

export interface ReceiptsDownload {
  name: string;
  truncated: boolean;
}

/** Fetches the closed tasks of the last `days` and saves the CSV under the server's file name. */
export async function downloadReceipts(days: number, now?: Date): Promise<ReceiptsDownload> {
  let res: Response;
  try {
    res = await fetch(receiptsUrl(days, now));
  } catch {
    throw new ReceiptsError(RECEIPTS_NETWORK_ERROR, 0);
  }
  if (!res.ok) {
    let msg = '';
    if (res.status === 400) {
      try {
        msg = (await res.json())?.error ?? '';
      } catch {
        // no JSON body
      }
    }
    throw new ReceiptsError(receiptsErrorText(res.status, typeof msg === 'string' ? msg : ''), res.status);
  }
  const name = downloadName(res.headers.get('content-disposition'), RECEIPTS_FALLBACK_NAME);
  const truncated = (res.headers.get('x-kwant-truncated') ?? '').trim().toLowerCase() === 'true';
  let blob: Blob;
  try {
    blob = await res.blob();
  } catch {
    throw new ReceiptsError(RECEIPTS_NETWORK_ERROR, 0);
  }
  const href = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = href;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(href), 1000);
  return { name, truncated };
}
