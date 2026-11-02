// Registration: each of Iris's 7 types is wired into REGISTRY with its own validator and component, and
// resolveBlock's generic rules apply to them correctly — a table/list type needs a same-shaped reply, a
// slot type (verification, files) only ever resolves without one. Pixel's own smartResults.test.mjs
// already covers resolveBlock/REGISTRY end to end for every fixture; this file checks the wiring itself.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { FIXTURES } from './src/components/results/fixtures';
  export { REGISTRY, resolveBlock, defaultMatches } from './src/components/results/registry';
  export { Chart, validChart } from './src/components/results/Chart';
  export { Metrics, validMetrics } from './src/components/results/Metrics';
  export { Facts, validFacts } from './src/components/results/Facts';
  export { Cards, validCards } from './src/components/results/Cards';
  export { Findings, validFindings } from './src/components/results/Findings';
  export { Verification, validVerification } from './src/components/results/Verification';
  export { Files, validFiles } from './src/components/results/Files';
`);

const block = (name) => m.FIXTURES[name].results.blocks[0];
const parts = (n) => Array.from({ length: n }, (_, i) => ({ node: String(i), text: `item ${i}` }));
const tableReply = (rows) => ({ kind: 'table', head: parts(1), body: Array.from({ length: rows }, () => parts(1)) });
const listReply = (items) => ({ kind: 'list', items: parts(items), start: 1, ordered: false, task: false });

test('every type Iris owns is registered with its own validator and component, nothing swapped', () => {
  const owned = [
    ['chart', m.validChart, m.Chart],
    ['metrics', m.validMetrics, m.Metrics],
    ['facts', m.validFacts, m.Facts],
    ['cards', m.validCards, m.Cards],
    ['findings', m.validFindings, m.Findings],
    ['verification', m.validVerification, m.Verification],
    ['files', m.validFiles, m.Files],
  ];
  for (const [type, validate, Component] of owned) {
    const entry = m.REGISTRY[type];
    assert.ok(entry, `${type} is registered`);
    assert.equal(entry.validate, validate, `${type} uses its own validator, not a copy or another type's`);
    assert.equal(entry.Component, Component, `${type} renders its own component`);
  }
});

test('resolveBlock resolves a table-sourced type only against a reply table of the same row count', () => {
  const b = block('chartLine');
  assert.ok(m.resolveBlock(b, tableReply(b.points.length)), 'same row count resolves');
  assert.equal(m.resolveBlock(b, tableReply(b.points.length - 1)), null, 'a short reply table is rejected, never padded or guessed');
  assert.equal(m.resolveBlock(b, listReply(b.points.length)), null, 'a table-sourced block never matches a list reply');
});

test('resolveBlock resolves a list-sourced type only against a reply list of the same item count', () => {
  const b = block('cards');
  assert.ok(m.resolveBlock(b, listReply(b.items.length)), 'same item count resolves');
  assert.equal(m.resolveBlock(b, listReply(b.items.length + 1)), null, 'an extra reply item is rejected');
});

test('a slot-only type (verification, files) resolves with no reply, and never against one, even a same-shaped one', () => {
  const verification = block('verification');
  const files = block('files');
  assert.ok(m.resolveBlock(verification, null), 'verification stands alone, after the reply');
  assert.ok(m.resolveBlock(files, null), 'files stands alone, after the reply');
  assert.equal(m.resolveBlock(verification, listReply(verification.checks.length)), null, 'a slot block is never placed into the reply body');
  assert.equal(m.resolveBlock(files, listReply(files.items.length)), null, 'a slot block is never placed into the reply body');
});

test('an unknown type, or a block its own validator rejects, resolves to null so the reply falls back to markdown', () => {
  const b = block('metrics');
  assert.equal(m.resolveBlock({ ...b, type: 'nonsense' }, null), null);
  assert.equal(m.resolveBlock({ ...b, items: b.items.slice(0, 1) }, listReply(1)), null, 'below the 2-item minimum, validMetrics itself rejects it');
});
