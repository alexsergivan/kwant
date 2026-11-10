// 0.9.0 G1: the review card's Changes section (Mira, release-090/designs/g1-line-notes.md). The unified
// diff, a gutter + per line, ranges, notes kept as drafts on the server, and one send bar that sends them
// to the PR's author bot as one message. Sent notes say "Changed in {sha}" or "Still open" after the bot's
// next push. Everything goes through the injected LineNotesApi (lineNotesApi.ts), so tests and the shot
// harness pass a fake one. Text from the server is rendered as text, never as HTML.

import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { AlertTriangle, ArrowUp, Check, ChevronDown, ChevronRight, Code2, Info, Loader2, MessageSquare, MoreHorizontal, Pencil, Plus, Trash2 } from 'lucide-react';
import { ModernAvatar } from '../ModernAvatar';
import { DiffStat, OutLink } from './parts';
import { clock } from '../../dev/logic';
import { isMacPlatform } from '../../browserInput';
import { useMedia } from '../../useMedia';
import type { ChangedFile, Changes, DiffLine, FlatLine, LinePoint, Note, NotesSummary } from '../../dev/lineNotes';
import {
  codeText,
  LOAD_FAILED,
  MAX_SEND,
  NO_DIFF,
  SEND_FAILED,
  TOO_MANY,
  addAria,
  addedSay,
  changedIn,
  clampRange,
  cutLine,
  discardAsk,
  filesWord,
  flatLines,
  goneLine,
  indexOfPoint,
  lineWords,
  movedLine,
  notBotLine,
  notesWord,
  draftsWord,
  pointOf,
  positionIn,
  readOnlyLine,
  resolvedSay,
  sendLabel,
  sendShort,
  sentLine,
  sentNoteLine,
  showLines,
  shownNo,
  startsCollapsed,
  stillOpen,
  summaryLine,
} from '../../dev/lineNotes';
import { LineNotesError, newSendId, type LineNotesApi } from '../../dev/lineNotesApi';

/** The app re-sends the `dev_card` SSE as this window event (DevDesk.tsx DEV_CARD_EVENT). */
const DEV_CARD_EVENT = 'kwant:dev-card';
const SENT_STAYS_MS = 8000;
const UNDO_MS = 5000;
const RESOLVED_FOLDS_MS = 3000;

const RING = 'outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-300';
const HEAD = 'text-[11px] font-bold uppercase tracking-[.06em] text-zinc-400';
const ICON_BTN = `inline-flex h-6 w-6 items-center justify-center rounded-md text-zinc-400 hover:bg-white/[.06] hover:text-zinc-100 max-sm:h-11 max-sm:w-11 ${RING}`;

export interface ChangesSectionProps {
  cardId: string;
  api: LineNotesApi;
  /** The card's own counts (ask 1), so the collapsed row needs no fetch. */
  stat: { files: number; additions: number; deletions: number };
  notesCount?: { drafts: number; total: number } | null;
  /** The card's head; a new one reloads the changes while they're open. */
  headSha?: string | null;
  prUrl?: string | null;
  /** Opens the author bot's 1:1 chat after a send. */
  onOpenChat?: (botId: string) => void;
  /** Shots and tests: start open. */
  defaultOpen?: boolean;
  /** Tests: force the phone layout (default: the 640 px breakpoint). */
  phone?: boolean;
}

type Load = { s: 'idle' } | { s: 'loading' } | { s: 'error' } | { s: 'ready'; changes: Changes };
/** A note being written: where (by anchor, so a reload keeps it), the code it quotes, and the text. */
interface Writing {
  path: string;
  start: LinePoint;
  end: LinePoint;
  code: DiffLine[];
  text: string;
  /** Editing a draft in place. */
  editing: string | null;
  asking: boolean;
  busy: boolean;
  error: string | null;
}
type Bar =
  | { s: 'idle' }
  | { s: 'confirm' }
  | { s: 'sending' }
  | { s: 'sent'; n: number; bot: string; botId: string | null; at: string | null }
  | { s: 'failed' };

const marker = (k: FlatLine['line']['kind']) => (k === 'add' ? '+' : k === 'del' ? '-' : ' ');
const quote = (lines: FlatLine[], a: number, b: number): DiffLine[] => lines.slice(a, b + 1).map((x) => x.line);
const base = (p: string) => p.split('/').pop() || p;

export const ChangesSection: React.FC<ChangesSectionProps> = ({ cardId, api, stat, notesCount = null, headSha = null, prUrl = null, onOpenChat, defaultOpen = false, phone: forcePhone }) => {
  const media = useMedia('(max-width: 639px)');
  const phone = forcePhone ?? media;
  const ids = useId();
  const bodyId = `${ids}-changes`;
  const [open, setOpen] = useState(defaultOpen);
  const [load, setLoad] = useState<Load>({ s: 'idle' });
  const [notes, setNotes] = useState<Note[] | null>(null);
  const [summary, setSummary] = useState<NotesSummary | null>(null);
  const [fileOpen, setFileOpen] = useState<Record<string, boolean>>({});
  const [sel, setSel] = useState<{ path: string; anchor: number; from: number; to: number } | null>(null);
  const [writing, setWriting] = useState<Writing | null>(null);
  const [rangeMode, setRangeMode] = useState(false);
  const [bar, setBar] = useState<Bar>({ s: 'idle' });
  const [deleted, setDeleted] = useState<{ id: string; note: Note } | null>(null);
  const [folded, setFolded] = useState<Set<string>>(() => new Set());
  const [said, setSaid] = useState('');
  const sendId = useRef<string | null>(null);
  const lastSummary = useRef<NotesSummary | null>(null);
  const loadedHead = useRef<string | null>(null);

  const changes = load.s === 'ready' ? load.changes : null;
  const bot = changes?.author_bot?.name ?? null;
  const canNote = !!changes && !!changes.author_bot && changes.pr_state === 'open';
  const drafts = useMemo(() => (notes ?? []).filter((n) => n.state === 'draft'), [notes]);

  // -------------------------------------------------------------------------------------------------
  // Loading

  const fetchNotes = useCallback(async () => {
    const page = await api.notes(cardId);
    setNotes(page.notes);
    // Say it once when the bot's push changed the count (not on the first load).
    const before = lastSummary.current;
    const s = page.summary;
    if (s && s.total > 0 && s.sha && before && before.sha !== s.sha) setSaid(resolvedSay(s));
    lastSummary.current = s;
    setSummary(s);
  }, [api, cardId]);

  const fetchAll = useCallback(async () => {
    setLoad({ s: 'loading' });
    try {
      const [c] = await Promise.all([api.changes(cardId), fetchNotes()]);
      loadedHead.current = c.head_sha;
      setFileOpen(Object.fromEntries(c.files.map((f) => [f.path, !startsCollapsed(f)])));
      setLoad({ s: 'ready', changes: c });
    } catch {
      setLoad({ s: 'error' });
    }
  }, [api, cardId, fetchNotes]);

  useEffect(() => {
    if (open && load.s === 'idle') void fetchAll();
  }, [open, load.s, fetchAll]);

  // Ask 2: the `dev_card` event refetches the notes; a new head refetches the changes too.
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ id?: string }>).detail;
      if (!d || d.id !== cardId || load.s !== 'ready') return;
      void fetchNotes().catch(() => undefined);
    };
    window.addEventListener(DEV_CARD_EVENT, on);
    return () => window.removeEventListener(DEV_CARD_EVENT, on);
  }, [cardId, load.s, fetchNotes]);
  useEffect(() => {
    if (load.s !== 'ready' || !headSha || headSha === loadedHead.current) return;
    loadedHead.current = headSha;
    void api
      .changes(cardId)
      .then((c) => setLoad({ s: 'ready', changes: c }))
      .catch(() => undefined);
    void fetchNotes().catch(() => undefined);
  }, [headSha, load.s, api, cardId, fetchNotes]);

  // Resolved notes fold to their header line once they've been on screen 3 s.
  const resolvedKey = (notes ?? [])
    .filter((n) => n.state === 'resolved' && !folded.has(n.id))
    .map((n) => n.id)
    .join(',');
  useEffect(() => {
    if (!open || !resolvedKey) return;
    const t = window.setTimeout(() => setFolded((f) => new Set([...f, ...resolvedKey.split(',')])), RESOLVED_FOLDS_MS);
    return () => window.clearTimeout(t);
  }, [open, resolvedKey]);

  // The Sent bar leaves after 8 s; the Undo after 5 s.
  useEffect(() => {
    if (bar.s !== 'sent') return;
    const t = window.setTimeout(() => setBar({ s: 'idle' }), SENT_STAYS_MS);
    return () => window.clearTimeout(t);
  }, [bar]);
  useEffect(() => {
    if (!deleted) return;
    const t = window.setTimeout(() => setDeleted(null), UNDO_MS);
    return () => window.clearTimeout(t);
  }, [deleted]);

  // -------------------------------------------------------------------------------------------------
  // Writing

  const startWriting = (f: ChangedFile, lines: FlatLine[], a: number, b: number) => {
    if (!canNote) return;
    setSel({ path: f.path, anchor: a, from: a, to: b });
    setRangeMode(false);
    if (bar.s === 'sent') setBar({ s: 'idle' });
    setWriting((w) => ({
      path: f.path,
      start: pointOf(lines[a].line),
      end: pointOf(lines[b].line),
      code: quote(lines, a, b),
      text: w && !w.editing ? w.text : '',
      editing: null,
      asking: false,
      busy: false,
      error: null,
    }));
  };

  const startEditing = (n: Note) => {
    setSel(null);
    setWriting({ path: n.path, start: n.start, end: n.end, code: n.code, text: n.text, editing: n.id, asking: false, busy: false, error: null });
  };

  const focusLine = (path: string, p: LinePoint | null) => {
    window.setTimeout(() => {
      const grid = document.querySelector<HTMLElement>(`[data-ln-grid="${CSS.escape(path)}"]`);
      const btn = p ? grid?.querySelector<HTMLElement>(`[data-ln-point="${p.side}:${p.line}"] button`) : null;
      (btn ?? grid?.querySelector<HTMLElement>('button[tabindex="0"]'))?.focus();
    }, 0);
  };

  const stopWriting = () => {
    const w = writing;
    setWriting(null);
    setSel(null);
    if (w) focusLine(w.path, w.end);
  };

  const submit = async () => {
    const w = writing;
    if (!w || !w.text.trim() || w.busy || !changes) return;
    setWriting({ ...w, busy: true, error: null });
    try {
      const note = w.editing ? await api.edit(w.editing, w.text) : await api.create(cardId, { path: w.path, start: w.start, end: w.end, head_sha: changes.head_sha, text: w.text });
      const next = (notes ?? []).filter((n) => n.id !== note.id).concat(note);
      setNotes(next);
      if (!w.editing && bot) setSaid(addedSay(lineWords(note.start, note.end), next.filter((n) => n.state === 'draft').length, bot));
      if (bar.s === 'sent' || bar.s === 'failed') setBar({ s: 'idle' });
      sendId.current = null;
      setWriting(null);
      setSel(null);
      focusLine(w.path, w.end);
    } catch (e) {
      const err = e instanceof LineNotesError ? e : null;
      // Ask 4: the card moved on. Reload the diff; the note stays where it was written, with its text.
      if (err?.code === 'stale_head') {
        void api
          .changes(cardId)
          .then((c) => {
            loadedHead.current = c.head_sha;
            setLoad({ s: 'ready', changes: c });
          })
          .catch(() => undefined);
      }
      setWriting({ ...w, busy: false, error: err ? err.message : 'The note didn’t save. Try again.' });
    }
  };

  const remove = async (n: Note) => {
    try {
      await api.remove(n.id);
      setNotes((all) => (all ?? []).filter((x) => x.id !== n.id));
      setDeleted({ id: n.id, note: n });
      setSaid('Note deleted.');
    } catch (e) {
      setSaid(e instanceof Error ? e.message : 'The note wasn’t deleted.');
    }
  };
  const undoDelete = async () => {
    const d = deleted;
    if (!d) return;
    setDeleted(null);
    try {
      const back = await api.restore(d.id);
      setNotes((all) => (all ?? []).filter((x) => x.id !== back.id).concat(back));
      setSaid('Note restored.');
    } catch (e) {
      setSaid(e instanceof Error ? e.message : 'The note couldn’t be restored.');
    }
  };

  // -------------------------------------------------------------------------------------------------
  // Sending

  const send = async (only?: string[]) => {
    if (!changes?.author_bot || bar.s === 'sending') return;
    const n = only ? only.length : drafts.length;
    if (n === 0 || n > MAX_SEND) return;
    sendId.current ??= newSendId();
    setBar({ s: 'sending' });
    try {
      const r = await api.send(cardId, sendId.current, only);
      sendId.current = null;
      const back = new Map(r.notes.map((x) => [x.id, x]));
      setNotes((all) => (all ?? []).map((x) => back.get(x.id) ?? x));
      const name = r.kwant_name ?? changes.author_bot.name;
      setBar({ s: 'sent', n: r.sent, bot: name, botId: r.kwant_id ?? changes.author_bot.id, at: clock(r.sent_at) });
      setSaid(`Sent ${notesWord(r.sent)} to ${name}.`);
    } catch {
      // The send id is kept, so Try again can't post the batch twice (ask 6).
      setBar({ s: 'failed' });
      setSaid(SEND_FAILED);
    }
  };

  const discardAll = async () => {
    try {
      await api.discardAll(cardId);
      setNotes((all) => (all ?? []).filter((x) => x.state !== 'draft'));
      setBar({ s: 'idle' });
      sendId.current = null;
      setSaid('Drafts discarded.');
    } catch (e) {
      setBar({ s: 'idle' });
      setSaid(e instanceof Error ? e.message : 'The drafts weren’t discarded.');
    }
  };

  // -------------------------------------------------------------------------------------------------

  const draftCount = notes ? drafts.length : notesCount?.drafts ?? 0;
  const showBar = canNote && (drafts.length > 0 || bar.s === 'sent' || bar.s === 'failed');

  return (
    <section data-line-notes={cardId} aria-label="Changes" className="relative mt-4">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <h4 className={`flex min-w-0 flex-1 flex-wrap items-center gap-1.5 ${HEAD}`}>
          <Code2 size={13} aria-hidden="true" />
          <span>Changes</span>
          <span className="font-mono text-[12px] font-medium normal-case tracking-normal text-zinc-400 max-sm:order-1">{filesWord(stat.files)}</span>
          <DiffStat add={stat.additions} del={stat.deletions} className="max-sm:order-1 max-sm:[&_.dv-bar]:hidden" />
          {draftCount > 0 && <span className="normal-case tracking-normal text-violet-200">· {draftsWord(draftCount)}</span>}
        </h4>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={() => setOpen((o) => !o)}
          className={`inline-flex min-h-8 items-center gap-1 self-start rounded-md px-1.5 text-[12px] font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}
        >
          {open ? 'Hide' : 'Show'}
          <span className="sr-only"> the changes</span>
          <ChevronDown size={14} aria-hidden="true" className={`transition-transform duration-[160ms] motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
        </button>
      </div>
      {summary && summary.total > 0 && summary.sha && (summary.changed > 0 || summary.open > 0) && (
        <p data-ln-summary className="mt-1 text-meta text-zinc-400">
          {summaryLine(summary)}
        </p>
      )}
      <p aria-live="polite" className="sr-only" data-ln-live>
        {said}
      </p>

      <div id={bodyId} hidden={!open}>
        {open && load.s === 'loading' && (
          <div data-ln-loading className="mt-2 rounded-[10px] border border-hairline p-3">
            <p role="status" className="flex items-center gap-2 text-meta text-zinc-400">
              <Loader2 size={13} aria-hidden="true" className="motion-safe:animate-spin" />
              Loading the changes…
            </p>
            <div aria-hidden="true" className="mt-3 grid gap-2">
              <div className="h-2.5 w-[62%] rounded-full bg-[var(--kw-hairline-strong)]" />
              <div className="h-2.5 w-[40%] rounded-full bg-[var(--kw-hairline-strong)]" />
            </div>
          </div>
        )}
        {open && load.s === 'error' && (
          <p data-ln-error className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-[10px] border border-hairline px-3 py-2.5 text-meta text-zinc-300">
            <span className="inline-flex items-center gap-1.5">
              <AlertTriangle size={13} aria-hidden="true" className="text-zinc-400" />
              {LOAD_FAILED}
            </span>
            <span className="ml-auto inline-flex flex-wrap items-center gap-3">
              <button type="button" onClick={() => void fetchAll()} className={`min-h-8 rounded font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}>
                Try again
              </button>
              {prUrl && /^https:\/\//.test(prUrl) && <OutLink href={prUrl}>Open on GitHub</OutLink>}
            </span>
          </p>
        )}
        {open && changes && (
          <div className="mt-2">
            {!changes.author_bot && (
              <Banner>
                {notBotLine(changes.author_login).split(changes.author_login ?? '\u0000').map((part, i, all) => (
                  <React.Fragment key={i}>
                    {part}
                    {i < all.length - 1 && <strong className="font-semibold text-zinc-100">{changes.author_login}</strong>}
                  </React.Fragment>
                ))}
              </Banner>
            )}
            {changes.author_bot && changes.pr_state !== 'open' && <Banner>{readOnlyLine(changes.pr_state)}</Banner>}
            {changes.files.length === 0 ? (
              <Banner>{NO_DIFF}</Banner>
            ) : (
              <div data-ln-files className="flex flex-col gap-2 sm:max-h-[min(70vh,720px)] sm:overflow-y-auto sm:pr-0.5">
                {changes.files.map((f) => (
                  <FileBlock
                    key={f.path}
                    file={f}
                    open={fileOpen[f.path] ?? true}
                    onToggle={() => setFileOpen((o) => ({ ...o, [f.path]: !(o[f.path] ?? true) }))}
                    notes={(notes ?? []).filter((n) => n.path === f.path)}
                    headSha={changes.head_sha}
                    canNote={canNote}
                    phone={phone}
                    bot={bot ?? ''}
                    prUrl={changes.pr_url ?? prUrl}
                    sel={sel && sel.path === f.path ? sel : null}
                    setSel={(s) => setSel(s ? { path: f.path, ...s } : null)}
                    writing={writing && writing.path === f.path ? writing : null}
                    setWriting={setWriting}
                    rangeMode={rangeMode}
                    startWriting={(lines, a, b) => startWriting(f, lines, a, b)}
                    stopWriting={stopWriting}
                    submit={() => void submit()}
                    onEdit={startEditing}
                    onDelete={(n) => void remove(n)}
                    onSendOne={(n) => void send([n.id])}
                    folded={folded}
                    toggleFold={(id) =>
                      setFolded((s) => {
                        const next = new Set(s);
                        if (next.has(id)) next.delete(id);
                        else next.add(id);
                        return next;
                      })
                    }
                  />
                ))}
              </div>
            )}
            {changes.cut && <p className="mt-2 text-meta text-[#8a8a96]">{cutLine(changes.cut)}</p>}
          </div>
        )}

        {deleted && (
          <p role="status" data-ln-undo className="mt-2 flex items-center gap-2 text-meta text-zinc-300">
            Note deleted
            <span aria-hidden="true">·</span>
            <button type="button" onClick={() => void undoDelete()} className={`min-h-8 rounded font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}>
              Undo
            </button>
          </p>
        )}

        {/* Phone: a range is chosen by tapping its last line. */}
        {open && phone && rangeMode && sel && (
          <div data-ln-range-bar className="fixed inset-x-0 bottom-0 z-40 flex items-center gap-3 border-t border-white/10 bg-[#1a1538]/95 px-4 py-2.5 backdrop-blur pb-[max(.625rem,env(safe-area-inset-bottom))]">
            <span className="min-w-0 flex-1 text-[15px] text-zinc-200">Line {lineAt(changes, sel)} · tap the last line</span>
            <button
              type="button"
              onClick={() => {
                const f = changes?.files.find((x) => x.path === sel.path);
                if (f) {
                  const lines = flatLines(f);
                  startWriting(f, lines, sel.from, sel.to);
                }
              }}
              className="kw-btn-primary min-h-11"
            >
              Done
            </button>
          </div>
        )}

        {/* Phone: the note sheet. */}
        {open && phone && writing && !rangeMode && (
          <NoteSheet
            writing={writing}
            setWriting={setWriting}
            bot={bot ?? ''}
            onCancel={stopWriting}
            onSubmit={() => void submit()}
            onExtend={
              writing.editing
                ? null
                : () => {
                    setRangeMode(true);
                  }
            }
          />
        )}

        {open && showBar && (
          <SendBar
            bar={bar}
            setBar={setBar}
            drafts={drafts.length}
            bot={bot ?? ''}
            phone={phone}
            onSend={() => void send()}
            onDiscard={() => void discardAll()}
            onOpenChat={onOpenChat}
          />
        )}
      </div>
    </section>
  );
};

function lineAt(changes: Changes | null, sel: { path: string; from: number }): number | string {
  const f = changes?.files.find((x) => x.path === sel.path);
  const l = f ? flatLines(f)[sel.from] : null;
  return l ? shownNo(l.line) : '';
}

const Banner: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <p data-ln-banner className="mb-2 flex items-start gap-2 rounded-[10px] border border-hairline px-3 py-2.5 text-meta text-zinc-300 [overflow-wrap:anywhere]">
    <Info size={13} aria-hidden="true" className="mt-[3px] shrink-0 text-zinc-400" />
    <span className="min-w-0">{children}</span>
  </p>
);

// ---------------------------------------------------------------------------------------------------
// One file

interface FileBlockProps {
  file: ChangedFile;
  open: boolean;
  onToggle: () => void;
  notes: Note[];
  headSha: string | null;
  canNote: boolean;
  phone: boolean;
  bot: string;
  prUrl: string | null;
  sel: { anchor: number; from: number; to: number } | null;
  setSel: (s: { anchor: number; from: number; to: number } | null) => void;
  writing: Writing | null;
  setWriting: React.Dispatch<React.SetStateAction<Writing | null>>;
  rangeMode: boolean;
  startWriting: (lines: FlatLine[], a: number, b: number) => void;
  stopWriting: () => void;
  submit: () => void;
  onEdit: (n: Note) => void;
  onDelete: (n: Note) => void;
  onSendOne: (n: Note) => void;
  folded: Set<string>;
  toggleFold: (id: string) => void;
}

/** Renders a file's lines once it scrolls near the viewport (no virtualiser). */
function useNear(ref: React.RefObject<HTMLElement>, on: boolean): boolean {
  const [near, setNear] = useState(() => typeof IntersectionObserver === 'undefined');
  useEffect(() => {
    if (near || !on || !ref.current || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver((es) => es.some((e) => e.isIntersecting) && setNear(true), { rootMargin: '600px 0px' });
    io.observe(ref.current);
    return () => io.disconnect();
  }, [near, on, ref]);
  return near;
}

const FileBlock: React.FC<FileBlockProps> = (p) => {
  const { file: f, notes, sel, writing, canNote, phone } = p;
  const lines = useMemo(() => flatLines(f), [f]);
  const [active, setActive] = useState(0);
  const [shown, setShown] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const near = useNear(wrap, p.open);
  const drag = useRef<number | null>(null);
  const bodyId = useId();

  // Where each note sits: under the last line of its range, or at the top of the file.
  const { under, top, noted } = useMemo(() => {
    const under = new Map<number, Note[]>();
    const top: Note[] = [];
    const noted = new Set<number>();
    for (const n of notes) {
      const at = positionIn(lines, n);
      if (!at) {
        top.push(n);
        continue;
      }
      for (let i = at[0]; i <= at[1]; i++) noted.add(i);
      under.set(at[1], [...(under.get(at[1]) ?? []), n]);
    }
    return { under, top, noted };
  }, [notes, lines]);

  const writeAt = writing ? indexOfPoint(lines, writing.end) : -1;
  const editingId = writing?.editing ?? null;

  useEffect(() => {
    const up = () => {
      const a = drag.current;
      drag.current = null;
      if (a === null || !sel) return;
      p.startWriting(lines, sel.from, sel.to);
    };
    window.addEventListener('pointerup', up);
    return () => window.removeEventListener('pointerup', up);
  });

  const focusAt = (i: number) => {
    const n = Math.max(0, Math.min(lines.length - 1, i));
    setActive(n);
    window.setTimeout(() => wrap.current?.querySelector<HTMLElement>(`[data-ln-i="${n}"] button`)?.focus(), 0);
    return n;
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (!(e.target instanceof HTMLButtonElement) || !e.target.closest('[data-ln-i]')) return;
    const i = active;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const n = focusAt(i + (e.key === 'ArrowDown' ? 1 : -1));
      if (e.shiftKey) {
        const anchor = sel?.anchor ?? i;
        const [a, b] = clampRange(lines, anchor, n);
        p.setSel({ anchor, from: a, to: b });
      } else p.setSel(null);
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      focusAt(e.key === 'Home' ? 0 : lines.length - 1);
      p.setSel(null);
    } else if (e.key === ']' || e.key === '[') {
      e.preventDefault();
      const grids = [...document.querySelectorAll<HTMLElement>('[data-ln-grid]')];
      const at = grids.findIndex((g) => g.contains(e.target as Node));
      grids[at + (e.key === ']' ? 1 : -1)]?.querySelector<HTMLElement>('button[tabindex="0"]')?.focus();
    } else if ((e.key === 'n' || e.key === 'N') && canNote) {
      e.preventDefault();
      open(i, false);
    }
  };

  /** A click (or Enter, or N) on line i: a note on the selected range when i ends it, else on line i. */
  const open = (i: number, shift: boolean) => {
    if (!canNote) return;
    if (shift && sel) {
      const [a, b] = clampRange(lines, sel.anchor, i);
      p.startWriting(lines, a, b);
      return;
    }
    if (sel && i === sel.to && sel.from !== sel.to) {
      p.startWriting(lines, sel.from, sel.to);
      return;
    }
    p.setSel({ anchor: i, from: i, to: i });
    p.startWriting(lines, i, i);
  };

  /** Phone: tap a line. In range mode it sets the range's last line. */
  const tap = (i: number) => {
    if (!canNote) return;
    if (p.rangeMode && sel) {
      const [a, b] = clampRange(lines, sel.anchor, i);
      p.setSel({ anchor: sel.anchor, from: a, to: b });
      p.startWriting(lines, a, b);
      return;
    }
    open(i, false);
  };

  const omitted = f.omitted;
  const renamedOnly = !omitted && lines.length === 0 && !!f.old_path;
  const hasBody = !omitted && lines.length > 0;
  const big = hasBody && !p.open;
  const n = notes.length;

  const noteBox = (note: Note) =>
    editingId === note.id && !phone ? (
      <Composer key={note.id} writing={writing!} setWriting={p.setWriting} bot={p.bot} onCancel={p.stopWriting} onSubmit={p.submit} />
    ) : (
      <NoteBox
        key={note.id}
        note={note}
        bot={p.bot}
        editable={canNote && note.state === 'draft'}
        folded={p.folded.has(note.id)}
        onFold={() => p.toggleFold(note.id)}
        onEdit={() => p.onEdit(note)}
        onDelete={() => p.onDelete(note)}
        onSendOne={() => p.onSendOne(note)}
        movedTo={note.moved && note.at_head ? note.at_head.end.line : null}
      />
    );

  return (
    <div ref={wrap} data-ln-file={f.path}>
      <div
        className={`flex flex-wrap items-center gap-x-2 gap-y-1 border border-hairline bg-white/[.02] px-2.5 py-[7px] font-mono text-[12px] text-zinc-200 ${hasBody && p.open ? 'rounded-t-[10px] border-b-0' : 'rounded-[10px]'}`}
      >
        {hasBody ? (
          <button
            type="button"
            aria-expanded={p.open}
            aria-controls={bodyId}
            onClick={p.onToggle}
            className={`inline-flex min-h-6 min-w-0 flex-1 items-center gap-1.5 rounded text-left [overflow-wrap:anywhere] max-sm:min-h-11 ${RING}`}
          >
            <ChevronRight size={13} aria-hidden="true" className={`shrink-0 text-zinc-500 transition-transform duration-[160ms] motion-reduce:transition-none ${p.open ? 'rotate-90' : ''}`} />
            <FileName file={f} phone={phone} />
            <DiffStat add={f.additions} del={f.deletions} className="shrink-0" />
          </button>
        ) : (
          <span className="inline-flex min-w-0 flex-1 flex-wrap items-center gap-1.5 [overflow-wrap:anywhere]">
            <Code2 size={13} aria-hidden="true" className="shrink-0 text-zinc-500" />
            <FileName file={f} phone={phone} />
            {!renamedOnly && <DiffStat add={f.additions} del={f.deletions} className="shrink-0" />}
          </span>
        )}
        {n > 0 && (
          <span className="inline-flex shrink-0 items-center gap-1 font-sans text-[12px] text-violet-200">
            <MessageSquare size={12} aria-hidden="true" />
            {notesWord(n)}
          </span>
        )}
        {omitted === 'binary' && <span className="w-full font-sans text-meta text-[#8a8a96] sm:w-auto">Binary file, not shown</span>}
        {omitted === 'too_large' && (
          <span className="w-full font-sans text-meta text-[#8a8a96] sm:w-auto">
            Too big to show here
            {p.prUrl && /^https:\/\//.test(p.prUrl) && (
              <>
                {' · '}
                <OutLink href={p.prUrl}>Open on GitHub</OutLink>
              </>
            )}
          </span>
        )}
        {renamedOnly && <span className="w-full font-sans text-meta text-[#8a8a96] sm:w-auto">Renamed, no changes</span>}
        {big && !shown && (
          <button
            type="button"
            onClick={() => {
              setShown(true);
              p.onToggle();
            }}
            className={`ml-auto min-h-6 rounded font-sans text-[12px] font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}
          >
            {showLines(lines.length)}
          </button>
        )}
      </div>

      {/* Notes whose lines aren't in this diff stay at the top of their file, with the code as written. */}
      {top.length > 0 && (
        <div className={`flex flex-col gap-1.5 border-x border-hairline bg-[var(--dv-code-bg)] py-1.5 ${hasBody && p.open ? '' : 'rounded-b-[10px] border-b'}`}>
          {top.map((note) => (
            <div key={note.id} className="px-2.5">
              {note.code.length > 0 && (
                <pre className="mb-1 overflow-x-auto rounded-md bg-black/20 px-2 py-1 font-mono text-[12px] leading-5 text-zinc-400 max-sm:whitespace-pre-wrap max-sm:break-words">
                  {codeText(note.code)}
                </pre>
              )}
              {noteBox(note)}
            </div>
          ))}
        </div>
      )}

      {hasBody && p.open && (
        <div
          id={bodyId}
          role="grid"
          aria-label={`Changes in ${f.path}`}
          aria-readonly={!canNote}
          data-ln-grid={f.path}
          tabIndex={canNote ? undefined : 0}
          onKeyDown={onKey}
          className={`overflow-x-auto rounded-b-[10px] border border-hairline bg-[var(--dv-code-bg)] font-mono text-[12px] leading-5 [container-type:inline-size] max-sm:text-[13px] ${canNote ? '' : RING}`}
        >
          {!near ? (
            <div aria-hidden="true" style={{ height: Math.min(lines.length, 400) * 20 }} />
          ) : (
            <div className="sm:w-max sm:min-w-full">
              {f.hunks.map((h, hi) => (
                <React.Fragment key={hi}>
                  <div role="row" className="grid grid-cols-[34px_1fr] bg-violet-500/[.06] text-[#8a8a96] sm:grid-cols-[26px_38px_38px_1fr]">
                    <span role="gridcell" className="max-sm:hidden" />
                    <span role="gridcell" className="max-sm:hidden" />
                    <span role="gridcell" />
                    <span role="gridcell" className="whitespace-pre px-1 max-sm:whitespace-pre-wrap max-sm:break-words">
                      {h.header}
                    </span>
                  </div>
                  {lines
                    .filter((x) => x.hunk === hi)
                    .map((x) => (
                      <React.Fragment key={x.i}>
                        <Row
                          x={x}
                          selected={!!sel && x.i >= sel.from && x.i <= sel.to}
                          noted={noted.has(x.i)}
                          active={x.i === active}
                          canNote={canNote}
                          phone={phone}
                          onFocus={() => setActive(x.i)}
                          onPress={(shift) => (phone ? tap(x.i) : open(x.i, shift))}
                          onDragStart={() => {
                            if (!canNote || phone) return;
                            drag.current = x.i;
                            p.setSel({ anchor: x.i, from: x.i, to: x.i });
                          }}
                          onDragOver={() => {
                            if (drag.current === null) return;
                            const [a, b] = clampRange(lines, drag.current, x.i);
                            p.setSel({ anchor: drag.current, from: a, to: b });
                          }}
                        />
                        {(under.get(x.i) ?? []).map((note) => (
                          <div key={note.id} role="row" className="sticky left-0 w-full sm:w-[100cqw]">
                            <div role="gridcell">{noteBox(note)}</div>
                          </div>
                        ))}
                        {!phone && writing && !writing.editing && writeAt === x.i && (
                          <div role="row" className="sticky left-0 w-full sm:w-[100cqw]">
                            <div role="gridcell">
                              <Composer writing={writing} setWriting={p.setWriting} bot={p.bot} onCancel={p.stopWriting} onSubmit={p.submit} />
                            </div>
                          </div>
                        )}
                      </React.Fragment>
                    ))}
                </React.Fragment>
              ))}
            </div>
          )}
        </div>
      )}
      {/* A note being written whose line left the diff (the bot pushed meanwhile): it stays, at the top. */}
      {!phone && writing && !writing.editing && writeAt < 0 && (
        <div className="mt-1">
          <Composer writing={writing} setWriting={p.setWriting} bot={p.bot} onCancel={p.stopWriting} onSubmit={p.submit} />
        </div>
      )}
    </div>
  );
};

const FileName: React.FC<{ file: ChangedFile; phone: boolean }> = ({ file, phone }) => (
  <span className="min-w-0 [overflow-wrap:anywhere]" title={file.path}>
    {file.old_path && file.old_path !== file.path ? (
      <>
        {phone ? base(file.old_path) : file.old_path}
        <span aria-hidden="true" className="text-zinc-500">
          {' → '}
        </span>
        <span className="sr-only"> renamed to </span>
        {phone ? base(file.path) : file.path}
      </>
    ) : phone ? (
      <>
        <span className="sr-only">{file.path}</span>
        <span aria-hidden="true">{base(file.path)}</span>
      </>
    ) : (
      file.path
    )}
  </span>
);

// ---------------------------------------------------------------------------------------------------
// One line

const LINE_TONE = {
  add: 'bg-[var(--dv-add-bg)] text-[var(--dv-add)]',
  del: 'bg-[var(--dv-del-bg)] text-[var(--dv-del)]',
  ctx: 'text-zinc-300',
} as const;

interface RowProps {
  x: FlatLine;
  selected: boolean;
  noted: boolean;
  active: boolean;
  canNote: boolean;
  phone: boolean;
  onFocus: () => void;
  onPress: (shift: boolean) => void;
  onDragStart: () => void;
  onDragOver: () => void;
}

const Row: React.FC<RowProps> = React.memo(({ x, selected, noted, active, canNote, phone, onFocus, onPress, onDragStart, onDragOver }) => {
  const l = x.line;
  const p = pointOf(l);
  return (
    <div
      role="row"
      data-ln-i={x.i}
      data-ln-point={`${p.side}:${p.line}`}
      data-ln-kind={l.kind}
      aria-selected={selected || undefined}
      onPointerEnter={onDragOver}
      className={`group relative grid grid-cols-[34px_1fr] sm:grid-cols-[26px_38px_38px_1fr] ${selected ? 'bg-violet-500/[.14] shadow-[inset_3px_0_0_theme(colors.violet.400)]' : LINE_TONE[l.kind]}`}
    >
      <span role="gridcell" className="flex items-center justify-center max-sm:absolute max-sm:inset-0 max-sm:z-10">
        {canNote ? (
          <button
            type="button"
            tabIndex={active ? 0 : -1}
            aria-label={`${addAria(l)}${noted ? '. It has a note' : ''}`}
            onFocus={onFocus}
            onPointerDown={(e) => {
              if (e.button === 0 && !phone) onDragStart();
            }}
            onClick={(e) => onPress(e.shiftKey)}
            className={`inline-flex h-[18px] w-5 items-center justify-center rounded-[5px] max-sm:h-full max-sm:w-full max-sm:!min-h-0 max-sm:!min-w-0 max-sm:rounded-none max-sm:opacity-0 ${
              noted ? 'text-violet-300' : 'bg-violet-600 text-white opacity-0 group-hover:opacity-100 focus-visible:opacity-100'
            } ${selected && !noted ? 'opacity-100' : ''} ${RING}`}
          >
            {noted ? <MessageSquare size={13} aria-hidden="true" /> : <Plus size={12} aria-hidden="true" />}
          </button>
        ) : noted ? (
          <MessageSquare size={13} aria-hidden="true" className="text-violet-300 max-sm:hidden" />
        ) : null}
      </span>
      <span role="gridcell" className="select-none pr-2 text-right text-[#8a8a96] max-sm:hidden" aria-hidden="true">
        {l.old ?? ''}
      </span>
      <span role="gridcell" className={`select-none pr-2 text-right ${l.kind === 'del' ? 'text-[#8a8a96] max-sm:text-rose-300/70' : 'text-[#8a8a96]'}`} aria-hidden="true">
        <span className="max-sm:hidden">{l.new ?? ''}</span>
        <span className="sm:hidden">{shownNo(l)}</span>
      </span>
      <span role="gridcell" className={`whitespace-pre pr-3 max-sm:whitespace-pre-wrap max-sm:break-words ${selected ? 'text-zinc-100' : ''}`}>
        {l.kind === 'add' && <span className="sr-only">Added: </span>}
        {l.kind === 'del' && <span className="sr-only">Removed: </span>}
        <span aria-hidden="true">{marker(l.kind)} </span>
        {l.text}
      </span>
    </div>
  );
});
Row.displayName = 'Row';

// ---------------------------------------------------------------------------------------------------
// Notes

const STATE_BAR: Record<Note['state'], string> = {
  draft: 'border-white/10',
  sent: 'border-white/10 shadow-[inset_3px_0_0_theme(colors.violet.400)]',
  resolved: 'border-white/10 bg-teal-400/[.04] shadow-[inset_3px_0_0_var(--ok,theme(colors.teal.300))]',
  open: 'border-white/10 shadow-[inset_3px_0_0_theme(colors.zinc.500)]',
};

interface NoteBoxProps {
  note: Note;
  bot: string;
  editable: boolean;
  folded: boolean;
  onFold: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onSendOne: () => void;
  movedTo: number | null;
}

const NoteBox: React.FC<NoteBoxProps> = ({ note, bot, editable, folded, onFold, onEdit, onDelete, onSendOne, movedTo }) => {
  const where = lineWords(note.start, note.end);
  const words =
    note.state === 'sent'
      ? sentNoteLine(clock(note.sent_at), bot || 'the bot')
      : note.state === 'resolved'
        ? changedIn(note.checked_sha)
        : note.state === 'open'
          ? stillOpen(note.checked_sha)
          : 'Draft';
  const resolved = note.state === 'resolved';
  const label =
    note.state === 'draft' ? `Draft note on ${where}` : `Note on ${where}, ${words}`;
  return (
    <div
      data-ln-note={note.id}
      data-ln-state={note.state}
      role="group"
      aria-label={label}
      tabIndex={editable ? 0 : undefined}
      onKeyDown={(e) => {
        if (!editable || e.target !== e.currentTarget) return;
        if (e.key === 'e' || e.key === 'E') {
          e.preventDefault();
          onEdit();
        } else if (e.key === 'Delete' || e.key === 'Backspace') {
          e.preventDefault();
          onDelete();
        }
      }}
      className={`my-1.5 ml-2.5 mr-2.5 rounded-[10px] border bg-surface-raised px-3 py-2.5 font-sans sm:ml-16 ${STATE_BAR[note.state]} ${editable ? RING : ''}`}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-zinc-400">
        <span className="font-semibold text-zinc-100">You</span>
        <span>{where}</span>
        {note.state === 'draft' ? (
          <span className="rounded-full border border-dashed border-zinc-500 px-1.5 text-[11px] font-medium text-zinc-200">Draft</span>
        ) : resolved ? (
          <button
            type="button"
            aria-expanded={!folded}
            onClick={onFold}
            className={`inline-flex items-center gap-1 rounded-full border border-teal-400/30 bg-teal-400/10 px-1.5 text-[11px] font-medium text-teal-200 max-sm:min-h-8 ${RING}`}
          >
            <Check size={11} aria-hidden="true" />
            {words}
          </button>
        ) : (
          <span className={`rounded-full border px-1.5 text-[11px] font-medium ${note.state === 'sent' ? 'border-violet-400/30 text-violet-200' : 'border-zinc-500/40 text-zinc-300'}`}>
            {words}
          </span>
        )}
        {editable && (
          <span className="ml-auto inline-flex items-center gap-0.5">
            <button type="button" aria-label={`Edit the note on ${where}`} onClick={onEdit} className={ICON_BTN}>
              <Pencil size={13} aria-hidden="true" />
            </button>
            <button type="button" aria-label={`Delete the note on ${where}`} onClick={onDelete} className={ICON_BTN}>
              <Trash2 size={13} aria-hidden="true" />
            </button>
          </span>
        )}
      </div>
      {!(resolved && folded) && (
        <p className="mt-1 whitespace-pre-wrap text-[13px] leading-5 text-zinc-200 [overflow-wrap:anywhere]">{note.text}</p>
      )}
      {note.state === 'draft' && note.moved && (
        <p className="mt-1.5 flex items-start gap-1.5 text-[11.5px] text-zinc-400">
          <Info size={12} aria-hidden="true" className="mt-[2px] shrink-0" />
          {movedLine(bot || 'The bot', note.seen_sha, movedTo)}
        </p>
      )}
      {note.state === 'draft' && note.gone && (
        <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-zinc-400">
          <Info size={12} aria-hidden="true" className="shrink-0" />
          {goneLine(note.seen_sha)}
          {editable && (
            <>
              <button type="button" onClick={onSendOne} className={`rounded font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}>
                Send it anyway
              </button>
              <span aria-hidden="true">·</span>
              <button type="button" onClick={onDelete} className={`rounded font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}>
                Delete
              </button>
            </>
          )}
        </p>
      )}
    </div>
  );
};

interface ComposerProps {
  writing: Writing;
  setWriting: React.Dispatch<React.SetStateAction<Writing | null>>;
  bot: string;
  onCancel: () => void;
  onSubmit: () => void;
}

const keyHint = () => (isMacPlatform() ? '⌘ Enter adds · Esc cancels' : 'Ctrl Enter adds · Esc cancels');

/** Shared by the inline box and the phone sheet. */
function useComposerKeys(w: Writing, setWriting: ComposerProps['setWriting'], onCancel: () => void, onSubmit: () => void) {
  return (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onSubmit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (w.text.trim() && w.text !== '' && !w.asking) setWriting((x) => (x ? { ...x, asking: true } : x));
      else if (!w.text.trim()) onCancel();
    }
  };
}

const AskThrowAway: React.FC<{ setWriting: ComposerProps['setWriting']; onCancel: () => void; big?: boolean }> = ({ setWriting, onCancel, big }) => {
  const keep = useRef<HTMLButtonElement>(null);
  useEffect(() => keep.current?.focus(), []);
  return (
    <div role="alertdialog" aria-label="Throw away this note?" className="mt-2 flex flex-wrap items-center gap-2 text-meta text-zinc-200">
      <span className="mr-auto">Throw away this note?</span>
      <button type="button" onClick={onCancel} className={`kw-btn-ghost ${big ? 'min-h-11' : ''}`}>
        Throw away
      </button>
      <button
        ref={keep}
        type="button"
        onClick={() => {
          setWriting((x) => (x ? { ...x, asking: false } : x));
          window.setTimeout(() => document.querySelector<HTMLTextAreaElement>('[data-ln-text]')?.focus(), 0);
        }}
        className={`kw-btn-primary ${big ? 'min-h-11' : ''}`}
      >
        Keep writing
      </button>
    </div>
  );
};

const Composer: React.FC<ComposerProps> = ({ writing: w, setWriting, bot, onCancel, onSubmit }) => {
  const onKey = useComposerKeys(w, setWriting, onCancel, onSubmit);
  const label = useId();
  return (
    <div data-ln-writing className="my-1.5 ml-2.5 mr-2.5 rounded-[10px] border border-violet-500 bg-surface-raised px-3 py-2.5 font-sans ring-[3px] ring-violet-500/15 sm:ml-16">
      <p id={label} className="text-[11.5px] text-zinc-400">
        <span className="font-semibold text-zinc-100">{w.editing ? 'Edit note' : `Note for ${bot}`}</span> {lineWords(w.start, w.end)}
      </p>
      <textarea
        data-ln-text
        autoFocus
        aria-labelledby={label}
        placeholder="Say what to change"
        value={w.text}
        maxLength={4000}
        onChange={(e) => setWriting((x) => (x ? { ...x, text: e.target.value, asking: false } : x))}
        onKeyDown={onKey}
        className="mt-1.5 block min-h-[52px] w-full resize-y rounded-lg border border-hairline bg-[var(--dv-code-bg)] px-2.5 py-2 text-[13px] leading-5 text-zinc-100 placeholder:text-[#8a8a96] focus:border-violet-400 focus:outline-none"
      />
      {w.error && (
        <p role="alert" className="mt-1.5 text-meta text-rose-300">
          {w.error}
        </p>
      )}
      {w.asking ? (
        <AskThrowAway setWriting={setWriting} onCancel={onCancel} />
      ) : (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <button type="button" disabled={!w.text.trim() || w.busy} onClick={onSubmit} className="kw-btn-primary !px-3 !py-1 !text-meta">
            {w.busy && <Loader2 size={13} aria-hidden="true" className="motion-safe:animate-spin" />}
            {w.editing ? 'Save' : 'Add note'}
          </button>
          <button type="button" onClick={onCancel} className="kw-btn-ghost !py-1">
            Cancel
          </button>
          <span className="ml-auto text-[11.5px] text-[#8a8a96] max-sm:hidden">{keyHint()}</span>
        </div>
      )}
    </div>
  );
};

const NoteSheet: React.FC<{ writing: Writing; setWriting: ComposerProps['setWriting']; bot: string; onCancel: () => void; onSubmit: () => void; onExtend: (() => void) | null }> = ({
  writing: w,
  setWriting,
  bot,
  onCancel,
  onSubmit,
  onExtend,
}) => {
  const onKey = useComposerKeys(w, setWriting, onCancel, onSubmit);
  const title = useId();
  return (
    <div className="fixed inset-0 z-50 flex flex-col justify-end bg-black/40" onClick={(e) => e.target === e.currentTarget && !w.text.trim() && onCancel()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={title}
        data-ln-sheet
        onKeyDown={(e) => e.key === 'Escape' && !w.text.trim() && onCancel()}
        className="max-h-[85vh] overflow-y-auto rounded-t-2xl border-t border-white/10 bg-surface-raised px-4 pb-[max(1rem,env(safe-area-inset-bottom))] pt-2.5"
      >
        <div aria-hidden="true" className="mx-auto mb-3 h-1 w-9 rounded-full bg-white/20" />
        <h2 id={title} className="flex flex-wrap items-baseline gap-x-2 text-[15px] font-semibold text-zinc-50">
          {w.editing ? 'Edit note' : `Note for ${bot}`}
          <span className="text-[13px] font-normal text-zinc-400 [overflow-wrap:anywhere]">
            {base(w.path)} · {lineWords(w.start, w.end)}
          </span>
        </h2>
        {w.code.length > 0 && (
          <pre className="mt-2 max-h-32 overflow-y-auto whitespace-pre-wrap break-words rounded-lg border border-hairline bg-[var(--dv-code-bg)] px-2.5 py-1.5 font-mono text-[12.5px] leading-5 text-zinc-300">
            {codeText(w.code)}
          </pre>
        )}
        <textarea
          data-ln-text
          autoFocus
          aria-labelledby={title}
          placeholder="Say what to change"
          value={w.text}
          maxLength={4000}
          onChange={(e) => setWriting((x) => (x ? { ...x, text: e.target.value, asking: false } : x))}
          onKeyDown={onKey}
          className="mt-2.5 block min-h-[76px] w-full resize-none rounded-xl border border-violet-400/70 bg-[var(--dv-code-bg)] px-3 py-2.5 text-[15px] leading-6 text-zinc-100 placeholder:text-[#8a8a96] focus:outline-none focus:ring-[3px] focus:ring-violet-500/20"
        />
        {w.error && (
          <p role="alert" className="mt-1.5 text-meta text-rose-300">
            {w.error}
          </p>
        )}
        {onExtend && (
          <p className="mt-2 text-[13px] text-zinc-400">
            Tap another line first to note a range.{' '}
            <button type="button" onClick={onExtend} className={`min-h-11 rounded font-medium text-violet-300 ${RING}`}>
              Extend to line…
            </button>
          </p>
        )}
        {w.asking ? (
          <AskThrowAway setWriting={setWriting} onCancel={onCancel} big />
        ) : (
          <div className="mt-2 grid grid-cols-2 gap-2.5">
            <button
              type="button"
              onClick={() => (w.text.trim() ? setWriting((x) => (x ? { ...x, asking: true } : x)) : onCancel())}
              className="kw-btn-ghost min-h-11 !text-[15px]"
            >
              Cancel
            </button>
            <button type="button" disabled={!w.text.trim() || w.busy} onClick={onSubmit} className="kw-btn-primary min-h-11 !text-[15px]">
              {w.busy && <Loader2 size={15} aria-hidden="true" className="motion-safe:animate-spin" />}
              {w.editing ? 'Save' : 'Add note'}
            </button>
          </div>
        )}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The send bar

interface SendBarProps {
  bar: Bar;
  setBar: (b: Bar) => void;
  drafts: number;
  bot: string;
  phone: boolean;
  onSend: () => void;
  onDiscard: () => void;
  onOpenChat?: (botId: string) => void;
}

const SendBar: React.FC<SendBarProps> = ({ bar, setBar, drafts, bot, phone, onSend, onDiscard, onOpenChat }) => {
  const [menu, setMenu] = useState(false);
  const tooMany = drafts > MAX_SEND;
  const failed = bar.s === 'failed';
  const where = phone
    ? 'fixed inset-x-0 bottom-0 z-40 px-4 pb-[max(.625rem,env(safe-area-inset-bottom))]'
    : 'sticky bottom-0 z-10 -mx-4 mt-3 rounded-b-[inherit] px-4';
  const keep = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (bar.s === 'confirm') keep.current?.focus();
  }, [bar.s]);
  return (
    <div
      data-ln-bar={bar.s}
      role="region"
      aria-label="Notes to send"
      className={`${where} flex flex-wrap items-center gap-x-3 gap-y-2 border-t py-2.5 backdrop-blur ${failed ? 'border-rose-300/35 bg-[#1a1538]/95' : 'border-white/10 bg-[#1a1538]/95'}`}
    >
      {bar.s === 'sent' ? (
        <>
          <span className="inline-flex min-w-0 flex-1 items-center gap-2 text-meta text-zinc-200">
            <Check size={14} aria-hidden="true" className="shrink-0 text-teal-300" />
            {sentLine(bar.n, bar.bot, bar.at)}
          </span>
          {onOpenChat && bar.botId && (
            <button type="button" onClick={() => onOpenChat(bar.botId!)} className={`inline-flex min-h-8 items-center gap-0.5 rounded text-[12px] font-medium text-violet-300 hover:text-violet-200 max-sm:min-h-11 ${RING}`}>
              Open {bar.bot}’s chat
              <ChevronRight size={14} aria-hidden="true" />
            </button>
          )}
        </>
      ) : failed ? (
        <>
          <span className="inline-flex min-w-0 flex-1 items-center gap-2 text-meta text-zinc-200">
            <AlertTriangle size={14} aria-hidden="true" className="shrink-0 text-rose-300" />
            {SEND_FAILED}
          </span>
          <button type="button" onClick={onSend} className="kw-btn-primary max-sm:min-h-11">
            Try again
          </button>
        </>
      ) : bar.s === 'confirm' ? (
        <>
          <span className="min-w-0 flex-1 text-meta text-zinc-200">{discardAsk(drafts)}</span>
          <button type="button" onClick={onDiscard} className="kw-btn-ghost max-sm:min-h-11">
            Discard
          </button>
          <button ref={keep} type="button" onClick={() => setBar({ s: 'idle' })} className="kw-btn-primary max-sm:min-h-11">
            Keep
          </button>
        </>
      ) : (
        <>
          <span className="inline-flex min-w-0 flex-1 items-center gap-2 text-meta text-zinc-300">
            <span aria-hidden="true" className="inline-flex shrink-0">
              <ModernAvatar name={bot} size={22} />
            </span>
            <span className="min-w-0 [overflow-wrap:anywhere]">
              <strong className="font-semibold text-zinc-100">{notesWord(drafts)}</strong> for {bot}
              {!phone && <span className="text-zinc-400"> · drafts, only you see them</span>}
              {tooMany && <span className="block text-zinc-400">{TOO_MANY}</span>}
            </span>
          </span>
          {phone ? (
            <span className="relative">
              <button
                type="button"
                aria-label="More"
                aria-haspopup="menu"
                aria-expanded={menu}
                onClick={() => setMenu((m) => !m)}
                className={`inline-flex h-11 w-11 items-center justify-center rounded-xl text-zinc-300 hover:bg-white/[.06] ${RING}`}
              >
                <MoreHorizontal size={18} aria-hidden="true" />
              </button>
              {menu && (
                <span role="menu" className="absolute bottom-12 right-0 min-w-[10rem] rounded-xl border border-white/10 bg-surface-raised p-1 shadow-card">
                  <button
                    type="button"
                    role="menuitem"
                    autoFocus
                    onClick={() => {
                      setMenu(false);
                      setBar({ s: 'confirm' });
                    }}
                    className={`flex min-h-11 w-full items-center rounded-lg px-3 text-left text-[15px] text-zinc-200 hover:bg-white/[.06] ${RING}`}
                  >
                    Discard all
                  </button>
                </span>
              )}
            </span>
          ) : (
            <button type="button" disabled={bar.s === 'sending'} onClick={() => setBar({ s: 'confirm' })} className="kw-btn-ghost">
              Discard all
            </button>
          )}
          <button
            type="button"
            data-ln-send
            disabled={bar.s === 'sending' || tooMany}
            onClick={onSend}
            aria-label={bar.s === 'sending' ? 'Sending…' : sendLabel(drafts, bot)}
            className={`kw-btn-primary ${phone ? 'min-h-11 !text-[15px]' : ''}`}
          >
            {bar.s === 'sending' ? (
              <>
                <Loader2 size={14} aria-hidden="true" className="motion-safe:animate-spin" />
                Sending…
              </>
            ) : (
              <>
                <ArrowUp size={14} aria-hidden="true" />
                {phone ? sendShort(drafts) : sendLabel(drafts, bot)}
              </>
            )}
          </button>
        </>
      )}
    </div>
  );
};
