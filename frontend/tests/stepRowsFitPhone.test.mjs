// 29e (autonomy/step-rows-fit-phone/brief.md): on a touch phone a Details step row with a › was 56 px tall, not 44.
// modern.css gives every <summary> a 44 px min height on (max-width: 767px) and (pointer: coarse), and the row's <li>
// added its own py-1.5 on top. On that phone only, the summary now takes in the li's padding (-my-N py-N with the
// li's py-N), so the row is 44 px like the rows without ›, and the tap area stays 44 px. Desktop is unchanged.
// Tests marked "fails before 29e" fail on 5f12c6c2 + #791.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { parseActivityMeta } from './src/activity';
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const PHONE = '[@media(max-width:767px)_and_(pointer:coarse)]:';
const at = (min) => new Date(Date.parse('2026-11-18T07:00:00Z') + min * 60000).toISOString();
const tool = (id, title, t, detail) => ({ id, kind: 'tool', status: 'ok', title, tool: 'Bash', started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000, ...(detail ? { detail } : {}) });

/** Each step <li> in rendered markup: its class, and its <summary>'s class when it has details (else null). */
export function stepRows(html) {
  const out = [];
  for (const li of html.matchAll(/<li\b[^>]*\bdata-step-status="[^"]*"[^>]*>([\s\S]*?)<\/li>/g)) {
    const liCls = /class="([^"]*)"/.exec(li[0].slice(0, li[0].indexOf('>') + 1))?.[1] ?? '';
    const sum = /<summary\b[^>]*class="([^"]*)"/.exec(li[1]);
    out.push({ li: liCls, summary: sum ? sum[1] : null });
  }
  return out;
}
/** A problem string, or null when a row with details keeps to 44 px on a touch phone. */
export function rowProblem({ li, summary }) {
  if (summary === null) return null;
  const toks = summary.split(/\s+/);
  const py = li.split(/\s+/).find((t) => /^py-/.test(t));
  if (!py) return toks.some((t) => t.startsWith(`${PHONE}-my-`)) ? 'summary pulls in padding the li no longer has' : null;
  const n = py.slice(3);
  const want = [`${PHONE}-my-${n}`, `${PHONE}py-${n}`];
  const missing = want.filter((t) => !toks.includes(t));
  if (missing.length) return `li has ${py} but its summary lacks ${missing.join(' ')}: on a touch phone the row is 44 + the li's padding (56 px), not 44`;
  // Desktop and tablets keep the li's padding outside the summary: no unprefixed margin or padding on the summary.
  const bare = toks.filter((t) => /^-?(m|p)[ytb]?-/.test(t));
  return bare.length ? `summary has unprefixed ${bare.join(' ')}: desktop would change` : null;
}

const reply = () =>
  m.parseActivityMeta({
    activity: {
      steps: [
        tool('t1', 'Running `npm run build`', 0, 'built in 4.1s'),
        tool('t2', 'Read a.ts', 1),
        tool('t3', 'Running `npm test`', 2, '42 passing'),
        tool('t4', 'Edit a.ts', 3),
      ],
      dropped: 0,
    },
  });

test('a step row with details takes in the li padding on a touch phone (fails before 29e)', () => {
  const rows = stepRows(m.saved({ activity: reply(), idBase: 'r1', defaultOpen: true }));
  assert.equal(rows.length, 4);
  assert.equal(rows.filter((r) => r.summary !== null).length, 2, 'two rows with ›');
  for (const r of rows) assert.equal(rowProblem(r), null, `${r.li} | ${r.summary}`);
  const withDetails = rows.find((r) => r.summary !== null);
  assert.ok(withDetails.li.split(/\s+/).includes('py-1.5'), 'the li keeps py-1.5 (rows without › and desktop rely on it)');
  assert.ok(withDetails.summary.split(/\s+/).includes(`${PHONE}items-center`), "#791's centring stays");
});

test('the live folded row icons carry no max-sm: top margin (unreachable below 768 px, Rigel)', () => {
  const src = readFileSync(new URL('../src/components/ActivityList.tsx', import.meta.url), 'utf8');
  const at = src.indexOf('data-steps-folded=');
  assert.ok(at > 0);
  const button = src.slice(at, src.indexOf('</button>', at));
  assert.match(button, /<ListTree\b/);
  assert.match(button, /<ChevronRight\b/);
  assert.doesNotMatch(button, /max-sm:mt-\[1\.5px\]/);
});

test('unit: rowProblem', () => {
  const li = 'px-3 py-1.5 text-xs';
  assert.equal(rowProblem({ li, summary: null }), null);
  assert.ok(rowProblem({ li, summary: `flex items-start ${PHONE}items-center gap-2` }));
  assert.ok(rowProblem({ li, summary: `flex ${PHONE}-my-1.5 gap-2` }));
  assert.ok(rowProblem({ li, summary: `flex ${PHONE}-my-1 ${PHONE}py-1 gap-2` }), 'the pair must match the li');
  assert.ok(rowProblem({ li, summary: `flex ${PHONE}-my-1.5 ${PHONE}py-1.5 py-1 gap-2` }), 'desktop untouched');
  assert.equal(rowProblem({ li, summary: `flex ${PHONE}-my-1.5 ${PHONE}py-1.5 gap-2` }), null);
  assert.equal(rowProblem({ li: 'px-3 text-xs', summary: 'flex gap-2' }), null);
  assert.deepEqual(stepRows('<ul><li data-step-status="ok" class="py-1.5"><details><summary class="a b">x</summary></details></li><li data-step-status="ok" class="py-1.5"><div>y</div></li></ul>'), [
    { li: 'py-1.5', summary: 'a b' },
    { li: 'py-1.5', summary: null },
  ]);
});
