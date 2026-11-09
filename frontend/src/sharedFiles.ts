// Files a bot shares in chat (0.8.0 P4, app half): who sent them, where they sit, and repeats.
// Shared-file rows store the bot's name as sender_id, like its replies, with the id in meta.kwant_id
// (vega-080-share-screens). Rows saved before that, and live events from an older server, carry the
// bot's id instead, so a row is resolved by meta.kwant_id, then id or name, against the chat's bots.
// It never falls back to the chat title: in a space that title is the space ("@booktun.ing").
import { isSharedFileMessage, parseAttachments } from './attachments';
import { parseMetaObject } from './questions';

/** What a row shows when its bot can't be found (deleted, or the list hasn't loaded yet). */
export const UNKNOWN_BOT_LABEL = 'A bot';

/** How close two shares of one file must be to count as repeats. */
export const REPEAT_WINDOW_MS = 5 * 60 * 1000;

export interface BotRef {
  id: string;
  name: string;
}

interface Msg {
  id: string;
  sender_role: 'user' | 'kwant' | 'system';
  sender_id: string;
  meta?: string | null;
  created_at?: string | null;
}

// Ids are uuids today; anything long without spaces is treated as an id, never shown as a name.
const looksLikeId = (s: string) => s.length > 20 && !/\s/.test(s);

/** The bot behind a sender id (or a stored name): the bot itself, or null when it isn't known. */
export function botOf(senderId: string | undefined | null, bots: ReadonlyArray<BotRef>): BotRef | null {
  if (!senderId) return null;
  return bots.find((b) => b.id === senderId) ?? bots.find((b) => b.name === senderId) ?? null;
}

/** A bot message's name: the bot by id or name, else a short stored name, else the neutral label. */
export function botDisplayName(senderId: string | undefined | null, bots: ReadonlyArray<BotRef>): string {
  const bot = botOf(senderId, bots);
  if (bot) return bot.name;
  if (!senderId || looksLikeId(senderId)) return UNKNOWN_BOT_LABEL;
  return senderId;
}

export interface ShareInfo {
  /** The bot's id from meta, when the server sent it. */
  kwantId: string | null;
  /** True when kwnt shared the turn's last screenshot itself, because the reply claimed a picture. */
  auto: boolean;
}

/** What a shared-file row's meta says about who shared it and how. */
export function shareInfo(meta: string | null | undefined): ShareInfo {
  const o = parseMetaObject(meta) ?? {};
  const id = typeof o.kwant_id === 'string' && o.kwant_id ? o.kwant_id : null;
  return { kwantId: id, auto: o.auto === true };
}

/** Who a message is from, for matching: a share's meta.kwant_id when present, else its sender_id. */
const senderOf = (m: Msg) => (isBotFileShare(m) ? shareInfo(m.meta).kwantId : null) ?? m.sender_id;

/** True for a bot's shared-file row (meta kind "file" with at least one file). */
export function isBotFileShare(m: Msg): boolean {
  return m.sender_role === 'kwant' && isSharedFileMessage(m.meta) && parseAttachments(m.meta).length > 0;
}

/** Same bot, by id or name, so an id row and a name reply match. */
function sameBot(a: string, b: string, bots: ReadonlyArray<BotRef>): boolean {
  if (a === b) return true;
  const x = botOf(a, bots);
  const y = botOf(b, bots);
  return !!x && !!y && x.id === y.id;
}

export interface ArrangedFiles<M> {
  /** The messages in display order: each turn's shares moved right under their bot's reply. */
  messages: M[];
  /** Share rows that now sit under their bot's reply (drawn without a second header). */
  attached: Set<string>;
  /** Newest share id -> the older copies folded behind it, oldest first. */
  earlier: Map<string, M[]>;
  /**
   * Shares from the bot whose turn is still running, taken out of `messages`: the chat draws them
   * under that bot's live row, so they are already where its saved reply will put them (no jump).
   */
  live: M[];
}

const timeOf = (m: Msg) => {
  const t = m.created_at ? Date.parse(m.created_at) : NaN;
  return Number.isNaN(t) ? null : t;
};
const fileKey = (m: Msg) =>
  parseAttachments(m.meta)
    .map((a) => a.name)
    .sort()
    .join('\n');

/**
 * Within one turn (between two of the person's messages), a bot's shared-file rows move to just
 * after that bot's next reply in the turn. Rows with no later reply stay where they are. Then in each
 * run of consecutive shares by one bot, copies of the same file name within REPEAT_WINDOW_MS fold
 * into the newest, which takes the place of the last copy.
 */
export function arrangeSharedFiles<M extends Msg>(
  input: ReadonlyArray<M>,
  bots: ReadonlyArray<BotRef>,
  /** The bot (id or name) whose reply is streaming now, when its live row is on screen. */
  liveBot: string | null = null,
): ArrangedFiles<M> {
  const attached = new Set<string>();
  const placed: M[] = [];
  let i = 0;
  while (i < input.length) {
    // One turn: the person's message(s), then everything up to their next message.
    let j = i;
    while (j < input.length && input[j].sender_role === 'user') j += 1;
    while (j < input.length && input[j].sender_role !== 'user') j += 1;
    const turn = input.slice(i, j);
    const waiting: M[] = [];
    for (let k = 0; k < turn.length; k += 1) {
      const m = turn[k];
      if (isBotFileShare(m)) {
        const replyLater = turn.slice(k + 1).some((r) => r.sender_role === 'kwant' && !isBotFileShare(r) && sameBot(r.sender_id, senderOf(m), bots));
        if (replyLater) {
          waiting.push(m);
          continue;
        }
        placed.push(m);
        continue;
      }
      placed.push(m);
      if (m.sender_role === 'kwant') {
        const mine = waiting.filter((w) => sameBot(senderOf(w), m.sender_id, bots));
        for (const w of mine) {
          placed.push(w);
          attached.add(w.id);
          waiting.splice(waiting.indexOf(w), 1);
        }
      }
    }
    placed.push(...waiting);
    i = j;
  }
  // A share that already sat right under its bot's reply is attached too (e.g. the server saved it after).
  for (let k = 1; k < placed.length; k += 1) {
    const m = placed[k];
    if (!isBotFileShare(m) || attached.has(m.id)) continue;
    let p = k - 1;
    while (p >= 0 && isBotFileShare(placed[p]) && sameBot(senderOf(placed[p]), senderOf(m), bots)) p -= 1;
    const prev = placed[p];
    if (prev && prev.sender_role === 'kwant' && !isBotFileShare(prev) && sameBot(prev.sender_id, senderOf(m), bots)) attached.add(m.id);
  }

  // Repeats: inside a run of consecutive shares by one bot, group by file name.
  const earlier = new Map<string, M[]>();
  const out: M[] = [];
  let r = 0;
  while (r < placed.length) {
    if (!isBotFileShare(placed[r])) {
      out.push(placed[r]);
      r += 1;
      continue;
    }
    let e = r + 1;
    while (e < placed.length && isBotFileShare(placed[e]) && sameBot(senderOf(placed[e]), senderOf(placed[r]), bots)) e += 1;
    const run = placed.slice(r, e);
    const folded = new Set<string>();
    // Newest first: each copy joins the newest one of its name while it is within the window of the copy after it.
    for (let a = run.length - 1; a >= 0; a -= 1) {
      const head = run[a];
      if (folded.has(head.id)) continue;
      const key = fileKey(head);
      const older: M[] = [];
      let newer = head;
      for (let b = a - 1; b >= 0; b -= 1) {
        const cand = run[b];
        if (folded.has(cand.id) || fileKey(cand) !== key) continue;
        const tNew = timeOf(newer);
        const tOld = timeOf(cand);
        if (tNew !== null && tOld !== null && tNew - tOld > REPEAT_WINDOW_MS) break;
        older.unshift(cand);
        folded.add(cand.id);
        newer = cand;
      }
      if (older.length) earlier.set(head.id, older);
    }
    for (const m of run) if (!folded.has(m.id)) out.push(m);
    // A run sits under a reply when any of its rows does.
    if (run.some((m) => attached.has(m.id))) for (const m of run) if (!folded.has(m.id)) attached.add(m.id);
    r = e;
  }

  // The running turn: its loose shares from the live bot (after the person's last message) go to the live row.
  const live: M[] = [];
  if (liveBot) {
    let start = out.length;
    while (start > 0 && out[start - 1].sender_role !== 'user') start -= 1;
    for (let k = out.length - 1; k >= start; k -= 1) {
      const m = out[k];
      if (isBotFileShare(m) && !attached.has(m.id) && sameBot(senderOf(m), liveBot, bots)) {
        live.unshift(m);
        out.splice(k, 1);
      }
    }
  }
  return { messages: out, attached, earlier, live };
}
