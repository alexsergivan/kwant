import React, { useRef, useState } from 'react';
import { CheckCircle2, ChevronDown, ChevronRight, Loader2, Moon, Users } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { MarkdownRenderer } from './MarkdownRenderer';
import { Decision, changeOptions, consultedLine, decidingDetail, decisionLabel, workAheadSummary } from '../decideFirst';

export interface DecisionChipsProps {
  messageId: string;
  decisions: Decision[];
  /** Teammates asked before deciding (decide-first gate). */
  consulted?: string[];
  /** Records the new pick and asks the bot to redo it; resolves to an error message on failure. */
  onChange?: (index: number, picked: string) => Promise<string | void>;
}

/** Under a reply: what the team decided on its own, each with Change. */
export const DecisionChips: React.FC<DecisionChipsProps> = ({ messageId, decisions, consulted = [], onChange }) => {
  const [open, setOpen] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Set synchronously, so a double tap can't send two changes.
  const lock = useRef(false);
  if (!decisions.length) return null;

  const pick = async (d: Decision, option: string) => {
    if (!onChange || lock.current) return;
    lock.current = true;
    setBusy(option);
    setError(null);
    const err = await onChange(d.index, option);
    lock.current = false;
    setBusy(null);
    if (err) setError(err);
    else setOpen(null);
  };

  return (
    <div role="group" aria-label="Decided by the team" data-decisions className="mt-2 w-full max-w-2xl space-y-2">
      {consulted.length > 0 && (
        <p className="flex items-center gap-1.5 text-[11px] text-zinc-500">
          <Users size={12} aria-hidden="true" />
          {consultedLine(consulted)}
        </p>
      )}
      {decisions.map((d) => {
        const others = changeOptions(d);
        const changing = open === d.index;
        const panelId = `dec-${messageId}-${d.index}`;
        const changed = d.status === 'changed';
        return (
          <div key={d.index} className="rounded-xl border border-zinc-800 bg-zinc-900/40 px-3 py-2">
            <div className="flex items-start gap-2 min-w-0">
              <div className="flex flex-1 flex-wrap items-center gap-x-2 gap-y-1.5 min-w-0">
                {d.question && <span className="text-[11px] text-zinc-400 break-words min-w-0">{d.question}</span>}
                <span
                  className={`inline-flex items-center gap-1 max-w-full px-2 py-0.5 rounded-full border text-[11px] font-medium ${
                    changed ? 'border-sky-500/40 bg-sky-500/10 text-sky-200' : 'border-emerald-500/35 bg-emerald-500/10 text-emerald-200'
                  }`}
                >
                  <CheckCircle2 size={12} className="shrink-0" aria-hidden="true" />
                  <span className="break-words min-w-0">{decisionLabel(d)}</span>
                </span>
              </div>
              {others.length > 0 && onChange && (
                <button
                  type="button"
                  aria-expanded={changing}
                  aria-controls={panelId}
                  onClick={() => {
                    setError(null);
                    setOpen(changing ? null : d.index);
                  }}
                  className="shrink-0 -my-1 px-2.5 py-1.5 min-h-8 rounded-lg text-[11px] font-medium text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 border border-zinc-700/60 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                >
                  {changing ? 'Keep it' : 'Change'}
                </button>
              )}
            </div>
            {d.why && <p className="mt-1 text-[11px] text-zinc-500 break-words">Why: {d.why}</p>}
            {changing && (
              <div id={panelId} className="mt-2 pt-2 border-t border-zinc-800 space-y-1.5">
                <p className="text-[11px] text-zinc-400">Pick another one. The bot redoes what depended on it.</p>
                <div role="group" aria-label={`Other choices for: ${d.question || d.picked}`} className="flex flex-col sm:flex-row sm:flex-wrap gap-1.5">
                  {others.map((o) => (
                    <button
                      key={o}
                      type="button"
                      disabled={busy !== null}
                      onClick={() => pick(d, o)}
                      className="inline-flex items-center justify-center gap-1.5 px-3 py-2 min-h-9 rounded-lg text-xs text-left text-zinc-100 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700/60 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
                    >
                      {busy === o && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                      Use {o}
                    </button>
                  ))}
                </div>
                {error && (
                  <p role="alert" className="text-[11px] text-rose-300">
                    {error}
                  </p>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
};

/** In place of the streaming reply while a held reply is being decided. */
export const DecidingLine: React.FC<{
  botName: string;
  consulted: string[];
}> = ({ botName, consulted }) => (
  <div className="flex items-start gap-3.5 w-full max-w-5xl" data-deciding>
    <ModernAvatar role={botName} name={botName} size={34} />
    <div className="flex flex-col items-start min-w-0 flex-1 max-w-4xl">
      <div className="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-400 font-medium">
        <span className="text-zinc-200">{botName}</span>
        <span className="inline-flex items-center gap-1 text-violet-300 text-[10px]">
          <span className="w-1.5 h-1.5 rounded-full bg-violet-300 animate-ping motion-reduce:animate-none" aria-hidden="true" />
          Deciding
        </span>
      </div>
      <div
        role="status"
        aria-live="polite"
        className="px-4 py-3 rounded-2xl rounded-tl-sm bg-violet-950/20 border border-violet-500/20 text-sm text-zinc-200 shadow-sm w-full"
      >
        <p className="flex items-center gap-2 font-medium text-violet-100">
          <Users size={15} className="text-violet-300 shrink-0" aria-hidden="true" />
          Team is deciding…
        </p>
        <p className="mt-0.5 text-xs text-zinc-400">{decidingDetail(botName, consulted)}</p>
      </div>
    </div>
  </div>
);

/** A "While you were away" message: a subtle label and one line, opening to the whole text. */
export const WorkAheadBody: React.FC<{ messageId: string; text: string }> = ({ messageId, text }) => {
  const [open, setOpen] = useState(false);
  const bodyId = `wa-${messageId}`;
  return (
    <div className="w-full">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-start gap-2 text-left rounded-lg focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        <Moon size={14} className="mt-0.5 shrink-0 text-indigo-300" aria-hidden="true" />
        <span className="flex-1 min-w-0">
          <span className="block text-[11px] font-medium uppercase tracking-wide text-indigo-300/90">While you were away</span>
          {!open && <span className="block text-sm text-zinc-300 break-words">{workAheadSummary(text)}</span>}
        </span>
        <span className="shrink-0 inline-flex items-center gap-0.5 text-[11px] text-zinc-500">
          {open ? 'Less' : 'More'}
          {open ? <ChevronDown size={13} aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
        </span>
      </button>
      <div id={bodyId} hidden={!open} className="mt-2">
        {open && <MarkdownRenderer content={text} />}
      </div>
    </div>
  );
};
