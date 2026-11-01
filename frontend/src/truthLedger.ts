// Truth Ledger, Phase C2 (docs/mission-control.md, contract §2): the facts bots and the person saved,
// labelled when unconfirmed or disputed, the disputes the person settles, and which tasks rely on a
// fact. Shown only with the ledger `on`: off and shadow leave the app as it was.

import type { MissionError } from './missions';

export type LedgerSetting = 'off' | 'shadow' | 'on';
export type FactStatus = 'proposed' | 'accepted' | 'superseded' | 'disputed';
export type FactScope = 'bot' | 'team' | 'space';
export type DisputeStatus = 'open' | 'resolved' | 'dismissed';
export type DisputeCause = 'same_subject' | 'numbers_differ' | 'negation' | 'person' | 'bot';

export interface Fact {
  id: string;
  space_id: string;
  scope: FactScope | string;
  scope_id: string;
  statement: string;
  subject: string | null;
  source_kind: 'person' | 'bot' | 'import' | 'check' | 'task' | string;
  source_ref: string | null;
  author_kwant_id: string | null;
  trust: 'person' | 'bot' | 'outside' | string;
  status: FactStatus | string;
  superseded_by: string | null;
  valid_from: string;
  valid_until: string | null;
  memory_id: string | null;
  pinned: boolean;
  decided_by: string | null;
  decided_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface Dispute {
  id: string;
  space_id?: string;
  fact_a: string;
  fact_b: string;
  cause: DisputeCause | string;
  status: DisputeStatus | string;
  winner: string | null;
  resolved_by: string | null;
  created_at: string;
  resolved_at: string | null;
  /** Both facts, A then B; a side can be null if it was deleted. */
  facts?: Array<Fact | null>;
}

export interface FactEvent {
  id: number;
  kind: string;
  actor: string;
  detail: string | null;
  created_at: string;
}
export interface FactEvidence {
  fact_id: string;
  evidence_id: string | null;
  check_id: string | null;
  message_id: string | null;
  added_by: string;
  added_at: string;
}
export interface Citation {
  fact_id: string;
  cited_by_kind: 'task' | 'fact' | string;
  cited_by_id: string;
  created_at: string;
}
export interface FactDetail {
  fact: Fact;
  events: FactEvent[];
  evidence: FactEvidence[];
  citations: Citation[];
  disputes: Dispute[];
}

/** The SSE payloads: a fact or dispute changed (the board reloads what it shows). */
export interface FactChangedEvent {
  id: string;
  status?: string;
  space_id?: string;
  scope?: string;
  scope_id?: string;
  kwant_id?: string | null;
}
export interface FactDisputeEvent {
  id: string;
  status?: string;
  space_id?: string;
  fact_a?: string;
  fact_b?: string;
  cause?: string;
  winner?: string | null;
}

export const ledgerSetting = (v: unknown): LedgerSetting => (v === 'on' || v === 'shadow' ? v : 'off');


/** The label a fact carries in lists and prompts; none for one that simply holds. */
export function factLabel(status: string): { text: string; tone: 'amber' | 'rose' | 'zinc' } | null {
  switch (status) {
    case 'proposed':
      return { text: 'Unconfirmed', tone: 'amber' };
    case 'disputed':
      return { text: 'Disputed', tone: 'rose' };
    case 'superseded':
      return { text: 'Replaced', tone: 'zinc' };
    default:
      return null;
  }
}

/** In force: what bots read (accepted, proposed with its label, disputed with its label). */
export const factInForce = (f: Pick<Fact, 'status'>) => f.status !== 'superseded';

/** Whose fact it is: "Sandy’s note", "Team Design", "Everyone in this space". */
export function scopeText(f: Pick<Fact, 'scope' | 'scope_id'>, names: Record<string, string> = {}, teams: Record<string, string> = {}): string {
  if (f.scope === 'bot') return `${names[f.scope_id] ?? 'A bot'}’s note`;
  if (f.scope === 'team') return teams[f.scope_id] ? `Team ${teams[f.scope_id]}` : 'A team';
  return 'Everyone in this space';
}

/** Where it came from, in a few words. */
export function sourceText(f: Pick<Fact, 'source_kind' | 'trust' | 'author_kwant_id'>, names: Record<string, string> = {}): string {
  const by = f.author_kwant_id ? names[f.author_kwant_id] ?? 'a bot' : null;
  switch (f.source_kind) {
    case 'person':
      return 'You wrote it';
    case 'import':
      return 'Imported';
    case 'check':
      return 'From a passing check';
    case 'task':
      return by ? `From ${by}’s task` : 'From a task';
    default:
      return f.trust === 'outside' ? `${by ? `${by}, ` : ''}from something it read outside` : by ? `${by} saved it` : 'A bot saved it';
  }
}

/** Why kwant thinks two facts disagree. */
export function causeText(cause: string): string {
  switch (cause) {
    case 'same_subject':
      return 'Both are about the same thing and say different things.';
    case 'numbers_differ':
      return 'They say nearly the same thing with different numbers.';
    case 'negation':
      return 'One says the opposite of the other.';
    case 'person':
      return 'You marked them as conflicting.';
    case 'bot':
      return 'A bot found they conflict.';
    default:
      return 'They can’t both hold.';
  }
}

/** A dispute's outcome line once it's no longer open. */
export function disputeOutcome(d: Pick<Dispute, 'status' | 'winner' | 'fact_a' | 'resolved_by'>, sides: Array<Fact | null | undefined>): string {
  if (d.status === 'dismissed') return 'Not a conflict: both stand, and this pair won’t be raised again.';
  if (d.status === 'resolved') {
    const w = sides.find((f) => f?.id === d.winner);
    const by = d.resolved_by && d.resolved_by !== 'person' ? ' (settled by a passing check)' : '';
    return w ? `Settled${by}: “${w.statement}” stands; the other is replaced.` : `Settled${by}.`;
  }
  return 'Waiting for you.';
}

/** A 409 on resolve or dismiss: someone else (or a check) got there first, or the ledger was switched off. */
export function disputeConflictText(e: MissionError): string {
  switch (e.code) {
    case 'resolved':
      return 'Someone got there first: this one is already settled.';
    case 'dismissed':
      return 'Someone got there first: it’s already marked as not a conflict.';
    default:
      return /isn.t on/i.test(e.message) ? 'The Truth Ledger was switched off, so facts can’t be changed right now.' : e.message;
  }
}

/** A 409 on accept/reject of an unconfirmed fact. */
export function factConflictText(e: MissionError): string {
  if (e.code === 'accepted') return 'Someone got there first: it’s already confirmed.';
  if (e.code === 'superseded') return 'Someone got there first: it’s already replaced.';
  if (e.code === 'disputed') return 'It’s disputed now: settle the dispute instead.';
  return /isn.t on|is off/i.test(e.message) ? 'The Truth Ledger was switched off, so facts can’t be changed right now.' : e.message;
}

/** Open disputes oldest first; the facts list with disputed, then unconfirmed, then the rest (newest first). */
export function sortFacts(list: Fact[]): Fact[] {
  const rank = (s: string) => (s === 'disputed' ? 0 : s === 'proposed' ? 1 : s === 'superseded' ? 3 : 2);
  return [...list].sort((a, b) => rank(a.status) - rank(b.status) || b.created_at.localeCompare(a.created_at));
}

export type FactFilter = 'all' | 'proposed' | 'disputed';
export const filterFacts = (list: Fact[], f: FactFilter) => (f === 'all' ? list.filter(factInForce) : list.filter((x) => x.status === f));

/** "Relied on by 2 tasks" (citations by tasks only; a fact citing a fact is internal). */
export function citationText(cs: Citation[]): string {
  const n = cs.filter((c) => c.cited_by_kind === 'task').length;
  return n === 0 ? 'No task relies on it yet.' : `${n} ${n === 1 ? 'task relies' : 'tasks rely'} on it.`;
}

/** Needs-you kinds on tasks with the ledger on. */
export const FACT_NEEDS_YOU = ['fact_changed', 'rests_on_disputed_fact'] as const;
export type FactNeedsYouKind = (typeof FACT_NEEDS_YOU)[number];
export const isFactNeedsYou = (k: string): k is FactNeedsYouKind => (FACT_NEEDS_YOU as readonly string[]).includes(k);
