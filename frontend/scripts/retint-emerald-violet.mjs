#!/usr/bin/env node
// Retint stock emerald to kwant teal/ok and stock violet to kwant sky (lavender) in frontend/src.
// Mira's ruling, Nov 16 2026: /home/maus/shared/ship-software-safely/autonomy/app-look-sweep/ruling.md
// The mapping lives below as data so this can be re-run after a rebase:
//   node scripts/retint-emerald-violet.mjs emerald   # commit 1: emerald -> teal / ok
//   node scripts/retint-emerald-violet.mjs violet    # commit 2: violet  -> sky
//   node scripts/retint-emerald-violet.mjs           # both
//   add --dry to print what would change without writing.
// Order matters: file-scoped hand rules first (exact strings), then role rules (buttons, focus, switches),
// then the plain shade map. Exempt by name: src/spaces.ts (user-picked space colours) and the holo conic
// gradient in src/styles/tickets.css (earned stickers only).
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src');
const args = process.argv.slice(2);
const dry = args.includes('--dry');
const only = args.find((a) => a === 'emerald' || a === 'violet');
const EXEMPT_FILES = ['src/spaces.ts', 'src/styles/tickets.css'];

// Tailwind variant prefix (hover:, focus-visible:, group-hover:, sm:, data-[x]:, ...), kept as is.
const V = String.raw`((?:[\w-]+(?:\[[^\]\s]*\])?:)*)`;
// An alpha suffix: /10, /[.22], /[0.05].
const A = String.raw`(\/(?:\d+|\[[\d.]+\]))?`;
const END = String.raw`(?![\w-])`;
const SOLID_OK = 'bg-teal-300 text-zinc-950 font-semibold hover:bg-teal-200';

// Each rule: [regex source, replacement (string or fn), note]. Regexes are global.
const RULES = {
  emerald: {
    // Hand rules: exact strings in named files (ruling: solid buttons, the Skills switch, the LetterView checkbox).
    hand: [
      ['src/components/ModernApprovalCard.tsx', 'bg-[#059669] hover:bg-emerald-500 text-[15px] sm:text-sm font-semibold text-white', 'bg-teal-300 hover:bg-teal-200 text-[15px] sm:text-sm font-semibold text-zinc-950', '"Allow once": 3.77:1 -> 12.7:1'],
      ['src/components/ReviewItemView.tsx', 'bg-[#059669] text-white hover:bg-emerald-500', SOLID_OK, 'btnAllow'],
      ['src/components/SkillsPanel.tsx', 'border-emerald-500/40 bg-emerald-600/80 text-white hover:bg-emerald-500', `border-teal-300/40 ${SOLID_OK}`, 'goBtn'],
      ['src/components/SkillsPanel.tsx', "'bg-emerald-500/70 border-emerald-400/60'", "'bg-sky-600 border-transparent'", 'on-switch: a control state, sky-600 like every other switch'],
      ['src/components/LetterView.tsx', 'accent-emerald-500', 'accent-sky-500', 'checkbox, like the 35 others'],
      ['src/components/SpaceReadyCard.tsx', "'bg-emerald-500 text-zinc-950'", "'bg-teal-300 text-zinc-950'", 'done step'],
    ],
    role: [
      // Solid buttons with white text, in the two orders the app writes them.
      [String.raw`bg-emerald-600(?:\/\d+)? text-white hover:bg-emerald-500${END}`, SOLID_OK, 'solid button'],
      [String.raw`bg-emerald-600(?:\/\d+)? hover:bg-emerald-500 text-white${END}`, SOLID_OK, 'solid button'],
      [String.raw`theme\((['"]?)colors\.emerald\.(\d+)\1\)`, (_m, q, n) => `theme(${q}colors.teal.${n}${q})`, 'css theme()'],
    ],
    shade: [
      // text: 300/400 are ok; with an alpha the same colour as a class that takes alpha.
      [String.raw`${V}text-emerald-(?:300|400)${A}${END}`, (_m, v, a) => (a ? `${v}text-teal-300${a}` : `${v}text-ok`)],
      [String.raw`${V}text-emerald-(50|100|200)${A}${END}`, (_m, v, n, a) => `${v}text-teal-${n}${a ?? ''}`],
      // dots: solid 400/500 are the ok status colour (kw-status-ok = bg-teal-300).
      [String.raw`${V}bg-emerald-(?:400|500)${END}(?!\/)`, (_m, v) => `${v}bg-teal-300`],
      // tints, borders and rings: teal-400 with the same alpha (the chip default is /10 and /30).
      [String.raw`${V}(bg|border|ring)-emerald-(?:300|400|500)${A}${END}`, (_m, v, p, a) => `${v}${p}-teal-400${a ?? ''}`],
      [String.raw`${V}(fill|stroke)-emerald-(?:300|400)${A}${END}`, (_m, v, p, a) => `${v}${p}-teal-300${a ?? ''}`],
      // Anything left keeps its shade in teal.
      [String.raw`${V}([\w-]+)-emerald-(\d+)${A}${END}`, (_m, v, p, n, a) => `${v}${p}-teal-${n}${a ?? ''}`],
    ],
    hex: [],
  },
  violet: {
    hand: [
      // The one main action of each view becomes the kit's primary button; size utilities stay (utilities beat components).
      ['src/components/SpaceDocsTab.tsx', 'inline-flex items-center justify-center gap-1.5 px-3 rounded-lg text-xs font-medium text-white bg-violet-600 hover:bg-violet-500 transition-colors motion-reduce:transition-none disabled:opacity-50 ${focusRing}', 'kw-btn-primary kw-focus px-3 rounded-lg text-xs motion-reduce:transition-none', '"Choose files"'],
      ['src/components/FirstRunWelcome.tsx', 'mt-6 inline-flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium text-white bg-violet-600 hover:bg-violet-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-300', 'kw-btn-primary kw-focus mt-6 gap-2 px-4 py-2 rounded-lg text-sm motion-reduce:transition-none', 'first-run main action'],
      ['src/components/EngineInstall.tsx', 'inline-flex items-center gap-1.5 px-2.5 py-1 min-h-[28px] rounded-md text-[11px] font-medium text-white bg-violet-600 hover:bg-violet-500 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-300', 'kw-btn-primary kw-focus px-2.5 py-1 min-h-[28px] rounded-md text-[11px] motion-reduce:transition-none', 'Install'],
    ],
    role: [
      // Focus: the kit's ring colour (kw-focus = outline-sky-400).
      [String.raw`${V}(outline|ring|border)-violet-(?:300|400)${END}`, (m, v, p) => (/focus/.test(v) ? `${v}${p}-sky-400` : m), 'focus ring'],
      // Solid buttons and badges with white text: sky-600 (5.1:1).
      [String.raw`bg-violet-(?:500|600)(?:\/80)? text-white hover:bg-violet-(?:400|500)${END}`, 'bg-sky-600 text-white hover:bg-sky-500', 'solid button'],
      [String.raw`bg-violet-(?:500|600)(?:\/80)? text-white${END}`, 'bg-sky-600 text-white', 'solid button'],
      [String.raw`bg-violet-600 border-violet-400 text-white${END}`, 'bg-sky-600 border-sky-400 text-white', 'point toggle'],
      [String.raw`theme\((['"]?)colors\.violet\.(\d+)\1\)`, (_m, q, n) => `theme(${q}colors.sky.${n}${q})`, 'css theme()'],
    ],
    shade: [
      [String.raw`${V}text-violet-300${END}(?!\/)`, (_m, v) => `${v}text-info`],
      [String.raw`${V}([\w-]+)-violet-(\d+)${A}${END}`, (_m, v, p, n, a) => `${v}${p}-sky-${n}${a ?? ''}`],
    ],
    // Stock violet hex and rgb -> kwant sky of the same number.
    hex: [
      [String.raw`#a78bfa(?![0-9a-f])`, '#9d8fff'],
      [String.raw`#c4b5fd(?![0-9a-f])`, '#b8adff'],
      [String.raw`#ddd6fe(?![0-9a-f])`, '#d3ccff'],
      [String.raw`rgba?\(\s*167[\s,]+139[\s,]+250\b`, (m) => m.replace(/167([\s,]+)139([\s,]+)250/, '157$1143$2255')],
    ],
  },
};

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (name !== 'generated' && name !== 'node_modules') yield* walk(p);
    } else if (/\.(ts|tsx|css)$/.test(name)) yield p;
  }
}

const families = only ? [only] : ['emerald', 'violet'];
let files = 0;
let edits = 0;
const missing = [];
for (const fam of families) {
  const r = RULES[fam];
  for (const [file, from, to, note] of r.hand) {
    const p = join(root, file);
    const text = readFileSync(p, 'utf8');
    if (!text.includes(from)) {
      if (!text.includes(to)) missing.push(`${file}: hand rule not found (${note})`);
      continue;
    }
    if (!dry) writeFileSync(p, text.split(from).join(to));
    edits++;
  }
}
for (const p of walk(src)) {
  const file = relative(root, p).split('\\').join('/');
  if (EXEMPT_FILES.includes(file)) continue;
  const before = readFileSync(p, 'utf8');
  let text = before;
  for (const fam of families) {
    const r = RULES[fam];
    for (const [re, to] of [...r.role, ...r.shade, ...r.hex]) text = text.replace(new RegExp(re, 'gi'), to);
  }
  if (text !== before) {
    files++;
    const n = before.split('\n').filter((l, i) => l !== text.split('\n')[i]).length;
    edits += n;
    console.log(`${dry ? 'would change' : 'changed'} ${file} (${n} line${n === 1 ? '' : 's'})`);
    if (!dry) writeFileSync(p, text);
  }
}
for (const m of missing) console.warn(`warning: ${m}`);
console.log(`${families.join(' + ')}: ${files} file(s), ${edits} line(s)${dry ? ' (dry run)' : ''}`);
