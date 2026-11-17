// kwant look guard (app look sweep, Nov 16): the app stays in kwant's palette.
// tailwind.config.js retints zinc (indigo) and sky (lavender), so Tailwind's own zinc/sky hex is the wrong grey
// and the wrong blue here, and the status colours each mean one thing (teal ok, amber warn, rose bad,
// lavender info). This walks frontend/src (*.ts, *.tsx, *.css, skipping generated/) and fails on:
//   (a) a stock Tailwind zinc/sky hex or rgb() (the list is the mockup kit's lint.py STOCK), and
//   (b) a colour class or theme() from a Tailwind family kwant doesn't use, and
//   (c) white text on a light lavender (26g follow-up): text-white in the same class list as bg-sky-500 or lighter,
//       hover:bg-sky-500 or lighter, or hover:brightness-1xx. White is 3.76:1 on sky-500 and 3.66:1 on the
//       brightened accent gradient; use bg-sky-600 (5.1:1) with hover:bg-sky-700 (7.0:1), kw-btn-primary, or
//       kw-hover-deepen on a bg-accent-gradient.
//   (d) the same in CSS (26g follow-up 2): filter: brightness() above 1 in a :hover rule whose selector, or its
//       base rule, sets white text (color #fff / white, @apply text-white). Use the deepen gradient instead.
//   (e) the accent gradient's retired light stop #7a62f0 (4.35:1 under white), in frontend/src and
//       tailwind.config.js. It is #745bef now. The rgb(122 98 240 / a) tints are a different thing and stay.
// emerald and violet (Mira's ruling, audit section C, roadmap 26g): emerald is teal / ok and violet is kwant's
// sky (lavender), so their classes and stock hex fail too. scripts/retint-emerald-violet.mjs does the mapping.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src');

// zinc and sky: same list as /home/maus/shared/ship-software-safely/mockup-kit/lint.py STOCK. emerald and violet:
// Tailwind 3's own hex, which map to kwant teal and sky (KWANT). All 50..950.
const SHADES = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];
const STOCK = {
  zinc: '#fafafa #f4f4f5 #e4e4e7 #d4d4d8 #a1a1aa #71717a #52525b #3f3f46 #27272a #18181b #09090b',
  sky: '#f0f9ff #e0f2fe #bae6fd #7dd3fc #38bdf8 #0ea5e9 #0284c7 #0369a1 #075985 #0c4a6e #082f49',
  emerald: '#ecfdf5 #d1fae5 #a7f3d0 #6ee7b7 #34d399 #10b981 #059669 #047857 #065f46 #064e3b #022c22',
  violet: '#f5f3ff #ede9fe #ddd6fe #c4b5fd #a78bfa #8b5cf6 #7c3aed #6d28d9 #5b21b6 #4c1d95 #2e1065',
};
// The kwant family each stock family's hex should become.
const KWANT = { zinc: 'zinc', sky: 'sky', emerald: 'teal', violet: 'sky' };
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
  emerald: 'teal: text-ok, kw-chip-ok, kw-status-ok; solid buttons bg-teal-300 text-zinc-950 (scripts/retint-emerald-violet.mjs)',
  violet: 'sky (lavender) of the same shade, text-info, kw-focus or kw-btn-primary (scripts/retint-emerald-violet.mjs)',
};
const fams = Object.keys(OFF).join('|');
// bg-red-500, hover:text-slate-300/80, ring-blue-400, from-indigo-600 ..., and theme('colors.red.500')
const classRe = new RegExp(`(?<![\\w])(?:[\\w-]+-)?(${fams})-(50|[1-9]00|950)(?![\\w-])|colors\\.(${fams})\\.(50|[1-9]00|950)`, 'g');

// Exemptions: file (relative to frontend/) and the exact values allowed there. Each one says why.
const EXEMPT = [
  // A categorical picker: each space needs a different hue, not a status meaning or a kwant grey.
  // #0ea5e9 is SPACE_COLORS' "sky" swatch and its NAMED_COLORS alias for older rows; changing it would recolour saved spaces.
  // #10b981 and #8b5cf6 are its emerald and violet swatches, user-picked the same way (Mira's ruling, exemption 1).
  { file: 'src/spaces.ts', values: ['#0ea5e9', '#10b981', '#8b5cf6'], why: 'SPACE_COLORS categorical picker' },
  // The holo sticker's conic rainbow (.tk-holo, earned results only). #c4b5fd is one stop of the rainbow, not an accent (exemption 2).
  { file: 'src/styles/tickets.css', values: ['#c4b5fd'], why: 'holo conic gradient' },
  // Exemption 3, Chart SERIES_COLORS, needs no entry: the sweep's section A moved the series to kwant classes, so no #34d399 is left.
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

// (c) One class list is one line here: the app writes each className, or each branch of a ternary, on one line.
const whiteRe = /(?<![\w-])text-white(?![\w-])/;
const lightSkyRe = /(?<![\w-])((?:[\w-]+:)*)bg-sky-(50|100|200|300|400|500)(?![\w/-])/g;
const brightRe = /(?<![\w-])((?:[\w-]+:)*hover:)brightness-(\d+)(?![\w-])/g;
const WHITE_EXEMPT = [
  // None. List a real exemption here as { at: 'src/x.tsx:12', why: '...' }.
];
export function whiteOnLight(line) {
  if (!whiteRe.test(line)) return [];
  const out = [];
  for (const m of line.matchAll(lightSkyRe)) {
    const hover = /hover:/.test(m[1]);
    out.push(`${m[0]} under text-white is ${m[2] === '500' ? '3.76' : 'under 3.8'}:1: use ${hover ? `${m[1]}bg-sky-700` : `${m[1]}bg-sky-600`} (bg-sky-600 rests at 5.1:1, hover:bg-sky-700 is 7.0:1)`);
  }
  for (const m of line.matchAll(brightRe)) {
    if (+m[2] > 100) out.push(`${m[0]} next to text-white lightens the fill under white text (3.66:1 on the accent gradient): use kw-hover-deepen (or hover:bg-sky-700 on a flat bg-sky-600)`);
  }
  return out;
}

// (d) Flat CSS rules (innermost blocks, so rules inside @media and @layer count too).
const WHITE_CSS = /(?:^|[;{\s])color\s*:\s*(?:#fff(?:fff)?|white)\b|@apply[^;]*(?<![\w-])text-white(?![\w-])/i;
const BRIGHT_CSS = /filter\s*:[^;]*brightness\(\s*([\d.]+)(%?)\s*\)/i;
export function cssBrightHovers(text) {
  const rules = [];
  for (const m of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const sel = m[1].replace(/\/\*[\s\S]*?\*\//g, '').trim();
    const line = text.slice(0, m.index + m[1].length).split('\n').length;
    rules.push({ sels: sel.split(',').map((x) => x.trim()), body: m[2], line });
  }
  const whiteSel = new Set(rules.filter((r) => WHITE_CSS.test(r.body)).flatMap((r) => r.sels));
  const out = [];
  for (const r of rules) {
    const b = r.body.match(BRIGHT_CSS);
    if (!b || (b[2] ? +b[1] / 100 : +b[1]) <= 1) continue;
    const hover = r.sels.filter((x) => x.includes(':hover'));
    const base = (x) => x.replace(/:hover/g, '').replace(/:not\([^)]*\)/g, '').trim();
    if (hover.length && (WHITE_CSS.test(r.body) || hover.some((x) => whiteSel.has(x) || whiteSel.has(base(x)))))
      out.push({ line: r.line, msg: `${hover[0]} brightens white text on hover (${b[0]}): deepen the gradient instead, background-image: linear-gradient(135deg, theme('colors.sky.600') 0%, theme('colors.sky.700') 100%) (kw-hover-deepen)` });
  }
  return out;
}
// (e)
const RETIRED_STOP = /#7a62f0(?![0-9a-f])/gi;
const retired = (at, line) => [...line.matchAll(RETIRED_STOP)].map((m) => `${at} ${m[0]} is the retired accent light stop (4.35:1 under white): use bg-accent-gradient / var(--accent-gradient), or #745bef`);

export function scan() {
  const out = [];
  for (const p of walk(src)) {
    const file = relative(root, p).split('\\').join('/');
    const text = readFileSync(p, 'utf8');
    if (file.endsWith('.css')) for (const h of cssBrightHovers(text)) out.push(`${file}:${h.line} ${h.msg}`);
    text.split('\n').forEach((line, i) => {
      const at = `${file}:${i + 1}`;
      out.push(...retired(at, line));
      for (const m of line.matchAll(hexRe)) {
        const h = m[0].slice(0, 7).toLowerCase();
        if (HEX.has(h) && !exempt(file, h)) {
          const [stock, n] = HEX.get(h).split('-');
          const fam = KWANT[stock];
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
      if (!file.endsWith('.css') && !WHITE_EXEMPT.some((e) => e.at === at)) for (const msg of whiteOnLight(line)) out.push(`${at} ${msg}`);
    });
  }
  readFileSync(join(root, 'tailwind.config.js'), 'utf8').split('\n').forEach((line, i) => out.push(...retired(`tailwind.config.js:${i + 1}`, line)));
  return out;
}

test('frontend/src stays in the kwant palette (no stock zinc/sky hex, no off-theme colour families, no white on light lavender)', () => {
  const hits = scan();
  assert.deepEqual(hits, [], `${hits.length} place(s) bypass kwant's look:\n  ${hits.join('\n  ')}\n`);
});

test('the guard itself catches what it should and lets kwant tokens through', () => {
  const t = (s) => [...s.matchAll(classRe)].map((m) => m[0]);
  assert.deepEqual(t('bg-red-500/20 hover:text-slate-300 ring-blue-400 theme(\'colors.orange.400\')'), ['bg-red-500', 'text-slate-300', 'ring-blue-400', 'colors.orange.400']);
  assert.deepEqual(t('bg-rose-500 text-zinc-400 border-sky-400 bg-teal-300 text-bad text-ok'), []);
  assert.deepEqual(t('text-emerald-300 hover:bg-violet-500/80 accent-emerald-500 theme(colors.violet.400)'), ['text-emerald-300', 'bg-violet-500', 'accent-emerald-500', 'colors.violet.400']);
  assert.equal(HEX.get('#059669'), 'emerald-600');
  assert.equal(HEX.get('#a78bfa'), 'violet-400');
  assert.equal(HEX.get('#38bdf8'), 'sky-400');
  assert.equal(RGB.get('39,39,42'), '#27272a');
  assert.ok(!'#1d1840 #6c58e0 #9d8fff'.match(hexRe).some((h) => HEX.has(h)), 'kwant retinted values are fine');
  assert.ok(exempt('src/spaces.ts', '#0EA5E9') && !exempt('src/components/Sidebar.tsx', '#0ea5e9'));
  assert.ok(exempt('src/spaces.ts', '#10b981') && exempt('src/styles/tickets.css', '#c4b5fd') && !exempt('src/styles/tickets.css', '#a78bfa'));
  // (c) white text on a light lavender
  assert.equal(whiteOnLight("'bg-sky-500 text-white border-sky-400'").length, 1);
  assert.equal(whiteOnLight('bg-sky-500 hover:bg-sky-400 text-white').length, 2);
  assert.equal(whiteOnLight('bg-accent-gradient text-white hover:brightness-110').length, 1);
  assert.equal(whiteOnLight('bg-sky-600 text-white hover:bg-sky-700').length, 0);
  assert.equal(whiteOnLight('bg-sky-500/10 text-white disabled:hover:brightness-100').length, 0, 'tints and a no-op brightness are fine');
  assert.equal(whiteOnLight('bg-sky-500 text-zinc-950 hover:brightness-110').length, 0, 'only white text');
  assert.equal(whiteOnLight('kw-hover-deepen bg-accent-gradient text-white').length, 0);
  // (d) CSS hovers that brighten white text
  const css = `.a { color: #fff; background: x; }\n.a:hover { filter: brightness(1.08); }\n.b { color: #fff; }\n.b:hover:not(:disabled) { filter: brightness(108%); }\n.c:hover { filter: brightness(1.1); }\n.d { color: #fff; }\n.d:hover { filter: brightness(1); }\n.e { @apply text-white; }\n.e:hover { filter: saturate(1.2) brightness(1.2); }`;
  assert.deepEqual(cssBrightHovers(css).map((h) => h.line), [2, 4, 9], 'white base + brightness > 1 only');
  // (e) the retired stop, any case; the rgb tint is fine
  assert.equal(retired('x', 'from-[#7A62F0] #7a62f0 #745bef rgb(122 98 240 / 0.2)').length, 2);
});
