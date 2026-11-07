// 0.7.7 Ready-made teams (T) and "Suggest members from the code" (D): types, the API client and the
// pure rules the preview sheet and the Needs card share. Design: next-077/design/design.md. API:
// next-077/api.md (Lin, branch lin-077-teams). The client turns api.md's shapes into the ones below;
// tests and shots run against the wire-format fixtures in templateFixtures.ts and suggestFixtures.ts.
import { personFetch } from '../auth';

// ---------------------------------------------------------------------------------------------------
// Types. The internal shapes the sheet uses; `Wire*` are api.md's, turned into these by the client.

export interface TemplateBot {
  /** api.md §0.1 `key` (inboxmanager, drafter, …, techlead, frontend, backend, qa): the key in `names`.
   * Never the preset (Coding has two `backend` bots) and never an index (the server refuses those). */
  key: string;
  preset: string;
  name: string;
  title: string;
  line: string;
  avatar_sprite: string;
  lead: boolean;
}

export type NeedOpens = 'loose_ends_imap' | 'connections' | 'dev_mode_setup' | 'composer_prompt' | string;

export interface TemplateNeed {
  id: string;
  label: string;
  why: string;
  required: boolean;
  opens: NeedOpens;
}

export interface TemplateSkill {
  /** api.md §0.2: the owning bot's `key`. */
  owner: string;
  /** For display only; the sheet shows the person's rename when there is one. */
  owner_name?: string;
  name: string;
  description: string;
  body: string;
}

export interface TemplateRoutine {
  name: string;
  when: string;
  /** api.md §0.9: the bot's `key`. */
  bot: string;
  enabled: false;
  cron?: string;
  prompt?: string;
}

export interface TeamTemplate {
  id: string;
  label: string;
  icon: string;
  handle: string;
  card_line: string;
  bots: TemplateBot[];
  needs: TemplateNeed[];
  never: string[];
  skills: TemplateSkill[];
  routine: TemplateRoutine | null;
  first_prompts: string[];
  used_in_space?: boolean;
}

export type WireTemplate = TeamTemplate;

/** One part of a D reason: plain text, or a path shown as inline code. */
export type ReasonPart = { text: string } | { path: string };

export interface SuggestedBot {
  key: string;
  preset: string;
  lead: boolean;
  name: string;
  title: string;
  avatar_sprite: string;
  reason: ReasonPart[];
  owns: string[];
  /** D6: a bot in this space with the same preset or title; the row offers "Add {name} (already in this space)". */
  existing_kwant_id?: string | null;
  existing_name?: string | null;
  existing_avatar?: string | null;
  /** That bot already leads a team, so adding it here makes it a member, not this team's lead. */
  existing_leads_team?: string | null;
}

/** api.md §0.6: the wire's `existing` object; the client flattens it onto the row. */
export interface WireExisting {
  kwant_id: string;
  name: string;
  avatar_sprite?: string | null;
  leads_team?: string | null;
}

export interface Suggestion {
  suggestion_id: string;
  handle?: string;
  facts: { name: string; detail: string }[];
  bots: SuggestedBot[];
  also_possible: SuggestedBot[];
  read: { files: number; manifests: number; ms: number };
  /** null, or which limit stopped the read (api.md §4; "file": one manifest over 256 KiB was skipped, §0.7). */
  partial: null | 'paths' | 'bytes' | 'time' | 'file';
  /** Without api.md's "This team's code: " lead-in; the sheet prints its own. */
  stack_line: string | null;
  /** Nothing kwant knows, so these are the Coding roster. */
  fallback: boolean;
  /** What was read, for the readline: the folder or the repo (client-side). */
  source: string;
}

type WireSuggested = Omit<SuggestedBot, 'reason' | 'owns' | 'existing_kwant_id' | 'existing_name' | 'existing_avatar' | 'existing_leads_team'> & {
  reason_parts?: ReasonPart[];
  reason?: string;
  owns?: string[];
  existing?: WireExisting | null;
};
export type WireSuggestion = Omit<Suggestion, 'bots' | 'also_possible' | 'source'> & { bots: WireSuggested[]; also_possible: WireSuggested[] };

export interface CreatedTeam {
  team: { id: string; name: string; workspace_path?: string | null; repo_url?: string | null };
  channel_id: string;
  action_id: string;
  template_id: string | null;
  /** `key`: the template bot key, or the suggestion row key. */
  bots: { key: string; id: string; name: string; preset: string; lead: boolean; existing: boolean }[];
  skills: { id: string; name: string; owner_id: string }[];
  routine: { id: string; name: string; enabled: boolean } | null;
  undo_until: string;
}

/** api.md §0.8: what a team row says about the template that made it. */
export type CreatedFrom = 'inbox' | 'sales' | 'coding' | 'suggested' | null;

/** GET /api/teams/:id/template-undo, read by the Undo confirm (api.md §0.10, §6). */
export interface TeamTemplateInfo {
  template_id: string | null;
  action_id: string;
  created_at: string;
  undo_until: string;
  can_undo: boolean;
  blocked: unknown[];
  removes: { bots: string[]; skills: number; routines: number; handle: string };
  chat_messages: number;
  /** Other teams a bot it made has joined since; it leaves them too. */
  other_teams: { bot: string; team: string }[];
  edited_skills: string[];
}
type WireTemplateInfo = Omit<TeamTemplateInfo, 'other_teams'> & { other_teams?: TeamTemplateInfo['other_teams']; also_in?: TeamTemplateInfo['other_teams'] };

/** GET /api/team-templates/:id/preview: the server's next free names for this space (api.md §0.10, §2). */
export interface TemplatePreview {
  template_id: string;
  handle: string;
  handle_wanted: string;
  /** Keyed by bot key. */
  names: Record<string, string>;
  clashes: Clash[];
}

/** One clash: `field` is `handle`, `names.<key>` or `edits.rows.<key>.name`. */
export interface Clash {
  field: string;
  wanted?: string;
  suggestion: string;
}

/** One D row as sent (api.md §3b). Only these fields: anything else is a 400. */
export interface RowSend {
  on?: boolean;
  name?: string;
  preset?: string;
  use_existing?: string | null;
}

export type CreateRequest =
  | { kind: 'template'; space: string; template_id: string; handle: string; names: Record<string, string> }
  /** `team_id` set: A18, add the ticked bots to that existing team (Manage team). */
  | { kind: 'suggestion'; space: string; suggestion_id: string; handle?: string; team_id?: string; rows: Record<string, RowSend> };

export type SuggestRequest = { space: string } & ({ workspace_path: string } | { repo_url: string } | { team_id: string });

export class TeamsError extends Error {
  constructor(
    message: string,
    public status: number,
    public code: string | null = null,
    public field: string | null = null,
    /** The row the error is about: the slot of `names.<slot>` or the key of `edits.rows.<key>.name`. */
    public key: string | null = null,
    public suggestion: string | null = null,
    public extra: Record<string, unknown> = {},
    /** api.md §0.4: every clash in a 409 `clash`; the top-level field and suggestion are the first. */
    public clashes: Clash[] = [],
  ) {
    super(message);
  }
}

/** `names.drafter` → drafter, `edits.rows.qa.name` → qa, `handle` → null. */

export function fieldKey(field: string | null): string | null {
  if (!field) return null;
  const m = /^names\.(.+)$/.exec(field) ?? /^edits\.rows\.([^.]+)\./.exec(field);
  return m ? m[1] : null;
}

// ---------------------------------------------------------------------------------------------------
// Transport (swappable for tests and shots)

export type TeamsTransport = (url: string, method: string, body?: unknown, headers?: Record<string, string>) => Promise<{ status: number; body: unknown }>;

const realTransport: TeamsTransport = async (url, method, body, headers) => {
  const res = await personFetch(url, {
    method,
    cache: 'no-store',
    headers: { ...(body === undefined ? {} : { 'Content-Type': 'application/json' }), ...(headers ?? {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: res.status, body: await res.json().catch(() => null) };
};

let transport: TeamsTransport = realTransport;
export const setTeamsTransport = (t: TeamsTransport | null) => {
  transport = t ?? realTransport;
};

async function call<T>(url: string, method = 'GET', body?: unknown, headers?: Record<string, string>): Promise<T> {
  let r: { status: number; body: unknown };
  try {
    r = await transport(url, method, body, headers);
  } catch (e) {
    if (e instanceof TeamsError) throw e;
    if (e instanceof DOMException && e.name === 'AbortError') throw e;
    throw new TeamsError('Couldn’t reach the server.', 0, 'offline');
  }
  if (r.status >= 200 && r.status < 300) return r.body as T;
  const b = (r.body && typeof r.body === 'object' ? r.body : {}) as Record<string, unknown>;
  const s = (k: string) => (typeof b[k] === 'string' ? (b[k] as string) : null);
  const field = s('field');
  const clashes = Array.isArray(b.clashes)
    ? (b.clashes as unknown[]).filter((c): c is Clash => !!c && typeof (c as Clash).field === 'string' && typeof (c as Clash).suggestion === 'string')
    : field && s('suggestion')
      ? [{ field, suggestion: s('suggestion') as string }]
      : [];
  throw new TeamsError(s('error') ?? `Something went wrong (HTTP ${r.status}).`, r.status, s('code'), field, fieldKey(field), s('suggestion'), b, clashes);
}

const enc = encodeURIComponent;

export const normTemplate = (t: WireTemplate): TeamTemplate => t;

/** The display name of a template bot by key, after the person's renames. */
export const botNameByKey = (t: Pick<TeamTemplate, 'bots'>, names: Record<string, string>, key: string, fallback?: string): string =>
  names[key] ?? t.bots.find((b) => b.key === key)?.name ?? fallback ?? key;

const STACK_LEAD = /^This team[’']s code:\s*/i;

export function normSuggestion(s: WireSuggestion, source: string): Suggestion {
  const row = ({ reason_parts, reason, owns, existing, ...b }: WireSuggested): SuggestedBot => ({
    ...b,
    owns: owns ?? [],
    reason: reason_parts ?? (reason ? [{ text: reason }] : []),
    existing_kwant_id: existing?.kwant_id ?? null,
    existing_name: existing?.name ?? null,
    existing_avatar: existing?.avatar_sprite ?? null,
    existing_leads_team: existing?.leads_team ?? null,
  });
  return {
    ...s,
    bots: s.bots.map(row),
    also_possible: (s.also_possible ?? []).map(row),
    stack_line: s.stack_line ? s.stack_line.replace(STACK_LEAD, '') : null,
    fallback: !!s.fallback,
    source,
  };
}

/** api.md §3 bodies: exactly the documented fields, so the strict server never answers unknown_field.
 * `idempotency_key` is required (§0.3): one UUID per sheet open. */
export function createBody(r: CreateRequest, idempotencyKey: string): Record<string, unknown> {
  if (r.kind === 'template') return { template_id: r.template_id, handle: r.handle, names: r.names, idempotency_key: idempotencyKey };
  const edits: Record<string, unknown> = { rows: r.rows };
  if (r.handle && !r.team_id) edits.handle = r.handle;
  const out: Record<string, unknown> = { suggestion_id: r.suggestion_id, edits, idempotency_key: idempotencyKey };
  if (r.team_id) out.team_id = r.team_id;
  return out;
}

const normInfo = ({ other_teams, also_in, ...i }: WireTemplateInfo): TeamTemplateInfo => ({ ...i, other_teams: other_teams ?? also_in ?? [], edited_skills: i.edited_skills ?? [] });

const newKey = (): string => {
  try {
    return crypto.randomUUID();
  } catch {
    return `k${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`;
  }
};

export const teamsApi = {
  templates: (space: string) =>
    call<{ templates: WireTemplate[] }>(`/api/team-templates?space=${enc(space)}`).then((r) => (r.templates ?? []).map(normTemplate)),
  /** `idempotencyKey`: one per sheet open (api.md §0.3); a retry of the same sheet reuses it. */
  create: (r: CreateRequest, idempotencyKey: string) =>
    call<CreatedTeam>(`/api/teams/from-template?space=${enc(r.space)}`, 'POST', createBody(r, idempotencyKey)),
  /** Optional: the server's free names (api.md §0.10). Callers fall back to their own guess on any error. */
  preview: (space: string, templateId: string) =>
    call<TemplatePreview>(`/api/team-templates/${enc(templateId)}/preview?space=${enc(space)}`),
  suggest: ({ space, ...src }: SuggestRequest, source: string) =>
    call<WireSuggestion>(`/api/teams/suggest?space=${enc(space)}`, 'POST', src).then((s) => normSuggestion(s, source)),
  templateInfo: (teamId: string, space: string) =>
    call<WireTemplateInfo>(`/api/teams/${enc(teamId)}/template-undo?space=${enc(space)}`).then(normInfo),
};
export { newKey as newIdempotencyKey };

// ---------------------------------------------------------------------------------------------------
// Names and handles (T9, A1–A3)

/** The New team slug rule, as typed: "Inbox Team" → "inbox-team". */
export const slugHandle = (s: string): string =>
  s
    .toLowerCase()
    .replace(/[\s_]+/g, '-')
    .replace(/[^a-z0-9-]/g, '');

/** A handle ready to send: no leading, trailing or doubled dashes. */
export const cleanHandle = (s: string): string => slugHandle(s).replace(/-+/g, '-').replace(/^-|-$/g, '');

const lower = (s: string) => s.toLowerCase();

/** `base`, or base2, base3… : the first not in `taken` (case-insensitive). A trailing number is replaced. */
export function nextFree(base: string, taken: Iterable<string>): string {
  const set = new Set([...taken].map(lower));
  if (!set.has(lower(base))) return base;
  const stem = base.replace(/\d+$/, '') || base;
  for (let n = 2; n < 1000; n++) {
    const c = `${stem}${n}`;
    if (!set.has(lower(c))) return c;
  }
  return `${stem}${Date.now() % 10000}`;
}

/** The suggestion for an invalid name: spaces and @ removed, then the next free number. */
export const suggestName = (raw: string, taken: Iterable<string>): string => nextFree(raw.replace(/[\s@]+/g, '') || 'Bot', taken);

/** Why a bot name can't be used, or null. `others` are the other names in this sheet. */
export function nameError(raw: string, taken: Iterable<string>, others: string[] = [], fallback = 'ReplyDrafter'): string | null {
  const name = raw.trim();
  if (!name) return 'Give this bot a name.';
  if (/[\s@]/.test(name)) {
    const s = name.replace(/[\s@]+/g, '');
    return `Bot names can’t contain spaces or @. Try ${s && s.length > 1 ? suggestName(s, [...taken, ...others]) : fallback}.`;
  }
  if (name.length > 40) return 'Keep the name to 40 characters or fewer.';
  const all = [...taken, ...others];
  if (all.some((t) => lower(t) === lower(name))) return `You already have a ${name}. Try ${nextFree(name, all)}.`;
  return null;
}

export interface Clashes {
  names: Record<string, string>;
  notes: Record<string, string>;
  handle: string;
  handleNote: string | null;
}

/** The pre-changed names and handle for a sheet (T9): Drafter → Drafter2, @inbox → @inbox2. */
export function resolveClashes(bots: { key: string; name: string }[], handle: string, takenNames: Iterable<string>, takenHandles: Iterable<string>): Clashes {
  const taken = [...takenNames];
  const names: Record<string, string> = {};
  const notes: Record<string, string> = {};
  const used: string[] = [];
  for (const b of bots) {
    const free = nextFree(b.name, [...taken, ...used]);
    names[b.key] = free;
    used.push(free);
    if (free !== b.name) notes[b.key] = `${b.name} is taken in this space, so this one is ${free}.`;
  }
  const h = nextFree(handle, takenHandles);
  return { names, notes, handle: h, handleNote: h !== handle ? `@${handle} is taken here, so this team is @${h}.` : null };
}

/** "3 bots · 2 skills · 1 routine, off" */
export function summaryLine(t: Pick<TeamTemplate, 'bots' | 'skills' | 'routine'>, bots = t.bots.length): string {
  const parts = [plural(bots, 'bot'), plural(t.skills.length, 'skill')];
  if (t.routine) parts.push('1 routine, off');
  return parts.join(' · ');
}

/** "3 bots, 2 skills and 1 routine (off)" */
export function summaryProse(t: Pick<TeamTemplate, 'bots' | 'skills' | 'routine'>, bots = t.bots.length): string {
  const parts = [plural(bots, 'bot'), plural(t.skills.length, 'skill')];
  if (t.routine) parts.push('1 routine (off)');
  return parts.length === 3 ? `${parts[0]}, ${parts[1]} and ${parts[2]}` : parts.join(' and ');
}

export const plural = (n: number, w: string) => `${n.toLocaleString('en-US')} ${w}${n === 1 ? '' : 's'}`;

/** The Sales "what you sell" item fills the composer with this starter (it never sends). */
export const SELL_STARTER = 'What we sell: \nWho our best customer is: \nWhat they usually pay: ';

// ---------------------------------------------------------------------------------------------------
// Needs (T5)

export type NeedState = 'req' | 'opt' | 'checking' | 'done' | 'bad';

export interface NeedsContext {
  /** The space has a working account (model). */
  canRun: boolean;
  /** The IMAP source in Loose ends for this team's email, if any. */
  imap: { state: 'none' | 'checking' | 'ok' | 'failed'; detail?: string | null };
  /** Active app connections, by name or kind (lower case). */
  apps: string[];
  /** Dev mode for this team's repo. */
  repo: { state: 'none' | 'checking' | 'watching' | 'failed'; detail?: string | null };
  /** The person has posted a message in this team's chat. */
  personPosted: boolean;
}

export interface NeedView {
  state: NeedState;
  /** The line under the label: the why, the checking text, the result or the reason. */
  text: string;
}

const MAIL_APPS = /gmail|outlook|microsoft|office ?365/i;
const CRMS = /hubspot|pipedrive|salesforce|crm|zoho|attio|close\.com|copper/i;

export function needView(n: TemplateNeed, ctx: NeedsContext): NeedView {
  const idle: NeedView = { state: n.required ? 'req' : 'opt', text: n.why };
  switch (n.opens) {
    case 'loose_ends_imap':
      if (ctx.imap.state === 'ok') return { state: 'done', text: ctx.imap.detail ? `Connected · ${ctx.imap.detail}` : 'Connected' };
      if (ctx.imap.state === 'checking') return { state: 'checking', text: ctx.imap.detail ? `Checking ${ctx.imap.detail}…` : 'Checking…' };
      if (ctx.imap.state === 'failed') return { state: 'bad', text: ctx.imap.detail || 'Couldn’t sign in. Check the app password.' };
      return idle;
    case 'connections': {
      const re = n.id === 'crm' ? CRMS : MAIL_APPS;
      const hit = ctx.apps.find((a) => re.test(a));
      return hit ? { state: 'done', text: `Connected · ${hit}` } : idle;
    }
    case 'dev_mode_setup':
      if (ctx.repo.state === 'watching') return { state: 'done', text: ctx.repo.detail ? `Watching ${ctx.repo.detail}` : 'Watching' };
      if (ctx.repo.state === 'checking') return { state: 'checking', text: ctx.repo.detail ? `Checking ${ctx.repo.detail}…` : 'Checking…' };
      if (ctx.repo.state === 'failed') return { state: 'bad', text: ctx.repo.detail || 'Couldn’t read this repo yet.' };
      return idle;
    case 'composer_prompt':
      return ctx.personPosted ? { state: 'done', text: 'Done · you told the team' } : idle;
    default:
      return idle;
  }
}

/** The required items still open, plus "Add an account" when nothing can run. */
export function openCount(t: TeamTemplate, ctx: NeedsContext): number {
  return t.needs.filter((n) => n.required && needView(n, ctx).state !== 'done').length + (ctx.canRun ? 0 : 1);
}

export function needsSub(t: TeamTemplate, ctx: NeedsContext): string {
  const open = openCount(t, ctx);
  const head = `${summaryProse(t)}.`;
  if (open === 0) return `${head} It has what it needs.`;
  return `${head} ${open === 1 ? 'One thing' : `${open} things`} before it can do the job:`;
}

export const STATUS_WORD: Record<NeedState, string> = { req: 'Needed', opt: 'Optional', checking: 'Checking', done: 'Done', bad: 'Failed' };

/** Chip text: a prompt ending ": " shows as ": …". */
export const chipText = (p: string) => (/:\s*$/.test(p) ? `${p.replace(/\s+$/, '')} …` : p);

// ---------------------------------------------------------------------------------------------------
// D: suggestions

export const ROLE_OPTIONS: { preset: string; label: string; name: string; title: string; sprite: string }[] = [
  { preset: 'backend', label: 'Backend', name: 'Backend', title: 'Backend Engineer', sprite: 'architect' },
  { preset: 'frontend', label: 'Frontend', name: 'Frontend', title: 'Frontend Engineer', sprite: 'tech_lead' },
  { preset: 'qa', label: 'QA', name: 'QA', title: 'QA & Test Engineer', sprite: 'qa' },
  { preset: 'sre', label: 'SRE', name: 'SRE', title: 'Site Reliability Engineer', sprite: 'devops' },
  { preset: 'data', label: 'Data', name: 'DataAnalyst', title: 'Data Analyst', sprite: 'data' },
  { preset: 'writer', label: 'Tech Writer', name: 'TechWriter', title: 'Technical Writer', sprite: 'writer' },
  { preset: 'security', label: 'Security', name: 'SecurityAuditor', title: 'Security Auditor', sprite: 'security' },
  { preset: 'designer', label: 'Designer', name: 'Designer', title: 'UX & Product Designer', sprite: 'designer' },
];
export const roleOf = (preset: string) => ROLE_OPTIONS.find((r) => r.preset === preset) ?? null;

export const MIN_BOTS = 2;
export const MAX_BOTS = 6;

/** D5: a folder name that may be shown and written to a persona. Anything else is never shown. */
export const safeFolder = (s: string) => /^[A-Za-z0-9._/-]{1,40}$/.test(s);

export interface RowEdit {
  ticked: boolean;
  preset: string;
  name: string;
  /** D6: the id of the bot already in the space to add instead of a new one; null = new bot. */
  useExisting: string | null;
  /** The person renamed it; a role swap keeps their name. */
  renamed: boolean;
}

export function initialEdits(s: Suggestion, takenNames: Iterable<string>): Record<string, RowEdit> {
  const out: Record<string, RowEdit> = {};
  const taken = [...takenNames];
  const used: string[] = [];
  for (const [i, b] of [...s.bots, ...s.also_possible].entries()) {
    const main = i < s.bots.length;
    const existing = b.existing_kwant_id ?? null;
    const name = existing ? b.name : nextFree(b.name, [...taken, ...used]);
    if (!existing) used.push(name);
    out[b.key] = { ticked: main, preset: b.preset, name, useExisting: existing, renamed: false };
  }
  return out;
}

/** Swapping a role keeps the reason and folders; the default name follows the role unless renamed. */
export function swapRole(e: RowEdit, preset: string, taken: Iterable<string>): RowEdit {
  const r = roleOf(preset);
  if (!r) return e;
  return { ...e, preset, name: e.renamed ? e.name : nextFree(r.name, taken) };
}

export const tickedCount = (edits: Record<string, RowEdit>) => Object.values(edits).filter((e) => e.ticked).length;

export function sizeNote(n: number): string | null {
  if (n < MIN_BOTS) return 'Pick at least 2 bots.';
  if (n >= MAX_BOTS) return '6 bots at most. Untick one to add another.';
  return null;
}

export const newBotCount = (edits: Record<string, RowEdit>) => Object.values(edits).filter((e) => e.ticked && !e.useExisting).length;

/** The partial banner's first line. */
export function partialHead(p: Suggestion['partial']): string | null {
  if (p === 'paths') return 'Read the first 20,000 files.';
  if (p === 'time') return 'Stopped at 10 seconds.';
  if (p === 'bytes') return 'Some manifests were too big to read.';
  if (p === 'file') return 'One manifest was too big to read.';
  return null;
}

/** The partial banner's second line. */
export function partialBody(p: Suggestion['partial']): string | null {
  if (p === 'paths' || p === 'time') return 'This project is bigger than that, so the suggestions come from those files only.';
  if (p === 'bytes') return 'The suggestions come from the manifests it could read.';
  if (p === 'file') return 'It was over 256 KB, so the suggestions come from the other manifests.';
  return null;
}

/** Undo refused because the 24 hours are up (api.md §0: 409 `undo_window_over`). */
export const undoWindowOver = (e: unknown) =>
  e instanceof TeamsError ? e.code === 'undo_window_over' || e.code === 'undo_expired' : !!e && typeof e === 'object' && ['undo_window_over', 'undo_expired'].includes((e as { code?: string }).code ?? '');

/** The Can't read block (api.md §0.5): 424 only. A 403 means the caller isn't the person, never "can't read". */
export const cantRead = (e: unknown) => e instanceof TeamsError && (e.status === 424 || e.code === 'cant_read') && e.status !== 403;

export const readLine = (r: Suggestion['read'], partial: Suggestion['partial']) =>
  `${partial === 'paths' ? 'Read the first 20,000 file names' : `Read ${plural(r.files, 'file name')}`} and ${plural(r.manifests, 'manifest')} in ${(r.ms / 1000).toFixed(1)} s. No file contents.`;

/** The folder or repo name a handle defaults to: "~/projects/shop" or "git.example.com/acme/shop" → "shop". */
export function sourceName(path: string | null | undefined): string {
  const s = (path ?? '').trim().replace(/\.git$/i, '').replace(/[/\\]+$/, '');
  return s.split(/[/\\:]/).pop() ?? '';
}

/** Which forge a repo URL is on, for the Can't read block's button. */
export const forgeOf = (url: string | null | undefined): 'GitHub' | 'GitLab' => (/github\.com/i.test(url ?? '') ? 'GitHub' : 'GitLab');

/** Rows that come back after a 410 keep the person's edits (A17). */
export function carryEdits(prev: Record<string, RowEdit>, next: Record<string, RowEdit>, oldS: Suggestion, newS: Suggestion): Record<string, RowEdit> {
  const out = { ...next };
  const byPreset = (s: Suggestion) => new Map([...s.bots, ...s.also_possible].map((b) => [b.preset + '|' + b.key, b.key]));
  const old = byPreset(oldS);
  for (const [sig, key] of byPreset(newS)) {
    const k = old.get(sig);
    if (k && prev[k]) out[key] = prev[k];
  }
  return out;
}

// ---------------------------------------------------------------------------------------------------
// Local preferences

const hideKey = (space: string) => `kwnt:hide-ready-teams:${space}`;
export const readyTeamsHidden = (space: string): boolean => {
  try {
    return localStorage.getItem(hideKey(space)) === '1';
  } catch {
    return false;
  }
};
export const hideReadyTeams = (space: string) => {
  try {
    localStorage.setItem(hideKey(space), '1');
  } catch {
    /* private mode: hidden for this session only */
  }
};

/** A card that has collapsed once stays collapsed on later opens (per team). */
const doneKey = (team: string) => `kwnt:team-needs-done:${team}`;
export const needsSeenDone = (team: string): boolean => {
  try {
    return localStorage.getItem(doneKey(team)) === '1';
  } catch {
    return false;
  }
};
export const markNeedsDone = (team: string) => {
  try {
    localStorage.setItem(doneKey(team), '1');
  } catch {
    /* ignore */
  }
};
