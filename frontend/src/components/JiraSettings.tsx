// 0.7.6 "Work from Jira": the two places it shows while it's off (design §1), and the settings box once
// it's on (§7). With Jira never connected, and on a server without the Jira routes, nothing else changes.

import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Loader2, Pencil, Power, RefreshCw, Ticket as TicketIcon } from 'lucide-react';
import type { DevTeamRef } from '../devMode';
import { JIRA_CHANGED_EVENT, JIRA_COPY, JiraState, hhmm, jiraApi, jiraHint, jiraHost, jiraNames, jiraStore, ticketSummary, tokenLine } from '../dev/jiraSetup';
import { storeApi } from '../passwordStore';
import { TeamFaces } from './TeamFaces';
import { openJiraSetup } from './JiraSetup';

/** This space's Jira state: null until GET jira answered (and on any error, which reads as off). */
export function useJira(space: string | null | undefined, enabled = true): JiraState | null {
  const st = useSyncExternalStore(
    jiraStore.subscribe,
    () => jiraStore.get(space),
    () => null,
  );
  useEffect(() => {
    if (!space || !enabled) return undefined;
    void jiraStore.load(space);
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ space?: string }>).detail;
      if (!d?.space || d.space === space) void jiraStore.load(space, true);
    };
    window.addEventListener(JIRA_CHANGED_EVENT, on);
    return () => window.removeEventListener(JIRA_CHANGED_EVENT, on);
  }, [space, enabled]);
  return enabled ? st : null;
}

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-[#b8adff] focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950';

/** The J1 hint inside the Reviews empty state. Only when GET jira answered "never connected". */
export const JiraHint: React.FC<{
  space: string;
  teams?: DevTeamRef[];
  phone?: boolean;
}> = ({ space, teams, phone }) => {
  const jira = useJira(space);
  const [hidden, setHidden] = useState(() => jiraHint.hidden(space));
  const [said, setSaid] = useState('');
  useEffect(() => (setHidden(jiraHint.hidden(space)), setSaid('')), [space]);
  if (!jira || jira.site) return null;
  // The polite line outlives the hint, so "Hidden…" is read after the button goes (shots-review A2).
  const live = (
    <span role="status" aria-live="polite" className="sr-only" data-jira-hint-live>
      {said}
    </span>
  );
  if (hidden) return live;
  return (
    <p className="mt-4 pt-3 border-t border-white/[0.06] flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] leading-[18px] text-zinc-400" data-jira-hint>
      <span>{JIRA_COPY.hint}</span>
      <button
        type="button"
        onClick={(e) => openJiraSetup({ space, anchor: e.currentTarget, teams })}
        className={`inline-flex items-center gap-1.5 font-semibold text-[#b4a9ff] hover:text-zinc-50 rounded ${phone ? 'py-3 -my-3' : ''} ${focusRing}`}
        data-jira-hint-open
      >
        <TicketIcon size={13} aria-hidden="true" />
        {JIRA_COPY.hintButton}
      </button>
      <button
        type="button"
        onClick={() => {
          jiraHint.hide(space);
          setHidden(true);
          setSaid(JIRA_COPY.hintHidden);
        }}
        className={`ml-auto px-1.5 py-0.5 -mr-1.5 rounded-md text-zinc-500 hover:text-zinc-200 ${phone ? 'py-3 -my-3' : ''} ${focusRing}`}
        aria-label={JIRA_COPY.hintHideLabel}
        data-jira-hint-hide
      >
        {JIRA_COPY.hintHide}
      </button>
      {live}
    </p>
  );
};

const box = 'rounded-xl border border-zinc-800 bg-zinc-950/60 p-4';
const rowBtn =
  'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-300 border border-zinc-700/60 bg-zinc-800/60 hover:bg-zinc-700/70 hover:text-zinc-100 transition-colors disabled:opacity-50';
const dangerBtn = 'flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-rose-300 border border-rose-400/30 hover:bg-rose-500/10 disabled:opacity-50';

/** Settings › This space › Dev mode: "Work from Jira · Off · Set up", or the On box. */
export const JiraSettings: React.FC<{
  space: string;
  teams?: DevTeamRef[];
}> = ({ space, teams }) => {
  const jira = useJira(space);
  const [confirm, setConfirm] = useState(false);
  const [forget, setForget] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [polling, setPolling] = useState(false);
  const [pollNote, setPollNote] = useState<string | null>(null);
  const [store, setStore] = useState<'local' | 'bitwarden' | null>(null);
  const setupBtn = useRef<HTMLButtonElement>(null);
  useEffect(() => setConfirm(false), [jira?.site?.jira_url]);
  const hasSite = !!jira?.site;
  useEffect(() => {
    if (!hasSite) return undefined;
    let live = true;
    storeApi.peek().then(
      (st) => live && setStore(st.store),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [hasSite]);
  if (!jira) return null; // unknown (still loading, or a server without the routes): nothing shows
  const site = jira.site;

  if (!site)
    return (
      <div className={box} data-jira-settings="off">
        <p className="flex items-center gap-2 text-xs font-medium text-zinc-200">
          <TicketIcon size={14} className="text-zinc-400" aria-hidden="true" /> Work from Jira
          <span className="kw-chip">Off</span>
          <button ref={setupBtn} type="button" className={`${rowBtn} ml-auto`} onClick={() => openJiraSetup({ space, anchor: setupBtn.current, teams })} data-jira-setup-open>
            Set up
          </button>
        </p>
        <p className="mt-1 text-[11px] text-zinc-500">{JIRA_COPY.settingsBlurb}</p>
      </div>
    );

  const turnOff = async () => {
    setBusy(true);
    setError(null);
    try {
      await jiraApi.remove(space, forget);
      await jiraStore.load(space, true);
      window.dispatchEvent(new window.CustomEvent(JIRA_CHANGED_EVENT, { detail: { space } }));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Couldn’t turn it off.');
    } finally {
      setBusy(false);
    }
  };
  const checkNow = async () => {
    setPolling(true);
    setPollNote(null);
    try {
      setPollNote(await jiraApi.poll(space));
      await jiraStore.load(space, true);
    } catch (e) {
      setPollNote(e instanceof Error ? e.message : 'Couldn’t check Jira.');
    } finally {
      setPolling(false);
    }
  };
  const team = teams?.find((t) => t.id === site.team_id);
  const rejected = site.token_rejected_at;
  const names = jiraNames.get(space);
  const chip = 'inline-block px-1.5 py-px rounded-md border border-[#3f3868] bg-[#9d8fff]/10 font-mono text-[11px] leading-4 font-semibold text-[#b4a9ff]';
  const kv: [string, React.ReactNode][] = [
    [
      'Jira',
      <>
        {jiraHost(site.jira_url)}{' '}
        <span className="text-zinc-500">
          · {site.kind === 'cloud' ? 'Jira Cloud' : 'Jira Data Center'}
          {site.email ? ` · as ${site.email}` : ''}
        </span>
      </>,
    ],
    [
      'Projects',
      site.projects.length ? (
        <span className="flex flex-wrap items-center gap-x-3 gap-y-1" data-jira-projects>
          {site.projects.map((k) => (
            <span key={k} className="inline-flex items-center gap-1.5">
              <code className={chip}>{k}</code>
              {names.projects[k] && <span>{names.projects[k]}</span>}
            </span>
          ))}
        </span>
      ) : (
        '—'
      ),
    ],
    [
      'Hand over by',
      site.handover === 'jql' ? (
        'Your search'
      ) : site.handover === 'assignee' ? (
        `Assigning it to ${site.assignee_name ?? 'a Jira user'}`
      ) : (
        <>
          Adding the label <code className={chip}>{site.label}</code>
        </>
      ),
    ],
    [
      'Who takes them',
      team ? (
        <span className="inline-flex items-center gap-1.5" data-jira-who>
          {team.lead && <TeamFaces bots={[team.lead]} />}
          <span>@{team.name}</span>
          {team.lead && <span className="text-zinc-500">· {team.lead.name} leads</span>}
        </span>
      ) : (
        'The space’s lead bot'
      ),
    ],
    ['Confluence', site.spaces.length ? site.spaces.map((k) => (names.spaces[k] ? `${k} ${names.spaces[k]}` : k)).join(', ') : 'Off'],
    ['On its own', `${site.wait_for_ok ? 'Waits for your OK before building' : 'Builds without waiting for your OK'} · ${site.comment_progress ? 'Comments on tickets' : 'No comments on tickets'}`],
    [
      'Token',
      <span data-jira-token-line>
        {tokenLine(store, null)}
        {site.scope_note && <span className="text-zinc-500"> · {site.scope_note}</span>}
      </span>,
    ],
  ];
  return (
    <div className={`${box} space-y-3`} data-jira-settings="on">
      <p className="flex items-center gap-2 text-xs font-medium text-zinc-200">
        <TicketIcon size={14} className="text-[#b4a9ff]" aria-hidden="true" /> Work from Jira
        <span className={`kw-chip ${site.on ? 'kw-chip-ok' : ''}`}>{site.on ? 'On' : 'Paused'}</span>
      </p>
      <p className={`flex items-center gap-2 text-[11px] ${rejected ? 'text-rose-300' : 'text-zinc-400'}`} data-jira-status>
        <span className={`w-1.5 h-1.5 rounded-full ${rejected ? 'bg-rose-400' : 'bg-teal-300'}`} aria-hidden="true" />
        {rejected ? `Jira stopped accepting the token at ${hhmm(rejected)}. Nothing new is picked up.` : `${site.polled_at ? `Checked ${hhmm(site.polled_at)} · ` : ''}${ticketSummary(jira.tickets)}`}
      </p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs">
        {kv.map(([k, v]) => (
          <React.Fragment key={k}>
            <dt className="text-zinc-500">{k}</dt>
            <dd className="text-zinc-200 min-w-0 break-words">{v}</dd>
          </React.Fragment>
        ))}
      </dl>
      <div className="flex items-center gap-2">
        <button ref={setupBtn} type="button" className={rowBtn} onClick={() => openJiraSetup({ space, anchor: setupBtn.current, teams, site })} data-jira-edit>
          <Pencil size={12} aria-hidden="true" /> {rejected ? 'Fix it' : 'Edit'}
        </button>
        <button type="button" className={rowBtn} disabled={polling} onClick={() => void checkNow()} data-jira-check-now>
          {polling ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <RefreshCw size={12} aria-hidden="true" />} Check now
        </button>
        <span className="flex-1" />
        <button type="button" className={dangerBtn} aria-expanded={confirm} onClick={() => setConfirm((c) => !c)} data-jira-turn-off>
          <Power size={12} aria-hidden="true" /> Turn off
        </button>
      </div>
      <p role="status" aria-live="polite" className={pollNote ? 'text-[11px] text-zinc-400' : 'sr-only'} data-jira-poll-note>
        {pollNote}
      </p>
      {confirm && (
        <div role="group" aria-label="Turn off Work from Jira" className="rounded-lg border border-rose-400/25 bg-rose-500/[0.06] p-3 space-y-2" data-jira-off-confirm>
          <p className="text-xs font-semibold text-zinc-100">Stop taking tickets from Jira?</p>
          <p className="text-[11px] text-zinc-400">New tickets are ignored. Tickets already in progress pause, and their cards, branches and briefs stay. Nothing in Jira changes.</p>
          <label className="flex items-start gap-2 text-xs text-zinc-200">
            <input type="checkbox" checked={forget} onChange={(e) => setForget(e.target.checked)} className="mt-0.5 accent-[#9d8fff]" />
            <span>
              Also forget the Jira token
              <span className="block text-[11px] text-zinc-500">You’ll paste a token again to turn it back on.</span>
            </span>
          </label>
          {error && (
            <p role="alert" className="text-[11px] text-rose-300">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <button type="button" className={rowBtn} onClick={() => setConfirm(false)}>
              Keep it on
            </button>
            <button type="button" className={dangerBtn} disabled={busy} onClick={() => void turnOff()} data-jira-off-yes>
              Stop taking tickets
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
