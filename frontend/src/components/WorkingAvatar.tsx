import React from 'react';
import { useCreatureFor, renderCreature } from './ModernAvatar';

// The bot at work (working-avatar/design.md §3): one inline SVG on a 36×18 cell grid, drawn at 3 px a cell.
// Every motion is a CSS keyframe in stream.css (.kw-wa-*); the resting state is the reduced-motion still frame.
const KEYS = [17, 19, 21, 23, 25, 27, 29, 31];
const PLUS: Array<[number, number]> = [[18, 1], [17, 2], [19, 2], [18, 3]];
const TIPS: Array<[number, number]> = [[18, 0], [16, 2], [20, 2], [18, 4]];
const LINES: Array<[number, number]> = [[8, 11], [10, 9], [12, 6]]; // [y, width], x = 19

export const WorkingAvatar: React.FC<{ name: string }> = ({ name }) => {
  // Same role and name as the row's avatar, so the bot at the laptop is the bot in the portrait.
  const c = useCreatureFor(name, name);
  const label = `${name} is working on the answer`;
  return (
    <div className="kw-wa" role="status" aria-label={label} data-working-avatar="">
      <svg className="kw-wa-scene" viewBox="0 0 36 18" width={108} height={54} aria-hidden="true" focusable="false">
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
        {LINES.map(([y, w], i) => (
          <rect key={y} className={`kw-wa-l kw-wa-l${i + 1}`} x={19} y={y} width={w} height={1} fill="var(--kw-wa-type)" />
        ))}
        <g className="kw-wa-paw"><rect x={13} y={14} width={3} height={1} fill={c.body} data-paw="" /></g>
        <g className="kw-wa-spark">
          {TIPS.map(([x, y]) => <rect key={`t${x}-${y}`} className="kw-wa-s3" x={x} y={y} width={1} height={1} fill="var(--kw-wa-spark)" />)}
          {PLUS.map(([x, y]) => <rect key={`p${x}-${y}`} className="kw-wa-s2" x={x} y={y} width={1} height={1} fill="var(--kw-wa-spark)" />)}
          <rect className="kw-wa-s1" x={18} y={2} width={1} height={1} fill="var(--kw-wa-spark-core)" />
        </g>
      </svg>
      <p className="kw-wa-text" aria-hidden="true">
        <span className="kw-wa-name">{name}</span> is working on it
        <span className="kw-wa-dots">
          <span className="kw-mood-dot">.</span><span className="kw-mood-dot kw-mood-dot-1">.</span><span className="kw-mood-dot kw-mood-dot-2">.</span>
        </span>
      </p>
      <span className="sr-only">{label}</span>
    </div>
  );
};
