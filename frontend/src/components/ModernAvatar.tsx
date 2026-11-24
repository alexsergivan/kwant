import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { HardHat, useDevModeOn } from './DevModeSwitch';
import { isKwantWorking, lastRunOf, liveRunOf, subscribeKwant } from '../kwantActivity';
import { badgeUsageFor, modelBadgeFor, modelBadgeSize, type ModelBadge } from '../engines';
import { badgeBotOf, badgeEngines, badgeProfiles, subscribeBadgeData } from '../modelBadgeData';
import { EngineMark } from './EngineMark';
import { AvatarMoodMark, useKwantMood, useKwantNotices } from './AvatarMood';
import { moodLabel, restingUntilOf, type Mood } from '../kwantMood';
import { STATUS_DOT, STATUS_PRIORITY, statusText, type BotStatus } from '../botStatus';
import { avatarStyleFor, botLooksRev, subscribeBotLooks } from '../botLooks';

interface ModernAvatarProps {
  /** The bot's avatar style, or a title or name hint; a known bot's saved style wins over a hint. */
  role?: string;
  name?: string;
  size?: number; // pixel diameter
  /** Draws the status dot; a bot with a running turn shows Working even if this lags behind. */
  status?: BotStatus;
  className?: string;
  /** The kwant this avatar shows; it animates while that kwant is working on a turn. */
  kwantId?: string;
  /** The AI model badge (bottom-left) on avatars of 24 px and up; off for reply avatars in the chat. */
  modelBadge?: boolean;
  /** Eyes shut and still: the empty chat while nothing can run yet (first-screen E). */
  asleep?: boolean;
  /**
   * A mood set by the screen instead of the bot's own turns, for a bot that isn't created yet or a
   * check that isn't a turn (House while Connect Home Assistant tests). `working` shows the ring.
   */
  mood?: Mood;
  /** Words for that set mood ("House is checking Home Assistant"); the avatar is then an image with this name. */
  moodText?: string;
}

const noopUnsubscribe = () => undefined;

/** Whether a kwant is mid-turn; only this kwant's turns re-render the caller. */
export const useKwantWorking = (kwantId: string | undefined): boolean => {
  const subscribe = useCallback(
    (onChange: () => void) => (kwantId ? subscribeKwant(kwantId, onChange) : noopUnsubscribe),
    [kwantId],
  );
  return useSyncExternalStore(subscribe, () => isKwantWorking(kwantId), () => false);
};

/** The bot's model badge: its pinned engine, Laya's pick for the running reply, or its last reply's. */
export const useModelBadge = (kwantId: string | undefined): ModelBadge | null => {
  const subscribe = useCallback(
    (onChange: () => void) => {
      if (!kwantId) return noopUnsubscribe;
      const a = subscribeKwant(kwantId, onChange);
      const b = subscribeBadgeData(onChange);
      return () => {
        a();
        b();
      };
    },
    [kwantId],
  );
  const working = useSyncExternalStore(subscribe, () => isKwantWorking(kwantId), () => false);
  const live = useSyncExternalStore(subscribe, () => liveRunOf(kwantId), () => undefined);
  const last = useSyncExternalStore(subscribe, () => lastRunOf(kwantId), () => undefined);
  const bot = useSyncExternalStore(subscribe, () => badgeBotOf(kwantId), () => undefined);
  const engines = useSyncExternalStore(subscribe, badgeEngines, () => null);
  return useMemo(
    () => (bot ? modelBadgeFor(last ? { ...bot, last_ran_on: last } : bot, working ? live : null, engines) : null),
    [bot, last, working, live, engines],
  );
};

/** The hover's usage line for the account the bot's next turn would use, re-read when accounts or engines change. */
const useBadgeUsage = (badge: ModelBadge | null) => {
  const engines = useSyncExternalStore(subscribeBadgeData, badgeEngines, () => null);
  const profiles = useSyncExternalStore(subscribeBadgeData, badgeProfiles, () => null);
  const mark = badge && !badge.cantRun ? badge.mark : null;
  return useMemo(() => badgeUsageFor(mark, engines, profiles), [mark, engines, profiles]);
};

const prefersReducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

const BadgeFace: React.FC<{ badge: ModelBadge; glyph: number; className?: string }> = ({ badge, glyph, className = '' }) => (
  <span
    className={`kw-mb-layer ${badge.cantRun ? 'text-zinc-600' : badge.mark === 'laya' ? 'text-info' : 'text-zinc-100'} ${className}`}
    style={badge.resting ? { opacity: 0.6 } : undefined}
  >
    <EngineMark mark={badge.mark} size={glyph} />
  </span>
);

/** Bottom-left, the mirror of the status dot. A changed mark cross-fades in 150 ms (instant under reduced motion). */
export const AvatarModelBadge: React.FC<{ kwantId: string; size: number }> = ({ kwantId, size }) => {
  const badge = useModelBadge(kwantId);
  const usage = useBadgeUsage(badge);
  const d = modelBadgeSize(size);
  const [leaving, setLeaving] = useState<ModelBadge | null>(null);
  const shownRef = useRef<ModelBadge | null>(badge);
  const key = badge?.key;
  useEffect(() => {
    const before = shownRef.current;
    shownRef.current = badge;
    if (!before || !badge || before.key === badge.key || prefersReducedMotion()) {
      setLeaving(null);
      return;
    }
    setLeaving(before);
    const t = window.setTimeout(() => setLeaving(null), 150);
    return () => window.clearTimeout(t);
    // Only a new mark fades; a new label on the same mark doesn't.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  if (!badge || !d) return null;
  const glyph = Math.round(d * (badge.mark === 'custom' ? 0.7 : 0.64));
  const tip = usage ? `${badge.label}\n${usage.text}` : badge.label;
  return (
    <span
      className={`kw-model-badge ring-2 ring-zinc-950 ${badge.cantRun ? 'bg-zinc-950' : 'bg-zinc-900'}`}
      style={{ width: d, height: d }}
      role="img"
      aria-label={tip.replace('\n', '. ')}
      title={tip}
      data-model-badge={badge.mark}
      data-badge-resting={badge.resting || undefined}
      data-badge-cant-run={badge.cantRun ?? undefined}
    >
      {leaving && <BadgeFace key={`out:${leaving.key}`} badge={leaving} glyph={glyph} className="kw-mb-out" />}
      <BadgeFace key={badge.key} badge={badge} glyph={glyph} className={leaving ? 'kw-mb-in' : ''} />
      {usage?.low && (
        <span
          className="absolute -top-px -right-px rounded-full bg-amber-400 ring-1 ring-zinc-950"
          style={{ width: Math.max(4, Math.round(d * 0.36)), height: Math.max(4, Math.round(d * 0.36)) }}
          aria-hidden="true"
          data-badge-low
        />
      )}
      {badge.resting && badge.auto && (
        <span className="kw-mb-spark text-info" style={{ fontSize: Math.max(8, Math.round(d * 0.55)) }} aria-hidden="true" data-badge-spark>
          ✦
        </span>
      )}
    </span>
  );
};

type Accessory =
  | 'antenna' | 'spikes' | 'ears' | 'horns' | 'sprout' | 'crown' | 'visor'
  | 'beret' | 'glasses' | 'goggles' | 'tie' | 'quill' | 'bulb'
  | 'mortarboard' | 'star' | 'bowtie' | 'cap' | 'sunhat' | 'toque' | 'headband'
  | 'partyhat' | 'hardhat' | 'flower' | 'house' | 'host' | 'knithat' | 'camera' | 'none';

export interface Creature {
  body: string;
  shade: string;
  light: string;
  accent: string;
  accessory: Accessory;
}

const FRAME = '#4a3f8a';
const PANEL = '#15112e';
const SCAN = '#0d0a1f';
const INK = '#1b1033';
const BLUSH = '#ff6b81';

const PALETTES: Record<string, Omit<Creature, 'accessory'>> = {
  yellow: { body: '#fdc934', shade: '#d98a1f', light: '#fff0a0', accent: '#5ce1ff' },
  green: { body: '#3ccf6e', shade: '#1f8f4c', light: '#a8f5bf', accent: '#c6ff6b' },
  pink: { body: '#ff7eb6', shade: '#c9487f', light: '#ffd0e4', accent: '#ffe066' },
  cyan: { body: '#5ce1ff', shade: '#2a92c9', light: '#d2f7ff', accent: '#ff6b81' },
  violet: { body: '#a98bff', shade: '#6d4fd6', light: '#e1d6ff', accent: '#ffd23f' },
  orange: { body: '#ff9f43', shade: '#c4621c', light: '#ffd9a8', accent: '#5ce1ff' },
  blue: { body: '#6d9bff', shade: '#3c5fcf', light: '#cddcff', accent: '#ff7eb6' },
  coral: { body: '#ff8a7a', shade: '#cf5446', light: '#ffd6cf', accent: '#6d4fd6' },
  teal: { body: '#2fd4b5', shade: '#17917c', light: '#b5f5e8', accent: '#ffd23f' },
  red: { body: '#ff6b6b', shade: '#c43c4c', light: '#ffc9c9', accent: '#5ce1ff' },
  gold: { body: '#f7c948', shade: '#c28a14', light: '#fff1b8', accent: '#e5484d' },
  lilac: { body: '#d49cff', shade: '#9a5fd1', light: '#f1ddff', accent: '#f5f2ff' },
  mint: { body: '#9be564', shade: '#5e9e33', light: '#e2ffc7', accent: '#ffe066' },
  sky: { body: '#7cc8ff', shade: '#3f8fd1', light: '#d6efff', accent: '#4b44a6' },
  magenta: { body: '#f06bd8', shade: '#b13a9c', light: '#ffd3f5', accent: '#ffd23f' },
  slate: { body: '#a3aecf', shade: '#66719a', light: '#e3e8f7', accent: '#e5484d' },
  amber: { body: '#ffb347', shade: '#c9771a', light: '#ffe2b8', accent: '#3c5fcf' },
  ocean: { body: '#3fb6c9', shade: '#227b95', light: '#c4f1f7', accent: '#e5484d' },
  berry: { body: '#e0607e', shade: '#a8324f', light: '#ffc2d0', accent: '#ffffff' },
  lime: { body: '#c6f04a', shade: '#86b31f', light: '#f0ffc2', accent: '#ff4d6d' },
  peach: { body: '#ffb894', shade: '#d47a55', light: '#ffe5d6', accent: '#5ce1ff' },
  steel: { body: '#7fa3b8', shade: '#4d7086', light: '#cfe2ec', accent: '#ffc21a' },
  blossom: { body: '#f7b3d2', shade: '#c97aa0', light: '#ffe3f0', accent: '#ffffff' },
};

/** Avatar styles an agent can pick; `avatar_sprite` stores the id. */
export const AVATAR_STYLES: Array<{ id: string; label: string }> = [
  { id: 'architect', label: 'Architecture' },
  { id: 'tech_lead', label: 'Tech Lead' },
  { id: 'devops', label: 'DevOps' },
  { id: 'security', label: 'Security' },
  { id: 'fast', label: 'Speed' },
  { id: 'designer', label: 'Design' },
  { id: 'data', label: 'Data' },
  { id: 'qa', label: 'QA' },
  { id: 'product', label: 'Product' },
  { id: 'writer', label: 'Writing' },
  { id: 'research', label: 'Research' },
  { id: 'teacher', label: 'Teacher' },
  { id: 'marketing', label: 'Marketing' },
  { id: 'legal', label: 'Legal' },
  { id: 'coach', label: 'Coach' },
  { id: 'traveler', label: 'Travel' },
  { id: 'chef', label: 'Chef' },
  { id: 'fitness', label: 'Fitness' },
  { id: 'party', label: 'Party' },
  { id: 'builder', label: 'Builder' },
  { id: 'zen', label: 'Wellbeing' },
  { id: 'home', label: 'Home' },
  { id: 'quizmaster', label: 'Quizmaster' },
  { id: 'santa', label: 'Santa' },
  { id: 'snap', label: 'Photo wall' },
  { id: 'default', label: 'Droid' },
];

const ROLE_CREATURES: Record<string, [keyof typeof PALETTES, Accessory]> = {
  brand: ['yellow', 'antenna'],
  laya: ['violet', 'crown'],
  user: ['blue', 'ears'],
  architect: ['violet', 'horns'],
  tech_lead: ['cyan', 'visor'],
  devops: ['green', 'spikes'],
  security: ['orange', 'ears'],
  fast: ['pink', 'sprout'],
  designer: ['coral', 'beret'],
  data: ['teal', 'glasses'],
  qa: ['red', 'goggles'],
  product: ['gold', 'tie'],
  writer: ['lilac', 'quill'],
  research: ['mint', 'bulb'],
  teacher: ['sky', 'mortarboard'],
  marketing: ['magenta', 'star'],
  legal: ['slate', 'bowtie'],
  coach: ['amber', 'cap'],
  traveler: ['ocean', 'sunhat'],
  chef: ['berry', 'toque'],
  fitness: ['lime', 'headband'],
  party: ['peach', 'partyhat'],
  builder: ['steel', 'hardhat'],
  zen: ['blossom', 'flower'],
  home: ['gold', 'house'],
  /** Pocket apps (pa0a): a violet creature in a white top hat and a gold bow tie. */
  quizmaster: ['violet', 'host'],
  /**
   * Pocket apps (pa10a): a green creature in a red knit hat (bobble flops left) and a striped scarf. Picked style
   * only: no name rule, so agents already named with "santa" keep the creature they have.
   */
  santa: ['green', 'knithat'],
  /** Pocket apps (pa14): a pink creature holding a white instant-style camera low on its left. Picked style only. */
  snap: ['pink', 'camera'],
};

/** Style ids (picked or saved), as opposed to a title or name passed as a hint. */
const isStyleId = (role: string) => role === 'default' || Object.prototype.hasOwnProperty.call(ROLE_CREATURES, role);

const hash = (s: string) => {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
};

function pickCreature(role: string, name: string): Creature {
  // An explicitly chosen style wins over hints in the name
  const chosen = ROLE_CREATURES[role];
  if (chosen) return { ...PALETTES[chosen[0]], accessory: chosen[1] };
  const norm = `${role} ${name}`.toLowerCase();
  const rules: Array<[string[], keyof typeof PALETTES, Accessory]> = [
    [['laya'], 'violet', 'crown'],
    // Before 'user'/'player': "Quizmaster" or "Pub quiz" names get the top hat.
    [['quiz'], 'violet', 'host'],
    [['user', 'player', 'hero', 'you'], 'blue', 'ears'],
    [['architect', 'cyber_mage', 'mage'], 'violet', 'horns'],
    [['tech', 'ninja', 'pixel', 'lead'], 'cyan', 'visor'],
    [['devops', 'terminal', 'knight', 'shell', 'infra'], 'green', 'spikes'],
    [['security', 'shield', 'guard'], 'orange', 'ears'],
    [['fast', 'grok', 'zap', 'speed'], 'pink', 'sprout'],
    [['design', 'ux'], 'coral', 'beret'],
    [['data', 'analyst'], 'teal', 'glasses'],
    [['qa', 'test'], 'red', 'goggles'],
    [['product', 'manager'], 'gold', 'tie'],
    [['writer', 'docs'], 'lilac', 'quill'],
    [['research', 'scientist'], 'mint', 'bulb'],
    [['teacher', 'tutor'], 'sky', 'mortarboard'],
    [['marketing'], 'magenta', 'star'],
    [['legal', 'lawyer'], 'slate', 'bowtie'],
    [['travel', 'trip'], 'ocean', 'sunhat'],
    [['chef', 'cook', 'recipe'], 'berry', 'toque'],
    [['fitness', 'workout', 'trainer'], 'lime', 'headband'],
    [['party', 'event'], 'peach', 'partyhat'],
    [['diy', 'builder', 'handy'], 'steel', 'hardhat'],
    [['wellbeing', 'mindful', 'zen'], 'blossom', 'flower'],
    [['coach', 'mentor'], 'amber', 'cap'],
  ];
  for (const [keys, palette, accessory] of rules) {
    if (keys.some((k) => norm.includes(k))) return { ...PALETTES[palette], accessory };
  }
  // Unknown roles still get a stable, distinct creature per name
  // Fixed list so adding palettes never recolors existing agents
  const palettes: Array<keyof typeof PALETTES> = ['yellow', 'green', 'pink', 'cyan', 'violet', 'orange', 'blue'];
  const accessories: Accessory[] = ['antenna', 'spikes', 'ears', 'horns', 'sprout', 'antenna'];
  const h = hash(name || role);
  return {
    ...PALETTES[palettes[h % palettes.length]],
    accessory: accessories[Math.floor(h / 7) % accessories.length],
  };
}

const px = (x: number, y: number, w: number, h: number, fill: string, key?: string) => (
  <rect key={key ?? `${x}-${y}-${w}-${h}-${fill}`} x={x} y={y} width={w} height={h} fill={fill} />
);

// Stepped octagon frame, as unions of rects
const FRAME_FILL: Array<[number, number, number, number]> = [
  [6, 2, 14, 22],
  [4, 3, 18, 20],
  [3, 4, 20, 18],
  [2, 6, 22, 14],
];

// The top hat's side shade: sprite art, a step below the violet palette's light (#e1d6ff).
const HAT_SHADE = '#c9b8f7';

// Santa's knit hat and scarf: sprite art from pa10a/src/santa-sprite.ts (the avatar red palette's colours).
const KNIT_RED = '#e5484d';
const KNIT_SHADE = '#a8323a';
const KNIT_SHINE = '#ff8f8f';
const KNIT_CUFF_SHADE = '#d2cbec';

// Snap's camera: sprite art from pa14/src/snap-sprite.ts (`// @overlay camera`), held at x4-11 / y14-19 so it
// overlaps the body edge and never covers an eye or the smile.
const CAM_BODY = '#f5f2ff';
const CAM_SIDE = '#d2cbec';
const CAM_SHUTTER = '#e5484d';
const CAM_LENS_RIM = '#8f86b8';
const CAM_GLINT = '#5ce1ff';

function renderAccessory(c: Creature) {
  switch (c.accessory) {
    case 'knithat':
      return [
        px(9, 5, 9, 1, KNIT_RED), px(10, 4, 7, 1, KNIT_RED), px(11, 3, 5, 1, KNIT_RED), px(8, 2, 6, 1, KNIT_RED), px(6, 3, 5, 1, KNIT_RED),
        px(5, 4, 2, 1, KNIT_SHADE), px(16, 4, 1, 2, KNIT_SHADE), px(15, 3, 1, 1, KNIT_SHADE), px(12, 3, 1, 1, KNIT_SHINE), px(9, 2, 2, 1, KNIT_SHINE),
        px(4, 5, 2, 2, '#ffffff'), px(4, 6, 1, 1, KNIT_CUFF_SHADE), px(8, 6, 10, 1, '#ffffff'), px(16, 6, 2, 1, KNIT_CUFF_SHADE),
      ];
    case 'host':
      // Quizmaster's white top hat with a gold band (pa0a/src/quizmaster-sprite.ts).
      return [
        px(10, 2, 6, 3, c.light), px(15, 2, 1, 3, HAT_SHADE), px(11, 2, 1, 2, '#ffffff'),
        px(10, 5, 6, 1, c.accent), px(7, 6, 12, 1, c.light), px(17, 6, 2, 1, HAT_SHADE),
      ];
    case 'antenna':
      return [px(15, 6, 1, 1, INK), px(15, 3, 2, 2, c.accent), px(15, 3, 1, 1, '#ffffff')];
    case 'spikes':
      return [
        px(10, 5, 1, 2, c.body), px(13, 4, 1, 3, c.body), px(16, 5, 1, 2, c.body),
        px(6, 10, 1, 1, c.shade), px(6, 13, 1, 1, c.shade), px(19, 10, 1, 1, c.shade), px(19, 13, 1, 1, c.shade),
        px(13, 4, 1, 1, c.light),
      ];
    case 'ears':
      return [px(8, 5, 2, 3, c.body), px(16, 5, 2, 3, c.shade), px(8, 6, 1, 1, BLUSH), px(17, 6, 1, 1, BLUSH)];
    case 'horns':
      return [px(8, 5, 1, 2, c.accent), px(9, 6, 1, 1, c.accent), px(17, 5, 1, 2, c.accent), px(16, 6, 1, 1, c.accent)];
    case 'sprout':
      return [px(13, 5, 1, 2, '#3ccf6e'), px(14, 4, 2, 1, '#3ccf6e'), px(11, 4, 2, 1, '#1f8f4c')];
    case 'crown':
      return [
        px(10, 5, 6, 2, c.accent), px(10, 4, 1, 1, c.accent), px(12, 3, 2, 2, c.accent), px(15, 4, 1, 1, c.accent),
        px(12, 5, 2, 1, BLUSH),
      ];
    case 'visor':
      // Headphones
      return [px(9, 5, 8, 1, c.accent), px(8, 6, 1, 3, c.accent), px(17, 6, 1, 3, c.accent), px(6, 10, 2, 3, c.accent), px(18, 10, 2, 3, c.accent)];
    case 'beret':
      return [px(9, 5, 8, 2, c.accent), px(8, 6, 1, 1, c.accent), px(13, 4, 1, 1, c.accent), px(10, 5, 2, 1, '#9a7cff')];
    case 'quill':
      return [
        px(18, 2, 2, 1, '#f5f2ff'), px(17, 3, 3, 1, '#f5f2ff'), px(16, 4, 3, 1, '#f5f2ff'), px(16, 5, 2, 1, '#f5f2ff'),
        px(19, 2, 1, 1, '#c9c2e8'), px(18, 3, 1, 1, '#8f86b8'), px(17, 4, 1, 1, '#8f86b8'), px(16, 5, 1, 1, '#8f86b8'),
        px(16, 6, 1, 1, INK),
      ];
    case 'bulb':
      return [
        px(12, 2, 2, 1, c.accent), px(11, 3, 4, 2, c.accent), px(12, 3, 1, 1, '#ffffff'),
        px(12, 5, 2, 1, '#8a8aa8'), px(12, 6, 2, 1, INK),
      ];
    case 'mortarboard':
      return [px(8, 5, 10, 1, c.accent), px(10, 4, 6, 1, c.accent), px(12, 3, 2, 1, c.accent), px(10, 6, 6, 1, c.accent), px(17, 6, 1, 3, '#ffd23f')];
    case 'star':
      return [
        px(13, 2, 1, 1, c.accent), px(12, 3, 3, 1, c.accent), px(11, 4, 5, 1, c.accent),
        px(12, 5, 3, 1, c.accent), px(11, 6, 1, 1, c.accent), px(15, 6, 1, 1, c.accent), px(13, 3, 1, 1, '#ffffff'),
      ];
    case 'cap':
      return [px(9, 5, 8, 2, c.accent), px(10, 4, 6, 1, c.accent), px(5, 6, 4, 1, c.accent), px(12, 4, 2, 1, '#ffffff')];
    case 'sunhat':
      return [
        px(10, 3, 6, 2, '#f3d27a'), px(9, 4, 8, 1, '#f3d27a'), px(9, 5, 8, 1, c.accent),
        px(5, 6, 16, 1, '#f3d27a'), px(5, 6, 1, 1, '#c99a3c'), px(20, 6, 1, 1, '#c99a3c'), px(11, 3, 2, 1, '#fff1b8'),
      ];
    case 'toque':
      return [
        px(10, 2, 6, 1, '#ffffff'), px(9, 3, 8, 2, '#ffffff'), px(10, 5, 6, 2, '#e3def5'),
        px(12, 3, 1, 1, '#d2cbec'), px(15, 4, 1, 1, '#d2cbec'), px(10, 6, 6, 1, '#c9c2e8'),
      ];
    case 'partyhat':
      return [
        px(12, 2, 2, 1, '#ffe066'), px(12, 3, 2, 1, c.accent), px(11, 4, 4, 1, BLUSH), px(11, 5, 4, 1, c.accent),
        px(10, 6, 6, 1, BLUSH), px(7, 4, 1, 1, '#ffe066'), px(18, 3, 1, 1, c.accent), px(19, 6, 1, 1, BLUSH),
        px(6, 7, 1, 1, '#3ccf6e'),
      ];
    case 'hardhat':
      return [
        px(10, 3, 6, 1, c.accent), px(9, 4, 8, 2, c.accent), px(7, 6, 12, 1, '#d99a00'),
        px(13, 3, 1, 3, '#d99a00'), px(10, 4, 2, 1, '#fff1b8'),
      ];
    case 'flower':
      return [
        px(15, 2, 2, 2, '#ffffff'), px(13, 4, 2, 2, '#ffffff'), px(17, 4, 2, 2, '#ffffff'), px(15, 6, 2, 1, '#ffffff'),
        px(15, 4, 2, 2, '#ffd23f'), px(11, 5, 2, 1, '#3ccf6e'), px(12, 6, 1, 1, '#1f8f4c'),
      ];
    default:
      return [];
  }
}

// Accessories worn in front of the body (on the face or chest)
function renderOverlay(c: Creature) {
  switch (c.accessory) {
    case 'glasses':
      return [
        px(9, 9, 4, 1, c.accent), px(13, 9, 4, 1, c.accent), px(9, 10, 1, 2, c.accent), px(16, 10, 1, 2, c.accent),
        px(12, 10, 2, 1, c.accent), px(7, 10, 2, 1, c.shade), px(17, 10, 2, 1, c.shade),
      ];
    case 'goggles':
      return [
        px(7, 8, 12, 1, INK), px(9, 7, 3, 2, c.accent), px(14, 7, 3, 2, c.accent),
        px(9, 7, 1, 1, '#ffffff'), px(14, 7, 1, 1, '#ffffff'),
      ];
    case 'camera':
      return [
        px(5, 14, 1, 1, CAM_SHUTTER), px(7, 14, 2, 1, CAM_SIDE), px(4, 15, 8, 5, CAM_BODY), px(11, 15, 1, 5, CAM_SIDE), px(4, 19, 8, 1, CAM_SIDE),
        px(6, 16, 3, 3, INK), px(7, 17, 1, 1, CAM_GLINT), px(6, 16, 1, 1, CAM_LENS_RIM), px(10, 16, 1, 1, INK),
      ];
    case 'knithat':
      // Santa's red-and-white striped scarf.
      return [
        px(8, 16, 10, 1, KNIT_RED), px(10, 16, 1, 1, '#ffffff'), px(13, 16, 1, 1, '#ffffff'), px(16, 16, 1, 1, '#ffffff'),
        px(12, 17, 2, 2, KNIT_RED), px(12, 18, 2, 1, KNIT_SHADE), px(13, 17, 1, 1, '#ffffff'),
      ];
    case 'host':
      // Quizmaster's gold bow tie with an amber knot.
      return [px(9, 16, 3, 2, c.accent), px(14, 16, 3, 2, c.accent), px(12, 16, 2, 2, '#c9771a'), px(9, 16, 1, 1, '#fff0a0')];
    case 'bowtie':
      return [px(10, 16, 2, 2, c.accent), px(14, 16, 2, 2, c.accent), px(12, 16, 2, 1, '#a8323a'), px(12, 17, 2, 1, c.accent)];
    case 'headband':
      return [
        px(8, 8, 10, 1, c.accent), px(7, 9, 12, 1, c.accent),
        px(19, 8, 2, 1, c.accent), px(20, 9, 1, 2, c.accent), px(10, 9, 3, 1, '#ffffff'),
      ];
    case 'tie':
      return [px(12, 16, 2, 1, c.accent), px(12, 17, 2, 1, c.accent), px(12, 18, 2, 1, '#a8323a')];
    default:
      return [];
  }
}

/** House (26e): the body IS a little house. Roof wider than the walls, square walls, the eyes are its windows,
 *  a door at the bottom centre. Eyes sit on the same cells as every creature, so blink and asleep still work. */
function renderHouse(c: Creature) {
  return [
    // Ground shadow (only drawn at 48 px and up, like every creature)
    px(6, 20, 14, 1, '#261d4a', 'shadow'),
    // Chimney (left slope, so the mood badge never hides it)
    px(8, 2, 2, 4, c.shade, 'c1'),
    px(8, 2, 2, 1, c.body, 'c2'),
    // Roof, 45 degree slopes
    px(12, 2, 2, 1, c.accent, 'r1'),
    px(11, 3, 4, 1, c.accent, 'r2'),
    px(10, 4, 6, 1, c.accent, 'r3'),
    px(9, 5, 8, 1, c.accent, 'r4'),
    px(8, 6, 10, 1, c.accent, 'r5'),
    px(7, 7, 12, 1, c.accent, 'r6'),
    px(6, 8, 14, 1, c.accent, 'r7'),
    // Eave: the widest row, 2 px past each wall
    px(5, 9, 16, 1, c.accent, 'r8'),
    // Right slope in shade
    px(13, 2, 1, 1, '#a8323a', 'rs1'),
    px(14, 3, 1, 1, '#a8323a', 'rs2'),
    px(15, 4, 1, 1, '#a8323a', 'rs3'),
    px(16, 5, 1, 1, '#a8323a', 'rs4'),
    px(17, 6, 1, 1, '#a8323a', 'rs5'),
    px(18, 7, 1, 1, '#a8323a', 'rs6'),
    px(19, 8, 1, 1, '#a8323a', 'rs7'),
    px(20, 9, 1, 1, '#a8323a', 'rs8'),
    // Ridge highlight
    px(12, 2, 1, 1, '#ff8f8f', 'rh1'),
    px(11, 3, 1, 1, '#ff8f8f', 'rh2'),
    // Walls
    px(7, 10, 12, 10, c.body, 'w1'),
    // Right wall in shade
    px(18, 10, 1, 10, c.shade, 'w2'),
    // Ground line
    px(7, 19, 11, 1, c.shade, 'w3'),
    // Highlight under the left eave
    px(8, 10, 1, 2, c.light, 'w4'),
    // Eyes, which are also the windows (same cells as every creature)
    px(10, 10, 2, 3, INK, 'e1'),
    px(14, 10, 2, 3, INK, 'e2'),
    px(10, 10, 1, 1, '#f5f2ff', 'e3'),
    px(14, 10, 1, 1, '#f5f2ff', 'e4'),
    // Blush
    px(8, 13, 2, 1, BLUSH, 'k1'),
    px(16, 13, 2, 1, BLUSH, 'k2'),
    // Door, bottom centre, round top
    px(12, 15, 2, 1, '#8f5b4a', 'd1'),
    px(11, 16, 4, 4, '#8f5b4a', 'd2'),
    // Door knob
    px(13, 17, 1, 1, '#ffe066', 'd3'),
  ];
}

function renderCreature(c: Creature) {
  if (c.accessory === 'house') return renderHouse(c);
  return [
    // Ground shadow
    px(7, 20, 12, 1, '#261d4a', 'shadow'),
    ...renderAccessory(c),
    // Body
    px(9, 7, 8, 1, c.body, 'b1'),
    px(8, 8, 10, 1, c.body, 'b2'),
    px(7, 9, 12, 7, c.body, 'b3'),
    px(8, 16, 10, 1, c.body, 'b4'),
    px(9, 17, 8, 1, c.shade, 'b5'),
    // Shading and highlight
    px(18, 9, 1, 7, c.shade, 's1'),
    px(17, 16, 1, 1, c.shade, 's2'),
    px(7, 15, 1, 1, c.shade, 's3'),
    px(8, 16, 1, 1, c.shade, 's4'),
    px(10, 8, 2, 1, c.light, 'h1'),
    px(9, 9, 1, 1, c.light, 'h2'),
    // Eyes
    px(10, 10, 2, 3, INK, 'e1'),
    px(14, 10, 2, 3, INK, 'e2'),
    px(10, 10, 1, 1, '#f5f2ff', 'e3'),
    px(14, 10, 1, 1, '#f5f2ff', 'e4'),
    // Blush and smile
    px(8, 13, 2, 1, BLUSH, 'k1'),
    px(16, 13, 2, 1, BLUSH, 'k2'),
    px(11, 14, 1, 1, INK, 'm1'),
    px(14, 14, 1, 1, INK, 'm2'),
    px(12, 15, 2, 1, INK, 'm3'),
    // Feet
    px(9, 18, 2, 1, INK, 'f1'),
    px(15, 18, 2, 1, INK, 'f2'),
    ...renderOverlay(c),
  ];
}

/** The bot's creature, resolved exactly as ModernAvatar resolves it, for scenes that draw the bot outside its frame. */
export function useCreatureFor(role: string | undefined, name: string, kwantId?: string): Creature {
  useSyncExternalStore(subscribeBotLooks, botLooksRev, botLooksRev);
  return pickCreature(avatarStyleFor(role, kwantId, name, isStyleId) ?? 'default', name);
}
export { renderCreature, pickCreature };

/** Pocket apps: team colours in join order (pa0a/design.md "Team colours"); a 9th team starts again. */
export const TEAM_PALETTES = ['cyan', 'coral', 'mint', 'yellow', 'blue', 'green', 'red', 'violet'] as const;
export const isPalette = (name: string): boolean => Object.prototype.hasOwnProperty.call(PALETTES, name);
/** A plain guest creature (no accessory) in one of the avatar palettes; an unknown name gets cyan. */
export const paletteCreature = (name: string): Creature => ({ ...(PALETTES[name] ?? PALETTES.cyan), accessory: 'none' });
/** The 8 creatures a guest picks on the pocket page (pa5 guest/src/dom.ts AVATARS), by id. */
export const GUEST_CREATURES: Record<string, [string, Accessory]> = {
  antenna: ['yellow', 'antenna'],
  ears: ['blue', 'ears'],
  visor: ['cyan', 'visor'],
  spikes: ['green', 'spikes'],
  beret: ['coral', 'beret'],
  goggles: ['red', 'goggles'],
  bulb: ['mint', 'bulb'],
  horns: ['violet', 'horns'],
};
/** A guest's creature from its page id; an unknown or missing id gets the first one. */
export const guestCreature = (id: string | null | undefined): Creature => {
  const [palette, accessory] = (id && Object.prototype.hasOwnProperty.call(GUEST_CREATURES, id) ? GUEST_CREATURES[id] : null) ?? GUEST_CREATURES.antenna;
  return { ...PALETTES[palette], accessory };
};


export const ModernAvatar: React.FC<ModernAvatarProps> = ({
  role,
  name = '',
  size = 36,
  status,
  className = '',
  kwantId,
  modelBadge = true,
  asleep = false,
  mood: moodOverride,
  moodText: moodTextOverride,
}) => {
  const turnWorking = useKwantWorking(kwantId);
  const working = moodOverride ? moodOverride === 'working' : turnWorking;
  // Never green (or idle grey) while a turn runs: the turn stream wins over a stale status.
  const shown = status && working && STATUS_PRIORITY.indexOf(status) > STATUS_PRIORITY.indexOf('working') ? 'working' : status;
  // The mood (thinking, finished, failed, needs you, resting) adds a pixel mark and its own words.
  const kwantMood = useKwantMood(kwantId);
  const mood: Mood = moodOverride ?? kwantMood;
  const notices = useKwantNotices(kwantId);
  const moodText = moodTextOverride ?? moodLabel(mood, name, restingUntilOf(kwantId));
  const label = moodText ?? (shown ? statusText(name, shown) : name ? (working ? `${name} is working…` : name) : undefined);
  // Same bot, same look in every place: re-reads when the app's bot list changes.
  useSyncExternalStore(subscribeBotLooks, botLooksRev, botLooksRev);
  const creature = pickCreature(avatarStyleFor(role, kwantId, name, isStyleId) ?? 'default', name);
  // Dev mode: every bot avatar of 28 px and up wears the hard hat (top-left, clear of the status dot).
  const devOn = useDevModeOn();
  const hat = devOn && !!kwantId && size >= 28;
  // At tiny sizes the frame would dominate, so zoom in on the creature alone
  const framed = size > 20;
  // Small framed avatars enlarge the creature so it stays readable
  const compact = framed && size < 48;

  return (
    <div
      className={`kw-avatar relative inline-flex flex-shrink-0 select-none ${working ? 'kw-avatar-working' : ''} ${mood === 'resting' ? 'kw-avatar-resting' : ''} ${className}`}
      style={{ width: size, height: size }}
      title={label}
      role={moodTextOverride ? 'img' : undefined}
      aria-label={moodTextOverride}
      data-working={working || undefined}
      data-status={shown}
      data-mood={mood === 'idle' ? undefined : mood}
    >
      {working && <span className="kw-avatar-ring" aria-hidden="true" />}
      <svg
        viewBox={framed ? '0 0 26 26' : '5 2.5 16 17'}
        className="w-full h-full"
        style={{ imageRendering: 'pixelated', shapeRendering: 'crispEdges' }}
        aria-hidden="true"
      >
        {framed && FRAME_FILL.map(([x, y, w, h]) => px(x - 1, y - 1, w + 2, h + 2, FRAME, `fo-${x}-${y}`))}
        {framed && FRAME_FILL.map(([x, y, w, h]) => px(x, y, w, h, PANEL, `fi-${x}-${y}`))}
        {/* Hide scanlines at small sizes, where they'd just muddy the colors */}
        {size >= 28 &&
          Array.from({ length: 16 }, (_, i) => (
            <rect key={`scan-${i}`} x={3.5} y={4.5 + i} width={19} height={0.25} fill={SCAN} />
          ))}
        <g transform={compact ? 'translate(13 13.2) scale(1.2) translate(-13 -12)' : undefined}>
          <g className="kw-avatar-body">
            {renderCreature(creature).filter((r) => framed && !compact || r.key !== 'shadow')}
            {/* One blink when the person sends this bot a message: the eyes close for 160 ms. */}
            {asleep && (
              <g data-asleep>
                {px(10, 10, 2, 2, creature.body, 'sl1')}
                {px(14, 10, 2, 2, creature.body, 'sl2')}
                {px(10, 12, 2, 1, creature.shade, 'sl3')}
                {px(14, 12, 2, 1, creature.shade, 'sl4')}
              </g>
            )}
            {!asleep && notices > 0 && (
              <g key={notices} className="kw-avatar-blink">
                {px(10, 10, 2, 3, creature.body, 'bl1')}
                {px(14, 10, 2, 3, creature.body, 'bl2')}
                {px(10, 12, 2, 1, creature.shade, 'bl3')}
                {px(14, 12, 2, 1, creature.shade, 'bl4')}
              </g>
            )}
          </g>
        </g>
      </svg>
      <AvatarMoodMark mood={mood} size={size} />
      {hat && <HardHat />}
      {modelBadge && kwantId && modelBadgeSize(size) !== null && <AvatarModelBadge kwantId={kwantId} size={size} />}
      {shown && (
        <>
          <span className={`absolute bottom-0 right-0 w-2.5 h-2.5 rounded-full ring-2 ring-zinc-950 ${STATUS_DOT[shown]}`} aria-hidden="true" />
          <span className="sr-only">{label}</span>
        </>
      )}
    </div>
  );
};
