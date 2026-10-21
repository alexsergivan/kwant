import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Bell, Camera, Check, CheckCircle2, ChevronLeft, Copy, ExternalLink, FileText, Loader2, Pencil, Plus, Quote, RotateCcw, Trash2, Upload, X } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { fileKind, formatBytes, isInlineImage, uploadAttachment } from '../attachments';
import { LooseCase, LooseEndsError, draftCopyText, draftOpen, looseApi, shortDate } from '../looseEnds';
import {
  FIELD_STATE_LABEL,
  Extraction,
  LETTER_ACCEPT,
  LIST_FIELDS,
  LetterField,
  LetterView as LetterViewData,
  ListKey,
  MAX_LETTER_PAGES,
  Reminder,
  SINGLE_FIELDS,
  SingleKey,
  checkLetterFiles,
  clearBody,
  confirmBody,
  deleteErrorText,
  draftChannelText,
  fieldState,
  fieldValueError,
  isIsoDate,
  letterApi,
  letterErrorText,
  listBody,
  readingText,
  reminderGate,
  reminderLabel,
  reminderWhen,
  setBody,
  stillReading,
} from '../letters';

const actionBtn =
  'inline-flex items-center gap-1.5 min-h-[36px] sm:min-h-[30px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
const primaryBtn = `${actionBtn} bg-sky-500 hover:bg-sky-400 text-zinc-950 font-semibold`;
const input =
  'w-full min-w-0 min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-sky-500';
const sectionTitle = 'text-[11px] font-semibold text-zinc-500 uppercase tracking-wider';

const STATE_TONE: Record<string, string> = {
  confirmed: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  uncertain: 'border-amber-500/40 bg-amber-500/15 text-amber-200',
  unconfirmed: 'border-zinc-700 bg-zinc-800 text-zinc-300',
  empty: 'border-zinc-800 bg-transparent text-zinc-500',
};
const BANNER_TONE = {
  sky: 'border-sky-500/30 bg-sky-500/10 text-sky-100',
  emerald: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-100',
  amber: 'border-amber-500/30 bg-amber-500/10 text-amber-100',
  rose: 'border-rose-500/30 bg-rose-500/10 text-rose-100',
};

const spin = 'animate-spin motion-reduce:animate-none';

/** Saves a change; resolves to an error message, or null when it was saved. */
type Save = (body: Record<string, unknown>) => Promise<string | null>;

const coarsePointer = () => typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;

// ---------------------------------------------------------------------------
// Add a letter

interface Picked {
  key: string;
  file: File;
  preview: string | null;
  /** Set once uploaded, so a retry after "busy" doesn't upload again. */
  attachmentId?: string;
  progress: number;
}

/** Photograph (on a phone) or pick 1–5 pages, upload them, then ask kwant to read them as one letter. */
export const AddLetterForm: React.FC<{ onCreated: (caseId: string) => void; onCancel: () => void; initialCamera?: boolean }> = ({ onCreated, onCancel, initialCamera }) => {
  const id = useId();
  const [pages, setPages] = useState<Picked[]>([]);
  const [phase, setPhase] = useState<'idle' | 'uploading' | 'starting'>('idle');
  const [error, setError] = useState<{ text: string; calm: boolean } | null>(null);
  const [camera] = useState(() => initialCamera ?? coarsePointer());
  const pickRef = useRef<HTMLInputElement>(null);
  const cameraRef = useRef<HTMLInputElement>(null);
  const pagesRef = useRef(pages);
  pagesRef.current = pages;

  useEffect(() => () => pagesRef.current.forEach((p) => p.preview && URL.revokeObjectURL(p.preview)), []);

  const problem = pages.length ? checkLetterFiles(pages.map((p) => ({ name: p.file.name, type: p.file.type, size: p.file.size }))) : null;
  const working = phase !== 'idle';

  const add = (list: FileList | null) => {
    if (!list || list.length === 0) return;
    const added = Array.from(list).map((file, i) => ({
      key: `${Date.now()}-${i}-${file.name}`,
      file,
      preview: isInlineImage(file.type) ? URL.createObjectURL(file) : null,
      progress: 0,
    }));
    setPages((cur) => [...cur, ...added]);
    setError(null);
  };

  const remove = (key: string) => {
    setPages((cur) => {
      const gone = cur.find((p) => p.key === key);
      if (gone?.preview) URL.revokeObjectURL(gone.preview);
      return cur.filter((p) => p.key !== key);
    });
    setError(null);
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const why = checkLetterFiles(pages.map((p) => ({ name: p.file.name, type: p.file.type, size: p.file.size })));
    if (why) return setError({ text: why, calm: false });
    setError(null);
    setPhase('uploading');
    const ids: string[] = [];
    try {
      for (const p of pages) {
        if (p.attachmentId) {
          ids.push(p.attachmentId);
          continue;
        }
        const a = await uploadAttachment(p.file, (f) => setPages((cur) => cur.map((x) => (x.key === p.key ? { ...x, progress: f } : x))));
        ids.push(a.id);
        setPages((cur) => cur.map((x) => (x.key === p.key ? { ...x, attachmentId: a.id, progress: 1 } : x)));
      }
    } catch (err) {
      setPhase('idle');
      return setError({ text: `Couldn’t upload a page: ${err instanceof Error ? err.message : 'something went wrong.'}`, calm: false });
    }
    setPhase('starting');
    try {
      const r = await letterApi.create(ids);
      playSfx('coin');
      onCreated(r.case_id);
    } catch (err) {
      const t = letterErrorText(err);
      // A page used by another letter must be uploaded again; a busy server can take the same ones.
      if (err instanceof LooseEndsError && err.status === 409 && err.code !== 'busy') setPages((cur) => cur.map((x) => ({ ...x, attachmentId: undefined, progress: 0 })));
      setError(t);
      setPhase('idle');
    }
  };

  return (
    <form onSubmit={submit} className="rounded-xl border border-sky-500/30 bg-zinc-900/60 p-3 sm:p-4 space-y-3" aria-labelledby={`${id}-h`} data-add-letter>
      <div className="flex items-center justify-between gap-2">
        <h3 id={`${id}-h`} className="text-sm font-semibold text-zinc-100">
          Add a letter
        </h3>
        <button type="button" onClick={onCancel} aria-label="Close" className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
          <X size={14} />
        </button>
      </div>
      <p className="text-xs text-zinc-400">
        Photograph each page, or pick up to {MAX_LETTER_PAGES} photos or PDFs of one letter (PNG, JPEG, WebP or PDF). It’s read into a case with the deadline and a reply draft. Nothing is sent to anyone.
      </p>

      <div className="flex flex-wrap gap-2">
        {camera && (
          <>
            <input ref={cameraRef} id={`${id}-cam`} type="file" accept="image/*" capture="environment" className="sr-only" tabIndex={-1} onChange={(e) => (add(e.target.files), (e.target.value = ''))} />
            <button type="button" disabled={working || pages.length >= MAX_LETTER_PAGES} onClick={() => cameraRef.current?.click()} className={primaryBtn}>
              <Camera size={14} aria-hidden="true" /> {pages.length ? 'Photograph another page' : 'Take a photo'}
            </button>
          </>
        )}
        <input ref={pickRef} id={`${id}-pick`} type="file" multiple accept={LETTER_ACCEPT} className="sr-only" tabIndex={-1} onChange={(e) => (add(e.target.files), (e.target.value = ''))} data-letter-files />
        <button type="button" disabled={working || pages.length >= MAX_LETTER_PAGES} onClick={() => pickRef.current?.click()} className={camera ? quietBtn : primaryBtn}>
          <Upload size={14} aria-hidden="true" /> {camera ? 'Choose files' : pages.length ? 'Add more pages' : 'Choose photos or PDFs'}
        </button>
      </div>

      {pages.length > 0 && (
        <ol className="grid grid-cols-2 sm:grid-cols-3 gap-2" aria-label="Pages to read">
          {pages.map((p, i) => (
            <li key={p.key} className="relative min-w-0 rounded-lg border border-zinc-800 bg-zinc-950/60 p-2 space-y-1">
              {p.preview ? (
                <img src={p.preview} alt={`Page ${i + 1}`} className="w-full h-24 object-cover object-top rounded" />
              ) : (
                <div className="h-24 flex items-center justify-center rounded bg-zinc-900 text-zinc-500">
                  <FileText size={28} aria-hidden="true" />
                </div>
              )}
              <p className="text-[11px] text-zinc-300 truncate" title={p.file.name}>
                {i + 1}. {p.file.name}
              </p>
              <p className="text-[11px] text-zinc-500">
                {fileKind(p.file.type, p.file.name)} · {formatBytes(p.file.size)}
                {p.attachmentId ? ' · uploaded' : phase === 'uploading' && p.progress > 0 ? ` · ${Math.round(p.progress * 100)}%` : ''}
              </p>
              <button
                type="button"
                disabled={working}
                onClick={() => remove(p.key)}
                aria-label={`Remove page ${i + 1}, ${p.file.name}`}
                className="absolute top-1 right-1 p-1.5 rounded-md bg-zinc-900/90 text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800"
              >
                <X size={12} />
              </button>
            </li>
          ))}
        </ol>
      )}

      <div role="status" aria-live="polite">
        {(error || problem) && (
          <p
            className={`flex items-start gap-1.5 text-xs break-words ${error?.calm ? 'px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-amber-100' : 'text-rose-300'}`}
            data-letter-error={error?.calm ? 'busy' : 'invalid'}
          >
            {!error?.calm && <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />}
            {error?.text ?? problem}
          </p>
        )}
      </div>

      <div className="flex flex-wrap justify-end gap-2">
        <button type="button" onClick={onCancel} disabled={working} className={quietBtn}>
          Cancel
        </button>
        <button type="submit" disabled={working || pages.length === 0 || !!problem} className={primaryBtn}>
          {working ? <Loader2 size={13} className={spin} aria-hidden="true" /> : <FileText size={13} aria-hidden="true" />}
          {phase === 'uploading' ? 'Uploading…' : phase === 'starting' ? 'Starting…' : error?.calm ? 'Try again' : `Read ${pages.length > 1 ? `${pages.length} pages` : 'the letter'}`}
        </button>
      </div>
    </form>
  );
};

// ---------------------------------------------------------------------------
// The letter itself

export const StateBadge: React.FC<{ field: LetterField | null | undefined }> = ({ field }) => {
  const s = fieldState(field);
  return (
    <span className={`inline-flex items-center gap-1 px-1.5 py-px rounded-full border text-[11px] ${STATE_TONE[s]}`} data-state={s}>
      {s === 'uncertain' && <AlertTriangle size={11} aria-hidden="true" />}
      {s === 'confirmed' && <Check size={11} aria-hidden="true" />}
      {FIELD_STATE_LABEL[s]}
    </span>
  );
};

const FieldQuote: React.FC<{ field: LetterField }> = ({ field }) =>
  field.quote ? (
    <blockquote className="flex gap-1.5 text-[11px] text-zinc-400 whitespace-pre-wrap break-words">
      <Quote size={11} className="mt-0.5 shrink-0 text-zinc-600" aria-hidden="true" />
      <span>
        <span className="sr-only">From the letter: </span>
        {field.quote}
      </span>
    </blockquote>
  ) : (
    <p className="text-[11px] text-zinc-500">No matching words were found in the letter. Check the page.</p>
  );

/** One field: its value, the quote it came from, whether it's uncertain or confirmed, and Confirm / Edit / Clear. */
export const FieldRow: React.FC<{ fkey: SingleKey; label: string; hint: string; field: LetterField | null | undefined; disabled: boolean; onSave: Save; initialEditing?: boolean; missing?: string }> = ({
  fkey,
  label,
  hint,
  field,
  disabled,
  onSave,
  initialEditing = false,
  missing = 'Not found in the letter.',
}) => {
  const id = useId();
  const [editing, setEditing] = useState(initialEditing);
  const [value, setValue] = useState(field?.value ?? '');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const state = fieldState(field);
  const isDate = fkey === 'deadline';
  // Confirming "as it is" still has to parse: a deadline must be a real YYYY-MM-DD date.
  const confirmBlocked = isDate && field?.value && !isIsoDate(field.value) ? 'This isn’t a YYYY-MM-DD date yet: edit it before confirming.' : null;

  const save = async (body: Record<string, unknown>) => {
    setSaving(true);
    setError(null);
    const err = await onSave(body);
    setSaving(false);
    if (err) setError(err);
    else setEditing(false);
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const why = fieldValueError(fkey, value);
    if (why) return setError(why);
    save(setBody(fkey, value));
  };

  const busy = disabled || saving;
  return (
    <div className={`rounded-lg border p-2.5 space-y-1.5 min-w-0 ${state === 'uncertain' ? 'border-amber-500/40 bg-amber-500/5' : 'border-zinc-800 bg-zinc-950/40'}`} data-field={fkey} data-state={state}>
      <div className="flex flex-wrap items-center justify-between gap-1.5">
        <span id={`${id}-l`} className="text-[11px] font-semibold text-zinc-400">
          {label}
        </span>
        <StateBadge field={field} />
      </div>
      {!editing && (
        <>
          {field?.value ? (
            <p className="text-sm text-zinc-100 break-words" aria-labelledby={`${id}-l`}>
              {isDate && isIsoDate(field.value) ? (
                <>
                  {shortDate(field.value)} <span className="font-mono text-[11px] text-zinc-500">{field.value}</span>
                </>
              ) : (
                field.value
              )}
            </p>
          ) : (
            <p className="text-xs text-zinc-500">{missing}</p>
          )}
          {field?.value && <FieldQuote field={field} />}
          {confirmBlocked && <p className="text-[11px] text-amber-200">{confirmBlocked}</p>}
          <div className="flex flex-wrap gap-2 pt-0.5">
            {field?.value && state !== 'confirmed' && (
              <button type="button" disabled={busy || !!confirmBlocked} onClick={() => save(confirmBody(fkey))} className={`${actionBtn} bg-emerald-600/90 hover:bg-emerald-500 text-white`}>
                <Check size={13} aria-hidden="true" /> Confirm
              </button>
            )}
            <button type="button" disabled={busy} onClick={() => (setValue(field?.value ?? ''), setError(null), setEditing(true))} className={quietBtn}>
              {field?.value ? <Pencil size={13} aria-hidden="true" /> : <Plus size={13} aria-hidden="true" />}
              {field?.value ? 'Edit' : 'Add'}
              <span className="sr-only"> {label}</span>
            </button>
            {field?.value && (
              <button type="button" disabled={busy} onClick={() => save(clearBody(fkey))} className={`${actionBtn} text-zinc-300 hover:bg-zinc-800`}>
                <X size={13} aria-hidden="true" /> Clear<span className="sr-only"> {label}</span>
              </button>
            )}
            {saving && <Loader2 size={13} className={`${spin} self-center text-zinc-400`} aria-label="Saving" />}
          </div>
        </>
      )}
      {editing && (
        <form onSubmit={submit} className="space-y-2" noValidate>
          {fkey === 'request' ? (
            <textarea
              aria-labelledby={`${id}-l`}
              className={`${input} min-h-[72px]`}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder={hint}
              maxLength={500}
              autoFocus
            />
          ) : (
            <input
              aria-labelledby={`${id}-l`}
              aria-describedby={isDate ? `${id}-fmt` : undefined}
              aria-invalid={!!error}
              className={`${input} ${isDate ? 'font-mono' : ''}`}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder={hint}
              maxLength={500}
              inputMode={isDate ? 'numeric' : undefined}
              autoComplete="off"
              spellCheck={!isDate}
              autoFocus
            />
          )}
          {isDate && (
            <p id={`${id}-fmt`} className="text-[11px] text-zinc-500">
              Year-month-day, for example 2026-11-14. Saving confirms it.
            </p>
          )}
          {error && (
            <p role="alert" className="text-[11px] text-rose-300 break-words">
              {error}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <button type="submit" disabled={busy} className={primaryBtn}>
              {saving ? <Loader2 size={13} className={spin} aria-hidden="true" /> : <Check size={13} aria-hidden="true" />} Save
            </button>
            <button type="button" disabled={saving} onClick={() => (setEditing(false), setError(null))} className={quietBtn}>
              Cancel
            </button>
          </div>
        </form>
      )}
      {!editing && error && (
        <p role="alert" className="text-[11px] text-rose-300 break-words">
          {error}
        </p>
      )}
    </div>
  );
};

/** References, amounts or the checklist. The server takes the whole list, and saving confirms all of it. */
export const ListField: React.FC<{ fkey: ListKey; label: string; hint: string; empty: string; items: LetterField[]; disabled: boolean; onSave: Save; initialEditing?: boolean }> = ({
  fkey,
  label,
  hint,
  empty,
  items,
  disabled,
  onSave,
  initialEditing = false,
}) => {
  const id = useId();
  const [editing, setEditing] = useState(initialEditing);
  const [rows, setRows] = useState<Array<{ value: string; done?: boolean | null }>>(items.map((i) => ({ value: i.value, done: i.done })));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const checklist = fkey === 'checklist';
  const unchecked = items.some((i) => fieldState(i) !== 'confirmed');
  const busy = disabled || saving;

  const save = async (list: Array<{ value: string; done?: boolean | null }>, confirm: boolean) => {
    setSaving(true);
    setError(null);
    const err = await onSave(listBody(fkey, list, confirm));
    setSaving(false);
    if (err) setError(err);
    else setEditing(false);
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const kept = rows.filter((r) => r.value.trim());
    for (const r of kept) {
      const why = fieldValueError(fkey, r.value);
      if (why) return setError(`“${r.value.trim().slice(0, 40)}”: ${why}`);
    }
    if (kept.length > 20) return setError('At most 20 entries.');
    save(kept, true);
  };

  const tick = (i: number, done: boolean) => save(items.map((x, j) => ({ value: x.value, done: j === i ? done : x.done })), false);

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-950/40 p-2.5 space-y-2 min-w-0" data-list={fkey}>
      <div className="flex flex-wrap items-center justify-between gap-1.5">
        <span id={`${id}-l`} className="text-[11px] font-semibold text-zinc-400">
          {label}
        </span>
        {items.length > 0 && !editing && <span className="text-[11px] text-zinc-500">{unchecked ? 'Not all checked' : 'All confirmed'}</span>}
      </div>

      {!editing && (
        <>
          {items.length === 0 ? (
            <p className="text-xs text-zinc-500">{empty}</p>
          ) : (
            <ul className="space-y-1.5" aria-labelledby={`${id}-l`}>
              {items.map((it, i) => {
                const s = fieldState(it);
                return (
                  <li key={`${i}-${it.value}`} className={`rounded-md px-2 py-1.5 space-y-1 ${s === 'uncertain' ? 'border border-amber-500/40 bg-amber-500/5' : 'border border-transparent'}`} data-state={s}>
                    <div className="flex items-start gap-2 min-w-0">
                      {checklist ? (
                        <label className="flex items-start gap-2 min-w-0 flex-1 cursor-pointer">
                          <input type="checkbox" className="mt-0.5 h-4 w-4 shrink-0 accent-emerald-500" checked={!!it.done} disabled={busy} onChange={(e) => tick(i, e.target.checked)} />
                          <span className={`text-xs break-words ${it.done ? 'text-zinc-500 line-through' : 'text-zinc-100'}`}>{it.value}</span>
                        </label>
                      ) : (
                        <span className={`text-xs text-zinc-100 break-words min-w-0 flex-1 ${fkey === 'amounts' || fkey === 'references' ? 'font-mono' : ''}`}>{it.value}</span>
                      )}
                      <StateBadge field={it} />
                    </div>
                    {s !== 'confirmed' && <FieldQuote field={it} />}
                  </li>
                );
              })}
            </ul>
          )}
          <div className="flex flex-wrap gap-2">
            {items.length > 0 && unchecked && (
              <button type="button" disabled={busy} onClick={() => save(items.map((x) => ({ value: x.value, done: x.done })), true)} className={`${actionBtn} bg-emerald-600/90 hover:bg-emerald-500 text-white`}>
                <Check size={13} aria-hidden="true" /> Confirm all<span className="sr-only"> {label}</span>
              </button>
            )}
            <button
              type="button"
              disabled={busy}
              onClick={() => (setRows(items.length ? items.map((x) => ({ value: x.value, done: x.done })) : [{ value: '' }]), setError(null), setEditing(true))}
              className={quietBtn}
            >
              {items.length ? <Pencil size={13} aria-hidden="true" /> : <Plus size={13} aria-hidden="true" />}
              {items.length ? 'Edit' : 'Add'}
              <span className="sr-only"> {label}</span>
            </button>
            {saving && <Loader2 size={13} className={`${spin} self-center text-zinc-400`} aria-label="Saving" />}
          </div>
        </>
      )}

      {editing && (
        <form onSubmit={submit} className="space-y-2" noValidate>
          {rows.map((r, i) => (
            <div key={i} className="flex items-center gap-2 min-w-0">
              <input
                aria-label={`${label} ${i + 1}`}
                className={`${input} ${fkey === 'checklist' ? '' : 'font-mono'}`}
                value={r.value}
                placeholder={hint}
                maxLength={500}
                autoComplete="off"
                onChange={(e) => setRows((cur) => cur.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))}
              />
              <button type="button" onClick={() => setRows((cur) => cur.filter((_, j) => j !== i))} aria-label={`Remove ${label.toLowerCase()} ${i + 1}`} className="shrink-0 p-2 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
                <X size={14} />
              </button>
            </div>
          ))}
          {rows.length < 20 && (
            <button type="button" onClick={() => setRows((cur) => [...cur, { value: '' }])} className={quietBtn}>
              <Plus size={13} aria-hidden="true" /> Add one
            </button>
          )}
          {fkey === 'amounts' && <p className="text-[11px] text-zinc-500">Include the currency, like GBP 142.60 or £142.60.</p>}
          <p className="text-[11px] text-zinc-500">Saving confirms every entry in this list.</p>
          {error && (
            <p role="alert" className="text-[11px] text-rose-300 break-words">
              {error}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <button type="submit" disabled={busy} className={primaryBtn}>
              {saving ? <Loader2 size={13} className={spin} aria-hidden="true" /> : <Check size={13} aria-hidden="true" />} Save
            </button>
            <button type="button" disabled={saving} onClick={() => (setEditing(false), setError(null))} className={quietBtn}>
              Cancel
            </button>
          </div>
        </form>
      )}
      {!editing && error && (
        <p role="alert" className="text-[11px] text-rose-300 break-words">
          {error}
        </p>
      )}
    </div>
  );
};

/** The pages as the person sent them: what each field should be checked against. */
export const LetterPages: React.FC<{ extraction: Extraction | null }> = ({ extraction }) => {
  const pages = extraction?.pages ?? [];
  return (
    <section aria-labelledby="letter-pages" className="space-y-2 min-w-0">
      <h3 id="letter-pages" className={sectionTitle}>
        The letter{pages.length > 1 ? ` · ${pages.length} pages` : ''}
      </h3>
      {pages.length === 0 && <p className="text-xs text-zinc-500">No pages are stored for this letter.</p>}
      <ol className="space-y-3">
        {pages.map((p, i) => (
          <li key={p.id} className="rounded-lg border border-zinc-800 bg-zinc-900/40 overflow-hidden">
            {isInlineImage(p.mime) ? (
              <a href={p.url} target="_blank" rel="noopener noreferrer" title="Open full size">
                <img src={p.url} alt={`Page ${i + 1} of the letter`} loading="lazy" className="block w-full max-h-[75vh] object-contain bg-zinc-950" />
              </a>
            ) : (
              <a href={p.url} target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 p-3 text-xs text-sky-300 hover:text-sky-200 hover:bg-zinc-800/60">
                <FileText size={18} className="shrink-0" aria-hidden="true" />
                <span className="min-w-0 break-all">{p.name}</span>
                <span className="ml-auto shrink-0 inline-flex items-center gap-1 text-[11px]">
                  Open {fileKind(p.mime, p.name)} <ExternalLink size={11} aria-hidden="true" />
                </span>
              </a>
            )}
            <p className="px-3 py-1 text-[11px] text-zinc-500 border-t border-zinc-800 truncate">
              Page {i + 1} · {p.name}
            </p>
          </li>
        ))}
      </ol>
      {extraction && extraction.text.trim() && (
        <details className="rounded-lg border border-zinc-800 bg-zinc-900/40">
          <summary className="cursor-pointer px-3 py-2 text-xs text-zinc-300 min-h-[36px] flex items-center">What was read from it</summary>
          <pre className="max-h-80 overflow-y-auto whitespace-pre-wrap break-words font-sans text-[11px] text-zinc-300 px-3 pb-3">{extraction.text}</pre>
        </details>
      )}
    </section>
  );
};

/** Reminders: off until the deadline is confirmed and certain; then 7, 1 and 0 days before. */
export const LetterReminders: React.FC<{ deadline: LetterField | null | undefined; reminders: Reminder[]; closed: boolean }> = ({ deadline, reminders, closed }) => {
  const gate = reminderGate(deadline);
  return (
    <section aria-labelledby="letter-reminders" className="space-y-2" data-reminders={gate.on && !closed ? 'on' : 'off'}>
      <h3 id="letter-reminders" className={`${sectionTitle} flex items-center gap-1.5`}>
        <Bell size={12} aria-hidden="true" /> Reminders
      </h3>
      {closed ? (
        <p className="text-xs text-zinc-400">This letter is closed, so its reminders are off. Reopen it to bring back the ones still ahead.</p>
      ) : (
        <p className={`text-xs ${gate.on ? 'text-zinc-300' : 'px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-amber-100'}`}>{gate.text}</p>
      )}
      {!closed && gate.on && reminders.length === 0 && <p className="text-xs text-zinc-500">No reminders are left: the deadline has passed or is today.</p>}
      {reminders.length > 0 && (
        <ul className="divide-y divide-zinc-800 rounded-lg border border-zinc-800">
          {[...reminders]
            .sort((a, b) => b.offset_days - a.offset_days)
            .map((r) => (
              <li key={r.offset_days} className="flex flex-wrap items-center gap-x-3 gap-y-0.5 px-3 py-2 text-xs">
                <span className="text-zinc-200 w-28 shrink-0">{reminderLabel(r.offset_days)}</span>
                <time dateTime={r.due_at} className="text-zinc-400">
                  {reminderWhen(r.due_at)}
                </time>
                <span className={`ml-auto px-1.5 py-px rounded-full border text-[11px] ${r.fired_at ? STATE_TONE.confirmed : STATE_TONE.unconfirmed}`}>
                  {r.fired_at ? `Sent ${reminderWhen(r.fired_at)}` : 'Due'}
                </span>
              </li>
            ))}
        </ul>
      )}
      <p className="text-[11px] text-zinc-500">A reminder is a notification with counts only, never the sender, amounts or words of the letter.</p>
    </section>
  );
};

/** The reply: copy it, or open it in the person's own mail app. Nothing is sent from kwant. */
export const LetterDraft: React.FC<{ c: LooseCase }> = ({ c }) => {
  const [copied, setCopied] = useState<'ok' | 'failed' | null>(null);
  const d = c.draft;
  if (!d) return null;
  const open = d.channel === 'email' ? draftOpen(d) : null;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(draftCopyText(d));
      setCopied('ok');
      playSfx('click');
    } catch {
      setCopied('failed');
    }
  };
  return (
    <section aria-labelledby="letter-draft" className="rounded-lg border border-sky-500/20 bg-sky-500/5 p-3 space-y-2" data-draft-channel={d.channel}>
      <h3 id="letter-draft" className="text-[11px] font-semibold text-sky-200 break-words">
        {d.channel === 'letter' ? 'Reply by post' : 'Reply by email'}
        {d.to ? ` to ${d.to}` : ''}
        {d.generated_by === 'model' && <span className="ml-1 font-normal text-zinc-500">(written by a model)</span>}
      </h3>
      <p className="text-[11px] text-amber-200" data-nothing-sent>
        {draftChannelText(d.channel)}
      </p>
      {d.subject && <p className="text-xs text-zinc-300 break-words">Subject: {d.subject}</p>}
      <pre className="max-h-64 overflow-y-auto whitespace-pre-wrap break-words font-sans text-xs text-zinc-100 bg-zinc-950/60 border border-zinc-800 rounded-lg p-2.5 select-text">{d.body}</pre>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={copy} className={quietBtn}>
          {copied === 'ok' ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
          {copied === 'ok' ? 'Copied' : 'Copy reply'}
        </button>
        {open && (
          <a href={open.href} rel="noopener noreferrer" className={quietBtn}>
            <ExternalLink size={13} aria-hidden="true" /> {open.label}
          </a>
        )}
        <span role="status" className="text-[11px] text-zinc-400">
          {copied === 'failed' ? 'Couldn’t copy here: select the text above and copy it.' : ''}
        </span>
      </div>
    </section>
  );
};

export interface LetterDetailProps {
  view: LetterViewData | null;
  loading: boolean;
  error: string | null;
  notice?: { text: string; calm: boolean } | null;
  acting?: boolean;
  onSave: Save;
  onDone: () => void;
  onReopen: () => void;
  onDelete: () => void;
  onBack: () => void;
  onRetry: () => void;
  /** Tests open the delete confirmation. */
  initialConfirmDelete?: boolean;
}

/** One letter: reading progress, the pages beside the fields, reminders, the reply and what to do with it. */
export const LetterDetail: React.FC<LetterDetailProps> = ({ view, loading, error, notice = null, acting = false, onSave, onDone, onReopen, onDelete, onBack, onRetry, initialConfirmDelete = false }) => {
  const [confirmDelete, setConfirmDelete] = useState(initialConfirmDelete);
  const c = view?.case;
  const e = view?.extraction ?? null;
  const reading = stillReading(view);
  const status = readingText(e);
  const fields = e?.fields ?? {};
  const closed = !!c && (c.status === 'closed' || c.status === 'dismissed');
  const canEdit = !!e && !reading;
  // Only a letter that was read can say a field was "not found" in it
  const wasRead = e?.status === 'read';

  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden" data-letter={c?.id}>
      <div className="h-14 border-b border-zinc-800/80 px-2 sm:px-6 flex items-center gap-2 bg-zinc-900/30 backdrop-blur-sm select-none">
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to Loose ends"
          className="shrink-0 p-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <ChevronLeft size={18} />
        </button>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">{c?.title ?? 'Letter'}</h2>
          <p className="text-[11px] text-zinc-400 truncate mt-0.5">Loose ends · letter</p>
        </div>
        {c && closed && <span className="shrink-0 px-1.5 py-px rounded-full border text-[11px] border-emerald-500/30 bg-emerald-500/10 text-emerald-200">{c.status === 'closed' ? 'Done' : 'Dismissed'}</span>}
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-6" aria-busy={loading || reading}>
        <div className="max-w-6xl mx-auto space-y-4">
          {error && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex flex-wrap items-center gap-2 text-xs text-rose-300">
              <span className="break-words">Couldn’t load the letter: {error}</span>
              <button type="button" onClick={onRetry} className="ml-auto px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10">
                Retry
              </button>
            </div>
          )}
          {!view && loading && (
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <Loader2 size={14} className={spin} aria-hidden="true" /> Loading…
            </p>
          )}

          {view && (
            <>
              <div role="status" aria-live="polite" className={`px-3 py-2 rounded-lg border text-xs space-y-1 ${BANNER_TONE[status.tone]}`} data-read-status={e?.status ?? 'none'}>
                <p className="font-semibold flex items-center gap-1.5">
                  {reading && <Loader2 size={13} className={spin} aria-hidden="true" />}
                  {status.title}
                </p>
                <p className="break-words opacity-90">{status.body}</p>
                {reading && (
                  <div className="h-1 rounded-full bg-sky-950 overflow-hidden" role="progressbar" aria-label="Reading the letter" aria-valuetext="Reading">
                    <div className="h-full w-1/3 rounded-full bg-sky-400 animate-pulse motion-reduce:animate-none" />
                  </div>
                )}
              </div>

              <div role="status" aria-live="polite">
                {notice && (
                  <p className={`px-3 py-2 rounded-lg border text-xs break-words ${notice.calm ? 'border-amber-500/30 bg-amber-500/10 text-amber-100' : 'border-rose-500/30 bg-rose-500/10 text-rose-200'}`}>{notice.text}</p>
                )}
              </div>

              <div className="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-5 items-start">
                <div className="lg:sticky lg:top-0 min-w-0">
                  <LetterPages extraction={e} />
                </div>

                <div className="space-y-5 min-w-0">
                  <section aria-labelledby="letter-fields" className="space-y-2">
                    <h3 id="letter-fields" className={sectionTitle}>
                      What it says
                    </h3>
                    {reading ? (
                      <p className="text-xs text-zinc-500">The fields appear here once the letter has been read.</p>
                    ) : (
                      <>
                        <p className="text-[11px] text-zinc-500">Nothing is confirmed until you confirm it. Fields marked Uncertain need a look at the page first.</p>
                        {SINGLE_FIELDS.map((f) => (
                          <FieldRow key={`${f.key}-${fields[f.key]?.value ?? ''}-${fields[f.key]?.confirmed}`} fkey={f.key} label={f.label} hint={f.hint} field={fields[f.key]} disabled={!canEdit || acting} onSave={onSave} missing={wasRead ? undefined : 'Not filled in yet.'} />
                        ))}
                        {LIST_FIELDS.map((f) => (
                          <ListField
                            key={`${f.key}-${JSON.stringify(fields[f.key] ?? [])}`}
                            fkey={f.key}
                            label={f.label}
                            hint={f.hint}
                            empty={wasRead ? f.empty : 'None added yet.'}
                            items={fields[f.key] ?? []}
                            disabled={!canEdit || acting}
                            onSave={onSave}
                          />
                        ))}
                      </>
                    )}
                  </section>

                  {!reading && <LetterReminders deadline={fields.deadline} reminders={view.reminders} closed={closed} />}

                  {!reading && c && !closed && <LetterDraft c={c} />}

                  {c && (
                    <section aria-label="What to do with this letter" className="space-y-2 border-t border-zinc-800 pt-4">
                      <div className="flex flex-wrap gap-2">
                        {closed ? (
                          <button type="button" disabled={acting} onClick={onReopen} className={quietBtn}>
                            <RotateCcw size={13} aria-hidden="true" /> Reopen
                          </button>
                        ) : (
                          <button type="button" disabled={acting || reading} onClick={onDone} className={`${actionBtn} bg-emerald-600/90 hover:bg-emerald-500 text-white`}>
                            <CheckCircle2 size={13} aria-hidden="true" /> Mark done
                          </button>
                        )}
                        {!confirmDelete && (
                          <button type="button" disabled={acting} onClick={() => setConfirmDelete(true)} className={`${actionBtn} text-rose-300 hover:bg-rose-500/10`}>
                            <Trash2 size={13} aria-hidden="true" /> Delete letter
                          </button>
                        )}
                        {acting && <Loader2 size={13} className={`${spin} self-center text-zinc-400`} aria-label="Saving" />}
                      </div>
                      {confirmDelete && (
                        <div role="group" aria-label="Delete this letter" className="rounded-lg border border-rose-500/30 bg-rose-500/5 p-3 space-y-2" data-confirm-delete>
                          <p className="text-xs text-rose-100">Delete this letter, its pages, everything read from it and its reminders? This can’t be undone.</p>
                          <div className="flex flex-wrap gap-2">
                            <button type="button" disabled={acting} onClick={() => (setConfirmDelete(false), onDelete())} className={`${actionBtn} bg-rose-600 hover:bg-rose-500 text-white`}>
                              <Trash2 size={13} aria-hidden="true" /> Delete it
                            </button>
                            <button type="button" onClick={() => setConfirmDelete(false)} className={quietBtn}>
                              Keep it
                            </button>
                          </div>
                        </div>
                      )}
                    </section>
                  )}
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
};

/** Loads one letter, polls while it's read, saves field changes and runs done / reopen / delete. */
export const LetterPage: React.FC<{ caseId: string; onBack: () => void; onDeleted: (title: string) => void }> = ({ caseId, onBack, onDeleted }) => {
  const [view, setView] = useState<LetterViewData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ text: string; calm: boolean } | null>(null);
  const [acting, setActing] = useState(false);
  const alive = useRef(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const v = await letterApi.get(caseId);
      if (alive.current) (setView(v), setError(null));
      return v;
    } catch (e) {
      if (alive.current) setError(e instanceof Error ? e.message : 'Something went wrong.');
      return null;
    } finally {
      if (alive.current) setLoading(false);
    }
  }, [caseId]);

  useEffect(() => {
    alive.current = true;
    load();
    return () => {
      alive.current = false;
    };
  }, [load]);

  const reading = stillReading(view);
  const wasReading = useRef(false);
  useEffect(() => {
    if (!reading) {
      // The server stores what was read a moment before it updates the case (title, draft):
      // one more load once reading ends picks those up.
      if (!wasReading.current) return;
      wasReading.current = false;
      const t = window.setTimeout(() => load(), 1000);
      return () => window.clearTimeout(t);
    }
    wasReading.current = true;
    const t = window.setInterval(() => load(), 2000);
    return () => window.clearInterval(t);
  }, [reading, load]);

  const save: Save = async (body) => {
    try {
      const v = await letterApi.patch(caseId, body);
      if (alive.current) (setView(v), setNotice(null));
      return null;
    } catch (e) {
      if (e instanceof LooseEndsError && e.status === 409) {
        load();
        return 'This letter is still being read. Try again once it has finished.';
      }
      return `${e instanceof Error ? e.message : 'Something went wrong.'} Nothing was changed.`;
    }
  };

  const act = async (a: 'done' | 'reopen') => {
    setActing(true);
    setNotice(null);
    try {
      await looseApi.act(caseId, a);
      await load();
    } catch (e) {
      setNotice({ text: `${e instanceof Error ? e.message : 'Something went wrong.'} The letter is unchanged.`, calm: false });
    } finally {
      setActing(false);
    }
  };

  const remove = async () => {
    setActing(true);
    setNotice(null);
    try {
      await letterApi.remove(caseId);
      onDeleted(view?.case.title ?? 'Letter');
    } catch (e) {
      if (alive.current) setNotice(deleteErrorText(e));
    } finally {
      if (alive.current) setActing(false);
    }
  };

  return (
    <LetterDetail
      view={view}
      loading={loading}
      error={error}
      notice={notice}
      acting={acting}
      onSave={save}
      onDone={() => act('done')}
      onReopen={() => act('reopen')}
      onDelete={remove}
      onBack={onBack}
      onRetry={load}
    />
  );
};
