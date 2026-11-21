// Chart: a hand-rolled SVG, line for a time axis or bars for categories, up to 4 series told apart by
// shape/pattern as well as colour. The Chart/Table toggle must show the same data either way, every point
// is keyboard-reachable with a value tooltip, and a null value renders as a gap, never a zero.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { FIXTURES } from './src/components/results/fixtures';
  export { Chart, validChart } from './src/components/results/Chart';
`);
const { React, createRoot } = m;
const h = React.createElement;
const noop = () => {};

const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
});
async function mount(block) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  mounted.add({ done: () => { root.unmount(); host.remove(); } });
  root.render(h(m.Chart, { block, messageId: 'm1', reply: null, ask: noop }));
  await settle();
  return host;
}
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const focus = async (el) => {
  el.dispatchEvent(new FocusEvent('focusin', { bubbles: true }));
  await settle();
};
const blur = async (el) => {
  el.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
  await settle();
};

const lineBlock = () => m.FIXTURES.chartLine.results.blocks[0];
const barBlock = () => m.FIXTURES.chartBar.results.blocks[0];

test('role=img with a title and description built from the block, never a computed claim', async () => {
  const host = await mount(lineBlock());
  const svg = host.querySelector('svg[role="img"]');
  assert.ok(svg);
  assert.equal(svg.querySelector('title').textContent, 'Signups by month');
  assert.equal(svg.querySelector('desc').textContent, 'October had 2,260 signups and 455 paid.');
  assert.match(host.textContent, /October had 2,260 signups and 455 paid\./, 'the summary is also printed under the chart');
});

test('a chart with no title or summary still describes itself, never a computed claim', async () => {
  const host = await mount(barBlock());
  const svg = host.querySelector('svg[role="img"]');
  assert.equal(svg.querySelector('title').textContent, 'Bar chart');
  assert.equal(svg.querySelector('desc').textContent, 'Channel, Visits');
});

test('Chart/Table toggle shows the same data either way', async () => {
  const host = await mount(lineBlock());
  const [chartBtn, tableBtn] = [...host.querySelectorAll('button')];
  assert.equal(chartBtn.getAttribute('aria-pressed'), 'true');
  await click(tableBtn);
  assert.equal(tableBtn.getAttribute('aria-pressed'), 'true');
  const table = host.querySelector('table');
  assert.ok(table, 'table view renders a real table, the source data');
  const rows = [...table.querySelectorAll('tbody tr')].map((tr) => [...tr.querySelectorAll('td')].map((td) => td.textContent));
  assert.deepEqual(rows[0], ['Jul 2026', '1,240', '310'], 'table cells are the reply’s own cells, not recomputed');
  assert.equal(host.querySelector('svg[role="img"]'), null, 'table view replaces the chart, not alongside it');
  await click(chartBtn);
  assert.ok(host.querySelector('svg[role="img"]'));
});

test('every point is keyboard-reachable and announces its value on focus, via aria-live', async () => {
  const host = await mount(lineBlock());
  const live = host.querySelector('[aria-live="polite"]');
  assert.match(live.textContent, /Tab through the chart/);
  const points = [...host.querySelectorAll('g[role="button"][tabindex="0"]')];
  assert.ok(points.length > 0, 'line points are focusable');
  await focus(points[0]);
  assert.match(live.textContent, /Signups at Jul 2026: 1,240/);
  await blur(points[0]);
  assert.match(live.textContent, /Tab through the chart/, 'the tooltip clears on blur');
});

test('bar points are also keyboard-reachable with a value label', async () => {
  const host = await mount(barBlock());
  const bars = [...host.querySelectorAll('rect[role="button"][tabindex="0"]')];
  assert.equal(bars.length, 5);
  assert.equal(bars[0].getAttribute('aria-label'), 'Visits at Search: 4,120');
});

test('a null value is a gap: no marker is drawn for it, and the line splits around it', async () => {
  const b = lineBlock();
  const withGap = m.validChart({ ...b, points: b.points.map((p, i) => (i === 1 ? { ...p, cells: ['', p.cells[1]], values: [null, p.values[1]] } : p)) });
  const host = await mount(withGap);
  const labels = [...host.querySelectorAll('g[role="button"][tabindex="0"]')].map((g) => g.getAttribute('aria-label'));
  assert.ok(!labels.some((l) => l.startsWith('Signups at Aug 2026')), 'no focusable point for the gapped value');
  const paths = host.querySelectorAll('path[fill="none"]');
  assert.ok(paths.length >= 2, 'the signups series line breaks into separate segments around the gap');
});

test('up to 4 series are told apart by marker shape as well as colour', async () => {
  const host = await mount(lineBlock());
  const legend = host.querySelectorAll('svg[aria-hidden="true"]');
  assert.equal(legend.length, 2, 'one legend swatch per series');
  assert.notEqual(legend[0].innerHTML, legend[1].innerHTML, 'each series gets a distinct marker shape, not colour alone');
});

test('a bar chart needs 5 rows; a smaller one falls back to markdown', () => {
  const b = barBlock();
  assert.ok(m.validChart(b));
  assert.equal(m.validChart({ ...b, points: b.points.slice(0, 4) }), null);
});

test('series in different units abstain: one shared Y axis would flatten the smaller one (SR-R-13)', () => {
  const b = lineBlock();
  assert.equal(b.series.length, 2);
  assert.ok(m.validChart(b), 'the fixture (both unitless) still draws');
  const withUnits = (u0, u1) => ({ ...b, series: [{ ...b.series[0], unit: u0 }, { ...b.series[1], unit: u1 }] });
  assert.equal(m.validChart(withUnits('EUR', 'rating (1-5)')), null, 'EUR next to a rating');
  assert.equal(m.validChart(withUnits('EUR', null)), null, 'a unit next to none');
  assert.ok(m.validChart(withUnits('EUR', 'EUR')), 'the same unit on every series draws');
  const one = { ...b, series: [{ ...b.series[0], unit: 'EUR' }], points: b.points.map((p) => ({ ...p, cells: [p.cells[0]], values: [p.values[0]] })) };
  assert.ok(m.validChart(one), 'a single series has nothing to differ from');
});

test('a value too large to scale abstains instead of drawing NaN coordinates (SR-R-24)', async () => {
  const b = lineBlock();
  const put = (v) => ({ ...b, points: b.points.map((p, i) => (i === 0 ? { ...p, values: [v, p.values[1]] } : p)) });
  assert.equal(m.validChart(put(1e308)), null);
  assert.equal(m.validChart(put(-1e308)), null);
  assert.equal(m.validChart(put(Infinity)), null);
  assert.equal(m.validChart(put(NaN)), null);
  assert.ok(m.validChart(put(1e12)), 'large but sane values still draw');
  const bar = barBlock();
  assert.equal(m.validChart({ ...bar, points: bar.points.map((p, i) => (i === 0 ? { ...p, values: p.values.map(() => 1e308) } : p)) }), null);
});

for (const width of [300, 270]) {
  test(`every bar category is named at a ${width} px chart width (390 and 360 px phones), shortened to fit its band`, async () => {
    const real = Object.getOwnPropertyDescriptor(dom.window.HTMLElement.prototype, 'clientWidth');
    Object.defineProperty(dom.window.HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => width });
    // The chart measures its box through a ResizeObserver; jsdom has none, so a no-op one lets the first measure run.
    globalThis.ResizeObserver = class { observe() {} disconnect() {} };
    try {
      const host = await mount(barBlock());
      const labels = [...host.querySelectorAll('svg[role="img"] text')].filter((t) => t.getAttribute('y') === '240');
      assert.equal(labels.length, barBlock().points.length, 'all 5 categories are labelled, not 3');
      // A shortened label keeps its full name in a <title> (hover); the drawn text is the last node.
      const drawn = labels.map((t) => t.lastChild.textContent);
      assert.ok(drawn.every((t) => t.length <= 7), 'shortened to fit its band: ' + drawn.join('|'));
      const cut = labels.filter((t) => t.querySelector('title'));
      assert.ok(cut.length > 0 && cut.every((t) => t.querySelector('title').textContent.length > t.lastChild.textContent.length), 'a shortened label keeps its full name');
    } finally {
      delete globalThis.ResizeObserver;
      if (real) Object.defineProperty(dom.window.HTMLElement.prototype, 'clientWidth', real);
      else delete dom.window.HTMLElement.prototype.clientWidth;
    }
  });
}

// 31m chart-last-label-fits: on a phone, a line chart's off-cadence last label printed right after the regular one
// ("2026-10-1926…"). Labels render at the card's measured width, so stub the measurement for narrow and wide.
async function xLabelsAt(block, width) {
  return (await xAxisAt(block, width)).at;
}
// The drawn axis texts and the point each names (a full date, or "MM-DD" once 31t drops a shared year).
async function xAxisAt(block, width) {
  const cw = Object.getOwnPropertyDescriptor(dom.window.HTMLElement.prototype, 'clientWidth');
  const hadRO = 'ResizeObserver' in globalThis;
  Object.defineProperty(dom.window.HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => width });
  globalThis.ResizeObserver = class { observe() {} disconnect() {} };
  try {
    const host = await mount(block);
    const xs = block.points.map((p) => p.x);
    const texts = [...host.querySelectorAll('svg[role="img"] text.fill-zinc-400')].map((t) => t.textContent);
    const at = texts.map((t) => (xs.includes(t) ? xs.indexOf(t) : xs.findIndex((x) => x.slice(5) === t)));
    return { host, texts, at };
  } finally {
    if (cw) Object.defineProperty(dom.window.HTMLElement.prototype, 'clientWidth', cw);
    else delete dom.window.HTMLElement.prototype.clientWidth;
    if (!hadRO) delete globalThis.ResizeObserver;
  }
}
const weekly = (n, kind = 'line') => ({
  ...lineBlock(),
  id: `b-weekly-${kind}-${n}`,
  chart_kind: kind,
  x: { label: 'Week', kind: kind === 'line' ? 'time' : 'category' },
  series: [{ key: 'orders', label: 'Orders', unit: null }],
  points: Array.from({ length: n }, (_, i) => {
    const d = new Date(Date.UTC(2026, 8, 7 + 7 * i)).toISOString().slice(0, 10);
    return { id: `w${i}`, x: d, cells: [String(10 + i)], values: [10 + i] };
  }),
});

test('a line chart labels the last point and never prints two dates on top of each other (31m)', async () => {
  const expected = {
    narrow: { 5: [0, 2, 4], 7: [0, 2, 4, 6], 8: [0, 2, 4, 7], 9: [0, 3, 8], 12: [0, 3, 6, 11] },
    // 32b: at 640 px a 7 to 9 point line labels every point, so the end-anchored last date crowds the one before it,
    // which goes.
    wide: { 5: [0, 1, 2, 3, 4], 7: [0, 1, 2, 3, 4, 6], 8: [0, 1, 2, 3, 4, 5, 7], 9: [0, 1, 2, 3, 4, 5, 6, 8], 12: [0, 2, 4, 6, 8, 11] },
  };
  for (const [size, width] of [['narrow', 358], ['wide', 640]]) {
    for (const n of [5, 7, 8, 9, 12]) {
      const at = await xLabelsAt(weekly(n), width);
      const what = `${size} n=${n}: ${at.join(',')}`;
      assert.deepEqual(at, expected[size][n], what);
      assert.equal(at.at(-1), n - 1, `last point labelled, ${what}`);
      assert.ok(!at.includes(-1), `every label names a whole point, ${what}`);
      // Thinned labels keep at least 2 steps between neighbours; unthinned (wide, n <= 10) keep every point but,
      // when crowded, the one before the last (32b).
      const minGap = Math.min(...at.slice(1).map((v, k) => v - at[k]));
      if (size === 'narrow' || n > 10) assert.ok(minGap >= 2, `neighbours >= 2 steps apart, ${what}`);
      else assert.ok(at.length >= n - 1 && at.every((v, k) => v === k || k === at.length - 1), `wide n<=10 labels every point up to the last, ${what}`);
    }
  }
});

test('bar labels stay as they were: 5 to 9 bars at a phone width name every bar (31m)', async () => {
  for (const n of [5, 7, 8, 9]) {
    const at = await xLabelsAt(weekly(n, 'bar'), 358);
    assert.equal(at.length, n, `n=${n}`);
  }
});

// 31t chart-axis-drops-same-year: on a phone, ten-character ISO dates sat edge to edge. When every date shares a year,
// the axis prints "09-07"; the year stays everywhere else, and nothing else about the axis changes.
test('a phone line chart drops a shared year from its date labels, and only there (31t)', async () => {
  const mmdd = (b, at) => at.map((i) => b.points[i].x.slice(5));
  const same = weekly(8);
  const narrow = await xAxisAt(same, 358);
  assert.deepEqual(narrow.texts, ['09-07', '09-21', '10-05', '10-26'], 'narrow, same year: MM-DD');
  assert.deepEqual(narrow.at, [0, 2, 4, 7], 'cadence from 31m unchanged');
  for (const n of [5, 9, 12]) {
    const r = await xAxisAt(weekly(n), 358);
    assert.deepEqual(r.texts, mmdd(weekly(n), r.at), `narrow n=${n} prints MM-DD`);
  }
  // Everything else keeps the full date.
  const wide = await xAxisAt(same, 640);
  assert.deepEqual(wide.texts, wide.at.map((i) => same.points[i].x), 'wide: full dates (32b may drop the one before the last)');
  const mixed = { ...weekly(8), points: weekly(8).points.map((p, i) => ({ ...p, x: i < 6 ? p.x : `2027-01-0${i - 5}` })) };
  const mx = await xAxisAt(mixed, 358);
  assert.ok(mx.texts.length && mx.texts.every((t) => /^\d{4}-\d{2}-\d{2}$/.test(t)), 'mixed years: full dates ' + mx.texts.join('|'));
  const months = { ...weekly(8), points: weekly(8).points.map((p, i) => ({ ...p, x: ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug'][i] })) };
  assert.deepEqual((await xAxisAt(months, 358)).texts, ['Jan', 'Mar', 'May', 'Aug'], 'non-date x: unchanged');
  const notDay = { ...weekly(8), points: weekly(8).points.map((p) => ({ ...p, x: p.x + 'T00:00' })) };
  assert.ok((await xAxisAt(notDay, 358)).texts.every((t) => t.startsWith('2026-')), 'only bare YYYY-MM-DD drops the year');
  // Bars keep their own labels (shortened to the band, never year-dropped).
  const bar = await xAxisAt(weekly(8, 'bar'), 358);
  assert.ok(bar.texts.length === 8 && bar.texts.every((t) => t.startsWith('2026-')), 'bar: year kept ' + bar.texts.join('|'));
});

test('with the year dropped from the axis, the tooltip, table and accessible names keep the full date (31t)', async () => {
  const block = weekly(8);
  const { host } = await xAxisAt(block, 358);
  const points = [...host.querySelectorAll('g[role="button"][tabindex="0"]')];
  assert.equal(points[0].getAttribute('aria-label'), 'Orders at 2026-09-07: 10');
  await focus(points[7]);
  assert.match(host.querySelector('[aria-live="polite"]').textContent, /^Orders at 2026-10-26: 17$/);
  await blur(points[7]);
  await click([...host.querySelectorAll('button')][1]);
  const firstCells = [...host.querySelectorAll('tbody tr td:first-child')].map((td) => td.textContent);
  assert.deepEqual(firstCells, block.points.map((p) => p.x), 'table keeps full dates');
});

// 32b chart-last-label-gap: on a wide line that labels every point, the end-anchored last date sat 5 px from the
// middle-anchored one before it (Rigel, 31t QA at 1280). With ~7 px digits at 11 px, keep at least 8 px between them:
// otherwise the label before the last goes. The first and last labels always stay.
const CHAR_W = 7;
function lastGap(block, width, at) {
  const plotW = width - 44 - 16;
  const step = plotW / (block.points.length - 1);
  const [prev, last] = at.slice(-2);
  const w = (i) => block.points[i].x.length * CHAR_W;
  return (44 + last * step - w(last)) - (44 + prev * step + w(prev) / 2);
}
test('a wide line chart keeps 8 px before its end-anchored last date, dropping the label before it (32b)', async () => {
  // Mira's 8-point weekly chart at 1280 (the plot is 786 px wide there).
  const eight = weekly(8);
  const r = await xAxisAt(eight, 786);
  assert.deepEqual(r.at, [0, 1, 2, 3, 4, 5, 7], '1280: the label before the last goes');
  assert.equal(r.at[0], 0, 'first kept');
  assert.equal(r.at.at(-1), 7, 'last kept');
  assert.deepEqual(r.texts, [0, 1, 2, 3, 4, 5, 7].map((i) => eight.points[i].x), 'full dates, untruncated');
  assert.ok(lastGap(eight, 786, r.at) >= 8, `last gap ${lastGap(eight, 786, r.at).toFixed(1)} px >= 8`);
  // A roomy chart keeps every label: 5 points at 786 px leave far more than 8 px.
  assert.deepEqual((await xAxisAt(weekly(5), 786)).at, [0, 1, 2, 3, 4], 'room enough: nothing dropped');
  // Never the first: with only two labels left, both stay even when close.
  const two = weekly(2);
  assert.deepEqual((await xAxisAt(two, 120)).at, [0, 1], 'two points: first and last both stay');
  // Phones (31m cadence, 31t MM-DD) are unchanged.
  assert.deepEqual((await xAxisAt(eight, 358)).texts, ['09-07', '09-21', '10-05', '10-26'], '390: unchanged');
  for (const [n, at] of [[5, [0, 2, 4]], [9, [0, 3, 8]], [12, [0, 3, 6, 11]]]) {
    assert.deepEqual((await xAxisAt(weekly(n), 358)).at, at, `390 n=${n}: unchanged`);
  }
  // Bars are middle-anchored and keep every label.
  assert.equal((await xAxisAt(weekly(8, 'bar'), 786)).at.length, 8, 'bars at 1280: every bar named');
  assert.equal((await xAxisAt(weekly(8, 'bar'), 358)).at.length, 8, 'bars at 390: every bar named');
});
