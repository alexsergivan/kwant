// A team from your own words (team-from-words brief, "Entry points" and "Flow"). The person says what
// their bots should do; Suggest a team asks the server (`POST /api/teams/suggest` with `{ need }`) and
// opens the preview sheet on the answer. Nothing is created before Create in the sheet. The same box
// sits in the Get ready card, the sidebar's "Describe your own" sheet and New team; the typed words are
// shared through the ReadyTeams provider, so they survive a cancel, an error or a switch of entry point.
import React, { useEffect, useId, useRef, useState } from 'react';
import { Loader2, Sparkles } from 'lucide-react';
import { useReadyTeams } from './ReadyTeams';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import { NEED_PLACEHOLDER, needError, needProblem, teamsApi } from '../teams/teamTemplates';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

export interface TeamNeedBoxProps {
  label: string;
  /** The Get ready card's box: the sidebar's "Describe your own" focuses it instead of opening a sheet. */
  register?: boolean;
  autoFocus?: boolean;
  /** New team and the small sheet hide themselves while the preview is open… */
  onHide?: () => void;
  /** …come back when it's cancelled… */
  onShow?: () => void;
  /** …and close after a create. */
  onDone?: () => void;
  /** Called before "Add an AI account first" opens Accounts (the small sheet closes itself). */
  onLeave?: () => void;
  className?: string;
}

type Phase = 'idle' | 'suggesting';

export const TeamNeedBox: React.FC<TeamNeedBoxProps> = ({ label, register, autoFocus, onHide, onShow, onDone, onLeave, className = '' }) => {
  const rt = useReadyTeams();
  const phone = useMedia(PHONE_QUERY);
  const id = useId();
  const ta = useRef<HTMLTextAreaElement>(null);
  const [phase, setPhase] = useState<Phase>('idle');
  const [slow, setSlow] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [err, setErr] = useState<ReturnType<typeof needError> | null>(null);
  const [live, setLive] = useState('');
  /** Each ask gets a number; Cancel moves it on, so a late answer is dropped. */
  const seq = useRef(0);
  const local = useState('');
  const value = rt ? rt.needDraft : local[0];
  const setValue = rt ? rt.setNeedDraft : local[1];

  const focusBox = () => {
    const el = ta.current;
    if (!el) return;
    el.scrollIntoView?.({ block: 'center', behavior: 'auto' });
    el.focus({ preventScroll: true });
  };
  useEffect(() => {
    if (!register || !rt) return;
    return rt.registerNeedBox(focusBox);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [register, rt?.registerNeedBox]);
  useEffect(() => {
    if (autoFocus) requestAnimationFrame(() => ta.current?.focus({ preventScroll: true }));
  }, [autoFocus]);
  useEffect(() => {
    if (phase !== 'suggesting') return;
    const t = window.setTimeout(() => setSlow(true), 8000);
    return () => window.clearTimeout(t);
  }, [phase]);
  useEffect(() => () => void (seq.current += 1), []);

  if (!rt?.available) return null;
  const busy = phase === 'suggesting';
  const noAccount = rt.readyAccounts === 0 || !!err?.noAccount;

  const cancel = () => {
    seq.current += 1;
    setPhase('idle');
    setSlow(false);
    setLive('Stopped. Your words are kept.');
    focusBox();
  };

  const submit = async () => {
    if (busy) return;
    if (noAccount) {
      onLeave?.();
      return rt.openAccounts?.();
    }
    const p = needProblem(value);
    setProblem(p);
    if (p) {
      setLive(p);
      return focusBox();
    }
    const need = value.trim();
    const n = ++seq.current;
    setErr(null);
    setSlow(false);
    setPhase('suggesting');
    setLive('Suggesting a team');
    try {
      const s = await teamsApi.suggest({ space: rt.space, need }, 'need');
      if (seq.current !== n) return;
      setPhase('idle');
      setLive('');
      onHide?.();
      rt.open(
        { kind: 'suggest', need, initial: s },
        {
          onCancel: () => {
            onShow?.();
            requestAnimationFrame(focusBox);
          },
          onDone: () => {
            setValue('');
            onDone?.();
          },
        },
      );
    } catch (e) {
      if (seq.current !== n) return;
      const ne = needError(e);
      setErr(ne);
      setPhase('idle');
      // The alert below reads it out; the polite line stays quiet so it isn't said twice.
      setLive('');
      focusBox();
    }
  };

  const hintId = `${id}-hint`;
  const errId = `${id}-err`;
  const hint = busy
    ? slow
      ? 'Usually under a minute.'
      : 'Drafting a team from your words.'
    : noAccount
      ? 'Suggest a team needs an AI account that can run. Your words are kept.'
      : phone
        ? 'Nothing is made until you create it.'
        : 'Enter to suggest · Shift+Enter for a new line. Nothing is made until you create it.';
  const btnH = phone ? 'h-11' : 'h-9';

  return (
    <div
      data-need-box
      className={`min-w-0 ${className}`}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && busy) {
          // Escape stops the ask; the dialog around stays (escape.ts layers respect preventDefault).
          e.preventDefault();
          e.stopPropagation();
          cancel();
        }
      }}
    >
      <label htmlFor={`${id}-ta`} className="block text-[13px] font-medium text-zinc-200 mb-1.5">
        {label}
      </label>
      <textarea
        ref={ta}
        id={`${id}-ta`}
        rows={3}
        value={value}
        placeholder={NEED_PLACEHOLDER}
        readOnly={busy}
        aria-busy={busy || undefined}
        aria-invalid={!!problem || undefined}
        aria-describedby={`${hintId}${problem || err ? ` ${errId}` : ''}`}
        onChange={(e) => {
          setValue(e.target.value);
          if (problem) setProblem(null);
        }}
        onKeyDown={(e) => {
          if (e.key !== 'Enter' || e.shiftKey || e.nativeEvent.isComposing) return;
          e.preventDefault();
          if (!noAccount) void submit();
        }}
        className={`block w-full min-h-[84px] max-h-[220px] resize-y rounded-lg border bg-zinc-950/60 px-3 py-2 text-sm leading-5 text-zinc-50 placeholder:text-zinc-500 outline-none ${
          problem ? 'border-amber-300/70' : 'border-zinc-700 focus:border-sky-400'
        } ${busy ? 'opacity-70' : ''}`}
      />
      <div className={`mt-2 flex ${phone ? 'flex-col items-stretch' : 'flex-wrap items-center'} gap-2`}>
        <button
          type="button"
          onClick={() => void submit()}
          aria-disabled={busy || undefined}
          data-need-submit
          className={`${btnH} inline-flex items-center justify-center gap-1.5 px-4 rounded-lg text-sm font-medium ${
            noAccount && !busy ? 'border border-sky-300/40 text-sky-200 hover:bg-sky-300/10' : 'bg-sky-600 hover:bg-sky-500 text-white'
          } aria-disabled:opacity-70 aria-disabled:cursor-progress ${FOCUS}`}
        >
          {busy ? (
            <>
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              Suggesting…
            </>
          ) : noAccount ? (
            'Add an AI account first'
          ) : (
            <>
              <Sparkles size={14} aria-hidden="true" />
              Suggest a team
            </>
          )}
        </button>
        {busy && (
          <button type="button" onClick={cancel} className={`${btnH} px-3.5 rounded-lg text-sm text-zinc-200 hover:bg-white/5 ${FOCUS}`}>
            Cancel
          </button>
        )}
        <span id={hintId} className="min-w-0 text-xs text-zinc-400">
          {hint}
        </span>
      </div>
      {problem && (
        <p id={errId} className="mt-1.5 text-xs text-amber-200">
          {problem}
        </p>
      )}
      {err && !problem && (
        <div id={errId} role="alert" className="mt-2 rounded-lg border border-rose-400/30 bg-rose-500/10 px-3 py-2 text-[12.5px] leading-[18px] text-rose-100" data-need-error>
          <p className="font-semibold">{err.head}</p>
          {err.reason && <p className="mt-0.5 text-rose-100/85">{err.reason}</p>}
        </div>
      )}
      <p className="sr-only" aria-live="polite">
        {live}
      </p>
    </div>
  );
};
