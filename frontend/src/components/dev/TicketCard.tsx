// 0.7.6 "Work from Jira": one ticket card that grows through its states (design.md §3).
// Ticket text is untrusted: every field is a React text node; links are https only (parseTicket checks).
// The holo "MATCHES the ticket" sticker shows only on Landed when every done-when line is met.

import React, { useState } from 'react';
import { ArrowUp, ArrowDown, Minus, Check, ExternalLink, GitBranch, Loader2, Ticket as TicketIcon, FileText, Users, AlertTriangle } from 'lucide-react';
import type { AskPosted, AskPreview, Brief, ReadStep, Ticket, TicketProgress } from '../../dev/tickets';
import { READ_LABEL, READ_STEPS, earnsMatches, openQuestions } from '../../dev/tickets';
import { clock } from '../../dev/logic';
import { TicketBrief } from './TicketBrief';
import { TicketQuestions } from './TicketQuestions';

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
/** "Nov 5" in the viewer's zone, or null. */
export const shortDay = (iso: string | null): string | null => {
  if (!iso) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : `${MONTHS[d.getMonth()]} ${d.getDate()}`;
};

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join('');

/** "Ann and Ben", "Ann, Ben and Cy". */
const names = (ns: string[]) => (ns.length <= 1 ? ns[0] ?? '' : `${ns.slice(0, -1).join(', ')} and ${ns[ns.length - 1]}`);

const RAIL = ['Picked up', 'Reading', 'Brief', 'Working', 'In review', 'Landed'] as const;
type RailS = 'done' | 'now' | 'needs' | 'paused' | 'bad' | 'todo';
const LED: Record<RailS, string> = { done: 'ok', now: 'run', needs: 'warn', paused: 'off', bad: 'bad', todo: 'off' };

/** Where the card sits on the 6-step rail, and how the current step looks. */
export function railFor(t: Ticket): { at: number; s: RailS } | null {
  switch (t.state) {
    case 'waiting':
    case 'closed':
      return null;
    case 'picked_up':
      return { at: 0, s: 'now' };
    case 'reading':
      return { at: 1, s: 'now' };
    case 'failed':
      return { at: 1, s: 'bad' };
    case 'needs_answer':
    case 'brief_ready':
      return { at: 2, s: 'needs' };
    case 'working':
      return { at: 3, s: 'now' };
    case 'in_review':
      return { at: 4, s: 'now' };
    case 'landed':
      return { at: 6, s: 'done' };
    case 'taken_back':
      return { at: t.work.branch ? 3 : 2, s: 'paused' };
  }
}

const Rail: React.FC<{ t: Ticket }> = ({ t }) => {
  const r = railFor(t);
  const steps = RAIL.map((label, i): { label: string; s: RailS } => {
    if (!r) return { label, s: 'todo' };
    if (i < r.at) return { label, s: 'done' };
    if (i === r.at) return { label: r.s === 'paused' ? 'Paused' : label, s: r.s };
    return { label, s: 'todo' };
  });
  const cur = steps.find((x) => x.s !== 'done' && x.s !== 'todo');
  const said = r === null ? 'Not started' : r.at >= RAIL.length ? 'All steps done' : `Step ${r.at + 1} of ${RAIL.length}: ${cur?.label ?? ''}`;
  return (
    <ol className="tk-rail" aria-label={`Progress. ${said}`}>
      {steps.map((x, i) => (
        <li key={i} data-s={x.s} aria-current={x.s !== 'done' && x.s !== 'todo' ? 'step' : undefined}>
          <span className="dv-led" data-s={LED[x.s]} aria-hidden="true" />
          <span className="lb">{x.label}</span>
        </li>
      ))}
    </ol>
  );
};

const Priority: React.FC<{ p: string }> = ({ p }) => {
  const lo = p.toLowerCase();
  const high = lo === 'high' || lo === 'highest' || lo === 'critical' || lo === 'blocker';
  const low = lo === 'low' || lo === 'lowest';
  return (
    <span className={high ? 'tk-pri hi' : 'tk-pri'}>
      {high ? <ArrowUp size={12} aria-hidden="true" /> : low ? <ArrowDown size={12} aria-hidden="true" /> : <Minus size={12} aria-hidden="true" />}
      {p}
      <span className="sr-only"> priority</span>
    </span>
  );
};

const Chk: React.FC<{ step: ReadStep; p: TicketProgress }> = ({ step, p }) => {
  const s = p[step];
  const cls = s.state === 'done' ? 'ok' : s.state === 'running' ? 'run' : 'wait';
  const word = s.state === 'done' ? 'done' : s.state === 'running' ? 'reading now' : s.state === 'skipped' ? 'skipped' : 'waiting';
  return (
    <li className={cls} data-step={step} data-state={s.state}>
      <span className="m" aria-hidden="true">
        {s.state === 'done' ? <Check size={12} /> : s.state === 'running' ? <Loader2 size={12} className="tk-spin" /> : s.state === 'skipped' ? <Minus size={12} /> : <span className="dot" />}
      </span>
      <span>
        {READ_LABEL[step]}
        {s.detail && <em>, {s.detail}</em>}
        <span className="sr-only">: {word}</span>
      </span>
    </li>
  );
};

export interface TicketCardProps {
  ticket: Ticket;
  brief?: Brief | null;
  /** The brief route's markdown, shown as plain text when the brief can't be read. */
  briefText?: string | null;
  /** The live reading checklist (SSE `dev_ticket_progress`). */
  progress?: TicketProgress | null;
  /** The label that hands tickets over ("kwant"); from the site settings. Falls back to "the label". */
  handoverLabel?: string | null;
  /** Each missing handler hides its button. */
  onStart?: () => void;
  onSkip?: () => void;
  onStop?: () => void;
  onRetry?: () => void;
  onCloseCard?: () => void;
  onStartBuilding?: () => void;
  /** Opens the lead's chat with the brief quoted (Change something). */
  onChangeBrief?: () => void;
  onOpenChat?: () => void;
  onOpenReview?: () => void;
  onAnswer?: (questionId: string, value: string) => Promise<void> | void;
  onDefaults?: () => Promise<void> | void;
  onAskPreview?: () => Promise<AskPreview>;
  onAskPost?: (p: AskPreview) => Promise<AskPosted>;
}

type Tone = 'needs' | 'ok' | 'bad' | 'plain';

const Btn: React.FC<{ primary?: boolean; onClick?: () => void; children: React.ReactNode; label?: string }> = ({ primary, onClick, children, label }) =>
  onClick ? (
    <button type="button" className={primary ? 'tk-btn primary' : 'tk-btn ghost'} onClick={onClick} aria-label={label}>
      {children}
    </button>
  ) : null;

const JiraLink: React.FC<{ t: Ticket; className?: string; children?: React.ReactNode }> = ({ t, className = 'tk-btn ghost', children }) =>
  t.url ? (
    <a className={className} href={t.url} target="_blank" rel="noreferrer noopener">
      {children ?? 'Open in Jira'}
      <ExternalLink size={12} aria-hidden="true" />
      <span className="sr-only"> (opens {t.key} in Jira in a new tab)</span>
    </a>
  ) : null;

/** The one-line Closed row (design §3, 13): key · title · "closed in Jira as Won't do · Nov 5 · branch kept". */
const ClosedRow: React.FC<{ t: Ticket; onShow: () => void }> = ({ t, onShow }) => {
  const res = t.closed_resolution;
  const how = !res ? 'closed in Jira' : res === 'deleted' ? 'deleted in Jira' : `closed in Jira as ${res.replace(/^done:\s*/i, '') || 'Done'}`;
  const bits = [how, shortDay(t.jira_updated ?? t.updated_at), t.work.branch ? 'branch kept' : null].filter(Boolean).join(' · ');
  return (
    <div className="tk-mini" data-ticket={t.key} data-state="closed">
      <span className="tk-key">{t.key}</span>
      <span className="t">{t.summary}</span>
      <span className="m">{bits}</span>
      <button type="button" className="tk-textbtn" onClick={onShow} aria-label={`Show ${t.key}`}>
        Show
      </button>
    </div>
  );
};

export const TicketCard: React.FC<TicketCardProps> = (props) => {
  const { ticket: t, brief = null, briefText = null, progress = null, handoverLabel = null } = props;
  const [expanded, setExpanded] = useState(false);
  if (t.state === 'closed' && !expanded) return <ClosedRow t={t} onShow={() => setExpanded(true)} />;

  const w = t.work;
  const lead = w.lead ?? 'The lead';
  const leadIn = w.lead ?? 'the lead';
  const reporter = t.reporter_name ?? t.assignee_name;
  const label = handoverLabel ?? (t.labels.length === 1 ? t.labels[0]! : null);
  const nQ = brief ? openQuestions(brief).length : 0;
  const asked = brief?.questions.map((q) => q.asked_on_jira).find(Boolean) ?? null;

  let tone: Tone = 'plain';
  let bold: React.ReactNode = null;
  let line: React.ReactNode = null;
  let acts: React.ReactNode = null;
  let card: 'needs' | 'landed' | 'back' | 'waiting' | 'active' | 'bad' = 'active';

  switch (t.state) {
    case 'waiting':
      card = 'waiting';
      bold = 'Waiting for your Start';
      line = t.waiting_since_connect ? 'It was open before you connected Jira. New tickets start on their own.' : 'It waits until you start it.';
      acts = (
        <>
          <Btn onClick={props.onSkip}>Skip</Btn>
          <Btn primary onClick={props.onStart}>
            Start
          </Btn>
        </>
      );
      break;
    case 'picked_up':
      bold = `${lead} picked it up`;
      line = 'Reading starts in a moment. Nothing is built yet.';
      acts = <Btn onClick={props.onStop}>Stop</Btn>;
      break;
    case 'reading':
      bold = `${lead} is reading everything about it`;
      line = 'About a minute. Nothing is built yet.';
      acts = (
        <>
          <Btn onClick={props.onOpenChat}>{w.lead ? `Open ${w.lead}’s chat` : 'Open the chat'}</Btn>
          <Btn onClick={props.onStop}>Stop</Btn>
        </>
      );
      break;
    case 'needs_answer':
      card = 'needs';
      tone = 'needs';
      if (asked) {
        bold = `Waiting for ${asked.to ?? reporter ?? 'their'}${asked.to || reporter ? '’s' : ''} answer on Jira`;
        line = 'Or answer here; the first answer wins.';
      } else if (nQ > 0) {
        bold = `${nQ} ${nQ === 1 ? 'question' : 'questions'} before ${leadIn} can build`;
        line = `Answer here, go with the defaults, or ask ${reporter ?? 'the reporter'} on Jira.`;
      } else {
        bold = `${lead} has a question before building`;
        line = `Answer it in ${w.lead ? `${w.lead}’s` : 'the lead’s'} chat.`;
      }
      break;
    case 'brief_ready':
      card = 'needs';
      tone = 'needs';
      bold = 'Brief ready. Start building?';
      line = `You asked to OK every brief first. ${lead} waits.`;
      acts = (
        <>
          <Btn onClick={props.onChangeBrief}>Change something</Btn>
          <Btn primary onClick={props.onStartBuilding}>
            Start building
          </Btn>
        </>
      );
      break;
    case 'working': {
      const who = w.builders.length ? names(w.builders) : lead;
      bold = `${who} ${w.builders.length > 1 ? 'are' : 'is'} building`;
      const bits: React.ReactNode[] = [];
      if (w.branch)
        bits.push(
          <>
            On <code className="dv-branch">{w.branch}</code>
          </>,
        );
      if (w.plan) bits.push(`${w.plan.done} of ${w.plan.total} plan steps`);
      if (clock(w.started_at)) bits.push(`started ${clock(w.started_at)}`);
      line = bits.length ? join(bits) : 'Building from the brief.';
      break;
    }
    case 'in_review': {
      bold = `${w.reviewer ?? 'The reviewer'} is reviewing the pull request`;
      const m = w.matches;
      const all = !!m && m.met === m.total;
      line = (
        <>
          {w.pr && (
            <>
              #{w.pr.number}
              {w.pr.title ? ` ${w.pr.title}` : ''}
            </>
          )}
          {m && (
            <>
              {w.pr ? ' · ' : ''}Matches the ticket:{' '}
              <b className={all ? 'tk-ok' : 'tk-warn'} data-matches={`${m.met}/${m.total}`}>
                {m.met} of {m.total} met
              </b>
            </>
          )}
          {!w.pr && !m && 'The review will check it against the brief.'}
        </>
      );
      acts = (
        <Btn primary onClick={props.onOpenReview}>
          Open the review
        </Btn>
      );
      break;
    }
    case 'landed': {
      card = 'landed';
      tone = 'ok';
      bold = w.sha ? (
        <>
          Landed on main <code className="dv-sha">{w.sha.slice(0, 7)}</code>
        </>
      ) : (
        'Landed on main'
      );
      const m = w.matches;
      line = !m
        ? 'Jira wasn’t changed: move the ticket when you’re happy.'
        : m.met === m.total
          ? `All ${m.total} done-when lines met. Jira wasn’t changed: move the ticket when you’re happy.`
          : `${m.met} of ${m.total} done-when lines met. See the review.`;
      acts = <JiraLink t={t} />;
      break;
    }
    case 'taken_back': {
      card = 'back';
      bold = t.taken_back_by ? `${t.taken_back_by} took it back in Jira` : 'Taken back in Jira';
      const how =
        t.handed_over_by === 'label'
          ? `Removed the label ${label ?? ''}`.trimEnd()
          : t.handed_over_by === 'assignee'
            ? 'Assigned it to someone else'
            : 'It left the ticket search';
      const at = clock(t.taken_back_at);
      line = `${how}${at ? ` at ${at}` : ''}. Work paused; nothing was thrown away. It carries on if the ticket comes back.`;
      acts = (
        <>
          <Btn onClick={props.onCloseCard}>Close card</Btn>
          <JiraLink t={t} />
        </>
      );
      break;
    }
    case 'failed':
      card = 'bad';
      tone = 'bad';
      bold = `${lead} couldn’t read ${t.key}`;
      line = w.error ?? 'Jira didn’t answer. Nothing was built.';
      acts = (
        <>
          <Btn primary onClick={props.onRetry}>
            Try again
          </Btn>
          <JiraLink t={t} />
        </>
      );
      break;
    case 'closed': {
      bold = 'Closed in Jira';
      const res = t.closed_resolution;
      line = res === 'deleted' ? 'The ticket was deleted. The card and any branch stay here.' : `Resolution: ${res?.replace(/^done:\s*/i, '') || 'Done'}. Any branch stays.`;
      acts = (
        <>
          <Btn onClick={() => setExpanded(false)}>Hide</Btn>
          <JiraLink t={t} />
        </>
      );
      break;
    }
  }

  const iconTone = card === 'needs' ? 'needs' : card === 'landed' ? 'ok' : card === 'waiting' || card === 'back' ? 'muted' : card === 'bad' ? 'bad' : 'plain';
  const handed = clock(t.handed_over_at);
  const waitSince = t.waiting_since_connect ? shortDay(t.jira_created) : null;
  const showJiraTop = t.state !== 'landed' && t.state !== 'taken_back' && t.state !== 'failed' && t.state !== 'closed';
  const showBrief = !!(brief || briefText) && ['needs_answer', 'brief_ready', 'working', 'in_review', 'landed', 'taken_back'].includes(t.state);
  const briefOpen = t.state === 'brief_ready';

  return (
    <article className="tk kw-card" data-s={card} data-ticket={t.key} data-state={t.state} aria-labelledby={`tk-${t.key}-t`}>
      {t.state === 'landed' && earnsMatches(t) && (
        <span className="tk-holo" role="img" aria-label="Earned: matches the ticket" data-holo>
          <b aria-hidden="true">MATCHES</b>
          <span aria-hidden="true">the ticket</span>
        </span>
      )}
      <header className="tk-head">
        <span className="tk-ic" data-tone={iconTone} aria-hidden="true">
          <TicketIcon size={18} />
        </span>
        <div className="tk-t">
          <h3 id={`tk-${t.key}-t`}>
            <span className="tk-key">{t.key}</span>
            <span className="sr-only">: </span>
            <span className="tx">{t.summary}</span>
          </h3>
          <p className="tk-meta">
            {t.issue_type && <span>{t.issue_type}</span>}
            {t.priority && <Priority p={t.priority} />}
            {reporter && (
              <span className="who">
                <span className="av" aria-hidden="true">
                  {initials(reporter)}
                </span>
                <span className="sr-only">{t.reporter_name ? 'Reported by ' : 'Assigned to '}</span>
                {reporter}
              </span>
            )}
            {waitSince ? (
              <span className="opt">open since {waitSince}, before you connected</span>
            ) : (
              handed && (
                <>
                  <span className="opt">handed over {handed}</span>
                  {t.handed_over_by === 'label' && (
                    <span className="opt">
                      with the label <code>{label ?? 'the label'}</code>
                    </span>
                  )}
                  {t.handed_over_by === 'assignee' && <span className="opt">assigned to kwant bots</span>}
                </>
              )
            )}
          </p>
        </div>
        {showJiraTop && <JiraLink t={t} className="tk-open" />}
      </header>

      <Rail t={t} />

      <div className="tk-status" data-tone={tone} aria-live="polite">
        <div className="tx">
          <p className="b">
            {tone === 'bad' && <AlertTriangle size={14} aria-hidden="true" />}
            {bold}
          </p>
          <p className="l">{line}</p>
        </div>
        {acts && <div className="acts">{acts}</div>}
      </div>

      {t.state === 'reading' && progress && (
        <ul className="tk-read" aria-label="What is being read" aria-live="polite">
          {READ_STEPS.map((s) => (
            <Chk key={s} step={s} p={progress} />
          ))}
        </ul>
      )}

      {t.state === 'needs_answer' && brief && (
        <TicketQuestions
          ticketKey={t.key}
          brief={brief}
          lead={w.lead ?? 'The lead'}
          askTo={reporter}
          onAnswer={props.onAnswer}
          onDefaults={props.onDefaults}
          onAskPreview={props.onAskPreview}
          onAskPost={props.onAskPost}
        />
      )}

      {showBrief && <TicketBrief key={`${t.key}-${t.state}`} brief={brief} fallbackText={briefText} lead={w.lead} defaultOpen={briefOpen} />}

      {t.state === 'taken_back' && (w.branch || brief) && (
        <p className="tk-links">
          <span className="m">Kept:</span>
          {w.branch && (
            <span className="tk-src f">
              <GitBranch aria-hidden="true" />
              {w.branch}
            </span>
          )}
          {brief && (
            <span className="tk-src">
              <FileText aria-hidden="true" />
              Brief
            </span>
          )}
        </p>
      )}

      {(t.state === 'working' || t.state === 'in_review') && (w.task_url || w.team) && (
        <p className="tk-links">
          {w.task_url && (
            <a className="tk-link" href={w.task_url} target="_blank" rel="noreferrer noopener">
              Open the task
              <ExternalLink size={12} aria-hidden="true" />
              <span className="sr-only"> (opens in a new tab)</span>
            </a>
          )}
          {w.team && (
            <span className="m">
              <Users size={12} aria-hidden="true" />
              {w.team}
            </span>
          )}
        </p>
      )}
    </article>
  );
};

function join(bits: React.ReactNode[]): React.ReactNode {
  return bits.map((b, i) => (
    <React.Fragment key={i}>
      {i > 0 ? ' · ' : ''}
      {b}
    </React.Fragment>
  ));
}

