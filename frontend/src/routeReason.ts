// The line under "Sending your question to X" on Laya's routing card: why that bot, in words that
// match how it was actually picked.

export type RouteBasis =
  | 'skills'
  | 'lead'
  | 'keywords'
  | 'only'
  | 'several'
  | 'discuss'
  | 'team'
  | 'asked'
  | 'last'
  | 'concierge'
  | 'ask'
  | 'handback'
  | 'learned';

export interface RouteReasonInput {
  kwantName?: string;
  reason?: string;
  basis?: string;
  fallback?: boolean;
  matchedTerms?: string[];
  /** Basis "handback": the name of the bot that said it wasn't their area. */
  handedBackBy?: string;
}

/** "Lin handed this back (…), so Ada takes it": the server's handback sentence starts with the name. */
const handbackName = (reason?: string) => reason?.match(/^(\S+) handed this back\b/)?.[1];

/** The fixed line for the router's newer picks (Concierge, ask card, hand-back, learned), or null for
 *  any other basis. These always use the app's own wording, never the server's longer sentence. */
export function pickedLine(r: RouteReasonInput): string | null {
  const name = r.kwantName || 'another bot';
  switch (r.basis) {
    case 'concierge':
      return 'No specialist fits, answering generally.';
    case 'ask':
      return `You picked ${name}.`;
    case 'handback':
      return `${r.handedBackBy || handbackName(r.reason) || 'The first bot'} said it’s not their area, so ${name} took it.`;
    case 'learned':
      return `You sent a message like this to ${name} before.`;
    default:
      return null;
  }
}

/** True when the server wrote its own sentence for this pick. Older servers say "Laya picked …" for
 *  every pick, even the lead fallback, so that wording is never trusted when a basis says otherwise. */
export function serverReason(r: { reason?: string; basis?: string }): string | null {
  const reason = r.reason?.trim();
  if (!reason || !r.basis || /^laya picked\b/i.test(reason)) return null;
  return /[.!?]$/.test(reason) ? reason : `${reason}.`;
}

export function routeReasonText(r: RouteReasonInput): string | null {
  const picked = pickedLine(r);
  if (picked) return picked;
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

/** The two or three words on the chip beside a reply's name: how this bot was picked. The full
 *  sentence (routeReasonText / routingLine) opens when the chip is tapped. */
export function routeChipLabel(r: { basis?: string; fallback?: boolean }): string {
  if (r.fallback) return 'By keywords';
  switch (r.basis) {
    case 'skills':
      return 'Matched skills';
    case 'keywords':
      return 'Matched words';
    case 'lead':
    case 'team':
      return 'Team lead';
    case 'only':
      return 'Only bot here';
    case 'several':
      return 'Best of several';
    case 'discuss':
      return 'Team discussion';
    case 'asked':
    case 'ask':
      return 'You chose';
    case 'last':
      return 'Same as last time';
    case 'concierge':
      return 'General answer';
    case 'handback':
      return 'Handed over';
    case 'learned':
      return 'Learned from you';
    default:
      return 'Picked by Laya';
  }
}
