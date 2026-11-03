import React, { useCallback, useEffect, useState } from 'react';
import { useUndoableDelete } from '../undoDelete';
import { UndoNotice } from './UndoNotice';
import { Bot, Check, Loader2, Pencil, Pin, PinOff, Plus, RefreshCw, Trash2, User, X } from 'lucide-react';
import { MAX_MEMORIES, MAX_MEMORY, MAX_PINNED, Memory, continuityApi, formatWhen, memoriesForDisplay, onContinuity } from '../continuity';
import { playSfx } from '../audio/chiptune';
import { editorProps } from '../escape';
import { LessonLine, missionsApi } from '../missions';

const smallBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700`;
const inputClass =
  'w-full bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2 text-xs text-zinc-100 placeholder-zinc-600 focus:border-sky-500 focus:outline-none transition-colors resize-none';

const SourceBadge: React.FC<{ source: Memory['source'] }> = ({ source }) =>
  source === 'agent' ? (
    <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium bg-violet-500/10 text-violet-300 border border-violet-500/20">
      <Bot size={10} aria-hidden="true" /> Bot
    </span>
  ) : (
    <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium bg-sky-500/10 text-sky-300 border border-sky-500/20">
      <User size={10} aria-hidden="true" /> You
    </span>
  );

export const MemoryRow: React.FC<{
  memory: Memory;
  onSave: (text: string) => Promise<string | void>;
  onDelete: () => Promise<string | void>;
  onPin?: (pinned: boolean) => Promise<string | void>;
  /** Tests: start in the Undo window. */
  initialPending?: boolean;
}> = ({ memory, onSave, onDelete, onPin, initialPending = false }) => {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(memory.text);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const removal = useUndoableDelete(undefined, initialPending);

  useEffect(() => {
    if (!editing) setText(memory.text);
  }, [memory.text, editing]);

  const save = async () => {
    const t = text.trim();
    if (!t || t.length > MAX_MEMORY) return;
    setBusy(true);
    const err = await onSave(t);
    setBusy(false);
    if (err) setError(err);
    else {
      setError(null);
      setEditing(false);
    }
  };

  const togglePin = async () => {
    if (!onPin) return;
    setBusy(true);
    const err = await onPin(!memory.pinned);
    setBusy(false);
    setError(err || null);
  };

  const remove = () => {
    setError(null);
    removal.start(async () => {
      const err = await onDelete();
      if (err) throw new Error(err);
    });
  };

  if (removal.pending) return <UndoNotice as="li" text="Memory deleted." onUndo={removal.undo} />;

  return (
    <li
      className={`p-2.5 rounded-lg border space-y-1.5 ${memory.pinned ? 'border-amber-500/30 bg-amber-500/5' : 'border-zinc-800 bg-zinc-950/60'}`}
    >
      <div className="flex items-center gap-2 text-[10px] text-zinc-500">
        <SourceBadge source={memory.source} />
        {memory.pinned && (
          <span className="inline-flex items-center gap-1 text-amber-300">
            <Pin size={10} aria-hidden="true" /> Pinned
          </span>
        )}
        <span title={`Added ${formatWhen(memory.created_at)}`}>
          {memory.updated_at !== memory.created_at ? 'Edited ' : ''}
          {formatWhen(memory.updated_at)}
        </span>
        <span className="ml-auto flex items-center gap-1">
          {onPin && !editing && (
            <button
              type="button"
              onClick={togglePin}
              disabled={busy}
              className={quietBtn}
              aria-label={memory.pinned ? 'Unpin memory' : 'Pin memory'}
              aria-pressed={!!memory.pinned}
              title={memory.pinned ? 'Unpin' : 'Pin: always load this note first'}
            >
              {memory.pinned ? <PinOff size={11} /> : <Pin size={11} />}
            </button>
          )}
          {!editing && (
            <button type="button" onClick={() => setEditing(true)} className={quietBtn} aria-label="Edit memory">
              <Pencil size={11} />
            </button>
          )}
          <button
            type="button"
            onClick={remove}
            disabled={busy}
            className={`${smallBtn} border-zinc-700/70 bg-zinc-800/70 text-zinc-300 hover:text-rose-200`}
            aria-label="Delete memory"
          >
            <Trash2 size={11} />
          </button>
        </span>
      </div>
      {editing ? (
        <div className="space-y-1.5" {...editorProps(text !== memory.text, () => setEditing(false))}>
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            rows={3}
            autoFocus
            aria-label="Memory text"
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) save();
            }}
            className={inputClass}
          />
          <div className="flex items-center gap-1.5">
            <span className={`text-[10px] font-mono ${text.length > MAX_MEMORY ? 'text-rose-400' : 'text-zinc-500'}`}>
              {text.length}/{MAX_MEMORY}
            </span>
            <button
              type="button"
              onClick={() => {
                setText(memory.text);
                setEditing(false);
              }}
              className={`${quietBtn} ml-auto`}
            >
              <X size={11} /> Cancel
            </button>
            <button
              type="button"
              onClick={save}
              disabled={busy || !text.trim() || text.length > MAX_MEMORY}
              className={`${smallBtn} border-sky-500/50 bg-sky-600 text-white hover:bg-sky-500`}
            >
              <Check size={11} /> Save
            </button>
          </div>
        </div>
      ) : (
        <p className="text-xs text-zinc-200 whitespace-pre-wrap break-words">{memory.text}</p>
      )}
      {(error ?? removal.error) && (
        <p className="text-[11px] text-rose-400" role="alert">
          {error ?? `Couldn’t delete it: ${removal.error}`}
        </p>
      )}
    </li>
  );
};

/**
 * "Lessons from earlier work": the short lines a bot sees before similar work, from its last
 * 30 days. Read-only: no buttons, nothing to decide. Shows nothing when there are no lines.
 * `onOpenTask` makes the "View task" link; without it (the Mission board is off) the line is text only.
 */
export const LessonsGroup: React.FC<{ kwantName: string; lines: LessonLine[] | null; onOpenTask?: (taskId: string) => void }> = ({
  kwantName,
  lines,
  onOpenTask,
}) => {
  if (!lines || lines.length === 0) return null;
  return (
    <section aria-labelledby="lessons-h" className="space-y-1.5 pt-1" data-testid="lessons-group">
      <h3 id="lessons-h" className="text-xs font-medium text-zinc-400">
        Lessons from earlier work
      </h3>
      <p className="text-[10px] text-zinc-500 leading-relaxed">
        {kwantName} sees these before similar work. They come from the last 30 days and update on their own.
      </p>
      <ul className="space-y-1.5" aria-labelledby="lessons-h">
        {lines.map((l) => (
          <li key={l.key} className="p-2.5 rounded-lg border border-zinc-800 bg-zinc-950/60 space-y-1">
            <p className="text-xs text-zinc-300 break-words">{l.text}</p>
            {(l.count > 1 || (l.task_id && onOpenTask)) && (
              <div className="flex items-center gap-2 text-[11px]">
                {l.count > 1 && (
                  <span className="px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-300 tabular-nums" title={`Happened ${l.count} times`}>
                    <span aria-hidden="true">×{l.count}</span>
                    <span className="sr-only">{l.count} times</span>
                  </span>
                )}
                {l.task_id && onOpenTask && (
                  <button
                    type="button"
                    onClick={() => onOpenTask(l.task_id as string)}
                    aria-label={`View task for this lesson: ${l.text.length > 70 ? `${l.text.slice(0, 70)}…` : l.text}`}
                    className="rounded px-0.5 py-1.5 -my-1.5 text-sky-300 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                  >
                    View task
                  </button>
                )}
              </div>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
};

/** A kwant's memory: what it keeps between chats. Refreshes live when the kwant changes it. */
export const MemoryPanel: React.FC<{ kwantId: string; kwantName: string; onOpenTask?: (taskId: string) => void }> = ({ kwantId, kwantName, onOpenTask }) => {
  const [memories, setMemories] = useState<Memory[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  // Read-only extra: stays null (and hidden) while loading, when off, on an older server or on any error.
  const [lessons, setLessons] = useState<LessonLine[] | null>(null);

  const reload = useCallback(async () => {
    try {
      setMemories(await continuityApi.memories(kwantId));
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : 'Could not load memories.');
    }
  }, [kwantId]);

  useEffect(() => {
    let live = true;
    setLessons(null);
    missionsApi.lessons(kwantId).then((l) => live && setLessons(l));
    return () => {
      live = false;
    };
  }, [kwantId]);

  useEffect(() => {
    setMemories(null);
    reload();
    return onContinuity('memory_changed', (d) => {
      if (d.kwant_id === kwantId) reload();
    });
  }, [kwantId, reload]);

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    const t = draft.trim();
    if (!t || t.length > MAX_MEMORY) return;
    setAdding(true);
    try {
      await continuityApi.addMemory(kwantId, t);
      setDraft('');
      setAddError(null);
      playSfx('coin');
      await reload();
    } catch (err) {
      setAddError(err instanceof Error ? err.message : 'Could not add it.');
    } finally {
      setAdding(false);
    }
  };

  const wrap = (fn: () => Promise<unknown>) => async () => {
    try {
      await fn();
      await reload();
    } catch (err) {
      return err instanceof Error ? err.message : 'That failed.';
    }
  };

  const full = (memories?.length ?? 0) >= MAX_MEMORIES;

  return (
    <div className="space-y-3">
      <p className="text-[11px] text-zinc-500 leading-relaxed">
        {kwantName} reads these at the start of every turn and can add to them itself. Pinned notes always load first; when the
        list gets long, the newest of the rest are shown and {kwantName} can search the others.
      </p>

      <form onSubmit={add} {...editorProps(!!draft.trim())} className="space-y-1.5">
        <label htmlFor="memory-new" className="block text-xs font-medium text-zinc-400">
          Add a memory
        </label>
        <textarea
          id="memory-new"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          rows={2}
          disabled={full}
          placeholder={full ? `${MAX_MEMORIES} memories is the limit; delete one first.` : 'e.g. Prefers short answers with a code sample.'}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) add(e);
          }}
          className={inputClass}
        />
        <div className="flex items-center gap-2">
          <span className={`text-[10px] font-mono ${draft.length > MAX_MEMORY ? 'text-rose-400' : 'text-zinc-500'}`}>
            {draft.length}/{MAX_MEMORY}
          </span>
          {addError && (
            <span className="text-[11px] text-rose-400" role="alert">
              {addError}
            </span>
          )}
          <button
            type="submit"
            disabled={adding || full || !draft.trim() || draft.length > MAX_MEMORY}
            className={`${smallBtn} ml-auto border-sky-500/50 bg-sky-600 text-white hover:bg-sky-500`}
          >
            {adding ? <Loader2 size={11} className="animate-spin motion-reduce:animate-none" /> : <Plus size={11} />} Add
          </button>
        </div>
      </form>

      {loadError && (
        <div className="flex items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs" role="alert">
          <span>{loadError}</span>
          <button type="button" onClick={reload} className={quietBtn}>
            <RefreshCw size={12} /> Retry
          </button>
        </div>
      )}
      {!memories && !loadError && (
        <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
          <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Loading memories…
        </p>
      )}
      {memories && memories.length === 0 && (
        <p className="text-xs text-zinc-500 border border-dashed border-zinc-800 rounded-lg p-4 text-center">
          Nothing remembered yet. Add something above, or ask {kwantName} to remember it.
        </p>
      )}
      {memories && memories.length > 0 && (
        <>
          <p className="text-[10px] text-zinc-500">
            {memories.length} of {MAX_MEMORIES} · {memories.filter((m) => m.pinned).length} of {MAX_PINNED} pinned · pinned, then newest first
          </p>
          <ul className="space-y-1.5" aria-label={`${kwantName}'s memories`}>
            {memoriesForDisplay(memories).map((m) => (
              <MemoryRow
                key={m.id}
                memory={m}
                onSave={(t) => wrap(() => continuityApi.updateMemory(kwantId, m.id, t))()}
                onPin={(pinned) => wrap(() => continuityApi.pinMemory(kwantId, m.id, pinned))()}
                onDelete={wrap(() => continuityApi.removeMemory(kwantId, m.id))}
              />
            ))}
          </ul>
        </>
      )}
      <LessonsGroup kwantName={kwantName} lines={lessons} onOpenTask={onOpenTask} />
    </div>
  );
};
