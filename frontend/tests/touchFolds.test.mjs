// Phone folds stay centred (28d, Nov 17). On a touch phone, modern.css's (max-width: 767px) and (pointer: coarse)
// block gives every <summary> a 44 px min height for tapping. A plain block summary then keeps its title at the top
// of that box with ~30 px empty under it, and a flex summary with items-start does the same. So:
//   (a) that phone block sets `align-content: center` on `summary` (block summaries centre in the 44 px row), and
//   (b) every <summary> in frontend/src whose className has `flex` or `inline-flex` also centres its items:
//       `items-center`, the phone-only `[@media(max-width:767px)_and_(pointer:coarse)]:items-center`, or it is a
//       `data-compact` control (which the 44 px rule skips).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src');
const PHONE_QUERY = '(max-width: 767px) and (pointer: coarse)';

// The bodies of every @media block with exactly this query, by brace matching.
export function mediaBlocks(css, query) {
  const out = [];
  for (let at = css.indexOf(`@media ${query}`); at >= 0; at = css.indexOf(`@media ${query}`, at + 1)) {
    const open = css.indexOf('{', at);
    let depth = 0;
    for (let i = open; i < css.length; i++) {
      if (css[i] === '{') depth++;
      else if (css[i] === '}' && --depth === 0) { out.push(css.slice(open + 1, i)); break; }
    }
  }
  return out;
}
// The phone block that holds the 44 px touch-target rule (modern.css has more than one block with this query).
export function touchBlock(css) {
  return mediaBlocks(css, PHONE_QUERY).find((b) => /\bsummary\b[^{}]*\{[^{}]*min-height:\s*44px/.test(b)) ?? null;
}

// True when the block has a rule whose selector list includes a bare `summary` and whose body sets align-content: center.
export function centresSummaries(block) {
  const clean = block.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const m of clean.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const sels = m[1].split(',').map((s) => s.trim());
    if (sels.includes('summary') && /(^|;)\s*align-content\s*:\s*center\s*(;|$)/.test(m[2].trim())) return true;
  }
  return false;
}

const PHONE_CENTRE = '[@media(max-width:767px)_and_(pointer:coarse)]:items-center';
// The className of a <summary> tag that starts at `text` (the tag's first few lines), or null.
export function summaryClass(text) {
  const tag = text.slice(0, text.search(/>\s*($|[^=])/) + 1 || 600);
  const m = tag.match(/className=(?:"([^"]*)"|\{`([^`]*)`\}|\{'([^']*)'\}|\{"([^"]*)"\})/);
  return m ? { cls: m[1] ?? m[2] ?? m[3] ?? m[4], compact: /\bdata-compact\b/.test(tag) } : null;
}
// A problem string, or null when the summary is fine.
export function flexSummaryProblem(text) {
  const s = summaryClass(text);
  if (!s) return null;
  const tokens = s.cls.split(/\s+/);
  if (!tokens.some((t) => t === 'flex' || t === 'inline-flex')) return null;
  if (s.compact || tokens.includes('items-center') || tokens.includes(PHONE_CENTRE)) return null;
  return `flex <summary> without items-center: on a touch phone it is 44 px tall and its title sticks to the top. Add ${PHONE_CENTRE} (or items-center).`;
}

const walk = (d) =>
  readdirSync(d).flatMap((n) => {
    const p = join(d, n);
    return statSync(p).isDirectory() ? (n === 'generated' ? [] : walk(p)) : /\.(tsx|ts)$/.test(n) ? [p] : [];
  });

test('the phone block centres a 44 px summary (align-content: center)', () => {
  const css = readFileSync(join(src, 'styles/modern.css'), 'utf8');
  const block = touchBlock(css);
  assert.ok(block, `modern.css has no @media ${PHONE_QUERY} block with the 44 px summary rule`);
  assert.ok(centresSummaries(block), `@media ${PHONE_QUERY} in modern.css must set summary { align-content: center; } so a fold's title sits in the middle of its 44 px row`);
});

test('every flex <summary> centres its items on a touch phone', () => {
  const hits = [];
  for (const f of walk(src)) {
    const text = readFileSync(f, 'utf8');
    for (const m of text.matchAll(/<summary\b/g)) {
      const problem = flexSummaryProblem(text.slice(m.index, m.index + 800).split('\n').slice(0, 6).join('\n'));
      if (problem) hits.push(`${relative(root, f)}:${text.slice(0, m.index).split('\n').length}: ${problem}`);
    }
  }
  assert.deepEqual(hits, []);
});

test('unit: the helpers', () => {
  const ok = '@media (max-width: 767px) and (pointer: coarse) {\n  summary { min-height: 44px; }\n  /* x */\n  summary {\n    align-content: center;\n  }\n}';
  assert.equal(centresSummaries(touchBlock(ok)), true);
  assert.equal(centresSummaries(touchBlock(ok.replace('align-content: center', 'align-content: start'))), false);
  assert.equal(centresSummaries('details summary { align-content: center; }'), false);
  assert.equal(touchBlock('a{}'), null);
  assert.equal(mediaBlocks(`@media ${PHONE_QUERY} { a { b: c; } }\n@media ${PHONE_QUERY} { d { e: f; } }`, PHONE_QUERY).length, 2);
  assert.ok(flexSummaryProblem('<summary className="flex items-start gap-2">'));
  assert.ok(flexSummaryProblem('<summary className={`inline-flex gap-1 ${x}`}>'));
  assert.equal(flexSummaryProblem('<summary className="flex items-center gap-2">'), null);
  assert.equal(flexSummaryProblem(`<summary className="flex items-start ${PHONE_CENTRE} gap-2">`), null);
  assert.equal(flexSummaryProblem('<summary data-compact className="flex items-start">'), null);
  assert.equal(flexSummaryProblem('<summary className="cursor-pointer flex-1">'), null);
  assert.equal(flexSummaryProblem('<summary className="cursor-pointer">Details</summary>'), null);
  assert.equal(flexSummaryProblem('<summary>Details</summary>'), null);
});
