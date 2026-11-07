// 0.7.6 "Work from Jira": the brief on a ticket card (design.md §4) and the source chips it shares with the
// questions and "Matches the ticket". Every word here came from Jira, Confluence or a model reading them:
// React text only, never HTML or markdown; a chip is a link only when its url is https.

import React, { useId, useState } from 'react';
import { AlertTriangle, BookOpen, ChevronDown, Code2, FileText, Link2, MessageSquare, Minus, Ticket as TicketIcon } from 'lucide-react';
import type { Brief, BriefLine, BriefQuestion, SourceRef } from '../../dev/tickets';
import { briefMeta, codeParts } from '../../dev/tickets';
import { clock } from '../../dev/logic';

const ICON: Record<SourceRef['kind'], React.ReactNode> = {
  ticket: <TicketIcon aria-hidden="true" />,
  comment: <MessageSquare aria-hidden="true" />,
  page: <BookOpen aria-hidden="true" />,
  file: <Code2 aria-hidden="true" />,
  linked_issue: <Link2 aria-hidden="true" />,
};

const KIND_WORD: Record<SourceRef['kind'], string> = { ticket: 'the ticket', comment: 'a comment', page: 'a page', file: 'a file', linked_issue: 'a linked issue' };

/** What a screen reader hears for a chip: "Source: Orders export, version 7". */
export const sourceLabel = (s: SourceRef): string => {
  if (s.kind === 'ticket') return 'Source: the ticket';
  if (s.kind === 'comment') return `Source: a comment${s.comment_author ? ` by ${s.comment_author}` : ''}`;
  if (s.kind === 'page') return `Source: ${s.label}${s.page_version !== null ? `, version ${s.page_version}` : ''}`;
  if (s.kind === 'file') return `Source: ${s.file ?? s.label}${s.line !== null && s.file ? `, line ${s.line}` : ''}`;
  return `Source: ${KIND_WORD[s.kind]}, ${s.label}`;
};

/** One source chip (design §4): ticket, comment · name, page title v7, or file:line in mono. */
export const SourceChip: React.FC<{ s: SourceRef }> = ({ s }) => {
  const cls = s.kind === 'file' ? 'tk-src f' : 'tk-src';
  const body = (
    <>
      {ICON[s.kind]}
      <span aria-hidden="true">{s.label}</span>
      {s.kind === 'page' && s.page_version !== null && (
        <span aria-hidden="true" className="v">
          v{s.page_version}
        </span>
      )}
    </>
  );
  if (s.url) {
    return (
      <a href={s.url} target="_blank" rel="noreferrer noopener" className={`${cls} kw-focus`} title={s.cite ?? undefined} data-src={s.kind}>
        {body}
        <span className="sr-only">
          {sourceLabel(s)} (opens in a new tab)
        </span>
      </a>
    );
  }
  return (
    <span className={cls} title={s.cite ?? undefined} data-src={s.kind}>
      {body}
      <span className="sr-only">{sourceLabel(s)}</span>
    </span>
  );
};

export const InferredChip: React.FC = () => (
  <span className="tk-src inf" data-src="inferred">
    <span aria-hidden="true">inferred</span>
    <span className="sr-only">Inferred, not written in the ticket</span>
  </span>
);

/** Plain text with `inline code` drawn as code chips; nothing else is read as markup. */
export const CodeText: React.FC<{ text: string }> = ({ text }) => (
  <>
    {codeParts(text).map((p, i) => (p.code ? <code key={i}>{p.text}</code> : <React.Fragment key={i}>{p.text}</React.Fragment>))}
  </>
);

const Line: React.FC<{ l: BriefLine; n?: number }> = ({ l, n }) => (
  <div className="tk-bl" data-inferred={l.inferred || undefined}>
    <span className="txt">
      {n !== undefined && (
        <span className="n" aria-hidden="true">
          {n}
        </span>
      )}
      {n !== undefined && <span className="sr-only">{n}. </span>}
      <CodeText text={l.text} />
    </span>
    {(l.inferred || l.sources.length > 0) && (
      <span className="srcs">
        {l.inferred && <InferredChip />}
        {l.sources.map((s, i) => (
          <SourceChip key={i} s={s} />
        ))}
      </span>
    )}
  </div>
);

const Section: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className="tk-bsec">
    <dt>{label}</dt>
    <dd>{children}</dd>
  </div>
);

/** "CSV" or "Dana answered on Jira: CSV": the brief's Questions row, one sentence per answered question. */
const answeredLine = (qs: BriefQuestion[]) =>
  qs
    .filter((q) => q.answer)
    .map((q) => q.answer!.value)
    .join('. ');

export interface TicketBriefProps {
  brief: Brief | null;
  /** The brief route's markdown, shown as plain text when the brief itself couldn't be read. */
  fallbackText?: string | null;
  /** The lead's name for "by Ann" and "Ann won't act on it"; falls back to the brief's author. */
  lead?: string | null;
  defaultOpen?: boolean;
}

/** The brief, open or collapsed. Collapsed: "BRIEF Ann · 10:03 · 6 parts, 11 sources" and Show brief. */
export const TicketBrief: React.FC<TicketBriefProps> = ({ brief, fallbackText = null, lead = null, defaultOpen = false }) => {
  const [open, setOpen] = useState(defaultOpen);
  const bodyId = useId();
  if (!brief && !fallbackText) return null;
  const who = brief?.author_bot ?? lead ?? null;
  const when = clock(brief?.created_at ?? null);
  const edited = clock(brief?.edited_by_person_at ?? null);
  const meta = brief ? briefMeta(brief) : null;
  const by = open ? [who ? `by ${who}` : null, when, edited ? `edited by you · ${edited}` : null] : [who, when, meta];
  const answered = brief ? answeredLine(brief.questions) : '';
  const lastAnswer = brief?.questions.map((q) => q.answer).filter(Boolean).pop() ?? null;

  return (
    <section className="tk-brief" aria-label="Brief" data-brief={open ? 'open' : 'closed'}>
      <header>
        <h4>
          <FileText size={12} aria-hidden="true" />
          Brief
        </h4>
        <span className="by">{by.filter(Boolean).join(' · ')}</span>
        <button type="button" className="tk-textbtn" aria-expanded={open} aria-controls={bodyId} onClick={() => setOpen((o) => !o)}>
          {open ? 'Hide' : 'Show brief'}
          {open ? <Minus size={12} aria-hidden="true" /> : <ChevronDown size={12} aria-hidden="true" />}
        </button>
      </header>
      <div id={bodyId} hidden={!open}>
        {open && brief && (
          <>
            <dl>
              {brief.goal && (
                <Section label="Goal">
                  <Line l={brief.goal} />
                </Section>
              )}
              {brief.done_when.length > 0 && (
                <Section label="Done when">
                  {brief.done_when.map((l, i) => (
                    <Line key={i} l={l} n={i + 1} />
                  ))}
                </Section>
              )}
              {brief.where.length > 0 && (
                <Section label="Where in the code">
                  {brief.where.map((l, i) => (
                    <Line key={i} l={l} />
                  ))}
                </Section>
              )}
              {brief.plan.length > 0 && (
                <Section label="Plan">
                  {brief.plan.map((l, i) => (
                    <Line key={i} l={l} n={i + 1} />
                  ))}
                </Section>
              )}
              {brief.assumptions.length > 0 && (
                <Section label="Assumptions">
                  {brief.assumptions.map((l, i) => (
                    <Line key={i} l={l} />
                  ))}
                </Section>
              )}
              {answered && (
                <Section label="Questions">
                  <div className="tk-bl">
                    <span className="txt">
                      <span className="tk-ok">Answered:</span> {answered}.
                    </span>
                    {lastAnswer && (
                      <span className="srcs">
                        <span className="tk-src">
                          {lastAnswer.by === 'jira_user' ? `${lastAnswer.name ?? 'Jira'} on Jira` : 'you'}
                          {clock(lastAnswer.at) ? ` · ${clock(lastAnswer.at)}` : ''}
                        </span>
                      </span>
                    )}
                  </div>
                </Section>
              )}
            </dl>
            {brief.flags.map((f, i) => (
              <div key={i} className="tk-flag" role="note" data-flag>
                <AlertTriangle size={14} aria-hidden="true" />
                <span>
                  <b>Ignored part of {f.from?.kind === 'comment' ? 'a comment' : f.from?.kind === 'page' ? 'a page' : 'the ticket'}.</b> <q>{f.quote}</q>{' '}
                  {f.why ?? 'reads like an order to a bot, not a requirement.'} {who ?? 'The lead'} won’t act on it.
                  {f.from?.url && (
                    <>
                      {' '}
                      <a href={f.from.url} target="_blank" rel="noreferrer noopener" className="tk-link kw-focus">
                        Show where<span className="sr-only"> (opens in a new tab)</span>
                      </a>
                    </>
                  )}
                </span>
              </div>
            ))}
          </>
        )}
        {open && !brief && fallbackText && <pre className="tk-brief-text">{fallbackText}</pre>}
      </div>
    </section>
  );
};
