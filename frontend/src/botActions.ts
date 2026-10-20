// Bots changing the app (bot actions) and proposing to hire a new bot. Each shows as a card in
// the chat; only the person can undo, confirm, decline, accept or deny.
// Contract: docs/bot-actions.md.
import { parseMetaObject } from './questions';
import { Schedule, WEEKDAYS, describeSchedule, formatWhen } from './continuity';
import { PersonCancelledError, personFetch } from './auth';

// ---------------------------------------------------------------------------
// Bot actions

export type BotActionStatus = 'applied' | 'undone' | 'pending' | 'confirmed' | 'declined';
const ACTION_STATUSES: BotActionStatus[] = ['applied', 'undone', 'pending', 'confirmed', 'declined'];

/** `meta` of a bot action card (`kind: "bot_action"`). */
export interface BotActionMeta {
  kind: 'bot_action';
  action_id: string;
  /** The tool that made it, e.g. `routine_create`. */
  action: string;
  kwant_id: string | null;
  target_kind: string;
  target_id: string | null;
  summary: string;
  details: Record<string, unknown>;
  status: BotActionStatus;
  can_undo: boolean;
  needs_confirmation: boolean;
  created_at: string | null;
  resolved_at: string | null;
}

/** A bot action from the API or `bot_action_updated`; `card` is its meta. */
export interface BotAction {
  id: string;
  status: string;
  message_id?: string | null;
  card?: unknown;
}

const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);

export function parseBotActionMeta(raw: unknown): BotActionMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'bot_action') return null;
  const id = str(m.action_id);
  if (!id) return null;
  const status = ACTION_STATUSES.includes(m.status as BotActionStatus) ? (m.status as BotActionStatus) : 'applied';
  return {
    kind: 'bot_action',
    action_id: id,
    action: str(m.action) ?? '',
    kwant_id: str(m.kwant_id),
    target_kind: str(m.target_kind) ?? '',
    target_id: str(m.target_id),
    summary: str(m.summary) ?? '',
    details: m.details && typeof m.details === 'object' && !Array.isArray(m.details) ? (m.details as Record<string, unknown>) : {},
    status,
    // A finished action never offers buttons, whatever the flags say.
    can_undo: m.can_undo === true && status === 'applied',
    needs_confirmation: m.needs_confirmation === true && status === 'pending',
    created_at: str(m.created_at),
    resolved_at: str(m.resolved_at),
  };
}

const ACTION_RANK: Record<BotActionStatus, number> = { pending: 0, applied: 1, undone: 2, confirmed: 2, declined: 2 };

/** Card state: the newer of the message's meta and the live card; a resolved card never goes back. */
export function botActionView(meta: BotActionMeta, live?: BotActionMeta | null): BotActionMeta {
  if (!live || live.action_id !== meta.action_id) return meta;
  return ACTION_RANK[live.status] >= ACTION_RANK[meta.status] ? live : meta;
}

/** Keeps the latest card per action from `bot_action_updated` and route replies. */
export function applyBotAction(map: Record<string, BotActionMeta>, a: BotAction | null | undefined): Record<string, BotActionMeta> {
  const card = parseBotActionMeta(a?.card);
  if (!card) return map;
  const prev = map[card.action_id];
  if (prev && ACTION_RANK[prev.status] > ACTION_RANK[card.status]) return map;
  return { ...map, [card.action_id]: card };
}

const VERBS: Record<string, string> = {
  routine_create: 'set up a routine',
  routine_update: 'changed a routine',
  routine_pause: 'paused a routine',
  routine_resume: 'turned a routine back on',
  routine_run_now: 'ran a routine',
  routine_delete: 'deleted a routine',
  follow_up_create: 'scheduled a check-in',
  bot_create: 'created a bot',
  bot_update: 'changed a bot',
  bot_delete: 'wants to delete a bot',
  bot_hire: 'hired a bot',
  settings_update: 'changed a setting',
  space_create: 'created a space',
  space_update: 'changed this space',
  space_archive: 'wants to archive this space',
  channel_create: 'created a channel',
  channel_update: 'changed a channel',
  profile_delete: 'wants to remove an account',
};

/** "set up a routine": what the bot did, after its name. */
export const actionVerb = (action: string) => VERBS[action] ?? 'changed something';

/** "Coach set up a routine": the card's heading. */
export const actionHeading = (botName: string, action: string) => `${botName} ${actionVerb(action)}`;

const BOT_ACTIONS_WITH_UNDO = ['bot_create', 'bot_update', 'bot_hire'];

/**
 * The bot an applied card made or changed no longer exists (deleted since, here or elsewhere), so
 * there's nothing left to undo. `botIds` is null until the bot list has loaded: never guess then.
 */
export function botGone(m: Pick<BotActionMeta, 'status' | 'action' | 'target_kind' | 'target_id'>, botIds: ReadonlySet<string> | null): boolean {
  return !!botIds && m.status === 'applied' && m.target_kind === 'bot' && BOT_ACTIONS_WITH_UNDO.includes(m.action) && !!m.target_id && !botIds.has(m.target_id);
}

/** An accepted hire whose bot has since been deleted. */
export function hiredBotGone(h: Pick<HireMeta, 'status' | 'hired_kwant_id'> & Partial<Pick<HireMeta, 'hired_kwant_ids'>>, botIds: ReadonlySet<string> | null): boolean {
  if (!botIds || (h.status !== 'accepted' && h.status !== 'auto_accepted')) return false;
  // A team hire is gone once every bot it made is.
  const ids = h.hired_kwant_ids?.length ? h.hired_kwant_ids : h.hired_kwant_id ? [h.hired_kwant_id] : [];
  return ids.length > 0 && ids.every((id) => !botIds.has(id));
}

/** The state line in plain words. Never "Done" once the change was undone or declined, or its bot is gone. */
export function actionStateText(m: Pick<BotActionMeta, 'status' | 'action'> & Partial<Pick<BotActionMeta, 'details'>>, gone = false): { label: string; tone: 'ok' | 'wait' | 'off' } {
  if (gone) {
    const name = str(m.details?.name);
    if (m.action === 'bot_create') return { label: name ? `Created ${name}. It’s since been deleted.` : 'Created. The bot has since been deleted.', tone: 'off' };
    if (m.action === 'bot_hire') return { label: name ? `Hired ${name}. It’s since been deleted.` : 'Hired. The bot has since been deleted.', tone: 'off' };
    return { label: name ? `Done. ${name} has since been deleted.` : 'Done. The bot has since been deleted.', tone: 'off' };
  }
  switch (m.status) {
    case 'pending':
      return { label: 'Needs your OK. Nothing changes until you confirm.', tone: 'wait' };
    case 'undone':
      return { label: 'Undone. It’s back the way it was.', tone: 'off' };
    case 'confirmed':
      return { label: 'Confirmed. It’s done.', tone: 'ok' };
    case 'declined':
      return { label: 'Declined. Nothing changed.', tone: 'off' };
    default:
      return { label: m.action === 'routine_run_now' ? 'Done. This can’t be undone.' : 'Done.', tone: 'ok' };
  }
}

/** "created routine 'x'" → "Created routine 'x'". */
export const sentence = (s: string) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

/** A schedule from the card or the API, when the panel's helpers know its shape. */
export function asSchedule(v: unknown): Schedule | null {
  if (!v || typeof v !== 'object') return null;
  const s = v as Record<string, unknown>;
  const tz = typeof s.tz === 'string' ? s.tz : '';
  switch (s.type) {
    case 'once':
      return typeof s.at === 'string' ? { type: 'once', at: s.at } : null;
    case 'daily':
      return typeof s.time === 'string' ? { type: 'daily', time: s.time, tz } : null;
    case 'weekly': {
      if (typeof s.time !== 'string' || !Array.isArray(s.weekdays)) return null;
      const ids = WEEKDAYS.map((d) => d.id);
      const days = s.weekdays.filter((d): d is (typeof ids)[number] => ids.includes(d as (typeof ids)[number]));
      return { type: 'weekly', time: s.time, weekdays: days, tz };
    }
    case 'interval':
      return typeof s.every_minutes === 'number' ? { type: 'interval', every_minutes: s.every_minutes } : null;
    case 'monthly': {
      if (typeof s.time !== 'string' || !Array.isArray(s.days)) return null;
      const days = s.days.filter((d): d is number | 'last' => d === 'last' || (typeof d === 'number' && Number.isInteger(d) && d >= 1 && d <= 31));
      return { type: 'monthly', time: s.time, days, tz };
    }
    case 'cron':
      return typeof s.expression === 'string' ? { type: 'cron', expression: s.expression, tz } : null;
  }
  return null;
}

/** When a routine runs, in words: ours when we know the shape, else the server's `schedule_text`. */
export function scheduleWords(schedule: unknown, scheduleText?: unknown): string {
  const s = asSchedule(schedule);
  if (s) return describeSchedule(s);
  return typeof scheduleText === 'string' && scheduleText ? sentence(scheduleText) : 'Unknown schedule';
}

/** The facts a routine card lists: when, as whom, where, and the next run. */
export function routineFacts(m: BotActionMeta): Array<{ label: string; value: string }> {
  if (m.target_kind !== 'routine') return [];
  const d = m.details;
  const rows: Array<{ label: string; value: string }> = [];
  if (d.schedule || d.schedule_text) rows.push({ label: 'When', value: scheduleWords(d.schedule, d.schedule_text) });
  if (str(d.bot_name)) rows.push({ label: 'Runs as', value: d.bot_name as string });
  if (str(d.delivers_to)) rows.push({ label: 'Posts in', value: d.delivers_to as string });
  if (m.action === 'routine_run_now') {
    if (str(d.run_status)) rows.push({ label: 'This run', value: RUN_WORDS[d.run_status as string] ?? (d.run_status as string) });
  } else if (m.status === 'applied' && m.action !== 'routine_delete') {
    const next = d.enabled === false ? 'Paused' : str(d.next_run_at) ? formatWhen(d.next_run_at as string) : null;
    if (next) rows.push({ label: 'Next run', value: next });
  }
  return rows;
}

const RUN_WORDS: Record<string, string> = { running: 'Running', ok: 'Finished', error: 'Failed', skipped: 'Skipped, it was already running' };

/** Edit and Pause make sense while the routine the card made or changed still exists. */
export function routineButtons(m: BotActionMeta): { edit: boolean; pause: boolean } {
  const live = m.target_kind === 'routine' && !!m.target_id && m.status === 'applied' && m.action !== 'routine_delete';
  return { edit: live, pause: live && m.details.enabled !== false && m.action !== 'routine_pause' };
}

// ---------------------------------------------------------------------------
// Hiring

export type HireStatus = 'pending' | 'accepted' | 'auto_accepted' | 'denied' | 'undone';
const HIRE_STATUSES: HireStatus[] = ['pending', 'accepted', 'auto_accepted', 'denied', 'undone'];

export type HireEvidence =
  | { basis: 'specialist'; needs: string[]; gap: string }
  | { basis: 'repeated'; similar_requests: string[] }
  | { basis: 'other'; text: string };

/** `meta` of a hire proposal card (`kind: "hire_proposal"`). */
export interface HireMeta {
  kind: 'hire_proposal';
  hire_id: string;
  kwant_id: string | null;
  proposer_name: string;
  name: string;
  role: string;
  instructions: string;
  model: string | null;
  tools: Array<{ id: string; name: string | null }>;
  team: { id: string; name: string } | null;
  reason: string;
  evidence: HireEvidence | null;
  requested_by_person: boolean;
  status: HireStatus;
  auto: boolean;
  choices: string[];
  hired_kwant_id: string | null;
  action_id: string | null;
  can_undo: boolean;
  policy: 'ask' | 'auto';
  note: string | null;
  created_at: string | null;
  resolved_at: string | null;
  /** Every bot the proposal hires, the first being the top-level one; one entry for a single hire. */
  members: HireMember[];
  /** The team the proposal creates for them, when it makes a new one. */
  /** `lead` is a member's name; the server makes the first member lead when it's missing. */
  new_team: { name: string; lead: string | null } | null;
  /** The bots made on accept, in `members` order; empty until then. */
  hired_kwant_ids: string[];
}

export interface HireMember {
  name: string;
  role: string;
  model: string | null;
  tools: Array<{ id: string; name: string | null }>;
  instructions: string;
}

/** A proposal from the API or `hire_updated`; `card` is its meta. */
export interface Hire {
  id: string;
  status: string;
  card?: unknown;
}

const strs = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && !!x) : []);

function parseEvidence(raw: unknown): HireEvidence | null {
  if (typeof raw === 'string') return raw.trim() ? { basis: 'other', text: raw.trim() } : null;
  if (!raw || typeof raw !== 'object') return null;
  const e = raw as Record<string, unknown>;
  if (e.basis === 'specialist') return { basis: 'specialist', needs: strs(e.needs), gap: str(e.gap) ?? '' };
  if (e.basis === 'repeated') return { basis: 'repeated', similar_requests: strs(e.similar_requests) };
  return null;
}

function parseTools(raw: unknown): Array<{ id: string; name: string | null }> {
  return Array.isArray(raw)
    ? raw.flatMap((t) => (t && typeof t === 'object' && str((t as Record<string, unknown>).id) ? [{ id: (t as Record<string, string>).id, name: str((t as Record<string, unknown>).name) }] : []))
    : [];
}

function parseMembers(raw: unknown): HireMember[] {
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((x) => {
    if (!x || typeof x !== 'object') return [];
    const o = x as Record<string, unknown>;
    const name = str(o.name);
    return name ? [{ name, role: str(o.role) ?? '', model: str(o.model), tools: parseTools(o.tools), instructions: str(o.instructions) ?? '' }] : [];
  });
}

export function parseHireMeta(raw: unknown): HireMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'hire_proposal') return null;
  const id = str(m.hire_id);
  const name = str(m.name);
  if (!id || !name) return null;
  const status = HIRE_STATUSES.includes(m.status as HireStatus) ? (m.status as HireStatus) : 'pending';
  const team = m.team && typeof m.team === 'object' ? (m.team as Record<string, unknown>) : null;
  return {
    kind: 'hire_proposal',
    hire_id: id,
    kwant_id: str(m.kwant_id),
    proposer_name: str(m.proposer_name) ?? '',
    name,
    role: str(m.role) ?? '',
    instructions: str(m.instructions) ?? '',
    model: str(m.model),
    tools: parseTools(m.tools),
    team: team && str(team.id) ? { id: team.id as string, name: str(team.name) ?? 'a team' } : null,
    reason: str(m.reason) ?? '',
    evidence: parseEvidence(m.evidence),
    requested_by_person: m.requested_by_person === true,
    status,
    auto: m.auto === true,
    choices: status === 'pending' ? strs(m.choices) : [],
    hired_kwant_id: str(m.hired_kwant_id),
    action_id: str(m.action_id),
    can_undo: m.can_undo === true && !!str(m.action_id) && (status === 'accepted' || status === 'auto_accepted'),
    policy: m.policy === 'auto' ? 'auto' : 'ask',
    note: str(m.note),
    created_at: str(m.created_at),
    resolved_at: str(m.resolved_at),
    members: (() => {
      const listed = parseMembers(m.members);
      const top: HireMember = { name, role: str(m.role) ?? '', model: str(m.model), tools: parseTools(m.tools), instructions: str(m.instructions) ?? '' };
      return listed.length ? listed : [top];
    })(),
    new_team: (() => {
      const t = m.new_team && typeof m.new_team === 'object' ? (m.new_team as Record<string, unknown>) : null;
      const teamName = t && str(t.name);
      return teamName ? { name: teamName, lead: str(t.lead)?.replace(/^@/, '') ?? null } : null;
    })(),
    hired_kwant_ids: strs(m.hired_kwant_ids),
  };
}

/** Index of the member who will lead the new team, or -1 when the proposal makes no team. */
export function hireLeadIndex(h: Pick<HireMeta, 'members' | 'new_team'>): number {
  if (!h.new_team) return -1;
  const lead = h.new_team.lead?.toLowerCase();
  const i = lead ? h.members.findIndex((m) => m.name.toLowerCase() === lead) : -1;
  return i >= 0 ? i : 0;
}

/** Whether the proposal hires several bots at once (a team), rather than one. */
export const isTeamHire = (h: Pick<HireMeta, 'members'>) => h.members.length > 1;

const HIRE_RANK: Record<HireStatus, number> = { pending: 0, accepted: 1, auto_accepted: 1, denied: 2, undone: 2 };

export function hireView(meta: HireMeta, live?: HireMeta | null): HireMeta {
  if (!live || live.hire_id !== meta.hire_id) return meta;
  return HIRE_RANK[live.status] >= HIRE_RANK[meta.status] ? live : meta;
}

export function applyHire(map: Record<string, HireMeta>, h: Hire | null | undefined): Record<string, HireMeta> {
  const card = parseHireMeta(h?.card);
  if (!card) return map;
  const prev = map[card.hire_id];
  if (prev && HIRE_RANK[prev.status] > HIRE_RANK[card.status]) return map;
  return { ...map, [card.hire_id]: card };
}

const NEEDS: Record<string, string> = { instructions: 'its own instructions', tools: 'its own tools', schedule: 'a schedule', model: 'a different model' };

/** What the proposal rests on, in one sentence. */
export function evidenceText(e: HireEvidence | null, requestedByPerson: boolean): string {
  if (requestedByPerson) return 'You asked for this kind of bot.';
  if (!e) return '';
  if (e.basis === 'repeated') {
    const n = e.similar_requests.length;
    return `You asked for this kind of work ${n} ${n === 1 ? 'time' : 'times'} in this space.`;
  }
  if (e.basis === 'specialist') {
    const needs = e.needs.map((n) => NEEDS[n] ?? n);
    const list = needs.length > 1 ? `${needs.slice(0, -1).join(', ')} and ${needs[needs.length - 1]}` : needs[0];
    const lead = list ? `This work needs ${list}.` : '';
    return [lead, e.gap ? sentence(e.gap.trim()).replace(/([^.!?])$/, '$1.') : ''].filter(Boolean).join(' ');
  }
  return e.text;
}

/** "auto" or empty means the app picks; anything else is shown as is. */
export const modelText = (model: string | null) => (!model || model === 'auto' ? 'Picked automatically' : model);

/** "Ana, Ben and Cy". */
export const andList = (xs: string[]) => (xs.length > 1 ? `${xs.slice(0, -1).join(', ')} and ${xs[xs.length - 1]}` : xs[0] ?? '');

export function hireStateText(
  h: Pick<HireMeta, 'status' | 'name' | 'team' | 'proposer_name'> & Partial<Pick<HireMeta, 'members' | 'new_team'>>,
  gone = false,
): { label: string; tone: 'ok' | 'wait' | 'off' } {
  const names = h.members && h.members.length > 1 ? h.members.map((m) => m.name) : [h.name];
  const many = names.length > 1;
  const who = andList(names);
  const team = h.new_team?.name ?? h.team?.name;
  const joined = team ? `joined ${team}` : many ? 'are ready' : 'is ready';
  if (gone) return { label: many ? `Hired ${who}. They’ve since been deleted.` : `Hired ${h.name}. It’s since been deleted.`, tone: 'off' };
  switch (h.status) {
    case 'accepted':
      return { label: `Hired. ${who} ${joined}.`, tone: 'ok' };
    case 'auto_accepted':
      return { label: `Hired at once, because you chose Always accept. ${who} ${joined}.`, tone: 'ok' };
    case 'denied':
      return { label: many ? 'Not hired. Bots here won’t suggest this team again unless you ask.' : `Not hired. Bots here won’t suggest this kind of bot again unless you ask.`, tone: 'off' };
    case 'undone':
      return { label: many ? `Hire undone. ${who} were removed.` : `Hire undone. ${h.name} was removed.`, tone: 'off' };
    default:
      return { label: `Waiting for you. ${h.proposer_name || 'The bot'} carries on with your request meanwhile.`, tone: 'wait' };
  }
}

// ---------------------------------------------------------------------------
// Hire policy (Space settings)

export interface HirePolicy {
  space_id: string;
  policy: 'ask' | 'auto';
  auto_hires_last_24h: number;
  auto_hires_per_day: number;
  denied: Array<{ hire_id: string; name: string; role: string; denied_at: string }>;
  denials_cleared_at: string | null;
  updated_at: string | null;
}

export function parseHirePolicy(raw: unknown): HirePolicy | null {
  if (!raw || typeof raw !== 'object') return null;
  const p = raw as Record<string, unknown>;
  const num = (v: unknown, d: number) => (typeof v === 'number' && Number.isFinite(v) ? v : d);
  return {
    space_id: str(p.space_id) ?? '',
    policy: p.policy === 'auto' ? 'auto' : 'ask',
    auto_hires_last_24h: num(p.auto_hires_last_24h, 0),
    auto_hires_per_day: num(p.auto_hires_per_day, 3),
    denied: Array.isArray(p.denied)
      ? p.denied.flatMap((d) => {
          if (!d || typeof d !== 'object') return [];
          const x = d as Record<string, unknown>;
          return str(x.name) || str(x.role) ? [{ hire_id: str(x.hire_id) ?? '', name: str(x.name) ?? '', role: str(x.role) ?? '', denied_at: str(x.denied_at) ?? '' }] : [];
        })
      : [],
    denials_cleared_at: str(p.denials_cleared_at),
    updated_at: str(p.updated_at),
  };
}

// ---------------------------------------------------------------------------
// Routes

/** An API error that keeps its HTTP status, so a 409 can read as "already decided". */
export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

async function call<T>(method: string, url: string, body?: unknown): Promise<T> {
  // Undo, confirm and decline are person-only: after "Confirm it's you" the call goes through.
  const res = await personFetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) {
    const fallback = res.status === 404 ? 'It no longer exists.' : `Request failed (HTTP ${res.status}).`;
    throw new ApiError(typeof data?.error === 'string' && data.error ? data.error : fallback, res.status);
  }
  return data as T;
}

/** A failed button in plain words; a 409 means someone (or another tab) got there first. */
export function decisionError(err: unknown, what: string): string {
  if (err instanceof ApiError && err.status === 409) {
    const why = err.message && !/^Request failed/.test(err.message) ? ` (${err.message.replace(/\.$/, '')})` : '';
    return `Couldn’t ${what}: it was already decided or changed${why}.`;
  }
  if (err instanceof PersonCancelledError) return err.message;
  if (err instanceof ApiError && err.status === 403) return `Couldn’t ${what}: only you can do this, from your own browser.`;
  return `Couldn’t ${what}: ${err instanceof Error ? err.message : 'something went wrong.'}`;
}

const enc = encodeURIComponent;

export const botActionsApi = {
  list: () => call<BotAction[]>('GET', '/api/bot-actions'),
  undo: (id: string) => call<BotAction>('POST', `/api/bot-actions/${enc(id)}/undo`),
  confirm: (id: string) => call<BotAction>('POST', `/api/bot-actions/${enc(id)}/confirm`),
  decline: (id: string) => call<BotAction>('POST', `/api/bot-actions/${enc(id)}/decline`),
};

export const hiresApi = {
  list: () => call<Hire[]>('GET', '/api/hires'),
  accept: (id: string, always = false) => call<Hire>('POST', `/api/hires/${enc(id)}/accept`, always ? { always: true } : {}),
  deny: (id: string) => call<Hire>('POST', `/api/hires/${enc(id)}/deny`),
  policy: (spaceId: string) => call<unknown>('GET', `/api/spaces/${enc(spaceId)}/hire-policy`),
  setPolicy: (spaceId: string, body: { policy?: 'ask' | 'auto'; clear_denials?: boolean }) =>
    call<unknown>('PUT', `/api/spaces/${enc(spaceId)}/hire-policy`, body),
};

// ---------------------------------------------------------------------------
// Sidebar

/**
 * Whether a live event added, changed or removed a bot, so the bot list should reload: a bot
 * action on a bot that was applied, undone or confirmed, or a hire that was accepted or undone.
 */
export function changesBots(type: string, data: unknown): boolean {
  const d = data && typeof data === 'object' ? (data as Record<string, unknown>) : null;
  if (!d) return false;
  const action = (card: BotActionMeta | null) => !!card && card.target_kind === 'bot' && ['applied', 'undone', 'confirmed'].includes(card.status);
  const hire = (card: HireMeta | null) => !!card && ['accepted', 'auto_accepted', 'undone'].includes(card.status);
  if (type === 'bot_action_updated' || type === 'BotActionUpdated') return action(parseBotActionMeta((d.action as BotAction | undefined)?.card));
  if (type === 'hire_updated' || type === 'HireUpdated') return hire(parseHireMeta((d.hire as Hire | undefined)?.card));
  // A card can land before its update event; until the list is fetched again it can't be judged.
  if (type === 'message_added' || type === 'MessageAdded') {
    const meta = (d.message as { meta?: unknown } | undefined)?.meta;
    return action(parseBotActionMeta(meta)) || hire(parseHireMeta(meta));
  }
  return false;
}
