// Skills: short written procedures an agent proposes and a person reviews (batch 2 / B).
import { request } from './threads';

export type SkillStatus = 'pending' | 'enabled' | 'disabled' | 'rejected' | 'superseded';

export interface Skill {
  id: string;
  /** null = shared by every agent. */
  kwant_id: string | null;
  name: string;
  description: string;
  body: string;
  status: SkillStatus;
  source: 'agent' | 'user';
  proposed_by: string | null;
  /** A proposed revision: the enabled skill it replaces once approved. */
  replaces_id: string | null;
  created_at: string;
  updated_at: string;
  reviewed_at: string | null;
}

/** `skill_updated` after a delete. */
export interface DeletedSkill {
  id: string;
  kwant_id: string | null;
  deleted: true;
}

export interface DiffLine {
  op: 'same' | 'add' | 'del';
  text: string;
}

export interface SkillDiff {
  skill: Skill;
  base: Skill | null;
  description_changed: boolean;
  lines: DiffLine[];
  unified: string;
}

export interface SkillDraft {
  name: string;
  description: string;
  body: string;
}

export const SKILL_LIMITS = { name: 48, description: 200, body: 8000 } as const;
const NAME_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

/** First problem with a draft (same rules as the server), or null when it can be saved. */
export function skillDraftError(d: SkillDraft): string | null {
  const name = d.name.trim();
  if (!name) return 'Give it a name.';
  if (name.length > SKILL_LIMITS.name) return `Names are at most ${SKILL_LIMITS.name} characters.`;
  if (!NAME_RE.test(name)) return 'Use lowercase letters, digits and dashes, e.g. weekly-report.';
  if (d.description.trim().length > SKILL_LIMITS.description) return `Descriptions are at most ${SKILL_LIMITS.description} characters.`;
  if (!d.body.trim()) return 'Write the steps.';
  if (d.body.length > SKILL_LIMITS.body) return `The steps are at most ${SKILL_LIMITS.body} characters.`;
  return null;
}

/** Turns free text into a valid-looking name ("Weekly Report!" -> "weekly-report"). */
export function slugifySkillName(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, SKILL_LIMITS.name)
    .replace(/-+$/, '');
}

/** Whether a skill applies to this agent: its own, or shared. */
export const skillAppliesTo = (s: Pick<Skill, 'kwant_id'>, kwantId: string) => s.kwant_id === null || s.kwant_id === kwantId;

/**
 * Applies a `skill_updated` payload to a list (newest first). Skills of other agents are ignored
 * when `kwantId` is given; a deleted one is dropped. Returns the same list when nothing changed.
 */
export function applySkillEvent(list: Skill[], ev: Skill | DeletedSkill, kwantId?: string): Skill[] {
  const i = list.findIndex((s) => s.id === ev.id);
  if ('deleted' in ev && ev.deleted) return i < 0 ? list : list.filter((s) => s.id !== ev.id);
  const skill = ev as Skill;
  if (kwantId !== undefined && !skillAppliesTo(skill, kwantId)) return i < 0 ? list : list.filter((s) => s.id !== ev.id);
  if (i < 0) return [skill, ...list];
  const next = list.slice();
  next[i] = skill;
  return next;
}

export interface SkillGroups {
  pending: Skill[];
  /** Enabled first, then disabled; each by name. */
  active: Skill[];
  /** Rejected and replaced, kept for the record. */
  history: Skill[];
}

export function groupSkills(list: Skill[]): SkillGroups {
  const byName = (a: Skill, b: Skill) => a.name.localeCompare(b.name);
  return {
    pending: list.filter((s) => s.status === 'pending'),
    active: [
      ...list.filter((s) => s.status === 'enabled').sort(byName),
      ...list.filter((s) => s.status === 'disabled').sort(byName),
    ],
    history: list.filter((s) => s.status === 'rejected' || s.status === 'superseded'),
  };
}

/** Pending skills per agent (shared ones count for nobody: they show in every agent's list). */
export function pendingByKwant(list: Skill[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const s of list) if (s.status === 'pending' && s.kwant_id) out[s.kwant_id] = (out[s.kwant_id] ?? 0) + 1;
  return out;
}

const enc = encodeURIComponent;

export const skillsApi = {
  list: (q: { kwant_id?: string; status?: SkillStatus } = {}) => {
    const p = new URLSearchParams();
    if (q.kwant_id) p.set('kwant_id', q.kwant_id);
    if (q.status) p.set('status', q.status);
    const qs = p.toString();
    return request<Skill[]>('GET', `/api/skills${qs ? `?${qs}` : ''}`);
  },
  create: (d: SkillDraft & { kwant_id?: string | null }) => request<Skill>('POST', '/api/skills', d),
  diff: (id: string) => request<SkillDiff>('GET', `/api/skills/${enc(id)}/diff`),
  approve: (id: string, edits?: Partial<SkillDraft>) => request<Skill>('POST', `/api/skills/${enc(id)}/approve`, edits ?? {}),
  reject: (id: string) => request<Skill>('POST', `/api/skills/${enc(id)}/reject`),
  patch: (id: string, body: Partial<SkillDraft> & { enabled?: boolean; kwant_id?: string | null }) =>
    request<Skill>('PATCH', `/api/skills/${enc(id)}`, body),
  remove: (id: string) => request<void>('DELETE', `/api/skills/${enc(id)}`),
};
