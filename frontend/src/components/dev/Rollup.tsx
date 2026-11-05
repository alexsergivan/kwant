import React from 'react';
import { Check, Sparkles, X } from 'lucide-react';
import type { RollupProps } from '../../dev/types';
import { clock } from '../../dev/logic';

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "3 changes landed on main. Nothing needs you." (design §4). */
export function rollupTitle(count: number, needsYou: number, branch: string): string {
  const landed = `${plural(count, 'change', 'changes')} landed on ${branch}.`;
  return `${landed} ${needsYou === 0 ? 'Nothing needs you.' : `${needsYou} ${needsYou === 1 ? 'needs' : 'need'} you.`}`;
}

/**
 * Areas that need the person come first (stable). Five or more areas: the first two, then a "+N more" tile,
 * so the grid stays one row; when more than two need the person, all of those show and only quiet ones fold.
 */
export function rollupTiles<T extends { area: string }>(areas: T[], needsYou: string[] = []): { shown: T[]; more: number } {
  const needs = (a: T) => needsYou.includes(a.area);
  const sorted = [...areas.filter(needs), ...areas.filter((a) => !needs(a))];
  if (sorted.length < 5) return { shown: sorted, more: 0 };
  const keep = Math.max(2, sorted.filter(needs).length);
  return { shown: sorted.slice(0, keep), more: sorted.length - keep };
}

/** The "Since you last looked" card at the top of What landed. */
export const Rollup: React.FC<RollupProps> = ({ rollup, last_merge, branch = 'main', needsYouAreas = [], onDraftNotes, onMarkSeen, drafting = false, marking = false }) => {
  const since = clock(rollup.since);
  const until = clock(rollup.until);
  const range = since && until ? `Since you last looked · ${since} → ${until}` : until ? `Everything so far · up to ${until}` : 'Since you last looked';

  if (rollup.count === 0) {
    const at = last_merge ? clock(last_merge.merged_at) : null;
    return (
      <section data-dev-rollup="empty" aria-label="Since you last looked" className="kw-card p-4">
        <p className="text-meta text-zinc-400">{range}</p>
        <p className="mt-1 text-body text-zinc-100">
          Nothing landed since you last looked.
          {last_merge && ` Last merge: #${last_merge.pr}${at ? `, ${at}` : ''}.`}
        </p>
      </section>
    );
  }

  const { shown, more } = rollupTiles(rollup.areas, needsYouAreas);
  const title = rollupTitle(rollup.count, rollup.needs_you, branch);
  return (
    <section data-dev-rollup={rollup.needs_you > 0 ? 'needs-you' : 'quiet'} aria-label="Since you last looked" className="kw-card p-4">
      <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-2">
        <div className="min-w-0">
          <p className="text-meta text-zinc-400">{range}</p>
          <h3 className="mt-0.5 text-title font-semibold text-zinc-50">{title}</h3>
        </div>
        {rollup.main_green === true && (
          <span className="kw-chip kw-chip-ok shrink-0">
            <Check size={12} aria-hidden="true" />
            {branch} is green
          </span>
        )}
        {rollup.main_green === false && (
          <span className="kw-chip kw-chip-bad shrink-0">
            <X size={12} aria-hidden="true" />
            {branch} is red
          </span>
        )}
      </div>

      <ul role="list" className="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-3">
        {shown.map((a) => {
          const warn = needsYouAreas.includes(a.area);
          return (
            <li
              key={a.area}
              data-area={a.area}
              data-needs-you={warn || undefined}
              className={`min-w-0 rounded-xl border bg-surface-main p-3 ${warn ? 'border-amber-300/50' : 'border-hairline'}`}
            >
              <div className="flex items-baseline justify-between gap-2">
                <span className="min-w-0 text-[13px] font-semibold text-zinc-100 [overflow-wrap:anywhere]">{a.area}</span>
                <span className="shrink-0 text-meta text-zinc-300" aria-label={plural(a.count, 'change', 'changes')}>
                  {a.count}
                </span>
              </div>
              <p className="mt-1 text-meta text-zinc-400 [overflow-wrap:anywhere]">{a.first_headline}</p>
              {warn && <p className="mt-1 text-2xs font-semibold text-warn">Needs you</p>}
            </li>
          );
        })}
        {more > 0 && (
          <li data-area-more className="flex items-center rounded-xl border border-hairline bg-surface-main p-3 text-meta text-zinc-300">
            +{more} more {more === 1 ? 'area' : 'areas'}
          </li>
        )}
      </ul>

      {/* Phones: both actions full width, one line each, Draft release notes first. The count is in the title. */}
      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center" data-dev-rollup-actions>
        <button type="button" onClick={onDraftNotes} disabled={drafting} aria-busy={drafting || undefined} className="kw-btn-primary min-h-[44px] sm:min-h-0 max-sm:w-full max-sm:justify-center whitespace-nowrap">
          <Sparkles size={14} aria-hidden="true" />
          {drafting ? 'Drafting…' : 'Draft release notes'}
        </button>
        <button type="button" onClick={onMarkSeen} disabled={marking} aria-busy={marking || undefined} className="kw-btn-ghost min-h-[44px] sm:min-h-0 max-sm:w-full max-sm:justify-center whitespace-nowrap">
          {marking ? 'Marking…' : 'Mark all seen'}
        </button>
        {/* The "Send me this every morning" link is 0.7.x (plan §1): hidden. */}
      </div>
    </section>
  );
};
