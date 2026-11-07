// Show it once (0.7.9, design §S): the start dialog, the Record steps button, the red Recording
// bar on the Take over stage and the Check the steps review. Everything shown comes from the
// session in showOnce.ts, which holds what Vega's API sent; nothing here counts or renumbers.
import React, { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  Check,
  CircleDot,
  Clock,
  CornerDownLeft,
  Globe,
  Keyboard,
  ListChecks,
  ListTodo,
  Loader2,
  Lock,
  MousePointer2,
  MoreHorizontal,
  Pencil,
  RefreshCw,
  Square,
  StickyNote,
  Trash2,
  TriangleAlert,
  Undo2,
  Upload,
  X,
  Hand,
} from 'lucide-react';
import { useEscapeLayer } from '../escape';
import {
  Part,
  RecordedStep,
  ShowOnceSession,
  clock,
  elapsedNow,
  engineName,
  limitsOf,
  nearLimit,
  reviewSub,
  showOnce,
  stepKey,
  stepParts,
  stepSentence,
  stepsWord,
  stopBanner,
  lockNote,
  writeFailedTail,
  dayOf,
  useShowOnce,
  writeBlocker,
} from '../showOnce';

// ---------------------------------------------------------------------------------------------
// Small parts

/** The record glyph: a ringed dot, in recording red. */
export const RecIcon: React.FC<{ size?: number }> = ({ size = 14 }) => <CircleDot size={size} className="so-rec-ic" aria-hidden="true" />;

/** A kept-as chip. The screen reader hears what it means, not the brackets. */
export const MaskChip: React.FC<{ text: string; label: string }> = ({ text, label }) => (
  <span className="so-mask" role="img" aria-label={label}>
    <Lock size={11} aria-hidden="true" />
    <span aria-hidden="true">{text}</span>
  </span>
);

/** One step's words. Text only: values from the page are never read as HTML. */
export const Parts: React.FC<{ parts: Part[] }> = ({ parts }) => (
  <>
    {parts.map((p, i) => {
      switch (p.t) {
        case 'text':
          return <React.Fragment key={i}>{p.v}</React.Fragment>;
        case 'q':
          return (
            <span key={i} className="so-q">
              “{p.v}”
            </span>
          );
        case 'url':
          return (
            <span key={i} className="so-url">
              {p.host}
              <span className="so-url-path">{p.path}</span>
            </span>
          );
        case 'mask':
          return <MaskChip key={i} text={p.text} label={p.label} />;
        case 'note':
          return (
            <React.Fragment key={i}>
              <span className="so-note-lbl">Note</span>
              {p.v}
            </React.Fragment>
          );
      }
    })}
  </>
);

const KIND_LABEL: Record<RecordedStep['kind'], string> = {
  page: 'Page',
  click: 'Click',
  type: 'Typing',
  select: 'Choice',
  key: 'Key',
  upload: 'Upload',
  note: 'Note',
};

export const StepIcon: React.FC<{ step: RecordedStep; size?: number }> = ({ step, size = 13 }) => {
  const p = { size, 'aria-hidden': true as const };
  if (step.masked) return <Lock {...p} />;
  switch (step.kind) {
    case 'page':
      return <Globe {...p} />;
    case 'click':
      return <MousePointer2 {...p} />;
    case 'type':
      return <Keyboard {...p} />;
    case 'select':
      return <ListChecks {...p} />;
    case 'key':
      return <CornerDownLeft {...p} />;
    case 'upload':
      return <Upload {...p} />;
    case 'note':
      return <StickyNote {...p} />;
  }
};

// ---------------------------------------------------------------------------------------------
// A26: Start recording, from the Skills tab

export const ShowOnceStart: React.FC<{
  botName: string;
  open: boolean;
  /** The page the bot's browser is on, when known */
  defaultUrl?: string;
  onCancel: () => void;
  onStart: (startUrl: string | null, firstNote: string | null) => void;
}> = ({ botName, open, defaultUrl = '', onCancel, onStart }) => {
  const rootRef = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState(defaultUrl);
  const [what, setWhat] = useState('');
  const id = useId();
  useEffect(() => {
    if (open) {
      setUrl(defaultUrl);
      setWhat('');
    }
  }, [open, defaultUrl]);
  useEscapeLayer(rootRef, open, onCancel, { modal: true });
  if (!open) return null;
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    onStart(url.trim() || null, what.trim() || null);
  };
  return (
    <div className="so-dialog-wrap fixed inset-0 z-[80] flex items-center justify-center p-4 bg-black/60" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div ref={rootRef} tabIndex={-1} className="so-start" role="dialog" aria-modal="true" aria-labelledby={`${id}-h`} aria-describedby={`${id}-d`}>
        <form onSubmit={submit}>
          <h3 id={`${id}-h`}>Show {botName} once</h3>
          <p id={`${id}-d`}>
            Do the task yourself in {botName}'s browser. {botName} writes it down as a skill, and you check it before it's used.
          </p>
          <label className="so-lbl" htmlFor={`${id}-url`}>
            Start at
          </label>
          <div className="so-urlwrap">
            <Globe size={14} aria-hidden="true" />
            <input
              id={`${id}-url`}
              className="so-input"
              type="text"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              placeholder="The page the browser is on"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </div>
          <label className="so-lbl" htmlFor={`${id}-what`}>
            What are you showing? <span className="so-lbl-opt">Optional</span>
          </label>
          <input id={`${id}-what`} className="so-input" type="text" autoFocus value={what} maxLength={500} onChange={(e) => setWhat(e.target.value)} />
          <ul className="so-kept">
            <li>
              <Check size={14} aria-hidden="true" />
              <span>
                <b>Recorded:</b> pages, clicks, what you type and pick, that you uploaded a file (never its name), your notes.
              </span>
            </li>
            <li>
              <Lock size={14} aria-hidden="true" />
              <span>
                <b>Never kept:</b> passwords, codes and card details, which become <MaskChip text="[your password…]" label="password not kept" /> and{' '}
                <MaskChip text="[code]" label="code not kept" />. No screenshots.
              </span>
            </li>
          </ul>
          <p className="so-small">{botName} waits while you record. Up to 200 steps or 30 minutes.</p>
          <div className="so-row">
            <button type="button" className="so-btn so-btn-ghost" onClick={onCancel}>
              Cancel
            </button>
            <button type="submit" className="so-btn so-btn-primary">
              <CircleDot size={14} aria-hidden="true" />
              Start recording
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------
// The stage's Record steps button (S1, desktop stage and phone overlay)

export const RecordStepsButton: React.FC<{ className: string; onClick: () => void; disabled?: boolean }> = ({ className, onClick, disabled }) => (
  <button type="button" className={`${className} so-record-steps`} onClick={onClick} disabled={disabled}>
    <CircleDot size={14} aria-hidden="true" />
    Record steps
  </button>
);

// ---------------------------------------------------------------------------------------------
// The Recording bar (A28, A34) and the note popover (A29)

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent || '');
const MOD = isMac ? '⌘' : 'Ctrl';

function NotePopover({ botName, onClose, phone }: { botName: string; onClose: () => void; phone: boolean }) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => ref.current?.focus(), []);
  const save = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    setError(null);
    const err = await showOnce.note(t);
    setBusy(false);
    if (err) setError(err);
    else onClose();
  };
  return (
    <div
      className="so-pop"
      role="dialog"
      aria-label="Add a note"
      onKeyDown={(e) => {
        // The stage around the bar takes Escape for itself, so the popover answers first
        if (e.key === 'Escape') {
          e.stopPropagation();
          e.preventDefault();
          onClose();
        } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          void save();
        }
      }}
    >
      <label className="so-lbl" htmlFor={`${id}-t`}>
        Note for {botName}
      </label>
      <textarea id={`${id}-t`} ref={ref} className="so-input" rows={phone ? 3 : 2} maxLength={1000} value={text} onChange={(e) => setText(e.target.value)} />
      <p className="so-hint">Saved as the next step, as you wrote it. Esc closes.</p>
      {error && (
        <p className="so-err" role="alert">
          {error}
        </p>
      )}
      <div className="so-pop-row">
        <button type="button" className="so-btn so-btn-ghost" onClick={onClose}>
          Cancel
        </button>
        <button type="button" className="so-btn so-btn-primary" disabled={!text.trim() || busy} onClick={save}>
          {busy ? <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <StickyNote size={14} aria-hidden="true" />}
          Add note
        </button>
      </div>
    </div>
  );
}

/** The last step as the bar shows it: the mask chip short form, so it fits on a phone. */
function LastStep({ step }: { step: RecordedStep | undefined }) {
  if (!step) return <span>Waiting for your first step…</span>;
  return (
    <>
      <StepIcon step={step} />
      <span>
        <Parts parts={stepParts(step, { short: true })} />
      </span>
    </>
  );
}

/**
 * What the stage shows above the page while Show it once runs on it: the start's progress, then
 * the red Recording bar. Nothing once it stops (the review sheet takes over).
 */
export const RecordingStrip: React.FC<{ stage: string; phone?: boolean }> = ({ stage, phone = false }) => {
  const s = useShowOnce();
  const mine = s && s.stage === stage ? s : null;
  if (!mine) return null;
  if (mine.phase === 'taking' || mine.phase === 'starting') return <StartingStrip s={mine} phone={phone} />;
  if (mine.phase === 'recording' && mine.rec) return <RecordingBar s={mine} phone={phone} />;
  return null;
};

function StartingStrip({ s, phone }: { s: ShowOnceSession; phone: boolean }) {
  return (
    <div className={`so-bar ${phone ? 'ph' : ''}`} role="status" aria-live="polite">
      <div className="so-bar-l">
        {s.error ? <TriangleAlert size={14} className="text-amber-300" aria-hidden="true" /> : <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
        <span className="so-bar-title">{s.error ? 'The recording didn’t start.' : s.phase === 'taking' ? 'Taking over the browser…' : 'Starting the recording…'}</span>
      </div>
      {s.error && <span className="so-last">{s.error}</span>}
      {s.error && (
        <div className="so-bar-btns">
          <button type="button" className="so-bar-btn so-bar-note" onClick={() => showOnce.cancel()}>
            Cancel
          </button>
          <button type="button" className="so-bar-btn so-bar-stop" onClick={() => void showOnce.startNow()}>
            <RefreshCw size={14} aria-hidden="true" />
            Try again
          </button>
        </div>
      )}
    </div>
  );
}

function RecordingBar({ s, phone }: { s: ShowOnceSession; phone: boolean }) {
  const rec = s.rec!;
  const [noteOpen, setNoteOpen] = useState(false);
  const [, tick] = useState(0);
  const noteBtn = useRef<HTMLButtonElement>(null);
  const askedAtLimit = useRef(false);
  const limits = limitsOf(rec);
  const elapsed = elapsedNow(s);

  // The clock runs between updates; at the limit, ask the server (it stops the recording itself)
  useEffect(() => {
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, []);
  useEffect(() => {
    if (elapsed >= limits.secs * 1000 && !askedAtLimit.current) {
      askedAtLimit.current = true;
      void showOnce.refresh();
    }
  }, [elapsed, limits.secs]);

  const closeNote = useCallback(() => {
    setNoteOpen(false);
    noteBtn.current?.focus();
  }, []);

  // Ctrl/⌘+Shift+S stops, Ctrl/⌘+Shift+N adds a note: caught before the page under the stage sees them
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || !e.shiftKey || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k !== 's' && k !== 'n') return;
      e.preventDefault();
      e.stopPropagation();
      if (k === 's') void showOnce.stop(false);
      else setNoteOpen(true);
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, []);

  const last = [...s.steps].reverse().find((x) => !x.deleted);
  const near = nearLimit(rec.step_count, elapsed, limits, rec.near);
  const count = (
    <span className="so-count" title={`${rec.step_count} of ${limits.steps} steps · ${clock(elapsed)} of ${clock(limits.secs * 1000)}`}>
      <b>{rec.step_count}</b> {rec.step_count === 1 ? 'step' : 'steps'} · <b>{clock(elapsed)}</b>
    </span>
  );
  const middle = near ? (
    <span className="so-near" role="status">
      <TriangleAlert size={14} aria-hidden="true" />
      <span>{near}</span>
    </span>
  ) : (
    <div className={`so-last ${last?.masked ? 'secret' : ''}`} aria-live="polite" aria-label={last ? `Last step: ${stepSentence(last, { short: true })}` : undefined}>
      <LastStep step={last} />
    </div>
  );
  const buttons = (
    <>
      <button
        ref={noteBtn}
        type="button"
        className="so-bar-btn so-bar-note"
        aria-expanded={noteOpen}
        aria-keyshortcuts={isMac ? 'Meta+Shift+N' : 'Control+Shift+N'}
        title={`Add a note (${MOD}+Shift+N)`}
        onClick={() => setNoteOpen((v) => !v)}
      >
        <StickyNote size={14} aria-hidden="true" />
        Add a note
      </button>
      <button
        type="button"
        className="so-bar-btn so-bar-stop"
        aria-keyshortcuts={isMac ? 'Meta+Shift+S' : 'Control+Shift+S'}
        title={`Stop (${MOD}+Shift+S)`}
        onClick={() => void showOnce.stop(false)}
      >
        <Square size={12} aria-hidden="true" />
        Stop
      </button>
    </>
  );
  const pop = noteOpen ? <NotePopover botName={s.botName} phone={phone} onClose={closeNote} /> : null;

  if (phone) {
    return (
      <div className="so-bar ph" role="region" aria-label="Recording">
        <div className="so-bar-l">
          <span className="so-dot" aria-hidden="true" />
          <b className="so-bar-title">Recording</b>
          {count}
        </div>
        {middle}
        <div className="so-bar-btns">{buttons}</div>
        {pop}
      </div>
    );
  }
  return (
    <div className="so-bar" role="region" aria-label="Recording">
      <span className="so-dot" aria-hidden="true" />
      <b className="so-bar-title">Recording</b>
      {count}
      <span className="so-sep" aria-hidden="true" />
      {middle}
      {buttons}
      {pop}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// A30–A32, A35: Check the steps

function AddNoteRow({ after, onDone }: { after: string; onDone: () => void }) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const save = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    const err = await showOnce.note(t, after);
    setBusy(false);
    if (err) setError(err);
    else onDone();
  };
  return (
    <li className="so-addnote">
      <span className="so-ic" aria-hidden="true">
        <StickyNote size={13} />
      </span>
      <label htmlFor={`${id}-n`} className="sr-only">
        Note after this step
      </label>
      <textarea
        id={`${id}-n`}
        className="so-input"
        rows={2}
        maxLength={1000}
        autoFocus
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void save();
          } else if (e.key === 'Escape' && !text.trim()) {
            e.preventDefault();
            onDone();
          }
        }}
      />
      <div className="so-addnote-btns">
        <button type="button" className="so-btn so-btn-ghost" onClick={onDone}>
          Cancel
        </button>
        <button type="button" className="so-btn so-btn-primary" disabled={!text.trim() || busy} onClick={save}>
          Add
        </button>
      </div>
      {error && (
        <p className="so-err w-full" role="alert">
          {error}
        </p>
      )}
    </li>
  );
}

function StepRow({ step, disabled, noteAfter, onNoteAfter }: { step: RecordedStep; disabled: boolean; noteAfter: boolean; onNoteAfter: (key: string | null) => void }) {
  const [menu, setMenu] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = stepKey(step);
  const undoRef = useRef<HTMLButtonElement>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const wasDeleted = useRef(step.deleted);
  useLayoutEffect(() => {
    if (step.deleted && !wasDeleted.current) undoRef.current?.focus();
    wasDeleted.current = step.deleted;
  }, [step.deleted]);

  const del = async () => {
    setMenu(false);
    setError(null);
    const err = await showOnce.deleteStep(key);
    if (err) setError(err);
  };
  const restore = async () => {
    setError(null);
    const err = await showOnce.restoreStep(key);
    if (err) setError(err);
  };
  const note = () => {
    setMenu(false);
    onNoteAfter(key);
  };

  if (step.deleted) {
    return (
      <li className="so-step gone" role="status">
        <span className="so-n">{step.n}</span>
        <span className="so-tx">Step {step.n} deleted.</span>
        <button ref={undoRef} type="button" className="so-undo" onClick={restore} aria-label={`Undo deleting step ${step.n}`}>
          <Undo2 size={13} aria-hidden="true" />
          Undo
        </button>
        {error && (
          <span className="so-err" role="alert">
            {error}
          </span>
        )}
      </li>
    );
  }
  const sentence = stepSentence(step);
  return (
    <>
      <li className={`so-step ${step.kind === 'note' ? 'note' : ''} ${step.masked ? 'secret' : ''}`} aria-label={`Step ${step.n}: ${sentence}`}>
        <span className="so-n" aria-hidden="true">
          {step.n}
        </span>
        <span className="so-ic" title={step.masked ? 'Not kept' : KIND_LABEL[step.kind]} aria-hidden="true">
          <StepIcon step={step} />
        </span>
        <span className="so-tx">
          <Parts parts={stepParts(step)} />
          {error && (
            <span className="so-err block" role="alert">
              {error}
            </span>
          )}
        </span>
        <span className="so-tools">
          <button type="button" className="so-icbtn" disabled={disabled} aria-label={`Add a note after step ${step.n}`} title="Add a note after" onClick={note}>
            <StickyNote size={14} aria-hidden="true" />
          </button>
          <button type="button" className="so-icbtn so-del" disabled={disabled} aria-label={`Delete step ${step.n}`} title="Delete" onClick={del}>
            <Trash2 size={14} aria-hidden="true" />
          </button>
          <button
            ref={moreRef}
            type="button"
            className="so-icbtn so-more"
            disabled={disabled}
            aria-label={`Step ${step.n}: add a note after or delete`}
            aria-expanded={menu}
            aria-haspopup="menu"
            onClick={() => setMenu((v) => !v)}
          >
            <MoreHorizontal size={16} aria-hidden="true" />
          </button>
        </span>
        {menu && (
          <div
            className="so-menu"
            role="menu"
            aria-label={`Step ${step.n}`}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                setMenu(false);
                moreRef.current?.focus();
              }
            }}
          >
            <button type="button" role="menuitem" autoFocus onClick={note}>
              <StickyNote size={15} aria-hidden="true" />
              Add a note after
            </button>
            <button type="button" role="menuitem" className="bad" onClick={del}>
              <Trash2 size={15} aria-hidden="true" />
              Delete step {step.n}
            </button>
          </div>
        )}
      </li>
      {noteAfter && <AddNoteRow after={key} onDone={() => onNoteAfter(null)} />}
    </>
  );
}

/**
 * The review sheet. Mounted once at the app's top level, outside the Take over stage, so the
 * stage's own keys (Escape gives focus back to the page) don't swallow the sheet's.
 */
export const StepReview: React.FC = () => {
  const s = useShowOnce();
  const show = !!s?.rec && (s.phase === 'review' || s.phase === 'writing' || s.phase === 'written');
  // Portalled to the body: the Take over stage makes #root inert while it is up (it is portalled
  // too), and a sheet left inside #root would show over the stage but take no clicks or keys.
  return show ? createPortal(<ReviewSheet s={s!} />, document.body) : null;
};

function ReviewSheet({ s }: { s: ShowOnceSession }) {
  const rec = s.rec!;
  const rootRef = useRef<HTMLDivElement>(null);
  const headRef = useRef<HTMLHeadingElement>(null);
  const [confirm, setConfirm] = useState(false);
  const [noteAfter, setNoteAfter] = useState<string | null>(null);
  const [busy, setBusy] = useState<'write' | 'resume' | 'discard' | null>(null);
  const [footErr, setFootErr] = useState<string | null>(null);
  const id = useId();
  const bot = s.botName;
  const writing = s.phase === 'writing';
  const written = s.phase === 'written';
  const listed = s.steps.filter((x) => !x.deleted).length;
  const blocker = writeBlocker(rec, s.steps, bot);
  const engine = engineName(rec.engine);

  useEffect(() => headRef.current?.focus(), []);

  const onEscape = () => {
    if (writing || written) showOnce.close();
    else if (confirm) setConfirm(false);
    else setConfirm(true);
  };
  useEscapeLayer(rootRef, true, onEscape, { modal: true });

  const write = async () => {
    setBusy('write');
    setFootErr(null);
    await showOnce.write();
    setBusy(null);
  };
  const resume = async () => {
    setBusy('resume');
    setFootErr(null);
    const err = await showOnce.resume();
    setBusy(null);
    if (err) setFootErr(err);
  };
  const discard = async () => {
    setBusy('discard');
    const err = await showOnce.discard();
    setBusy(null);
    if (err) setFootErr(err);
  };

  const until = dayOf(rec.steps_expire_at, false);
  // Mira §1: Hand back pressed here keeps Take over until this review closes (A27)
  const banner = stopBanner(rec, listed, { botName: bot, handedBack: s.handBackAfter, holdsStage: s.handBackAfter });
  const writeError = !writing && !written ? rec.write_error ?? (s.error && !footErr ? s.error : null) : null;

  let foot: React.ReactNode;
  if (written) {
    foot = (
      <>
        <p className="so-writing" role="status">
          <Check size={16} aria-hidden="true" />
          <span>
            <b>{bot} wrote the skill.</b> It's in Skills to review.
          </span>
        </p>
        <button type="button" className="so-btn" onClick={() => showOnce.close()}>
          Close
        </button>
      </>
    );
  } else if (writing) {
    foot = (
      <>
        <p className="so-writing" role="status">
          <Loader2 size={16} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
          <span>
            <b>
              {bot} is writing the skill from {stepsWord(listed)}.
            </b>{' '}
            It shows up in Skills to review, usually within a minute. You can close this.
          </span>
        </p>
        <button type="button" className="so-btn" onClick={() => showOnce.close()}>
          Close
        </button>
      </>
    );
  } else if (confirm) {
    foot = (
      <div className="so-confirm" role="alertdialog" aria-labelledby={`${id}-dc`}>
        <p id={`${id}-dc`}>
          <b>Discard all {stepsWord(listed)}?</b> Nothing is written, and the steps are deleted now.
        </p>
        <div className="so-confirm-btns">
          <button type="button" className="so-btn so-btn-ghost" autoFocus onClick={() => setConfirm(false)}>
            Keep them
          </button>
          <button type="button" className="so-btn so-btn-danger" disabled={busy === 'discard'} onClick={discard}>
            <Trash2 size={14} aria-hidden="true" />
            Discard
          </button>
        </div>
        {footErr && (
          <p className="so-err w-full" role="alert">
            {footErr}
          </p>
        )}
      </div>
    );
  } else {
    foot = (
      <>
        <p className="so-foot-note">
          <Lock size={12} aria-hidden="true" />
          <span>
            {lockNote(bot, engine, until)}
          </span>
        </p>
        <button type="button" className="so-btn so-btn-discard" onClick={() => setConfirm(true)}>
          Discard
        </button>
        <span className="so-grow" />
        <button type="button" className="so-btn" disabled={!!busy} onClick={resume}>
          <CircleDot size={14} className="so-rec-ic" aria-hidden="true" />
          Keep recording
        </button>
        <button type="button" className="so-btn so-btn-primary" disabled={!!busy || !!blocker} aria-describedby={blocker ? `${id}-blk` : undefined} onClick={write}>
          {busy === 'write' ? <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Pencil size={14} aria-hidden="true" />}
          Write the skill
        </button>
        {blocker && (
          <p id={`${id}-blk`} className="so-block">
            {blocker}
          </p>
        )}
        {footErr && (
          <p className="so-err w-full" role="alert">
            {footErr}
          </p>
        )}
      </>
    );
  }

  return (
    <div className="so-sheet-wrap">
      <div ref={rootRef} tabIndex={-1} className="so-rv" role="dialog" aria-modal="true" aria-labelledby={`${id}-h`} data-busy={writing ? '' : undefined}>
        <div className="so-rv-head">
          <span className="so-glyph" aria-hidden="true">
            <ListTodo size={16} />
          </span>
          <div className="min-w-0 flex-1">
            <h3 id={`${id}-h`} ref={headRef} tabIndex={-1}>
              Check the steps
            </h3>
            <p>{reviewSub(rec, listed)}</p>
          </div>
          <button type="button" className="so-icbtn" aria-label="Close" onClick={onEscape}>
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="so-rv-body" aria-busy={writing || undefined}>
          {banner && (
            <p className={`so-banner ${banner.tone}`} role="status">
              {banner.tone === 'info' ? <Hand size={14} aria-hidden="true" /> : <Clock size={15} aria-hidden="true" />}
              <span>
                <b>{banner.lead}</b>
                {banner.rest}
              </span>
            </p>
          )}
          {writeError && (
            <p className="so-banner bad" role="alert">
              <TriangleAlert size={15} aria-hidden="true" />
              <span>
                {rec.write_error ? (
                  <>
                    <b>{bot} couldn't write the skill.</b> {rec.write_error} {writeFailedTail(until)}
                  </>
                ) : (
                  writeError
                )}
              </span>
              {rec.write_error && !blocker && (
                <button type="button" className="so-btn" disabled={!!busy} onClick={write}>
                  <RefreshCw size={14} aria-hidden="true" />
                  Try again
                </button>
              )}
            </p>
          )}
          <p className="so-lead">Delete anything {bot} shouldn't learn. Add a note where a choice needs a reason.</p>
          {s.steps.length === 0 ? (
            <p className="so-lead" role="status">
              No steps were recorded.
            </p>
          ) : (
            <ol className="so-steps" aria-label="Recorded steps">
              {s.steps.map((step) => (
                <StepRow key={stepKey(step)} step={step} disabled={writing || written} noteAfter={noteAfter === stepKey(step)} onNoteAfter={setNoteAfter} />
              ))}
            </ol>
          )}
        </div>
        <div className="so-rv-foot">{foot}</div>
      </div>
    </div>
  );
}
