// The line under "Sending your question to X" on Laya's routing card: why that bot, in words that
// match how it was actually picked.

export type RouteBasis = 'skills' | 'lead' | 'keywords' | 'only' | 'several' | 'discuss' | 'team';

export interface RouteReasonInput {
  kwantName?: string;
  reason?: string;
  basis?: string;
  fallback?: boolean;
  matchedTerms?: string[];
}

/** True when the server wrote its own sentence for this pick. Older servers say "Laya picked …" for
 *  every pick, even the lead fallback, so that wording is never trusted when a basis says otherwise. */
export function serverReason(r: { reason?: string; basis?: string }): string | null {
  const reason = r.reason?.trim();
  if (!reason || !r.basis || /^laya picked\b/i.test(reason)) return null;
  return /[.!?]$/.test(reason) ? reason : `${reason}.`;
}

export function routeReasonText(r: RouteReasonInput): string | null {
  const own = serverReason(r);
  if (own) return own;
  const name = r.kwantName || 'the team lead';
  const terms = (r.matchedTerms ?? []).filter(Boolean).slice(0, 4);
  switch (r.basis) {
    case 'lead':
      return `No one’s role matched this, so it went to the team lead, ${name}.`;
    case 'only':
      return `${name} is the only bot here that can answer.`;
    case 'team':
      return `This asks to put a team of bots together, so it went to the lead, ${name}, to propose them for you to approve.`;
    case 'keywords':
      return terms.length ? `Picked by matching words: ${terms.join(', ')}.` : 'Picked by matching words in your message.';
    default:
      // Older servers phrase every pick, even the lead fallback, as "Laya picked …".
      return r.reason || null;
  }
}
