// 0.7.6 "Work from Jira": the setup sheet (design.md §2–§4, J2–J4, J9, J10). A popover next to the button
// that opened it on desktop (448 px), a bottom sheet on the phone; the same content in both.
//
// Tokens: read from their fields into refs, sent with the check and the save, cleared on close. They are
// never put in React state, the store, a URL or a log.

import React, { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, ChevronRight, Loader2, Send, Ticket as TicketIcon, X } from 'lucide-react';
import { useEscapeLayer } from '../escape';
import { usePhoneLayout } from '../usePhoneLayout';
import { SaveGate, saveGate, storeApi } from '../passwordStore';
import type { DevTeamRef } from '../devMode';
import { popoverPlace } from './GitHubConnect';
import { SaveFirst } from './LooseEndsView';
import { TeamFaces } from './TeamFaces';
import {
  CheckOutcome,
  DEFAULT_LABEL,
  Handover,
  JIRA_CHANGED_EVENT,
  JIRA_COPY,
  JiraError,
  JiraForm,
  JiraKind,
  JiraSite,
  JiraUser,
  builtJql,
  jiraApi,
  jiraHost,
  jiraStore,
  kindFromHost,
  waitForOkText,
  matchLine,
  noProjectsText,
  normaliseJiraUrl,
  rateLimitedText,
  step1Error,
  zeroMatchHelp,
} from '../dev/jiraSetup';

export const JIRA_SETUP_EVENT = 'kwnt:jira-setup';
const WIDTH = 448;

export interface JiraSetupRequest {
  space: string;
  anchor?: HTMLElement | null;
  teams?: DevTeamRef[];
  /** Edit: start from the saved site (step 1 folded, the token kept unless changed). */
  site?: JiraSite | null;
}

export function openJiraSetup(req: JiraSetupRequest): void {
  window.dispatchEvent(new window.CustomEvent(JIRA_SETUP_EVENT, { detail: req }));
}

export const JiraSetupHost: React.FC<{ onSaved?: (space: string) => void }> = ({ onSaved }) => {
  const [req, setReq] = useState<(JiraSetupRequest & { n: number }) | null>(null);
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<JiraSetupRequest>).detail;
      if (d?.space) setReq((r) => ({ ...d, n: (r?.n ?? 0) + 1 }));
    };
    window.addEventListener(JIRA_SETUP_EVENT, on);
    return () => window.removeEventListener(JIRA_SETUP_EVENT, on);
  }, []);
  if (!req) return null;
  return (
    <JiraSetupLayer
      key={req.n}
      req={req}
      onClose={() => setReq(null)}
      onSaved={() => {
        setReq(null);
        void jiraStore.load(req.space, true);
        window.dispatchEvent(
          new window.CustomEvent(JIRA_CHANGED_EVENT, {
            detail: { space: req.space },
          }),
        );
        onSaved?.(req.space);
      }}
    />
  );
};

const JiraSetupLayer: React.FC<{
  req: JiraSetupRequest;
  onClose: () => void;
  onSaved: () => void;
}> = ({ req, onClose, onSaved }) => {
  const phone = usePhoneLayout();
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{
    left: number;
    top: number;
    width: number;
  } | null>(null);
  const close = () => {
    onClose();
    const a = req.anchor;
    if (a?.isConnected) window.setTimeout(() => a.focus({ preventScroll: true }), 0);
  };
  useEscapeLayer(ref, true, close, {
    modal: true,
    fallback: () => (req.anchor?.isConnected ? req.anchor : null),
  });

  useLayoutEffect(() => {
    if (phone) return undefined;
    const measure = () => {
      const r = req.anchor?.isConnected ? req.anchor.getBoundingClientRect() : null;
      setPlace(popoverPlace(r, window.innerWidth, window.innerHeight, Math.min(ref.current?.offsetHeight ?? 420, window.innerHeight - 24), WIDTH));
    };
    measure();
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(measure) : null;
    if (ref.current) ro?.observe(ref.current);
    window.addEventListener('resize', measure);
    return () => {
      ro?.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [phone, req.anchor]);

  useEffect(() => {
    if (phone) return undefined;
    const down = (e: MouseEvent) => {
      const t = e.target as Node | null;
      if (!t || ref.current?.contains(t) || req.anchor?.contains(t)) return;
      close();
    };
    document.addEventListener('mousedown', down);
    return () => document.removeEventListener('mousedown', down);
  });

  const body = <JiraSetupForm space={req.space} teams={req.teams ?? []} site={req.site ?? null} phone={phone} onCancel={close} onSaved={onSaved} />;

  if (phone)
    return createPortal(
      <div className="fixed inset-0 z-[70] flex flex-col justify-end" data-jira-setup-sheet>
        <button type="button" aria-label="Close" tabIndex={-1} onClick={close} className="absolute inset-0 bg-black/50 cursor-default" />
        <div
          ref={ref}
          tabIndex={-1}
          role="dialog"
          aria-modal="true"
          aria-label={JIRA_COPY.title}
          className="kw-sheet outline-none relative flex flex-col max-h-[88vh] rounded-t-2xl bg-zinc-900 border-t border-zinc-800 shadow-2xl"
        >
          <div className="flex justify-center pt-2 pb-1" aria-hidden="true">
            <span className="w-9 h-1 rounded-sm bg-zinc-700" />
          </div>
          {body}
        </div>
      </div>,
      document.body,
    );

  return createPortal(
    <div
      ref={ref}
      tabIndex={-1}
      role="dialog"
      aria-modal="false"
      aria-label={JIRA_COPY.title}
      data-jira-setup-popover
      style={place ? { left: place.left, top: place.top, width: place.width } : { visibility: 'hidden', left: 0, top: 0, width: WIDTH }}
      className="fixed z-[70] outline-none flex flex-col max-h-[calc(100vh-24px)] rounded-xl bg-zinc-900 shadow-2xl shadow-black/60 ring-1 ring-zinc-700"
    >
      {body}
    </div>,
    document.body,
  );
};

// ---------------------------------------------------------------------------------------------------
// Pieces drawn as design.md shows them (jira.css), in the app's Tailwind.

const fl = 'block mt-2.5 mb-1 text-[12.5px] leading-[18px] font-medium text-zinc-200';
const inp = (bad: boolean) =>
  `w-full h-9 px-2.5 rounded-lg border bg-zinc-950 text-zinc-100 font-mono text-[13px] placeholder:text-zinc-500 focus:outline-none focus:border-[#9d8fff] focus:ring-2 focus:ring-[#9d8fff]/55 focus:ring-offset-2 focus:ring-offset-zinc-900 ${bad ? 'border-rose-300/70' : 'border-zinc-800'}`;
const help = 'mt-1.5 text-xs leading-[17px] text-zinc-400';
const textBtn = 'text-xs font-medium text-[#b4a9ff] hover:text-[#cfc7ff] focus:outline-none focus-visible:underline';
const ghostBtn = 'h-9 px-3 rounded-lg text-[13px] font-medium text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100 disabled:opacity-50';
const primaryBtn = 'inline-flex items-center gap-1.5 h-9 px-3.5 rounded-lg text-[13px] font-semibold text-[#120e26] bg-[#9d8fff] hover:bg-[#b4a9ff] disabled:opacity-60';

const Step: React.FC<{
  n: number;
  title: string;
  done?: React.ReactNode;
  first?: boolean;
  children?: React.ReactNode;
}> = ({ n, title, done, first, children }) => (
  <section className={`${first ? 'mt-3.5' : 'mt-4'} pt-3.5 border-t border-white/[0.06]`} aria-label={title} data-jira-step={n}>
    <h4 className="m-0 mb-2 flex items-center gap-2 whitespace-nowrap text-[11px] leading-4 font-semibold tracking-[.08em] uppercase text-zinc-400">
      {done ? (
        <span className="w-[18px] h-[18px] rounded-full inline-grid place-items-center bg-teal-300 text-[#120e26]" aria-hidden="true">
          <Check size={11} strokeWidth={3} />
        </span>
      ) : (
        <span className="w-[18px] h-[18px] rounded-full inline-grid place-items-center font-mono text-[10.5px] font-bold tracking-normal text-[#b4a9ff] bg-[#9d8fff]/15" aria-hidden="true">
          {n}
        </span>
      )}
      {title}
      {done && <span className="ml-auto min-w-0 inline-flex items-center gap-0.5 normal-case tracking-normal font-medium text-xs text-zinc-400">{done}</span>}
    </h4>
    {children}
  </section>
);

const Err: React.FC<{ id?: string; children: React.ReactNode }> = ({ id, children }) => (
  <p id={id} role="alert" className="mt-1.5 flex gap-1.5 text-[12.5px] leading-[18px] text-rose-300" data-jira-error>
    <X size={14} className="mt-0.5 shrink-0" aria-hidden="true" />
    <span>{children}</span>
  </p>
);

const Found: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span className="inline-flex items-center gap-1.5 h-[22px] px-2 rounded-full text-xs font-sans text-teal-300 bg-teal-300/10 border border-teal-300/25 whitespace-nowrap" data-jira-found>
    <Check size={12} aria-hidden="true" />
    {children}
  </span>
);

const Pick: React.FC<{
  k: string;
  name: string;
  on: boolean;
  sug?: string | null;
  onToggle: () => void;
  phone: boolean;
}> = ({ k, name, on, sug, onToggle, phone }) => (
  <button
    type="button"
    role="checkbox"
    aria-checked={on}
    onClick={onToggle}
    data-jira-pick={k}
    className={`inline-flex items-center gap-1.5 ${phone ? 'min-h-[44px]' : 'min-h-[30px]'} pl-2 pr-2.5 rounded-full border text-[12.5px] text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[#9d8fff]/70 ${on ? 'border-[#9d8fff]/60 bg-[#9d8fff]/10 text-zinc-100' : 'border-white/10 text-zinc-300 hover:border-white/20'}`}
  >
    <span className={`w-3.5 h-3.5 rounded grid place-items-center border-[1.5px] ${on ? 'bg-[#9d8fff] border-[#9d8fff] text-[#120e26]' : 'border-zinc-500'}`} aria-hidden="true">
      {on && <Check size={10} strokeWidth={3.5} />}
    </span>
    <span className="font-mono text-[11.5px] font-semibold text-[#b4a9ff]">{k}</span>
    {name}
    {sug && <span className="text-[11px] text-teal-300">· {sug}</span>}
  </button>
);

const CheckRow: React.FC<{
  on: boolean;
  onChange: (v: boolean) => void;
  title: string;
  text: string;
  sw?: boolean;
  name: string;
}> = ({ on, onChange, title, text, sw, name }) => {
  // The words toggle too (a 44 px target on the phone without a bigger control); the button keeps the role.
  const id = useId();
  return (
    <div className="flex items-start gap-2.5 py-2">
      <button
        type="button"
        role={sw ? 'switch' : 'checkbox'}
        aria-checked={on}
        aria-labelledby={`${id}-t`}
        aria-describedby={`${id}-d`}
        onClick={() => onChange(!on)}
        data-jira-toggle={name}
        data-compact
        className={
          sw
            ? `relative shrink-0 mt-px w-9 h-5 rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-[#9d8fff]/70 ${on ? 'bg-[#6c58e0]' : 'bg-[#2c2552] shadow-[inset_0_0_0_1px_rgba(255,255,255,.14)]'}`
            : `shrink-0 mt-px w-[18px] h-[18px] rounded-[5px] grid place-items-center border-[1.5px] focus:outline-none focus-visible:ring-2 focus-visible:ring-[#9d8fff]/70 ${on ? 'bg-[#9d8fff] border-[#9d8fff] text-[#120e26]' : 'border-zinc-500'}`
        }
      >
        {sw ? (
          <span
            className={`absolute left-[3px] top-[3px] w-3.5 h-3.5 rounded-full transition-transform motion-reduce:transition-none ${on ? 'translate-x-4 bg-zinc-50' : 'shadow-[inset_0_0_0_2px_#d4d4d8]'}`}
            aria-hidden="true"
          />
        ) : (
          on && <Check size={12} strokeWidth={3} aria-hidden="true" />
        )}
      </button>
      <div className="flex-1 min-w-0 cursor-pointer select-none" onClick={() => onChange(!on)}>
        <b id={`${id}-t`} className="block text-[13px] leading-[19px] font-medium text-zinc-100">
          {title}
        </b>
        <span id={`${id}-d`} className="block text-xs leading-[17px] text-zinc-400">
          {text}
        </span>
      </div>
    </div>
  );
};

const POLLS = [
  { v: 60, label: '1 minute' },
  { v: 300, label: '5 minutes' },
  { v: 900, label: '15 minutes' },
  { v: 3600, label: '1 hour' },
];

/** Which team to suggest: the one whose name shares a word with the picked project's name, else the only team. */
/** The step 2 picker's line under the team (design desk-07). */
export const leadLine = (lead: string): string => `${lead} leads and reads every ticket`;

export function suggestTeam(teams: DevTeamRef[], projectName: string | null): { team: DevTeamRef; why: string | null } | null {
  if (!teams.length) return null;
  const words = (s: string) =>
    s
      .toLowerCase()
      .split(/[^a-z0-9]+/)
      .filter((w) => w.length > 2);
  const pw = projectName ? words(projectName) : [];
  const hit = pw.length ? teams.find((t) => words(t.name).some((w) => pw.some((p) => p.startsWith(w) || w.startsWith(p)))) : undefined;
  if (hit)
    return {
      team: hit,
      why: `because @${hit.name} works on ${projectName!.toLowerCase()}.`,
    };
  return teams.length === 1 ? { team: teams[0], why: null } : null;
}

// ---------------------------------------------------------------------------------------------------
// The form

export const JiraSetupForm: React.FC<{
  space: string;
  teams: DevTeamRef[];
  site: JiraSite | null;
  phone: boolean;
  onCancel: () => void;
  onSaved: () => void;
  /** Test hook: start past the store check. */
  initialGate?: SaveGate;
  /** Test and harness hook: the recount wait (400 ms). */
  recountMs?: number;
}> = ({ space, teams, site, phone, onCancel, onSaved, initialGate, recountMs = 400 }) => {
  const id = useId();
  // ---- step 1
  const [address, setAddress] = useState(site ? jiraHost(site.jira_url) : '');
  const [kind, setKind] = useState<JiraKind | null>(site?.kind ?? null);
  /** Confirmed by the server (a token-less check ran its type detection), not only guessed from the host. */
  const [confirmed, setConfirmed] = useState(!!site);
  const [email, setEmail] = useState(site?.email ?? '');
  const token = useRef('');
  const confToken = useRef('');
  const [hasToken, setHasToken] = useState(false);
  const [addrErr, setAddrErr] = useState<string | null>(null);
  const [tokErr, setTokErr] = useState<string | null>(null);
  const [mailErr, setMailErr] = useState<string | null>(null);
  const [noProjects, setNoProjects] = useState(false);
  const [slowDown, setSlowDown] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [gate, setGate] = useState<SaveGate | 'checking'>(initialGate ?? (site ? 'open' : 'checking'));
  const [acceptRisk, setAcceptRisk] = useState(false);
  const [stage, setStage] = useState<1 | 2>(site ? 2 : 1);
  const [check, setCheck] = useState<CheckOutcome | null>(null);
  // ---- step 2
  const [projects, setProjects] = useState<string[]>(site?.projects ?? []);
  const [moreProjects, setMoreProjects] = useState(false);
  const [handover, setHandover] = useState<Handover>(site?.handover ?? 'label');
  const [label] = useState(site?.label ?? DEFAULT_LABEL);
  const [assignee, setAssignee] = useState<{ id: string; name: string } | null>(site?.assignee_id ? { id: site.assignee_id, name: site.assignee_name ?? site.assignee_id } : null);
  const [userQuery, setUserQuery] = useState('');
  const [users, setUsers] = useState<JiraUser[]>([]);
  const [teamId, setTeamId] = useState<string | null>(site?.team_id ?? null);
  const [teamTouched, setTeamTouched] = useState(!!site);
  const [counting, setCounting] = useState(false);
  // ---- step 3
  const [conf, setConf] = useState(site ? site.spaces.length > 0 : true);
  const [confUrl, setConfUrl] = useState(site?.kind === 'dc' && site.confluence_url ? jiraHost(site.confluence_url) : '');
  const [spaces, setSpaces] = useState<string[]>(site?.spaces ?? []);
  // ---- step 4
  const [waitOk, setWaitOk] = useState(site?.wait_for_ok ?? false);
  const [comment, setComment] = useState(site?.comment_progress ?? false);
  const [jql, setJql] = useState<string | null>(site?.handover === 'jql' ? site.jql : null);
  const [poll, setPoll] = useState(site?.poll_secs ?? 300);
  // ---- footer
  const [saving, setSaving] = useState(false);
  const [saveErr, setSaveErr] = useState<string | null>(null);
  const addrRef = useRef<HTMLInputElement>(null);
  const tokRef = useRef<HTMLInputElement>(null);

  // The password store decides whether the "Before you save the token" card comes first (as GitHub's form).
  useEffect(() => {
    if (gate !== 'checking') return undefined;
    let live = true;
    storeApi.peek().then(
      (st) => live && setGate(saveGate(st)),
      () => live && setGate('open'),
    );
    return () => {
      live = false;
    };
  }, [gate]);
  useEffect(() => {
    if (gate !== 'store' && gate !== 'locked') return undefined;
    const recheck = () =>
      void storeApi.peek().then(
        (st) => setGate((g) => (g === 'store' || g === 'locked' ? saveGate(st) : g)),
        () => undefined,
      );
    window.addEventListener('focus', recheck);
    return () => window.removeEventListener('focus', recheck);
  }, [gate]);
  // Tokens leave memory when the sheet goes.
  useEffect(
    () => () => {
      token.current = '';
      confToken.current = '';
    },
    [],
  );
  useEffect(() => {
    if (stage === 1) window.setTimeout(() => addrRef.current?.focus({ preventScroll: true }), 0);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const url = normaliseJiraUrl(address);
  const effKind: JiraKind = kind ?? 'cloud';
  const showToken = gate === 'open' && !!kind;

  // The address: Cloud is known from its host; anything else asks the server (a check with no token).
  const detect = useCallback(async () => {
    setAddrErr(null);
    if (!address.trim()) return;
    const u = normaliseJiraUrl(address);
    if (!u) {
      setAddrErr('Paste your Jira address, like yourcompany.atlassian.net.');
      return;
    }
    const k = kindFromHost(u);
    if (k) {
      setKind(k);
      setConfirmed(true);
      return;
    }
    setKind('dc');
    setConfirmed(false);
    try {
      const c = await jiraApi.detect(space, u);
      if (c.result === 'not_jira' || c.result === 'unreachable') {
        setAddrErr(JIRA_COPY.notJira);
        setKind(null);
      } else if (c.kind) {
        setKind(c.kind);
        setConfirmed(true);
      }
    } catch {
      /* an older server: keep the guess, the full check tells */
    }
  }, [address, space]);

  const form = (over: Partial<JiraForm> = {}): JiraForm => ({
    jira_url: url ?? address.trim(),
    kind: effKind,
    email: effKind === 'cloud' ? email.trim() : undefined,
    token: token.current || undefined,
    confluence_url: effKind === 'dc' && conf && confUrl.trim() ? (normaliseJiraUrl(confUrl) ?? confUrl.trim()) : undefined,
    confluence_token: effKind === 'dc' && conf ? confToken.current || undefined : undefined,
    ...over,
  });

  const runCheck = async () => {
    setTokErr(null);
    setMailErr(null);
    setNoProjects(false);
    setSlowDown(null);
    if (!url) {
      setAddrErr('Paste your Jira address, like yourcompany.atlassian.net.');
      addrRef.current?.focus();
      return;
    }
    if (effKind === 'cloud' && !/^[^\s@]+@[^\s@]+$/.test(email.trim())) {
      setMailErr('Type the email of your Atlassian account.');
      return;
    }
    if (!token.current && !site) {
      setTokErr(effKind === 'cloud' ? 'Paste the API token.' : 'Paste the personal access token.');
      tokRef.current?.focus();
      return;
    }
    setChecking(true);
    try {
      const c = await jiraApi.check(space, form());
      const err = step1Error(c, effKind, email.trim() || null);
      if (err) {
        if (err.field === 'address') setAddrErr(err.text);
        else if (err.field === 'email') setMailErr(err.text);
        else setTokErr(err.text);
        return;
      }
      if (c.result === 'no_projects') {
        setNoProjects(true);
        return;
      }
      if (c.result === 'rate_limited') {
        setSlowDown(rateLimitedText(c.retry_at));
        return;
      }
      if (c.kind) setKind(c.kind);
      setConfirmed(true);
      setCheck(c);
      setStage(2);
      if (!site) {
        const sug = c.suggested_project?.key;
        setProjects(sug ? [sug] : []);
        const names = c.confluence?.spaces ?? [];
        setSpaces(names.slice(0, 2).map((s) => s.key));
      }
    } catch (e) {
      setTokErr(e instanceof Error ? e.message : 'The check didn’t run.');
    } finally {
      setChecking(false);
    }
  };

  // Editing a saved site: one quiet check for the project list (the token stays on the server).
  useEffect(() => {
    if (!site) return;
    jiraApi
      .check(
        space,
        form({
          token: undefined,
          projects: site.projects,
          handover: site.handover,
          jql: site.jql,
        }),
      )
      .then(
        (c) => c.result === 'found' && setCheck(c),
        () => undefined,
      );
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // The live result: counted again 400 ms after the choices settle.
  const autoJql = builtJql({
    projects,
    handover,
    label,
    assignee_id: assignee?.id ?? null,
  });
  const rule = jql ?? autoJql;
  useEffect(() => {
    if (stage !== 2 || !projects.length) return undefined;
    if (handover === 'assignee' && !assignee && jql === null) return undefined;
    setCounting(true);
    const h = window.setTimeout(() => {
      jiraApi
        .check(
          space,
          form({
            projects,
            handover: jql !== null ? 'jql' : handover,
            label,
            assignee_id: assignee?.id ?? null,
            jql,
          }),
        )
        .then(
          (c) =>
            setCheck((old) =>
              c.result === 'found'
                ? {
                    ...c,
                    projects: c.projects.length ? c.projects : (old?.projects ?? []),
                  }
                : old && { ...old, jql_error: c.message ?? c.jql_error },
            ),
          () => undefined,
        )
        .finally(() => setCounting(false));
    }, recountMs);
    return () => window.clearTimeout(h);
  }, [stage, projects.join(','), handover, assignee?.id, jql]); // eslint-disable-line react-hooks/exhaustive-deps

  // The Jira user picker (assignee rule).
  useEffect(() => {
    if (handover !== 'assignee' || userQuery.trim().length < 2) {
      setUsers([]);
      return undefined;
    }
    const h = window.setTimeout(() => jiraApi.users(space, userQuery.trim()).then(setUsers, () => setUsers([])), 250);
    return () => window.clearTimeout(h);
  }, [handover, userQuery, space]);

  const allProjects = check?.projects ?? (site?.projects ?? []).map((k) => ({ key: k, name: k }));
  const sugKey = check?.suggested_project?.key ?? null;
  const ordered = [...allProjects].sort((a, b) => Number(b.key === sugKey) - Number(a.key === sugKey) || Number(projects.includes(b.key)) - Number(projects.includes(a.key)));
  const shown = moreProjects ? ordered : ordered.slice(0, 3);
  const firstName = allProjects.find((p) => p.key === projects[0])?.name ?? null;
  const sug = suggestTeam(teams, firstName);
  const team = teams.find((t) => t.id === teamId) ?? (!teamTouched ? sug?.team : undefined) ?? null;
  const lead = team?.lead ?? null;
  const confSpaces = check?.confluence?.spaces ?? (site?.spaces ?? []).map((k) => ({ id: null, key: k, name: k }));
  const confNeedsToken = effKind === 'dc' && check?.confluence?.result !== 'ok';

  const save = async () => {
    setSaveErr(null);
    if (!projects.length && jql === null) {
      setSaveErr('Pick at least one project.');
      return;
    }
    if (handover === 'assignee' && !assignee && jql === null) {
      setSaveErr('Pick the Jira user tickets are assigned to.');
      return;
    }
    setSaving(true);
    try {
      await jiraApi.save(
        space,
        form({
          projects,
          handover: jql !== null ? 'jql' : handover,
          label,
          assignee_id: handover === 'assignee' ? (assignee?.id ?? null) : null,
          jql,
        }),
        {
          spaces: conf ? spaces : [],
          team_id: team?.id ?? null,
          assignee_name: handover === 'assignee' ? (assignee?.name ?? null) : null,
          comment_progress: comment,
          wait_for_ok: waitOk,
          poll_secs: poll,
          accept_risk: acceptRisk || undefined,
        },
      );
      token.current = '';
      confToken.current = '';
      onSaved();
    } catch (e) {
      if (e instanceof JiraError && e.status === 422 && e.check) {
        const err = step1Error(e.check, effKind, email.trim() || null);
        if (err) {
          setStage(1);
          if (err.field === 'address') setAddrErr(err.text);
          else setTokErr(err.text);
        } else if (e.check.result === 'no_projects') {
          setStage(1);
          setNoProjects(true);
        } else if (e.check.result === 'rate_limited') setSaveErr(rateLimitedText(e.check.retry_at));
        else setSaveErr(e.check.jql_error ?? e.message);
      } else setSaveErr(e instanceof Error ? e.message : 'Couldn’t save.');
    } finally {
      setSaving(false);
    }
  };

  const host = jiraHost(url);
  const kindName = effKind === 'cloud' ? 'Jira Cloud' : 'Jira Data Center';
  const kindLine = effKind === 'cloud' ? 'Found Jira Cloud.' : confirmed ? 'Found Jira Data Center (your company runs it).' : 'Looks like Jira Data Center (your company runs it).';
  const segH = phone ? 'min-h-[44px]' : 'min-h-[30px]';

  // ---- step 1, open
  const step1 =
    stage === 1 ? (
      <Step n={1} title="Your Jira" first>
        <label htmlFor={`${id}-addr`} className={`${fl} mt-0`}>
          Jira address
        </label>
        <div className="relative">
          <input
            ref={addrRef}
            id={`${id}-addr`}
            value={address}
            onChange={(e) => {
              setAddress(e.target.value);
              setAddrErr(null);
              setConfirmed(false);
              setKind(null);
            }}
            onBlur={() => void detect()}
            onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), void detect())}
            placeholder="yourcompany.atlassian.net"
            autoComplete="off"
            spellCheck={false}
            inputMode="url"
            aria-invalid={!!addrErr || undefined}
            aria-describedby={addrErr ? `${id}-addr-e` : undefined}
            className={`${inp(!!addrErr)} ${kind && !addrErr ? 'pr-36' : ''}`}
            data-jira-address
          />
          {kind && !addrErr && (
            <span className="absolute right-2 top-1/2 -translate-y-1/2">
              <Found>{kindName}</Found>
            </span>
          )}
        </div>
        {addrErr ? (
          <Err id={`${id}-addr-e`}>{addrErr}</Err>
        ) : (
          kind && (
            <div className="mt-2 flex items-center gap-2 text-[12.5px] text-zinc-300" data-jira-kind={effKind} data-jira-kind-confirmed={confirmed || undefined}>
              <span>{kindLine}</span>
              <button type="button" className={textBtn} onClick={() => (setKind(effKind === 'cloud' ? 'dc' : 'cloud'), setConfirmed(false))} data-jira-kind-change>
                Not right? Change
              </button>
            </div>
          )
        )}
        {kind && !addrErr && (gate === 'store' || gate === 'locked') && (
          <div className="mt-2.5">
            <SaveFirst
              gate={gate}
              why={JIRA_COPY.saveFirstWhy}
              onReady={(_note, accept) => {
                if (accept) setAcceptRisk(true);
                setGate('open');
                window.setTimeout(() => tokRef.current?.focus(), 0);
              }}
            />
          </div>
        )}
        {showToken && !addrErr && (
          <>
            {effKind === 'cloud' && (
              <>
                <label htmlFor={`${id}-mail`} className={fl}>
                  Your Atlassian email
                </label>
                <input
                  id={`${id}-mail`}
                  type="email"
                  value={email}
                  onChange={(e) => (setEmail(e.target.value), setMailErr(null))}
                  placeholder="you@yourcompany.com"
                  autoComplete="email"
                  aria-invalid={!!mailErr || undefined}
                  className={inp(!!mailErr)}
                  data-jira-email
                />
                {mailErr && <Err>{mailErr}</Err>}
              </>
            )}
            <label htmlFor={`${id}-tok`} className={fl}>
              {effKind === 'cloud' ? 'API token' : 'Personal access token'}
            </label>
            <input
              ref={tokRef}
              id={`${id}-tok`}
              type="password"
              onChange={(e) => {
                token.current = e.target.value;
                setHasToken(!!e.target.value);
                setTokErr(null);
                setNoProjects(false);
              }}
              placeholder={effKind === 'cloud' ? 'Paste the API token' : 'Paste the personal access token'}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={!!tokErr || undefined}
              aria-describedby={`${id}-tok-h`}
              className={inp(!!tokErr)}
              data-jira-token
            />
            {tokErr ? (
              <Err id={`${id}-tok-h`}>{tokErr}</Err>
            ) : noProjects ? (
              <div id={`${id}-tok-h`} role="alert" className="mt-2 rounded-lg border border-amber-400/30 bg-amber-400/10 px-3 py-2" data-jira-no-projects>
                <p className="text-[12.5px] leading-[18px] text-amber-100">{noProjectsText(email.trim() || null)}</p>
                <button type="button" onClick={() => void runCheck()} className={`${textBtn} mt-1`}>
                  Check now
                </button>
              </div>
            ) : (
              <p id={`${id}-tok-h`} className={help}>
                {effKind === 'cloud' ? JIRA_COPY.cloudHelp : JIRA_COPY.dcHelp}
              </p>
            )}
            {slowDown && (
              <p role="status" className="mt-2 text-[12.5px] leading-[18px] text-amber-200" data-jira-slow>
                {slowDown}
              </p>
            )}
          </>
        )}
      </Step>
    ) : (
      <Step
        n={1}
        title="Your Jira"
        first
        done={
          <>
            <span className="truncate" data-jira-done>
              {host} · {effKind === 'cloud' && email ? email : 'token saved'}
            </span>
            <button type="button" className={`${textBtn} ml-1.5 shrink-0`} onClick={() => setStage(1)} data-jira-change>
              Change
            </button>
          </>
        }
      />
    );

  const live = check && (
    <div
      role="status"
      aria-live="polite"
      className="mt-3.5 flex gap-2.5 items-start px-3 py-2.5 rounded-[10px] bg-[var(--dv-code-bg,#0c0c10)] border border-white/[0.06]"
      data-jira-live
      data-count={check.count ?? undefined}
    >
      <span className={`mt-1.5 w-2 h-2 rounded-full shrink-0 ${counting ? 'bg-zinc-500 animate-pulse motion-reduce:animate-none' : check.count ? 'bg-teal-300' : 'bg-zinc-500'}`} aria-hidden="true" />
      <div className="flex-1 min-w-0">
        {check.jql_error ? (
          <>
            <b className="block text-[13px] leading-[19px] font-semibold text-rose-300">Jira couldn’t run this search.</b>
            <span className="block text-xs leading-[17px] text-zinc-400">{check.jql_error}</span>
          </>
        ) : (
          <>
            <b className="block text-[13px] leading-[19px] font-semibold text-zinc-100">{counting && check.count === null ? 'Counting…' : matchLine(check.count ?? 0)}</b>
            <span className="block text-xs leading-[17px] text-zinc-400">
              {check.count ? JIRA_COPY.waitNote : handover === 'label' && jql === null ? zeroMatchHelp(label) : 'Tickets handed over from now on start on their own.'}
            </span>
            {check.sample.length > 0 && (
              <ul className="mt-2 grid gap-1">
                {check.sample.map((s) => (
                  <li key={s.key} className="flex gap-2 text-[12.5px] leading-[18px] text-zinc-200 min-w-0">
                    <span className="shrink-0 inline-flex items-center h-[18px] px-1.5 rounded-md bg-zinc-950 border border-white/[0.06] font-mono text-[11px] font-semibold text-[#b4a9ff]">
                      {s.key}
                    </span>
                    <span className="truncate">{s.summary}</span>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>
    </div>
  );

  const step2 = (
    <Step n={2} title="Which tickets">
      <p className={`${fl} mt-0`} id={`${id}-pl`}>
        Projects
      </p>
      <div className="flex flex-wrap gap-1.5" role="group" aria-labelledby={`${id}-pl`} data-jira-projects>
        {shown.map((p) => (
          <Pick
            key={p.key}
            k={p.key}
            name={p.name}
            phone={phone}
            on={projects.includes(p.key)}
            sug={p.key === sugKey ? 'in your branch names' : null}
            onToggle={() => setProjects((ps) => (ps.includes(p.key) ? ps.filter((x) => x !== p.key) : [...ps, p.key]))}
          />
        ))}
        {!moreProjects && ordered.length > 3 && (
          <button type="button" className={`${textBtn} px-1`} onClick={() => setMoreProjects(true)} data-jira-more>
            +{ordered.length - 3} more
          </button>
        )}
      </div>
      <p className={fl} id={`${id}-hl`}>
        Hand a ticket over by
      </p>
      {jql !== null ? (
        <p className="text-[12.5px] text-zinc-300 flex items-center gap-1.5" data-jira-set-by-search>
          <span className="inline-flex items-center gap-1 text-[11.5px] text-teal-300">
            <Check size={11} aria-hidden="true" />
            Set by your search
          </span>
          <button type="button" className={textBtn} onClick={() => setJql(null)}>
            Use the choices again
          </button>
        </p>
      ) : (
        <div
          className={`grid ${phone ? 'grid-flow-row' : 'grid-flow-col auto-cols-fr'} gap-1 p-[3px] rounded-[10px] bg-zinc-950 border border-zinc-800`}
          role="radiogroup"
          aria-labelledby={`${id}-hl`}
          data-jira-handover={handover}
        >
          {(
            [
              [
                'label',
                <>
                  <TicketIcon size={13} aria-hidden="true" />
                  <span>
                    Adding the label <b className="font-mono font-semibold text-[#b4a9ff]">{label}</b>
                  </span>
                </>,
              ],
              [
                'assignee',
                <>
                  <Send size={13} aria-hidden="true" />
                  <span>Assigning it to a Jira user</span>
                </>,
              ],
            ] as const
          ).map(([v, content]) => (
            <button
              key={v}
              type="button"
              role="radio"
              aria-checked={handover === v}
              tabIndex={handover === v ? 0 : -1}
              onClick={() => setHandover(v)}
              onKeyDown={(e) => {
                if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) {
                  e.preventDefault();
                  const next = handover === 'label' ? 'assignee' : 'label';
                  setHandover(next);
                  (e.currentTarget.parentElement?.querySelector(`[data-v="${next}"]`) as HTMLElement | null)?.focus();
                }
              }}
              data-v={v}
              className={`flex items-center justify-center gap-1.5 ${segH} px-2 py-1 rounded-[7px] text-[12.5px] leading-4 text-center focus:outline-none focus-visible:ring-2 focus-visible:ring-[#9d8fff]/70 ${handover === v ? 'bg-zinc-800 text-zinc-100 shadow-[inset_0_0_0_1px_rgba(157,143,255,.55)]' : 'text-zinc-300 hover:text-zinc-100'}`}
            >
              {content}
            </button>
          ))}
        </div>
      )}
      {jql === null && handover === 'label' && <p className={help}>{JIRA_COPY.handoverHelp}</p>}
      {jql === null && handover === 'assignee' && (
        <div className="mt-2" data-jira-assignee>
          {assignee ? (
            <p className="flex items-center gap-2 text-[13px] text-zinc-100">
              <span className="truncate">{assignee.name}</span>
              <button type="button" className={textBtn} onClick={() => setAssignee(null)}>
                Change
              </button>
            </p>
          ) : (
            <>
              <label htmlFor={`${id}-user`} className="sr-only">
                Jira user
              </label>
              <input
                id={`${id}-user`}
                value={userQuery}
                onChange={(e) => setUserQuery(e.target.value)}
                placeholder="Search Jira users, like a bot account"
                autoComplete="off"
                className={inp(false)}
                data-jira-user-search
              />
              {users.length > 0 && (
                <ul className="mt-1 rounded-lg border border-zinc-800 bg-zinc-950 py-1" role="listbox" aria-label="Jira users">
                  {users.map((u) => (
                    <li key={u.id} role="option" aria-selected={false}>
                      <button
                        type="button"
                        className="w-full text-left px-2.5 py-1.5 text-[13px] text-zinc-200 hover:bg-zinc-800"
                        onClick={() => (setAssignee({ id: u.id, name: u.display_name }), setUserQuery(''))}
                      >
                        {u.display_name}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <p className={help}>Assigned tickets leave the real owner’s list. Use a bot account in Jira for this.</p>
            </>
          )}
        </div>
      )}
      {teams.length > 0 && (
        <>
          <label htmlFor={`${id}-team`} className={fl}>
            Who takes them
          </label>
          <div className="relative">
            <select
              id={`${id}-team`}
              value={team?.id ?? ''}
              onChange={(e) => (setTeamId(e.target.value || null), setTeamTouched(true))}
              className="appearance-none w-full min-h-9 pl-2.5 pr-8 rounded-lg border border-zinc-800 bg-zinc-950 text-[13px] text-transparent focus:outline-none focus:border-[#9d8fff]"
              aria-describedby={team?.lead ? `${id}-team-lead` : undefined}
              data-jira-team
            >
              <option value="" className="text-zinc-100 bg-zinc-900">
                The space’s lead bot
              </option>
              {teams.map((t) => (
                <option key={t.id} value={t.id} className="text-zinc-100 bg-zinc-900">
                  @{t.name}
                </option>
              ))}
            </select>
            {/* What the closed picker shows (design desk-07): the team's faces, its name and who leads. */}
            <span className="pointer-events-none absolute inset-y-0 left-2.5 right-8 flex items-center gap-2 min-w-0 text-[13px] text-zinc-100" aria-hidden="true" data-jira-team-face>
              {team?.members?.length ? <TeamFaces bots={team.members} size={18} /> : null}
              <span className="shrink-0">{team ? `@${team.name}` : 'The space’s lead bot'}</span>
              {team?.lead && !phone && <span className="truncate text-[12px] text-zinc-500">· {leadLine(team.lead.name)}</span>}
            </span>
            {team?.lead && (
              <span id={`${id}-team-lead`} className="sr-only">
                {leadLine(team.lead.name)}
              </span>
            )}
            <ChevronDown size={14} className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-400" aria-hidden="true" />
          </div>
          {team?.lead && phone && (
            <p className={help} aria-hidden="true" data-jira-team-lead>
              {leadLine(team.lead.name)}
            </p>
          )}
          {sug?.why && team?.id === sug.team.id && (
            <p className={help} data-jira-team-why>
              <span className="inline-flex items-center gap-1 text-[11.5px] text-teal-300">
                <Check size={11} aria-hidden="true" />
                Suggested
              </span>{' '}
              {sug.why}
            </p>
          )}
        </>
      )}
      {live}
    </Step>
  );

  const step3 = (
    <Step n={3} title="More context">
      <CheckRow sw name="confluence" on={conf} onChange={setConf} title="Also read Confluence" text={JIRA_COPY.confluence} />
      {conf && (
        <div className={phone ? 'ml-0' : 'ml-[46px]'}>
          {effKind === 'cloud' ? (
            <p className="mb-2 text-xs leading-[17px] text-zinc-400">{JIRA_COPY.confluenceCloud}</p>
          ) : (
            <div className="mb-2" data-jira-confluence-dc>
              <label htmlFor={`${id}-curl`} className={`${fl} mt-0`}>
                Confluence address
              </label>
              <input
                id={`${id}-curl`}
                value={confUrl}
                onChange={(e) => setConfUrl(e.target.value)}
                placeholder="confluence.yourcompany.com"
                autoComplete="off"
                spellCheck={false}
                className={inp(false)}
                data-jira-confluence-url
              />
              <label htmlFor={`${id}-ctok`} className={fl}>
                Confluence personal access token
              </label>
              <input
                id={`${id}-ctok`}
                type="password"
                onChange={(e) => (confToken.current = e.target.value)}
                onBlur={() =>
                  confToken.current &&
                  void jiraApi.check(space, form()).then(
                    (c) => c.result === 'found' && setCheck(c),
                    () => undefined,
                  )
                }
                placeholder={confNeedsToken ? 'Paste the personal access token' : 'Saved'}
                autoComplete="off"
                className={inp(false)}
                data-jira-confluence-token
              />
              <p className={help}>{JIRA_COPY.confluenceDc}</p>
            </div>
          )}
          {confSpaces.length > 0 && (
            <div className="flex flex-wrap gap-1.5" role="group" aria-label="Confluence spaces" data-jira-spaces>
              {confSpaces.map((s) => (
                <Pick
                  key={s.key}
                  k={s.key}
                  name={s.name}
                  phone={phone}
                  on={spaces.includes(s.key)}
                  onToggle={() => setSpaces((xs) => (xs.includes(s.key) ? xs.filter((x) => x !== s.key) : [...xs, s.key]))}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </Step>
  );

  const step4 = (
    <Step n={4} title="How much your bots do on their own">
      <CheckRow name="wait" on={waitOk} onChange={setWaitOk} title="Wait for my OK before building" text={waitForOkText(lead?.name)} />
      <CheckRow name="comment" on={comment} onChange={setComment} title="Comment on the ticket when work starts and when the PR is ready" text={JIRA_COPY.comment} />
      <details className="group mt-3 border-t border-white/[0.06] pt-2.5" data-jira-advanced open={jql !== null || undefined}>
        <summary className="list-none [&::-webkit-details-marker]:hidden inline-flex items-center gap-1 min-h-7 text-[12.5px] text-zinc-300 cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-[#9d8fff]/70 rounded">
          <ChevronRight size={12} className="transition-transform group-open:rotate-90 motion-reduce:transition-none" aria-hidden="true" />
          Advanced: the Jira search your bots run
        </summary>
        <label htmlFor={`${id}-jql`} className="sr-only">
          The Jira search
        </label>
        <textarea
          id={`${id}-jql`}
          value={rule}
          onChange={(e) => setJql(e.target.value === autoJql ? null : e.target.value)}
          rows={3}
          spellCheck={false}
          className="mt-2 w-full px-2.5 py-2 rounded-lg bg-zinc-950 border border-zinc-800 font-mono text-xs leading-[18px] text-zinc-200 [font-variant-ligatures:none] resize-y focus:outline-none focus:border-[#9d8fff]"
          data-jira-jql
        />
        <p className={help}>{JIRA_COPY.advancedHelp}</p>
        <label htmlFor={`${id}-poll`} className={fl}>
          Look for new tickets every
        </label>
        <div className="relative w-40">
          <select
            id={`${id}-poll`}
            value={poll}
            onChange={(e) => setPoll(Number(e.target.value))}
            className="appearance-none w-full min-h-9 pl-2.5 pr-8 rounded-lg border border-zinc-800 bg-zinc-950 text-[13px] text-zinc-100 focus:outline-none focus:border-[#9d8fff]"
            data-jira-poll
          >
            {POLLS.map((p) => (
              <option key={p.v} value={p.v}>
                {p.label}
              </option>
            ))}
          </select>
          <ChevronDown size={14} className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-400" aria-hidden="true" />
        </div>
      </details>
    </Step>
  );

  return (
    <>
      <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-4 pt-4 pb-4" data-jira-setup data-stage={stage}>
        <h3 className="m-0 flex items-center gap-2 text-[15px] leading-[22px] font-semibold text-zinc-100">
          <TicketIcon size={16} className="text-[#b4a9ff]" aria-hidden="true" />
          {JIRA_COPY.title}
          <button type="button" aria-label="Close" onClick={onCancel} className="ml-auto w-7 h-7 rounded-lg grid place-items-center text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800">
            <X size={16} aria-hidden="true" />
          </button>
        </h3>
        <p className="mt-0.5 text-[12.5px] leading-[18px] text-zinc-400">{JIRA_COPY.sub}</p>
        {step1}
        {stage === 2 && (
          <>
            {step2}
            {step3}
            {step4}
          </>
        )}
      </div>
      <div
        className={`flex ${phone ? 'flex-wrap' : ''} items-center gap-2 px-4 py-3 border-t border-white/[0.06] bg-zinc-900 ${phone ? 'pb-[calc(0.75rem+env(safe-area-inset-bottom))] rounded-none' : 'rounded-b-xl'}`}
        data-jira-foot
      >
        {/* Phone (shots-review 18): no "Change any of this later" note, which Settings already says; Cancel and the primary share the row at 44 px. */}
        {(() => {
          const note = saveErr ? <span className="text-rose-300">{saveErr}</span> : stage === 1 ? checking ? JIRA_COPY.checking : JIRA_COPY.nothingSaved : phone ? null : JIRA_COPY.laterNote;
          return note || !phone ? (
            <span className={`${phone ? 'basis-full' : 'flex-1'} text-[11.5px] leading-4 text-zinc-400`} role={saveErr ? 'alert' : undefined}>
              {note}
            </span>
          ) : null;
        })()}
        <button type="button" className={phone ? `${ghostBtn.replace('h-9', 'h-11')} flex-1 justify-center` : ghostBtn} onClick={onCancel} data-jira-cancel>
          Cancel
        </button>
        {stage === 1 ? (
          <button
            type="button"
            className={phone ? `${primaryBtn.replace('h-9', 'h-11')} flex-1 justify-center` : primaryBtn}
            disabled={checking || !kind || !!addrErr || (!showToken && !site)}
            onClick={() => void runCheck()}
            data-jira-check
            aria-disabled={!hasToken && !site ? true : undefined}
          >
            {checking && <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            {checking ? 'Checking' : 'Check and continue'}
          </button>
        ) : (
          <button type="button" className={phone ? `${primaryBtn.replace('h-9', 'h-11')} flex-1 justify-center` : primaryBtn} disabled={saving} onClick={() => void save()} data-jira-save>
            {saving && <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
            {site ? 'Save' : 'Start taking tickets'}
          </button>
        )}
      </div>
    </>
  );
};
