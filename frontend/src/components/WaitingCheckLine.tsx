import React, { useEffect, useState } from 'react';
import { AlertTriangle, Hourglass } from 'lucide-react';
import { missionsApi, type Check, type Task } from '../missions';
import { cardWaits, detailLineText, isWaiting, nextRefresh, shortLabel, waitLine, waitLineText, waitingFallback, type WaitLine } from '../waitingChecks';

/**
 * One waiting check as a line: "Waiting on <label> since 09:12 · next look 09:27", and under it
 * the last line the script printed. Spans only, so it can sit inside the card's button.
 * `compact` (the card) shortens the label; the detail shows it whole and adds the deadline.
 */
export const WaitingCheckLine: React.FC<{ line: WaitLine; compact?: boolean }> = ({ line, compact = false }) => {
  const label = compact ? shortLabel(line.label) : line.label;
  const tone = line.timedOut ? 'text-amber-200' : 'text-sky-200';
  // The detail's stopped row already has the amber hand: no second warning icon there.
  const Icon = line.timedOut ? (compact ? AlertTriangle : null) : Hourglass;
  const indent = Icon ? 'pl-4' : '';
  // The detail row prints the label just above, so the detail line leaves it out.
  const detailText = compact ? null : detailLineText(line);
  return (
    <span className={`block text-[11px] ${tone}`} data-waiting-line={line.timedOut ? 'timed-out' : 'waiting'}>
      <span className="flex items-start gap-1 min-w-0">
        {Icon && <Icon size={11} className="mt-0.5 shrink-0" aria-hidden="true" />}
        {detailText !== null ? (
          <span className="min-w-0 break-words" data-waiting-detail>
            <span className="sr-only">{detailText.replace(' · ', ', ')}</span>
            <span aria-hidden="true">{detailText}</span>
          </span>
        ) : (
          <span className="min-w-0 break-words">
            <span className="sr-only">{waitLineText(line)}</span>
            <span aria-hidden="true">
              {!line.timedOut && 'Waiting on '}
              <span className="font-mono break-all text-zinc-200" title={line.label}>
                {label}
              </span>
              {line.rest ? ` ${line.rest}` : ''}
            </span>
          </span>
        )}
      </span>
      {line.last && (
        <span className={`mt-0.5 flex min-w-0 ${indent} text-zinc-400`} data-waiting-last>
          <span className="sr-only">It last printed: </span>
          <span className={`min-w-0 font-mono ${compact ? 'truncate' : 'break-all whitespace-pre-wrap'}`} title={line.last}>
            {line.last}
          </span>
        </span>
      )}
      {!compact && line.deadline && (
        <span className={`mt-0.5 block ${indent} text-zinc-400`} data-waiting-deadline>
          {line.deadline}
        </span>
      )}
    </span>
  );
};

/**
 * A task card's waiting checks. The task list carries only the count (`checks_summary.waiting`),
 * so a card that has some looks its checks up once, again when the task changes, and again a
 * minute after the earliest next look. Until then it says "Waiting on an outside service".
 */
export function useWaitingChecks(t: Task): Check[] | null {
  const waits = cardWaits(t);
  const [checks, setChecks] = useState<Check[] | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!waits) {
      setChecks(null);
      return;
    }
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    missionsApi.task(t.id).then(
      (d) => {
        if (!live) return;
        const list = d.checks.filter(isWaiting);
        setChecks(list);
        const ms = nextRefresh(list);
        if (ms !== null) timer = setTimeout(() => setTick((n) => n + 1), ms);
      },
      () => live && setChecks(null),
    );
    return () => {
      live = false;
      if (timer) clearTimeout(timer);
    };
  }, [waits, t.id, t.updated_at, t.checks_summary.waiting, tick]);
  return waits ? checks : null;
}

/** The card's lines: one per waiting check (at most two, then "+N more"), or the fallback. */
export const CardWaitingChecks: React.FC<{ task: Task; now?: number }> = ({ task, now = Date.now() }) => {
  const checks = useWaitingChecks(task);
  if (!cardWaits(task)) return null;
  const lines = (checks ?? []).flatMap((c) => {
    const l = waitLine(c, now);
    return l ? [{ id: c.id, l }] : [];
  });
  if (!lines.length) {
    return (
      <span className="flex items-start gap-1 text-[11px] text-sky-200" data-waiting-line="summary">
        <Hourglass size={11} className="mt-0.5 shrink-0" aria-hidden="true" />
        {waitingFallback(task.checks_summary.waiting ?? 1)}
      </span>
    );
  }
  return (
    <span className="block space-y-1" data-waiting-checks>
      {lines.slice(0, 2).map(({ id, l }) => (
        <WaitingCheckLine key={id} line={l} compact />
      ))}
      {lines.length > 2 && <span className="block pl-4 text-[11px] text-zinc-400">+{lines.length - 2} more waiting</span>}
    </span>
  );
};
