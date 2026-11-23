// Pocket apps (pa4c): Edit questions, under the quiz card (desktop) or full screen (phone). pa0a/design.md §7 and
// edit-1440.html / edit-390.html, on Juno's four person-only draft routes (pa4c-server/impl.md "Draft edit API").
// Every edit sends `base` (the draft's `updated_at` as shown); a stale one gets "Reload" and keeps the typed text,
// never a silent overwrite. 422 shows the server's numbered problems under the edited question. An open or closed
// quiz is read only. Chips are plain: "Checked" (ok), "Yours, not checked", "Not checked" (warn); no holo here.
// Not in this item: the ask bar and its chips, "Try it as a player", picture Replace (no route).

import React, { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, BookOpen, Check, ChevronLeft, Loader2, Pencil, Plus, RotateCw, Trash2, X } from 'lucide-react';
import { pocketApi, PocketApiError } from '../../pocketApi';
import type { DraftCard, DraftQuestion, DraftQuestionKind, QuestionFields, QuizDraft } from '../../pocketApi';

/** The server's words where it has them; these are for the cases decided here. */
export const EDIT_TEXT = {
  changed: 'The Quizmaster changed this quiz. Reload to see the new version.',
  open: "An open quiz's questions can't change.",
  closed: "A closed quiz's questions can't change.",
  saving: 'Saving re-runs the dry run (about 30 s).',
  yours: "Your own questions aren't source-checked. Quizmaster still runs them in the dry run.",
  hint: 'Pick a round, then a question to change it by hand.',
} as const;

const KIND_LABEL: Record<DraftQuestionKind, string> = { text: 'Typed answer', number: 'Number', choice: 'Choice' };

type Target = { mode: 'edit'; n: number } | { mode: 'add'; round: number };
interface Form {
  kind: DraftQuestionKind;
  question: string;
  answer: string;
  accepted: string[];
  options: string[];
}
type Note = { kind: 'error'; text: string; problems: string[] } | { kind: 'ok'; text: string } | null;

const formOf = (q: DraftQuestion): Form => ({ kind: q.kind, question: q.question, answer: String(q.answer ?? ''), accepted: [...(q.accepted ?? [])], options: [...(q.options ?? [])] });
const blankForm = (): Form => ({ kind: 'text', question: '', answer: '', accepted: [], options: [] });
const same = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** The PATCH body: only what changed (impl.md "only what's sent changes"). Null when nothing did. */
export function editBody(q: DraftQuestion, f: Form): QuestionFields | null {
  const o = formOf(q);
  const body: QuestionFields = {};
  if (f.question.trim() !== o.question.trim()) body.question = f.question.trim();
  if (f.answer.trim() !== o.answer.trim()) body.answer = f.answer.trim();
  if (!same(f.accepted, o.accepted)) body.accepted = f.accepted;
  if (q.kind === 'choice' && !same(f.options, o.options)) body.options = f.options;
  return Object.keys(body).length ? body : null;
}

/** A row's "where it came from" line. */
function sourceLine(q: DraftQuestion): string {
  if (q.status === 'yours') return 'Your own question, not checked';
  if (q.status === 'not_checked') return q.why ? `Not checked: ${q.why}` : 'Not checked against a source';
  if (!q.source_url) return 'Checked against a source';
  try {
    return new URL(q.source_url).hostname.replace(/^www\./, '');
  } catch {
    return 'Checked against a source';
  }
}

export const StatusChip: React.FC<{ q: Pick<DraftQuestion, 'status'> }> = ({ q }) =>
  q.status === 'checked' ? (
    <span className="kw-chip kw-chip-ok whitespace-nowrap" data-status="checked">
      <Check size={12} aria-hidden="true" />
      Checked
    </span>
  ) : q.status === 'yours' ? (
    <span className="kw-chip whitespace-nowrap text-zinc-100" data-status="yours">
      Yours, not checked
    </span>
  ) : (
    <span className="kw-chip kw-chip-warn whitespace-nowrap" data-status="not_checked">
      <AlertTriangle size={12} aria-hidden="true" />
      Not checked
    </span>
  );

// ---------------------------------------------------------------------------------------------------
// Fields

const FIELD =
  'w-full rounded-xl border border-hairline-strong bg-surface-rail px-3 text-[16px] text-zinc-50 placeholder:text-zinc-500 focus:border-sky-400 focus:outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-sky-400 disabled:opacity-60 sm:text-[14px]';
const FIELD_H = 'min-h-[44px] py-2.5 sm:min-h-[38px] sm:py-2';
const LABEL = 'mb-1 block text-[13px] font-semibold text-zinc-300 sm:text-[12px]';
const BTN = 'min-h-[44px] sm:min-h-[36px] whitespace-nowrap text-[15px] sm:text-meta';
const PRIMARY = `kw-btn-primary kw-hover-deepen ${BTN}`;
const GHOST = `kw-btn-ghost ${BTN}`;
const STOP = `inline-flex items-center justify-center gap-1.5 rounded-xl px-3 font-medium text-bad hover:bg-rose-300/10 kw-focus ${BTN}`;

/** "Also accept" and "Options": tags with ×, typed text added on Enter or comma, Backspace on empty drops the last. */
const TagInput: React.FC<{ id: string; label: string; values: string[]; onChange: (v: string[]) => void; placeholder: string; disabled?: boolean }> = ({
  id,
  label,
  values,
  onChange,
  placeholder,
  disabled,
}) => {
  const [text, setText] = useState('');
  const add = () => {
    const t = text.trim();
    if (t && !values.includes(t)) onChange([...values, t]);
    setText('');
  };
  return (
    <div>
      <label htmlFor={id} className={LABEL}>
        {label}
      </label>
      <div className={`${FIELD} ${FIELD_H} flex flex-wrap items-center gap-1.5 focus-within:border-sky-400`} data-tags={id}>
        {values.map((v) => (
          <span key={v} className="inline-flex items-center gap-1 rounded-lg border border-sky-700 bg-sky-950 py-0.5 pl-2 pr-0.5 text-[14px] text-zinc-100 sm:text-[13px]">
            {v}
            <button
              type="button"
              disabled={disabled}
              onClick={() => onChange(values.filter((x) => x !== v))}
              aria-label={`Remove ${v}`}
              className="kw-focus inline-flex h-7 w-7 items-center justify-center rounded-md text-zinc-400 hover:text-zinc-100 sm:h-5 sm:w-5"
            >
              <X size={12} aria-hidden="true" />
            </button>
          </span>
        ))}
        <input
          id={id}
          value={text}
          disabled={disabled}
          placeholder={placeholder}
          onChange={(e) => {
            const v = e.target.value;
            if (v.endsWith(',')) {
              setText(v.slice(0, -1));
              const t = v.slice(0, -1).trim();
              if (t && !values.includes(t)) onChange([...values, t]);
              setText('');
            } else setText(v);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            } else if (e.key === 'Backspace' && !text && values.length) onChange(values.slice(0, -1));
          }}
          onBlur={add}
          className="min-w-[8rem] flex-1 bg-transparent py-1 text-zinc-50 placeholder:text-zinc-500 focus:outline-none"
        />
      </div>
    </div>
  );
};

/** The edit form, inline on desktop and as the phone's full-screen editor. */
const EditForm: React.FC<{
  target: Target;
  q: DraftQuestion | null;
  form: Form;
  setForm: (f: Form) => void;
  busy: boolean;
  note: Note;
  phone: boolean;
  onSave: () => void;
  onCancel: () => void;
  onDelete: () => void;
}> = ({ target, q, form, setForm, busy, note, phone, onSave, onCancel, onDelete }) => {
  const id = useId();
  const [confirm, setConfirm] = useState(false);
  const first = useRef<HTMLTextAreaElement | null>(null);
  useEffect(() => {
    first.current?.focus();
  }, []);
  const set = (k: keyof Form) => (v: string | string[]) => setForm({ ...form, [k]: v });
  const yours = target.mode === 'add' || q?.status === 'yours';
  return (
    <form
      className="flex flex-col gap-3"
      data-edit-form={target.mode === 'edit' ? target.n : `add-${target.round}`}
      aria-busy={busy || undefined}
      onSubmit={(e) => {
        e.preventDefault();
        if (!busy) onSave();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !busy) {
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      {target.mode === 'add' && (
        <fieldset>
          <legend className={LABEL}>Kind</legend>
          <div className="flex flex-wrap gap-2" role="radiogroup">
            {(['text', 'number', 'choice'] as DraftQuestionKind[]).map((k) => (
              <label key={k} className={`kw-focus inline-flex min-h-[44px] cursor-pointer items-center gap-2 rounded-xl border px-3 text-[14px] sm:min-h-[34px] sm:text-[13px] ${form.kind === k ? 'border-sky-400 bg-sky-500/15 text-zinc-50' : 'border-hairline-strong text-zinc-300'}`}>
                <input type="radio" name={`${id}-kind`} className="accent-sky-500" checked={form.kind === k} disabled={busy} onChange={() => setForm({ ...form, kind: k })} />
                {KIND_LABEL[k]}
              </label>
            ))}
          </div>
        </fieldset>
      )}
      <div>
        <label htmlFor={`${id}-q`} className={LABEL}>
          Question
        </label>
        <textarea
          id={`${id}-q`}
          ref={first}
          rows={2}
          value={form.question}
          disabled={busy}
          onChange={(e) => set('question')(e.target.value)}
          className={`${FIELD} ${FIELD_H} resize-y leading-snug`}
        />
      </div>
      {(form.kind === 'choice' || (q?.options?.length ?? 0) > 0) && (
        <TagInput id={`${id}-o`} label="Options" values={form.options} onChange={set('options')} placeholder="Add an option…" disabled={busy} />
      )}
      <div>
        <label htmlFor={`${id}-a`} className={LABEL}>
          Answer
        </label>
        <input
          id={`${id}-a`}
          value={form.answer}
          disabled={busy}
          inputMode={form.kind === 'number' ? 'decimal' : undefined}
          onChange={(e) => set('answer')(e.target.value)}
          className={`${FIELD} ${FIELD_H}`}
        />
      </div>
      {form.kind !== 'choice' && <TagInput id={`${id}-acc`} label="Also accept" values={form.accepted} onChange={set('accepted')} placeholder="Add a spelling…" disabled={busy} />}
      {yours && phone && (
        <div className="rounded-xl border border-hairline-strong px-3.5 py-3 text-[15px] leading-[22px] text-zinc-300" data-yours-note>
          <span className="kw-chip mb-1.5 text-zinc-100">Yours, not checked</span>
          <p>{EDIT_TEXT.yours}</p>
        </div>
      )}
      {note?.kind === 'error' && (
        <div role="alert" className="rounded-xl border border-rose-300/35 bg-rose-300/[0.07] px-3.5 py-3 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5" data-problems={note.problems.length}>
          <p className="flex items-start gap-2 font-semibold text-bad">
            <AlertTriangle size={15} className="mt-[3px] shrink-0 sm:mt-0.5" aria-hidden="true" />
            {note.text}
          </p>
          {note.problems.length > 0 && (
            <ul className="mt-1.5 flex flex-col gap-1 pl-6">
              {note.problems.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ul>
          )}
        </div>
      )}
      {confirm && q ? (
        <div className="flex flex-col gap-2 rounded-xl border border-rose-300/35 bg-rose-300/[0.07] p-3 sm:flex-row sm:items-center" role="group" aria-label={`Delete Q${q.n}`}>
          <p className="flex-1 text-[15px] text-zinc-200 sm:text-[13px]">Delete Q{q.n}? The questions after it move up one. {EDIT_TEXT.saving}</p>
          <button type="button" disabled={busy} onClick={onDelete} className={`${STOP} border border-rose-300/40`} data-action="delete-yes">
            {busy ? <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> : <Trash2 size={14} aria-hidden="true" />}
            Delete
          </button>
          <button type="button" disabled={busy} onClick={() => setConfirm(false)} className={GHOST}>
            Keep it
          </button>
        </div>
      ) : (
        <div className={phone ? 'flex flex-col gap-3' : 'flex flex-wrap items-center gap-2'}>
          <button type="submit" disabled={busy} className={`${PRIMARY} ${phone ? 'w-full' : ''}`} data-action="save">
            {busy ? <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
            {busy ? 'Saving…' : 'Save'}
          </button>
          {!phone && (
            <button type="button" disabled={busy} onClick={onCancel} className={GHOST} data-action="cancel">
              Cancel
            </button>
          )}
          <span className={`text-[13px] text-zinc-400 sm:text-[12px] ${phone ? 'text-center' : ''}`}>{EDIT_TEXT.saving}</span>
          {q && (
            <button type="button" disabled={busy} onClick={() => setConfirm(true)} className={`${STOP} ${phone ? 'self-center' : 'sm:ml-auto'}`} data-action="delete">
              <Trash2 size={14} aria-hidden="true" />
              Delete question
            </button>
          )}
        </div>
      )}
    </form>
  );
};

// ---------------------------------------------------------------------------------------------------
// Rows

const QuestionRow: React.FC<{ q: DraftQuestion; changed: boolean; readOnly: boolean; phone: boolean; disabled: boolean; onEdit: () => void }> = ({ q, changed, readOnly, phone, disabled, onEdit }) => {
  const body = (
    <>
      <span className="pt-0.5 font-mono text-[13px] font-semibold text-zinc-400 sm:text-[12px]">Q{q.n}</span>
      <span className="min-w-0">
        <span className="block break-words text-[16px] font-medium leading-[23px] text-zinc-50 sm:text-[14px] sm:leading-[21px]">{q.question}</span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[15px] text-zinc-400 sm:text-[13px]">
          <span>
            Answer <strong className="font-semibold text-ok">{String(q.answer)}</strong>
          </span>
          {q.accepted.length > 0 && (
            <>
              <span>· also</span>
              {q.accepted.map((a) => (
                <span key={a} className="rounded-md border border-hairline-strong px-1.5 text-[13px] text-zinc-200 sm:text-[12px]">
                  {a}
                </span>
              ))}
            </>
          )}
        </span>
        <span className={`mt-0.5 flex items-start gap-1.5 text-[14px] sm:text-[12px] ${q.status === 'not_checked' ? 'text-warn' : 'text-zinc-400'}`} data-source>
          <BookOpen size={12} className="mt-[3px] shrink-0 sm:mt-0.5" aria-hidden="true" />
          <span className="min-w-0 break-words">{sourceLine(q)}</span>
        </span>
      </span>
    </>
  );
  const bar = changed ? 'before:absolute before:inset-y-0 before:left-0 before:w-[3px] before:rounded-full before:bg-sky-400' : '';
  if (phone)
    return (
      <li className={`relative border-b border-hairline last:border-b-0 ${bar}`} data-q={q.n} data-changed={changed || undefined}>
        <button
          type="button"
          disabled={readOnly || disabled}
          onClick={onEdit}
          aria-label={readOnly ? undefined : `Edit Q${q.n}: ${q.question}`}
          className="kw-focus grid w-full grid-cols-[40px_minmax(0,1fr)] gap-x-2 px-4 py-4 text-left disabled:cursor-default"
        >
          {body}
          <span className="col-start-2 mt-2 flex items-center gap-2">
            <StatusChip q={q} />
            {!readOnly && <Pencil size={16} className="ml-auto text-sky-300" aria-hidden="true" />}
          </span>
        </button>
      </li>
    );
  return (
    <li className={`relative grid grid-cols-[44px_minmax(0,1fr)_auto] gap-x-2 border-b border-hairline py-3 pl-3 last:border-b-0 ${bar}`} data-q={q.n} data-changed={changed || undefined}>
      {body}
      <span className="flex items-start gap-1.5">
        <StatusChip q={q} />
        {!readOnly && (
          <button type="button" disabled={disabled} onClick={onEdit} aria-label={`Edit Q${q.n}`} className="kw-focus inline-flex h-8 w-8 items-center justify-center rounded-lg text-sky-300 hover:bg-white/[0.05] disabled:opacity-50" data-edit={q.n}>
            <Pencil size={15} aria-hidden="true" />
          </button>
        )}
      </span>
    </li>
  );
};

// ---------------------------------------------------------------------------------------------------
// The panel

export interface EditPanelProps {
  draftId: string;
  /** The card's quiz name, until the draft says its own. */
  name: string;
  phone: boolean;
  /** Read only before loading: the card is live or ended ("See questions"). */
  readOnly?: boolean;
  /** Swap the reply's card into the pocket card, the way save-and-show does. */
  onCard: (card: DraftCard) => void;
  /** The switch is off: the card drops its Edit button. */
  onOff: (text: string) => void;
  onClose: () => void;
}

export const EditPanel: React.FC<EditPanelProps> = ({ draftId, name, phone, readOnly: roCard, onCard, onOff, onClose }) => {
  const [draft, setDraft] = useState<QuizDraft | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [round, setRound] = useState(1);
  const [target, setTarget] = useState<Target | null>(null);
  const [form, setForm] = useState<Form>(blankForm);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<Note>(null);
  const [stale, setStale] = useState(false);
  const [ro, setRo] = useState<string | null>(null);
  const [changed, setChanged] = useState<Set<number>>(() => new Set());
  const [reloading, setReloading] = useState(false);
  const opener = useRef<string | null>(null);
  const root = useRef<HTMLDivElement | null>(null);
  const titleId = useId();

  const roFromDraft = (d: QuizDraft) => (d.state === 'open' ? EDIT_TEXT.open : d.state === 'closed' ? EDIT_TEXT.closed : null);

  const load = async (keepTarget: boolean) => {
    try {
      const r = await pocketApi.getDraft(draftId);
      setDraft(r.draft);
      setLoadErr(null);
      setRo(roFromDraft(r.draft));
      setRound((n) => (r.draft.rounds.some((x) => x.n === n) ? n : r.draft.rounds[0]?.n ?? 1));
      if (keepTarget && target?.mode === 'edit' && !r.draft.questions.some((q) => q.n === target.n)) {
        const was = draft?.questions.find((q) => q.n === target.n);
        setTarget({ mode: 'add', round: was?.round ?? round });
      }
      if (!keepTarget) setTarget(null);
      return true;
    } catch (e) {
      if (e instanceof PocketApiError && e.code === 'pocket_apps_off') {
        onOff(e.message);
        return false;
      }
      setLoadErr(e instanceof PocketApiError ? e.message : "Couldn't open the questions. Try again.");
      return false;
    }
  };
  useEffect(() => {
    void load(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftId]);

  // Phone: the editor is the page; the stream behind doesn't scroll.
  useEffect(() => {
    if (!phone) return;
    document.documentElement.classList.add('overflow-hidden');
    return () => document.documentElement.classList.remove('overflow-hidden');
  }, [phone]);

  const readOnly = !!(roCard || ro);
  const roText = ro ?? (roCard ? EDIT_TEXT.open : null);

  const focusLater = (sel: string) => window.setTimeout(() => root.current?.querySelector<HTMLElement>(sel)?.focus(), 0);
  const open = (t: Target) => {
    if (!draft || readOnly || busy) return;
    opener.current = t.mode === 'edit' ? `[data-q="${t.n}"] button` : `[data-action="add"]`;
    const q = t.mode === 'edit' ? draft.questions.find((x) => x.n === t.n) ?? null : null;
    setForm(q ? formOf(q) : blankForm());
    setNote(null);
    setTarget(t);
  };
  const close = () => {
    setTarget(null);
    setNote(null);
    if (opener.current) focusLater(opener.current);
  };

  const fail = (e: unknown) => {
    if (!(e instanceof PocketApiError)) return setNote({ kind: 'error', text: 'Something went wrong. Try again.', problems: [] });
    switch (e.code) {
      case 'changed':
        setStale(true);
        return focusLater('[data-action="reload"]');
      case 'quiz_open':
      case 'quiz_closed':
      case 'still_writing':
        setRo(e.message);
        setTarget(null);
        return;
      case 'pocket_apps_off':
        return onOff(e.message);
      default:
        return setNote({ kind: 'error', text: e.message, problems: e.problems });
    }
  };

  const done = (r: { draft: QuizDraft; card: DraftCard }, n: number | null, removed: number | null) => {
    setDraft(r.draft);
    setRo(roFromDraft(r.draft));
    onCard(r.card);
    setChanged((s) => {
      const next = new Set<number>();
      for (const x of s) if (removed === null || x < removed) next.add(x);
      else if (x > removed) next.add(x - 1);
      if (n !== null) next.add(n);
      return next;
    });
    const ok = r.card.dry_run?.ok ?? r.card.state === 'ready';
    setNote({ kind: 'ok', text: ok ? 'Saved. The dry run passed again.' : 'Saved, but the dry run stopped. The card says why.' });
    setTarget(null);
    if (n !== null) focusLater(`[data-q="${n}"] button`);
  };

  const save = async () => {
    if (!draft || !target) return;
    setNote(null);
    if (target.mode === 'edit') {
      const q = draft.questions.find((x) => x.n === target.n);
      if (!q) return;
      const body = editBody(q, form);
      if (!body) return close();
      setBusy(true);
      try {
        done(await pocketApi.editQuestion(draftId, q.n, { ...body, base: draft.updated_at }), q.n, null);
      } catch (e) {
        fail(e);
      } finally {
        setBusy(false);
      }
      return;
    }
    setBusy(true);
    try {
      const r = await pocketApi.addQuestion(draftId, {
        round: target.round,
        kind: form.kind,
        question: form.question.trim(),
        answer: form.answer.trim(),
        ...(form.kind === 'choice' ? { options: form.options } : { accepted: form.accepted }),
        base: draft.updated_at,
      });
      const added = r.draft.questions.filter((x) => x.round === target.round).reduce((m, x) => Math.max(m, x.n), 0);
      done(r, added || null, null);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!draft || target?.mode !== 'edit') return;
    setBusy(true);
    try {
      done(await pocketApi.deleteQuestion(draftId, target.n, draft.updated_at), null, target.n);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const reload = async () => {
    setReloading(true);
    // The typed text stays: the person saves it again against the new version.
    if (await load(true)) setStale(false);
    setReloading(false);
  };

  // ---- pieces

  const title = readOnly ? 'Questions' : 'Edit questions';
  const quizName = draft?.title || name;
  const roundQs = draft ? draft.questions.filter((q) => q.round === round) : [];
  const roundInfo = draft?.rounds.find((r) => r.n === round);
  const roundLabel = (r: { n: number; title: string | null }) => `Round ${r.n}${r.title ? ` · ${r.title}` : ''}`;
  const editingQ = target?.mode === 'edit' ? draft?.questions.find((q) => q.n === target.n) ?? null : null;

  const banners = (
    <>
      {stale && (
        <div role="alert" className="flex flex-col gap-2 rounded-xl border border-amber-300/35 bg-amber-300/[0.07] px-3.5 py-3 text-[15px] text-zinc-200 sm:flex-row sm:items-center sm:text-[13px]" data-stale>
          <span className="flex flex-1 items-start gap-2">
            <AlertTriangle size={15} className="mt-[3px] shrink-0 text-warn sm:mt-0.5" aria-hidden="true" />
            {EDIT_TEXT.changed}
          </span>
          <button type="button" onClick={() => void reload()} disabled={reloading} className={GHOST} data-action="reload">
            {reloading ? <Loader2 size={14} className="motion-safe:animate-spin" aria-hidden="true" /> : <RotateCw size={14} aria-hidden="true" />}
            Reload
          </button>
        </div>
      )}
      {roText && (
        <p className="flex items-start gap-2 rounded-xl border border-hairline bg-surface-rail/60 px-3.5 py-3 text-[15px] text-zinc-300 sm:text-[13px]" data-read-only>
          {roText} You can still read them here.
        </p>
      )}
      {note?.kind === 'ok' && !target && (
        <p role="status" className="flex items-start gap-2 text-[15px] text-ok sm:text-[13px]" data-saved>
          <Check size={15} className="mt-[3px] shrink-0 sm:mt-0.5" aria-hidden="true" />
          {note.text}
        </p>
      )}
    </>
  );

  const addButton = !readOnly && draft && (
    <button type="button" onClick={() => open({ mode: 'add', round })} disabled={busy || !!target} className={`${GHOST} ${phone ? 'm-4 w-[calc(100%-2rem)]' : 'mt-2 self-start'}`} data-action="add">
      <Plus size={14} aria-hidden="true" />
      Add question to round {round}
    </button>
  );

  const formFor = (t: Target) => (
    <EditForm target={t} q={editingQ} form={form} setForm={setForm} busy={busy} note={note} phone={phone} onSave={() => void save()} onCancel={close} onDelete={() => void remove()} />
  );

  const body = !draft ? (
    <div className="flex flex-col items-start gap-3 p-4">
      {loadErr ? (
        <>
          <p role="alert" className="flex items-start gap-2 text-[15px] text-bad sm:text-[13px]">
            <AlertTriangle size={15} className="mt-[3px] shrink-0 sm:mt-0.5" aria-hidden="true" />
            {loadErr}
          </p>
          <button type="button" className={GHOST} onClick={() => void load(false)}>
            <RotateCw size={14} aria-hidden="true" />
            Try again
          </button>
        </>
      ) : (
        <p className="flex items-center gap-2 text-[15px] text-zinc-300 sm:text-[13px]" role="status">
          <Loader2 size={14} className="motion-safe:animate-spin text-sky-300" aria-hidden="true" />
          Opening the questions…
        </p>
      )}
    </div>
  ) : null;

  // ---- phone: the list, or one question as its own screen

  if (phone) {
    const screen =
      target && draft ? (
        <>
          <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-hairline bg-surface-main px-2 py-2">
            <button type="button" onClick={close} disabled={busy} aria-label="Back to the questions" className="kw-focus inline-flex h-11 w-11 items-center justify-center rounded-xl text-zinc-300">
              <ChevronLeft size={22} aria-hidden="true" />
            </button>
            <div className="min-w-0">
              <h2 id={titleId} className="text-[20px] font-bold leading-[26px] text-zinc-50">
                {target.mode === 'edit' ? `Q${target.n} · Round ${editingQ?.round ?? round}` : `New question · Round ${target.round}`}
              </h2>
              {(target.mode === 'edit' ? draft.rounds.find((r) => r.n === editingQ?.round) : roundInfo)?.title && (
                <p className="text-[15px] text-zinc-400">{(target.mode === 'edit' ? draft.rounds.find((r) => r.n === editingQ?.round) : roundInfo)?.title}</p>
              )}
            </div>
          </header>
          <div className="flex flex-col gap-3 p-4">
            {stale && banners}
            {formFor(target)}
          </div>
        </>
      ) : (
        <>
          <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-hairline bg-surface-main px-2 py-2">
            <button type="button" onClick={onClose} aria-label="Close" className="kw-focus inline-flex h-11 w-11 items-center justify-center rounded-xl text-zinc-300">
              <ChevronLeft size={22} aria-hidden="true" />
            </button>
            <div className="min-w-0 flex-1">
              <h2 id={titleId} className="text-[20px] font-bold leading-[26px] text-zinc-50">
                {title}
              </h2>
              <p className="truncate text-[15px] text-zinc-400" data-header>
                {draft?.header ?? quizName}
              </p>
            </div>
            <button type="button" onClick={onClose} className={`${PRIMARY} px-5`} data-action="done">
              Done
            </button>
          </header>
          {body ?? (
            <div className="flex flex-col gap-3 p-4">
              {draft && draft.rounds.length > 1 && (
                <div role="tablist" aria-label="Rounds" className="grid auto-cols-fr grid-flow-col gap-1 rounded-2xl border border-hairline bg-surface-raised p-1">
                  {draft.rounds.map((r) => (
                    <button
                      key={r.n}
                      type="button"
                      role="tab"
                      aria-selected={r.n === round}
                      aria-label={roundLabel(r)}
                      onClick={() => setRound(r.n)}
                      className={`kw-focus min-h-[44px] rounded-xl text-[16px] font-semibold ${r.n === round ? 'bg-sky-600 text-white' : 'text-zinc-300'}`}
                    >
                      R{r.n}
                    </button>
                  ))}
                </div>
              )}
              {banners}
              <section aria-label={roundInfo ? roundLabel(roundInfo) : 'Questions'} className="overflow-hidden rounded-2xl border border-hairline bg-surface-raised">
                {roundQs.length ? (
                  <ol>
                    {roundQs.map((q) => (
                      <QuestionRow key={q.n} q={q} changed={changed.has(q.n)} readOnly={readOnly} phone disabled={busy} onEdit={() => open({ mode: 'edit', n: q.n })} />
                    ))}
                  </ol>
                ) : (
                  <p className="p-4 text-[15px] text-zinc-400">No questions in this round yet.</p>
                )}
                {addButton}
              </section>
            </div>
          )}
        </>
      );
    return createPortal(
      <div
        ref={root}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="fixed inset-0 z-50 overflow-y-auto overscroll-contain bg-surface-main pb-[env(safe-area-inset-bottom)] pt-[env(safe-area-inset-top)]"
        data-pocket-edit="phone"
        data-read-only={readOnly || undefined}
        onKeyDown={(e) => {
          if (e.key === 'Escape' && !busy) (target ? close : onClose)();
        }}
      >
        {screen}
      </div>,
      document.body,
    );
  }

  // ---- desktop: in place under the card

  return (
    <section
      ref={root}
      aria-labelledby={titleId}
      className="pocket-edit max-w-[1000px] overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised shadow-card"
      data-pocket-edit="desktop"
      data-read-only={readOnly || undefined}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && !target && !busy) onClose();
      }}
    >
      <header className="flex items-center gap-3 border-b border-hairline px-[18px] py-3.5">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950 text-sky-200" aria-hidden="true">
          <Pencil size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <h3 id={titleId} className="truncate text-[16px] font-bold leading-[22px] text-zinc-50">
            {title} · {quizName}
          </h3>
          <p className="text-[12px] text-zinc-400" data-header>
            {draft?.header ?? ' '}
          </p>
        </div>
        <button type="button" onClick={onClose} className={`${PRIMARY} px-4`} data-action="done">
          Done
        </button>
      </header>
      {body ?? (
        <div className="grid grid-cols-[220px_minmax(0,1fr)]">
          <nav aria-label="Rounds" className="flex flex-col gap-1 border-r border-hairline p-2.5">
            {draft!.rounds.map((r) => {
              const count = draft!.questions.filter((q) => q.round === r.n).length;
              return (
                <button
                  key={r.n}
                  type="button"
                  aria-current={r.n === round ? 'true' : undefined}
                  onClick={() => setRound(r.n)}
                  className={`kw-focus flex min-h-[38px] items-center justify-between gap-2 rounded-xl border px-3 text-left text-[14px] ${
                    r.n === round ? 'border-sky-700 bg-sky-500/15 text-zinc-50' : 'border-transparent text-zinc-300 hover:bg-white/[0.04]'
                  }`}
                >
                  <span className="min-w-0 truncate">{roundLabel(r)}</span>
                  <span className="shrink-0 text-[12px] tabular-nums text-zinc-400">{count}</span>
                </button>
              );
            })}
            <p className="mt-3 px-3 text-[12px] leading-[18px] text-zinc-400">{readOnly ? 'Pick a round to read its questions.' : EDIT_TEXT.hint}</p>
          </nav>
          <div className="flex min-w-0 flex-col gap-3 p-[18px]">
            {banners}
            <h4 className="text-[13px] font-semibold text-zinc-200">{roundInfo ? roundLabel(roundInfo) : `Round ${round}`}</h4>
            {roundQs.length ? (
              <ol className="-mt-1 flex flex-col">
                {roundQs.map((q) =>
                  target?.mode === 'edit' && target.n === q.n ? (
                    <li key={q.n} data-q={q.n} className="grid grid-cols-[44px_minmax(0,1fr)] gap-x-2 rounded-xl border border-sky-500/60 bg-sky-500/[0.04] p-3 pl-3">
                      <span className="pt-0.5 font-mono text-[12px] font-semibold text-zinc-400">Q{q.n}</span>
                      {formFor(target)}
                    </li>
                  ) : (
                    <QuestionRow key={q.n} q={q} changed={changed.has(q.n)} readOnly={readOnly} phone={false} disabled={busy || !!target} onEdit={() => open({ mode: 'edit', n: q.n })} />
                  ),
                )}
              </ol>
            ) : (
              <p className="text-[13px] text-zinc-400">No questions in this round yet.</p>
            )}
            {target?.mode === 'add' ? (
              <div className="grid grid-cols-[44px_minmax(0,1fr)] gap-x-2 rounded-xl border border-sky-500/60 bg-sky-500/[0.04] p-3" data-q="new">
                <span className="pt-0.5 font-mono text-[12px] font-semibold text-zinc-400">New</span>
                <div className="flex flex-col gap-2">
                  {formFor(target)}
                  <p className="text-[12px] text-zinc-400">{EDIT_TEXT.yours}</p>
                </div>
              </div>
            ) : (
              addButton
            )}
          </div>
        </div>
      )}
    </section>
  );
};
