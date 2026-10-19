// The line under "Sending your question to X" on Laya's routing card: why that bot, in words that
// match how it was actually picked.

export type RouteBasis = 'skills' | 'lead' | 'keywords' | 'only';

export interface RouteReasonInput {
  kwantName?: string;
  reason?: string;
  basis?: string;
  fallback?: boolean;
  matchedTerms?: string[];
}

export function routeReasonText(r: RouteReasonInput): string | null {
  const name = r.kwantName || 'the team lead';
  const terms = (r.matchedTerms ?? []).filter(Boolean).slice(0, 4);
  switch (r.basis) {
    case 'lead':
      return `No one’s role matched this, so it went to the team lead, ${name}.`;
    case 'only':
      return `${name} is the only bot here that can answer.`;
    case 'keywords':
      return terms.length ? `Picked by matching words: ${terms.join(', ')}.` : 'Picked by matching words in your message.';
    default:
      // Older servers phrase every pick, even the lead fallback, as "Laya picked …".
      return r.reason || null;
  }
}
