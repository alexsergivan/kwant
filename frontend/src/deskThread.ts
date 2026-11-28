// desk-host-reply-composer (front desk fix #3): a front desk guest's chat thread on the answer bot.
// The host's words there go to the guest (pocketApi.deskReply), never to the bot. Pure helpers, tested in
// tests/deskHostReply.test.mjs; the hook and the composer are in components/DeskReplyComposer.tsx.
import { timeText } from './clock';

/** `crate::drivers::POCKET_GUEST_SOURCE`: the thread's `created_by`, and each guest message's `meta.source`. */
export const DESK_GUEST_SOURCE = 'pocket-guest';
/** `outsider_turns::HANDOFF_MARK`: the bot's line that hands the guest to the host. */
export const HANDOFF_MARK = '[[person]]';
/** `desk::REQUEST_PREFIX`: a hand-off that asks the host's permission (the server strips it from the Inbox row). */
const REQUEST_PREFIX = 'request:';

/** Who the composer writes to. `code`/`guestId` are null while no guest message carries them yet. */
export interface DeskGuest {
  code: string | null;
  guestId: string | null;
  guestName: string;
}

type MetaMessage = { meta?: string | null };

function metaOf(m: MetaMessage): Record<string, unknown> | null {
  if (!m.meta) return null;
  try {
    const v = JSON.parse(m.meta) as unknown;
    return v && typeof v === 'object' ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);

/**
 * The open thread's front desk guest, or null for any other chat (Telegram customers keep today's composer).
 * A desk thread is `created_by === "pocket-guest"`, else (an older list) one whose guest messages say so.
 * The newest guest message gives the code, the guest and the name; the thread's title is the name until then.
 */
export function deskGuestOf(createdBy: string | null | undefined, messages: readonly MetaMessage[], title?: string | null): DeskGuest | null {
  let found: Record<string, unknown> | null = null;
  for (let i = messages.length - 1; i >= 0; i--) {
    const meta = metaOf(messages[i]);
    if (meta?.source === DESK_GUEST_SOURCE && str(meta.pocket_code) && str(meta.guest_id)) {
      found = meta;
      break;
    }
  }
  if (createdBy !== DESK_GUEST_SOURCE && !found) return null;
  return {
    code: found ? str(found.pocket_code) : null,
    guestId: found ? str(found.guest_id) : null,
    guestName: (found && str(found.from_customer)) || str(title) || 'the guest',
  };
}

/**
 * A bot reply's `[[person]] …` line, hidden from the bubble and shown as the hand-off chip with the same reason
 * the desk card and the Inbox row show ("Request:" dropped). Null when the reply has no such line.
 */
export function deskHandoff(text: string | null | undefined, bot: string): { text: string; reason: string } | null {
  if (!text || !text.includes(HANDOFF_MARK)) return null;
  const lines = text.split('\n');
  const at = lines.findIndex((l) => l.trim().startsWith(HANDOFF_MARK));
  if (at < 0) return null;
  let reason = lines[at].trim().slice(HANDOFF_MARK.length).trim().replace(/^[:\-—]+/, '').trim();
  if (reason.toLowerCase().startsWith(REQUEST_PREFIX)) reason = reason.slice(REQUEST_PREFIX.length).trim();
  lines.splice(at, 1);
  return { text: lines.join('\n').trim(), reason: reason || `${bot} asked you to reply` };
}

/** The chip's words: "Needs you · Late check-out: 1 PM, Sun Nov 29". */
export const handoffChipText = (reason: string): string => `Needs you · ${reason}`;

/** A line the host sent the guest. `asked_at` places it among the thread's messages (settledPlace.ts). */
export interface DeskSentLine {
  id: string;
  text: string;
  at: string;
  asked_at: string;
  /** A local echo while the call is out. */
  sending?: boolean;
}

/** A thread message the host sent the guest: the server saves each reply and decline line with `to_guest` (desk_turns.rs after_act). */
export function deskSentTo(m: MetaMessage & { sender_role?: string }): string | null {
  if (m.sender_role !== undefined && m.sender_role !== 'user') return null;
  return str(metaOf(m)?.to_guest);
}

/** The pocket has ended or closed (its card's own rule, PocketDesk.tsx). */
export const pocketEnded = (pocket: { state?: string; ended_at?: string | null } | null | undefined): boolean =>
  !!pocket && (pocket.state === 'ended' || pocket.state === 'closed' || !!pocket.ended_at);

/** An API error that means nobody is there to read a reply: the pocket ended or went, or the guest left. */
export function goneError(e: unknown): boolean {
  const x = e as { status?: number; code?: string | null } | null;
  if (!x || typeof x !== 'object') return false;
  if (x.status === 404 || x.status === 410) return true;
  return x.code === 'ended' || x.code === 'not_found' || x.code === 'no_such_player';
}

/** The sent line's header: "You → Tom · 4:31 PM", in the chat's own clock format (clock.ts timeText). */
export function sentLabel(guestName: string, at: string, locale?: string): string {
  const time = timeText(at, { locale }) ?? '';
  return time ? `You → ${guestName} · ${time}` : `You → ${guestName}`;
}

/** The plain line above the composer. */
export const deskReplyNote = (guestName: string, bot: string): string => `${guestName} reads what you send here. ${bot} doesn’t answer it.`;
