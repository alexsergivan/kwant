// 0.7.6 "Work from Jira": the lead's questions on a ticket card (design.md §5). At most 3, each blocking.
// Ask on Jira never posts in one tap: it first shows the exact comment, and only **Post comment** posts it.
// Question words, choices and the preview come from Jira and a model: rendered as text only.

import React, { useState } from 'react';
import { Check, ExternalLink, HelpCircle, Loader2, MessageSquare } from 'lucide-react';
import type { AskPosted, AskPreview, Brief, BriefQuestion } from '../../dev/tickets';
import { mentionParts, openQuestions } from '../../dev/tickets';
import { clock } from '../../dev/logic';
import { SourceChip } from './TicketBrief';

export interface TicketQuestionsProps {
  ticketKey: string;
  brief: Brief;
  lead: string;
  /** Who the Jira comment goes to (reporter, else assignee); null hides Ask on Jira's name. */
  askTo: string | null;
  /** Answer one question in the card. Missing: the choices are shown but read-only. */
  onAnswer?: (questionId: string, value: string) => Promise<void> | void;
  /** Answer every open question with its default. Missing hides the button. */
  onDefaults?: () => Promise<void> | void;
  /** Fetch the exact comment. Missing hides Ask on Jira. */
  onAskPreview?: () => Promise<AskPreview>;
  /** Post it. Called only from the Post comment button. */
  onAskPost?: (preview: AskPreview) => Promise<AskPosted>;
}

type Ask =
  | { s: 'idle' }
  | { s: 'loading' }
  | { s: 'open'; p: AskPreview }
  | { s: 'posting'; p: AskPreview }
  | { s: 'error'; p: AskPreview | null; message: string };

const errText = (e: unknown) => (e instanceof Error && e.message ? e.message : 'That didn’t work. Try again.');

const Question: React.FC<{
  q: BriefQuestion;
  n: number;
  busy: boolean;
  onPick?: (v: string) => void;
}> = ({ q, n, busy, onPick }) => {
  const [changing, setChanging] = useState(false);
  const labelId = `tkq-${q.id}`;
  const a = q.answer;
  const picked = a && !changing ? a.value : null;
  return (
    <li className="tk-q" data-q={q.id} data-answered={a ? 'yes' : 'no'}>
      <h5 id={labelId}>
        <span className="n" aria-hidden="true">
          {n}
        </span>
        <span className="sr-only">Question {n}: </span>
        {q.text}
      </h5>
      {q.why_blocks && (
        <p className="blocks">
          <span className="lb">Why it blocks:</span> {q.why_blocks}
          {q.conflict_sources.length > 0 && (
            <span className="srcs">
              {q.conflict_sources.map((s, i) => (
                <SourceChip key={i} s={s} />
              ))}
            </span>
          )}
        </p>
      )}
      {q.jira_reply && !a && (
        <p className="blocks" data-jira-reply>
          <span className="lb">{q.jira_reply.name ?? 'Jira'} replied on Jira:</span> <q>{q.jira_reply.text}</q> Pick the choice it means.
        </p>
      )}
      {q.choices.length > 0 && (!a || changing) && (
        <div className="tk-choices" role="group" aria-labelledby={labelId}>
          {q.choices.map((c) => (
            <button
              key={c}
              type="button"
              className="tk-ch"
              aria-pressed={picked === c}
              disabled={!onPick || busy}
              onClick={() => {
                setChanging(false);
                onPick?.(c);
              }}
            >
              {c}
              {q.default === c && (
                <span className="def" aria-label="(default)">
                  default
                </span>
              )}
            </button>
          ))}
        </div>
      )}
      {a && !changing ? (
        <p className="tk-answered">
          <Check size={13} aria-hidden="true" />
          {a.by === 'jira_user' ? (
            <span>
              {a.name ?? 'Jira'} answered on Jira: <b>{a.value}</b>
              {clock(a.at) ? ` · ${clock(a.at)}` : ''}
            </span>
          ) : (
            <span>
              {a.went_with_default ? 'Went with the default' : 'You picked'} <b>{a.value}</b>
              {clock(a.at) ? ` · ${clock(a.at)}` : ''}
            </span>
          )}
          {a.comment_url && (
            <a className="tk-src" href={a.comment_url} target="_blank" rel="noreferrer noopener">
              <MessageSquare aria-hidden="true" />
              <span>comment</span>
              <span className="sr-only"> (opens in a new tab)</span>
            </a>
          )}
          {onPick && q.choices.length > 0 && (
            <button type="button" className="tk-textbtn" onClick={() => setChanging(true)}>
              Change<span className="sr-only"> the answer to question {n}</span>
            </button>
          )}
        </p>
      ) : (
        q.default && (
          <p className="tk-dflt">
            <b>Default: {q.default}</b>
            {q.default_reason ? `, because ${q.default_reason.replace(/^because\s+/i, '')}` : ''}
          </p>
        )
      )}
    </li>
  );
};

/** The Jira comment preview: the exact text, with the mention drawn as a chip. Text only. */
const Preview: React.FC<{ p: AskPreview; ticketKey: string; posting: boolean; error: string | null; onCancel: () => void; onPost: () => void }> = ({
  p,
  ticketKey,
  posting,
  error,
  onCancel,
  onPost,
}) => (
  <div className="tk-jprev" role="region" aria-label={`Comment preview for ${ticketKey}`}>
    <div className="bar">
      This exact comment goes on <b>{p.key || ticketKey}</b>
      {p.as.email || p.as.display_name ? `, as you (${p.as.email ?? p.as.display_name})` : ', as you'}
    </div>
    <div className="body" data-preview>
      {mentionParts(p.text, p.to).map((m, i) =>
        m.mention ? (
          <span key={i} className="mention">
            {m.text}
          </span>
        ) : (
          <React.Fragment key={i}>{m.text}</React.Fragment>
        ),
      )}
    </div>
    {error && (
      <p className="tk-err" role="alert">
        {error}
      </p>
    )}
    <div className="acts">
      <span className="note">Posted once. kwant bots never edit or delete it.</span>
      <button type="button" className="tk-btn ghost" onClick={onCancel} disabled={posting}>
        Cancel
      </button>
      <button type="button" className="tk-btn primary" onClick={onPost} disabled={posting} aria-busy={posting || undefined} data-post>
        {posting && <Loader2 size={14} className="tk-spin" aria-hidden="true" />}
        {posting ? 'Posting' : 'Post comment'}
      </button>
    </div>
  </div>
);

export const TicketQuestions: React.FC<TicketQuestionsProps> = ({ ticketKey, brief, lead, askTo, onAnswer, onDefaults, onAskPreview, onAskPost }) => {
  const [ask, setAsk] = useState<Ask>({ s: 'idle' });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const qs = brief.questions.slice(0, 3);
  if (qs.length === 0) return null;
  const open = openQuestions(brief);
  const asked = qs.map((q) => q.asked_on_jira).find(Boolean) ?? null;
  const askedTo = asked?.to ?? askTo;

  const run = async (fn: () => Promise<void> | void) => {
    setBusy(true);
    setErr(null);
    try {
      await fn();
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const openPreview = async () => {
    if (!onAskPreview) return;
    setAsk({ s: 'loading' });
    try {
      setAsk({ s: 'open', p: await onAskPreview() });
    } catch (e) {
      setAsk({ s: 'error', p: null, message: errText(e) });
    }
  };
  const post = async (p: AskPreview) => {
    if (!onAskPost) return;
    setAsk({ s: 'posting', p });
    try {
      await onAskPost(p);
      setAsk({ s: 'idle' });
    } catch (e) {
      setAsk({ s: 'error', p, message: errText(e) });
    }
  };

  const preview = ask.s === 'open' || ask.s === 'posting' || (ask.s === 'error' && ask.p) ? (ask as { p: AskPreview }).p : null;

  return (
    <section className="tk-qs" aria-label={`${lead}'s questions`}>
      <header>
        <h4>
          <HelpCircle size={13} aria-hidden="true" />
          {lead}’s questions
        </h4>
        <span className="note">At most 3. Each one blocks the build.</span>
      </header>
      <ol>
        {qs.map((q, i) => (
          <Question key={q.id} q={q} n={i + 1} busy={busy} onPick={onAnswer ? (v) => run(() => onAnswer(q.id, v)) : undefined} />
        ))}
      </ol>
      {err && (
        <p className="tk-err" role="alert">
          {err}
        </p>
      )}
      {preview && (
        <Preview
          p={preview}
          ticketKey={ticketKey}
          posting={ask.s === 'posting'}
          error={ask.s === 'error' ? ask.message : null}
          onCancel={() => setAsk({ s: 'idle' })}
          onPost={() => post(preview)}
        />
      )}
      {!preview && open.length > 0 && (
        <footer>
          {asked ? (
            <span className="note">
              Asked {askedTo ?? 'them'} on Jira{clock(asked.at) ? ` at ${clock(asked.at)}` : ''}. You can still answer here. {lead} checks the Jira comment every 5
              minutes. The first answer wins, here or there.
            </span>
          ) : (
            <span className="note">
              {onAskPreview ? `Or let ${askTo ?? 'the reporter'} answer: kwant bots post one comment on the ticket.` : 'Answer here or in ' + lead + '’s chat.'}
            </span>
          )}
          {ask.s === 'error' && !ask.p && (
            <p className="tk-err" role="alert">
              {ask.message}
            </p>
          )}
          <span className="acts">
            {onDefaults && (
              <button type="button" className="tk-btn ghost" disabled={busy} onClick={() => run(onDefaults)}>
                Go with the defaults
              </button>
            )}
            {asked
              ? asked.comment_url && (
                  <a className="tk-btn ghost" href={asked.comment_url} target="_blank" rel="noreferrer noopener">
                    See the comment
                    <ExternalLink size={12} aria-hidden="true" />
                    <span className="sr-only"> (opens in a new tab)</span>
                  </a>
                )
              : onAskPreview &&
                onAskPost && (
                  <button type="button" className="tk-btn primary" disabled={busy || ask.s === 'loading'} aria-busy={ask.s === 'loading' || undefined} onClick={openPreview} data-ask>
                    {ask.s === 'loading' && <Loader2 size={14} className="tk-spin" aria-hidden="true" />}
                    Ask on Jira
                  </button>
                )}
          </span>
        </footer>
      )}
    </section>
  );
};
