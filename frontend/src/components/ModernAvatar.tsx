import React from 'react';

interface ModernAvatarProps {
  role?: string;
  name?: string;
  size?: number; // pixel diameter
  isOnline?: boolean;
  className?: string;
}

type Accessory = 'antenna' | 'spikes' | 'ears' | 'horns' | 'sprout' | 'crown' | 'visor' | 'none';

interface Creature {
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
};

const hash = (s: string) => {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
};

function pickCreature(role: string, name: string): Creature {
  const norm = `${role} ${name}`.toLowerCase();
  const rules: Array<[string[], keyof typeof PALETTES, Accessory]> = [
    [['laya'], 'violet', 'crown'],
    [['user', 'player', 'hero', 'you'], 'blue', 'ears'],
    [['architect', 'cyber_mage', 'mage'], 'violet', 'horns'],
    [['tech', 'ninja', 'pixel', 'lead'], 'cyan', 'visor'],
    [['devops', 'terminal', 'knight', 'shell', 'infra'], 'green', 'spikes'],
    [['security', 'shield', 'guard'], 'orange', 'ears'],
    [['fast', 'grok', 'zap', 'speed'], 'pink', 'sprout'],
  ];
  for (const [keys, palette, accessory] of rules) {
    if (keys.some((k) => norm.includes(k))) return { ...PALETTES[palette], accessory };
  }
  // Unknown roles still get a stable, distinct creature per name
  const palettes = Object.keys(PALETTES);
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

function renderAccessory(c: Creature) {
  switch (c.accessory) {
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
    default:
      return [];
  }
}

function renderCreature(c: Creature) {
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
  ];
}

export const ModernAvatar: React.FC<ModernAvatarProps> = ({
  role = 'default',
  name = '',
  size = 36,
  isOnline,
  className = '',
}) => {
  const creature = pickCreature(role, name);
  // At tiny sizes the frame would dominate, so zoom in on the creature alone
  const framed = size > 20;
  // Small framed avatars enlarge the creature so it stays readable
  const compact = framed && size < 48;

  return (
    <div
      className={`relative inline-flex flex-shrink-0 select-none ${className}`}
      style={{ width: size, height: size }}
      title={name || undefined}
    >
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
        <g transform={compact ? 'translate(13 12) scale(1.2) translate(-13 -12)' : undefined}>
          {renderCreature(creature).filter((r) => framed && !compact || r.key !== 'shadow')}
        </g>
      </svg>
      {isOnline !== undefined && (
        <span
          className={`absolute bottom-0 right-0 w-2.5 h-2.5 rounded-full ring-2 ring-zinc-950 ${
            isOnline ? 'bg-emerald-400' : 'bg-zinc-600'
          }`}
        />
      )}
    </div>
  );
};
