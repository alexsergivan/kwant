import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Brain, Check, ChevronRight, CircleDashed, CircleSlash, Hand, Info, ListTree, Loader2, ShieldQuestion, TimerOff, X } from 'lucide-react';
import {
  ANSWER_LABEL,
  ActivityStep,
  CardAnswer,
  LiveActivity,
  SavedActivity,
  activitySummary,
  shownStepCount,
  foldAnswers,
  formatDuration,
  isOpenStep,
  plainTitle,
  shownStatus,
  statusLabel,
  thinkingText,
} from '../activity';
import { readableStepTitle } from '../toolLabel';

/** "Running `cargo test`" with the quoted part in mono. */
const Title: React.FC<{ text: string }> = ({ text }) => (
  <>
    {text.split(/(`[^`]*`)/g).map((part, i) =>
      part.startsWith('`') && part.endsWith('`') && part.length > 1 ? (
        <code key={i} className="font-mono text-[11px] px-1 py-px rounded bg-zinc-800/80 text-zinc-100 [overflow-wrap:anywhere] [box-decoration-break:clone] [-webkit-box-decoration-break:clone]">
          {part.slice(1, -1)}
        </code>
      ) : (
        <React.Fragment key={i}>{part}</React.Fragment>
      ),
    )}
  </>
);

const StatusIcon: React.FC<{ step: ActivityStep }> = ({ step }) => {
  const label = statusLabel(step);
  const icon = (() => {
    switch (shownStatus(step)) {
      case 'running':
        return step.kind === 'thinking' ? (
          <Brain size={13} className="text-info animate-pulse motion-reduce:animate-none" />
        ) : (
          <Loader2 size={13} className="text-sky-300 animate-spin motion-reduce:animate-none" />
        );
      case 'waiting':
        return step.kind === 'approval' ? <ShieldQuestion size={13} className="text-amber-300" /> : <Hand size={13} className="text-amber-300" />;
      case 'ok':
      case 'done':
        return step.kind === 'thinking' ? <Brain size={13} className="text-sky-300/80" /> : <Check size={13} className="text-ok" />;
      case 'failed':
        return <X size={13} className="text-rose-300" />;
      case 'declined':
        return <CircleSlash size={13} className="text-zinc-400" />;
      case 'stopped':
        return <CircleSlash size={13} className="text-zinc-400" />;
      case 'unanswered':
        return <TimerOff size={13} className="text-amber-300" />;
      case 'rehearsed':
        return <CircleDashed size={13} className="text-zinc-400" />;
    }
  })();
  return (
    <span className="mt-px flex-shrink-0 inline-flex" role="img" aria-label={label} title={label}>
      {icon}
    </span>
  );
};

/** Re-renders every second while `on`, for the time a running step has taken so far. */
function useNow(on: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [on]);
  return now;
}

const stepTime = (s: ActivityStep, now: number): string | null => {
  if (typeof s.duration_ms === 'number') return s.kind === 'note' ? null : formatDuration(s.duration_ms);
  if (!isOpenStep(s)) return null;
  const started = Date.parse(s.started_at);
  return Number.isFinite(started) && now - started >= 1000 ? formatDuration(now - started) : null;
};

/** Live thinking text: follows the end while the person hasn't scrolled up. */
const ThinkingBox: React.FC<{ text: string; live: boolean; id: string }> = ({ text, live, id }) => {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && live && stick.current) el.scrollTop = el.scrollHeight;
  }, [text, live]);
  return (
    <div
      id={id}
      ref={ref}
      tabIndex={0}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
      className="mt-1 max-h-56 overflow-y-auto rounded-lg bg-zinc-950/80 border border-zinc-800 px-3 py-2 text-xs text-zinc-300 whitespace-pre-wrap break-words leading-relaxed focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
      aria-label="Thinking"
    >
      {text || <span className="text-zinc-400 italic">Nothing shared yet…</span>}
    </div>
  );
};

const STEP_MARK: Partial<Record<ActivityStep['status'], string>> = {
  failed: 'text-rose-300 border-rose-500/30',
  unanswered: 'text-amber-300 border-amber-500/30',
  // 28a: a call the person said no to says so in words too, neutral (their choice, not a failure).
  declined: 'text-zinc-300 border-zinc-500/40',
};

/**
 * The row's chip, and words only a screen reader hears. 28s: a command row whose card folded into it carries the
 * card's answer: "You said no" (grey) or "Not approved in time" (amber) as its chip. "You said OK" has no chip
 * (Mira: too loud on a reply of many approved calls); it is read after the title instead. A command that then
 * failed keeps "Failed" and reads its answer after it. A human step the person said no to has no chip (its title
 * says "You said no: …"), nor does a card that said no on its own row (its title says so too).
 */
function stepMark(step: ActivityStep, answer: CardAnswer | undefined): { chip: { text: string; cls: string; kind: string } | null; also?: string } {
  const shown = shownStatus(step);
  if (answer === 'ok' && shown !== 'failed') return { chip: null, also: ANSWER_LABEL.ok };
  if (answer && shown !== 'failed') return { chip: { text: ANSWER_LABEL[answer], cls: STEP_MARK[answer]!, kind: answer } };
  const also = answer ? ANSWER_LABEL[answer] : undefined;
  if (step.status === 'declined' && (step.kind === 'approval' || step.kind === 'human_step')) return { chip: null, also };
  const cls = STEP_MARK[shown];
  return { chip: cls ? { text: statusLabel(step), cls, kind: shown } : null, also };
}

const StepRow: React.FC<{ step: ActivityStep; live?: LiveActivity; now: number; listId: string; opened?: boolean; answer?: CardAnswer }> = ({
  step,
  live,
  now,
  listId,
  opened = false,
  answer,
}) => {
  const time = stepTime(step, now);
  const running = step.kind === 'thinking' && step.status === 'running';
  const text = step.kind === 'thinking' ? thinkingText(step, live) : '';
  // The running thought is open; finished ones fold until asked.
  const [open, setOpen] = useState(running);
  useEffect(() => {
    if (running) setOpen(true);
  }, [running]);

  if (step.kind === 'note')
    return (
      <li className="flex items-start gap-2 px-3 py-1.5 text-xs text-zinc-400">
        <Info size={13} className="mt-px flex-shrink-0 text-zinc-400" aria-hidden="true" />
        <span className="min-w-0 break-words">{step.title}</span>
      </li>
    );

  const waiting = step.status === 'waiting';
  // 26d: a step that failed or wasn't approved in time says so in words, not only by its icon, even when a later
  // step worked. The icon's label already names it for screen readers, so the mark is hidden from them.
  // The approval row keeps its icon only: the mark belongs on the command the person said no to.
  const { chip: mark, also } = stepMark(step, answer);
  const head = (
    <>
      <StatusIcon step={step} />
      <span className={`min-w-0 flex-1 break-words ${step.status === 'stopped' ? 'text-zinc-400' : waiting ? 'text-amber-100' : 'text-zinc-200'}`}>
        {step.kind === 'thinking' ? (running ? 'Thinking…' : 'Thought') : <Title text={readableStepTitle(step.title, step.outcome ?? step.status)} />}
      </span>
      {also && <span className="sr-only">{also}</span>}
      {mark && (
        <span
          aria-hidden="true"
          data-step-mark={mark.kind}
          className={`flex-shrink-0 rounded border px-1 text-[10px] leading-4 font-medium ${mark.cls}`}
          data-answer={answer}
        >
          {mark.text}
        </span>
      )}
      {time && <span className="flex-shrink-0 font-mono text-[10px] text-zinc-400 tabular-nums">{time}</span>}
    </>
  );

  if (step.kind === 'thinking') {
    const boxId = `${listId}-${step.id}-text`;
    return (
      <li className="px-3 py-1.5 text-xs">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-controls={open ? boxId : undefined}
          disabled={!text && !running}
          className="w-full flex items-start gap-2 text-left rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 disabled:cursor-default"
        >
          {head}
          {(text || running) && (
            <ChevronRight size={13} className={`mt-px flex-shrink-0 text-zinc-400 transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
          )}
        </button>
        {open && (text || running) && <ThinkingBox id={boxId} text={text} live={running} />}
      </li>
    );
  }

  return (
    <li
      id={`${listId}-${step.id}`}
      tabIndex={-1}
      data-step-status={step.status}
      className={`px-3 py-1.5 text-xs focus:outline-none focus-visible:bg-zinc-800/60 focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-sky-400 ${waiting ? 'bg-amber-500/[0.06]' : ''}`}
    >
      {step.detail ? (
        <details className="group" open={opened || undefined}>
          <summary className="flex items-start gap-2 cursor-pointer list-none rounded [&::-webkit-details-marker]:hidden focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400">
            {head}
            <ChevronRight size={13} className="mt-px flex-shrink-0 text-zinc-400 transition-transform motion-reduce:transition-none group-open:rotate-90" aria-hidden="true" />
          </summary>
          <pre
            className={`mt-1 ml-5 whitespace-pre-wrap [overflow-wrap:anywhere] rounded-md border px-2 py-1.5 max-h-40 overflow-y-auto ${
              // 28a: the person's no is a sentence, not output, so it isn't set in mono.
              step.status === 'declined' ? 'font-sans text-xs' : 'font-mono text-[11px]'
            } ${step.status === 'failed' ? 'text-rose-200 bg-rose-950/30 border-rose-500/20' : 'text-zinc-300 bg-zinc-950/80 border-zinc-800'}`}
          >
            {step.detail}
          </pre>
        </details>
      ) : (
        <div className="flex items-start gap-2">{head}</div>
      )}
    </li>
  );
};

const Steps: React.FC<{ steps: ActivityStep[]; live?: LiveActivity; dropped?: number; id: string; openStep?: string | null }> = ({ steps, live, dropped = 0, id, openStep = null }) => {
  const now = useNow(steps.some(isOpenStep));
  // 28s: one row per answer. A settled card folds into its command row; draw time only.
  const { rows, answers, foldedInto } = useMemo(() => foldAnswers(steps), [steps]);
  const opened = openStep ? foldedInto.get(openStep) ?? openStep : null;
  return (
    <ol id={id} aria-label="Steps" className="divide-y divide-zinc-700/40">
      {rows.map((s) => (
        <StepRow key={s.id} step={s} live={live} now={now} listId={id} opened={s.id === opened} answer={answers.get(s.id)} />
      ))}
      {dropped > 0 && (
        <li className="px-3 py-1.5 text-xs text-zinc-400">
          {dropped} more {dropped === 1 ? 'step' : 'steps'} not kept
        </li>
      )}
    </ol>
  );
};

/** The saved reply's steps row (and the live one once it folds, 18f): one class string, so both are the same height. */
// 28z: on a phone the saved summary may wrap to two lines, so both rows keep their icons on the first line (items-start + 1.5px).
// Below sm the three children (icon, text, chevron) are a one-row grid centred in the 44 px touch box (content-center),
// so a one-line row still sits 14/14 and a two-line row fills the box (Mira, 28z round 2).
export const STEPS_ROW_BUTTON =
  'max-w-full min-w-0 flex items-center gap-2 px-3 py-1.5 rounded-xl text-xs text-zinc-400 hover:text-zinc-200 bg-zinc-900/70 hover:bg-zinc-800/70 border border-zinc-700/60 hover:border-zinc-600/80 transition-colors cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400 max-sm:grid max-sm:grid-cols-[auto_minmax(0,1fr)_auto] max-sm:content-center max-sm:items-start';
/** The list the steps row opens to. */
export const STEPS_ROW_LIST = 'mt-1.5 w-full max-w-2xl rounded-xl border border-zinc-700/60 bg-zinc-900/60 overflow-hidden shadow-[var(--kws-shadow)]';

/**
 * While the bot works: every step so far, newest last. `compact` (phone, 0.6.13 spec C2) folds it to
 * one 44 px row, "Steps · 2 ▸", that opens to the same list; the engine's note (what it can't show of
 * its thinking) moves behind an ⓘ in that row, since the working strip already says the bot works.
 * `folded` (once the reply has words, 18f) is rendered below the bubble: on desktop it draws the saved reply's
 * one row instead of the card, and on a phone the compact row keeps its look. Either way it sits where the landed
 * action row goes, so nothing above the words moves; opened, it stays open until the reply lands.
 */
export const LiveActivityList: React.FC<{ live: LiveActivity; idBase: string; compact?: boolean; folded?: boolean; lead?: React.ReactNode }> = ({ live, idBase, compact = false, folded = false, lead }) => {
  const { steps } = live;
  const [open, setOpen] = useState(false);
  const [noteOpen, setNoteOpen] = useState(false);
  if (steps.length === 0) return null;
  const work = shownStepCount(steps);
  const current = [...steps].reverse().find(isOpenStep);
  // Only waits are announced; a stream of tool names would be noise.
  const announce = (
    <p className="sr-only" role="status" aria-live="polite">
      {current?.status === 'waiting' ? readableStepTitle(plainTitle(current.title)) : ''}
    </p>
  );
  if (compact) {
    const notes = steps.filter((s) => s.kind === 'note');
    const workSteps = steps.filter((s) => s.kind !== 'note');
    const listId = `${idBase}-live`;
    const noteId = `${idBase}-note`;
    return (
      <section aria-label="What the bot is doing" data-steps-compact="" className={`${folded ? 'mt-1' : 'mb-2'} w-full max-w-2xl rounded-xl border border-zinc-700/60 bg-zinc-900/60 overflow-hidden`}>
        {lead && <div className="flex items-center min-h-[44px] px-2.5 border-b border-zinc-700/40">{lead}</div>}
        <div className="flex items-center">
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            aria-controls={open ? listId : undefined}
            disabled={work === 0}
            className="min-w-0 flex-1 flex items-center gap-2 min-h-[44px] px-3 text-left text-sm text-zinc-300 disabled:cursor-default focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[#b8adff]"
          >
            <ListTree size={14} className="shrink-0 text-sky-400/80" aria-hidden="true" />
            <span className="truncate">Steps · {work}</span>
            {work > 0 && (
              <ChevronRight size={14} className={`shrink-0 text-zinc-400 transition-transform duration-150 motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
            )}
          </button>
          {notes.length > 0 && (
            <button
              type="button"
              onClick={() => setNoteOpen((o) => !o)}
              aria-expanded={noteOpen}
              aria-controls={noteOpen ? noteId : undefined}
              aria-label="About this engine's steps"
              className="shrink-0 inline-flex items-center justify-center w-11 h-11 text-zinc-400 hover:text-zinc-200 focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[#b8adff]"
            >
              <Info size={16} aria-hidden="true" />
            </button>
          )}
        </div>
        {announce}
        {noteOpen && notes.length > 0 && (
          <div id={noteId} className="border-t border-zinc-700/40 px-3 py-2 space-y-1 text-xs leading-5 text-zinc-400">
            {notes.map((n) => (
              <p key={n.id} className="break-words">{n.title}</p>
            ))}
          </div>
        )}
        {open && work > 0 && (
          <div className="border-t border-zinc-700/40 max-h-[22rem] overflow-y-auto">
            <Steps steps={workSteps} live={live} id={listId} />
          </div>
        )}
      </section>
    );
  }
  if (folded) {
    const listId = `${idBase}-live`;
    return (
      <section aria-label="What the bot is doing" data-steps-folded="" className="mt-2 max-w-full">
        <div className="flex max-w-full min-w-0 items-center gap-2">
          <button type="button" onClick={() => setOpen((o) => !o)} aria-expanded={open} aria-controls={open ? listId : undefined} className={STEPS_ROW_BUTTON}>
            <ListTree size={13} className="flex-shrink-0 max-sm:mt-[1.5px] text-sky-400/80" aria-hidden="true" />
            {/* No time while running, as in the card */}
            <span className="truncate min-w-0">Working · {work} {work === 1 ? 'step' : 'steps'}</span>
            <ChevronRight size={13} className={`flex-shrink-0 max-sm:mt-[1.5px] text-zinc-400 transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
          </button>
          {lead && <div className="min-w-0 shrink">{lead}</div>}
        </div>
        {announce}
        {open && (
          <div className={STEPS_ROW_LIST}>
            <div className="max-h-[22rem] overflow-y-auto">
              <Steps steps={steps} live={live} id={listId} />
            </div>
          </div>
        )}
      </section>
    );
  }
  return (
    <section aria-label="What the bot is doing" className="mb-2 w-full max-w-2xl rounded-xl border border-zinc-700/60 bg-zinc-900/60 overflow-hidden shadow-[var(--kws-shadow)]">
      <p className="flex items-center gap-1.5 px-3 pt-2 pb-1 text-[11px] font-medium uppercase tracking-wide text-zinc-400">
        <ListTree size={11} className="text-sky-400/80" aria-hidden="true" />
        Working
        <span className="normal-case tracking-normal font-normal">
          {/* No time while running: the working strip shows the whole turn's, the open step its own. */}
          · {work} {work === 1 ? 'step' : 'steps'}
        </span>
      </p>
      {lead && <div className="px-3 pt-1 pb-2">{lead}</div>}
      {announce}
      <div className="max-h-[22rem] overflow-y-auto">
        <Steps steps={steps} live={live} id={`${idBase}-live`} />
      </div>
    </section>
  );
};

/** On a saved reply: "Worked 0:41 · 12 steps", opening to the list. `children` go under the steps. */
export const SavedActivityList: React.FC<{
  activity: SavedActivity;
  idBase: string;
  children?: React.ReactNode;
  /** 26d: a nonce from the line under the reply; each new value opens the list at `focusStep` and focuses it. */
  openRequest?: number;
  focusStep?: string | null;
  /** Start open (tests and shots). */
  defaultOpen?: boolean;
}> = ({ activity, idBase, children, openRequest, focusStep = null, defaultOpen = false }) => {
  const [open, setOpen] = useState(defaultOpen);
  const [asked, setAsked] = useState<string | null>(defaultOpen ? focusStep : null);
  const listId = `${idBase}-steps`;
  useEffect(() => {
    if (!openRequest) return;
    setOpen(true);
    setAsked(focusStep);
  }, [openRequest, focusStep]);
  useEffect(() => {
    if (!openRequest || !focusStep) return;
    // 28s: a card that folded into its command row is focused there.
    const el = document.getElementById(`${listId}-${foldAnswers(activity.steps).foldedInto.get(focusStep) ?? focusStep}`);
    el?.focus({ preventScroll: true });
    el?.scrollIntoView({ block: 'nearest', behavior: window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  }, [openRequest, focusStep, listId, open, activity.steps]);
  const failed = activity.steps.some((s) => s.status === 'failed');
  const unanswered = activity.steps.some((s) => s.status === 'unanswered');
  return (
    <div className="mb-2 max-w-full">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        className={STEPS_ROW_BUTTON}
      >
        <ListTree size={13} className={`flex-shrink-0 max-sm:mt-[1.5px] ${unanswered ? 'text-amber-300' : failed ? 'text-rose-300' : 'text-sky-400/80'}`} aria-hidden="true" />
        {/* 28z: on a phone two lines at most, left-aligned, and a line only breaks between " · " parts ("you said no to 1" stays whole). */}
        <span className="min-w-0 text-left break-words max-sm:line-clamp-2 sm:truncate" data-steps-summary="">
          {activitySummary(activity).split(' · ').map((part, i) => (
            <React.Fragment key={i}>{i > 0 && ' · '}<span className="whitespace-nowrap">{part}</span></React.Fragment>
          ))}
        </span>
        <ChevronRight size={13} className={`flex-shrink-0 max-sm:mt-[1.5px] text-zinc-400 transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
      </button>
      {open && (
        <div className={STEPS_ROW_LIST}>
          <Steps steps={activity.steps} dropped={activity.dropped} id={listId} openStep={asked} />
          {children}
        </div>
      )}
    </div>
  );
};

