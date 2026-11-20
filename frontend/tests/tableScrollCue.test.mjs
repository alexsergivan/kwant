// 31p table-scroll-cue (Mira's UX pass at 0.9.32, "Also seen" a and c).
// (a) A reply's plain table that scrolls sideways shows a right-edge fade while there is more to the right;
//     it goes at the end, and a table that fits has none.
// (c) On the phone comparison card a one-item Pros or Cons gets its + or − like a multi-item one.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};

// jsdom has no layout: the scroller reports the box the test sets, and a ResizeObserver the test fires.
const box = { scrollWidth: 0, clientWidth: 0 };
const left = new WeakMap();
const proto = g.window.HTMLElement.prototype;
const scroller = (el) => el.classList.contains('overflow-x-auto');
Object.defineProperty(proto, 'scrollWidth', { configurable: true, get() { return scroller(this) ? box.scrollWidth : 0; } });
Object.defineProperty(proto, 'clientWidth', { configurable: true, get() { return scroller(this) ? box.clientWidth : 0; } });
Object.defineProperty(proto, 'scrollLeft', { configurable: true, get() { return left.get(this) ?? 0; }, set(v) { left.set(this, v); } });
const observers = new Set();
g.ResizeObserver = g.window.ResizeObserver = class {
  constructor(cb) { this.cb = cb; this.els = []; }
  observe(el) { this.els.push(el); observers.add(this); }
  disconnect() { observers.delete(this); }
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { act } from 'react';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { shortList } from './src/components/results/Comparison';
`);
const { React, createRoot } = m;
const h = React.createElement;
g.IS_REACT_ACT_ENVIRONMENT = false;

const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => { root.unmount(); host.remove(); } };
  mounted.add(v);
  return v;
}

const table = (head, rows) =>
  [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows.map((r) => `| ${r.join(' | ')} |`)].join('\n');

// Mira's table-b: ten columns, far wider than a 390 px phone.
const WIDE_HEAD = ['Plan', 'Price', 'Seats', 'Storage', 'Builds', 'Regions', 'SSO', 'Audit log', 'Support', 'Uptime'];
const WIDE = `Here is how they compare.\n\n${table(WIDE_HEAD, [
  ['Pagecraft', 'EUR 19', '3', '10 GB', '500', '1', 'no', 'no', 'email', '99.5%'],
  ['Northdeck', 'EUR 49', '5', '50 GB', '3,000', '2', 'no', 'yes', 'chat', '99.9%'],
])}\n\nThat is all of them.`;

const cue = (host) => host.querySelectorAll('[data-scroll-cue]');
const scrollerOf = (host) => host.querySelector('table').closest('.overflow-x-auto');
const scrollTo = async (el, x) => {
  el.scrollLeft = x;
  el.dispatchEvent(new Event('scroll'));
  await settle();
};
const resize = async () => {
  for (const o of observers) o.cb([], o);
  await settle();
};

// ---------------------------------------------------------------------------
// (a) The plain reply table

test('a 10-column table wider than its box shows the right-edge fade, inside the border, as a cue only', async () => {
  Object.assign(box, { scrollWidth: 900, clientWidth: 356 });
  const v = await mount(h(m.MarkdownRenderer, { content: WIDE }));
  const sc = scrollerOf(v.host);
  assert.ok(sc, 'the table scrolls in its wrapper');
  assert.equal(cue(v.host).length, 1, 'one fade while it can still scroll right');
  const fade = cue(v.host)[0];
  assert.equal(fade.getAttribute('aria-hidden'), 'true', 'not read out');
  assert.match(fade.className, /\bpointer-events-none\b/, 'never in the way of a tap or a scroll');
  assert.match(fade.className, /\babsolute\b.*\binset-y-px\b.*\bright-px\b/, 'inside the 1 px border');
  assert.match(fade.className, /\bbg-gradient-to-l\b.*\bfrom-zinc-900\b/, 'a token colour, no hex');
  assert.doesNotMatch(fade.className, /#[0-9a-f]{3,6}/i);
  assert.equal(fade.parentElement, sc.parentElement, 'a sibling of the scroller, so it stays at the right edge while the table scrolls');
  assert.match(sc.parentElement.className, /\brelative\b/);
  assert.equal(sc.querySelectorAll('th').length, 10);
});

test('the fade goes once the table is scrolled to the end, and comes back when scrolled left', async () => {
  Object.assign(box, { scrollWidth: 900, clientWidth: 356 });
  const v = await mount(h(m.MarkdownRenderer, { content: WIDE }));
  const sc = scrollerOf(v.host);
  await scrollTo(sc, 300);
  assert.equal(cue(v.host).length, 1, 'still more to the right');
  await scrollTo(sc, 544);
  assert.equal(cue(v.host).length, 0, 'at the end: no fade');
  await scrollTo(sc, 543.5);
  assert.equal(cue(v.host).length, 0, 'within a pixel of the end counts as the end');
  await scrollTo(sc, 0);
  assert.equal(cue(v.host).length, 1, 'back at the start: the fade again');
});

test('a table that fits has no fade, and its wrapper looks as before', async () => {
  Object.assign(box, { scrollWidth: 700, clientWidth: 700 });
  const v = await mount(h(m.MarkdownRenderer, { content: WIDE }));
  assert.equal(cue(v.host).length, 0);
  const sc = scrollerOf(v.host);
  assert.deepEqual(
    [...sc.classList].sort(),
    ['bg-zinc-900/60', 'border', 'border-zinc-700/60', 'overflow-x-auto', 'rounded-xl', 'shadow-[var(--kws-shadow)]'].sort(),
  );
  assert.equal(sc.parentElement.className, 'relative my-3.5', 'the same 14 px gap above and below');
  assert.equal(sc.querySelector('table').className, 'w-full text-left border-collapse text-xs font-sans');
});

test('the fade follows a resize: a table that stops fitting gets it, one that fits again loses it', async () => {
  Object.assign(box, { scrollWidth: 700, clientWidth: 700 });
  const v = await mount(h(m.MarkdownRenderer, { content: WIDE }));
  assert.equal(cue(v.host).length, 0);
  Object.assign(box, { clientWidth: 356 });
  await resize();
  assert.equal(cue(v.host).length, 1, 'narrowed to a phone: the fade');
  Object.assign(box, { clientWidth: 700 });
  await resize();
  assert.equal(cue(v.host).length, 0, 'wide again: none');
});

// ---------------------------------------------------------------------------
// (c) The phone comparison card

const CMP_HEAD = ['Host', 'Price', 'Pros', 'Cons'];
const CMP_ROWS = [
  ['Pagecraft', 'EUR 19', 'Cheapest; simple setup', 'No preview builds'],
  ['Swiftbox', 'EUR 29', '**Fast** builds', 'Pricey; US only'],
  ['Fernlane', 'EUR 129', 'SSO; audit log', ''],
];
const CMP = {
  reply: `Three hosts.\n\n${table(CMP_HEAD, CMP_ROWS)}\n\nThat is all.`,
  results: {
    version: 1,
    blocks: [{
      id: 'b-cmp', type: 'comparison', title: 'Hosts',
      source: { kind: 'markdown_table', index: 0 },
      provenance: { kind: 'reply_text' },
      columns: [
        { key: 'host', label: 'Host', numeric: false, unit: null, role: 'label' },
        { key: 'price', label: 'Price', numeric: true, unit: 'EUR', role: 'value' },
        { key: 'pros', label: 'Pros', numeric: false, unit: null, role: 'pros' },
        { key: 'cons', label: 'Cons', numeric: false, unit: null, role: 'cons' },
      ],
      rows: CMP_ROWS.map((cells, i) => ({ id: `r${i}`, cells })),
      highlight_row: null, highlight_reason: null,
    }],
  },
};
const cards = (host) => [...host.querySelectorAll('[data-smart-result="comparison"] > div.sm\\:hidden > ul > li')];
const cell = (card, label) => {
  const dt = [...card.querySelectorAll('dt')].find((d) => d.textContent === label);
  return dt.nextElementSibling;
};
const items = (dd) => [...dd.querySelectorAll('ul[role="list"] > li')].map((li) => ({
  text: li.textContent,
  mark: li.querySelector('svg.lucide-minus') ? '−' : li.querySelector('svg.lucide-plus') ? '+' : null,
}));

test('a one-item Cons and a one-item Pros get their mark, as one list item', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: CMP.reply, results: CMP.results, messageId: 'mc' }));
  const [pagecraft, swiftbox] = cards(v.host);
  assert.deepEqual(items(cell(pagecraft, 'Cons')), [{ text: 'No preview builds', mark: '−' }]);
  assert.deepEqual(items(cell(swiftbox, 'Pros')), [{ text: 'Fast builds', mark: '+' }]);
  const li = cell(swiftbox, 'Pros').querySelector('li');
  assert.equal(li.querySelector('strong')?.textContent, 'Fast', 'the reply’s own cell: bold stays bold');
  assert.equal(li.querySelector('svg').getAttribute('aria-hidden'), 'true', 'the mark is not read out');
  assert.match(li.querySelector('svg').getAttribute('class'), /text-ok/);
  assert.match(cell(pagecraft, 'Cons').querySelector('svg').getAttribute('class'), /text-rose-300/);
});

test('multi-item Pros and Cons are unchanged, and an empty cell gets no mark', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: CMP.reply, results: CMP.results, messageId: 'mc' }));
  const [pagecraft, swiftbox, fernlane] = cards(v.host);
  assert.deepEqual(items(cell(pagecraft, 'Pros')), [{ text: 'Cheapest', mark: '+' }, { text: 'simple setup', mark: '+' }]);
  assert.deepEqual(items(cell(swiftbox, 'Cons')), [{ text: 'Pricey', mark: '−' }, { text: 'US only', mark: '−' }]);
  assert.equal(cell(fernlane, 'Cons').querySelector('ul, svg'), null, 'empty stays empty');
  assert.equal(cell(fernlane, 'Price').querySelector('ul, svg'), null, 'other columns never get a mark');
  // shortList keeps its contract.
  assert.equal(m.shortList('No preview builds'), null);
  assert.deepEqual(m.shortList('Pricey; US only'), ['Pricey', 'US only']);
});
