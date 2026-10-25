// Channels people make themselves: the name rules, what "New channel" sends, and when Laya picks
// who answers. No imports, so the node tests can load it on its own.

const unique = (ids: string[]) => ids.filter((id, i) => ids.indexOf(id) === i);

type ChannelLike = { name: string; lead_kwant_id?: string | null; member_kwants?: string[] | string | null; routing_mode?: string | null };

/** What a typed name becomes: no leading #, lower case, spaces and other characters as dashes. */
export function channelSlug(typed: string): string {
  return typed
    .trim()
    .replace(/^#+/, '')
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-|-$/g, '');
}

/** Why `slug` can't be a new channel here, or null when it can. */
export function channelNameError(slug: string, existing: { name: string }[]): string | null {
  if (!slug) return 'Give the channel a name.';
  if (slug.length > 80) return 'Use 80 characters or fewer.';
  if (slug.startsWith('team-')) return 'Names starting with "team-" are kept for team chats.';
  if (existing.some((c) => c.name.toLowerCase() === slug)) return `#${slug} already exists in this space.`;
  return null;
}

export interface NewChannelBody {
  name: string;
  description?: string;
  lead_kwant_id: string;
  member_kwants: string[];
  routing_mode: 'auto';
}

/** The `POST /api/channels` body: the first bot picked leads, and the lead is always a member. */
export function newChannelBody(name: string, description: string, members: string[]): NewChannelBody {
  const ids = unique(members);
  const body: NewChannelBody = { name, lead_kwant_id: ids[0] ?? '', member_kwants: ids, routing_mode: 'auto' };
  const d = description.trim();
  if (d) body.description = d;
  return body;
}

/** A channel's member list: the server sends it as a JSON string, older code as an array. */
function memberList(raw: ChannelLike['member_kwants']): string[] {
  if (Array.isArray(raw)) return raw;
  if (typeof raw !== 'string') return [];
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

/** The channel's bots: its members, plus its lead where an older row leaves it out. */
export function channelBotIds(c: ChannelLike): string[] {
  const ids = memberList(c.member_kwants);
  return unique(c.lead_kwant_id ? [c.lead_kwant_id, ...ids] : ids);
}

/** Whether Laya picks who answers in `c`: #general once the space has 2+ bots, any other channel
 * on auto routing with 2+ bots of its own. */
export function layaRoutes(c: ChannelLike, botsInSpace: number): boolean {
  if (c.name === 'general') return botsInSpace >= 2;
  return (c.routing_mode ?? 'auto') === 'auto' && channelBotIds(c).length >= 2;
}

/** The line under the bot picker, for `picked` bots. */
export function routingHint(picked: number): string {
  if (picked === 0) return 'Pick at least one bot.';
  if (picked === 1) return 'With one bot, it answers every message. Add another and Laya picks who answers.';
  return 'Laya picks which of these bots answers each message.';
}
