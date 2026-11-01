// Flight School (Mission Control Phase C3, docs/mission-control.md §14): a bot's tier per kind of
// work. A tier only adds approval cards, never removes one, and payments always ask. Off by default;
// with it off nothing here shows.

export type Tier = 'observe' | 'sandbox' | 'act';
export const TIERS: Tier[] = ['observe', 'sandbox', 'act'];
/** The class that covers every kind of work without its own tier. */
export const ANY_CLASS = '*';
export const DEFAULT_CLASS = 'general';

export const TIER_LABELS: Record<Tier, string> = { observe: 'Observe', sandbox: 'Sandbox', act: 'Act' };
export const TIER_HINTS: Record<Tier, string> = {
  observe: 'Plans, reads and reports. Every change asks you first.',
  sandbox: 'Changes files in its task folder when they can be undone. Commands that can change things ask you; reading its own folder doesn’t.',
  act: 'Works as it does today, with its usual approvals.',
};

export const isTier = (v: unknown): v is Tier => v === 'observe' || v === 'sandbox' || v === 'act';
export const tierLabel = (v: string | null | undefined): string => (isTier(v) ? TIER_LABELS[v] : 'Act');

export interface CapabilityPolicy {
  id: string;
  kwant_id: string;
  task_class: string;
  tier: Tier;
  granted_by: string;
  granted_at: string;
  revoked_at: string | null;
  note: string | null;
}

export interface CapabilityEvent {
  id: number;
  kwant_id: string;
  task_class: string;
  /** granted, revoked or new_bot_default; others are shown humanized. */
  kind: string;
  tier: Tier | null;
  actor: string;
  detail: string | null;
  created_at: string;
}

/** Where a class's tier comes from: its own row, the bot's `*` row, today's default, or a row kwant couldn't read. */
export type TierFrom = 'class' | 'any_class' | 'default' | 'unreadable';

export interface Capabilities {
  flight_school: boolean;
  kwant_id: string;
  default_tier: Tier;
  policies: CapabilityPolicy[];
  effective: Array<{ task_class: string; tier: Tier; from: TierFrom | string }>;
  events: CapabilityEvent[];
  /** C4: the bot's last 20 proposals, newest first (older servers leave it out). */
  proposals?: TierProposal[];
}

/** POST /api/capabilities and …/revoke. `unchanged`: it already had that tier (safe to repeat). */
export interface CapabilityChange {
  unchanged: boolean;
  capabilities: Capabilities;
}

export function fromText(from: string): string {
  switch (from) {
    case 'class':
      return 'Set for this kind of work';
    case 'any_class':
      return 'From its tier for all work';
    case 'unreadable':
      return 'Couldn’t be read, so it’s treated as Observe';
    default:
      return 'Not set: works as today (Act)';
  }
}

/** A task class as people read it: "code-review" → "Code review". */
export const classLabel = (c: string): string => {
  if (c === ANY_CLASS) return 'All work';
  if (c === DEFAULT_CLASS) return 'General';
  const words = c.replace(/[-_]+/g, ' ').replace(/\s+/g, ' ').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : c;
};

/** One line for a tier change in the bot's history. */
export function capabilityEventText(e: Pick<CapabilityEvent, 'kind' | 'tier' | 'task_class'>): string {
  const where = e.task_class === ANY_CLASS ? 'all work' : classLabel(e.task_class).toLowerCase();
  const tier = tierLabel(e.tier);
  switch (e.kind) {
    case 'granted':
      return `You set ${where} to ${tier}`;
    case 'revoked':
      return `You revoked ${where} to ${tier}`;
    case 'new_bot_default':
      return `Started at ${tier} as a new bot`;
    default:
      return `${e.kind.replace(/_/g, ' ')}: ${where}${e.tier ? `, ${tier}` : ''}`;
  }
}

/** The tier one Revoke lowers to, or null at Observe (nothing lower). */
export const revokeTarget = (tier: Tier): Tier | null => (tier === 'act' ? 'sandbox' : tier === 'sandbox' ? 'observe' : null);

/** A class slug the server accepts: 1–40 of a-z, 0-9 and -. */
export const validClass = (c: string): boolean => /^[a-z0-9-]{1,40}$/.test(c);

/** A typed kind of work as a slug: "Release notes" → "release-notes". */
export const toClass = (s: string): string =>
  s
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40);

export interface FlightSchoolSettings {
  flight_school: boolean;
  flight_school_new_bot_tier: Tier;
}

/** From GET /api/settings; null on a server without Flight School, so the switch stays hidden. */
export function parseFlightSchoolSettings(data: unknown): FlightSchoolSettings | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  if (typeof d.flight_school !== 'boolean') return null;
  return { flight_school: d.flight_school, flight_school_new_bot_tier: isTier(d.flight_school_new_bot_tier) ? d.flight_school_new_bot_tier : 'sandbox' };
}

/** Why the bot's tier asked, from an approval card's details, or null. Never set on a payment card. */
export function tierReason(details: Record<string, unknown> | null | undefined): string | null {
  const why = details?.tier_reason;
  if (typeof why !== 'string' || !why.trim()) return null;
  return why.trim().replace(/[.\s]+$/, '');
}

/** The Needs-you kind the engine guard (D7) raises, and what the person can do about it. */
export const ENGINE_CANT_SANDBOX = 'engine_cant_sandbox';
export const engineCantSandboxText = (bot: string | null) =>
  `${bot ?? 'Its bot'} runs on an engine that can’t be held to a lower tier, so nothing ran. Switch ${bot ?? 'the bot'} to a Claude model, or set its tier for this kind of work to Act, then press Unblock.`;

/** Task fields Flight School adds; `owner_tier` is null while the setting is off. */
export interface TaskTier {
  task_class?: string;
  owner_tier?: Tier | null;
}

/** Whether a task shows its kind of work and tier: only with Flight School on (the server sends `owner_tier`). */
export const showsTier = (t: TaskTier): t is { task_class?: string; owner_tier: Tier } => isTier(t.owner_tier);

// ---------------------------------------------------------------------------
// Scorecards and proposals (Phase C4, docs/mission-control.md §14): a bot earns the next tier from
// its own verified work, asks for it, and only the person grants or declines. Revoke stays one tap.

export interface Ratio {
  k: number;
  n: number;
  /** Null when n is 0. */
  rate: number | null;
}

export interface Scorecard {
  kwant_id: string;
  task_class: string;
  window_days: number;
  since: string;
  tier: Tier;
  tier_from: TierFrom | string;
  next_tier: Tier | null;
  eligible: boolean;
  /** Ready-to-show sentences, e.g. "2 more verified task(s) (has 3, needs 5)". */
  missing: string[];
  tasks: { settled: number; verified: number; failed: number; cancelled: number };
  first_try: Ratio;
  verified_rate: Ratio;
  false_success: Ratio;
  failures?: { total: number; by_class: Record<string, number> };
  cards?: { shown: number; approved: number; denied: number; approved_rate: Ratio };
  calls?: number;
  external_calls?: number;
  interventions?: number;
  tokens?: { spent: number; recovery: number };
  /** Null at Act: there is no next step to earn. */
  recent_denials: { n: number; of: number; with_denial: number } | null;
}

export type ProposalStatus = 'proposed' | 'granted' | 'declined' | 'stale';

export interface TierProposal {
  kind: 'tier_proposal';
  id: string;
  space_id: string;
  kwant_id: string;
  task_class: string;
  from_tier: Tier;
  to_tier: Tier;
  reason: string | null;
  status: ProposalStatus;
  note: string | null;
  decided_at: string | null;
  created_at: string;
  /** The scorecard when the bot asked. */
  scorecard: Partial<Scorecard> | null;
}

/** POST …/grant and …/decline answer the decided proposal and the bot's tiers after it. */
export interface ProposalDecision {
  proposal: TierProposal;
  capabilities: Capabilities;
}

/** The SSE `capability_changed` payload: reload that bot's tiers and the open proposals. */
export interface CapabilityChangedEvent {
  space_id?: string;
  kwant_id: string;
  task_class?: string;
  proposal?: TierProposal | null;
}

export const isOpenProposal = (p: Pick<TierProposal, 'status'>): boolean => p.status === 'proposed';

/** Open proposals, oldest first: the first to ask is the first in line. */
export const openProposals = (list: TierProposal[] | null | undefined): TierProposal[] =>
  (list ?? []).filter(isOpenProposal).sort((a, b) => a.created_at.localeCompare(b.created_at));

/** Puts a changed proposal into the list by id, newest first. */
export function upsertProposal(list: TierProposal[], p: TierProposal): TierProposal[] {
  return [p, ...list.filter((x) => x.id !== p.id)].sort((a, b) => b.created_at.localeCompare(a.created_at));
}

const pct = (r: number) => `${Math.round(r * 100)}%`;

/** "2 of 3 (67%)", or "None yet" when there's nothing to count. */
export function ratioText(r: Ratio | null | undefined): string {
  if (!r || r.n <= 0) return 'None yet';
  return r.rate === null ? `${r.k} of ${r.n}` : `${r.k} of ${r.n} (${pct(r.rate)})`;
}

/** "Sandy asks for Sandbox on Code" (the card's heading). */
export const proposalTitle = (p: Pick<TierProposal, 'task_class' | 'to_tier'>, bot: string): string =>
  `${bot} asks for ${tierLabel(p.to_tier)} on ${classLabel(p.task_class)}`;

/** "From Observe to Sandbox: …" (what changes if you grant it). */
export const proposalChange = (p: Pick<TierProposal, 'from_tier' | 'to_tier'>): string =>
  `${tierLabel(p.from_tier)} → ${tierLabel(p.to_tier)}. ${TIER_HINTS[isTier(p.to_tier) ? p.to_tier : 'act']}`;

/** What happened to a proposal that's no longer open, for the line that replaces its buttons. */
export function proposalOutcome(p: Pick<TierProposal, 'status' | 'to_tier' | 'task_class' | 'note'>, bot: string): string {
  const where = classLabel(p.task_class);
  switch (p.status) {
    case 'granted':
      return `Granted: ${bot} is now ${tierLabel(p.to_tier)} on ${where}. You can revoke it in one tap from ${bot}’s settings.`;
    case 'declined':
      return `Declined. ${bot} can ask again in 7 days.`;
    case 'stale':
      return `No longer holds${p.note ? `: ${p.note.replace(/[.\s]+$/, '')}` : ''}. ${bot} has to ask again.`;
    default:
      return 'Waiting for you.';
  }
}

/**
 * A 409 on grant or decline: someone (another tab or device) decided first, or the proposal went
 * stale. The server's `status` arrives as the error's code.
 */
export function proposalConflictText(status: string | null, message: string): string {
  switch (status) {
    case 'granted':
      return 'Someone got there first: it’s already granted.';
    case 'declined':
      return 'Someone got there first: it’s already declined.';
    case 'stale': {
      const why = /^this proposal is stale:\s*(.+?)[.\s]*$/i.exec(message.trim())?.[1];
      return `This request no longer holds${why ? `: ${why}` : ''}. The bot has to ask again.`;
    }
    default:
      return message;
  }
}

export interface ScorecardRow {
  label: string;
  value: string;
  /** Rose when it holds the bot back. */
  bad?: boolean;
}

/** The scorecard as label/value rows, in the order the rules read. */
export function scorecardRows(c: Partial<Scorecard>): ScorecardRow[] {
  const rows: ScorecardRow[] = [];
  const t = c.tasks;
  if (t) rows.push({ label: 'Verified', value: t.settled > 0 ? `${t.verified} of ${t.settled} settled` : 'None settled yet' });
  if (c.first_try) rows.push({ label: 'Verified on the first try', value: ratioText(c.first_try) });
  if (c.false_success) rows.push({ label: 'Said done, check failed', value: c.false_success.n > 0 ? `${c.false_success.k} of ${c.false_success.n}` : 'None yet', bad: c.false_success.k > 0 });
  if (c.recent_denials) {
    const d = c.recent_denials;
    rows.push({ label: `Denied in its last ${d.of}`, value: d.n > 0 ? `${d.with_denial} of ${d.n}` : 'None yet', bad: d.with_denial > 0 });
  }
  if (c.cards && c.cards.shown > 0) rows.push({ label: 'Approval cards', value: `${c.cards.approved} approved, ${c.cards.denied} denied`, bad: c.cards.denied > 0 });
  return rows;
}

/** One line under the scorecard: what it can ask for, or what it still needs. */
export function scorecardStatus(c: Pick<Scorecard, 'tier' | 'next_tier' | 'eligible' | 'missing'>): { text: string; ready: boolean } {
  if (!c.next_tier) return { text: `${tierLabel(c.tier)} is the top tier: nothing more to earn here.`, ready: false };
  if (c.eligible) return { text: `Qualifies to ask for ${tierLabel(c.next_tier)}. Only you can grant it.`, ready: true };
  const need = c.missing.map((m) => m.replace(/[.\s]+$/, '')).join('; ');
  return { text: `Not yet ready for ${tierLabel(c.next_tier)}${need ? `: needs ${need}` : ''}.`, ready: false };
}
