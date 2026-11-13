// 0.8.1 F3 (Mira, designs/f3-prepush-in-team.md): in a team chat or channel, one line per member
// bot that points at its newest pre-push card. The card and its buttons stay in the bot's 1:1
// (prepushAct posts to whichever chat is open), so a line only opens that chat at the card.
// Cards have no channel_id yet, so lines match by membership, like approvalsForChat.

import React, { useEffect, useRef, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { Led } from './parts';
import { NEEDS_YOU_TRIES, hhmm, openCount, phaseOf, requiredPassed, type Phase, type PrepushCard as Card } from '../../dev/prepush';

/** How many lines a chat shows before "+{n} more in the Inbox". */
export const POINTER_LINES = 3;
/** A card is a candidate for 24 h after it was created. */
export const POINTER_WINDOW_MS = 24 * 60 * 60 * 1000;
/** P2, P6 and P0 lines leave 10 min after the card's last update. */
export const POINTER_SETTLED_MS = 10 * 60 * 1000;
const SETTLED: ReadonlySet<Phase> = new Set(['P2', 'P6', 'P0', 'SS']);

export interface PointerPick {
  lines: Card[];
  /** Bots with a line that didn't fit. */
  more: number;
  /** When the next settled line leaves (ms), so the chat can drop it without a new event. */
  nextExpiry: number | null;
}

/**
 * Each member bot's newest card (by updated_at) created in the last 24 h; a settled one leaves 10 min
 * after its update. Needs-you lines first, then newest first; at most 3.
 */
export function pointerLines(cards: Card[], memberIds: readonly string[], now: number): PointerPick {
  const members = new Set(memberIds);
  const newest = new Map<string, Card>();
  for (const c of cards) {
    if (!members.has(c.kwant_id)) continue;
    if (c.created_at === null || now - c.created_at > POINTER_WINDOW_MS) continue;
    const cur = newest.get(c.kwant_id);
    if (!cur || (c.updated_at ?? 0) > (cur.updated_at ?? 0)) newest.set(c.kwant_id, c);
  }
  let nextExpiry: number | null = null;
  const live: Card[] = [];
  for (const c of newest.values()) {
    if (SETTLED.has(phaseOf(c))) {
      const leaves = (c.updated_at ?? 0) + POINTER_SETTLED_MS;
      if (leaves <= now) continue;
      nextExpiry = nextExpiry === null ? leaves : Math.min(nextExpiry, leaves);
    }
    // Lines also leave when the 24 h window closes.
    const closes = (c.created_at ?? 0) + POINTER_WINDOW_MS;
    nextExpiry = nextExpiry === null ? closes : Math.min(nextExpiry, closes);
    live.push(c);
  }
  live.sort((a, b) => {
    const pa = phaseOf(a) === 'P4' ? 0 : 1;
    const pb = phaseOf(b) === 'P4' ? 0 : 1;
    return pa - pb || (b.updated_at ?? 0) - (a.updated_at ?? 0);
  });
  return { lines: live.slice(0, POINTER_LINES), more: Math.max(0, live.length - POINTER_LINES), nextExpiry };
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** One piece of a line: plain text, the bot's name (bold) or the branch (code). */
export type Part = { t: 'text' | 'bot' | 'branch'; v: string };

/** Mira's copy for each phase, in parts so the name is bold and the branch is code. */
export function pointerParts(c: Card, bot: string): Part[] {
  const T = (v: string): Part => ({ t: 'text', v });
  const B: Part = { t: 'bot', v: bot };
  const Bs: Part = { t: 'bot', v: `${bot}'s` };
  const R: Part = { t: 'branch', v: c.branch };
  const n = openCount(c);
  switch (phaseOf(c)) {
    case 'P1':
      return [B, T(' is checking before pushing '), R];
    case 'RT':
      return [Bs, T(' push of '), R, T(' is being checked again')];
    case 'P3':
      return n > 0
        ? [B, T(` is fixing ${plural(n, 'finding')} before pushing `), R, T(` · Try ${c.tries} of ${NEEDS_YOU_TRIES}`)]
        : [B, T(' is fixing what blocked pushing '), R, T(` · Try ${c.tries} of ${NEEDS_YOU_TRIES}`)];
    case 'P7':
      return [T("A check couldn't run on "), Bs, T(' push of '), R];
    case 'P4':
      return n > 0
        ? [Bs, T(' push of '), R, T(` is waiting for you: ${plural(n, 'finding')} open`)]
        : [Bs, T(' push of '), R, T(" is waiting for you: the checks couldn't run")];
    case 'P2':
      return [T('Cleared: '), B, T(' can push '), R, T(` · ${plural(requiredPassed(c), 'check')} passed`)];
    case 'P6':
      return [T('You let '), Bs, T(' push of '), R, T(' go anyway')];
    case 'P0':
      return [T('Nothing to push: '), R, T(' has no new commits')];
    case 'SS':
      return [Bs, T(' push of '), R, T(' was replaced by a newer push')];
    case 'UN':
      return [B, T(" has a push check this version can't show · update the app to see where it is")];
  }
}

export const pointerText = (c: Card, bot: string): string => pointerParts(c, bot).map((p) => p.v).join('');

/** The button's name: the line, then where it goes. */
export const pointerLabel = (c: Card, bot: string): string => `${pointerText(c, bot)}. Open ${bot}'s chat`;

const LED: Record<Phase, 'run' | 'bad' | 'warn' | 'ok' | 'off'> = {
  P1: 'run',
  RT: 'run',
  P3: 'bad',
  P7: 'warn',
  P4: 'warn',
  P2: 'ok',
  P6: 'off',
  P0: 'off',
  SS: 'off',
  UN: 'off',
};

const ROW =
  'flex w-full max-w-[860px] min-h-11 sm:min-h-10 items-center gap-2.5 rounded-xl border px-3 py-2 text-left text-meta max-sm:text-[13px] max-sm:leading-[18px] outline-none transition-colors duration-[120ms] motion-reduce:transition-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-surface-main';
const PLAIN = 'border-hairline bg-surface-main text-zinc-300 hover:border-hairline-strong hover:bg-surface-raised';
const AMBER = 'border-amber-500/30 bg-amber-950/20 text-zinc-200 hover:bg-amber-950/30';

export interface PrepushPointersProps {
  cards: Card[];
  memberIds: readonly string[];
  names: Record<string, string>;
  onOpen: (c: Card) => void;
  onOpenInbox: () => void;
  /** The bot's avatar (the app passes its pixel avatar, 20 px). */
  avatar?: (kwantId: string, name: string) => React.ReactNode;
  /** For tests. */
  now?: () => number;
}

export const PrepushPointers: React.FC<PrepushPointersProps> = ({ cards, memberIds, names, onOpen, onOpenInbox, avatar, now = Date.now }) => {
  // Re-render when the next settled line is due to leave, without waiting for a new event.
  const [, setTick] = useState(0);
  const pick = pointerLines(cards, memberIds, now());
  useEffect(() => {
    if (pick.nextExpiry === null) return;
    const wait = Math.max(0, pick.nextExpiry - now()) + 50;
    const t = window.setTimeout(() => setTick((n) => n + 1), Math.min(wait, 2 ** 31 - 1));
    return () => window.clearTimeout(t);
  }, [pick.nextExpiry, now]);

  // Announce phase changes only: one short sentence, the line's text.
  const phases = useRef(new Map<string, Phase>());
  const [said, setSaid] = useState('');
  const sig = pick.lines.map((c) => `${c.id}:${phaseOf(c)}`).join('|');
  useEffect(() => {
    let say = '';
    for (const c of pick.lines) {
      const p = phaseOf(c);
      const before = phases.current.get(c.id);
      if (before !== undefined && before !== p) say = pointerText(c, names[c.kwant_id] || 'A bot');
      phases.current.set(c.id, p);
    }
    if (say) setSaid(say);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sig]);

  if (pick.lines.length === 0) return null;
  return (
    <section aria-label="Checks before pushing" data-prepush-pointers className="flex flex-col gap-1.5">
      <p aria-live="polite" className="sr-only" data-prepush-pointers-live>
        {said}
      </p>
      {pick.lines.map((c) => {
        const bot = names[c.kwant_id] || 'A bot';
        const phase = phaseOf(c);
        const p4 = phase === 'P4';
        const at = hhmm(c.updated_at);
        return (
          <button
            key={c.id}
            type="button"
            data-prepush-pointer={c.id}
            data-prepush-phase={phase}
            aria-label={pointerLabel(c, bot)}
            onClick={() => onOpen(c)}
            className={`${ROW} ${p4 ? AMBER : PLAIN}`}
          >
            <Led s={LED[phase]} className={`dv-led-still shrink-0 max-sm:hidden${phase === 'UN' ? ' opacity-60' : ''}`} />
            {avatar && <span className="inline-flex shrink-0">{avatar(c.kwant_id, bot)}</span>}
            <span
              className={`min-w-0 flex-1 [overflow-wrap:anywhere] max-sm:line-clamp-2${phase === 'P0' || phase === 'UN' ? ' text-zinc-500' : ''}`}
            >
              {p4 && <span className="kw-chip kw-chip-warn mr-1.5 !py-0 !px-2 align-[1px] !text-[11px] !leading-4">Needs you</span>}
              {pointerParts(c, bot).map((part, i) =>
                part.t === 'bot' ? (
                  <span key={i} className="font-semibold text-zinc-100">
                    {part.v}
                  </span>
                ) : part.t === 'branch' ? (
                  <code key={i} className="dv-sha">
                    {part.v}
                  </code>
                ) : (
                  <React.Fragment key={i}>{part.v}</React.Fragment>
                ),
              )}
            </span>
            {at && <span className="shrink-0 tabular-nums text-zinc-500 max-sm:hidden">{at}</span>}
            <span aria-hidden="true" className="inline-flex shrink-0 items-center gap-0.5 font-medium text-violet-300">
              Open
              <ChevronRight size={14} />
            </span>
          </button>
        );
      })}
      {pick.more > 0 && (
        <button
          type="button"
          data-prepush-pointers-more
          onClick={onOpenInbox}
          className="inline-flex min-h-11 max-w-[860px] items-center self-start rounded-lg px-3 text-meta font-medium text-violet-300 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-sky-400 sm:min-h-10"
        >
          +{pick.more} more in the Inbox
        </button>
      )}
    </section>
  );
};
