import { queuedText } from '../claudeVm';
import type { QueuedState } from './ChatView';

/**
 * P6 (#467): the person wrote again while the bot was still answering in this chat. The new turn
 * waits (`agent_busy`) and starts when the running reply ends. Under that reply, one calm line per
 * waiting turn says it was received and where it is in line: the same dot and words as the
 * "In line" bubble, no spinner, no warning tone.
 */
export function QueuedBeside({ items }: { items?: QueuedState[] }) {
  if (!items || items.length === 0) return null;
  return (
    <ul className="mt-2 flex flex-col gap-1 w-full" data-queued-beside="" aria-label="Waiting to start">
      {items.map((q, i) => (
        <li key={q.turnId ?? i} role="status" className="flex items-start gap-1.5 text-[11px] leading-snug text-zinc-400">
          <span className="mt-[5px] w-1.5 h-1.5 rounded-full bg-zinc-500 shrink-0" aria-hidden="true" />
          <span className="min-w-0">
            <strong className="font-medium text-zinc-200">{q.position ? `#${q.position} in line.` : 'In line.'}</strong> {queuedText(q.reason, q.slots)}
          </span>
        </li>
      ))}
    </ul>
  );
}
