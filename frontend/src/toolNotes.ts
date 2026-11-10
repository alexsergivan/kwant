// 0.9.0 P5: bots' notes about their tools. A bot files a note (didn't work, tip, missing); the
// person checks, edits, marks fixed or deletes it on Connections. Contract:
// release-090/p5-contract.md (Lin), design release-090/designs/p5-tool-notes.md (Mira).

import { ConnectionsError, connectionsApi } from './connections';
import { parseMetaObject } from './questions';

export type ToolNoteKind = 'broke' | 'tip' | 'missing';
export type ToolNoteState = 'open' | 'fixed';
export const NOTE_KINDS: ToolNoteKind[] = ['broke', 'tip', 'missing'];
export const NOTE_TEXT_MAX = 280;
export const NOTE_TOOL_MAX = 80;

export interface ToolNoteWhere {
  channel_id: string | null;
  direct_kwant_id: string | null;
  thread_id: string | null;
  message_id: string | null;
}

/** One note, as GET/POST/PATCH /api/tool-notes give it (contract §1). */
export interface ToolNote {
  id: string;
  tool: string;
  connection: string | null;
  builtin: boolean;
  removed: boolean;
  kind: ToolNoteKind;
  state: ToolNoteState;
  text: string;
  bot_text: string | null;
  edited_by_person: boolean;
  written_by_person: boolean;
  count: number;
  bots: Array<{ id: string; name: string }>;
  created_at: string;
  last_at: string;
  updated_at: string | null;
  fixed_at: string | null;
  purge_at: string | null;
  where: ToolNoteWhere | null;
}

export interface ToolNoteCounts {
  all: number;
  broke: number;
  tip: number;
  missing: number;
  fixed: number;
}

export interface ToolNotesList {
  notes: ToolNote[];
  counts: ToolNoteCounts;
}

/** The filter chips: open notes of every kind, one kind, or the fixed ones. */
export type NoteFilter = 'all' | ToolNoteKind | 'fixed';
export const NOTE_FILTERS: NoteFilter[] = ['all', 'broke', 'tip', 'missing', 'fixed'];

export const KIND_LABEL: Record<ToolNoteKind, string> = { broke: 'Didn’t work', tip: 'Tip', missing: 'Missing' };
export const FILTER_LABEL: Record<NoteFilter, string> = { all: 'All', ...KIND_LABEL, fixed: 'Fixed' };

const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);
const num = (v: unknown, d = 0): number => (typeof v === 'number' && Number.isFinite(v) ? v : d);
const isKind = (v: unknown): v is ToolNoteKind => NOTE_KINDS.includes(v as ToolNoteKind);

/** Reads one note, defensively; null when it has no id or tool. Unknown kinds read as "tip" (the quietest). */
export function parseToolNote(raw: unknown): ToolNote | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const r = raw as Record<string, unknown>;
  const id = str(r.id);
  const tool = str(r.tool);
  if (!id || !tool) return null;
  const connection = str(r.connection);
  const w = r.where && typeof r.where === 'object' ? (r.where as Record<string, unknown>) : null;
  const where: ToolNoteWhere | null = w
    ? { channel_id: str(w.channel_id), direct_kwant_id: str(w.direct_kwant_id), thread_id: str(w.thread_id), message_id: str(w.message_id) }
    : null;
  const bots = Array.isArray(r.bots)
    ? r.bots.flatMap((b) => {
        const o = b && typeof b === 'object' ? (b as Record<string, unknown>) : {};
        const bid = str(o.id);
        return bid ? [{ id: bid, name: str(o.name) ?? 'A bot' }] : [];
      })
    : [];
  return {
    id,
    tool,
    connection,
    builtin: r.builtin === true || !connection,
    removed: r.removed === true,
    kind: isKind(r.kind) ? r.kind : 'tip',
    state: r.state === 'fixed' ? 'fixed' : 'open',
    text: typeof r.text === 'string' ? r.text : '',
    bot_text: str(r.bot_text),
    edited_by_person: r.edited_by_person === true,
    written_by_person: r.written_by_person === true,
    count: Math.max(1, Math.round(num(r.count, 1))),
    bots,
    created_at: str(r.created_at) ?? '',
    last_at: str(r.last_at) ?? str(r.created_at) ?? '',
    updated_at: str(r.updated_at),
    fixed_at: str(r.fixed_at),
    purge_at: str(r.purge_at),
    where: where && (where.channel_id || where.direct_kwant_id) ? where : null,
  };
}

export function parseToolNotesList(raw: unknown): ToolNotesList {
  const r = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const notes = Array.isArray(r.notes) ? r.notes.map(parseToolNote).filter((n): n is ToolNote => !!n) : [];
  const c = r.counts && typeof r.counts === 'object' ? (r.counts as Record<string, unknown>) : {};
  return { notes, counts: { all: num(c.all), broke: num(c.broke), tip: num(c.tip), missing: num(c.missing), fixed: num(c.fixed) } };
}

/** How the tool reads in a row: built-in tools say so (contract §1 `builtin`). */
export const toolLabel = (n: Pick<ToolNote, 'tool' | 'builtin'>): string => (n.builtin && !/^built-in · /i.test(n.tool) ? `Built-in · ${n.tool}` : n.tool);

/** The chip on a connection row: "1 note" · "{n} notes". */
export const notesChipText = (open: number): string => (open === 1 ? '1 note' : `${open} notes`);

/** The query for one list, from the filter and an optional connection. */
export function notesQuery(filter: NoteFilter, connection: string | null, note: string | null = null): string {
  const p = new URLSearchParams();
  if (note) p.set('note', note);
  else {
    p.set('state', filter === 'fixed' ? 'fixed' : 'open');
    if (filter !== 'all' && filter !== 'fixed') p.set('kind', filter);
    if (connection) p.set('connection', connection);
  }
  return p.toString();
}

/** A text the person typed: trimmed, at most 4 lines and 280 characters; null when empty. */
export function cleanNoteText(raw: string): { text: string } | { error: string } {
  const lines = raw.replace(/\r\n?/g, '\n').trim().split('\n');
  const text = lines.slice(0, 4).join('\n').trim();
  if (!text) return { error: 'Write what the bots should know.' };
  if ([...text].length > NOTE_TEXT_MAX) return { error: `A note is ${NOTE_TEXT_MAX} characters at most.` };
  return { text };
}

/** The chat line a bot posts when it files or repeats a note (meta kind "tool_note", contract §2.3). */
export interface ToolNoteMeta {
  kind: 'tool_note';
  note_id: string;
  kwant_id: string | null;
  tool: string;
  note_kind: ToolNoteKind;
  repeat: boolean;
  count: number;
}

export function parseToolNoteMeta(raw: unknown): ToolNoteMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'tool_note') return null;
  const id = str(m.note_id);
  const tool = str(m.tool);
  if (!id || !tool) return null;
  return {
    kind: 'tool_note',
    note_id: id,
    kwant_id: str(m.kwant_id),
    tool,
    note_kind: isKind(m.note_kind) ? m.note_kind : 'tip',
    repeat: m.repeat === true,
    count: Math.max(1, Math.round(num(m.count, 1))),
  };
}

/** The words around the tool in the chat line (design copy table); the tool itself is drawn in mono. */
export function chatLineParts(bot: string, meta: Pick<ToolNoteMeta, 'note_kind' | 'repeat'>): [string, string] {
  const again = meta.repeat ? ' again' : '';
  if (meta.note_kind === 'broke') return [`${bot} noted${again} that `, ' didn’t work.'];
  if (meta.note_kind === 'missing') return [`${bot} noted${again} that it was missing `, '.'];
  return [`${bot} noted${again} a tip for `, '.'];
}

const enc = encodeURIComponent;

async function req<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  // The same person-only fetch and error words as Connections; `?space=` is added by the fetch wrapper.
  return connectionsApi.raw<T>(url, method, body, 'This server doesn’t keep tool notes yet. Update the app and try again.');
}

export const toolNotesApi = {
  list: async (query: string): Promise<ToolNotesList> => parseToolNotesList(await req<unknown>(`/api/tool-notes?${query}`)),
  add: async (body: { tool: string; kind: ToolNoteKind; text: string }): Promise<ToolNote> => {
    const n = parseToolNote(await req<unknown>('/api/tool-notes', 'POST', body));
    if (!n) throw new ConnectionsError('The server answered with something that isn’t a note.', 500);
    return n;
  },
  update: async (id: string, body: Partial<{ text: string; kind: ToolNoteKind; state: ToolNoteState }>): Promise<ToolNote> => {
    const n = parseToolNote(await req<unknown>(`/api/tool-notes/${enc(id)}`, 'PATCH', body));
    if (!n) throw new ConnectionsError('The server answered with something that isn’t a note.', 500);
    return n;
  },
  remove: (id: string): Promise<unknown> => req<unknown>(`/api/tool-notes/${enc(id)}`, 'DELETE'),
};
