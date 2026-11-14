// What the empty-chat "bot home" and the composer's answer pill say, worked out from data the client
// already has: no new requests. Pure, so tests can check the words.

export interface HomeBot {
  id: string;
  name: string;
  title: string;
  avatar_sprite: string;
  /** "Good for" phrases routing reads; absent on older servers. */
  covers?: string[];
  preferred_model?: string;
  approvals_effective?: string | null;
}

export interface RecentItem {
  id: string;
  title: string;
  updated_at: string;
}

/** What the chat pane is: a bot's own chat, #general, a team's chat, or another channel. */
export type ChatKind = 'bot' | 'general' | 'team' | 'channel';

export interface ChatHome {
  kind: ChatKind;
  /** The bot, for a direct chat. */
  bot?: HomeBot;
  /** The bot's most recent other conversations, newest first; absent until they are loaded. */
  recent?: RecentItem[];
  onOpenRecent?: (id: string) => void;
  /** The team, for a team's chat: all of it from App's state, no new request. */
  team?: HomeTeam;
}

/** What a team's empty chat shows: its name, purpose, lead and members (lead first). */
export interface HomeTeam {
  name: string;
  /** The team's description; blank or absent means the home says who is in it instead. */
  purpose?: string;
  /** The lead's name. */
  lead?: string;
  members: HomeBot[];
}

/**
 * The team home's subtitle: its purpose when it has one, else who is in it ("3 bots who work together.
 * Ada leads."). One bot is "1 bot in this team."; no lead drops the second sentence.
 */
export function teamHomeLine(team: Pick<HomeTeam, 'purpose' | 'lead' | 'members'>): string {
  const purpose = clean(team.purpose ?? '');
  if (purpose) return purpose;
  const n = team.members.length;
  const who = n === 0 ? 'No bots in this team yet.' : n === 1 ? '1 bot in this team.' : `${n} bots who work together.`;
  const lead = clean(team.lead ?? '');
  return lead && n > 0 ? `${who} ${lead} leads.` : who;
}

/** Lead first, then the other members, each once: the faces and chips of a team home. */
export function teamMembersLeadFirst<T extends { id: string }>(members: T[], leadId?: string | null): T[] {
  const seen = new Set<string>();
  const lead = leadId ? members.filter((m) => m.id === leadId) : [];
  return [...lead, ...members].filter((m) => (seen.has(m.id) ? false : (seen.add(m.id), true)));
}

const MAX_CHIPS = 4;
const MAX_CHIP_LEN = 32;

const clean = (s: string) => s.replace(/\s+/g, ' ').trim();

/**
 * "Good at" chips: the bot's own "Good for" phrases (covers), at most four, short, no repeats.
 * Nothing beyond its role means none: the role is already the subtitle, so echoing it adds nothing.
 * Phrases that only repeat the role are dropped for the same reason.
 */
export function goodAt(bot: Pick<HomeBot, 'title' | 'covers'>): string[] {
  const role = clean(bot.title || '').toLowerCase();
  const parts = (bot.covers ?? []).map(clean).filter((p) => p && p.toLowerCase() !== role);
  const seen = new Set<string>();
  const out: string[] = [];
  for (const p of parts) {
    const text = p.length > MAX_CHIP_LEN ? `${p.slice(0, MAX_CHIP_LEN - 1).trimEnd()}…` : p;
    const key = text.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(text);
    if (out.length === MAX_CHIPS) break;
  }
  return out;
}

/** The two newest chats that aren't archived, from a bot's thread list. */
export function recentWork(
  threads: Array<{ id: string; title: string; updated_at: string; archived?: boolean; kind?: string }> | undefined,
  limit = 2,
): RecentItem[] | undefined {
  if (!threads) return undefined;
  return threads
    .filter((t) => !t.archived && t.title && t.kind !== 'delegation')
    .sort((a, b) => (a.updated_at < b.updated_at ? 1 : a.updated_at > b.updated_at ? -1 : 0))
    .slice(0, limit)
    .map(({ id, title, updated_at }) => ({ id, title, updated_at }));
}

/** Starter cards: the suggested prompts, capped, each with a first-letter capital. */
export function starterCards(prompts: string[], max = 4): string[] {
  return prompts
    .map(clean)
    .filter(Boolean)
    .slice(0, max)
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1));
}

export interface AnswerTarget {
  kind: 'laya' | 'bot' | 'team' | 'mention';
  /** The pill's words: "Laya picks", "@Nutritionist", "@Kitchen team". */
  label: string;
  /** The full sentence for the tooltip and screen readers. */
  detail: string;
}

/** Who will answer what's typed: a leading or inline @Name wins, otherwise the chat decides. */
export function answerTarget(opts: {
  kind: ChatKind;
  text: string;
  botName?: string;
  teamName?: string;
  names: string[];
}): AnswerTarget {
  const m = opts.text.match(/(?:^|\s)@([a-zA-Z0-9_-]+)/);
  if (m) {
    const hit = opts.names.find((n) => n.toLowerCase() === m[1].toLowerCase());
    if (hit) return { kind: 'mention', label: `@${hit}`, detail: `@${hit} will answer, because you mentioned them.` };
  }
  if (opts.kind === 'bot') {
    return { kind: 'bot', label: `@${opts.botName ?? 'bot'}`, detail: `${opts.botName ?? 'This bot'} answers in this chat.` };
  }
  if (opts.kind === 'general') {
    return { kind: 'laya', label: 'Best fit', detail: 'The best bot for each message answers. Type @ and a name to choose one yourself.' };
  }
  if (opts.kind === 'team') {
    const t = opts.teamName ? `@${opts.teamName} team` : 'The team';
    return { kind: 'team', label: t, detail: `${t} answers. Type @ and a name to ask one member.` };
  }
  return { kind: 'team', label: 'This channel', detail: 'The bots in this channel answer. Type @ and a name to ask one.' };
}

/** "5m ago", "2h ago", "3d ago", else the date: short, for the recent-work cards. */
export function ago(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso.includes('T') || iso.endsWith('Z') ? iso : `${iso.replace(' ', 'T')}Z`);
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)}d ago`;
  return new Date(t).toISOString().slice(0, 10);
}

/**
 * Which bots a message goes to as written: the bot whose own chat it is, plus every bot @mentioned
 * by name (case-insensitive, whole word). Laya's pick isn't known until she answers, so it isn't here.
 */
export function recipientsOf(text: string, bots: Array<{ id: string; name: string }>, directId?: string): string[] {
  const out = new Set<string>();
  if (directId) out.add(directId);
  for (const b of bots) {
    const esc = b.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    if (esc && new RegExp(`(?:^|\\s)@${esc}(?![\\w-])`, 'i').test(text)) out.add(b.id);
  }
  return [...out];
}
