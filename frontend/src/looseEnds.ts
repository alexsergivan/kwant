// Loose ends (Zombie Mission Hunter): things the person started and never finished, found
// read-only in their own conversations. Every route is person-only. Nothing here sends anything:
// a case carries a draft the person copies and sends themselves.
//
// The card is not tied to mail: `source`, `conv_kind` and `kind` are shown with a readable
// fallback, so later sources (a photographed letter, a bank statement) need no new card.

import { PersonCancelledError, personFetch } from './auth';
import { needsBitwardenFrom } from './passwordStore';

export interface Evidence {
  quote: string;
  from: string;
  at: string;
  outbound: boolean;
}

export interface CaseDraft {
  /** `email`, `github`, `kwant` or `letter` (a paper reply: copy only). Where the person sends it themselves. */
  channel: string;
  to: string | null;
  subject: string | null;
  body: string;
  mailto: string | null;
  open_url: string | null;
  kwant_thread_id: string | null;
  /** `template` or `model`. */
  generated_by: string;
  created_at: string;
}

export type CaseStatus = 'open' | 'snoozed' | 'dismissed' | 'waiting_reply' | 'closed';

export interface LooseCase {
  id: string;
  status: CaseStatus;
  kind: string;
  conv_kind: string;
  source: string;
  source_id: string;
  title: string;
  counterpart: string | null;
  counterpart_name: string | null;
  reference: string | null;
  link: string | null;
  why: string;
  evidence: Evidence[];
  draft: CaseDraft | null;
  age_days: number;
  last_activity_at: string;
  snoozed_until: string | null;
  waiting_since: string | null;
  follow_ups: number;
  /** `YYYY-MM-DD`. */
  deadline: string | null;
  resolution: string | null;
  dismiss_reason: string | null;
  /** The bot whose chat a `kwant_*` case is in; `null` for other sources and older cases. */
  kwant_id?: string | null;
}

export interface LooseSource {
  id: string;
  kind: string;
  label: string;
  config: Record<string, unknown>;
  enabled: boolean;
  builtin: boolean;
  last_scan_at: string | null;
  last_status: string | null;
  last_error: string | null;
}

export type Tone = 'amber' | 'emerald' | 'rose' | 'sky' | 'zinc';

const KINDS: Record<string, { label: string; tone: Tone }> = {
  awaiting_reply: { label: 'No reply yet', tone: 'amber' },
  promise_lapsed: { label: 'Promise passed', tone: 'rose' },
  you_promised: { label: 'You promised', tone: 'sky' },
  you_owe_reply: { label: 'You owe a reply', tone: 'sky' },
  stale_issue: { label: 'Gone quiet', tone: 'zinc' },
  failed_task: { label: 'Task failed', tone: 'rose' },
  unanswered_question: { label: 'Question open', tone: 'amber' },
  document_request: { label: 'Letter to deal with', tone: 'sky' },
};

const SOURCES: Record<string, string> = { imap: 'Email', github: 'GitHub', kwant: 'Bot chats', document: 'Letter' };

/** `bank_statement` → `Bank statement`: how an unknown kind or source still reads. */
export function humanize(s: string): string {
  const t = s.replace(/[_-]+/g, ' ').trim();
  return t ? t[0].toUpperCase() + t.slice(1) : 'Other';
}

export const kindLabel = (kind: string): { label: string; tone: Tone } => KINDS[kind] ?? { label: humanize(kind), tone: 'zinc' };
export const sourceLabel = (source: string): string => SOURCES[source] ?? humanize(source);

/** Which list a status belongs in. */
export type CaseTab = 'open' | 'snoozed' | 'waiting' | 'done';
export const CASE_TABS: Array<{ key: CaseTab; label: string; empty: string }> = [
  { key: 'open', label: 'Open', empty: 'Nothing is stalled. Loose ends show up here when something goes quiet.' },
  { key: 'waiting', label: 'Waiting', empty: 'Nothing you followed up on is waiting for an answer.' },
  { key: 'snoozed', label: 'Snoozed', empty: 'Nothing is snoozed.' },
  { key: 'done', label: 'Done', empty: 'Nothing marked done or dismissed yet.' },
];
export function tabOf(status: string): CaseTab {
  if (status === 'snoozed') return 'snoozed';
  if (status === 'waiting_reply') return 'waiting';
  if (status === 'closed' || status === 'dismissed') return 'done';
  return 'open';
}

export type CaseAction = 'draft' | 'snooze' | 'dismiss' | 'done' | 'followed-up' | 'reopen';

/** The buttons a case offers in its status. */
export function actionsFor(c: Pick<LooseCase, 'status' | 'draft'>): CaseAction[] {
  switch (c.status) {
    case 'open':
      return [...(c.draft ? (['draft'] as CaseAction[]) : []), 'followed-up', 'snooze', 'dismiss', 'done'];
    case 'snoozed':
    case 'waiting_reply':
      return ['reopen', 'done', 'dismiss'];
    default:
      return ['reopen'];
  }
}

export const ACTION_LABELS: Record<CaseAction, string> = {
  draft: 'Draft again',
  snooze: 'Snooze',
  dismiss: 'Dismiss',
  done: 'Mark done',
  'followed-up': 'I followed up',
  reopen: 'Reopen',
};

/** What changed, for the status line after an action. */
export function actionDoneText(a: CaseAction, c: LooseCase): string {
  const t = `“${c.title}”`;
  switch (a) {
    case 'draft':
      return `New draft for ${t}.`;
    case 'snooze':
      return `Snoozed ${t}${c.snoozed_until ? ` until ${shortDate(c.snoozed_until)}` : ''}.`;
    case 'dismiss':
      return `Dismissed ${t}. It won’t come back.`;
    case 'done':
      return `Marked ${t} done.`;
    case 'followed-up':
      return `${t} is waiting for an answer. It comes back if nobody replies.`;
    case 'reopen':
      return `Reopened ${t}.`;
  }
}

export const shortDate = (iso: string): string => {
  const d = new Date(iso.length === 10 ? `${iso}T12:00:00` : iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
};

export const ageText = (days: number): string => (days <= 0 ? 'today' : days === 1 ? '1 day quiet' : `${days} days quiet`);

/** A promised date: "was due 3 Oct 2026" once passed, else "due …". */
export function deadlineText(deadline: string, now = Date.now()): { text: string; passed: boolean } {
  const end = new Date(`${deadline}T23:59:59`).getTime();
  const passed = !Number.isNaN(end) && end < now;
  return { text: `${passed ? 'Was due' : 'Due'} ${shortDate(deadline)}`, passed };
}

/** Only web links open from a card; anything else (kwant://, javascript:) is shown, not linked. */
export const safeHref = (url: string | null | undefined, scheme: 'https' | 'mailto' = 'https'): string | null =>
  url && url.toLowerCase().startsWith(`${scheme}:`) ? url : null;

/** The bot chat a case came from, to open in the app. Needs both the bot (`kwant_id`, missing on
 * cases found before it was added) and the thread (`kwant://thread/<id>` in `link`). */
export function caseChat(c: Pick<LooseCase, 'kwant_id' | 'link' | 'draft'>): { kwantId: string; threadId: string } | null {
  const fromLink = c.link?.startsWith('kwant://thread/') ? c.link.slice('kwant://thread/'.length) : null;
  const threadId = fromLink || c.draft?.kwant_thread_id || null;
  return c.kwant_id && threadId ? { kwantId: c.kwant_id, threadId } : null;
}

/** The text "Copy draft" puts on the clipboard. */
export const draftCopyText = (d: CaseDraft): string => (d.subject ? `Subject: ${d.subject}\n\n${d.body}` : d.body);

/** Where the person sends a draft themselves. */
export function draftOpen(d: CaseDraft): { href: string; label: string } | null {
  const mail = safeHref(d.mailto, 'mailto');
  if (mail) return { href: mail, label: 'Open in my mail app' };
  const web = safeHref(d.open_url);
  if (web) return { href: web, label: d.channel === 'github' ? 'Open on GitHub' : 'Open' };
  return null;
}

/** Why a call failed, as the panel shows it. */
export class LooseEndsError extends Error {
  constructor(
    message: string,
    public status: number,
    /** 409: approvals are off and the login would sit on this server; Bitwarden first. */
    public needsBitwarden = false,
    /** 423: Bitwarden holds the logins and is locked. */
    public locked = false,
    /** The server's `code`, e.g. `busy` on a 409. */
    public code: string | null = null,
  ) {
    super(message);
  }
}

const sentence = (s: string) => {
  const t = s.trim();
  if (!t) return t;
  const c = t[0].toUpperCase() + t.slice(1);
  return /[.!?]$/.test(c) ? c : `${c}.`;
};

export async function call<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new LooseEndsError(e.message, 401);
    throw new LooseEndsError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const answer = await res.json().catch(() => null);
  if (res.ok) return answer as T;
  const needs = needsBitwardenFrom(res.status, answer);
  if (needs) throw new LooseEndsError(needs.message, 409, true);
  const err = answer && typeof answer === 'object' ? (answer as Record<string, unknown>).error : null;
  const code = answer && typeof answer === 'object' && typeof (answer as Record<string, unknown>).code === 'string' ? ((answer as Record<string, unknown>).code as string) : null;
  const msg = typeof err === 'string' && err.trim() ? sentence(err) : `Something went wrong (HTTP ${res.status}).`;
  if (res.status === 423) throw new LooseEndsError(msg, 423, false, true);
  if (res.status === 403) throw new LooseEndsError('Only you can see and change loose ends.', 403);
  if (res.status === 404 && !err) throw new LooseEndsError('This server doesn’t have loose ends yet. Update the app and try again.', 404);
  throw new LooseEndsError(msg, res.status, false, false, code);
}

const enc = encodeURIComponent;

export interface NewSource {
  kind: 'imap' | 'github';
  label: string;
  config: Record<string, unknown>;
  secret: string;
}

export const looseApi = {
  cases: () => call<{ cases: LooseCase[]; running: boolean }>('/api/zombie/cases'),
  act: (id: string, action: CaseAction, body?: unknown) => call<LooseCase>(`/api/zombie/cases/${enc(id)}/${action}`, 'POST', body ?? {}),
  sources: () => call<{ sources: LooseSource[]; vault_locked: boolean }>('/api/zombie/sources'),
  addSource: (s: NewSource) => call<{ source: LooseSource; test: unknown }>('/api/zombie/sources', 'POST', s),
  setEnabled: (id: string, enabled: boolean) => call<{ source: LooseSource }>(`/api/zombie/sources/${enc(id)}`, 'PATCH', { enabled }),
  removeSource: (id: string) => call<{ deleted: boolean; cases_removed: number }>(`/api/zombie/sources/${enc(id)}`, 'DELETE'),
  testSource: (id: string) => call<{ ok: boolean; test: unknown }>(`/api/zombie/sources/${enc(id)}/test`, 'POST'),
  run: () => call<{ started: boolean }>('/api/zombie/run', 'POST', {}),
  status: () => call<{ running: boolean; next_review_at: string | null }>('/api/zombie/status'),
};

/** IMAP over plain text or with a self-signed certificate is only for a server on this computer. */
export const isLoopbackHost = (host: string): boolean => {
  const h = host.trim().replace(/^\[|\]$/g, '').toLowerCase();
  return h === 'localhost' || h === '::1' || /^127\./.test(h);
};

export interface ImapFields {
  label: string;
  host: string;
  port: string;
  security: 'tls' | 'starttls' | 'plain';
  username: string;
  folders: string;
  accept_invalid_certs: boolean;
}

/** The IMAP form as the API wants it, or why it can't be sent yet. */
export function imapSource(f: ImapFields, secret: string): NewSource | { error: string } {
  const host = f.host.trim();
  if (!host) return { error: 'Enter the mail server, like imap.gmail.com.' };
  if (!f.username.trim()) return { error: 'Enter the mailbox login, usually your email address.' };
  if (!secret) return { error: 'Enter the password. For Gmail, use an app password.' };
  const port = Number(f.port || (f.security === 'tls' ? 993 : 143));
  if (!Number.isInteger(port) || port < 1 || port > 65535) return { error: 'The port must be a number like 993.' };
  const local = isLoopbackHost(host);
  if (f.security === 'plain' && !local) return { error: 'An unencrypted connection is only allowed to a server on this computer.' };
  return {
    kind: 'imap',
    label: f.label.trim() || f.username.trim(),
    config: {
      host,
      port,
      security: f.security,
      username: f.username.trim(),
      folders: f.folders
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
      accept_invalid_certs: local && f.accept_invalid_certs,
    },
    secret,
  };
}

export function githubSource(label: string, apiBase: string, token: string): NewSource | { error: string } {
  if (!token) return { error: 'Enter a GitHub token. It only needs to read issues and pull requests.' };
  const base = apiBase.trim() || 'https://api.github.com';
  return { kind: 'github', label: label.trim() || 'GitHub', config: { api_base: base }, secret: token };
}

/** One line for a source's last scan. */
export function sourceStatusText(s: LooseSource): { text: string; error: boolean } {
  if (!s.enabled) return { text: 'Off', error: false };
  if (s.last_status === 'error' || s.last_error) return { text: s.last_error ? sentence(s.last_error) : 'The last scan failed.', error: true };
  if (s.last_scan_at) return { text: `Last read ${shortDate(s.last_scan_at)}`, error: false };
  return { text: 'Not read yet', error: false };
}
