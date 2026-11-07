// 0.7.6 "Work from Jira": the Dev desk's Tickets tab (design.md §3 "Where"). It exists only while Jira is
// connected: `useTickets` reads GET …/dev/jira first and reports `site: null` otherwise, so a space without
// Jira looks exactly as it did in 0.7.5. The count is tickets that need the person (open questions or a
// brief waiting for OK), never news.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Settings } from 'lucide-react';
import type { Brief, JiraSite, Ticket, TicketNames, TicketProgress } from '../../dev/tickets';
import { applyProgress, emptyProgress, orderTickets, siteOrigins, ticketsNeedCount, workView } from '../../dev/tickets';
import * as api from '../../dev/ticketApi';
import { TicketCard } from './TicketCard';
import { DevModeError } from '../../devMode';

/** App re-sends the `dev_ticket` and `dev_ticket_progress` SSE messages as these window events. */
export const DEV_TICKET_EVENT = 'kwant:dev-ticket';
export const DEV_TICKET_PROGRESS_EVENT = 'kwant:dev-ticket-progress';

const BRIEF_STATES = new Set<Ticket['state']>(['needs_answer', 'brief_ready', 'working', 'in_review', 'landed', 'taken_back']);

export interface TicketsData {
  site: JiraSite | null;
  tickets: Ticket[];
  dropped: number;
  loading: boolean;
  error: string | null;
  needs: number;
  reload: () => void;
}

/** The Jira site and its tickets for one space. `on` false (Dev mode off) reads nothing. */
export function useTickets(space: string, on: boolean): TicketsData {
  const [site, setSite] = useState<JiraSite | null>(null);
  const [list, setList] = useState<{ tickets: Ticket[]; dropped: number }>({ tickets: [], dropped: 0 });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  const reload = useCallback(() => {
    if (!on) return;
    const n = ++seq.current;
    setLoading(true);
    api
      .getJiraSite(space)
      .then(async (s) => {
        if (n !== seq.current) return;
        setSite(s);
        if (!s) return setList({ tickets: [], dropped: 0 });
        const l = await api.listTickets(space);
        if (n === seq.current) setList({ tickets: l.tickets, dropped: l.dropped });
        setError(null);
      })
      .catch((e: unknown) => n === seq.current && setError(e instanceof Error ? e.message : 'Couldn’t read the tickets.'))
      .finally(() => n === seq.current && setLoading(false));
  }, [space, on]);

  useEffect(() => {
    setSite(null);
    setList({ tickets: [], dropped: 0 });
    reload();
  }, [reload]);

  useEffect(() => {
    if (!on) return;
    const onTicket = (e: Event) => {
      const d = (e as CustomEvent<{ space_id?: string }>).detail;
      if (d?.space_id === space) reload();
    };
    window.addEventListener(DEV_TICKET_EVENT, onTicket);
    return () => window.removeEventListener(DEV_TICKET_EVENT, onTicket);
  }, [space, on, reload]);

  return { site, tickets: list.tickets, dropped: list.dropped, loading, error, needs: ticketsNeedCount(list.tickets), reload };
}

export interface TicketsPanelProps {
  space: string;
  data: TicketsData;
  /** The label that hands tickets over, for "with the label kwant"; defaults to the site's own label. */
  handoverLabel?: string | null;
  /** Bot and team names for the ids on each ticket's work and the site's team. */
  names?: TicketNames;
  onOpenSettings?: () => void;
  /** Opens the lead's chat (Open Ann's chat, Change something). */
  onOpenChat?: (t: Ticket) => void;
  onOpenReview?: (t: Ticket) => void;
}

/** One card with its own brief, checklist and actions. */
const LiveCard: React.FC<
  { space: string; t: Ticket; reload: () => void; origins: readonly string[]; names: TicketNames; jiraOff: boolean } & Pick<TicketsPanelProps, 'handoverLabel' | 'onOpenChat' | 'onOpenReview'>
> = ({ space, t, reload, origins, names, jiraOff, handoverLabel, onOpenChat, onOpenReview }) => {
  const [brief, setBrief] = useState<{ brief: Brief | null; text: string | null }>({ brief: null, text: null });
  const [progress, setProgress] = useState<TicketProgress>(emptyProgress);
  const wantsBrief = BRIEF_STATES.has(t.state);

  useEffect(() => {
    if (!wantsBrief) return;
    let live = true;
    api.getBrief(space, t.key).then((b) => live && setBrief(b), () => undefined);
    return () => {
      live = false;
    };
  }, [space, t.key, t.state, t.updated_at, wantsBrief]);

  useEffect(() => {
    if (t.state !== 'reading') return;
    const on = (e: Event) => setProgress((p) => applyProgress(p, t.key, (e as CustomEvent).detail));
    window.addEventListener(DEV_TICKET_PROGRESS_EVENT, on);
    return () => window.removeEventListener(DEV_TICKET_PROGRESS_EVENT, on);
  }, [t.key, t.state]);

  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => setNotice(null), [t.state, jiraOff]);
  const after = <T,>(p: Promise<T>) => p.then((v) => (setNotice(null), reload(), v));
  /** A failed card action: Off gets its own sentence (never a raw error); the rest show the server's words. */
  const failed = (e: unknown) => {
    setNotice(e instanceof DevModeError && e.code === 'jira_off' ? api.JIRA_OFF : e instanceof Error ? e.message : 'Something went wrong.');
    reload();
  };
  const team = workView(t, names).team ?? '';
  const b = brief.brief;
  const act = (fn: (space: string, key: string) => Promise<unknown>) => () => void after(fn(space, t.key)).catch(failed);

  return (
    <TicketCard
      ticket={t}
      brief={b}
      briefText={brief.text}
      progress={progress}
      handoverLabel={handoverLabel}
      names={names}
      origins={origins}
      jiraOff={jiraOff}
      notice={notice}
      onStart={t.state === 'waiting' ? act(api.startTicket) : undefined}
      onSkip={t.state === 'waiting' ? act(api.skipTicket) : undefined}
      onStop={t.state === 'picked_up' || t.state === 'reading' ? act(api.stopTicket) : undefined}
      onRetry={t.state === 'failed' ? act(api.retryTicket) : undefined}
      onCloseCard={t.state === 'taken_back' ? act(api.closeTicket) : undefined}
      onStartBuilding={() => void after(api.okBrief(space, t.key)).catch(failed)}
      onChangeBrief={onOpenChat ? () => onOpenChat(t) : undefined}
      onOpenChat={onOpenChat ? () => onOpenChat(t) : undefined}
      onOpenReview={onOpenReview ? () => onOpenReview(t) : undefined}
      onAnswer={(id, v) => after(api.answerQuestion(space, t.key, id, v)).then(() => undefined)}
      onDefaults={() => after(api.goWithDefaults(space, t.key)).then(() => undefined)}
      onAskPreview={b ? () => api.askPreview(space, t.key, b, team) : undefined}
      onAskPost={b ? (p) => after(api.askPost(space, t.key, b, team, p)) : undefined}
    />
  );
};

export const TicketsPanel: React.FC<TicketsPanelProps> = ({ space, data, handoverLabel: given = null, names = {}, onOpenSettings, onOpenChat, onOpenReview }) => {
  const { active, waiting, closed } = orderTickets(data.tickets);
  const handoverLabel = given ?? data.site?.label ?? null;
  const origins = siteOrigins(data.site);
  const teamName = data.site?.team_id ? names.teams?.[data.site.team_id] ?? null : null;
  const projects = data.site?.projects ?? [];
  const common = { space, reload: data.reload, handoverLabel, origins, names, jiraOff: data.site ? !data.site.on : false };
  const host = (() => {
    try {
      return data.site?.site ? new URL(data.site.site).host : null;
    } catch {
      return data.site?.site ?? null;
    }
  })();
  return (
    <div className="tk-list" data-tickets>
      <p className="tk-source">
        <span data-source-line>
          From {host ?? 'Jira'}
          {projects.length > 0 && ` · ${projects.join(', ')}`}
          {handoverLabel && (
            <>
              {' · label '}
              <code>{handoverLabel}</code>
            </>
          )}
          {teamName && ` · @${teamName}`}
        </span>
        {onOpenSettings && (
          <button type="button" className="tk-textbtn" onClick={onOpenSettings}>
            <Settings size={12} aria-hidden="true" />
            Settings
          </button>
        )}
      </p>
      {data.error && (
        <p className="tk-err" role="alert">
          {data.error}
        </p>
      )}
      {data.tickets.length === 0 && !data.loading && !data.error && (
        <p className="tk-empty">No tickets yet. Hand one over in Jira and it shows up here within 5 minutes.</p>
      )}
      {active.map((t) => (
        <LiveCard key={t.key} t={t} {...common} onOpenChat={onOpenChat} onOpenReview={onOpenReview} />
      ))}
      {waiting.length > 0 && (
        <section aria-label={`Waiting for your Start, ${waiting.length}`} className="tk-group">
          <h3 className="tk-group-h">
            Waiting for your Start · {waiting.length}
          </h3>
          {waiting.map((t) => (
            <LiveCard key={t.key} t={t} {...common} />
          ))}
        </section>
      )}
      {closed.map((t) => (
        <LiveCard key={t.key} t={t} {...common} />
      ))}
      {data.dropped > 0 && (
        <p className="tk-empty">
          {data.dropped} {data.dropped === 1 ? 'ticket' : 'tickets'} couldn’t be shown.
        </p>
      )}
    </div>
  );
};
