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
