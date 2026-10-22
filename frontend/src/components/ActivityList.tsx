import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Brain, Check, ChevronRight, CircleSlash, Hand, Info, ListTree, Loader2, ShieldQuestion, X } from 'lucide-react';
import {
  ActivityStep,
  LiveActivity,
  SavedActivity,
  activitySpan,
  activitySummary,
  formatDuration,
  isOpenStep,
  plainTitle,
  statusLabel,
  thinkingText,
} from '../activity';

/** "Running `cargo test`" with the quoted part in mono. */
const Title: React.FC<{ text: string }> = ({ text }) => (
  <>
    {text.split(/(`[^`]*`)/g).map((part, i) =>
      part.startsWith('`') && part.endsWith('`') && part.length > 1 ? (
        <code key={i} className="font-mono text-[11px] px-1 py-px rounded bg-zinc-800/80 text-zinc-100 break-all">
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
    switch (step.status) {
      case 'running':
        return step.kind === 'thinking' ? (
          <Brain size={13} className="text-violet-300 animate-pulse motion-reduce:animate-none" />
        ) : (
          <Loader2 size={13} className="text-sky-300 animate-spin motion-reduce:animate-none" />
        );
      case 'waiting':
        return step.kind === 'approval' ? <ShieldQuestion size={13} className="text-amber-300" /> : <Hand size={13} className="text-amber-300" />;
      case 'ok':
      case 'done':
        return step.kind === 'thinking' ? <Brain size={13} className="text-violet-300/80" /> : <Check size={13} className="text-emerald-300" />;
      case 'failed':
        return <X size={13} className="text-rose-300" />;
      case 'declined':
        return <CircleSlash size={13} className="text-zinc-400" />;
      case 'stopped':
        return <CircleSlash size={13} className="text-zinc-400" />;
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
      className="mt-1 max-h-56 overflow-y-auto rounded-lg bg-zinc-950/80 border border-zinc-800 px-3 py-2 text-xs text-zinc-300 whitespace-pre-wrap break-words leading-relaxed focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400"
      aria-label="Thinking"
    >
      {text || <span className="text-zinc-400 italic">Nothing shared yet…</span>}
    </div>
  );
};

const StepRow: React.FC<{ step: ActivityStep; live?: LiveActivity; now: number; listId: string }> = ({ step, live, now, listId }) => {
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
      <li className="flex items-start gap-2 px-3 py-1.5 text-[11px] text-zinc-400">
        <Info size={13} className="mt-px flex-shrink-0 text-zinc-400" aria-hidden="true" />
        <span className="min-w-0 break-words">{step.title}</span>
      </li>
    );

  const waiting = step.status === 'waiting';
  const head = (
    <>
      <StatusIcon step={step} />
      <span className={`min-w-0 flex-1 break-words ${step.status === 'stopped' ? 'text-zinc-400' : waiting ? 'text-amber-100' : 'text-zinc-200'}`}>
        {step.kind === 'thinking' ? (running ? 'Thinking…' : 'Thought') : <Title text={step.title} />}
      </span>
      {time && <span className="flex-shrink-0 font-mono text-[10px] text-zinc-400 tabular-nums">{time}</span>}
    </>
  );

  if (step.kind === 'thinking') {
    const boxId = `${listId}-${step.id}-text`;
    return (
      <li className="px-3 py-1.5 text-[12px]">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-controls={open ? boxId : undefined}
          disabled={!text && !running}
          className="w-full flex items-start gap-2 text-left rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400 disabled:cursor-default"
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
    <li className={`px-3 py-1.5 text-[12px] ${waiting ? 'bg-amber-500/[0.06]' : ''}`}>
      {step.detail ? (
        <details className="group">
          <summary className="flex items-start gap-2 cursor-pointer list-none rounded [&::-webkit-details-marker]:hidden focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400">
            {head}
            <ChevronRight size={13} className="mt-px flex-shrink-0 text-zinc-400 transition-transform motion-reduce:transition-none group-open:rotate-90" aria-hidden="true" />
          </summary>
          <pre
            className={`mt-1 ml-5 font-mono text-[11px] whitespace-pre-wrap break-all rounded-md border px-2 py-1.5 max-h-40 overflow-y-auto ${
              step.status === 'failed' ? 'text-rose-200 bg-rose-950/30 border-rose-500/20' : 'text-zinc-300 bg-zinc-950/80 border-zinc-800'
            }`}
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

const Steps: React.FC<{ steps: ActivityStep[]; live?: LiveActivity; dropped?: number; id: string }> = ({ steps, live, dropped = 0, id }) => {
  const now = useNow(steps.some(isOpenStep));
  return (
    <ol id={id} aria-label="Steps" className="divide-y divide-zinc-800/60">
      {steps.map((s) => (
        <StepRow key={s.id} step={s} live={live} now={now} listId={id} />
      ))}
      {dropped > 0 && (
        <li className="px-3 py-1.5 text-[11px] text-zinc-400">
          {dropped} more {dropped === 1 ? 'step' : 'steps'} not kept
        </li>
      )}
    </ol>
  );
};

/** While the bot works: every step so far, newest last. */
export const LiveActivityList: React.FC<{ live: LiveActivity; idBase: string }> = ({ live, idBase }) => {
  const { steps } = live;
  if (steps.length === 0) return null;
  const work = steps.filter((s) => s.kind !== 'note').length;
  const current = [...steps].reverse().find(isOpenStep);
  const span = activitySpan(steps);
  return (
    <section aria-label="What the bot is doing" className="mb-2 w-full max-w-2xl rounded-xl border border-zinc-800 bg-zinc-950/50 overflow-hidden">
      <p className="flex items-center gap-1.5 px-3 pt-2 pb-1 text-[10px] font-medium uppercase tracking-wide text-zinc-400">
        <ListTree size={11} className="text-sky-400/80" aria-hidden="true" />
        Working
        <span className="normal-case tracking-normal font-normal">
          · {work} {work === 1 ? 'step' : 'steps'}
          {span !== null && span >= 1000 ? ` · ${formatDuration(span)}` : ''}
        </span>
      </p>
      {/* Only waits are announced; a stream of tool names would be noise. */}
      <p className="sr-only" role="status" aria-live="polite">
        {current?.status === 'waiting' ? plainTitle(current.title) : ''}
      </p>
      <div className="max-h-[22rem] overflow-y-auto">
        <Steps steps={steps} live={live} id={`${idBase}-live`} />
      </div>
    </section>
  );
};

/** On a saved reply: "12 steps · 41s", opening to the list. `children` go under the steps. */
export const SavedActivityList: React.FC<{ activity: SavedActivity; idBase: string; children?: React.ReactNode }> = ({ activity, idBase, children }) => {
  const [open, setOpen] = useState(false);
  const listId = `${idBase}-steps`;
  // The engine's "no thinking shared" note stays in view while it explains a gap.
  const note = activity.steps.some((s) => s.kind === 'thinking') ? undefined : activity.steps.find((s) => s.kind === 'note');
  const failed = activity.steps.some((s) => s.status === 'failed');
  return (
    <div className="mb-2 max-w-full">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        className="max-w-full min-w-0 flex items-center gap-2 px-3 py-1.5 rounded-xl text-xs text-zinc-400 hover:text-zinc-200 bg-zinc-900/80 hover:bg-zinc-900 border border-zinc-800 hover:border-zinc-700/80 transition-colors cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400"
      >
        <ListTree size={13} className={failed ? 'text-rose-300' : 'text-sky-400/80'} aria-hidden="true" />
        <span className="truncate min-w-0">{activitySummary(activity)}</span>
        <ChevronRight size={13} className={`flex-shrink-0 text-zinc-400 transition-transform motion-reduce:transition-none ${open ? 'rotate-90' : ''}`} aria-hidden="true" />
      </button>
      {open && (
        <div className="mt-1.5 w-full max-w-2xl rounded-xl border border-zinc-800 bg-zinc-950/50 overflow-hidden">
          <Steps steps={activity.steps} dropped={activity.dropped} id={listId} />
          {children}
        </div>
      )}
      {!open && note && <p className="mt-1 px-1 text-[11px] text-zinc-400 break-words">{note.title}</p>}
    </div>
  );
};

