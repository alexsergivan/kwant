// The front desk's words: every server answer the guest can meet, as the page's own copy (design.md "For Iris" 5,
// with Lin's codes from impl.md). {host} and {bot} come from the view's host_name and bot.name; an empty host_name
// reads "the host". The code or the cause of a refusal is never shown.
import type { Fail } from '../api';
import { clock } from '../app';
import { fmt } from '../dom';
import { cap, pinCopy, Problem } from '../santa/pin';
import type { DeskIcon } from './look';
import type { DeskView, NoteKind } from './view';

export interface Names {
  /** "Marta", or "the host" (sentence case where it starts a sentence: use `Host`). */
  host: string;
  Host: string;
  /** "Marta's", "the host's". */
  hosts: string;
  bot: string;
  /** host_name was left out: the host's tile shows the kit bell icon instead of an initial. */
  anon: boolean;
}

export function names(v: Pick<DeskView, 'host_name' | 'bot'> | null | undefined): Names {
  const name = v?.host_name ?? '';
  const host = name || 'the host';
  const bot = v?.bot || 'the bot';
  return { host, Host: cap(host), hosts: `${host}'s`, bot: cap(bot), anon: !name };
}

/** A line with a bold lead: `[bold, rest]`, drawn as <b>bold</b> rest. */
export type Line = [string, string];

export function noteCopy(kind: NoteKind, n: Names): { line: Line; icon: DeskIcon } {
  if (kind === 'handoff') return { line: [`${n.Host} will reply here.`, "You can close this page; your chat stays, and it's here when you come back."], icon: 'bell' };
  if (kind === 'daily_cap')
    return { line: [`${n.bot} has answered all it can for today.`, `${n.Host} will read your message and reply here. ${n.bot} is back tomorrow morning.`], icon: 'moon' };
  return { line: [`${n.bot} can't answer right now.`, `${n.Host} has your message and will reply here.`], icon: 'moon' };
}

/** G6, 429 slow_down: the open time is the server's retry_at in this phone's own time. */
export const slowCopy = (retryAt: string): { lead: string; time: string; rest: string; hint: string } => {
  const time = clock(retryAt);
  return {
    lead: "That's a lot of messages for one hour.",
    time,
    rest: 'Your message is still here, ready to send.',
    hint: time ? `You can send it at ${time}` : 'You can send it soon',
  };
};

/** G8, 413 too_long, checked on the phone first from limits.max_chars. */
export const longCopy = (max: number): Line => ["That's a long one.", `Messages can be up to ${fmt(max)} characters. Try two.`];

/** G9: a message that didn't reach the box stays on this phone. */
export const offCopy: Line = ['No connection.', 'Your message is saved on this phone.'];

/** A refused join or rejoin. 409 guest_max (and pa1's own full) is the desk's; the rest is pa1's PIN flow. */
export function joinCopy(r: Fail, n: Names, joining: boolean): Problem {
  if (r.error === 'guest_max' || r.error === 'full') return { title: 'This guide is full right now.', sub: `Ask ${n.host} to make room.` };
  return pinCopy(r, n.host, joining);
}

/** A refused message other than 429 / 413 / 410: plain words, never the code. */
export function sayCopy(r: Fail): string {
  if (r.error === 'empty') return 'Write a message first.';
  return "That message didn't go through. Try again.";
}
