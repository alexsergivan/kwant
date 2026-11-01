import { personFetch } from './auth';

// The ask card (docs/channel-routing.md, "Ask band"): when a channel message is a close call between
// two bots, the router posts a card instead of answering, and nothing runs until the person taps who
// answers (or "Answer generally").

export interface RouteChoiceBot {
  kwant_id: string;
  name: string;
  title: string;
}

export interface RouteChoiceMeta {
  route_choice_id: string;
  channel_id: string;
  user_message_id: string;
  status: 'open' | 'resolved';
  reason: string;
  choices: RouteChoiceBot[];
  /** Who "Answer generally" goes to; `kwant_id` is null when the Concierge doesn't exist yet. */
  general: { label: string; name: string; kwant_id: string | null };
  chosen_kwant_id: string | null;
  chose_general: boolean;
}

const str = (v: unknown) => (typeof v === 'string' ? v : '');

function parseBot(raw: unknown): RouteChoiceBot | null {
  if (!raw || typeof raw !== 'object') return null;
  const b = raw as Record<string, unknown>;
  return str(b.kwant_id) && str(b.name) ? { kwant_id: str(b.kwant_id), name: str(b.name), title: str(b.title) } : null;
}

/** The card's meta from a message's `meta` (JSON text or object) or a `route_choice_updated` event. */
export function parseRouteChoiceMeta(raw: unknown): RouteChoiceMeta | null {
  let m: unknown = raw;
  if (typeof raw === 'string') {
    try {
      m = JSON.parse(raw);
    } catch {
      return null;
    }
  }
  if (!m || typeof m !== 'object') return null;
  const o = m as Record<string, unknown>;
  if (o.kind !== 'route_choice' || !str(o.route_choice_id)) return null;
  const g = (o.general && typeof o.general === 'object' ? o.general : {}) as Record<string, unknown>;
  return {
    route_choice_id: str(o.route_choice_id),
    channel_id: str(o.channel_id),
    user_message_id: str(o.user_message_id),
    status: o.status === 'resolved' ? 'resolved' : 'open',
    reason: str(o.reason),
    choices: (Array.isArray(o.choices) ? o.choices : []).map(parseBot).filter((b): b is RouteChoiceBot => !!b),
    general: { label: str(g.label) || 'Answer generally', name: str(g.name), kwant_id: str(g.kwant_id) || null },
    chosen_kwant_id: str(o.chosen_kwant_id) || null,
    chose_general: o.chose_general === true,
  };
}

/** The newest state of a card: a resolved card never goes back to open. */
export function routeChoiceView(meta: RouteChoiceMeta, live?: RouteChoiceMeta | null): RouteChoiceMeta {
  if (!live || live.route_choice_id !== meta.route_choice_id) return meta;
  return meta.status === 'resolved' && live.status === 'open' ? meta : live;
}

/** Keeps the newest card per id, from `route_choice_updated`. */
export function applyRouteChoice(map: Record<string, RouteChoiceMeta>, raw: unknown): Record<string, RouteChoiceMeta> {
  const card = parseRouteChoiceMeta(raw);
  if (!card) return map;
  const prev = map[card.route_choice_id];
  return { ...map, [card.route_choice_id]: prev ? routeChoiceView(prev, card) : card };
}

/** "You picked Nutritionist." / "You asked for a general answer." once settled, else null. */
export function routeChoiceOutcome(c: RouteChoiceMeta): string | null {
  if (c.status !== 'resolved') return null;
  if (c.chose_general) return c.general.name ? `You asked for a general answer, so ${c.general.name} answers.` : 'You asked for a general answer.';
  const bot = c.choices.find((b) => b.kwant_id === c.chosen_kwant_id);
  return bot ? `You picked ${bot.name}.` : 'Already answered.';
}

/** What "Answer generally" will do, under its button. */
export function generalHint(c: RouteChoiceMeta): string {
  const { name, kwant_id } = c.general;
  if (!name) return '';
  return kwant_id ? `${name} answers` : `Adds ${name}, a generalist, to answer`;
}

export type ResolvePick = { kwant_id: string } | { general: true };

/** How a tap ended: sent, already answered (a double tap or another device), or an error to show. */
export type ResolveResult = { kind: 'sent' } | { kind: 'already' } | { kind: 'error'; message: string };

/** Turns the resolve route's reply into a result. 409 "already chosen" is not an error: the card
 *  just shows it's settled. Other 409s (no one can answer generally) and failures are shown. */
export async function resolveResult(res: Response): Promise<ResolveResult> {
  if (res.ok) return { kind: 'sent' };
  let message = '';
  try {
    const body = await res.json();
    if (body && typeof body.error === 'string') message = body.error;
  } catch {
    // not JSON
  }
  if (res.status === 409 && (!message || /already/i.test(message))) return { kind: 'already' };
  if (res.status === 409) return { kind: 'error', message: sentence(message) };
  if (res.status === 401) return { kind: 'error', message: 'Enter your access token, then tap again.' };
  if (res.status === 403) return { kind: 'error', message: 'Only you can choose who answers.' };
  if (res.status === 404) return { kind: 'error', message: 'This choice is gone. Send your message again.' };
  return { kind: 'error', message: message ? sentence(message) : `Couldn’t send your choice (HTTP ${res.status}).` };
}

const sentence = (s: string) => {
  const t = s.trim();
  return t ? `${t[0].toUpperCase()}${t.slice(1)}${/[.!?]$/.test(t) ? '' : '.'}` : t;
};

export async function resolveRouteChoice(id: string, pick: ResolvePick): Promise<ResolveResult> {
  try {
    const res = await personFetch(`/api/route-choices/${encodeURIComponent(id)}/resolve`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(pick),
    });
    return await resolveResult(res);
  } catch {
    return { kind: 'error', message: 'Could not reach the server.' };
  }
}

/** `/api/chat/send` answered "asking": a card is waiting for the person, and no reply is coming. */
export function isAskingReply(body: unknown): body is { status: 'asking'; route_choice_id: string; card_message_id?: string; user_message_id?: string } {
  return !!body && typeof body === 'object' && (body as { status?: unknown }).status === 'asking';
}

/** The id of the person's message a routed reply answers (`meta.routing.message_id`); null on
 *  @mentions, DMs and replies saved before the server recorded it. */
export function answeredMessageId(meta: string | null | undefined): string | null {
  if (!meta) return null;
  try {
    const id = JSON.parse(meta)?.routing?.message_id;
    return typeof id === 'string' && id ? id : null;
  } catch {
    return null;
  }
}

/** For each bot reply, the person's message "Wrong bot?" corrects and resends: the one its routing
 *  says it answers, even when the person wrote more since. Older replies without that id fall back
 *  to the person's latest saved channel message before them. Thread and optimistic (temp id)
 *  messages are never picked; a recorded id that isn't loaded gets no "Wrong bot?" rather than a guess. */
export function questionsOfReplies<M extends { id: string; sender_role: string; thread_id?: string | null; meta?: string | null }>(messages: M[]): Map<string, M> {
  const out = new Map<string, M>();
  const byId = new Map<string, M>();
  let last: M | null = null;
  for (const m of messages) {
    if (m.sender_role === 'user') {
      const saved = !m.id.startsWith('temp') && !m.thread_id;
      last = saved ? m : null;
      if (saved) byId.set(m.id, m);
    } else if (m.sender_role === 'kwant') {
      const answered = answeredMessageId(m.meta);
      const q = answered ? byId.get(answered) : last;
      if (q) out.set(m.id, q);
    }
  }
  return out;
}

/** The bots "Wrong bot?" can send to: those that answer in the channel (every bot in #general, else
 *  the members and lead), minus the Concierge, which the server won't take as a correction. */
export function rerouteTargetsFor<K extends { id: string; name: string; title: string }>(
  channel: { name: string; lead_kwant_id: string; member_kwants: string[] } | null | undefined,
  kwants: K[],
): Array<{ id: string; name: string; title: string }> {
  if (!channel) return [];
  const ids = channel.name === 'general' ? null : new Set([channel.lead_kwant_id, ...channel.member_kwants]);
  return kwants
    .filter((k) => (ids ? ids.has(k.id) : true) && k.name !== 'Concierge')
    .map((k) => ({ id: k.id, name: k.name, title: k.title }));
}

/** "This should have gone to X": teaches routing. Returns an error sentence, or null when learned. */
export async function correctRoute(messageId: string, kwantId: string): Promise<string | null> {
  try {
    const res = await personFetch('/api/route-corrections', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message_id: messageId, kwant_id: kwantId }),
    });
    if (res.ok) return null;
    const body = await res.json().catch(() => null);
    const msg = body && typeof body.error === 'string' ? body.error : '';
    if (res.status === 403) return 'Only you can change who answers.';
    return msg ? sentence(msg) : `Couldn’t change who answers (HTTP ${res.status}).`;
  } catch {
    return 'Could not reach the server.';
  }
}
