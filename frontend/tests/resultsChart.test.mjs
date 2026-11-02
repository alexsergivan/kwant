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
