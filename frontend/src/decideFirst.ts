// Proactive bots (contract: proactive-bots/contract.md). Bots decide reversible choices themselves
// and record them on the reply (meta.decisions) so the person can Change one; while a held reply
// is being decided, the chat shows "Team is deciding…" (SSE decide_updated); work done while the
// person was away arrives as a message with meta kind "work_ahead".
import { parseMetaObject } from './questions';
import { request } from './threads';

export interface Decision {
  index: number;
  question: string;
  options: string[];
  picked: string;
  why: string;
  status: 'decided' | 'changed';
  changed_to: string | null;
}

const str = (v: unknown): string => (typeof v === 'string' ? v : '');

/** A reply's decisions, in the order the bot made them; [] for anything else. */
export function parseDecisions(raw: unknown): Decision[] {
  const m = parseMetaObject(raw);
  if (!m || !Array.isArray(m.decisions)) return [];
  const out: Decision[] = [];
  m.decisions.forEach((d: unknown, i: number) => {
    if (!d || typeof d !== 'object') return;
    const r = d as Record<string, unknown>;
    const options = Array.isArray(r.options) ? r.options.filter((o): o is string => typeof o === 'string' && o.trim() !== '') : [];
    const picked = str(r.picked);
    if (!picked) return;
    const changedTo = typeof r.changed_to === 'string' && r.changed_to ? r.changed_to : null;
    out.push({
      index: typeof r.index === 'number' && Number.isInteger(r.index) ? r.index : i,
      question: str(r.question),
      options,
      picked,
      why: str(r.why),
      status: r.status === 'changed' && changedTo ? 'changed' : 'decided',
      changed_to: changedTo,
    });
  });
  return out;
}

/** What the chip says: the current choice. */
export const decisionLabel = (d: Decision): string => (d.status === 'changed' ? `You chose: ${d.changed_to}` : `Decided: ${d.picked}`);

/** The options the person can switch to: every other one. Empty means no Change button. */
export const changeOptions = (d: Decision): string[] => (d.status === 'changed' ? [] : d.options.filter((o) => o !== d.picked));

/** Teammates the bot asked before deciding, when the reply came from the decide-first gate. */
export function consultedNames(raw: unknown): string[] {
  const df = parseMetaObject(raw)?.decide_first;
  if (!df || typeof df !== 'object') return [];
  const c = (df as Record<string, unknown>).consulted;
  return Array.isArray(c) ? c.filter((n): n is string => typeof n === 'string' && n.trim() !== '') : [];
}

/** "Asked Designer and Writer first." */
export function consultedLine(names: string[]): string {
  if (!names.length) return '';
  const list = names.length === 1 ? names[0] : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  return `Asked ${list} first.`;
}

export interface ChangeResult<M> {
  message: M;
  prompt: string;
}

export const decisionsApi = {
  /** Records the person's new pick; the client then sends `prompt` to the same chat. */
  change: <M>(messageId: string, index: number, picked: string) =>
    request<ChangeResult<M>>('POST', `/api/messages/${encodeURIComponent(messageId)}/decisions/${index}/change`, { picked }),
};

// ---------------------------------------------------------------------------
// "Team is deciding…"

export interface Deciding {
  kwantId: string;
  heldTurnId: string | null;
  consulted: string[];
}

/** Chats where a held reply is being decided, by context id. */
export type DecidingMap = Readonly<Record<string, Deciding>>;

/** Applies a `decide_updated` event; returns the same map for anything else. */
export function applyDecideEvent(map: DecidingMap, ev: { type?: string; data?: any }): DecidingMap {
  if (ev.type !== 'decide_updated' && ev.type !== 'DecideUpdated') return map;
  const d = ev.data ?? {};
  const ctx: unknown = d.context_id ?? d.thread_id ?? d.kwant_id;
  if (typeof ctx !== 'string' || !ctx) return map;
  if (d.state === 'deciding') {
    return {
      ...map,
      [ctx]: {
        kwantId: str(d.kwant_id),
        heldTurnId: typeof d.held_turn_id === 'string' ? d.held_turn_id : null,
        consulted: Array.isArray(d.consulted) ? d.consulted.filter((n: unknown): n is string => typeof n === 'string') : (map[ctx]?.consulted ?? []),
      },
    };
  }
  if (!(ctx in map)) return map;
  const next = { ...map };
  delete next[ctx];
  return next;
}

/** After a (re)connect: chats with a consult or decide turn running (`purpose: "decide"`). */
export function hydrateDeciding(turns: Array<{ context_id?: string; kwant_id?: string; purpose?: string }> | undefined): DecidingMap {
  const out: Record<string, Deciding> = {};
  for (const t of turns ?? []) {
    if (t?.purpose !== 'decide' || !t.context_id || out[t.context_id]) continue;
    out[t.context_id] = { kwantId: t.kwant_id ?? '', heldTurnId: null, consulted: [] };
  }
  return out;
}

/** The line under "Team is deciding…". */
export function decidingDetail(botName: string, consulted: string[]): string {
  const who = botName || 'The bot';
  if (!consulted.length) return `${who} is running the checks it listed and making the call, instead of asking you.`;
  return `${who} is checking with ${consultedLine(consulted).replace(/^Asked /, '').replace(/ first\.$/, '')} and making the call, instead of asking you.`;
}

// ---------------------------------------------------------------------------
// "While you were away"

export interface WorkAheadMeta {
  kind: 'work_ahead';
  date: string;
  run_id: string;
}

export function parseWorkAhead(raw: unknown): WorkAheadMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'work_ahead') return null;
  return { kind: 'work_ahead', date: str(m.date), run_id: str(m.run_id) };
}

/** One plain line for the collapsed message: the first sentence-ish line without markdown. */
export function workAheadSummary(text: string, max = 140): string {
  const line =
    text
      .split('\n')
      .map((l) => l.replace(/^[#>*\-\s]+/, '').replace(/[*_`]/g, '').trim())
      .find(Boolean) ?? '';
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

// ---------------------------------------------------------------------------
// ask_user cards: why only the person can answer

export type WhyYou = 'irreversible' | 'outside' | 'money' | 'secret_or_signin' | 'taste_no_signal' | 'missing_after_research';

const WHY_YOU: Record<WhyYou, string> = {
  irreversible: 'It can’t be undone.',
  outside: 'It sends something or contacts someone.',
  money: 'It spends money.',
  secret_or_signin: 'It needs a password or a sign-in.',
  taste_no_signal: 'It’s a matter of taste and there’s nothing to go on yet.',
  missing_after_research: 'The bot looked and couldn’t find it.',
};

export const whyYouText = (v: unknown): string | null => (typeof v === 'string' && v in WHY_YOU ? WHY_YOU[v as WhyYou] : null);

// ---------------------------------------------------------------------------
// App settings

export const WORK_AHEAD_CAP_MIN = 1;
export const WORK_AHEAD_CAP_MAX = 10;

export interface ProactiveSettings {
  decide_first: boolean;
  work_ahead: boolean;
  work_ahead_daily_cap: number;
}

/** The three settings from GET /api/settings; null on a server without them. */
export function parseProactiveSettings(data: unknown): ProactiveSettings | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  if (typeof d.decide_first !== 'boolean') return null;
  const cap = typeof d.work_ahead_daily_cap === 'number' && Number.isInteger(d.work_ahead_daily_cap) ? d.work_ahead_daily_cap : 3;
  return {
    decide_first: d.decide_first,
    work_ahead: typeof d.work_ahead === 'boolean' ? d.work_ahead : true,
    work_ahead_daily_cap: Math.min(WORK_AHEAD_CAP_MAX, Math.max(WORK_AHEAD_CAP_MIN, cap)),
  };
}
