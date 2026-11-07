// 0.7.6 "Work from Jira": the shared ticket and brief types, their parsers and the small display rules.
// Owned by Iris; Pixel's setup sheet, settings row and badge import from here.
//
// Shapes follow the server as written on 2026-11-07:
//   - TicketRow / TicketEventRow / JiraSite: Juno, src/db/dev_jira.rs and next-076/jira-tickets/api.md §1.4, §2.2.
//   - Brief / Line / SourceRef / Question / Flag: Vega, src/server/dev/tickets/brief.rs and brief.md.
//   - TicketProgress: the dev_ticket_progress SSE event (api.md §3).
// Every field the design needs but the server doesn't send yet is optional and has a fallback (design.md §12).
//
// Everything that came from Jira, Confluence, or a model reading them is untrusted text. Components render
// it as React text only: never HTML, never markdown, never a link unless the url is https.

// ---------------------------------------------------------------------------------------------------
// Small readers (the same rules as dev/types.ts)

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, max = 2000): string | null => (typeof v === 'string' && v.trim() !== '' ? v.slice(0, max) : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const oneOf = <T extends string>(v: unknown, all: readonly T[], fallback: T): T => (all.includes(v as T) ? (v as T) : fallback);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

/** Only an https address may become a link; anything else (javascript:, data:, http:, "") stays text. */
export const safeHref = (v: unknown): string | null => {
  const s = str(v, 2000);
  if (!s || !/^https:\/\/[^\s"'<>]+$/i.test(s)) return null;
  try {
    return new URL(s).protocol === 'https:' ? s : null;
  } catch {
    return null;
  }
};

/** Juno serializes list columns (labels, projects, spaces) as a JSON string; accept that or a real array. */
export function strList(v: unknown, max = 50): string[] {
  let a: unknown = v;
  if (typeof v === 'string') {
    try {
      a = JSON.parse(v);
    } catch {
      a = v.trim() ? [v] : [];
    }
  }
  return arr(a)
    .map((x) => str(x, 200))
    .filter((x): x is string => !!x)
    .slice(0, max);
}

// ---------------------------------------------------------------------------------------------------
// The ticket row (GET /api/spaces/:id/dev/tickets, GET …/tickets/:key)

export const TICKET_STATES = [
  'waiting',
  'picked_up',
  'reading',
  'needs_answer',
  'brief_ready',
  'working',
  'in_review',
  'landed',
  'taken_back',
  'closed',
  'failed',
] as const;
export type TicketState = (typeof TICKET_STATES)[number];
export type StatusCategory = 'todo' | 'in_progress' | 'done';
export type HandedOverBy = 'label' | 'assignee' | 'jql';

/**
 * What the card shows about the work kwant did on the ticket. TicketRow doesn't carry these (it has
 * thread_id/task_id/mission_id on the brief route only), so each is optional and the copy falls back to
 * "The lead" and no chips. Assumed names; see the mismatch list in the PR.
 */
export interface TicketWork {
  lead: string | null;
  builders: string[];
  reviewer: string | null;
  team: string | null;
  branch: string | null;
  pr: { number: number; title: string | null; url: string | null } | null;
  sha: string | null;
  merged_at: string | null;
  plan: { done: number; total: number } | null;
  started_at: string | null;
  /** From the review card's intent rows ("Matches the ticket"); null until reviewed. */
  matches: { met: number; total: number } | null;
  review_card_id: string | null;
  task_url: string | null;
  /** The failure line for `failed` ("Jira said 403 for the attachments. Everything else was read."). */
  error: string | null;
}

export interface Ticket {
  key: string;
  url: string | null;
  summary: string;
  issue_type: string | null;
  status: string | null;
  status_category: StatusCategory | null;
  priority: string | null;
  priority_rank: number | null;
  assignee_name: string | null;
  reporter_name: string | null;
  labels: string[];
  state: TicketState;
  handed_over_by: HandedOverBy | null;
  handed_over_at: string | null;
  taken_back_at: string | null;
  taken_back_by: string | null;
  /** "done: Won't do" or "deleted". */
  closed_resolution: string | null;
  waiting_since_connect: boolean;
  asked_comment_id: string | null;
  jira_created: string | null;
  jira_updated: string | null;
  updated_at: string | null;
  work: TicketWork;
}

const work = (v: unknown): TicketWork => {
  const o = isObj(v) ? v : {};
  const pr = isObj(o.pr) && num(o.pr.number) !== null ? { number: num(o.pr.number)!, title: str(o.pr.title, 300), url: safeHref(o.pr.url) } : null;
  const pair = (p: unknown, a: string, b: string) => {
    if (!isObj(p)) return null;
    const x = num(p[a]);
    const y = num(p[b]);
    return x !== null && y !== null && x >= 0 && y > 0 && x <= y ? { x, y } : null;
  };
  const plan = pair(o.plan, 'done', 'total');
  const m = pair(o.matches, 'met', 'total');
  return {
    lead: str(o.lead, 80),
    builders: strList(o.builders, 6).map((n) => n.slice(0, 80)),
    reviewer: str(o.reviewer, 80),
    team: str(o.team, 80),
    branch: str(o.branch, 200),
    pr,
    sha: str(o.sha, 64),
    merged_at: str(o.merged_at, 40),
    plan: plan ? { done: plan.x, total: plan.y } : null,
    started_at: str(o.started_at, 40),
    matches: m ? { met: m.x, total: m.y } : null,
    review_card_id: str(o.review_card_id, 120),
    task_url: safeHref(o.task_url),
    error: str(o.error, 300),
  };
};

/** One ticket row, or null when it has no key or summary (the card then isn't drawn; the list says so). */
export function parseTicket(raw: unknown): Ticket | null {
  const o = isObj(raw) && isObj(raw.ticket) ? raw.ticket : raw;
  if (!isObj(o)) return null;
  const key = str(o.key, 40);
  if (!key || !/^[A-Z][A-Z0-9_]*-\d+$/.test(key)) return null;
  const summary = str(o.summary, 300);
  if (!summary) return null;
  return {
    key,
    url: safeHref(o.url),
    summary,
    issue_type: str(o.issue_type, 40),
    status: str(o.status, 60),
    status_category: str(o.status_category) ? oneOf(o.status_category, ['todo', 'in_progress', 'done'] as const, 'todo') : null,
    priority: str(o.priority, 40),
    priority_rank: num(o.priority_rank),
    assignee_name: str(o.assignee_name, 80),
    reporter_name: str(o.reporter_name, 80),
    labels: strList(o.labels, 20),
    // An unknown state never reads as progress: it shows as waiting, which asks for nothing on its own.
    state: oneOf(o.state, TICKET_STATES, 'waiting'),
    handed_over_by: str(o.handed_over_by) ? oneOf(o.handed_over_by, ['label', 'assignee', 'jql'] as const, 'jql') : null,
    handed_over_at: str(o.handed_over_at, 40),
    taken_back_at: str(o.taken_back_at, 40),
    taken_back_by: str(o.taken_back_by, 80),
    closed_resolution: str(o.closed_resolution, 120),
    waiting_since_connect: o.waiting_since_connect === true || o.waiting_since_connect === 1,
    asked_comment_id: str(o.asked_comment_id, 80),
    jira_created: str(o.jira_created, 40),
    jira_updated: str(o.jira_updated, 40),
    updated_at: str(o.updated_at, 40),
    work: work(o.work),
  };
}

/** GET …/tickets: `{tickets, slots}`. Rows that can't be read are counted, not drawn. */
export function parseTicketList(raw: unknown): { tickets: Ticket[]; dropped: number; slots: number | null } {
  const rows = arr(isObj(raw) ? raw.tickets : raw);
  const tickets = rows.map(parseTicket).filter((t): t is Ticket => !!t);
  return { tickets, dropped: rows.length - tickets.length, slots: isObj(raw) ? num(raw.slots) : null };
}

/** The ticket waits on the person: a blocking question or a brief waiting for OK (design §7: the badge counts needs, not news). */
export const ticketNeedsYou = (t: Pick<Ticket, 'state'>): boolean => t.state === 'needs_answer' || t.state === 'brief_ready';

/** The Tickets tab count and the sidebar badge: needs only. */
export const ticketsNeedCount = (ts: Pick<Ticket, 'state'>[]): number => ts.filter(ticketNeedsYou).length;

const at = (t: Ticket) => Date.parse(t.updated_at ?? t.jira_updated ?? '') || 0;

/**
 * Design §3 "Where": needs first, then the rest by most recent activity; backfilled Waiting last (in their
 * own group); closed after everything.
 */
export function orderTickets(ts: Ticket[]): { active: Ticket[]; waiting: Ticket[]; closed: Ticket[] } {
  const byRecent = (a: Ticket, b: Ticket) => at(b) - at(a) || a.key.localeCompare(b.key);
  const needs = ts.filter(ticketNeedsYou).sort(byRecent);
  const rest = ts.filter((t) => !ticketNeedsYou(t) && t.state !== 'waiting' && t.state !== 'closed').sort(byRecent);
  return {
    active: [...needs, ...rest],
    waiting: ts.filter((t) => t.state === 'waiting').sort(byRecent),
    closed: ts.filter((t) => t.state === 'closed').sort(byRecent),
  };
}

// ---------------------------------------------------------------------------------------------------
// Events and the reading checklist

export interface TicketEvent {
  id: number | null;
  kind: 'state' | 'hook' | 'comment' | 'note';
  from_state: string | null;
  to_state: string | null;
  detail: string | null;
  at: string | null;
}

export const parseTicketEvents = (v: unknown): TicketEvent[] =>
  arr(v)
    .filter(isObj)
    .slice(0, 100)
    .map((e) => ({
      id: num(e.id),
      kind: oneOf(e.kind, ['state', 'hook', 'comment', 'note'] as const, 'note'),
      from_state: str(e.from_state, 40),
      to_state: str(e.to_state, 40),
      detail: str(e.detail, 300),
      at: str(e.at, 40),
    }));

export const READ_STEPS = ['ticket', 'comments', 'links', 'confluence', 'code'] as const;
export type ReadStep = (typeof READ_STEPS)[number];
export type ReadState = 'waiting' | 'running' | 'done' | 'skipped';
export type TicketProgress = Record<ReadStep, { state: ReadState; detail: string | null }>;

export const emptyProgress = (): TicketProgress =>
  Object.fromEntries(READ_STEPS.map((s) => [s, { state: 'waiting' as ReadState, detail: null }])) as TicketProgress;

/**
 * Folds one `dev_ticket_progress` SSE message (`{type, data:{space_id,key,step,state,detail}}` or its
 * `data`) into the checklist for `key`. Anything for another ticket or of the wrong shape is ignored.
 */
export function applyProgress(p: TicketProgress, key: string, msg: unknown): TicketProgress {
  const d = isObj(msg) && isObj(msg.data) ? msg.data : msg;
  if (!isObj(d) || d.key !== key) return p;
  const step = oneOf(d.step, READ_STEPS, 'ticket');
  if (!READ_STEPS.includes(d.step as ReadStep)) return p;
  const state = oneOf(d.state, ['running', 'done', 'skipped'] as const, 'running');
  return { ...p, [step]: { state, detail: str(d.detail, 120) } };
}

/** The checklist's fixed words when the server sent no detail. */
export const READ_LABEL: Record<ReadStep, string> = {
  ticket: 'The ticket and its attachments',
  comments: 'Comments',
  links: 'Linked issues',
  confluence: 'Confluence',
  code: 'The code',
};

// ---------------------------------------------------------------------------------------------------
// The brief (Vega: brief.rs / brief.md; GET …/tickets/:key/brief)

export type SourceKind = 'ticket' | 'comment' | 'page' | 'file' | 'linked_issue';
export interface SourceRef {
  kind: SourceKind;
  label: string;
  /** https only; "" from the server for files (open by path), which becomes null here. */
  url: string | null;
  page_version: number | null;
  comment_author: string | null;
  file: string | null;
  line: number | null;
  cite: string | null;
}
export interface BriefLine {
  text: string;
  sources: SourceRef[];
  inferred: boolean;
}
export interface BriefAnswer {
  value: string;
  by: 'person' | 'jira_user';
  name: string | null;
  where: 'card' | 'chat' | 'jira';
  at: string | null;
  comment_url: string | null;
  went_with_default: boolean;
}
export interface BriefQuestion {
  id: string;
  text: string;
  why_blocks: string | null;
  conflict_sources: SourceRef[];
  choices: string[];
  default: string | null;
  default_reason: string | null;
  asked_on_jira: { at: string | null; comment_url: string | null; to: string | null } | null;
  answer: BriefAnswer | null;
  /** A Jira reply that didn't map to a choice: shown quoted, and the lead asks the person to confirm. */
  jira_reply: { text: string; name: string | null; at: string | null; comment_url: string | null } | null;
}
export interface BriefFlag {
  quote: string;
  from: SourceRef | null;
  why: string | null;
  by: 'kwant' | 'bot';
}
export interface Brief {
  key: string;
  author_bot: string | null;
  created_at: string | null;
  edited_by_person_at: string | null;
  counts: { parts: number; sources: number } | null;
  goal: BriefLine | null;
  done_when: BriefLine[];
  where: BriefLine[];
  plan: BriefLine[];
  assumptions: BriefLine[];
  questions: BriefQuestion[];
  flags: BriefFlag[];
}

export function parseSource(v: unknown): SourceRef | null {
  if (!isObj(v)) return null;
  const kind = oneOf(v.kind, ['ticket', 'comment', 'page', 'file', 'linked_issue'] as const, 'ticket');
  const file = str(v.file, 400);
  const line = num(v.line);
  const label = str(v.label, 120) ?? (file ? (line !== null ? `${file}:${line}` : file) : null);
  if (!label) return null;
  return {
    kind,
    label,
    url: safeHref(v.url),
    page_version: num(v.page_version),
    comment_author: str(v.comment_author, 80),
    file,
    line,
    cite: str(v.cite, 300),
  };
}

const sources = (v: unknown, max = 6) => arr(v).map(parseSource).filter((s): s is SourceRef => !!s).slice(0, max);

export function parseLine(v: unknown): BriefLine | null {
  if (typeof v === 'string') return str(v, 600) ? { text: v.slice(0, 600), sources: [], inferred: false } : null;
  if (!isObj(v)) return null;
  const text = str(v.text, 600);
  if (!text) return null;
  return { text, sources: sources(v.sources), inferred: v.inferred === true };
}

const lines = (v: unknown, max: number) => arr(v).map(parseLine).filter((l): l is BriefLine => !!l).slice(0, max);

const answer = (v: unknown): BriefAnswer | null => {
  if (!isObj(v)) return null;
  const value = str(v.value, 300);
  if (!value) return null;
  return {
    value,
    by: oneOf(v.by, ['person', 'jira_user'] as const, 'person'),
    name: str(v.name, 80),
    where: oneOf(v.where, ['card', 'chat', 'jira'] as const, 'card'),
    at: str(v.at, 40),
    comment_url: safeHref(v.comment_url),
    went_with_default: v.went_with_default === true,
  };
};

export function parseQuestion(v: unknown, i = 0): BriefQuestion | null {
  if (!isObj(v)) return null;
  const text = str(v.text, 400);
  if (!text) return null;
  const choices = arr(v.choices)
    .map((c) => str(c, 120))
    .filter((c): c is string => !!c)
    .slice(0, 6);
  const def = str(v.default, 120);
  const asked = isObj(v.asked_on_jira) ? v.asked_on_jira : null;
  const reply = isObj(v.jira_reply) && str(v.jira_reply.text) ? v.jira_reply : null;
  return {
    id: str(v.id, 80) ?? `q${i + 1}`,
    text,
    why_blocks: str(v.why_blocks, 400),
    conflict_sources: sources(v.conflict_sources, 4),
    choices,
    // A default that isn't one of the choices can't be picked by "Go with the defaults": shown as none.
    default: def && choices.includes(def) ? def : null,
    default_reason: str(v.default_reason, 300),
    asked_on_jira: asked ? { at: str(asked.at, 40), comment_url: safeHref(asked.comment_url), to: str(asked.to, 80) } : null,
    answer: answer(v.answer),
    jira_reply: reply ? { text: str(reply.text, 1000)!, name: str(reply.name, 80), at: str(reply.at, 40), comment_url: safeHref(reply.comment_url) } : null,
  };
}

/**
 * The brief, from `GET …/brief` (`{…, brief}`) or the brief itself. Null when there's no goal and no
 * done-when line: then the card shows the server's markdown as plain text (`briefFallbackText`).
 */
export function parseBrief(raw: unknown): Brief | null {
  const o = isObj(raw) && isObj(raw.brief) ? raw.brief : raw;
  if (!isObj(o)) return null;
  const goal = parseLine(o.goal);
  const done = lines(o.done_when, 12);
  if (!goal && done.length === 0) return null;
  const c = isObj(o.counts) ? o.counts : null;
  return {
    key: str(o.key, 40) ?? '',
    author_bot: str(o.author_bot, 80),
    created_at: str(o.created_at, 40),
    edited_by_person_at: str(o.edited_by_person_at, 40),
    counts: c && num(c.parts) !== null && num(c.sources) !== null ? { parts: num(c.parts)!, sources: num(c.sources)! } : null,
    goal,
    done_when: done,
    where: lines(o.where, 8),
    plan: lines(o.plan, 5),
    assumptions: lines(o.assumptions, 8),
    // At most 3 (J8): more than that is the server's bug, and we don't draw a fourth blocker.
    questions: arr(o.questions)
      .map(parseQuestion)
      .filter((q): q is BriefQuestion => !!q)
      .slice(0, 3),
    flags: arr(o.flags)
      .filter(isObj)
      .map((f) => ({ quote: str(f.quote, 400) ?? '', from: parseSource(f.from), why: str(f.why, 300), by: oneOf(f.by, ['kwant', 'bot'] as const, 'kwant') }))
      .filter((f) => f.quote)
      .slice(0, 5),
  };
}

/** The brief route's own `markdown`, kept as plain text for the fallback (never rendered as markdown). */
export const briefFallbackText = (raw: unknown): string | null => (isObj(raw) ? str(raw.markdown, 6000) : null);

/** Questions still waiting on an answer. */
export const openQuestions = (b: Pick<Brief, 'questions'> | null): BriefQuestion[] => (b ? b.questions.filter((q) => !q.answer) : []);

/** "6 parts, 11 sources" from the server's counts; nothing when it sent none (we don't count for it). */
export const briefMeta = (b: Pick<Brief, 'counts'>): string | null =>
  b.counts ? `${b.counts.parts} ${b.counts.parts === 1 ? 'part' : 'parts'}, ${b.counts.sources} ${b.counts.sources === 1 ? 'source' : 'sources'}` : null;

// ---------------------------------------------------------------------------------------------------
// Ask on Jira (Juno: POST …/tickets/:key/ask/preview and …/ask)

export interface AskPreview {
  key: string;
  to: string | null;
  text: string;
  as: { email: string | null; display_name: string | null };
  digest: string | null;
}
export interface AskPosted {
  comment_id: string | null;
  url: string | null;
  to: string | null;
  at: string | null;
}

export function parseAskPreview(v: unknown): AskPreview | null {
  if (!isObj(v)) return null;
  const text = str(v.text, 4000);
  if (!text) return null;
  const as = isObj(v.as) ? v.as : {};
  return { key: str(v.key, 40) ?? '', to: str(v.to, 80), text, as: { email: str(as.email, 200), display_name: str(as.display_name, 80) }, digest: str(v.digest, 200) };
}

export const parseAskPosted = (v: unknown): AskPosted =>
  isObj(v) ? { comment_id: str(v.comment_id, 80), url: safeHref(v.url), to: str(v.to, 80), at: str(v.at, 40) } : { comment_id: null, url: null, to: null, at: null };

/** The body Juno's preview and ask routes take (AskBody): the open questions' words and choices. */
export const askBody = (b: Pick<Brief, 'questions'>, team: string, digest?: string | null) => ({
  questions: openQuestions(b).map((q) => ({ text: q.text, choices: q.choices })),
  team,
  ...(digest ? { digest } : {}),
});

/**
 * The preview text split around the mention, so "@Dana Park" can be drawn as a mention chip. Text only:
 * the parts are rendered as React strings.
 */
export function mentionParts(text: string, to: string | null): { text: string; mention: boolean }[] {
  if (!to) return [{ text, mention: false }];
  const m = `@${to}`;
  const out: { text: string; mention: boolean }[] = [];
  let rest = text;
  for (let i = rest.indexOf(m); i >= 0; i = rest.indexOf(m)) {
    if (i > 0) out.push({ text: rest.slice(0, i), mention: false });
    out.push({ text: m, mention: true });
    rest = rest.slice(i + m.length);
  }
  if (rest) out.push({ text: rest, mention: false });
  return out;
}

/**
 * Inline code in a brief line: `OrdersPage.tsx` becomes a code chip. Split on backticks only, so the result
 * is always plain strings; no other markup is read.
 */
export function codeParts(text: string): { text: string; code: boolean }[] {
  const bits = text.split('`');
  // An odd count of backticks: the last one has no pair, so it stays text.
  if (bits.length % 2 === 0) bits.splice(bits.length - 2, 2, `${bits[bits.length - 2]}\`${bits[bits.length - 1]}`);
  return bits.map((b, i) => ({ text: b, code: i % 2 === 1 })).filter((b) => b.text !== '');
}

// ---------------------------------------------------------------------------------------------------
// Matches the ticket

/** "2 of 4 met" from the review card's intent rows. Counting rows, not a new figure. */
export function matchCount(rows: { state: string }[]): { met: number; total: number } {
  return { met: rows.filter((r) => r.state === 'met').length, total: rows.length };
}

/** The holo sticker on Landed: only when the review says every done-when line is met (design §3 rules). */
export const earnsMatches = (t: Pick<Ticket, 'state' | 'work'>): boolean =>
  t.state === 'landed' && !!t.work.matches && t.work.matches.total > 0 && t.work.matches.met === t.work.matches.total;

// ---------------------------------------------------------------------------------------------------
// Jira site (GET …/dev/jira; Pixel's setup and settings read the full row)

export interface JiraSite {
  site: string | null;
  tickets: number;
}
export const parseJiraSite = (v: unknown): JiraSite | null => {
  if (!isObj(v)) return null;
  const site = isObj(v.site) ? str(v.site.jira_url, 300) : str(v.site, 300);
  if (!site) return null;
  return { site, tickets: num(v.tickets) ?? 0 };
};
