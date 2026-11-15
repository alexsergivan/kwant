// 0.9.0 P5: "Notes from bots" at the top of Connections. Bots file notes about their tools; the
// person edits, marks fixed, reopens or deletes them. Design: release-090/designs/p5-tool-notes.md.
import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Check, CircleX, Ellipsis, Pencil, Plus, StickyNote, Trash2, Undo2, X, Zap, CirclePlus } from 'lucide-react';
import { relativeTime } from '../away';
import { useEscapeLayer } from '../escape';
import { ConnectionsError } from '../connections';
import {
  FILTER_LABEL,
  KIND_LABEL,
  NOTE_FILTERS,
  NOTE_KINDS,
  NOTE_TEXT_MAX,
  NOTE_TOOL_MAX,
  NoteFilter,
  ToolNote,
  ToolNoteKind,
  ToolNotesList,
  ToolNoteWhere,
  cleanNoteText,
  notesQuery,
  toolLabel,
  toolNotesApi,
} from '../toolNotes';
import { timeText } from '../clock';

export type ToolNotesApi = Pick<typeof toolNotesApi, 'list' | 'add' | 'update' | 'remove'>;

const SHOWN = 5;

/**
 * What a failed save or delete says (D11): the server's own sentence for a refusal (409 "already
 * open", 400 "280 characters at most", 403 "Only you…"), the offline sentence when the server
 * couldn't be reached, and the generic line only otherwise.
 */
export function noteProblem(e: unknown, fallback: string): string {
  if (e instanceof ConnectionsError && e.message.trim() && (e.status === 0 || (e.status >= 400 && e.status < 500))) return e.message;
  return fallback;
}
const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const btn = `inline-flex items-center justify-center gap-1.5 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 ${focusRing}`;
const quietBtn = `${btn} min-h-[30px] px-2.5 border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const ghostBtn = `${btn} min-h-[30px] px-2.5 text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100`;
const iconBtn = `${btn} h-[30px] w-[30px] text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100`;
const primaryBtn = `${btn} min-h-[30px] px-3 bg-sky-500 text-zinc-950 hover:bg-sky-400`;

const KIND_TAG: Record<ToolNoteKind | 'fixed', string> = {
  broke: 'text-rose-300 border-rose-300/35 bg-rose-300/[0.08]',
  tip: 'text-teal-200 border-teal-400/30 bg-teal-400/[0.08]',
  missing: 'text-violet-300 border-[#9d8fff]/35 bg-[#9d8fff]/[0.08]',
  fixed: 'text-zinc-400 border-zinc-700 bg-transparent',
};
const KIND_ICON: Record<ToolNoteKind | 'fixed', React.ReactNode> = {
  broke: <CircleX size={11} aria-hidden="true" />,
  tip: <Zap size={11} aria-hidden="true" />,
  missing: <CirclePlus size={11} aria-hidden="true" />,
  fixed: <Check size={11} aria-hidden="true" />,
};

const KindTag: React.FC<{ kind: ToolNoteKind | 'fixed' }> = ({ kind }) => (
  <span className={`inline-flex items-center gap-1 h-5 px-1.5 rounded-md border text-[11px] font-semibold ${KIND_TAG[kind]}`} data-note-kind={kind}>
    {KIND_ICON[kind]}
    {kind === 'fixed' ? 'Fixed' : KIND_LABEL[kind]}
  </span>
);

const ToolChip: React.FC<{ n: ToolNote }> = ({ n }) => (
  <code className={`inline-flex items-center px-1.5 h-5 rounded-md border border-zinc-800 bg-zinc-900 font-mono text-xs break-all ${n.removed ? 'text-zinc-500' : 'text-zinc-200'}`}>
    {toolLabel(n)}
    {n.removed && <span className="ml-1 font-sans">(removed)</span>}
  </code>
);

/** The bots' small initial discs (14 px) and names; the person's own notes say so instead. */
const WhoLine: React.FC<{ n: ToolNote }> = ({ n }) =>
  n.written_by_person ? (
    <span>Written by you</span>
  ) : (
    <span className="inline-flex flex-wrap items-center gap-x-2 gap-y-0.5">
      {n.bots.map((b) => (
        <span key={b.id} className="inline-flex items-center gap-1 font-medium text-zinc-300">
          <span aria-hidden="true" className="inline-grid place-items-center h-3.5 w-3.5 rounded-full bg-[#9d8fff]/25 text-[9px] font-bold text-violet-200">
            {b.name.charAt(0).toUpperCase()}
          </span>
          {b.name}
        </span>
      ))}
    </span>
  );

/** "2:02 PM" today (the device's own clock), otherwise relative ("yesterday", "3 days ago"). */
const shortWhen = (ts: string | null, now: number): string => {
  if (!ts) return '';
  const t = Date.parse(ts);
  if (Number.isNaN(t)) return '';
  const d = new Date(t);
  const today = new Date(now);
  if (d.toDateString() === today.toDateString()) return timeText(d) ?? '';
  return relativeTime(ts, now);
};

// ---------------------------------------------------------------------------
// The editor: edit a note's text and kind, or add one (with a tool picker first)

const KindSwitch: React.FC<{ value: ToolNoteKind; onChange: (k: ToolNoteKind) => void; phone: boolean }> = ({ value, onChange, phone }) => {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const move = (e: React.KeyboardEvent, i: number) => {
    const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const next = (i + d + NOTE_KINDS.length) % NOTE_KINDS.length;
    onChange(NOTE_KINDS[next]);
    refs.current[next]?.focus();
  };
  return (
    <div role="radiogroup" aria-label="Kind of note" className="inline-flex rounded-lg border border-zinc-700 overflow-hidden">
      {NOTE_KINDS.map((k, i) => (
        <button
          key={k}
          ref={(el) => { refs.current[i] = el; }}
          type="button"
          role="radio"
          aria-checked={value === k}
          tabIndex={value === k ? 0 : -1}
          onClick={() => onChange(k)}
          onKeyDown={(e) => move(e, i)}
          data-kind-option={k}
          className={`${phone ? 'min-h-[44px]' : 'min-h-[30px]'} px-2.5 text-xs ${i ? 'border-l border-zinc-700' : ''} ${
            value === k ? 'bg-surface-overlay text-zinc-50' : 'text-zinc-300 hover:bg-zinc-800'
          } focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-500`}
        >
          {KIND_LABEL[k]}
        </button>
      ))}
    </div>
  );
};

export interface NoteEditorProps {
  /** Editing this note; none adds a new one. */
  note?: ToolNote;
  /** For adding: the MCP servers a note can be about. */
  connections?: string[];
  phone?: boolean;
  onSave: (body: { tool?: string; kind: ToolNoteKind; text: string }) => Promise<string | null>;
  onCancel: () => void;
  /** Tests: a save error already showing. */
  initialError?: string | null;
}

const BUILTIN = '__builtin__';

export const NoteEditor: React.FC<NoteEditorProps> = ({ note, connections = [], phone = false, onSave, onCancel, initialError = null }) => {
  const id = useId();
  const [text, setText] = useState(note?.text ?? '');
  const [kind, setKind] = useState<ToolNoteKind>(note?.kind ?? 'tip');
  const [server, setServer] = useState(connections[0] ?? BUILTIN);
  const [toolName, setToolName] = useState('');
  const [error, setError] = useState<string | null>(initialError);
  const [saving, setSaving] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);
  const length = [...text].length;
  const dirty = note ? text !== note.text || kind !== note.kind : !!text.trim() || !!toolName.trim();

  useEffect(() => {
    if (note) {
      const el = area.current;
      el?.focus();
      el?.setSelectionRange(el.value.length, el.value.length);
    }
  }, [note]);

  const save = async () => {
    if (saving) return;
    const cleaned = cleanNoteText(text);
    if ('error' in cleaned) return setError(cleaned.error);
    let tool: string | undefined;
    if (!note) {
      const t = toolName.trim();
      if (server === BUILTIN && !t) return setError('Name the built-in tool, like bash or browser.');
      tool = server === BUILTIN ? t : t ? `${server} · ${t}` : server;
      if ([...tool].length > NOTE_TOOL_MAX) return setError(`A tool name is ${NOTE_TOOL_MAX} characters at most.`);
    }
    setSaving(true);
    setError(null);
    const problem = await onSave({ tool, kind, text: cleaned.text });
    setSaving(false);
    if (problem) setError(problem);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape' && !e.defaultPrevented) {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
  };
  const tall = phone ? 'min-h-[44px]' : '';

  return (
    <div className="space-y-2" onKeyDown={onKey} data-note-editor={note ? note.id : 'new'}>
      {!note && (
        <div className="flex flex-wrap gap-2">
          <div className="min-w-0 basis-40 flex-1 space-y-1">
            <label htmlFor={`${id}-srv`} className="block text-[11px] font-medium text-zinc-300">
              Tool
            </label>
            <select
              id={`${id}-srv`}
              autoFocus
              value={server}
              onChange={(e) => setServer(e.target.value)}
              className={`w-full min-h-[36px] px-2 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 focus:outline-none focus:border-sky-500 ${tall}`}
            >
              {connections.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
              <option value={BUILTIN}>Built-in…</option>
            </select>
          </div>
          <div className="min-w-0 basis-40 flex-[2] space-y-1">
            <label htmlFor={`${id}-tool`} className="block text-[11px] font-medium text-zinc-300">
              Tool name {server !== BUILTIN && <span className="font-normal text-zinc-400">(optional)</span>}
            </label>
            <input
              id={`${id}-tool`}
              value={toolName}
              onChange={(e) => setToolName(e.target.value)}
              maxLength={NOTE_TOOL_MAX}
              autoComplete="off"
              spellCheck={false}
              placeholder={server === BUILTIN ? 'bash' : 'create_pull_request'}
              className={`w-full min-h-[36px] px-2.5 bg-zinc-950 border border-zinc-800 rounded-lg font-mono text-xs text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-500 ${tall}`}
            />
          </div>
        </div>
      )}
      <div className="relative">
        <label htmlFor={`${id}-t`} className="sr-only">
          Note text
        </label>
        <textarea
          id={`${id}-t`}
          ref={area}
          value={text}
          rows={2}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void save();
            }
          }}
          placeholder="What should the next bot know about this tool?"
          aria-describedby={`${id}-c ${id}-e`}
          aria-invalid={error ? true : undefined}
          className="block w-full resize-y min-h-[64px] px-3 pt-2.5 pb-6 bg-zinc-950 border border-zinc-700 rounded-lg text-[13.5px] leading-normal text-zinc-100 placeholder:text-zinc-500 focus:outline-none focus:border-sky-500"
        />
        <span id={`${id}-c`} className={`absolute right-2.5 bottom-1.5 text-[11px] tabular-nums ${length > NOTE_TEXT_MAX ? 'text-rose-300' : 'text-zinc-400'}`} aria-live="off">
          {length} / {NOTE_TEXT_MAX}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <KindSwitch value={kind} onChange={setKind} phone={phone} />
        <div className={`flex gap-2 ${phone ? 'w-full' : 'ml-auto'}`}>
          <button type="button" className={`${quietBtn} ${phone ? 'flex-1 min-h-[44px]' : ''}`} onClick={onCancel} data-dirty={dirty ? '' : undefined}>
            Cancel
          </button>
          <button type="button" className={`${primaryBtn} ${phone ? 'flex-1 min-h-[44px]' : ''}`} onClick={save} disabled={saving}>
            Save
          </button>
        </div>
      </div>
      <p id={`${id}-e`} role="alert" className="text-xs text-rose-300 empty:hidden">
        {error}
      </p>
    </div>
  );
};

// ---------------------------------------------------------------------------
// One note

export type RowMode = 'view' | 'edit' | 'confirm';

export interface NoteRowProps {
  n: ToolNote;
  now: number;
  phone: boolean;
  readOnly: boolean;
  mode: RowMode;
  leaving?: boolean;
  menuOpen?: boolean;
  onMode: (m: RowMode) => void;
  onMenu: (open: boolean) => void;
  onFix: (n: ToolNote, fixed: boolean) => void;
  onSave: (n: ToolNote, body: { kind: ToolNoteKind; text: string }) => Promise<string | null>;
  onDelete: (n: ToolNote) => void;
  onCopy: (n: ToolNote) => void;
  onOpenWhere?: (where: ToolNoteWhere) => void;
  /** Tests: a save error under the open editor. */
  editError?: string | null;
}

/** The desktop ⋯ menu (Open where it happened · Copy text · Delete). */
const RowMenu: React.FC<{ n: ToolNote; onClose: () => void; onWhere?: () => void; onCopy: () => void; onDelete: () => void }> = ({ n, onClose, onWhere, onCopy, onDelete }) => {
  const ref = useRef<HTMLDivElement>(null);
  useEscapeLayer(ref, true, onClose);
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('button')?.focus();
    const down = (e: PointerEvent) => !ref.current?.parentElement?.contains(e.target as Node) && onClose();
    window.addEventListener('pointerdown', down);
    return () => window.removeEventListener('pointerdown', down);
  }, [onClose]);
  const item = `w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-left text-xs hover:bg-zinc-800 focus-visible:bg-zinc-800 ${focusRing}`;
  return (
    <div ref={ref} role="menu" aria-label={`Actions for ${toolLabel(n)}`} className="absolute right-0 top-full mt-1 z-30 w-56 rounded-xl border border-zinc-700 bg-zinc-900 p-1.5 shadow-xl shadow-black/40">
      {onWhere && (
        <button type="button" role="menuitem" className={`${item} text-zinc-100`} onClick={() => (onClose(), onWhere())}>
          Open where it happened
        </button>
      )}
      <button type="button" role="menuitem" className={`${item} text-zinc-100`} onClick={() => (onClose(), onCopy())}>
        Copy text
      </button>
      <button type="button" role="menuitem" className={`${item} text-rose-300`} onClick={() => (onClose(), onDelete())}>
        Delete
      </button>
    </div>
  );
};

/** The phone ⋯ sheet: the tool and kind as the title, then 52 px rows. */
export const NoteSheet: React.FC<{
  n: ToolNote;
  onClose: () => void;
  onFix: () => void;
  onEdit: () => void;
  onWhere?: () => void;
  onDelete: () => void;
}> = ({ n, onClose, onFix, onEdit, onWhere, onDelete }) => {
  const id = useId();
  const ref = useRef<HTMLDivElement>(null);
  useEscapeLayer(ref, true, onClose, { modal: true });
  const fixed = n.state === 'fixed';
  const row = `w-full flex items-center gap-3 h-[52px] px-4 text-left text-sm border-t border-zinc-800 hover:bg-zinc-800/60 ${focusRing}`;
  const run = (f?: () => void) => () => (onClose(), f?.());
  return (
    <div className="fixed inset-0 z-50 flex items-end" data-note-sheet={n.id}>
      <button type="button" aria-label="Close" tabIndex={-1} className="absolute inset-0 bg-black/60" onClick={onClose} />
      <div ref={ref} role="dialog" aria-modal="true" aria-labelledby={`${id}-t`} className="relative w-full rounded-t-2xl border-t border-zinc-700 bg-zinc-900 pb-[env(safe-area-inset-bottom)]">
        <div className="px-4 pt-3 pb-3 space-y-1.5">
          <div aria-hidden="true" className="mx-auto mb-2 h-1 w-9 rounded-full bg-zinc-700" />
          <h4 id={`${id}-t`} className="flex flex-wrap items-center gap-1.5 text-sm text-zinc-100">
            <KindTag kind={fixed ? 'fixed' : n.kind} />
            <ToolChip n={n} />
          </h4>
        </div>
        <button type="button" className={`${row} text-zinc-100`} onClick={run(onFix)}>
          {fixed ? <Undo2 size={16} aria-hidden="true" /> : <Check size={16} aria-hidden="true" />}
          {fixed ? 'Reopen' : 'Mark fixed'}
        </button>
        {!fixed && (
          <button type="button" className={`${row} text-zinc-100`} onClick={run(onEdit)}>
            <Pencil size={16} aria-hidden="true" /> Edit
          </button>
        )}
        {onWhere && (
          <button type="button" className={`${row} text-zinc-100`} onClick={run(onWhere)}>
            <StickyNote size={16} aria-hidden="true" /> Open where it happened
          </button>
        )}
        <button type="button" className={`${row} text-rose-300`} onClick={run(onDelete)}>
          <Trash2 size={16} aria-hidden="true" /> Delete
        </button>
      </div>
    </div>
  );
};

export const NoteRow: React.FC<NoteRowProps> = ({ n, now, phone, readOnly, mode, leaving = false, menuOpen = false, onMode, onMenu, onFix, onSave, onDelete, onCopy, onOpenWhere, editError = null }) => {
  const fixed = n.state === 'fixed';
  const where = n.where && onOpenWhere ? () => onOpenWhere(n.where!) : undefined;
  const label = `${fixed ? 'Fixed' : KIND_LABEL[n.kind]}: ${toolLabel(n)}`;
  const editing = mode === 'edit' && !readOnly;
  // S1: when the editor closes (Esc, Cancel or Save), keyboard focus goes back to this note's
  // "Edit note" button (⋯ on the phone) instead of falling to <body>.
  const rowRef = useRef<HTMLLIElement>(null);
  const editBtn = useRef<HTMLButtonElement>(null);
  const moreBtn = useRef<HTMLButtonElement>(null);
  const wasEditing = useRef(editing);
  useEffect(() => {
    const closed = wasEditing.current && !editing;
    wasEditing.current = editing;
    if (!closed || mode !== 'view') return;
    const active = document.activeElement;
    if (active && active !== document.body && !rowRef.current?.contains(active)) return;
    (editBtn.current ?? moreBtn.current)?.focus();
  }, [editing, mode]);
  return (
    <li
      ref={rowRef}
      aria-label={label}
      data-note={n.id}
      data-note-state={n.state}
      className={`relative px-3.5 py-3 transition-opacity duration-[160ms] motion-reduce:transition-none ${leaving ? 'opacity-0' : ''} ${editing ? 'bg-[#9d8fff]/5' : ''}`}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <KindTag kind={fixed ? 'fixed' : n.kind} />
            <ToolChip n={n} />
            {n.count > 1 && (
              <span className="inline-flex items-center px-1 h-[18px] rounded bg-zinc-800 font-mono text-[11px] text-zinc-300" aria-label={`noted ${n.count} times`}>
                ×{n.count}
              </span>
            )}
            {n.edited_by_person && !n.written_by_person && (
              <span className="inline-flex items-center gap-1 h-5 px-1.5 rounded-md border border-zinc-700 text-[11px] text-zinc-300">
                <Pencil size={10} aria-hidden="true" /> Edited by you
              </span>
            )}
          </div>
          {editing ? (
            <NoteEditor note={n} phone={phone} initialError={editError} onCancel={() => onMode('view')} onSave={(b) => onSave(n, b)} />
          ) : (
            <p className={`text-[13.5px] leading-normal whitespace-pre-line break-words ${fixed ? 'text-zinc-400' : 'text-zinc-100'}`}>{n.text}</p>
          )}
          {!editing && (
            <p className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11.5px] text-zinc-400">
              <WhoLine n={n} />
              {n.last_at && <span>last {shortWhen(n.last_at, now)}</span>}
              {fixed && <span>Fixed by you{n.fixed_at ? ` · ${shortWhen(n.fixed_at, now)}` : ''} · bots no longer see it</span>}
            </p>
          )}
          {mode === 'confirm' && !readOnly && (
            <div className="mt-2 rounded-lg border border-zinc-700 bg-zinc-900 p-3 space-y-2.5" role="group" aria-label="Delete this note?">
              <p className="text-[13px] text-zinc-100">Delete this note? Bots stop seeing it, and it isn’t kept under Fixed.</p>
              <div className="flex gap-2">
                <button type="button" className={`${quietBtn} ${phone ? 'flex-1 min-h-[44px]' : ''}`} onClick={() => onMode('view')} autoFocus>
                  Cancel
                </button>
                <button type="button" className={`${btn} min-h-[30px] px-3 bg-rose-700 text-white hover:bg-rose-600 ${phone ? 'flex-1 min-h-[44px]' : ''}`} onClick={() => onDelete(n)} data-confirm-delete>
                  Delete
                </button>
              </div>
            </div>
          )}
        </div>
        {!readOnly && !editing && mode !== 'confirm' && (
          <div className="relative flex shrink-0 items-center gap-1">
            {phone ? (
              <button ref={moreBtn} type="button" className={`${btn} h-11 w-11 -my-2 -mr-2 text-zinc-300 hover:bg-zinc-800`} aria-label="More actions for this note" aria-haspopup="dialog" onClick={() => onMenu(true)}>
                <Ellipsis size={18} aria-hidden="true" />
              </button>
            ) : fixed ? (
              <>
                <button type="button" className={ghostBtn} onClick={() => onFix(n, false)}>
                  <Undo2 size={13} aria-hidden="true" /> Reopen
                </button>
                <button type="button" className={iconBtn} aria-label="Delete note" onClick={() => onMode('confirm')}>
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </>
            ) : (
              <>
                <button type="button" className={quietBtn} onClick={() => onFix(n, true)}>
                  <Check size={13} aria-hidden="true" /> Mark fixed
                </button>
                <button ref={editBtn} type="button" className={iconBtn} aria-label="Edit note" onClick={() => onMode('edit')}>
                  <Pencil size={14} aria-hidden="true" />
                </button>
                <button type="button" className={iconBtn} aria-label="More actions for this note" aria-haspopup="menu" aria-expanded={menuOpen} onClick={() => onMenu(!menuOpen)}>
                  <Ellipsis size={16} aria-hidden="true" />
                </button>
                {menuOpen && <RowMenu n={n} onClose={() => onMenu(false)} onWhere={where} onCopy={() => onCopy(n)} onDelete={() => onMode('confirm')} />}
              </>
            )}
          </div>
        )}
      </div>
      {phone && menuOpen && (
        <NoteSheet
          n={n}
          onClose={() => onMenu(false)}
          onFix={() => onFix(n, !fixed)}
          onEdit={() => onMode('edit')}
          onWhere={where}
          onDelete={() => onMode('confirm')}
        />
      )}
    </li>
  );
};

// ---------------------------------------------------------------------------
// The section

export interface ToolNotesSectionProps {
  api?: ToolNotesApi;
  /** MCP servers in this space, for the "Add a note" tool picker. */
  connections?: string[];
  /** Filter to one connection (from a row's chip); null for all. */
  connection?: string | null;
  onConnection?: (name: string | null) => void;
  /** "See the note" from a chat: show just this note. */
  focusNote?: string | null;
  onClearFocus?: () => void;
  /** Tells the page a note changed, so the row chips recount. */
  onChanged?: () => void;
  onOpenWhere?: (where: ToolNoteWhere) => void;
  phone?: boolean;
  readOnly?: boolean;
  now?: number;
  /** Goes up by one when a row's chip is clicked: scroll the section into view. */
  scrollRequest?: number;
  /** Tests and screenshots: a list already loaded, and rows in a given state. */
  initialList?: ToolNotesList | null;
  initialError?: string | null;
  initialFilter?: NoteFilter;
  initialMode?: Record<string, RowMode>;
  initialMenu?: string | null;
  initialAdding?: boolean;
  initialEditError?: string | null;
}

export const ToolNotesSection: React.FC<ToolNotesSectionProps> = ({
  api = toolNotesApi,
  connections = [],
  connection = null,
  onConnection,
  focusNote = null,
  onClearFocus,
  onChanged,
  onOpenWhere,
  phone = false,
  readOnly = false,
  now = Date.now(),
  scrollRequest = 0,
  initialList = null,
  initialError = null,
  initialFilter = 'all',
  initialMode = {},
  initialMenu = null,
  initialAdding = false,
  initialEditError = null,
}) => {
  const hid = useId();
  const [filter, setFilter] = useState<NoteFilter>(initialFilter);
  const [list, setList] = useState<ToolNotesList | null>(initialList);
  const [error, setError] = useState<string | null>(initialError);
  const [modes, setModes] = useState<Record<string, RowMode>>(initialMode);
  const [menu, setMenu] = useState<string | null>(initialMenu);
  const [adding, setAdding] = useState(initialAdding);
  const [leaving, setLeaving] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const [said, setSaid] = useState('');
  const [rowError, setRowError] = useState<string | null>(null);
  const sectionRef = useRef<HTMLElement>(null);
  const chipRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const seq = useRef(0);
  const query = notesQuery(filter, connection, focusNote);

  const load = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const r = await api.list(query);
      if (mine !== seq.current) return;
      setList(r);
      setError(null);
    } catch {
      if (mine === seq.current) setError('Couldn’t load notes.');
    }
  }, [api, query]);

  // A preloaded list (tests, screenshots) is shown as given.
  const preloaded = useRef(initialList !== null || initialError !== null);
  useEffect(() => {
    if (preloaded.current) {
      preloaded.current = false;
      return;
    }
    void load();
  }, [load]);

  useEffect(() => {
    if (!scrollRequest && !focusNote) return;
    sectionRef.current?.scrollIntoView?.({ behavior: 'smooth', block: 'start' });
  }, [scrollRequest, focusNote]);

  useEffect(() => setShowAll(false), [query]);

  const setMode = (id: string, m: RowMode) => {
    setRowError(null);
    setModes((x) => ({ ...x, [id]: m }));
  };

  const changed = () => {
    onChanged?.();
    return load();
  };

  const fix = async (n: ToolNote, fixed: boolean) => {
    setRowError(null);
    try {
      await api.update(n.id, { state: fixed ? 'fixed' : 'open' });
      // The row fades out of the list it no longer belongs to (static under reduced motion).
      if (!focusNote) {
        setLeaving(n.id);
        await new Promise((r) => setTimeout(r, 160));
      }
      setSaid(fixed ? 'Marked fixed. Bots no longer see it.' : 'Note reopened.');
      await changed();
    } catch (e) {
      setRowError(noteProblem(e, 'Couldn’t save the note. Try again.'));
    } finally {
      setLeaving(null);
    }
  };

  const saveEdit = async (n: ToolNote, body: { kind: ToolNoteKind; text: string }): Promise<string | null> => {
    const patch: { kind?: ToolNoteKind; text?: string } = {};
    if (body.text !== n.text) patch.text = body.text;
    if (body.kind !== n.kind) patch.kind = body.kind;
    if (!patch.text && !patch.kind) return setMode(n.id, 'view'), null;
    try {
      const saved = await api.update(n.id, patch);
      setList((l) => (l ? { ...l, notes: l.notes.map((x) => (x.id === n.id ? saved : x)) } : l));
      setMode(n.id, 'view');
      setSaid('Note saved.');
      void changed();
      return null;
    } catch (e) {
      return noteProblem(e, 'Couldn’t save the note. Try again.');
    }
  };

  const add = async (body: { tool?: string; kind: ToolNoteKind; text: string }): Promise<string | null> => {
    try {
      await api.add({ tool: body.tool ?? '', kind: body.kind, text: body.text });
      setAdding(false);
      setSaid('Note saved.');
      // Show it: the new note is open, so leave Fixed.
      if (filter === 'fixed' || (filter !== 'all' && filter !== body.kind)) setFilter('all');
      void changed();
      return null;
    } catch (e) {
      return noteProblem(e, 'Couldn’t save the note. Try again.');
    }
  };

  const remove = async (n: ToolNote) => {
    try {
      await api.remove(n.id);
      setSaid('Note deleted.');
      if (focusNote === n.id) onClearFocus?.();
      await changed();
    } catch (e) {
      setRowError(noteProblem(e, 'Couldn’t delete the note. Try again.'));
    }
  };

  const copy = async (n: ToolNote) => {
    try {
      await navigator.clipboard.writeText(n.text);
      setSaid('Copied.');
    } catch {
      setSaid('Couldn’t copy. Select the text instead.');
    }
  };

  const chooseFilter = (f: NoteFilter) => {
    setFilter(f);
    if (focusNote) onClearFocus?.();
  };
  const chipKey = (e: React.KeyboardEvent, i: number) => {
    const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    const to = e.key === 'Home' ? 0 : e.key === 'End' ? NOTE_FILTERS.length - 1 : d ? (i + d + NOTE_FILTERS.length) % NOTE_FILTERS.length : -1;
    if (to < 0) return;
    e.preventDefault();
    chooseFilter(NOTE_FILTERS[to]);
    chipRefs.current[to]?.focus();
  };

  const counts = list?.counts;
  const notes = list?.notes ?? [];
  const shown = showAll ? notes : notes.slice(0, SHOWN);
  const nothingEver = !!counts && counts.all === 0 && counts.fixed === 0 && !focusNote;
  const chip = (active: boolean) =>
    `inline-flex items-center gap-1.5 ${phone ? 'h-9' : 'h-7'} px-3 rounded-full border text-xs whitespace-nowrap shrink-0 ${focusRing} ${
      active ? 'bg-surface-overlay text-zinc-50 border-[#9d8fff]/45' : 'border-zinc-800 text-zinc-300 hover:bg-zinc-800/70'
    }`;
  const activeFilter = focusNote ? null : filter;

  return (
    <section ref={sectionRef} aria-labelledby={`${hid}-h`} className="space-y-2 scroll-mt-4" data-tool-notes>
      <div className="flex items-center gap-3">
        <h3 id={`${hid}-h`} className="flex items-center gap-2 text-xs font-semibold text-zinc-300 uppercase tracking-wider">
          <StickyNote size={14} aria-hidden="true" className="text-zinc-400" />
          Notes from bots
          {counts && <span className="normal-case tracking-normal font-normal text-zinc-400 tabular-nums">{counts.all} open</span>}
        </h3>
        {!readOnly && !adding && (
          <button type="button" className={`${ghostBtn} ml-auto ${phone ? 'min-h-[44px]' : ''}`} onClick={() => (setAdding(true), setRowError(null))}>
            <Plus size={13} aria-hidden="true" /> Add a note
          </button>
        )}
      </div>
      <p className="text-xs text-zinc-400">
        What bots found out about their tools. Open notes are shown to every bot that has the tool, so it doesn’t trip twice. Mark a note fixed when it no longer holds.
      </p>

      <div role="status" aria-live="polite" className="sr-only" data-notes-said>
        {said}
      </div>

      {error && (
        <p className="flex flex-wrap items-center gap-2 px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 text-xs text-rose-300" data-notes-error>
          {error}
          <button type="button" onClick={() => void load()} className={`ml-auto px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 ${focusRing}`}>
            Retry
          </button>
        </p>
      )}

      {counts && !nothingEver && (
        <div className={`flex items-center gap-2 ${phone ? 'overflow-x-auto -mx-4 px-4 pb-1 [scrollbar-width:none]' : 'flex-wrap'}`}>
          <div role="radiogroup" aria-label="Filter notes" className="flex items-center gap-2">
            {NOTE_FILTERS.map((f, i) => {
              const on = activeFilter === f;
              const tab = activeFilter === null ? i === 0 : on;
              return (
                <button
                  key={f}
                  ref={(el) => (chipRefs.current[i] = el)}
                  type="button"
                  role="radio"
                  aria-checked={on}
                  tabIndex={tab ? 0 : -1}
                  onClick={() => chooseFilter(f)}
                  onKeyDown={(e) => chipKey(e, i)}
                  className={chip(on)}
                  data-note-filter={f}
                >
                  {FILTER_LABEL[f]}
                  <span className="tabular-nums text-zinc-400">{counts[f]}</span>
                </button>
              );
            })}
          </div>
          {connection && !focusNote && (
            <button type="button" className={chip(true)} onClick={() => onConnection?.(null)} aria-label={`Show notes for every tool, not only ${connection}`} data-note-connection={connection}>
              {connection}
              <X size={12} aria-hidden="true" />
            </button>
          )}
          {focusNote && (
            <button type="button" className={chip(true)} onClick={onClearFocus} aria-label="Show every note, not only the one from the chat" data-note-focus={focusNote}>
              Note from the chat
              <X size={12} aria-hidden="true" />
            </button>
          )}
        </div>
      )}

      {rowError && (
        <p role="alert" className="text-xs text-rose-300">
          {rowError}
        </p>
      )}

      {list && (adding || notes.length > 0) && (
        <ul className="rounded-xl border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800" aria-label="Notes">
          {adding && (
            <li className="px-3.5 py-3 bg-[#9d8fff]/5" aria-label="New note">
              <NoteEditor connections={connections} phone={phone} onCancel={() => setAdding(false)} onSave={add} initialError={initialEditError} />
            </li>
          )}
          {shown.map((n) => (
            <NoteRow
              key={n.id}
              n={n}
              now={now}
              phone={phone}
              readOnly={readOnly}
              mode={modes[n.id] ?? 'view'}
              leaving={leaving === n.id}
              menuOpen={menu === n.id}
              onMode={(m) => (setMenu(null), setMode(n.id, m))}
              onMenu={(o) => setMenu(o ? n.id : null)}
              onFix={fix}
              onSave={saveEdit}
              onDelete={remove}
              onCopy={copy}
              onOpenWhere={onOpenWhere}
              editError={modes[n.id] === 'edit' ? initialEditError : null}
            />
          ))}
        </ul>
      )}

      {list && notes.length > SHOWN && !showAll && (
        <button type="button" className={ghostBtn} onClick={() => setShowAll(true)}>
          Show {notes.length - SHOWN} more
        </button>
      )}

      {list && notes.length === 0 && !adding && (
        <p className="px-3 py-3 rounded-xl border border-dashed border-zinc-800 text-xs text-zinc-400" data-notes-empty={nothingEver ? 'never' : filter === 'all' ? 'none-open' : 'filter'}>
          {nothingEver
            ? 'No notes yet. When a tool fails, surprises a bot or is missing, the bot writes it down here.'
            : focusNote
              ? 'That note is gone.'
              : filter === 'all'
                ? 'No open notes.'
                : 'No notes of this kind.'}
        </p>
      )}
    </section>
  );
};
