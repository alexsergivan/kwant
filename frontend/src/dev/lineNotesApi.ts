// 0.9.0 G1: the line-notes routes (Juno, release-090/g1-api.md). The card's Changes section only talks to
// this interface, so tests and the shot harness pass a fake one. Every answer goes through the parsers in
// lineNotes.ts; an answer the app can't read is an error with code `bad_shape`.

import type { Changes, LinePoint, Note, NotesPage, SendResult } from './lineNotes';
import { parseChanges, parseNote, parseNotes, parseSendResult } from './lineNotes';

/** A failed call: `code` is Juno's machine code (ask 7), `status` the HTTP status (0: no connection). */
export class LineNotesError extends Error {
  constructor(message: string, public status: number, public code: string | null = null, public data: Record<string, unknown> | null = null) {
    super(message);
  }
}

export interface NewNote {
  path: string;
  start: LinePoint;
  end: LinePoint;
  /** The head the person was looking at (ask 4: 409 stale_head when the card moved on). */
  head_sha: string | null;
  text: string;
}

export interface LineNotesApi {
  changes(cardId: string): Promise<Changes>;
  notes(cardId: string): Promise<NotesPage>;
  create(cardId: string, n: NewNote): Promise<Note>;
  edit(noteId: string, text: string): Promise<Note>;
  remove(noteId: string): Promise<void>;
  restore(noteId: string): Promise<Note>;
  discardAll(cardId: string): Promise<number>;
  /** `sendId` makes a Try again after a timeout safe (ask 6). */
  send(cardId: string, sendId: string, ids?: string[]): Promise<SendResult>;
}

/** (url, method, body) → parsed JSON, or throws LineNotesError. */
export type Caller = (url: string, method: string, body?: unknown) => Promise<unknown>;

const enc = encodeURIComponent;

/** The words for a code the person can act on; anything else is the server's own sentence or a generic one. */
export function errorWords(code: string | null, fallback: string | null, status: number): string {
  switch (code) {
    case 'too_many':
      return 'Up to 20 notes per send';
    case 'too_long':
      return 'A note can be up to 4,000 characters.';
    case 'range_too_big':
      return 'A note can cover up to 200 lines.';
    case 'empty':
      return 'Say what to change first.';
    case 'stale_head':
      return 'The bot pushed again. The changes reloaded; your note is still here.';
    case 'no_author':
      return 'This PR has no bot to send the notes to.';
    case 'pr_merged':
      return 'This PR is merged. Notes are read-only.';
    case 'pr_closed':
      return 'This PR is closed. Notes are read-only.';
    case 'not_draft':
      return 'This note was already sent.';
    case 'forge_failed':
      return 'Couldn’t load the changes.';
    case 'cross_hunk':
      return 'A note can cover lines from one block of changes only.';
    case 'not_found':
      return 'That note is gone. It can only be restored for a minute after deleting.';
    case 'send_id_reused':
      return 'These notes changed while sending. Try sending again.';
    case 'bad_request':
      return 'Something went wrong sending this. Try again.';
  }
  if (status === 0) return 'Couldn’t reach the server. Check your connection and try again.';
  return fallback && !/^[a-z_]+$/.test(fallback) ? fallback : `Something went wrong (HTTP ${status}).`;
}

/** The default caller: same-origin fetch with the person's session (the app passes personFetch). */
export function fetchCaller(send: typeof fetch): Caller {
  return async (url, method, body) => {
    let res: Response;
    try {
      res = await send(url, {
        method,
        cache: 'no-store',
        headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch {
      throw new LineNotesError(errorWords(null, null, 0), 0);
    }
    const data: unknown = await res.json().catch(() => null);
    if (res.ok) return data;
    const o = data && typeof data === 'object' ? (data as Record<string, unknown>) : {};
    const code = typeof o.code === 'string' ? o.code : typeof o.error === 'string' && /^[a-z_]+$/.test(o.error) ? o.error : null;
    throw new LineNotesError(errorWords(code, typeof o.error === 'string' ? o.error : null, res.status), res.status, code, o);
  };
}

const shape = () => new LineNotesError('The server sent notes in a shape this app doesn’t know.', 200, 'bad_shape');
function need<T>(v: T | null): T {
  if (v === null) throw shape();
  return v;
}

/** Ask 3: start and end each name their side, so a range may cross added and removed lines. */
const createBody = (n: NewNote) => ({ path: n.path, start: n.start, end: n.end, head_sha: n.head_sha, text: n.text });

export function lineNotesApi(call: Caller): LineNotesApi {
  const card = (id: string) => `/api/dev/cards/${enc(id)}`;
  const note = (id: string) => `/api/dev/notes/${enc(id)}`;
  return {
    changes: async (id) => need(parseChanges(await call(`${card(id)}/changes`, 'GET'))),
    notes: async (id) => need(parseNotes(await call(`${card(id)}/notes`, 'GET'))),
    create: async (id, n) => need(parseNote(await call(`${card(id)}/notes`, 'POST', createBody(n)))),
    edit: async (id, text) => need(parseNote(await call(note(id), 'PATCH', { text }))),
    remove: async (id) => {
      await call(note(id), 'DELETE');
    },
    restore: async (id) => need(parseNote(await call(`${note(id)}/restore`, 'POST', {}))),
    discardAll: async (id) => {
      const r = await call(`${card(id)}/notes`, 'DELETE');
      const n = r && typeof r === 'object' ? (r as Record<string, unknown>).discarded : null;
      return typeof n === 'number' && Number.isInteger(n) && n >= 0 ? n : 0;
    },
    send: async (id, sendId, ids) =>
      need(parseSendResult(await call(`${card(id)}/notes/send`, 'POST', ids ? { ids, send_id: sendId } : { send_id: sendId }))),
  };
}

/** A send id per batch: a Try again reuses it, a new batch gets a new one. */
export function newSendId(): string {
  const c = (globalThis as { crypto?: Crypto }).crypto;
  if (c?.randomUUID) return c.randomUUID();
  return `snd_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}
