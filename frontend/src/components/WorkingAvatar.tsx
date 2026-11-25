import React, { useId, useState } from 'react';
import { ChevronDown, ChevronRight, RotateCcw } from 'lucide-react';
import { useCreatureFor, renderCreature } from './ModernAvatar';
import { Steps } from './ActivityList';
import { isOpenStep, retriedSteps, shownStepCount, type LiveActivity } from '../activity';
import { sceneLine } from '../workingStrip';
import { Clock, useThrottledText } from './WorkingClock';
import { WatchButton } from './WatchLive';

// The bot at work (working-avatar/design.md §3): one inline SVG on a 36×18 cell grid, drawn at 3 px a cell (2 on a phone).
// Every motion is a CSS keyframe in stream.css (.kw-wa-*); the resting state is the reduced-motion still frame.
// The clock and the spoken sentence tick in WorkingClock.tsx: text, never motion.
const KEYS = [17, 19, 21, 23, 25, 27, 29, 31];
const PLUS: Array<[number, number]> = [[18, 1], [17, 2], [19, 2], [18, 3]];
const TIPS: Array<[number, number]> = [[18, 0], [16, 2], [20, 2], [18, 4]];
const LINES: Array<[number, number]> = [[8, 11], [10, 9], [12, 6]]; // [y, width], x = 19
// 36d (working-scene/design.md §2): a shell step shows a prompt on the screen: a static chevron, the typed command, one output line.
const CHEVRON: Array<[number, number]> = [[19, 8], [20, 9], [19, 10]];

export interface WorkingAvatarProps {
  name: string;
  /**
   * 16b: this turn is a double-check of another bot's work. The line says so in the same words as the
   * bot's status and the Live now chip (`auditTurnLine`), instead of the step.
   */
  audit?: { line: string; short?: string; onOpenTask?: () => void } | null;
  /** 36d: the running turn's steps: line 1's words, the Steps toggle and the list it opens. */
  live?: LiveActivity | null;
  /** The reply's words have started: line 1 says so and the words stream under the scene (`children`). */
  writing?: boolean;
  /** 2 px cells and the meta row under the scene (design §2, phone 390). */
  phone?: boolean;
  /** 36d: the live engine/switch note, the last line of the opened step list. */
  footer?: string | null;
  /** Something that must stay in reach while the bot works (the browsed-pages chip), under the scene row. */
  lead?: React.ReactNode;
  /** 36a: the bot doing the work, for a retried step's title. */
  who?: string;
  /** W1: this chat's browser is active and its panel is closed: "Watch" opens it (it was on the strip before 36d). */
  onWatch?: () => void;
  /** Unique per turn: the step list's id. */
  idBase?: string;
  /** The streaming words (B · writing): under the scene, behind a hairline. */
  children?: React.ReactNode;
}

const Dot = () => <span className="text-zinc-600" aria-hidden="true">·</span>;

export const WorkingAvatar: React.FC<WorkingAvatarProps> = ({ name, audit, live, writing = false, phone = false, footer, lead, who, onWatch, idBase, children }) => {
  // Same role and name as the row's avatar, so the bot at the laptop is the bot in the portrait.
  const c = useCreatureFor(name, name);
  const [open, setOpen] = useState(false);
  const autoId = useId();
  const listId = `${idBase ?? autoId}-steps`;
  const line = sceneLine(name, { live, writing }, audit?.line);
  const said = useThrottledText(`${line.text}.`);
  const steps = live?.steps ?? [];
  const work = shownStepCount(steps);
  const retried = retriedSteps(steps, true).size;
  const hasList = steps.length > 0 || !!footer;
  const cell = phone ? 2 : 3;
  const glyph = line.command && steps.some(isOpenStep);
  const stepsName = `Steps, ${work}${retried ? `, ${retried} retried` : ''}`;
  // 16b: on a phone an audit shows its short line ("Double-checking “Weekly report”"); the region says the full one.
  const sentence = line.audit ? (
    phone && audit?.short ? (
      audit.short
    ) : line.text.startsWith(name) ? (
      // Mira's 36d fix 3: the bot's name is bright in C as in A and B.
      <>
        <span className="font-medium text-zinc-50">{name}</span>
        {line.text.slice(name.length)}
      </>
    ) : (
      line.text
    )
  ) : (
    <>
      <span className="font-medium text-zinc-50">{name}</span>
      {line.host ? (
        <>
          {line.text.slice(name.length, line.text.length - line.host.length)}
          <span className="font-medium text-zinc-50">{line.host}</span>
        </>
      ) : (
        line.text.slice(name.length)
      )}
    </>
  );
  return (
    <div className="kw-wa-card" data-working-avatar="" data-audit={audit ? '' : undefined} data-phone={phone ? '' : undefined} data-writing={writing ? '' : undefined}>
      {/* One polite region per turn: one sentence per step, never the clock (design §5). */}
      <p className="sr-only" role="status" aria-live="polite" data-scene-said="">{said}</p>
      <div className="kw-wa">
        <div className="kw-wa-stage">
          <svg className="kw-wa-scene" viewBox="0 0 36 18" width={36 * cell} height={18 * cell} aria-hidden="true" focusable="false" data-glyph={glyph ? 'command' : 'lines'}>
            <rect x={1} y={17} width={35} height={1} fill="var(--kw-wa-ground)" />
            <g transform="translate(-5 -2)">
              <g className="kw-wa-bob">{renderCreature(c).filter((r) => r.key !== 'shadow')}</g>
            </g>
            <rect x={14} y={16} width={22} height={1} fill="var(--kw-wa-edge)" />
            <rect x={15} y={15} width={20} height={1} fill="var(--kw-wa-deck)" />
            {KEYS.map((x) => <rect key={x} x={x} y={15} width={1} height={1} fill="var(--kw-wa-key)" />)}
            <rect x={17} y={6} width={16} height={9} fill="var(--kw-wa-lid)" />
            <rect x={17} y={6} width={16} height={1} fill="var(--kw-wa-rim)" />
            <rect x={17} y={7} width={1} height={8} fill="var(--kw-wa-rim)" />
            <rect x={18} y={7} width={14} height={7} fill="var(--kw-wa-screen)" />
            {glyph ? (
              <>
                {CHEVRON.map(([x, y]) => <rect key={`c${x}-${y}`} x={x} y={y} width={1} height={1} fill="var(--kw-wa-type)" />)}
                <rect className="kw-wa-l kw-wa-l1" x={22} y={9} width={9} height={1} fill="var(--kw-wa-type)" />
                <rect className="kw-wa-l kw-wa-l3" x={19} y={12} width={7} height={1} fill="var(--kw-wa-type)" />
              </>
            ) : (
              LINES.map(([y, w], i) => (
                <rect key={y} className={`kw-wa-l kw-wa-l${i + 1}`} x={19} y={y} width={w} height={1} fill="var(--kw-wa-type)" />
              ))
            )}
            <g className="kw-wa-paw"><rect x={13} y={14} width={3} height={1} fill={c.body} data-paw="" /></g>
            <g className="kw-wa-spark">
              {TIPS.map(([x, y]) => <rect key={`t${x}-${y}`} className="kw-wa-s3" x={x} y={y} width={1} height={1} fill="var(--kw-wa-spark)" />)}
              {PLUS.map(([x, y]) => <rect key={`p${x}-${y}`} className="kw-wa-s2" x={x} y={y} width={1} height={1} fill="var(--kw-wa-spark)" />)}
              <rect className="kw-wa-s1" x={18} y={2} width={1} height={1} fill="var(--kw-wa-spark-core)" />
            </g>
          </svg>
        </div>
        <p className="kw-wa-now line-clamp-2 text-sm leading-5 text-zinc-300 [overflow-wrap:anywhere]" aria-hidden="true" data-scene-line="">
          {sentence}
        </p>
        <div className="kw-wa-meta flex min-w-0 items-center gap-x-2 whitespace-nowrap text-zinc-400" data-scene-meta="">
          <Clock live={live} />
          {hasList && (
            <>
              <Dot />
              <button
                type="button"
                onClick={() => setOpen((o) => !o)}
                aria-expanded={open}
                aria-controls={listId}
                aria-label={stepsName}
                data-scene-steps=""
                className={`${phone ? 'min-h-11' : 'min-h-7'} -mx-2 inline-flex items-center gap-2 rounded-md px-2 text-zinc-300 hover:bg-surface-overlay hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]`}
              >
                <span>Steps {work}</span>
                {retried > 0 && (
                  <>
                    <Dot />
                    <span className="inline-flex items-center gap-1">
                      <RotateCcw size={11} aria-hidden="true" />
                      {retried} retried
                    </span>
                  </>
                )}
                {open ? <ChevronDown size={13} aria-hidden="true" /> : <ChevronRight size={13} aria-hidden="true" />}
              </button>
            </>
          )}
          {audit?.onOpenTask && (
            <>
              <Dot />
              <button type="button" className="kw-wa-open" onClick={audit.onOpenTask} data-open-audited-task="">
                Open the task
              </button>
            </>
          )}
          {onWatch && (
            <>
              <Dot />
              <span className="-ml-2 inline-flex">
                <WatchButton name={name} line={!phone} phone={phone} onWatch={onWatch} />
              </span>
            </>
          )}
        </div>
      </div>
      {lead && <div className="kw-wa-lead" data-scene-lead="">{lead}</div>}
      {/* Rendered closed too (hidden), so aria-controls always names an element. No height animation (design §4). */}
      <div id={listId} hidden={!open} className="border-t border-hairline" data-scene-steps-list="">
        {steps.length > 0 && (
          <div className="max-h-[22rem] overflow-y-auto">
            <Steps steps={steps} live={live ?? undefined} id={`${listId}-rows`} who={who} />
          </div>
        )}
        {footer && (
          <p className={`${steps.length ? 'border-t border-zinc-700/40 ' : ''}px-3 py-2 text-xs leading-5 text-zinc-400 break-words`} data-failover-note="" data-scene-footer="">
            {footer}
          </p>
        )}
      </div>
      {children && (
        <div className="kw-wa-words border-t border-hairline" data-scene-words="">
          {children}
        </div>
      )}
    </div>
  );
};
