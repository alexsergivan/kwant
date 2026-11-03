import React from 'react';
import { GraduationCap, ShieldCheck, ArrowRightLeft, Cpu } from 'lucide-react';
import type { HeartDot } from '../signature';
import { heartbeatText } from '../signature';
import { Sticker } from './Sticker';

const PIXEL_COLORS = ['#f9c6ff', '#c4d4ff', '#b9fbe6', '#fff1b8', '#ffc2d9', '#bcb7de'];

/** Pixel confetti over its (relative) parent. Hidden outright under reduced motion. */
export const PixelConfetti: React.FC<{ n: number }> = ({ n }) =>
  n > 0 ? (
    <span key={n} className="kws-confetti" aria-hidden="true" data-confetti="">
      {Array.from({ length: 14 }, (_, i) => (
        <span
          key={i}
          className="kws-confetti-px"
          style={{
            ['--kws-a' as string]: `${i * (360 / 14)}deg`,
            ['--kws-d' as string]: `${-22 - (i % 3) * 8}px`,
            ['--kws-c' as string]: PIXEL_COLORS[i % PIXEL_COLORS.length],
          }}
        />
      ))}
    </span>
  ) : null;

/** "Verified · 3/3" (or "First verified") as a holographic sticker that presses in once; a button when it can open the receipt. */
export const VerifiedStamp: React.FC<{ text: string; label: string; first?: boolean; press?: boolean; burst?: number; onOpen?: () => void; className?: string }> = ({
  text,
  label,
  first = false,
  press = true,
  burst = 0,
  onOpen,
  className = '',
}) => (
  <span className={`relative inline-flex ${press ? 'kws-stamp' : ''} ${className}`} data-verified-stamp="">
    <Sticker
      tone="holo"
      icon={<ShieldCheck size={12} aria-hidden="true" />}
      onClick={onOpen ? () => onOpen() : undefined}
      title={onOpen ? 'Open receipt' : label}
      ariaLabel={onOpen ? (first ? `First verified task. ${label}` : label) : undefined}
    >
      {first ? 'First verified' : text}
    </Sticker>
    <PixelConfetti n={burst} />
  </span>
);

/** "Learned from N earlier tasks": the 0.6.8 lessons a task started with. */
export const LessonsChip: React.FC<{ text: string; title: string }> = ({ text, title }) => (
  <span
    className="inline-flex items-center gap-1 rounded-full border border-zinc-600/60 bg-zinc-800/70 px-2 py-0.5 text-[11px] font-medium text-zinc-200"
    title={title}
    data-lessons-chip=""
  >
    <GraduationCap size={12} className="text-violet-300" aria-hidden="true" />
    {text}
  </span>
);

const DOT: Record<HeartDot['tone'], string> = {
  ok: 'bg-emerald-400',
  failing: 'bg-rose-400',
  fixing: 'bg-amber-400',
  unknown: 'bg-zinc-500',
};

/** The last few checks as tiny dots. The dots are a picture; the sentence is for screen readers and the hover. */
export const GoalHeartbeat: React.FC<{ dots: HeartDot[]; className?: string }> = ({ dots, className = '' }) => {
  if (!dots.length) return null;
  const text = heartbeatText(dots);
  return (
    <span className={`inline-flex items-center gap-1 ${className}`} role="img" aria-label={text} title={text} data-heartbeat="">
      {dots.map((d, i) => (
        <span key={i} className={`h-2 w-2 rounded-full ${DOT[d.tone]} ${i === dots.length - 1 ? 'ring-2 ring-zinc-500/60' : ''}`} data-dot={d.tone} />
      ))}
    </span>
  );
};

/** The engine that answered; after a failover it reads "Claude → Codex" with the reason on hover. */
export const EngineChip: React.FC<{ label: string; handoff?: { from: string; to: string; why: string } | null }> = ({ label, handoff }) => (
  <span
    className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-full border border-zinc-700/70 bg-zinc-900/70 px-2 py-0.5 text-xs font-medium text-zinc-300"
    title={handoff?.why}
    data-engine-chip=""
  >
    {handoff ? <ArrowRightLeft size={12} className="shrink-0 text-zinc-400" aria-hidden="true" /> : <Cpu size={12} className="shrink-0 text-zinc-400" aria-hidden="true" />}
    <span className="truncate">{handoff ? `${handoff.from} → ${handoff.to}` : label}</span>
    {handoff && <span className="sr-only">. {handoff.why}</span>}
  </span>
);
