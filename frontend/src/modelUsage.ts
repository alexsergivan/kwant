// Per-bot model calls and tokens from `GET /api/kwants/:id/model-usage` (API-key, custom API and
// Claude CLI bots), per-day totals from `GET /api/usage/daily`, and the split on each reply.
import { request } from './threads';

export type UsagePurpose = 'reply' | 'tool_round' | 'summary' | string;

export interface UsageSums {
  calls: number;
  fresh_input_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  output_tokens: number;
  tools_chars: number;
  system_chars: number;
  messages_chars: number;
  /** USD, summed over the calls that reported a cost (Claude CLI does); null when none did. */
  cost_usd?: number | null;
}

export interface UsageCall {
  id: number;
  context_id: string;
  purpose: UsagePurpose;
  model: string;
  fresh_input_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  output_tokens: number;
  /** False when the provider sent no usage numbers for this call. */
  reported: boolean;
  tools_chars: number;
  system_chars: number;
  messages_chars: number;
  cost_usd?: number | null;
  created_at: string;
}

export interface ModelUsage {
  kwant_id: string;
  since: string | null;
  totals: UsageSums;
  by_purpose: Array<UsageSums & { purpose: UsagePurpose }>;
  /** Newest first. */
  recent: UsageCall[];
}

const PURPOSE_LABEL: Record<string, [string, string]> = {
  reply: ['Reply', 'Replies'],
  tool_round: ['Tool round', 'Tool rounds'],
  summary: ['Summary', 'Summaries'],
};
/** "Replies" for a group, "Reply" for one call; unknown purposes as they are. */
export const purposeLabel = (p: UsagePurpose, one = false): string => PURPOSE_LABEL[p]?.[one ? 0 : 1] ?? p;

/** 1234 → "1.2k", 2_500_000 → "2.5M"; small numbers as they are. */
export function compactTokens(n: number): string {
  const v = Number.isFinite(n) ? Math.max(0, n) : 0;
  if (v < 1000) return String(Math.round(v));
  if (v < 1_000_000) return `${+(v / 1000).toFixed(v < 100_000 ? 1 : 0)}k`;
  return `${+(v / 1_000_000).toFixed(1)}M`;
}

/** Share of input tokens served from the cache, 0–100, or null with no input. */
export function cacheShare(s: Pick<UsageSums, 'fresh_input_tokens' | 'cache_read_tokens' | 'cache_write_tokens'>): number | null {
  const input = s.fresh_input_tokens + s.cache_read_tokens + s.cache_write_tokens;
  return input > 0 ? Math.round((s.cache_read_tokens / input) * 100) : null;
}

/** One bot's totals for one UTC day (`day` is YYYY-MM-DD). */
export interface DailyUsage extends UsageSums {
  day: string;
  kwant_id: string;
  /** Null once the bot is deleted. */
  kwant_name?: string | null;
}

export interface DailyUsageResponse {
  days: number;
  /** Newest day first. */
  rows: DailyUsage[];
}

/** Tokens the model had to take in or write new: everything but cache reads. */
export const freshOf = (s: Pick<UsageSums, 'fresh_input_tokens' | 'cache_write_tokens' | 'output_tokens'>): number =>
  s.fresh_input_tokens + s.cache_write_tokens + s.output_tokens;

/** "$0.42", "$0.003", "<$0.001"; null without a cost. */
export function formatCost(usd: number | null | undefined): string | null {
  if (usd == null || !Number.isFinite(usd) || usd < 0) return null;
  if (usd === 0) return '$0';
  if (usd < 0.001) return '<$0.001';
  return `$${usd < 1 ? +usd.toFixed(3) : usd.toFixed(2)}`;
}

/** A reply's usage split, saved in its meta as `usage`. */
export interface TurnUsage {
  fresh_input: number;
  cache_read: number;
  cache_write: number;
  output: number;
  cost_usd?: number;
}

export function parseTurnUsage(meta: string | null | undefined): TurnUsage | null {
  if (!meta) return null;
  try {
    const u = JSON.parse(meta)?.usage;
    if (!u || typeof u !== 'object') return null;
    const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : 0);
    const cost = typeof u.cost_usd === 'number' && Number.isFinite(u.cost_usd) ? u.cost_usd : undefined;
    return { fresh_input: n(u.fresh_input), cache_read: n(u.cache_read), cache_write: n(u.cache_write), output: n(u.output), cost_usd: cost };
  } catch {
    return null;
  }
}

/** What a reply shows for its tokens: fresh tokens when the split is known, with the whole split
 *  (and cost) for the tooltip; otherwise the old total, which counts cache reads too. */
export function replyTokens(tokensUsed: number | null | undefined, meta: string | null | undefined): { count: number; label: string; detail: string } | null {
  const u = parseTurnUsage(meta);
  if (u) {
    const count = u.fresh_input + u.cache_write + u.output;
    const fmt = (n: number) => n.toLocaleString('en-US');
    const parts = [
      `${fmt(count)} fresh tokens: ${fmt(u.fresh_input)} new input, ${fmt(u.cache_write)} written to cache, ${fmt(u.output)} output`,
      `${fmt(u.cache_read)} more read from cache (cheap, not counted)`,
    ];
    const cost = formatCost(u.cost_usd);
    if (cost) parts.push(`Cost ${cost}`);
    return { count, label: `${compactTokens(count)} fresh`, detail: parts.join('. ') + '.' };
  }
  if (!tokensUsed) return null;
  return { count: tokensUsed, label: `${compactTokens(tokensUsed)} tokens`, detail: `${tokensUsed.toLocaleString('en-US')} tokens, cache reads included` };
}

export const modelUsageApi = {
  get: (kwantId: string, limit = 20) =>
    request<ModelUsage>('GET', `/api/kwants/${encodeURIComponent(kwantId)}/model-usage?limit=${limit}`),
  daily: (days = 14) => request<DailyUsageResponse>('GET', `/api/usage/daily?days=${days}`),
};
