// The Dev desk shell's view of a card. Iris owns the full card types in `./types.ts`; until hers
// land the shell needs only these fields, and passes the rest through to her renderers untouched.

export type CardKind = 'landed' | 'review' | 'release_notes';
export type CardStatus = 'pending' | 'ready' | 'failed' | 'stale';

export interface ShellCard {
  id: string;
  kind: CardKind | string;
  status: CardStatus | string;
  created_at?: string | null;
  merged_at?: string | null;
  pr?: number | null;
  title?: string | null;
  headline?: string | null;
  [k: string]: unknown;
}

export interface LandedRollup {
  since: string | null;
  until: string | null;
  count: number;
  needs_you: number;
  main_green: boolean | null;
  areas: Array<{ area: string; count: number; first_headline: string }>;
}

export interface LandedPage {
  cards: ShellCard[];
  rollup: LandedRollup | null;
  last_tag: string | null;
}

/** Same shape as iris-dev-cards `dev/types.ts` FindingAction. */
export type FindingAction = { action: 'fix' } | { action: 'undo_fix' } | { action: 'dismiss'; reason: 'not_a_problem' | 'fine_here' | 'later' | 'wrong'; note?: string };

const isCard = (v: unknown): v is ShellCard =>
  !!v && typeof v === 'object' && typeof (v as ShellCard).id === 'string' && typeof (v as ShellCard).kind === 'string' && typeof (v as ShellCard).status === 'string';

export function parseCards(raw: unknown): ShellCard[] {
  const list = raw && typeof raw === 'object' ? (raw as { cards?: unknown }).cards : null;
  return Array.isArray(list) ? list.filter(isCard) : [];
}

export function parseLandedPage(raw: unknown): LandedPage {
  const o = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const r = o.rollup && typeof o.rollup === 'object' ? (o.rollup as Record<string, unknown>) : null;
  return {
    cards: parseCards(raw),
    rollup: r
      ? {
          since: typeof r.since === 'string' ? r.since : null,
          until: typeof r.until === 'string' ? r.until : null,
          count: typeof r.count === 'number' ? r.count : 0,
          needs_you: typeof r.needs_you === 'number' ? r.needs_you : 0,
          main_green: typeof r.main_green === 'boolean' ? r.main_green : null,
          areas: Array.isArray(r.areas) ? (r.areas as LandedRollup['areas']) : [],
        }
      : null,
    last_tag: typeof o.last_tag === 'string' ? o.last_tag : null,
  };
}

export const asCard = (raw: unknown): ShellCard | null => (isCard(raw) ? raw : null);
