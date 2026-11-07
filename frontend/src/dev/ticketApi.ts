// 0.7.6 "Work from Jira": the card's calls. Routes are Juno's api.md 2.2 (`/api/spaces/:id/dev/…`), Vega's
// brief.md (`…/tickets/:key/brief`, `defaults`, `ok`) and api-decisions.md: answer (13), PUT brief (14),
// ask/preview and ask (16), stop/skip/close/retry (12). Every 4xx is `{error, code}`. Every answer goes
// through the parsers in tickets.ts.

import { personFetch } from '../auth';
import { DevModeError } from '../devMode';
import type { AskPosted, AskPreview, Brief, JiraSite, Ticket, TicketEvent } from './tickets';
import { askBody, briefFallbackText, parseAskPosted, parseAskPreview, parseBrief, parseJiraSite, parseTicket, parseTicketEvents, parseTicketList } from './tickets';

const enc = encodeURIComponent;

/** Juno's 409 `jira_off` (Start, Retry and Ask while Work from Jira is Off), in the card's words. */
export const JIRA_OFF = 'Work from Jira is off. Turn it on in Settings to start this ticket.';
const base = (space: string) => `/api/spaces/${enc(space)}/dev`;

async function call(url: string, method = 'GET', body?: unknown): Promise<unknown> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new DevModeError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const data: unknown = await res.json().catch(() => null);
  if (res.ok) return data;
  const o = data && typeof data === 'object' ? (data as Record<string, unknown>) : {};
  const code = typeof o.code === 'string' ? o.code : typeof o.error === 'string' && /^[a-z_]+$/.test(o.error) ? o.error : null;
  const msg =
    code === 'jira_off'
      ? JIRA_OFF
      : code === 'wrong_state'
      ? 'The ticket moved on in the meantime. The card shows where it is now.'
      : code === 'already_asked' || code === 'asked'
      ? 'This was already asked on Jira.'
      : code === 'changed'
        ? 'The questions changed. Check the new comment before posting.'
        : typeof o.error === 'string' && o.error.trim() && !/^[a-z_]+$/.test(o.error)
          ? o.error
          : `Something went wrong (HTTP ${res.status}).`;
  throw new DevModeError(msg, res.status, code);
}

/** `null` when Jira isn't connected in this space (Off: no Tickets tab): 200 `{site: null}`, or 404 from an older server. */
export async function getJiraSite(space: string): Promise<JiraSite | null> {
  try {
    return parseJiraSite(await call(`${base(space)}/jira`));
  } catch (e) {
    if (e instanceof DevModeError && e.status === 404) return null;
    throw e;
  }
}

export const listTickets = async (space: string) => parseTicketList(await call(`${base(space)}/tickets`));

export async function getTicket(space: string, key: string): Promise<{ ticket: Ticket | null; events: TicketEvent[] }> {
  const d = await call(`${base(space)}/tickets/${enc(key)}`);
  const o = d && typeof d === 'object' ? (d as Record<string, unknown>) : {};
  return { ticket: parseTicket(o.ticket), events: parseTicketEvents(o.events) };
}

export async function getBrief(space: string, key: string): Promise<{ brief: Brief | null; text: string | null }> {
  try {
    const d = await call(`${base(space)}/tickets/${enc(key)}/brief`);
    return { brief: parseBrief(d), text: briefFallbackText(d) };
  } catch (e) {
    if (e instanceof DevModeError && e.status === 404) return { brief: null, text: null };
    throw e;
  }
}

export const startTicket = async (space: string, key: string) => parseTicket(await call(`${base(space)}/tickets/${enc(key)}/start`, 'POST'));
export const okBrief = (space: string, key: string) => call(`${base(space)}/tickets/${enc(key)}/ok`, 'POST');
export const goWithDefaults = (space: string, key: string) => call(`${base(space)}/tickets/${enc(key)}/defaults`, 'POST');
/** api-decisions 13. */
export const answerQuestion = (space: string, key: string, question_id: string, value: string) =>
  call(`${base(space)}/tickets/${enc(key)}/answer`, 'POST', { question_id, value });

export async function askPreview(space: string, key: string, brief: Brief, team: string): Promise<AskPreview> {
  const p = parseAskPreview(await call(`${base(space)}/tickets/${enc(key)}/ask/preview`, 'POST', askBody(brief, team)));
  if (!p) throw new DevModeError('The server sent no comment to preview.', 0);
  return p;
}

/** Posts the exact previewed comment; the server checks `digest` and answers 409 `changed` if it moved. */
export const askPost = async (space: string, key: string, brief: Brief, team: string, p: AskPreview): Promise<AskPosted> =>
  parseAskPosted(await call(`${base(space)}/tickets/${enc(key)}/ask`, 'POST', askBody(brief, team, p.digest)));

/** api-decisions 14: the person's edit; the server sets `edited_by_person_at` and the card says "Edited by you". */
export const editBrief = async (space: string, key: string, brief: unknown) => parseBrief(await call(`${base(space)}/tickets/${enc(key)}/brief`, 'PUT', brief));

/**
 * api-decisions 12: none of these writes to Jira. Each answers with the updated row, or 409 `wrong_state`.
 * Stop pauses the work, Skip closes a waiting ticket (`skipped`), Close closes the card, Retry reads again.
 */
const lifecycle = (verb: 'stop' | 'skip' | 'close' | 'retry') => async (space: string, key: string) =>
  parseTicket(await call(`${base(space)}/tickets/${enc(key)}/${verb}`, 'POST'));
export const stopTicket = lifecycle('stop');
export const skipTicket = lifecycle('skip');
export const closeTicket = lifecycle('close');
export const retryTicket = lifecycle('retry');
