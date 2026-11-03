// A table cell's 200-character cap counts what the reader sees: a markdown link `[text](url)` counts
// as its text, a bare URL in full, and a cell holds at most 1,000 characters raw. The server's
// `cell_fits` / `shown_len` (src/server/smart_results/validate.rs) count the same way; its test
// `shown_length_counts_as_the_client_counts` (selector_tests.rs) holds the same cases as SHOWN_CASES.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { shownLength, isCell, MAX_CELL, MAX_CELL_RAW, validComparison } from './src/smartResults';
  export { validChart } from './src/components/results/Chart';
  export { FIXTURES } from './src/components/results/fixtures';
`);

const SHOWN_CASES = [
  ['plain', 5],
  ['[Aldi](https://example.com/very/long/path)', 4],
  ['![logo](https://example.com/l.png) text', 9],
  ['see https://example.com/a', 25],
  ['[a [b](u)', 4],
  ['[a] (u)', 7],
  ['[x](u', 5],
  ['!x [y](z)', 4],
  ['[](https://example.com)', 0],
  ['😀[😀](https://example.com)', 4],
  ['[a](b) and [c](d)', 7],
  ['[a](b(c)d)', 3],
];

test('shownLength counts a link as its text, as the server does', () => {
  for (const [s, n] of SHOWN_CASES) assert.equal(m.shownLength(s), n, s);
});

// Oleks's "Compare Aldi with Lidl": two cells were over 200 raw only because of their source links.
const prices =
  'Basket of 20 staples €42.07, about 4% less than Lidl in the latest test; private labels decide most of it ([Stiftung Warentest 2025](https://www.test.de/Discounter-Preisvergleich-Aldi-Lidl-Penny-Netto-5912345-0/?utm_source=chat))';
const app =
  'Lidl Plus: weekly coupons, digital receipts and a scratch card ([Lidl Plus](https://www.lidl.de/c/lidl-plus/s10007311?utm_source=chatgpt.com&utm_medium=referral&utm_campaign=lidl-plus-coupons-and-digital-receipts-2025))';

const aldi = (cell = prices) => ({
  id: 'b-aldi',
  type: 'comparison',
  title: null,
  source: { kind: 'markdown_table', index: 0 },
  provenance: { kind: 'reply_text' },
  columns: [
    { key: 'c0', label: 'Point', numeric: false, unit: null, role: 'label' },
    { key: 'c1', label: 'Aldi Süd', numeric: false, unit: null, role: 'value' },
    { key: 'c2', label: 'Lidl', numeric: false, unit: null, role: 'value' },
  ],
  rows: [
    { id: 'r0', cells: ['Prices', cell, 'Basket €44.02'] },
    { id: 'r1', cells: ['Deals', 'Tighter range', 'More non-food deals'] },
    { id: 'r2', cells: ['App', 'Nothing comparable', app] },
    { id: 'r3', cells: ['Store feel', 'Quicker to shop', 'Bigger, more variety'] },
  ],
  highlight_row: null,
  highlight_reason: null,
});

test('the Aldi table with long source links is a comparison the client keeps', () => {
  assert.ok(prices.length > 200 && app.length > 200, 'over 200 raw');
  assert.ok(m.shownLength(prices) < 200 && m.shownLength(app) < 200, 'under 200 shown');
  const c = m.validComparison(aldi());
  assert.ok(c, 'kept');
  assert.equal(c.rows[0].cells[1], prices, 'the cell keeps its link');
});

test('a cell showing more than 200 characters is still refused', () => {
  const url = `https://example.com/${'p'.repeat(300)}`;
  assert.ok(m.isCell(`${'x'.repeat(197)} [ab](${url})`), '200 shown');
  assert.equal(m.isCell(`${'x'.repeat(197)} [abc](${url})`), false, '201 shown');
  assert.equal(m.validComparison(aldi(`${'x'.repeat(197)} [abc](${url})`)), null, 'a comparison with one');
  assert.equal(m.isCell(`${'x'.repeat(10)} ${url}`), false, 'a bare URL counts in full');
});

test('a cell holds at most 1,000 characters raw', () => {
  assert.equal(m.MAX_CELL, 200);
  assert.equal(m.MAX_CELL_RAW, 1000);
  const cell = (n) => `xxx [ab](https://example.com/${'p'.repeat(n)})`;
  assert.equal(cell(970).length, 1000);
  assert.ok(m.isCell(cell(970)), '1,000 raw');
  assert.equal(m.isCell(cell(971)), false, '1,001 raw');
  assert.equal(m.validComparison(aldi(cell(971))), null, 'a comparison with one');
});

test('chart cells and labels count the same way', () => {
  const b = m.FIXTURES.chartLine.results.blocks[0];
  assert.ok(m.validChart(b));
  const link = `[Jul 2026](https://example.com/${'p'.repeat(300)})`;
  assert.ok(m.validChart({ ...b, points: b.points.map((p, i) => (i ? p : { ...p, x: link })) }), 'a linked label');
  assert.equal(m.validChart({ ...b, points: b.points.map((p, i) => (i ? p : { ...p, x: 'x'.repeat(201) })) }), null, '201 shown');
});
