// Iris's 7 validators (chart, metrics, facts, cards, findings, verification, files): a good fixture
// always passes, and a block that breaks the plan §A.3 contract always comes back null (the client falls
// back to markdown rather than trust it). Each validator is built from the shared helpers in smartResults.ts.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { FIXTURES } from './src/components/results/fixtures';
  export { validChart } from './src/components/results/Chart';
  export { validMetrics } from './src/components/results/Metrics';
  export { validFacts } from './src/components/results/Facts';
  export { validCards } from './src/components/results/Cards';
  export { validFindings } from './src/components/results/Findings';
  export { validVerification } from './src/components/results/Verification';
  export { validFiles } from './src/components/results/Files';
  export { blockRows, blockCsv } from './src/components/results/exporters';
  export { previewNote } from './src/smartResults';
`);

const block = (name) => m.FIXTURES[name].results.blocks[0];

test('every fixture of a type this file owns passes its own validator', () => {
  assert.ok(m.validChart(block('chartLine')), 'line chart');
  assert.ok(m.validChart(block('chartBar')), 'bar chart');
  assert.ok(m.validMetrics(block('metrics')));
  assert.ok(m.validFacts(block('facts')));
  assert.ok(m.validCards(block('cards')));
  assert.ok(m.validFindings(block('findings')));
  assert.ok(m.validVerification(block('verification')));
  assert.ok(m.validFiles(block('files')));
});

test('chart: series count, point/series alignment, enum and null values are checked', () => {
  const b = block('chartLine');
  assert.equal(m.validChart({ ...b, chart_kind: 'pie' }), null, 'chart_kind is a fixed enum');
  assert.equal(m.validChart({ ...b, series: [] }), null, 'at least one series');
  assert.equal(m.validChart({ ...b, series: Array(5).fill(b.series[0]).map((s, i) => ({ ...s, key: `s${i}` })) }), null, 'at most 4 series');
  assert.equal(m.validChart({ ...b, series: [b.series[0], { ...b.series[0] }] }), null, 'series keys must be unique');
  assert.equal(
    m.validChart({ ...b, points: b.points.map((p) => ({ ...p, cells: [p.cells[0]] })) }),
    null,
    'a point needs one cell per series',
  );
  assert.equal(
    m.validChart({ ...b, points: b.points.map((p) => ({ ...p, values: [p.values[0]] })) }),
    null,
    'a point needs one value per series',
  );
  const withGap = m.validChart({ ...b, points: b.points.map((p, i) => (i === 1 ? { ...p, values: [null, null] } : p)) });
  assert.ok(withGap, 'a null value is a gap, not a rejection');
  assert.equal(withGap.points[1].values[0], null);
  assert.equal(m.validChart({ ...b, points: b.points.slice(0, 2) }), null, 'at least 3 points');
});

test('metrics: 2-6 items, a real number, and HTML in any text is rejected', () => {
  const b = block('metrics');
  assert.equal(m.validMetrics({ ...b, items: b.items.slice(0, 1) }), null, 'at least 2 items');
  assert.equal(m.validMetrics({ ...b, items: [...b.items, ...b.items, ...b.items] }), null, 'at most 6 items');
  assert.equal(m.validMetrics({ ...b, items: [{ ...b.items[0], value: 'eighteen' }, b.items[1]] }), null, 'value must be a number');
  assert.equal(m.validMetrics({ ...b, items: [{ ...b.items[0], label: '<b>Revenue</b>' }, b.items[1]] }), null, 'no HTML in label');
});

test('facts: 3-12 items, and a value may be blank but never HTML', () => {
  const b = block('facts');
  assert.equal(m.validFacts({ ...b, items: b.items.slice(0, 2) }), null, 'at least 3 items');
  const blank = m.validFacts({ ...b, items: b.items.map((i) => ({ ...i, value: '' })) });
  assert.ok(blank, 'a blank value is allowed');
  assert.equal(m.validFacts({ ...b, items: b.items.map((i) => ({ ...i, value: '<script>1</script>' })) }), null);
});

test('cards: url is http(s) or absent, never javascript: or a relative path', () => {
  const b = block('cards');
  assert.equal(m.validCards({ ...b, items: b.items.slice(0, 1) }), null, 'at least 3 items');
  assert.equal(m.validCards({ ...b, items: [{ ...b.items[0], url: 'javascript:alert(1)' }, b.items[1], b.items[2]] }), null);
  assert.equal(m.validCards({ ...b, items: [{ ...b.items[0], url: '/relative' }, b.items[1], b.items[2]] }), null);
  const noLink = m.validCards({ ...b, items: [{ ...b.items[0], url: null }, b.items[1], b.items[2]] });
  assert.ok(noLink, 'a card may have no link');
  assert.equal(noLink.items[0].url, null);
});

test('findings: severity is one of the fixed enum values, refs name a path', () => {
  const b = block('findings');
  assert.equal(m.validFindings({ ...b, items: b.items.slice(0, 1) }), null, 'at least 2 items');
  assert.equal(m.validFindings({ ...b, items: [{ ...b.items[0], severity: 'urgent' }, b.items[1]] }), null, 'severity is a fixed enum');
  assert.equal(
    m.validFindings({ ...b, items: [{ ...b.items[0], refs: [{ path: 'a.sh', line: -1 }] }, b.items[1]] }),
    null,
    'a ref line is a non-negative integer or null',
  );
  const nullLine = m.validFindings({ ...b, items: [{ ...b.items[0], refs: [{ path: 'a.sh', line: null }] }, b.items[1]] });
  assert.ok(nullLine, 'a ref may have no line');
});

test('verification: status is a fixed enum, target is a task, source is always slot', () => {
  const b = block('verification');
  assert.equal(m.validVerification({ ...b, source: { kind: 'markdown_table', index: 0 } }), null, 'source is always slot');
  assert.equal(m.validVerification({ ...b, target: { kind: 'file', id: 'x', title: 'x' } }), null, 'target is always a task');
  assert.equal(m.validVerification({ ...b, checks: [{ ...b.checks[0], status: 'skipped' }] }), null, 'status is a fixed enum');
  assert.equal(m.validVerification({ ...b, checks: [] }), null, 'at least 1 check');
});

test('files: size is a non-negative integer, preview kinds are checked, source is always slot', () => {
  const b = block('files');
  assert.equal(m.validFiles({ ...b, source: { kind: 'markdown_list', index: 0 } }), null, 'source is always slot');
  assert.equal(m.validFiles({ ...b, items: [] }), null, 'at least 1 file');
  assert.equal(m.validFiles({ ...b, items: [{ ...b.items[0], size_bytes: -1 }] }), null, 'size is non-negative');
  assert.equal(m.validFiles({ ...b, items: [{ ...b.items[0], size_bytes: 1.5 }] }), null, 'size is an integer');
  assert.equal(
    m.validFiles({ ...b, items: [{ ...b.items[0], preview: { kind: 'table', columns: ['a', 'b'], rows: [['x']] } }] }),
    null,
    'a table preview row must match the column count',
  );
  const noPreview = m.validFiles({ ...b, items: [{ ...b.items[0], preview: null }] });
  assert.ok(noPreview, 'a file may have no preview');
});

test('files: cut-preview fields are optional, non-negative integers, and truncated is only ever true', () => {
  const b = block('files');
  const table = (extra) => ({ ...b, items: [{ ...b.items[0], preview: { kind: 'table', columns: ['a'], rows: [['x']], ...extra } }] });
  const text = (extra) => ({ ...b, items: [{ ...b.items[0], preview: { kind: 'text', excerpt: 'x', ...extra } }] });
  assert.ok(m.validFiles(table({})), 'a complete preview needs none of them');
  assert.equal(m.validFiles(table({ total_rows: 143, total_cols: 9 })).items[0].preview.total_rows, 143);
  assert.equal(m.validFiles(text({ total_lines: 512 })).items[0].preview.total_lines, 512);
  assert.equal(m.validFiles(text({ truncated: true })).items[0].preview.truncated, true);
  assert.equal(m.validFiles(table({ total_rows: -1 })), null, 'negative total');
  assert.equal(m.validFiles(table({ total_rows: 1.5 })), null, 'fractional total');
  assert.equal(m.validFiles(table({ total_cols: '9' })), null, 'string total');
  assert.equal(m.validFiles(text({ total_lines: NaN })), null, 'NaN total');
  assert.equal(m.validFiles(text({ truncated: false })), null, 'truncated is only ever true');
  assert.equal(m.validFiles(text({ truncated: 'yes' })), null, 'truncated is a boolean');
});

test('files: Copy data on a cut preview says it copies the preview only; complete previews are unchanged', () => {
  const rows = m.blockRows(block('files'));
  const by = (n) => rows.find((r) => r.name === n);
  assert.deepEqual(Object.keys(by('redirects.csv')), ['name', 'type', 'bytes'], 'complete preview: no extra column');
  assert.deepEqual(Object.keys(by('dns.png')), ['name', 'type', 'bytes']);
  assert.match(by('urls-all.csv').preview, /^Showing 2 of 143 rows; Copy data copies the preview only$/);
  assert.match(by('crawl.log').preview, /Showing 3 of 512 lines/);
  assert.match(by('dump.txt').preview, /Preview, file is longer/);
  assert.match(m.blockCsv(block('files')), /copies the preview only/);
});

test('previewNote: counts the rows and lines drawn, and says nothing when the totals do not exceed them', () => {
  const t = (extra) => ({ kind: 'table', columns: ['a', 'b', 'c'], rows: [['1', '2', '3']], ...extra });
  assert.equal(m.previewNote(t({ total_rows: 20, total_cols: 12 })), 'Showing 1 of 20 rows, 3 of 12 columns');
  assert.equal(m.previewNote(t({ total_cols: 12 })), 'Showing 3 of 12 columns');
  assert.equal(m.previewNote(t({ total_rows: 1 })), 'Preview, file is longer', 'a total that is not larger than the rows drawn still marks a cut');
  assert.equal(m.previewNote(t({})), null);
  assert.equal(m.previewNote({ kind: 'text', excerpt: 'a\nb\n', total_lines: 9 }), 'Showing 2 of 9 lines', 'a trailing newline is not a line');
  assert.equal(m.previewNote({ kind: 'image' }), null);
  assert.equal(m.previewNote(null), null);
});
