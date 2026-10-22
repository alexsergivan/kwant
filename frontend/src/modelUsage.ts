// Per-bot model calls and tokens from `GET /api/kwants/:id/model-usage` (API-key and custom API bots only).
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

export const modelUsageApi = {
  get: (kwantId: string, limit = 20) =>
    request<ModelUsage>('GET', `/api/kwants/${encodeURIComponent(kwantId)}/model-usage?limit=${limit}`),
};
