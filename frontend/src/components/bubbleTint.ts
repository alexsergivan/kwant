import type { CSSProperties } from 'react';

// Generated from autonomy/reply-bubble-bot-tint/tools/tint.py; recipe in design.md (OKLCH L 0.220, chroma <= 0.038, border L 0.38).
/** A bot reply bubble's tint, keyed by the lowercase avatar body hex from ModernAvatar's PALETTES. */
export const BUBBLE_TINTS: Readonly<Record<string, { bg: string; line: string }>> = Object.freeze({
  '#fdc934': { bg: '#221904', line: '#553f00' }, // yellow
  '#3ccf6e': { bg: '#0c2011', line: '#194f29' }, // green
  '#ff7eb6': { bg: '#28131c', line: '#642d45' }, // pink
  '#5ce1ff': { bg: '#021f25', line: '#004c5b' }, // cyan
  '#a98bff': { bg: '#1b172b', line: '#45386b' }, // violet
  '#ff9f43': { bg: '#271607', line: '#623605' }, // orange
  '#6d9bff': { bg: '#121a2c', line: '#2b406f' }, // blue
  '#ff8a7a': { bg: '#2a1310', line: '#682e27' }, // coral
  '#2fd4b5': { bg: '#02201a', line: '#005041' }, // teal
  '#ff6b6b': { bg: '#2a1312', line: '#682d2d' }, // red
  '#f7c948': { bg: '#221904', line: '#553f00' }, // gold
  '#d49cff': { bg: '#201528', line: '#503464' }, // lilac
  '#9be564': { bg: '#131e0c', line: '#2f4c18' }, // mint
  '#7cc8ff': { bg: '#0c1c28', line: '#1e4662' }, // sky
  '#f06bd8': { bg: '#251422', line: '#5d2f54' }, // magenta
  '#a3aecf': { bg: '#171a24', line: '#3a4158' }, // slate
  '#ffb347': { bg: '#251705', line: '#5e3900' }, // amber
  '#3fb6c9': { bg: '#051f23', line: '#054b55' }, // ocean
  '#e0607e': { bg: '#291317', line: '#672d3a' }, // berry
  '#c6f04a': { bg: '#171d08', line: '#3a490b' }, // lime
  '#ffb894': { bg: '#26160f', line: '#5b3927' }, // peach
  '#7fa3b8': { bg: '#111c23', line: '#2c4655' }, // steel
  '#f7b3d2': { bg: '#24161c', line: '#583746' }, // blossom
});

/** Inline vars for `.kw-bubble-tint`, or undefined when the body is not one of the avatar palettes. */
export function bubbleTintStyle(body: string | undefined): CSSProperties | undefined {
  const t = body ? BUBBLE_TINTS[body.toLowerCase()] : undefined;
  if (!t) return undefined;
  return { '--kw-bubble-bg': t.bg, '--kw-bubble-line': t.line } as CSSProperties;
}
