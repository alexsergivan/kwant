// 0.9.0 P1: watch a mission. Design: release-090/designs/p1-watch-mission.md (Mira). Backend: Vega,
// contract release-090/p1-contract.md. When every task in a mission has stopped, one bot (the watcher)
// treats each status as a claim, checks it once, restarts what doesn't hold and names the person only
// where a claim needs them. This file is the typed client and the pure logic the views draw from:
// chip state and copy, the default watcher, report row order, the chat and Inbox lines.
//
// Nothing here works out a verdict or a count the server didn't send: counts are the server's, and a
// report whose counts don't match its rows, or that doesn't parse, is dropped (the chat then shows the
// message text as written).

import { PersonCancelledError, personFetch } from './auth';
import { parseMetaObject } from './questions';
import type { Mission } from './missions';

export type WatchState =
  | 'off'
  | 'no_tasks'
  | 'finished'
  | 'watching'
  | 'checking'
  | 'dont_hold'
  | 'all_hold'
  | 'cant_tell'
  | 'same_stop'
  | 'cant_run'
  | 'over_limit'
  | 'failed';
export type Verdict = 'holds' | 'doesnt_hold' | 'cant_tell';
export type CheckStatus = 'checking' | 'done' | 'failed' | 'cant_run' | 'over_limit';

/** The server's counts (contract section 4). Shown as sent; never added up here. */
export interface WatchCounts {
  claims: number;
  dont_hold: number;
  needs_you: number;
  cant_tell: number;
  holds: number;
}

export interface WatchReason {
  code: string;
  text: string;
}

export interface WatchClaim {
  task_id: string;
  task_title: string;
  owner_kwant_id: string | null;
  owner_name: string | null;
  /** The claim as the server words it: "In review", "Blocked: …", "Waiting for your OK". */
  claimed: string;
  /** What the watcher checked; may hold `code` spans. */
  checked: string;
  verdict: Verdict;
  /** A needs-you action names this task. Not a verdict. */
  needs_you: boolean;
}

export type WatchAction =
  | { kind: 'restart'; task_id: string; bot_id: string | null; bot_name: string; reason: string; applied: boolean; why: string | null }
  | { kind: 'ask'; task_id: string; bot_id: string | null; bot_name: string; what: string; applied: boolean; why: string | null }
  | { kind: 'needs_you'; task_id: string; what: string; since: string | null }
  | { kind: 'refused'; what: string; tool: string | null; why: string | null };
export type NeedsYouAction = Extract<WatchAction, { kind: 'needs_you' }>;

/** One check: the report (contract section 4). */
export interface WatchReport {
  id: string;
  mission_id: string;
  kwant_id: string | null;
  kwant_name: string;
  trigger: 'stop' | 'manual';
  status: CheckStatus;
  stopped_at: string | null;
  started_at: string | null;
  finished_at: string | null;
  error: string | null;
  reason: WatchReason | null;
  counts: WatchCounts | null;
  claims: WatchClaim[];
  actions: WatchAction[];
  inbox: { open: boolean; line: string } | null;
  message_id: string | null;
}

/** The newest check as the mission payload carries it. */
export interface WatchCheckSummary {
  id: string;
  status: CheckStatus;
  trigger: 'stop' | 'manual';
  started_at: string | null;
  finished_at: string | null;
  stopped_at: string | null;
  counts: WatchCounts | null;
  message_id: string | null;
}

/** `mission.watch` (contract section 1). The chip is drawn from this alone. */
export interface MissionWatch {
  state: WatchState;
  kwant_id: string | null;
  kwant_name: string | null;
  set_at: string | null;
  stopped: boolean;
  stopped_at: string | null;
  last_check: WatchCheckSummary | null;
  reason: WatchReason | null;
  can_check_now: boolean;
}

export type Relation = 'no_task' | 'lead' | 'has_task';
export interface WatchCandidate {
  kwant_id: string;
  name: string;
  title: string | null;
  relation: Relation;
  task_ids: string[];
  can_run: boolean;
  reason: WatchReason | null;
  /** The server's default pick, or the current watcher when a watch is set. */
  default: boolean;
}

/** A mission as the board has it, with the watch the server adds (absent on an older server). */
export type WatchedMission = Mission & { watch?: MissionWatch | null };

// ---------- Parsing (data from the server is checked, never trusted) ----------

const STATES: WatchState[] = ['off', 'no_tasks', 'finished', 'watching', 'checking', 'dont_hold', 'all_hold', 'cant_tell', 'same_stop', 'cant_run', 'over_limit', 'failed'];
const VERDICTS: Verdict[] = ['holds', 'doesnt_hold', 'cant_tell'];
const STATUSES: CheckStatus[] = ['checking', 'done', 'failed', 'cant_run', 'over_limit'];
const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);
const obj = (v: unknown): Record<string, unknown> | null => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const okTime = (v: unknown): string | null => {
  const s = str(v);
  return s && Number.isFinite(Date.parse(s)) ? s : null;
};
const count = (v: unknown): number | null => (typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : null);

function parseCounts(raw: unknown): WatchCounts | null {
  const c = obj(raw);
  if (!c) return null;
  const k = { claims: count(c.claims), dont_hold: count(c.dont_hold), needs_you: count(c.needs_you), cant_tell: count(c.cant_tell), holds: count(c.holds) };
  return Object.values(k).every((n) => n !== null) ? (k as WatchCounts) : null;
}

function parseReason(raw: unknown): WatchReason | null {
  const r = obj(raw);
  const text = r && str(r.text);
  return r && text ? { code: str(r.code) ?? 'unknown', text } : null;
}

function parseClaim(raw: unknown): WatchClaim | null {
  const c = obj(raw);
  if (!c) return null;
  const task_id = str(c.task_id);
  const task_title = str(c.task_title);
  const claimed = str(c.claimed);
  const checked = str(c.checked);
  if (!task_id || !task_title || !claimed || !checked || !VERDICTS.includes(c.verdict as Verdict)) return null;
  return { task_id, task_title, owner_kwant_id: str(c.owner_kwant_id), owner_name: str(c.owner_name), claimed, checked, verdict: c.verdict as Verdict, needs_you: c.needs_you === true };
}

function parseAction(raw: unknown): WatchAction | null {
  const a = obj(raw);
  if (!a) return null;
  const task_id = str(a.task_id);
  const applied = a.applied !== false;
  const why = str(a.why);
  switch (a.kind) {
    case 'restart': {
      const bot_name = str(a.bot_name);
      const reason = str(a.reason);
      return task_id && bot_name && reason ? { kind: 'restart', task_id, bot_id: str(a.bot_id), bot_name, reason, applied, why } : null;
    }
    case 'ask': {
      const bot_name = str(a.bot_name);
      const what = str(a.what);
      return task_id && bot_name && what ? { kind: 'ask', task_id, bot_id: str(a.bot_id), bot_name, what, applied, why } : null;
    }
    case 'needs_you': {
      const what = str(a.what);
      return task_id && what ? { kind: 'needs_you', task_id, what, since: okTime(a.since) } : null;
    }
    case 'refused': {
      const what = str(a.what);
      return what ? { kind: 'refused', what, tool: str(a.tool), why } : null;
    }
    default:
      return null;
  }
}

/**
 * The counts must say what the rows say, or the table and the chips would disagree. The contract
 * reads two ways on whether `holds` includes the needs-you rows; both are accepted.
 */
function countsMatch(k: WatchCounts, claims: WatchClaim[]): boolean {
  const n = (f: (c: WatchClaim) => boolean) => claims.filter(f).length;
  const holds = n((c) => c.verdict === 'holds');
  const holdsOnly = n((c) => c.verdict === 'holds' && !c.needs_you);
  return (
    k.claims === claims.length &&
    k.dont_hold === n((c) => c.verdict === 'doesnt_hold') &&
    k.cant_tell === n((c) => c.verdict === 'cant_tell') &&
    k.needs_you === n((c) => c.needs_you) &&
    (k.holds === holds || k.holds === holdsOnly)
  );
}

/**
 * A check we can draw as a report, or null: a finished (`done`) check needs its counts and every
 * claim row to parse, and the counts to match the rows. A failed check parses with no claims.
 * Unknown action kinds are skipped; they're notes about what the watcher did, not claims.
 */
export function parseReport(raw: unknown): WatchReport | null {
  const r = obj(raw);
  if (!r) return null;
  const id = str(r.id);
  const mission_id = str(r.mission_id);
  const kwant_name = str(r.kwant_name);
  const status = r.status as CheckStatus;
  if (!id || !mission_id || !kwant_name || !STATUSES.includes(status)) return null;
  const rawClaims = Array.isArray(r.claims) ? r.claims : [];
  const claims = rawClaims.map(parseClaim);
  if (claims.some((c) => c === null)) return null;
  const counts = parseCounts(r.counts);
  if (status === 'done' && (!counts || claims.length === 0 || !countsMatch(counts, claims as WatchClaim[]))) return null;
  const inbox = obj(r.inbox);
  return {
    id,
    mission_id,
    kwant_id: str(r.kwant_id),
    kwant_name,
    trigger: r.trigger === 'manual' ? 'manual' : 'stop',
    status,
    stopped_at: okTime(r.stopped_at),
    started_at: okTime(r.started_at),
    finished_at: okTime(r.finished_at),
    error: str(r.error),
    reason: parseReason(r.reason),
    counts,
    claims: claims as WatchClaim[],
    actions: Array.isArray(r.actions) ? r.actions.map(parseAction).filter((a): a is WatchAction => a !== null) : [],
    inbox: inbox && str(inbox.line) ? { open: inbox.open === true, line: str(inbox.line)! } : null,
    message_id: str(r.message_id),
  };
}

/** Chat message meta `{kind: "watch", status, watch_report}`, or null for any other message. */
export interface WatchMeta { report: WatchReport | null; failed: string | null; missionId: string | null; checkId: string | null }
export function parseWatchMeta(raw: unknown): WatchMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'watch') return null;
  const report = parseReport(m.watch_report);
  const failed = m.status === 'failed' ? report?.reason?.text ?? report?.error ?? null : null;
  return { report: m.status === 'failed' ? null : report, failed, missionId: str(m.mission_id), checkId: str(m.check_id) };
}

function parseCheckSummary(raw: unknown): WatchCheckSummary | null {
  const c = obj(raw);
  const id = c && str(c.id);
  if (!c || !id || !STATUSES.includes(c.status as CheckStatus)) return null;
  return {
    id,
    status: c.status as CheckStatus,
    trigger: c.trigger === 'manual' ? 'manual' : 'stop',
    started_at: okTime(c.started_at),
    finished_at: okTime(c.finished_at),
    stopped_at: okTime(c.stopped_at),
    counts: parseCounts(c.counts),
    message_id: str(c.message_id),
  };
}

export function parseWatch(raw: unknown): MissionWatch | null {
  const w = obj(raw);
  if (!w || !STATES.includes(w.state as WatchState)) return null;
  return {
    state: w.state as WatchState,
    kwant_id: str(w.kwant_id),
    kwant_name: str(w.kwant_name),
    set_at: okTime(w.set_at),
    stopped: w.stopped === true,
    stopped_at: okTime(w.stopped_at),
    last_check: parseCheckSummary(w.last_check),
    reason: parseReason(w.reason),
    can_check_now: w.can_check_now === true,
  };
}

export function parseCandidates(raw: unknown): WatchCandidate[] {
  if (!Array.isArray(raw)) return [];
  const out: WatchCandidate[] = [];
  for (const x of raw) {
    const c = obj(x);
    const id = c && str(c.kwant_id);
    const name = c && str(c.name);
    if (!c || !id || !name || !['no_task', 'lead', 'has_task'].includes(c.relation as string)) continue;
    out.push({
      kwant_id: id,
      name,
      title: str(c.title),
      relation: c.relation as Relation,
      task_ids: Array.isArray(c.task_ids) ? c.task_ids.filter((t): t is string => typeof t === 'string') : [],
      can_run: c.can_run !== false,
      reason: parseReason(c.reason),
      default: c.default === true,
    });
  }
  return out;
}

// ---------- Times ----------

/** 24-hour HH:MM in the person's zone (or the one given, for tests). */
export function hm(at: string | null | undefined, timeZone?: string): string {
  if (!at) return '';
  const d = new Date(at);
  if (!Number.isFinite(d.getTime())) return '';
  return new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23', ...(timeZone ? { timeZone } : {}) }).format(d);
}

// ---------- Order and rows ----------

/** Rank: doesn't hold, then needs you, then can't tell, then holds. */
export function claimRank(c: WatchClaim): number {
  if (c.verdict === 'doesnt_hold') return 0;
  if (c.needs_you) return 1;
  if (c.verdict === 'cant_tell') return 2;
  return 3;
}

/**
 * The rows in the design's order (the server sends them sorted; this keeps that order within a rank),
 * split into what shows and what folds into "Show {n} that hold".
 */
export function orderClaims(r: WatchReport): { shown: WatchClaim[]; folded: WatchClaim[] } {
  const sorted = r.claims.map((c, i) => [c, i] as const).sort((a, b) => claimRank(a[0]) - claimRank(b[0]) || a[1] - b[1]).map(([c]) => c);
  return { shown: sorted.filter((c) => claimRank(c) < 3), folded: sorted.filter((c) => claimRank(c) === 3) };
}

export type RowTint = 'rose' | 'amber' | null;
export const rowTint = (c: WatchClaim): RowTint => (c.verdict === 'doesnt_hold' ? 'rose' : c.needs_you ? 'amber' : null);

/** Summary chips in the design's order, skipping zeros. Numbers are the server's counts. */
export function summaryChips(k: WatchCounts): { tone: 'rose' | 'amber' | 'zinc' | 'teal'; text: string }[] {
  return [
    { tone: 'rose' as const, n: k.dont_hold, text: `${k.dont_hold} don’t hold` },
    { tone: 'amber' as const, n: k.needs_you, text: `${k.needs_you} needs you` },
    { tone: 'zinc' as const, n: k.cant_tell, text: `${k.cant_tell} can’t tell` },
    { tone: 'teal' as const, n: k.holds, text: `${k.holds} hold` },
  ]
    .filter((c) => c.n > 0)
    .map(({ tone, text }) => ({ tone, text }));
}

export const VERDICT_TEXT: Record<Verdict, string> = { holds: 'Holds', doesnt_hold: 'Doesn’t hold', cant_tell: 'Can’t tell' };

/** A task's title in the report, by id; actions name tasks by id only. */
export const taskTitle = (r: WatchReport, taskId: string): string => r.claims.find((c) => c.task_id === taskId)?.task_title ?? 'a task';

/** Split "Ran `npm test` on `x`" into text and code parts. Text only: nothing becomes HTML. */
export function codeParts(s: string): { code: boolean; text: string }[] {
  const out: { code: boolean; text: string }[] = [];
  const re = /`([^`\n]+)`/g;
  let at = 0;
  for (let m = re.exec(s); m; m = re.exec(s)) {
    if (m.index > at) out.push({ code: false, text: s.slice(at, m.index) });
    out.push({ code: true, text: m[1] });
    at = m.index + m[0].length;
  }
  if (at < s.length) out.push({ code: false, text: s.slice(at) });
  return out;
}

// ---------- The chip ----------

export type ChipTone = 'ghost' | 'accent' | 'rose' | 'teal' | 'muted' | 'amber';
export type ChipIcon = 'eye' | 'loader' | 'alert-triangle' | 'alert-circle';
export interface ChipView {
  /** What the person sees on it (the watcher's avatar sits after "Watching · "). */
  text: string;
  tone: ChipTone;
  icon: ChipIcon;
  /** Screen-reader label: the text, plus ", watch settings" where the click opens them. */
  label: string;
  /** What a click does. */
  click: 'setup' | 'settings' | 'report' | 'retry' | 'none';
  disabled?: boolean;
  /** Disabled reason, shown as its tooltip. */
  tooltip?: string;
  /** Watching: show the avatar of this bot. */
  avatar?: { kwant_id: string | null; name: string } | null;
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);

/**
 * The chip for a mission from `mission.watch.state`, or null when it hides (finished, or a draft
 * that isn't running yet). An older server that sends no watch gets the plain Watch chip.
 */
export function chipView(m: WatchedMission, timeZone?: string): ChipView | null {
  if (m.status === 'draft' || m.status === 'done' || m.status === 'cancelled') return null;
  const w = m.watch ?? null;
  const off: ChipView = { text: 'Watch', tone: 'ghost', icon: 'eye', label: 'Watch, watch settings', click: 'setup' };
  if (!w) return off;
  const name = w.kwant_name ?? 'The watcher';
  const lc = w.last_check;
  const at = hm(lc?.finished_at ?? lc?.started_at, timeZone);
  const k = lc?.counts ?? null;
  const settings = (text: string, tone: ChipTone, icon: ChipIcon = 'eye'): ChipView => ({ text, tone, icon, label: `${text}, watch settings`, click: 'settings' });
  const watching: ChipView = { ...settings(`Watching · ${name}`, 'accent'), avatar: { kwant_id: w.kwant_id, name } };
  switch (w.state) {
    case 'finished':
      return null;
    case 'no_tasks': {
      const tip = 'Add tasks first. There’s nothing to check yet.';
      return { ...off, disabled: true, click: 'none', tooltip: tip, label: `Watch, watch settings. ${tip}` };
    }
    case 'off':
      return off;
    case 'watching':
      return watching;
    case 'checking': {
      // Counts are null while checking, so the chip can't say how many.
      const text = `${name} is checking the claims`;
      return { text, tone: 'accent', icon: 'loader', label: `${text}, watch settings`, click: 'settings' };
    }
    case 'dont_hold': {
      if (!k) return watching;
      const text = `${k.dont_hold} ${plural(k.dont_hold, 'claim doesn’t hold', 'claims don’t hold')} · ${at}`;
      return { text, tone: 'rose', icon: 'eye', label: text, click: 'report' };
    }
    case 'all_hold': {
      if (!k) return watching;
      const text = `All ${k.claims} ${plural(k.claims, 'claim holds', 'claims hold')} · ${at}`;
      return { text, tone: 'teal', icon: 'eye', label: text, click: 'report' };
    }
    case 'cant_tell': {
      // No row in the design; the contract's text, muted. Never "All hold" over a "Can't tell".
      if (!k) return watching;
      const text = `${k.claims} ${plural(k.claims, 'claim', 'claims')} checked · ${at}`;
      return { text, tone: 'muted', icon: 'eye', label: text, click: 'report' };
    }
    case 'same_stop':
      return settings(`No change since the ${at} check`, 'muted');
    case 'cant_run':
      return settings(`${name} can’t check: ${w.reason?.text ?? 'it can’t run'}`, 'amber', 'alert-triangle');
    case 'over_limit':
      return settings('Over the mission’s limit', 'amber', 'alert-triangle');
    case 'failed':
      return w.can_check_now
        ? { text: 'The check failed · Try again', tone: 'rose', icon: 'alert-circle', label: 'The check failed. Try again', click: 'retry' }
        : settings('The check failed', 'rose', 'alert-circle');
  }
}

/** The report opens on its own after a check with a "doesn't hold" or a "needs you". */
export const reportOpensItself = (w: MissionWatch | null): boolean =>
  !!w && !!w.last_check?.counts && ['dont_hold', 'all_hold', 'cant_tell'].includes(w.state) && (w.last_check.counts.dont_hold > 0 || w.last_check.counts.needs_you > 0);

/** States with a report to show (the last check, greyed while checking again). */
export const hasReport = (w: MissionWatch | null): boolean => !!w?.last_check && w.state !== 'off' && w.state !== 'finished';

export const CHIP_TONES: Record<ChipTone, string> = {
  ghost: 'border-white/10 bg-white/[.03] text-zinc-200 hover:bg-white/[.06]',
  accent: 'border-[#9d8fff]/40 bg-[#8270f5]/12 text-[#d3ccff] hover:bg-[#8270f5]/20',
  rose: 'border-rose-300/40 bg-rose-300/8 text-rose-300 hover:bg-rose-300/15',
  teal: 'border-teal-400/35 bg-teal-400/8 text-teal-300 hover:bg-teal-400/15',
  muted: 'border-white/10 bg-white/[.03] text-zinc-400 hover:bg-white/[.06]',
  amber: 'border-amber-300/40 bg-amber-300/8 text-amber-300 hover:bg-amber-300/15',
};

// ---------- Setup ----------

export function relationText(c: WatchCandidate, phone = false): string {
  if (!c.can_run) return `can’t run: ${c.reason?.text ?? 'it can’t run now'}`;
  if (c.relation === 'lead') return phone ? 'leads it' : 'leads it, so it checks its own work';
  if (c.relation === 'has_task') return 'has a task here';
  return 'has no task in this mission';
}

/** The server marks the default (or the current watcher); else the first that can run, in its order. */
export function defaultCandidate(cs: WatchCandidate[]): WatchCandidate | null {
  const runs = cs.filter((c) => c.can_run);
  return runs.find((c) => c.default) ?? runs.find((c) => c.relation === 'no_task') ?? runs.find((c) => c.relation === 'lead') ?? runs[0] ?? null;
}

export const SETUP_BODY =
  'When every task has stopped, one bot checks each claim once: is “done” really done, is “blocked” still blocked? Then it restarts what doesn’t hold or tells you who has to act.';
export const SETUP_BODY_PHONE = 'When every task has stopped, one bot checks each claim once. Then it restarts what doesn’t hold or tells you who has to act.';
export const guardText = (bot: string) => `${bot} can restart tasks and ask bots to act. It can’t approve, spend, add bots or do your steps.`;
export const costText = (bot: string) => `Each check is one turn for ${bot} and counts toward this mission’s limit.`;
export const footerText = (bot: string) => `${bot} checks again only when the mission stops in a different state. It can’t approve, spend, add bots or do your steps.`;
export const REPORT_SUB = 'Every task had stopped: nothing running, waiting in line or due to retry. Each “done”, “blocked” and “waiting” is a claim, checked against what’s really there.';

// ---------- Chat message, the needs-you lines ----------

/** The watcher's message, as parts so "{n} don't hold" can be bold, with local times. Text only. */
export function chatParts(r: WatchReport, timeZone?: string): { bold?: boolean; text: string }[] {
  const k = r.counts;
  if (!k) return [];
  const head = `Checked ${k.claims} ${plural(k.claims, 'claim', 'claims')} after the mission stopped at ${hm(r.stopped_at, timeZone)}. `;
  if (k.dont_hold === 0 && k.cant_tell === 0 && k.needs_you === 0) return [{ text: `${head}All hold.` }];
  const parts: { bold?: boolean; text: string }[] = [{ text: head }];
  if (k.dont_hold) parts.push({ bold: true, text: `${k.dont_hold} don’t hold` });
  const rest = [k.needs_you && `${k.needs_you} needs you`, k.cant_tell && `${k.cant_tell} can’t tell`, k.holds && `${k.holds} hold`].filter(Boolean) as string[];
  if (rest.length) parts.push({ text: `${k.dont_hold ? ', ' : ''}${rest.join(', ')}` });
  parts.push({ text: '.' });
  const restarts = r.actions.filter((a): a is Extract<WatchAction, { kind: 'restart' }> => a.kind === 'restart' && a.applied);
  if (restarts.length) {
    const list = restarts.map((a) => `${taskTitle(r, a.task_id)} with ${a.bot_name}`);
    const joined = list.length === 1 ? list[0] : `${list.slice(0, -1).join(', ')} and ${list[list.length - 1]}`;
    parts.push({ text: ` I restarted ${joined}.` });
  }
  return parts;
}

export const chatText = (r: WatchReport, timeZone?: string) =>
  chatParts(r, timeZone)
    .map((p) => p.text)
    .join('');

export const failedChatText = (reason: string) => `The check failed: ${reason}. Nothing was restarted.`;

/** "Deploy the site waits for your OK to deploy since 13:55." (`what` is the watcher's phrase.) */
export const youText = (r: WatchReport, a: NeedsYouAction, timeZone?: string) => `${taskTitle(r, a.task_id)} ${a.what}${a.since ? ` since ${hm(a.since, timeZone)}` : ''}.`;

/** The phone's sticky bar. */
export const stickyText = (r: WatchReport, a: NeedsYouAction) => `${taskTitle(r, a.task_id)} ${a.what}`;

export const firstNeedsYou = (r: WatchReport): NeedsYouAction | null => r.actions.find((a): a is NeedsYouAction => a.kind === 'needs_you') ?? null;

// ---------- API ----------

export class WatchError extends Error {
  constructor(message: string, readonly status: number, readonly code: string | null = null) {
    super(message);
  }
}

async function call<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new WatchError(e.message, 401);
    throw new WatchError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const answer = (await res.json().catch(() => null)) as Record<string, unknown> | null;
  if (res.ok) return answer as T;
  const err = answer && typeof answer.error === 'string' ? answer.error : null;
  const code = answer && typeof answer.code === 'string' ? answer.code : null;
  if (res.status === 403) throw new WatchError('Only you can change a mission’s watch.', 403, code);
  if (res.status === 404 && !err) throw new WatchError('This server can’t watch missions yet. Update the app and try again.', 404, code);
  throw new WatchError((code && WATCH_ERRORS[code]) || err || `Something went wrong (HTTP ${res.status}).`, res.status, code);
}

const enc = encodeURIComponent;

export interface WatchApi {
  /** The watch, the last check and who could check, in the server's order. */
  get: (missionId: string) => Promise<{ watch: MissionWatch | null; report: WatchReport | null; candidates: WatchCandidate[] }>;
  /** Turn the watch on, or change who checks. */
  set: (missionId: string, kwantId: string) => Promise<MissionWatch | null>;
  stop: (missionId: string) => Promise<MissionWatch | null>;
  /** "Check again now" and "Try again": one new check, ignoring the last fingerprint. */
  checkNow: (missionId: string) => Promise<MissionWatch | null>;
  /** One check by id ("See the claims" in the chat). */
  check: (checkId: string) => Promise<WatchReport | null>;
  /** The mission's checks, newest first (for the last finished report while a new one runs). */
  checks?: (missionId: string, limit?: number) => Promise<WatchReport[]>;
}

const watchOf = (r: { watch?: unknown } | null) => parseWatch(r?.watch);

export const watchApi: WatchApi = {
  get: (id) =>
    call<{ watch?: unknown; report?: unknown; candidates?: unknown }>(`/api/missions/${enc(id)}/watch`).then((r) => ({
      watch: watchOf(r),
      report: parseReport(r?.report),
      candidates: parseCandidates(r?.candidates),
    })),
  set: (id, kwant_id) => call<{ watch?: unknown }>(`/api/missions/${enc(id)}/watch`, 'PUT', { kwant_id }).then(watchOf),
  stop: (id) => call<{ watch?: unknown }>(`/api/missions/${enc(id)}/watch`, 'DELETE').then(watchOf),
  checkNow: (id) => call<{ watch?: unknown }>(`/api/missions/${enc(id)}/watch/check`, 'POST', {}).then(watchOf),
  check: (checkId) => call<unknown>(`/api/watch-checks/${enc(checkId)}`).then(parseReport),
  checks: (id, limit = 10) =>
    call<{ checks?: unknown }>(`/api/missions/${enc(id)}/watch/checks?limit=${limit}`).then((r) =>
      (Array.isArray(r?.checks) ? r.checks : []).map(parseReport).filter((x): x is WatchReport => !!x),
    ),
};

/** The id of the newest check that has finished, or null while the newest one is still checking. */
export const finishedCheckId = (w: MissionWatch | null): string | null => (w?.last_check && w.last_check.status !== 'checking' ? w.last_check.id : null);

/** The mission's watch from the board payload, parsed; null when the server sends none. */
export const missionWatch = (m: Mission): MissionWatch | null => parseWatch((m as { watch?: unknown }).watch);

/** Error codes from the write routes, in the person's words where the server's text is terse. */
export const WATCH_ERRORS: Record<string, string> = {
  not_stopped: 'Tasks are still moving. The check starts when every task has stopped.',
  checking: 'A check is already running.',
  over_limit: 'This mission is over its limit. Raise or remove the limit first.',
  finished: 'This mission is finished. There’s nothing left to watch.',
  no_tasks: 'Add tasks first. There’s nothing to check yet.',
};
