// Replies that ended without an answer: cut off by a restart (system note with
// `turn_interrupted` meta) or stopped by the stall watchdog (the bot's own "⚠️ [Name] The model
// sent nothing for N s" message). Both get a notice with Retry, which resends the person's message.
import type { ChatMessage } from './components/ChatView';
import { AttachmentRef, parseAttachments } from './attachments';
import { parseMetaObject } from './questions';

export interface Interruption {
  kind: 'restart' | 'stall';
  botName: string;
  /** Seconds of silence before the watchdog stopped the reply (stall only). */
  secs?: number;
}

// Written by routes.rs when the stall watchdog fires; there is no meta on that message.
const STALL_RE = /^⚠️ \[(.+?)\] The model sent nothing for (\d+) s, so this reply was stopped\./;

export function parseInterruption(msg: ChatMessage): Interruption | null {
  if (msg.sender_role === 'system') {
    const m = parseMetaObject(msg.meta);
    if (m?.kind !== 'turn_interrupted' || m.interrupted !== true) return null;
    const name = typeof m.kwant_name === 'string' && m.kwant_name.trim() ? m.kwant_name : 'The bot';
    return { kind: 'restart', botName: name };
  }
  if (msg.sender_role === 'kwant') {
    const s = STALL_RE.exec(msg.text.trim());
    if (s) return { kind: 'stall', botName: s[1], secs: Number(s[2]) };
  }
  return null;
}

export function interruptionText(i: Interruption): string {
  return i.kind === 'restart'
    ? `${i.botName}'s reply was cut off when the app restarted.`
    : `${i.botName} sent nothing for ${i.secs} s, so the reply was stopped.`;
}

export interface RetryTarget {
  text: string;
  attachments: AttachmentRef[];
}

/**
 * The message Retry would resend, keyed by notice id. Only the newest notice gets one, and only
 * while the person hasn't written since: after that it was already retried or moved past.
 */
export function retryTargets(messages: ChatMessage[]): Map<string, RetryTarget> {
  const out = new Map<string, RetryTarget>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.sender_role === 'user') return out;
    if (!parseInterruption(msg)) continue;
    for (let j = i - 1; j >= 0; j--) {
      const prev = messages[j];
      if (prev.sender_role !== 'user') continue;
      const attachments = parseAttachments(prev.meta);
      if (prev.text.trim() || attachments.length) out.set(msg.id, { text: prev.text, attachments });
      break;
    }
    return out;
  }
  return out;
}

export type RetryState = 'idle' | 'sending' | 'sent';

/**
 * One Retry click: sends only from `idle`, holds `sending` until the send settles, then `sent`,
 * or back to `idle` if it failed so the person can try again. Returns whether it sent.
 */
export async function runRetryOnce(
  lock: { current: RetryState },
  target: RetryTarget,
  send: (target: RetryTarget) => void | Promise<unknown>,
  setState: (s: RetryState) => void,
): Promise<boolean> {
  if (lock.current !== 'idle') return false;
  lock.current = 'sending';
  setState('sending');
  try {
    await send(target);
    lock.current = 'sent';
    setState('sent');
    return true;
  } catch {
    lock.current = 'idle';
    setState('idle');
    return false;
  }
}

/** kwnt's own notes (sender "system"): drawn as a notice line, never as a bot's reply. */
export const isSystemNotice = (msg: ChatMessage) => msg.sender_role === 'system';
