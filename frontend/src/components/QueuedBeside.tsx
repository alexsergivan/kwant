import { queuedText } from '../claudeVm';
import type { QueuedState } from './ChatView';

/**
 * P6 (#467): the person wrote again while the bot was still answering in this chat. The new turn
 * waits (`agent_busy`) and starts when the running reply ends. Under that reply, one calm line per
 * waiting turn says it was received and where it is in line: the same dot and words as the
 * "In line" bubble, no spinner, no warning tone.
 *
 * The reason sentence is said once: a row repeats it only when its reason, slots or named bot differ
 * from the row above ("#2 in line." alone otherwise). In a room, a turn waiting for a different bot
 * than the one replying leads with that bot's name, so "this bot" never points at the wrong one.
 * The list is one polite live region (aria-live only: a status role would replace its list role and
 * leave the rows without a list, #476 L5); its rows stay plain list items. A named row says whose
 * other replies it waits for (#476 L6).
 */
export function QueuedBeside({ items }: { items?: QueuedState[] }) {
  if (!items || items.length === 0) return null;
  return (
    <ul
      className="mt-2 flex flex-col gap-1 w-full"
      data-queued-beside=""
      aria-live="polite"
      aria-label="Waiting to start"
    >
      {items.map((q, i) => {
        const prev = items[i - 1];
        const sameAsAbove =
          !!prev && prev.reason === q.reason && prev.slots === q.slots && !!prev.named === !!q.named && (!q.named || prev.speaker === q.speaker);
        const place = q.position ? `#${q.position} in line.` : 'In line.';
        return (
          <li key={q.turnId ?? i} className="flex items-start gap-1.5 text-[11px] leading-snug text-zinc-400">
            <span className="mt-[5px] w-1.5 h-1.5 rounded-full bg-zinc-500 shrink-0" aria-hidden="true" />
            <span className="min-w-0">
              <strong className="font-medium text-zinc-200">{q.named ? `${q.speaker} · ${place}` : place}</strong>
              {!sameAsAbove && <> {queuedText(q.reason, q.slots, q.named ? q.speaker : undefined)}</>}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
