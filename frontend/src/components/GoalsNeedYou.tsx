import React, { useEffect, useRef, useState } from 'react';
import { Target } from 'lucide-react';
import { Goal, goalsApi, goalsNeedingYou, heldFixTask, needsYouText, readGoalNotNow } from '../goals';
import { onMission } from '../missions';

/** Why it waits, after the chip that already says Needs you: "A fix is ready for your OK". Null when there's no reason. */
export function goalNeedsYouReason(g: Goal): string | null {
  const t = needsYouText(g.status_reason || (heldFixTask(g) ? 'fix_ready' : null)).replace(/^Needs you:?\s*/, '');
  return t ? t.charAt(0).toUpperCase() + t.slice(1) : null;
}

/**
 * The Inbox's "Needs you" rows for goals waiting on the person (33x, Mira's UX pass at 0.9.40 §3). Built like
 * SpendStopNeedsYou: reads the goals on mount and on `refreshKey` (and live on goal_updated), keeps the ones the
 * board's strip draws as Needs you, and renders nothing otherwise: no loading flash, no error line (the board
 * owns those). The section carries `data-needs-you`, so "You're all caught up" hides while it shows.
 */
export const GoalsNeedYou: React.FC<{
  onOpen: (id: string) => void;
  refreshKey?: unknown;
  now?: number;
  api?: Pick<typeof goalsApi, 'list'>;
}> = ({ onOpen, refreshKey, now, api = goalsApi }) => {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [seq, setSeq] = useState(0);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  useEffect(() => {
    let live = true;
    api.list().then(
      (list) => live && alive.current && setGoals(goalsNeedingYou(list, now ?? Date.now(), readGoalNotNow())),
      // A server without Goals, or a failed read: leave the rows as they were (none at first).
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api, refreshKey, seq, now]);

  // A check run updates a goal several times: one read per burst.
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | null = null;
    const off = onMission('goal_updated', () => {
      if (t) clearTimeout(t);
      t = setTimeout(() => setSeq((n) => n + 1), 250);
    });
    return () => {
      off();
      if (t) clearTimeout(t);
    };
  }, []);

  if (goals.length === 0) return null;
  return (
    <section aria-labelledby="needs-goals-h" data-needs-goals data-needs-you>
      <h3 id="needs-goals-h" className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">
        <Target size={12} aria-hidden="true" />
        Needs you
      </h3>
      <ul role="list" className="space-y-2">
        {goals.map((g) => {
          const why = goalNeedsYouReason(g);
          return (
          <li key={g.id}>
            <button
              type="button"
              data-needs-goal={g.id}
              onClick={() => onOpen(g.id)}
              className="flex min-h-11 w-full items-center gap-2.5 rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 text-left text-sm text-zinc-200 outline-none hover:bg-amber-950/30 focus-visible:ring-2 focus-visible:ring-sky-400 sm:px-4"
            >
              <span className="kw-chip kw-chip-warn shrink-0">Needs you</span>
              <span className="min-w-0 flex-1">
                <span className="block [overflow-wrap:anywhere]">
                  <span className="sr-only">Goal: </span>
                  {g.title}
                </span>
                {why && (
                  <span className="mt-0.5 block text-xs text-amber-200/70 [overflow-wrap:anywhere]" data-needs-goal-why>
                    {why}
                  </span>
                )}
              </span>
            </button>
          </li>
          );
        })}
      </ul>
    </section>
  );
};
