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
}

const MAX_CHIPS = 4;
const MAX_CHIP_LEN = 32;

const clean = (s: string) => s.replace(/\s+/g, ' ').trim();

/**
 * "Good at" chips: the bot's own "Good for" phrases when it has them, otherwise the parts of its
 * role ("Nutrition & Diet Advisor" → "Nutrition", "Diet Advisor"). At most four, short, no repeats.
 */
export function goodAt(bot: Pick<HomeBot, 'title' | 'covers'>): string[] {
  const own = (bot.covers ?? []).map(clean).filter(Boolean);
  const parts = own.length ? own : clean(bot.title || '').split(/\s*(?:&|\/|,|\+|\band\b)\s*/i).map(clean).filter(Boolean);
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
    return { kind: 'laya', label: 'Laya picks', detail: 'Laya picks the best bot to answer. Type @ and a name to choose one yourself.' };
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
