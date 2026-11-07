// 0.7.6 "Work from Jira": one ticket card that grows through its states (design.md §3).
// Ticket text is untrusted: every field is a React text node. A Jira link is a link only on the saved site
// (api-decisions 18); the PR link is https only. `work` carries ids (api-decisions 11): names come from `names`.
// The holo "MATCHES the ticket" sticker shows only on Landed when every done-when line is met.

import React, { useState } from 'react';
import { ArrowUp, ArrowDown, Minus, Check, ExternalLink, GitBranch, Loader2, Ticket as TicketIcon, FileText, Users, AlertTriangle } from 'lucide-react';
import type { AskPosted, AskPreview, Brief, ReadStep, Ticket, TicketNames, TicketProgress } from '../../dev/tickets';
import { READ_LABEL, READ_STEPS, closedWhy, earnsMatches, onSite, openQuestions, workView } from '../../dev/tickets';
import { clock } from '../../dev/logic';
import { SiteOrigins, TicketBrief } from './TicketBrief';
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
const andList = (ns: string[]) => (ns.length <= 1 ? ns[0] ?? '' : `${ns.slice(0, -1).join(', ')} and ${ns[ns.length - 1]}`);

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
      return { at: t.work?.branch ? 3 : 2, s: 'paused' };
  }
}

const Rail: React.FC<{ t: Ticket }> = ({ t }) => {
  const r = railFor(t);
  const steps = RAIL.map((label, i): { label: string; s: RailS } => {
    if (!r) return { label, s: 'todo' };
    if (i < r.at) return { label, s: 'done' };
    // design §3: while the questions or the brief wait on the person, step 3 reads "Needs you".
    if (i === r.at) return { label: r.s === 'paused' ? 'Paused' : r.s === 'needs' ? 'Needs you' : label, s: r.s };
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

/** One checklist row: the server's detail when it sent one, else the fixed words; the status on the right. */
const Chk: React.FC<{ step: ReadStep; p: TicketProgress; repo: string | null }> = ({ step, p, repo }) => {
  const s = p[step];
  const cls = s.state === 'done' ? 'ok' : s.state === 'running' ? 'run' : 'wait';
  const word = s.state === 'done' ? 'done' : s.state === 'running' ? 'reading' : s.state === 'skipped' ? 'skipped' : 'waiting';
  const words = s.detail ?? (step === 'code' && repo ? `The code in ${repo}` : READ_LABEL[step]);
  return (
    <li className={cls} data-step={step} data-state={s.state}>
      <span className="m" aria-hidden="true">
        {s.state === 'done' ? <Check size={12} /> : s.state === 'running' ? <Loader2 size={12} className="tk-spin" /> : s.state === 'skipped' ? <Minus size={12} /> : <span className="dot" />}
      </span>
      <span className="tx">{words}</span>
      <span className="st">
        <span className="sr-only">: </span>
        {s.note ?? word}
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
  /** Bot and team names for the ids in `work`. */
  names?: TicketNames;
  /** The saved Jira and Confluence origins (api-decisions 18). Without them no Jira link is a link. */
  origins?: readonly string[];
  /** The repo's short name for "The code in storefront". */
  repo?: string | null;
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

const JiraLink: React.FC<{ t: Ticket; origins: readonly string[]; className?: string; children?: React.ReactNode }> = ({ t, origins, className = 'tk-btn ghost', children }) => {
  const url = onSite(t.url, origins);
  return url ? (
    <a className={className} href={url} target="_blank" rel="noreferrer noopener">
      {children ?? 'Open in Jira'}
      <ExternalLink size={12} aria-hidden="true" />
      <span className="sr-only"> (opens {t.key} in Jira in a new tab)</span>
    </a>
  ) : null;
};

/** "closed in Jira as Won't do · Nov 5 · branch kept": the same words on the row and the open card (item 17). */
export const closedLine = (t: Ticket): string =>
  [closedWhy(t), shortDay(t.jira_updated ?? t.updated_at), t.work?.branch ? 'branch kept' : null].filter(Boolean).join(' · ');

/** The 8 px dot before the status title (item 13). */
const DOT: Record<Ticket['state'], 'run' | 'needs' | 'ok' | 'off' | 'bad'> = {
  waiting: 'off',
  picked_up: 'run',
  reading: 'run',
  needs_answer: 'needs',
  brief_ready: 'needs',
  working: 'run',
  in_review: 'run',
  landed: 'ok',
  taken_back: 'off',
  failed: 'bad',
  closed: 'off',
};

/** The one-line Closed row (design §3, 13): key · title · "closed in Jira as Won't do · Nov 5 · branch kept". */
const ClosedRow: React.FC<{ t: Ticket; onShow: () => void }> = ({ t, onShow }) => {
  const bits = closedLine(t);
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
  const { ticket: t, brief = null, briefText = null, progress = null, handoverLabel = null, names = {}, origins = [], repo = null } = props;
  const [expanded, setExpanded] = useState(false);
  const [briefShown, setBriefShown] = useState(false);
  if (t.state === 'closed' && !expanded) return <ClosedRow t={t} onShow={() => setExpanded(true)} />;

  const w = workView(t, names);
  const lead = w.lead ?? 'The lead';
  const leadIn = w.lead ?? 'the lead';
  const briefId = `tk-${t.key}-brief`;
  const hasBrief = !!(brief || briefText);
  /** "Brief" in the Landed footer and Taken back's Kept line: shows the brief under the card (items 7, 16). */
  const BriefLink: React.FC<{ chip?: boolean }> = ({ chip }) =>
    hasBrief ? (
      <button type="button" className={chip ? 'tk-src' : undefined} aria-expanded={briefShown} aria-controls={briefId} onClick={() => setBriefShown((v) => !v)} data-brief-link>
        {chip && <FileText aria-hidden="true" />}
        Brief
      </button>
    ) : null;
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
      const who = w.builders.length ? andList(w.builders) : lead;
      bold = `${who} ${w.builders.length > 1 ? 'are' : 'is'} building`;
      const bits: React.ReactNode[] = [];
      if (w.branch)
        bits.push(
          <>
            On <code className="dv-branch">{w.branch}</code>
          </>,
        );
      if (w.plan) bits.push(`${w.plan.done} of ${w.plan.total} plan steps`);
      line = bits.length ? join(bits) : 'Building from the brief.';
      break;
    }
    case 'in_review': {
      bold = `${w.reviewer ?? 'The reviewer'} is reviewing the pull request`;
      const m = w.matches;
      const pr = w.pr ? (w.pr.number !== null ? `#${w.pr.number}` : 'The pull request') : null;
      line = (
        <>
          {pr}
          {m && (
            <>
              {pr ? ' · ' : ''}Matches the ticket:{' '}
              <b className={m === 'met' ? 'tk-ok' : 'tk-warn'} data-matches={m}>
                {m === 'met' ? 'every line met' : m === 'not_met' ? 'not every line met yet' : 'unclear so far'}
              </b>
            </>
          )}
          {!pr && !m && 'The review will check it against the brief.'}
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
      const n = brief?.done_when.length ?? 0;
      line =
        m === 'met' ? (
          `${n > 0 ? `All ${n} done-when lines met` : 'Every done-when line met'}. Jira wasn’t changed: move the ticket when you’re happy.`
        ) : m ? (
          <>
            Not every done-when line is met.{' '}
            {props.onOpenReview ? (
              <button type="button" className="tk-inl" onClick={props.onOpenReview} data-see-review>
                See the review
              </button>
            ) : (
              'See the review.'
            )}
          </>
        ) : (
          'Jira wasn’t changed: move the ticket when you’re happy.'
        );
      acts = <JiraLink t={t} origins={origins} />;
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
          <JiraLink t={t} origins={origins} />
        </>
      );
      break;
    }
    case 'failed':
      card = 'bad';
      tone = 'bad';
      // api-decisions 22: the row says why in kwant's own words (plain text); the old copy is the fallback.
      bold = t.error && (t.error.code === 'work_failed' || t.error.code === 'failed') ? `Work on ${t.key} stopped` : `${lead} couldn’t read ${t.key}`;
      line = t.error?.message ?? 'Jira didn’t answer. Nothing was built.';
      acts = (
        <>
          <Btn primary onClick={props.onRetry}>
            Try again
          </Btn>
          <JiraLink t={t} origins={origins} />
        </>
      );
      break;
    case 'closed': {
      card = 'waiting';
      bold = t.closed_resolution === 'skipped' ? 'Skipped' : t.closed_resolution === 'deleted' ? 'Deleted in Jira' : 'Closed';
      line = closedLine(t);
      acts = (
        <>
          <Btn onClick={() => setExpanded(false)}>Hide</Btn>
          <JiraLink t={t} origins={origins} />
        </>
      );
      break;
    }
  }

  const iconTone = card === 'needs' ? 'needs' : card === 'landed' ? 'ok' : card === 'waiting' || card === 'back' ? 'muted' : card === 'bad' ? 'bad' : 'plain';
  const handed = clock(t.handed_over_at);
  const waitSince = t.waiting_since_connect ? shortDay(t.jira_created) : null;
  const showJiraTop = t.state !== 'landed' && t.state !== 'taken_back' && t.state !== 'failed' && t.state !== 'closed';
  // Landed and Taken back show the brief only from their "Brief" link (items 7, 16).
  const linkBrief = t.state === 'landed' || t.state === 'taken_back';
  const showBrief = hasBrief && (['needs_answer', 'brief_ready', 'working', 'in_review'].includes(t.state) || (linkBrief && briefShown));
  const briefOpen = t.state === 'brief_ready';

  return (
 <SiteOrigins.Provider value={origins}>
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
                  {t.handed_over_by === 'assignee' && <span className="opt">assigned to your bots</span>}
                </>
              )
            )}
          </p>
        </div>
        {showJiraTop && <JiraLink t={t} origins={origins} className="tk-open" />}
      </header>

      <Rail t={t} />

      <div className="tk-status" data-tone={tone} aria-live="polite">
        <div className="tx">
          <p className="b">
            {tone === 'bad' ? <AlertTriangle size={14} aria-hidden="true" /> : <span className="tk-sdot" data-d={DOT[t.state]} aria-hidden="true" />}
            {bold}
          </p>
          <p className="l">{line}</p>
        </div>
        {acts && <div className="acts">{acts}</div>}
      </div>

      {t.state === 'reading' && progress && (
        <ul className="tk-read" aria-label="What is being read" aria-live="polite">
          {READ_STEPS.map((s) => (
            <Chk key={s} step={s} p={progress} repo={repo} />
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

      {t.state === 'landed' && <LandedFoot w={w} brief={<BriefLink />} />}

      {t.state === 'taken_back' && (w.branch || hasBrief) && (
        <p className="tk-links">
          <span className="m">Kept:</span>
          {w.branch && (
            <span className="tk-src f">
              <GitBranch aria-hidden="true" />
              {w.branch}
            </span>
          )}
          {w.branch && hasBrief && (
            <span className="m" aria-hidden="true">
              ·
            </span>
          )}
          <BriefLink chip />
        </p>
      )}

      {showBrief && (
        <div id={briefId}>
          <TicketBrief key={`${t.key}-${t.state}`} brief={brief} fallbackText={briefText} lead={w.lead} defaultOpen={briefOpen || linkBrief} />
        </div>
      )}
      {linkBrief && !showBrief && <div id={briefId} hidden />}

      {(t.state === 'working' || t.state === 'in_review') && w.team && (
        <p className="tk-links">
          {w.team && (
            <span className="m">
              <Users size={12} aria-hidden="true" />
              {w.team}
            </span>
          )}
        </p>
      )}
    </article>
    </SiteOrigins.Provider>
  );
};

/** Landed's footer (item 7): "#61 merged 11:20 · Brief · Built by Ann and Ben · reviewed by Cody". */
const LandedFoot: React.FC<{ w: ReturnType<typeof workView>; brief: React.ReactNode }> = ({ w, brief }) => {
  const at = clock(w.mergedAt);
  const builders = w.builders.length ? w.builders : w.lead ? [w.lead] : [];
  const bits: React.ReactNode[] = [];
  if (w.pr)
    bits.push(
      <span>
        <a href={w.pr.url} target="_blank" rel="noreferrer noopener" data-pr-link>
          {w.pr.number !== null ? `#${w.pr.number}` : 'Pull request'}
          <span className="sr-only"> (opens in a new tab)</span>
        </a>
        {at ? ` merged ${at}` : ' merged'}
      </span>,
    );
  if (brief) bits.push(brief);
  if (builders.length) bits.push(<span>Built by {andList(builders)}</span>);
  if (w.reviewer) bits.push(<span>reviewed by {w.reviewer}</span>);
  if (!bits.length) return null;
  return (
    <p className="tk-foot" data-landed-foot>
      {bits.map((b, i) => (
        <React.Fragment key={i}>
          {i > 0 && (
            <span className="sep" aria-hidden="true">
              ·
            </span>
          )}
          {b}
        </React.Fragment>
      ))}
    </p>
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

