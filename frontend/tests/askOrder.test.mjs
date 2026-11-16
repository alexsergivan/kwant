// 15e: the composer's "Ask:" row in a team chat starts with the team's lead, then the rest in roster order.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const { askOrder } = await loadBundled(`export * from './src/askOrder';`);
const ids = (xs) => xs.map((x) => x.id);
const roster = [{ id: 'a' }, { id: 'b' }, { id: 'lead' }, { id: 'c' }];

test('the lead comes first, everyone else keeps roster order', () => {
  assert.deepEqual(ids(askOrder(roster, 'lead')), ['lead', 'a', 'b', 'c']);
});

test('a lead already first, no lead, or a lead not in the team leaves the order alone', () => {
  assert.deepEqual(ids(askOrder([{ id: 'lead' }, { id: 'a' }], 'lead')), ['lead', 'a']);
  assert.deepEqual(ids(askOrder(roster, null)), ['a', 'b', 'lead', 'c']);
  assert.deepEqual(ids(askOrder(roster, undefined)), ['a', 'b', 'lead', 'c']);
  assert.deepEqual(ids(askOrder(roster, 'gone')), ['a', 'b', 'lead', 'c']);
  assert.deepEqual(askOrder([], 'lead'), []);
});

test('never drops, repeats or mutates the members', () => {
  const before = ids(roster);
  const out = askOrder(roster, 'c');
  assert.deepEqual(ids(out), ['c', 'a', 'b', 'lead']);
  assert.equal(new Set(ids(out)).size, roster.length);
  assert.deepEqual(ids(roster), before, 'input untouched');
  assert.notEqual(askOrder(roster, null), roster, 'always a new array');
});
