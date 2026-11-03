import React from 'react';
import { Lightbulb, ShieldCheck, Cpu } from 'lucide-react';
import type { HeartDot, HeartWord } from '../signature';
import { heartbeatCount, heartbeatText, heartbeatWord } from '../signature';
import { Sticker } from './Sticker';
import { CheckBurst } from './CheckBurst';

/**
 * "Verified · 3/3" (or "First verified") as Pixel's holo sticker, tilted −2° (S1) or −3° for the
 * first-time sticker (S3), pressing in once when it appears (the wrapper scales, so the tilt stays).
 */
export const VerifiedStamp: React.FC<{ text: string; label: string; first?: boolean; press?: boolean; burst?: number; onOpen?: () => void; className?: string }> = ({
  text,
  label,
  first = false,
  press = true,
  burst = 0,
  onOpen,
  className = '',
}) => {
  // Clickable: Pixel's Sticker puts it in its 44x44 .kw-sticker-hit button (spec §1).
  return (
    <span className={`relative inline-flex ${press ? 'kws-stamp' : ''} ${className}`} data-verified-stamp="">
      <Sticker
        tone="holo"
        tilt={first ? -3 : -2}
        icon={<ShieldCheck size={12} aria-hidden="true" />}
        title={onOpen ? 'Open receipt' : label}
        ariaLabel={onOpen ? (first ? `First verified task. ${label}` : label) : undefined}
        onClick={onOpen ? () => onOpen() : undefined}
      >
        {first ? 'First verified' : text}
      </Sticker>
      <CheckBurst n={burst} />
    </span>
  );
};

/** "Learned from N": the 0.6.8 lessons a task started with (spec §4). Information, so never holo. */
export const LessonsChip: React.FC<{ text: string; title: string }> = ({ text, title }) => (
  <span className="kws-chip kws-chip-learned kws-tip" data-tip={title} tabIndex={0} aria-label={`${text}. ${title}`} data-lessons-chip="">
    <Lightbulb size={12} aria-hidden="true" />
    {text}
  </span>
);

const WORD_TONE: Record<HeartWord, string> = {
  Steady: 'text-ok',
  'Back to ok': 'text-ok',
  Flapping: 'text-warn',
  Fixing: 'text-warn',
  Failing: 'text-bad',
  'Can’t check': 'text-muted',
};

const when = (at?: string): string | undefined => {
  if (!at) return undefined;
  const d = new Date(at);
  return Number.isNaN(d.getTime()) ? undefined : d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
};

/**
 * The goal's last checks as 6×6 squares, oldest left, newest outlined (spec §3), then "5/7 ok" and a word.
 * Phones show the last 5; with under 64 px free (or under 360 px wide) only the newest square and its
 * "1/5 ok". One picture for screen readers; nothing at all when there is no check history.
 */
export const GoalHeartbeat: React.FC<{ dots: HeartDot[]; className?: string }> = ({ dots, className = '' }) => {
  if (!dots.length) return null;
  const phone = dots.slice(-5);
  const word = heartbeatWord(dots);
  const square = (d: HeartDot, i: number, newest: boolean, extra = '') => (
    <span key={i} className={`kws-beat kws-beat-${d.tone} ${newest ? 'kws-beat-newest' : ''} ${extra}`} title={[d.label, when(d.at)].filter(Boolean).join(' · ')} data-dot={d.tone} />
  );
  const older = dots.length - phone.length;
  return (
    <span className={`kws-heartbeat ${className}`} data-heartbeat="">
      <span className="kws-heartbeat-full" role="img" aria-label={heartbeatText(dots)}>
        <span className="kws-beats" aria-hidden="true">
          {dots.map((d, i) => square(d, i, i === dots.length - 1, i < older ? 'kws-beat-desk' : ''))}
        </span>
        <span className="kws-beat-count kws-beat-count-desk" aria-hidden="true">{heartbeatCount(dots)}</span>
        {word && <span className={`kws-beat-word ${WORD_TONE[word]}`} aria-hidden="true">{word}</span>}
      </span>
      <span className="kws-heartbeat-tight" role="img" aria-label={heartbeatText(phone)}>
        <span className="kws-beats" aria-hidden="true">{square(phone[phone.length - 1], 0, true)}</span>
        <span className="kws-beat-count" aria-hidden="true">{heartbeatCount(phone)}</span>
      </span>
    </span>
  );
};

/** A 12×12 pixel baton (spec §4): the engine that took over carries it on. */
const PixelBaton: React.FC = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true" style={{ shapeRendering: 'crispEdges' }} className="shrink-0">
    <rect x="2" y="8" width="2" height="2" fill="#e8e4ff" />
    <rect x="4" y="6" width="2" height="2" fill="#e8e4ff" />
    <rect x="6" y="4" width="2" height="2" fill="#e8e4ff" />
    <rect x="8" y="2" width="2" height="2" fill="#fcd34d" />
    <rect x="9" y="1" width="2" height="2" fill="#fcd34d" />
  </svg>
);

/** The engine that answered (ranOn + account, spec §4); after a failover, "Claude → Codex" with a pixel baton and the reason on hover. */
export const EngineChip: React.FC<{ label: string; handoff?: { from: string; to: string; why: string } | null }> = ({ label, handoff }) =>
  handoff ? (
    <span className="kws-chip kws-chip-failover kws-tip min-w-0 max-w-full" data-tip={handoff.why} tabIndex={0} aria-label={`${handoff.from} to ${handoff.to}. ${handoff.why}`} data-engine-chip="" data-failover="">
      <PixelBaton />
      <span className="truncate" aria-hidden="true">{`${handoff.from} → ${handoff.to}`}</span>
    </span>
  ) : (
    <span className="kws-chip kws-chip-engine min-w-0 max-w-full" data-engine-chip="">
      <Cpu size={12} className="shrink-0" aria-hidden="true" />
      <span className="truncate">{label}</span>
    </span>
  );
