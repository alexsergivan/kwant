// kwant look guard (app look sweep, Nov 16): the app stays in kwant's palette.
// tailwind.config.js retints zinc (indigo) and sky (lavender), so Tailwind's own zinc/sky hex is the wrong grey
// and the wrong blue here, and the status colours each mean one thing (teal ok, amber warn, rose bad,
// lavender info). This walks frontend/src (*.ts, *.tsx, *.css, skipping generated/) and fails on:
//   (a) a stock Tailwind zinc/sky hex or rgb() (the list is the mockup kit's lint.py STOCK), and
//   (b) a colour class or theme() from a Tailwind family kwant doesn't use.
// emerald and violet are left out on purpose: they wait on Mira's ruling (audit section C).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src');

// Same list as /home/maus/shared/ship-software-safely/mockup-kit/lint.py STOCK (Tailwind 3 zinc and sky, 50..950).
const SHADES = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];
const STOCK = {
  zinc: '#fafafa #f4f4f5 #e4e4e7 #d4d4d8 #a1a1aa #71717a #52525b #3f3f46 #27272a #18181b #09090b',
  sky: '#f0f9ff #e0f2fe #bae6fd #7dd3fc #38bdf8 #0ea5e9 #0284c7 #0369a1 #075985 #0c4a6e #082f49',
};
const HEX = new Map();
for (const [fam, list] of Object.entries(STOCK)) list.split(' ').forEach((h, i) => HEX.set(h, `${fam}-${SHADES[i]}`));
const rgbOf = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const RGB = new Map([...HEX.keys()].map((h) => [rgbOf(h).join(','), h]));

const hexRe = /#[0-9a-f]{6}(?:[0-9a-f]{2})?(?![0-9a-z_-])/gi;
const rgbRe = /rgba?\(\s*(\d{1,3})[\s,]+(\d{1,3})[\s,]+(\d{1,3})\b/gi;

// Families kwant doesn't use, and the kwant token that carries the meaning instead.
const OFF = {
  gray: 'zinc (kwant greys)', slate: 'zinc (kwant greys)', neutral: 'zinc (kwant greys)', stone: 'zinc (kwant greys)',
  red: 'rose, or text-bad / var(--kw-bad)', pink: 'rose, or text-bad / var(--kw-bad)',
  orange: 'amber, or text-warn / var(--kw-warn)', yellow: 'amber, or text-warn / var(--kw-warn)',
  lime: 'teal, or text-ok / var(--kw-ok)', green: 'teal, or text-ok / var(--kw-ok)',
  cyan: 'sky (lavender), or text-info / var(--kw-info)', blue: 'sky (lavender), or text-info; control only for "you have the wheel"',
  indigo: 'sky (lavender), or text-info / var(--kw-info)', purple: 'sky (lavender), or text-info / var(--kw-info)',
  fuchsia: 'sky (lavender), or text-info / var(--kw-info)',
};
const fams = Object.keys(OFF).join('|');
// bg-red-500, hover:text-slate-300/80, ring-blue-400, from-indigo-600 ..., and theme('colors.red.500')
const classRe = new RegExp(`(?<![\\w])(?:[\\w-]+-)?(${fams})-(50|[1-9]00|950)(?![\\w-])|colors\\.(${fams})\\.(50|[1-9]00|950)`, 'g');

// Exemptions: file (relative to frontend/) and the exact values allowed there. Each one says why.
const EXEMPT = [
  // A categorical picker: each space needs a different hue, not a status meaning or a kwant grey.
  // #0ea5e9 is SPACE_COLORS' "sky" swatch and its NAMED_COLORS alias for older rows; changing it would recolour saved spaces.
  { file: 'src/spaces.ts', values: ['#0ea5e9'], why: 'SPACE_COLORS categorical picker' },
];
const exempt = (file, value) => EXEMPT.some((e) => e.file === file && e.values.includes(value.toLowerCase()));

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (name !== 'generated' && name !== 'node_modules') yield* walk(p);
    } else if (/\.(ts|tsx|css)$/.test(name)) yield p;
  }
}

export function scan() {
  const out = [];
  for (const p of walk(src)) {
    const file = relative(root, p).split('\\').join('/');
    readFileSync(p, 'utf8').split('\n').forEach((line, i) => {
      const at = `${file}:${i + 1}`;
      for (const m of line.matchAll(hexRe)) {
        const h = m[0].slice(0, 7).toLowerCase();
        if (HEX.has(h) && !exempt(file, h)) {
          const [fam, n] = HEX.get(h).split('-');
          out.push(`${at} stock Tailwind ${HEX.get(h)} ${m[0]}: use kwant's ${fam}-${n} (class ${fam}-${n}, theme('colors.${fam}.${n}')) or a --kw-* token`);
        }
      }
      for (const m of line.matchAll(rgbRe)) {
        const h = RGB.get(`${+m[1]},${+m[2]},${+m[3]}`);
        if (h && !exempt(file, h)) out.push(`${at} stock Tailwind ${HEX.get(h)} as ${m[0]}): use kwant's ${HEX.get(h)} or a --kw-* token`);
      }
      for (const m of line.matchAll(classRe)) {
        const fam = m[1] ?? m[3];
        if (!exempt(file, m[0])) out.push(`${at} ${m[0]} is outside kwant's palette: use ${OFF[fam]}`);
      }
    });
  }
  return out;
}

test('frontend/src stays in the kwant palette (no stock zinc/sky hex, no off-theme colour families)', () => {
  const hits = scan();
  assert.deepEqual(hits, [], `${hits.length} place(s) bypass kwant's look:\n  ${hits.join('\n  ')}\n`);
});

test('the guard itself catches what it should and lets kwant tokens through', () => {
  const t = (s) => [...s.matchAll(classRe)].map((m) => m[0]);
  assert.deepEqual(t('bg-red-500/20 hover:text-slate-300 ring-blue-400 theme(\'colors.orange.400\')'), ['bg-red-500', 'text-slate-300', 'ring-blue-400', 'colors.orange.400']);
  assert.deepEqual(t('bg-rose-500 text-zinc-400 border-sky-400 text-emerald-300 bg-violet-500 text-bad'), []);
  assert.equal(HEX.get('#38bdf8'), 'sky-400');
  assert.equal(RGB.get('39,39,42'), '#27272a');
  assert.ok(!'#1d1840 #6c58e0 #9d8fff'.match(hexRe).some((h) => HEX.has(h)), 'kwant retinted values are fine');
  assert.ok(exempt('src/spaces.ts', '#0EA5E9') && !exempt('src/components/Sidebar.tsx', '#0ea5e9'));
});
