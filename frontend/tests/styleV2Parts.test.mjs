// Dev mode style v2 (Mira, style-v2.md §2): the shared parts every Dev card uses (LED, diffstat, sha chip)
// and the shared .dv-* block in modern.css. Demo data only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export * from './src/dev/logic';
  export { Led, DiffStat, Sha } from './src/components/dev/parts';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const html = (C, props) => m.renderToStaticMarkup(m.createElement(C, props));

test('diffBar: 31 px split by share, 3 px floor either side, whole bar when one side is zero', () => {
  assert.deepEqual(m.diffBar(157, 15), { add: 28, del: 3 });
  assert.deepEqual(m.diffBar(10, 10), { add: 16, del: 15 });
  assert.deepEqual(m.diffBar(1, 500), { add: 3, del: 28 }, 'a > 0 keeps 3 px');
  assert.deepEqual(m.diffBar(52, 0), { add: 32, del: 0 });
  assert.deepEqual(m.diffBar(0, 4), { add: 0, del: 32 });
  assert.deepEqual(m.diffBar(0, 0), { add: 0, del: 0 });
  for (const [a, d] of [[3, 7], [64, 12], [1, 1], [999, 1]]) {
    const b = m.diffBar(a, d);
    assert.equal(b.add + b.del, 31, `${a}/${d}: 31 px plus the 1 px gap`);
  }
});

test('DiffStat: mono numbers with U+2212, the bar, and one label for screen readers', () => {
  const out = html(m.DiffStat, { add: 157, del: 15 });
  assert.match(out, /^<span role="img" aria-label="157 lines added, 15 removed"/);
  assert.match(out, /<span class="a">\+157<\/span> <span class="d">−15<\/span>/);
  assert.match(out, /<span aria-hidden="true" class="dv-bar"><i class="a" style="width:28px"><\/i><i class="d" style="width:3px"><\/i><\/span>/);
  assert.equal(m.diffLabel(1, 0), '1 line added, 0 removed');
});

test('Led is decoration with a state; Sha is 7 chars and says Commit to screen readers', () => {
  assert.equal(html(m.Led, { s: 'run' }), '<span aria-hidden="true" data-s="run" class="dv-led"></span>');
  assert.equal(html(m.Sha, { sha: '9f3c2a1e44b0' }), '<span data-sha="true" class="dv-sha"><span class="sr-only">Commit </span>9f3c2a1</span>');
});

test('the shared .dv-* block is in modern.css: tokens, LED, stat, sha, rail, stamp, and a reduced-motion rule', () => {
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  for (const sel of ['--dv-code-bg', '--dv-led-run', '--dv-rail', '--dv-stamp', '.dv-led[data-s="run"]::after', '.dv-stat', '.dv-bar', '.dv-sha', '.dv-branch', '.dv-node[data-s="bad"]::after', '.dv-arc', '.dv-stamp', '@keyframes dv-ping']) {
    assert.ok(css.includes(sel), sel);
  }
  const rm = css.slice(css.lastIndexOf('@media (prefers-reduced-motion: reduce)'));
  assert.match(rm, /\.dv-led::after, \.dv-led\[data-ping\]::after, \.dv-node::before, \.dv-caret, \.dv-stamp \{ animation: none !important; \}/);
});
