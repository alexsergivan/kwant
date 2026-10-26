import React, { useRef, useState } from 'react';
import { ArrowUp, Check, CheckCircle2, Clock, HelpCircle, Loader2, MinusCircle } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ANSWER_MAX, Question, QuestionMeta, answerTextError, questionView } from '../questions';
import { whyYouText } from '../decideFirst';

export interface QuestionCardProps {
  meta: QuestionMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  /** Live record from `question_updated`, when known; wins over `meta`. */
  live?: Question | null;
  /** A turn is paused on this question right now (else an answer starts a new reply). */
  waiting: boolean;
  /** Resolves to an error message on failure. */
  onAnswer: (questionId: string, answer: { choice: number } | { text: string }) => Promise<string | void>;
  onDismiss: (questionId: string) => Promise<string | void>;
  flash?: boolean;
}

/**
 * An agent's question in the chat: 2–5 one-tap choices plus a typed answer. Closed (answered or
 * dismissed) cards keep the answer and disable everything. Choices are a group: arrow keys move.
 */
export const QuestionCard: React.FC<QuestionCardProps> = ({ meta, messageId, kwantName, kwantId, live, waiting, onAnswer, onDismiss, flash }) => {
  const view = questionView(meta, live);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState<number | 'text' | 'dismiss' | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Shown right away; the server's question_updated confirms it.
  const [sent, setSent] = useState<string | null>(null);
  const choiceRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const open = view.status === 'open' && sent === null;
  const answer = view.answer ?? sent;
  const headingId = `q-${messageId}`;
  const whyYou = whyYouText(view.why_you);

  const submit = async (a: { choice: number } | { text: string }) => {
    if (!open || busy !== null) return;
    if ('text' in a) {
      const problem = answerTextError(a.text);
      if (problem) {
        setError(problem);
        return;
      }
    }
    setBusy('choice' in a ? a.choice : 'text');
    setError(null);
    const err = await onAnswer(meta.question_id, 'text' in a ? { text: a.text.trim() } : a);
    setBusy(null);
    if (err) setError(err);
    else {
      setSent('text' in a ? a.text.trim() : view.choices[a.choice]);
      setText('');
    }
  };

  const dismiss = async () => {
    if (!open || busy !== null) return;
    setBusy('dismiss');
    setError(null);
    const err = await onDismiss(meta.question_id);
    setBusy(null);
    if (err) setError(err);
  };

  const onChoiceKey = (e: React.KeyboardEvent, i: number) => {
    const n = view.choices.length;
    const to = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? (i + 1) % n : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? (i - 1 + n) % n : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : -1;
    if (to < 0) return;
    e.preventDefault();
    choiceRefs.current[to]?.focus();
  };

  const status =
    view.status === 'cancelled' ? (
      <span className="inline-flex items-center gap-1.5 text-zinc-400">
        <MinusCircle size={13} aria-hidden="true" /> Dismissed: {kwantName} was told you skipped it
      </span>
    ) : !open ? (
      <span className="inline-flex items-center gap-1.5 text-emerald-300">
        <CheckCircle2 size={13} aria-hidden="true" /> Answered{view.status === 'open' ? ', sending…' : ''}
      </span>
    ) : waiting ? (
      <span className="inline-flex items-center gap-1.5 text-amber-300">
        <Clock size={13} aria-hidden="true" /> {kwantName} is waiting for your answer. You can keep chatting meanwhile.
      </span>
    ) : (
      <span className="inline-flex items-center gap-1.5 text-zinc-400">
        <Clock size={13} aria-hidden="true" /> {kwantName} stopped waiting. Answer any time to pick it up in a new reply.
      </span>
    );

  return (
    <section
      data-message-id={messageId}
      aria-labelledby={headingId}
      className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}
    >
      <ModernAvatar role={kwantName} name={kwantName} size={34} kwantId={kwantId} />
      <div
        className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${
          open ? 'border-amber-500/35' : 'border-zinc-800'
        }`}
      >
        <div className="px-4 pt-3 pb-2 space-y-1">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <HelpCircle size={12} className="text-amber-300" aria-hidden="true" />
            <span className="text-zinc-200">{kwantName}</span> asks
          </span>
          <h3 id={headingId} className="text-sm font-semibold text-zinc-100 whitespace-pre-wrap break-words leading-snug">
            {view.question}
          </h3>
          {whyYou && <p className="text-[11px] text-zinc-400 break-words">Only you can answer: {whyYou}</p>}
          {view.checked && <p className="text-[11px] text-zinc-500 break-words">Already tried: {view.checked}</p>}
        </div>

        <div role="group" aria-label="Choices" className="px-4 pb-2 flex flex-col sm:flex-row sm:flex-wrap gap-2">
          {view.choices.map((c, i) => {
            const chosen = !open && answer === c;
            return (
              <button
                key={i}
                ref={(el) => (choiceRefs.current[i] = el)}
                type="button"
                disabled={!open || busy !== null}
                aria-pressed={chosen}
                onClick={() => submit({ choice: i })}
                onKeyDown={(e) => onChoiceKey(e, i)}
                className={`inline-flex items-center justify-center sm:justify-start gap-1.5 min-h-[40px] sm:min-h-[34px] px-3 py-1.5 rounded-xl text-sm text-left border transition-colors break-words focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400 ${
                  chosen
                    ? 'bg-emerald-500/15 border-emerald-500/40 text-emerald-100'
                    : open
                      ? 'bg-zinc-800/80 border-zinc-700 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600 active:scale-[0.98] disabled:opacity-60'
                      : 'bg-zinc-900 border-zinc-800 text-zinc-500'
                }`}
              >
                {busy === i ? (
                  <Loader2 size={13} className="shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                ) : chosen ? (
                  <Check size={13} className="shrink-0" aria-hidden="true" />
                ) : null}
                <span className="min-w-0">{c}</span>
              </button>
            );
          })}
        </div>

        {open ? (
          <form
            className="px-4 pb-2"
            onSubmit={(e) => {
              e.preventDefault();
              submit({ text });
            }}
          >
            <label htmlFor={`${headingId}-text`} className="sr-only">
              Or type your own answer
            </label>
            <div className="flex items-end gap-2 rounded-xl bg-zinc-950 border border-zinc-800 focus-within:border-amber-500/50 p-1.5">
              <textarea
                id={`${headingId}-text`}
                rows={1}
                value={text}
                maxLength={ANSWER_MAX}
                disabled={busy !== null}
                onChange={(e) => {
                  setText(e.target.value);
                  e.target.style.height = 'auto';
                  e.target.style.height = `${Math.min(e.target.scrollHeight, 120)}px`;
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    submit({ text });
                  }
                }}
                placeholder="Or type your own answer…"
                className="flex-1 bg-transparent px-2 py-1 text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none resize-none leading-relaxed"
              />
              <button
                type="submit"
                disabled={!text.trim() || busy !== null}
                aria-label="Send answer"
                className="w-7 h-7 shrink-0 rounded-lg flex items-center justify-center bg-amber-500 text-zinc-950 hover:bg-amber-400 disabled:bg-zinc-800 disabled:text-zinc-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
              >
                {busy === 'text' ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ArrowUp size={14} strokeWidth={2.4} aria-hidden="true" />}
              </button>
            </div>
          </form>
        ) : (
          answer &&
          !view.choices.includes(answer) && (
            <p className="mx-4 mb-2 px-3 py-2 rounded-xl bg-zinc-800/70 border border-zinc-700/60 text-sm text-zinc-100 whitespace-pre-wrap break-words">
              <span className="sr-only">Your answer: </span>
              {answer}
            </p>
          )
        )}

        <div className="px-4 pb-3 pt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
          <p role="status" aria-live="polite" className="min-w-0">
            {status}
          </p>
          {open && (
            <button
              type="button"
              onClick={dismiss}
              disabled={busy !== null}
              className="ml-auto text-zinc-500 hover:text-zinc-300 underline-offset-2 hover:underline disabled:opacity-50 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-zinc-500"
            >
              {busy === 'dismiss' ? 'Skipping…' : 'Skip question'}
            </button>
          )}
          {error && (
            <span role="alert" className="basis-full text-rose-300">
              {error}
            </span>
          )}
        </div>
      </div>
    </section>
  );
};
