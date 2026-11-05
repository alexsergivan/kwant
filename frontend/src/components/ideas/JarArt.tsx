import React from 'react';

// The pixel jar (14x16 grid, crispEdges), ported from Mira's mockups/jar.js. Our own art.
const SLIP_COLORS = ['#ecebfa', '#fcd34d', '#5eead4', '#f9a8d4', '#b8adff'];
const W = 14;
const H = 16;

export interface JarArtProps {
  /** Width in px; the height follows (16/14). */
  size: number;
  /** Slips drawn inside, at most 15. */
  slips?: number;
  lid?: 'on' | 'open';
  dim?: boolean;
  /** Extra class on the svg (the wobble). */
  className?: string;
}

export const JarArt: React.FC<JarArtProps> = ({ size, slips = 0, lid = 'on', dim = false, className = '' }) => {
  const c = size / W;
  const rects: React.ReactNode[] = [];
  let k = 0;
  const R = (x: number, y: number, fill: string, w = 1, h = 1, op = 1) =>
    rects.push(<rect key={k++} x={x * c} y={y * c} width={w * c} height={h * c} fill={fill} fillOpacity={op < 1 ? op : undefined} />);
  const sides: Record<number, [number, number]> = { 2: [4, 9], 3: [3, 10], 4: [2, 11], 14: [2, 11] };
  for (let y = 5; y <= 13; y++) sides[y] = [1, 12];
  const glass = dim ? '#3a3366' : '#8d84d6';
  for (let y = 2; y <= 14; y++) {
    const [a, b] = sides[y];
    R(a + 1, y, 'rgb(184 173 255)', b - a - 1, 1, 0.07);
    R(a, y, glass);
    R(b, y, glass);
  }
  R(3, 15, glass, 8);
  let n = Math.max(0, Math.min(Math.floor(slips), 15));
  let y = 13;
  let row = 0;
  while (n > 0 && y >= 9) {
    for (let x = 2 + (row % 2); x + 1 <= 11 && n > 0; x += 2) {
      R(x, y, SLIP_COLORS[(x + y) % 5], 2, 1, dim ? 0.35 : 0.9);
      n--;
    }
    y--;
    row++;
  }
  R(2, 6, '#ffffff', 1, 3, 0.28);
  R(3, 5, '#ffffff', 1, 1, 0.2);
  if (lid === 'on') {
    R(4, 0, '#b8adff', 6);
    R(3, 1, '#7c6cf0', 8);
    R(3, 1, '#9d8fff', 1);
  } else {
    R(9, 0, '#b8adff', 4);
    R(9, 1, '#7c6cf0', 4);
  }
  return (
    <svg className={`kw-jar-pix ${className}`} width={size} height={(size * H) / W} viewBox={`0 0 ${size} ${(size * H) / W}`} aria-hidden="true" focusable="false" data-jar-slips={Math.min(slips, 15)}>
      {rects}
    </svg>
  );
};

/** The 24-unit stroke jar for nav rows and section headings, in currentColor. Not lucide's Amphora. */
export const JarIcon: React.FC<{ size?: number; className?: string }> = ({ size = 15, className = '' }) => (
  <svg
    viewBox="0 0 24 24"
    width={size}
    height={size}
    aria-hidden="true"
    focusable="false"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    className={`shrink-0 ${className}`}
    data-jar-icon
  >
    <path d="M8 3h8M7 6h10" />
    <path d="M8 6v1.5L5.5 10V19a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-9L16 7.5V6" />
    <path d="M9 15h2M13 17h2M10 18.5h1" />
  </svg>
);
