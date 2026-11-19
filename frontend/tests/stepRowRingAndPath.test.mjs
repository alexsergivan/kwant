// 29i (autonomy/step-row-ring-and-path/brief.md), two of Mira's calls on the same StepRow:
// (1) since 29e a touch-phone summary reaches its li's top and bottom edges, so its focus ring must sit inside
//     (-outline-offset-2, like the li's own ring), or the list's overflow-hidden clips it and it crosses an open
//     row's output.
// (2) a single-line path detail may break after each "/" (<wbr>), so the file name stays whole; every other detail
//     renders exactly as before, and the text a person copies is the detail itself.
// Every test here fails on fa47a67b (#827).
import assert from 'node:assert/strict';
import test from 'node:test';
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
const at = (min) => new Date(Date.parse('2026-11-18T09:00:00Z') + min * 60000).toISOString();
const step = (id, title, t, detail, extra = {}) => ({
  id, kind: 'tool', status: 'ok', title, tool: 'Read', started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000,
  ...(detail ? { detail } : {}), ...extra,
});
const render = (steps) =>
  m.saved({ activity: m.parseActivityMeta({ activity: { steps, dropped: 0 } }), idBase: 'r1', defaultOpen: true });

/** Each step row's summary class and the inner HTML of its detail <pre>, in order. */
function rows(html) {
  return [...html.matchAll(/<li\b[^>]*\bdata-step-status="[^"]*"[^>]*>([\s\S]*?)<\/li>/g)].map((li) => ({
    summary: /<summary\b[^>]*class="([^"]*)"/.exec(li[1])?.[1] ?? null,
    pre: /<pre\b[^>]*>([\s\S]*?)<\/pre>/.exec(li[1])?.[1] ?? null,
  }));
}
const ENT = { '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#x27;': "'", '&#39;': "'" };
/** The text a person copies from the pre: tags dropped (<wbr> adds no characters), entities decoded. */
const copied = (inner) => inner.replace(/<[^>]*>/g, '').replace(/&(?:amp|lt|gt|quot|#x27|#39);/g, (e) => ENT[e]);
const escaped = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#x27;');

const LONG = '/home/me/projects/spring-sale/drafts/spring-sale-plan.md';

test('a touch-phone step summary keeps its focus ring inside the row (fails on fa47a67b)', () => {
  const r = rows(render([step('a', 'Read plan', 0, LONG), step('b', 'Read b', 1)]));
  const toks = r[0].summary.split(/\s+/);
  assert.ok(toks.includes(`${PHONE}focus-visible:-outline-offset-2`), r[0].summary);
  // Desktop keeps its ring as it was: no unprefixed offset, and the 29e pair is untouched.
  assert.ok(!toks.some((t) => /^focus-visible:-?outline-offset/.test(t)), 'desktop ring unchanged');
  for (const t of ['focus-visible:outline', 'focus-visible:outline-2', 'focus-visible:outline-sky-400', `${PHONE}-my-1.5`, `${PHONE}py-1.5`])
    assert.ok(toks.includes(t), t);
});

test('a path detail can break after each separator, and copies as the same text (fails on fa47a67b)', () => {
  const paths = [
    [LONG, 6],
    ['~/notes/today.md', 2],
    ['./src/a b/c.ts', 3],
    ['../up/x.txt', 2],
    ['C:\\Users\\me\\Desk top\\plan.docx', 4],
    ['D:/work/plan.md', 2],
  ];
  const r = rows(render(paths.map(([p], i) => step(`p${i}`, `Read ${i}`, i, p))));
  paths.forEach(([p, seps], i) => {
    const inner = r[i].pre;
    assert.equal((inner.match(/<wbr\/?>/g) ?? []).length, seps, `${p}: ${inner}`);
    assert.equal(copied(inner), p, 'copied text is the detail');
    // Each break comes right after a separator, never inside a name.
    for (const piece of inner.split(/<wbr\/?>/).slice(0, -1)) assert.match(copied(piece), /[/\\]$/, `${p}: ${piece}`);
  });
  // The file name is one piece.
  assert.ok(r[0].pre.endsWith('spring-sale-plan.md'), r[0].pre);
});

test('every other detail renders exactly as before (fails on fa47a67b only through the path rows)', () => {
  const others = [
    'built in 4.1s',
    'https://example.com/a/b/c.html',
    'npm run build && cp dist/a.js /tmp/a.js',
    '/home/me/a.md\n/home/me/b.md',
    'Error: ENOENT: no such file, open /home/me/x.md',
    '/',
    'You said no: not this folder <yet>',
  ];
  const html = render([...others.map((d, i) => step(`o${i}`, `Step ${i}`, i, d)), step('p', 'Read', 9, LONG)]);
  const r = rows(html);
  others.forEach((d, i) => assert.equal(r[i].pre, escaped(d), JSON.stringify(d)));
  assert.match(r[others.length].pre, /<wbr\/?>/, 'and the path row in the same list does break');
  // The pre keeps overflow-wrap:anywhere as the fallback for a segment longer than the line.
  assert.match(html, /<pre\b[^>]*\[overflow-wrap:anywhere\]/);
});
