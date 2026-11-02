// Smart Results (0.6.5): a bot reply's table or list shown as a block (a sortable comparison, steps, a
// checklist, a timeline; Iris adds the other types), from the `results` the server attaches. Always on:
// there is no setting. Anything the client can't trust, and the reply renders as plain markdown.
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
const clipboard = { text: null, writeText: async (t) => void (clipboard.text = t) };
Object.defineProperty(g.navigator, 'clipboard', { value: clipboard, configurable: true });
g.fetch = g.window.fetch = async () => new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * from './src/smartResults';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { SmartResultBoundary } from './src/components/results/SmartResult';
  export { SmartActionsSlot } from './src/components/results/SmartActions';
  export { REGISTRY, resolveBlock } from './src/components/results/registry';
  export { FIXTURES } from './src/components/results/fixtures';
  export { blockCsv, blockMarkdown, blockRows } from './src/components/results/exporters';
  export { COMPOSE_PREFILL_EVENT, evidenceText } from './src/components/results/SmartResult';
  export { shortList } from './src/components/results/Comparison';
`);
const { React, createRoot } = m;
const h = React.createElement;

const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  localStorage.clear();
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const v = {
    host,
    render: async (e) => {
      root.render(e);
      await settle();
    },
    done: () => {
      root.unmount();
      host.remove();
    },
  };
  mounted.add(v);
  await v.render(el);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const md = (content, extra = {}) => m.renderToStaticMarkup(h(m.MarkdownRenderer, { content, ...extra }));

// ---------------------------------------------------------------------------
// Fixtures that follow plan.md's payload v1 exactly

const REPLY = `Here are the three plans.

| Plan | Price | Seats |
|---|---|---|
| A | EUR 12 | 1 |
| **B** | EUR 100 | 5 |
| C | EUR 20 | 10 |

I recommend B for a team of five.`;

const comparison = (over = {}) => ({
  id: 'b0',
  type: 'comparison',
  source: { kind: 'markdown_table', index: 0 },
  columns: [
    { key: 'c0', label: 'Plan', numeric: false, unit: null },
    { key: 'c1', label: 'Price', numeric: true, unit: 'EUR' },
    { key: 'c2', label: 'Seats', numeric: true, unit: null },
  ],
  rows: [
    { id: 'r0', cells: ['A', 'EUR 12', '1'] },
    { id: 'r1', cells: ['**B**', 'EUR 100', '5'] },
    { id: 'r2', cells: ['C', 'EUR 20', '10'] },
  ],
  highlight_row: 'r1',
  highlight_reason: 'named as the pick in this reply',
  provenance: { kind: 'reply_text' },
  ...over,
});
const set = (...blocks) => ({ version: 1, blocks });

const STEPS_REPLY = `To set it up:

1. Open Settings.
2. Turn on **Smart results**.
3. Send a reply with a table.

That's it.`;
const steps = (over = {}) => ({
  id: 'b1',
  type: 'steps',
  source: { kind: 'markdown_list', index: 0 },
  items: [
    { id: 's0', text: 'Open Settings.' },
    { id: 's1', text: 'Turn on **Smart results**.' },
    { id: 's2', text: 'Send a reply with a table.' },
  ],
  provenance: { kind: 'reply_text' },
  ...over,
});

const bodyRows = (host) => [...host.querySelectorAll('[data-smart-result="comparison"] tbody tr')].map((tr) => tr.querySelector('td').textContent.replace(/Pick in this reply(Pick)?$/, ''));
const header = (host, label) => [...host.querySelectorAll('th')].find((th) => th.textContent.includes(label));

// ---------------------------------------------------------------------------
// Off path: identical to 0.6.4

test('no results, results null or a streaming reply: the exact 0.6.4 markup', () => {
  const plain = md(REPLY);
  assert.match(plain, /<table/);
  assert.doesNotMatch(plain, /data-smart-result/);
  assert.equal(md(REPLY, { results: null, messageId: 'm1' }), plain);
  assert.equal(md(REPLY, { results: undefined, messageId: 'm1' }), plain);
  // Streaming never shows a block, even with one on hand.
  assert.equal(md(REPLY, { results: set(comparison()), messageId: 'm1', isStreaming: true }), md(REPLY, { isStreaming: true }));
});

// ---------------------------------------------------------------------------
// Comparison

test('a valid comparison replaces table #0 and shows the reply’s own cells', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: 'm1' }));
  const block = v.host.querySelector('[data-smart-result="comparison"]');
  assert.ok(block, 'block rendered');
  assert.equal(v.host.querySelectorAll('table').length, 1, 'one table: the block’s, not the markdown one too');
  assert.deepEqual(bodyRows(v.host), ['A', 'B', 'C']);
  // The cell is the reply's rendered markdown: **B** is bold, not asterisks.
  assert.equal(block.querySelector('tbody tr:nth-child(2) td strong')?.textContent, 'B');
  assert.doesNotMatch(block.textContent, /\*\*/);
  // The prose around the table is untouched.
  assert.match(v.host.textContent, /Here are the three plans\..*I recommend B for a team of five\./s);
  // Wide view keeps the markdown table's classes, so the swap from the streamed table moves nothing.
  const wrap = block.querySelector('.overflow-x-auto');
  assert.equal(wrap.className, 'overflow-x-auto rounded-xl border border-zinc-800 bg-zinc-950/70 shadow-sm');
  assert.equal(wrap.querySelector('table').className, 'w-full text-left border-collapse text-xs font-sans');
});

test('the highlighted row carries one “Pick in this reply” label per view', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: 'm1' }));
  const picked = [...v.host.querySelectorAll('tr[data-pick]')];
  assert.equal(picked.length, 1);
  assert.match(picked[0].textContent, /^B\s*Pick in this reply/);
  assert.equal(picked[0].querySelectorAll('.smart-pick-full').length, 1, 'one label a screen reader reads, however narrow');
  assert.equal(picked[0].querySelector('.smart-pick-short')?.getAttribute('aria-hidden'), 'true');
  const cards = [...v.host.querySelectorAll('li[data-pick]')];
  assert.equal(cards.length, 1, 'the narrow view marks the same row');
  assert.equal(v.host.querySelectorAll('tr:not([data-pick])').length, 3, 'header + two plain rows');
  // No highlight in the block, no label.
  const w = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison({ highlight_row: null, highlight_reason: null })), messageId: 'm1' }));
  assert.doesNotMatch(w.host.textContent, /Pick in this reply/);
});

test('header buttons sort ascending, descending, then back; numbers sort as numbers', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: 'm1' }));
  const price = () => header(v.host, 'Price');
  const seats = () => header(v.host, 'Seats');
  assert.equal(price().getAttribute('aria-sort'), 'none');
  assert.equal(price().querySelector('button').textContent, 'Price', 'the button is named by its column');
  await click(price().querySelector('button'));
  assert.equal(price().getAttribute('aria-sort'), 'ascending');
  assert.deepEqual(bodyRows(v.host), ['A', 'C', 'B'], 'EUR 12, EUR 20, EUR 100');
  await click(price().querySelector('button'));
  assert.equal(price().getAttribute('aria-sort'), 'descending');
  assert.deepEqual(bodyRows(v.host), ['B', 'C', 'A']);
  await click(price().querySelector('button'));
  assert.equal(price().getAttribute('aria-sort'), 'none');
  assert.deepEqual(bodyRows(v.host), ['A', 'B', 'C'], 'the reply’s order again');
  await click(seats().querySelector('button'));
  assert.deepEqual(bodyRows(v.host), ['A', 'B', 'C'], '1, 5, 10 (not 1, 10, 5)');
  assert.equal(price().getAttribute('aria-sort'), 'none', 'only the active column says sorted');
  assert.match(v.host.querySelector('[aria-live]').textContent, /Sorted by Seats, ascending/);
  // The narrow view's select shares the order.
  assert.equal(v.host.querySelector('select').value, '2:ascending');
});

test('sorting helpers: units, separators, signs, text and blanks', () => {
  assert.equal(m.cellNumber('EUR 1,200', 'EUR'), 1200);
  assert.equal(m.cellNumber('$9.50', null), 9.5);
  assert.equal(m.cellNumber('-3%', null), -3);
  assert.equal(m.cellNumber('1 200', null), 1200);
  assert.equal(m.cellNumber('12 GB', 'GB'), 12);
  assert.ok(Number.isNaN(m.cellNumber('n/a', null)));
  const b = comparison({
    rows: [
      { id: 'r0', cells: ['A', 'n/a', '1'] },
      { id: 'r1', cells: ['b', 'EUR 9', '2'] },
      { id: 'r2', cells: ['C', 'EUR 10', '3'] },
    ],
    highlight_row: null,
  });
  assert.deepEqual(m.sortRows(b, { col: 1, dir: 'ascending' }).map((r) => r.id), ['r1', 'r2', 'r0'], 'a cell without a number goes last');
  assert.deepEqual(m.sortRows(b, { col: 1, dir: 'descending' }).map((r) => r.id), ['r2', 'r1', 'r0'], 'last both ways');
  assert.deepEqual(m.sortRows(b, { col: 0, dir: 'ascending' }).map((r) => r.id), ['r0', 'r1', 'r2'], 'text ignores case');
  assert.deepEqual(m.nextSort(null, 1), { col: 1, dir: 'ascending' });
  assert.deepEqual(m.nextSort({ col: 1, dir: 'ascending' }, 1), { col: 1, dir: 'descending' });
  assert.equal(m.nextSort({ col: 1, dir: 'descending' }, 1), null);
  assert.deepEqual(m.nextSort({ col: 1, dir: 'descending' }, 0), { col: 0, dir: 'ascending' });
});

const copyData = async (host, format) => {
  // Its label says "Copied" for 2 s after a copy.
  const button = host.querySelector('button[aria-haspopup="menu"]');
  assert.ok(button, 'Copy data is there');
  await click(button);
  assert.equal(button.getAttribute('aria-expanded'), 'true');
  const item = [...host.querySelectorAll('[role="menu"] [role="menuitem"]')].find((b) => b.textContent === format);
  assert.deepEqual([...host.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent), ['Markdown', 'CSV', 'JSON']);
  await click(item);
  assert.equal(host.querySelector('[role="menu"]'), null, 'the menu closes after copying');
  return button;
};

test('Copy data copies Markdown, CSV or JSON; spreadsheet formulas are neutralised', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: 'm1' }));
  assert.equal([...v.host.querySelectorAll('button')].filter((b) => b.textContent.includes('Copy data')).length, 1, 'one button for both views');
  const button = await copyData(v.host, 'CSV');
  assert.equal(clipboard.text, 'Plan,Price,Seats\nA,EUR 12,1\nB,EUR 100,5\nC,EUR 20,10\n');
  assert.match(button.textContent, /Copied/);
  await copyData(v.host, 'Markdown');
  assert.equal(clipboard.text, '| Plan | Price | Seats |\n| --- | --- | --- |\n| A | EUR 12 | 1 |\n| B | EUR 100 | 5 |\n| C | EUR 20 | 10 |');
  await copyData(v.host, 'JSON');
  const json = JSON.parse(clipboard.text);
  assert.equal(json.type, 'comparison');
  assert.deepEqual(json.rows[1].cells, ['**B**', 'EUR 100', '5']);
  // Escape closes the menu and puts focus back on the button.
  await click(button);
  const menu = v.host.querySelector('[role="menu"]');
  menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(v.host.querySelector('[role="menu"]'), null);
  assert.equal(document.activeElement, button);
  const risky = comparison({
    rows: [
      { id: 'r0', cells: ['=HYPERLINK("x")', '-3', '1'] },
      { id: 'r1', cells: ['say "hi", ok', '+2', '2'] },
      { id: 'r2', cells: ['-1+cmd', '@sum', '3'] },
    ],
    highlight_row: null,
  });
  assert.equal(m.comparisonCsv(risky), `Plan,Price,Seats\n"'=HYPERLINK(""x"")",-3,1\n"say ""hi"", ok",+2,2\n'-1+cmd,'@sum,3\n`);
});

test('below 640 px the same rows are label: value cards (both views rendered, CSS picks one)', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: 'm1' }));
  const wide = v.host.querySelector('[data-smart-result="comparison"] > div.hidden.sm\\:block');
  const narrow = v.host.querySelector('[data-smart-result="comparison"] > div.sm\\:hidden');
  assert.ok(wide && narrow);
  const cards = [...narrow.querySelectorAll('ul > li')];
  assert.equal(cards.length, 3);
  assert.deepEqual([...cards[0].querySelectorAll('dt')].map((d) => d.textContent), ['Price', 'Seats']);
  assert.deepEqual([...cards[0].querySelectorAll('dd')].map((d) => d.textContent), ['EUR 12', '1']);
  assert.ok(cards.every((c) => /overflow-wrap:anywhere/.test(c.innerHTML)), 'long strings wrap instead of clipping');
});

test('in a chat, the block’s buttons go in the message’s action row, so the reply keeps its size', async () => {
  const Message = ({ id, content, results }) =>
    h('div', null, h('div', { id: 'bubble' }, h(m.MarkdownRenderer, { content, results, messageId: id })), h('div', { id: 'row' }, h('button', null, 'Copy markdown'), h(m.SmartActionsSlot, { messageId: id })));
  const other = document.createElement('span');
  other.setAttribute('data-smart-actions', 'm1-other');
  document.body.append(other);
  const v = await mount(h('div', null, h(Message, { id: 'm1', content: REPLY, results: set(comparison()) }), h(Message, { id: 'm7', content: STEPS_REPLY, results: set(steps()) })));
  const rows = [...v.host.querySelectorAll('#row')];
  for (const row of rows) {
    assert.deepEqual([...row.querySelectorAll('button')].map((b) => b.textContent.trim()), ['Copy markdown', 'Show as text', 'Copy data']);
    assert.equal(row.querySelector('[data-evidence]').textContent, 'From this reply');
  }
  assert.equal(other.childNodes.length, 0, 'only its own message’s row');
  const extra = (bubble) => [...bubble.querySelectorAll('button')].filter((b) => !b.closest('th') && !/^Ask about/.test(b.getAttribute('aria-label') ?? b.textContent));
  for (const bubble of v.host.querySelectorAll('#bubble')) assert.equal(extra(bubble).length, 0, 'nothing but sort and Ask about this inside the reply');
  other.remove();
});

test('no setting: ChatView always passes results and adds the action-row spot when a message has them', async () => {
  const { readFile } = await import('node:fs/promises');
  const read = (f) => readFile(new URL(`../src/${f}`, import.meta.url), 'utf8');
  const chat = await read('components/ChatView.tsx');
  assert.match(chat, /\{msg\.results != null && <SmartActionsSlot messageId=\{msg\.id\} \/>\}/);
  assert.match(chat, /results=\{msg\.results\} messageId=\{msg\.id\}/);
  for (const f of ['components/ChatView.tsx', 'components/SettingsModal.tsx', 'smartResults.ts', 'components/MarkdownRenderer.tsx']) assert.doesNotMatch(await read(f), /smart_results|SmartResultsOn|smart-results-label/, f);
});

test('Show as text swaps the block for the reply’s markdown, per message, and is remembered', async () => {
  const plain = md(REPLY);
  const Message = ({ id }) => h('div', null, h('div', { id: 'bubble' }, h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: id })), h('div', { id: 'row' }, h(m.SmartActionsSlot, { messageId: id })));
  const v = await mount(h(Message, { id: 'm1' }));
  const toggle = () => [...v.host.querySelectorAll('button')].find((b) => b.textContent === 'Show as text');
  assert.equal(toggle().getAttribute('aria-pressed'), 'false');
  await click(toggle());
  assert.equal(toggle().getAttribute('aria-pressed'), 'true');
  assert.equal(v.host.querySelector('[data-smart-result]'), null);
  assert.ok(v.host.querySelector('#bubble table'), 'the reply’s own table');
  assert.equal(v.host.querySelectorAll('#bubble th button').length, 0, 'no sort buttons');
  assert.match(plain, /<table/);
  assert.equal(localStorage.getItem('kwnt:smart-text:m1'), '1');
  v.done();
  mounted.delete(v);
  const again = await mount(h(Message, { id: 'm1' }));
  assert.equal(again.host.querySelector('[data-smart-result]'), null, 'still text after a reload');
  const other = await mount(h(Message, { id: 'm2' }));
  assert.ok(other.host.querySelector('[data-smart-result="comparison"]'), 'other messages keep their block');
});

test('Ask about this fills in the composer through an event and never sends', async () => {
  const asked = [];
  const onAsk = (e) => asked.push(e.detail.text);
  window.addEventListener(m.COMPOSE_PREFILL_EVENT, onAsk);
  const sent = [];
  const fetchBefore = g.fetch;
  g.fetch = g.window.fetch = async (...a) => (sent.push(a), new Response('{}'));
  try {
    const v = await mount(h(m.MarkdownRenderer, { content: REPLY, results: set(comparison()), messageId: 'm1' }));
    const row = v.host.querySelector('tbody tr:nth-child(2) button[aria-label^="Ask about"]');
    assert.equal(row.getAttribute('aria-label'), 'Ask about B');
    await click(row);
    const card = [...v.host.querySelectorAll('li button')].filter((b) => b.textContent.trim() === 'Ask about this');
    assert.equal(card.length, 3, 'each stacked card has one');
    await click(card[2]);
    assert.deepEqual(asked, ['About B in your Plan table: ', 'About C in your Plan table: ']);
    assert.equal(sent.length, 0, 'nothing sent');
    const chat = await (await import('node:fs/promises')).readFile(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
    const listener = chat.slice(chat.indexOf('const prefill = '), chat.indexOf('window.removeEventListener(COMPOSE_PREFILL_EVENT'));
    assert.match(listener, /setInputVal/);
    assert.doesNotMatch(listener, /handleSubmit|onSendMessage/, 'the composer is filled in, not sent');
  } finally {
    window.removeEventListener(m.COMPOSE_PREFILL_EVENT, onAsk);
    g.fetch = g.window.fetch = fetchBefore;
  }
});

test('pros and cons columns: tinted headers, and short lists in the stacked cards', async () => {
  const { reply, results } = m.FIXTURES.comparison;
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results, messageId: 'mc' }));
  assert.ok(v.host.querySelector('[data-smart-result="comparison"]'), 'the fixture renders');
  assert.match(header(v.host, 'Pros').querySelector('button').className, /text-emerald-300/);
  assert.match(header(v.host, 'Cons').querySelector('button').className, /text-rose-300/);
  const card = v.host.querySelectorAll('[data-smart-result="comparison"] > div.sm\\:hidden > ul > li')[1];
  assert.deepEqual([...card.querySelectorAll('dd ul li')].map((li) => li.textContent), ['Preview builds', 'EU region', 'chat support']);
  assert.deepEqual(m.shortList('Cheapest; simple setup'), ['Cheapest', 'simple setup']);
  assert.deepEqual(m.shortList('Fast, cheap, small'), ['Fast', 'cheap', 'small']);
  assert.equal(m.shortList('No preview builds'), null);
  assert.equal(m.shortList('Good if you need SSO, audit logs and a named contact for every team'), null, 'a sentence stays a sentence');
});

// ---------------------------------------------------------------------------
// Steps

test('steps: numbered and tickable; ticks stay per message and block; Copy data copies the markdown', async () => {
  const v = await mount(h(m.MarkdownRenderer, { content: STEPS_REPLY, results: set(steps()), messageId: 'm7' }));
  const block = v.host.querySelector('[data-smart-result="steps"]');
  assert.ok(block);
  const boxes = () => [...v.host.querySelectorAll('input[type="checkbox"]')];
  assert.equal(boxes().length, 3);
  assert.ok(boxes().every((b) => b.labels.length === 1), 'each tick is a labelled checkbox');
  assert.match(boxes()[1].labels[0].textContent, /^2\.\s*Turn on Smart results\.$/);
  assert.equal(boxes()[1].labels[0].querySelector('strong')?.textContent, 'Smart results', 'the reply’s markdown');
  assert.match(block.textContent, /0 of 3 done/);
  await click(boxes()[1]);
  assert.equal(boxes()[1].checked, true);
  assert.match(block.textContent, /1 of 3 done/);
  assert.equal(localStorage.getItem('kwnt:steps:m7:b1'), '["s1"]', 'ids only, no reply text');
  v.done();
  mounted.delete(v);
  const again = await mount(h(m.MarkdownRenderer, { content: STEPS_REPLY, results: set(steps()), messageId: 'm7' }));
  assert.deepEqual([...again.host.querySelectorAll('input[type="checkbox"]')].map((b) => b.checked), [false, true, false], 'kept after a reload');
  const other = await mount(h(m.MarkdownRenderer, { content: STEPS_REPLY, results: set(steps()), messageId: 'm8' }));
  assert.deepEqual([...other.host.querySelectorAll('input[type="checkbox"]')].map((b) => b.checked), [false, false, false], 'another message starts fresh');
  await copyData(again.host, 'Markdown');
  assert.equal(clipboard.text, '1. Open Settings.\n2. Turn on **Smart results**.\n3. Send a reply with a table.\n');
});

test('steps follow the list’s own start number, and a loose list reads as one line per step', async () => {
  const numbered = STEPS_REPLY.replace('1. Open', '4. Open').replace('2. Turn', '5. Turn').replace('3. Send', '6. Send');
  const v = await mount(h(m.MarkdownRenderer, { content: numbered, results: set(steps()), messageId: 'm7' }));
  assert.match(v.host.querySelector('label').textContent, /^4\./);
  const loose = STEPS_REPLY.replace('\n2.', '\n\n2.').replace('\n3.', '\n\n3.');
  const w = await mount(h(m.MarkdownRenderer, { content: loose, results: set(steps()), messageId: 'm9' }));
  assert.ok(w.host.querySelector('[data-smart-result="steps"]'));
  assert.equal(w.host.querySelector('[data-smart-result="steps"] p'), null, 'no paragraph blocks inside a label');
});

test('checklist: a task list whose ticks start from the reply’s [x], with the same box in the streamed list', async () => {
  const { reply, results } = m.FIXTURES.checklist;
  const streamed = await mount(h(m.MarkdownRenderer, { content: reply, isStreaming: true }));
  const sBoxes = [...streamed.host.querySelectorAll('input[type="checkbox"]')];
  assert.deepEqual(sBoxes.map((b) => [b.checked, b.disabled]), [[true, true], [true, true], [false, true], [false, true]], 'read-only while streaming');
  assert.match(streamed.host.querySelector('ul').className, /list-none pl-1 my-2\.5 space-y-1/);
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results, messageId: 'mk' }));
  const block = v.host.querySelector('[data-smart-result="steps"][data-variant="checklist"]');
  assert.ok(block);
  assert.equal(block.querySelector('ul').getAttribute('role'), 'list', 'an unordered task list stays a ul');
  const boxes = [...block.querySelectorAll('input[type="checkbox"]')];
  assert.deepEqual(boxes.map((b) => b.checked), [true, true, false, false]);
  assert.ok(boxes.every((b) => !b.disabled && b.labels.length === 1));
  assert.equal(boxes[0].className, sBoxes[0].className.replace(' rounded-sm accent-sky-500', ' cursor-pointer rounded-sm accent-sky-500') + ' focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500/70 focus-visible:ring-offset-1 focus-visible:ring-offset-zinc-900', 'the same box, now clickable');
  assert.doesNotMatch(block.textContent, /^\s*\d\./m, 'no numbers on a checklist');
  assert.equal(block.querySelector('label code')?.textContent, 'docs', 'the reply’s markdown');
  await click(boxes[0]);
  assert.equal(localStorage.getItem('kwnt:steps:mk:b-check'), '["c1"]');
  await click(boxes[0]);
  assert.equal(localStorage.getItem('kwnt:steps:mk:b-check'), null, 'back to the reply’s ticks: nothing stored');
  await click(boxes[2]);
  await copyData(v.host, 'Markdown');
  assert.equal(clipboard.text, '- [x] Export the old site as static files\n- [x] Create the Northdeck project\n- [x] Move the `docs` CNAME (owner: Sam)\n- [ ] Turn on preview builds\n');
});

test('timeline: a rail in the bullets’ place, the reply’s own items', async () => {
  const { reply, results } = m.FIXTURES.timeline;
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results, messageId: 'mt' }));
  const block = v.host.querySelector('[data-smart-result="steps"][data-variant="timeline"]');
  assert.ok(block);
  assert.match(block.className, /my-2\.5 space-y-1 pl-5/, 'the list’s own box');
  assert.deepEqual([...block.querySelectorAll(':scope > li')].map((li) => li.textContent), ['Mon 9 Nov: Freeze content on the old site', 'Tue 10 Nov: Switch DNS (Owner: Sam)', 'Wed 11 Nov: Check search console for errors', 'Fri 13 Nov: Cancel the old plan']);
  assert.equal(block.querySelectorAll('[aria-hidden="true"].rounded-full').length, 4);
  assert.equal(block.querySelectorAll('[aria-hidden="true"].w-px').length, 3, 'no line after the last item');
  assert.equal(m.blockMarkdown(m.validSteps(results.blocks[0])), '- Mon 9 Nov: Freeze content on the old site\n- Tue 10 Nov: Switch DNS (Owner: Sam)\n- Wed 11 Nov: Check search console for errors\n- Fri 13 Nov: Cancel the old plan\n');
  assert.equal(m.validSteps({ ...results.blocks[0], items: results.blocks[0].items.map((i) => ({ ...i, when: null })) }), null, 'a timeline needs every when');
});

test('a task list only becomes a checklist; nested or quoted lists stay markdown', () => {
  const task = STEPS_REPLY.replace('1. Open', '1. [ ] Open').replace('2. Turn', '2. [ ] Turn').replace('3. Send', '3. [ ] Send');
  assert.equal(md(task, { results: set(steps()), messageId: 'm1' }), md(task), 'numbered steps never take a task list');
  const bullets = STEPS_REPLY.replace(/^\d\./gm, '-');
  assert.equal(md(bullets, { results: set(steps({ source: { kind: 'markdown_list', index: 0, ordered: false } })), messageId: 'm1' }), md(bullets), 'numbered steps need a numbered list');
  assert.equal(md(STEPS_REPLY, { results: set(steps({ variant: 'checklist' })), messageId: 'm1' }), md(STEPS_REPLY), 'a checklist needs a task list');
  const quoted = STEPS_REPLY.replace(/^(\d\.)/gm, '> $1');
  assert.equal(md(quoted, { results: set(steps()), messageId: 'm1' }), md(quoted));
});

// ---------------------------------------------------------------------------
// Registry, frame, fixtures

test('fixtures: one per block type, every block well formed; Pixel’s types render from them', async () => {
  const types = new Set(Object.values(m.FIXTURES).map((f) => f.results.blocks[0].type));
  assert.deepEqual([...types].sort(), ['cards', 'chart', 'comparison', 'facts', 'files', 'findings', 'metrics', 'steps', 'verification']);
  const strings = (v) => (typeof v === 'string' ? [v] : Array.isArray(v) ? v.flatMap(strings) : v && typeof v === 'object' ? Object.values(v).flatMap(strings) : []);
  for (const [name, f] of Object.entries(m.FIXTURES)) {
    assert.equal(m.resultBlocks(f.results).length, 1, name);
    assert.ok(m.parseCommon(f.results.blocks[0], f.results.blocks[0].type), name);
    assert.ok(strings(f.results).every((t) => t.length <= 300), `${name}: strings ≤ 300`);
    if (m.REGISTRY[f.results.blocks[0].type]) {
      const v = await mount(h(m.MarkdownRenderer, { content: f.reply, results: f.results, messageId: `fx-${name}` }));
      assert.ok(v.host.querySelector('[data-smart-frame]'), `${name} renders`);
    }
  }
  assert.deepEqual(Object.keys(m.REGISTRY).sort(), ['comparison', 'steps']);
});

test('a slot block renders after the reply with its title; Show as text gives its data as markdown', async () => {
  m.REGISTRY.verification = {
    validate: (raw) => m.parseCommon(raw, 'verification'),
    Component: ({ block }) => h('div', { 'data-test-verification': '' }, block.summary),
  };
  try {
    const { reply, results } = m.FIXTURES.verification;
    const titled = { version: 1, blocks: [{ ...results.blocks[0], title: 'Outcome checks' }] };
    const Message = ({ id }) => h('div', null, h('div', { id: 'bubble' }, h(m.MarkdownRenderer, { content: reply, results: titled, messageId: id })), h('div', { id: 'row' }, h(m.SmartActionsSlot, { messageId: id })));
    const v = await mount(h(Message, { id: 'mv' }));
    const bubble = v.host.querySelector('#bubble > div');
    const frame = bubble.lastElementChild;
    assert.equal(frame.getAttribute('data-smart-frame'), 'verification', 'after the reply text');
    assert.equal(frame.getAttribute('role'), 'group');
    assert.equal(frame.firstElementChild.textContent, 'Outcome checks', 'a slot block shows its title');
    assert.match(bubble.firstElementChild.textContent, /The migration task is done/);
    assert.equal(v.host.querySelector('[data-evidence]').textContent, 'From task checks');
    assert.equal(md(reply, { results: titled, messageId: 'mv', isStreaming: true }), md(reply, { isStreaming: true }), 'nothing while streaming');
    await click([...v.host.querySelectorAll('button')].find((b) => b.textContent === 'Show as text'));
    assert.equal(v.host.querySelector('[data-test-verification]'), null);
    assert.deepEqual([...frame.querySelectorAll('tbody tr td:first-child')].map((td) => td.textContent), ['docs.trailhead.shop answers 200', 'Certificate is valid', 'Old host redirects', 'Sitemap lists 120 pages']);
    // A slot block with a reply source elsewhere, or a sourced block with no table there, is dropped.
    assert.equal(m.resolveBlock(results.blocks[0], { kind: 'table', head: [], body: [] }), null);
    assert.equal(m.resolveBlock(m.FIXTURES.comparison.results.blocks[0], null), null);
  } finally {
    delete m.REGISTRY.verification;
  }
});

test('evidence wording follows provenance', () => {
  const ev = (provenance, extra = {}) => m.evidenceText({ ...m.FIXTURES.files.results.blocks[0], provenance, ...extra });
  assert.equal(ev({ kind: 'reply_text' }), 'From this reply');
  assert.equal(ev({ kind: 'bot_declared', tool_call_id: 't1' }), 'From this reply');
  assert.equal(ev({ kind: 'kwant_data', ref: 'task:7f3a9c' }), 'From task checks');
  assert.equal(ev({ kind: 'kwant_data', ref: 'attachment:att-redirects' }), 'From attachment redirects.csv');
  assert.equal(ev({ kind: 'kwant_data', ref: 'attachment:gone' }), 'From attachments');
  assert.equal(m.parseProvenance({ kind: 'kwant_data', ref: '../etc/passwd' }), null);
});

test('generic Copy data: every fixture exports CSV and Markdown with one row per item', () => {
  for (const [name, f] of Object.entries(m.FIXTURES)) {
    const block = f.results.blocks[0];
    const rows = m.blockRows(block);
    const n = (block.rows ?? block.points ?? block.checks ?? block.items).length;
    assert.equal(rows.length, n, name);
    assert.equal(m.blockCsv(block).trim().split('\n').length, n + 1, `${name}: header + rows`);
    assert.ok(m.blockMarkdown(block).length > 0, name);
  }
  assert.equal(m.blockCsv(m.FIXTURES.chartLine.results.blocks[0]).split('\n')[1], 'Jul 2026,"1,240",310', 'chart cells verbatim');
});

// ---------------------------------------------------------------------------
// Fallback: anything off renders the original markdown, seamlessly

test('unknown version or type, or no message id: the original markdown', () => {
  const plain = md(REPLY);
  assert.equal(md(REPLY, { results: { version: 2, blocks: [comparison()] }, messageId: 'm1' }), plain);
  assert.equal(md(REPLY, { results: set(comparison({ type: 'chart' })), messageId: 'm1' }), plain);
  assert.equal(md(REPLY, { results: set(comparison()) }), plain);
  assert.equal(md(REPLY, { results: 'nonsense', messageId: 'm1' }), plain);
});

test('malformed or untrustworthy blocks: the original markdown', () => {
  const plain = md(REPLY);
  const two = comparison().rows.slice(0, 2);
  const bad = {
    'two rows': comparison({ rows: two }),
    'one column': comparison({ columns: comparison().columns.slice(0, 1), rows: comparison().rows.map((r) => ({ ...r, cells: r.cells.slice(0, 1) })) }),
    'ragged row': comparison({ rows: [...two, { id: 'r2', cells: ['C', 'EUR 20'] }] }),
    'duplicate row ids': comparison({ rows: [...two, { id: 'r1', cells: ['C', 'EUR 20', '10'] }] }),
    'duplicate labels': comparison({ rows: [...two, { id: 'r2', cells: ['a', 'EUR 20', '10'] }] }),
    'highlight not a row': comparison({ highlight_row: 'r9' }),
    'html in a cell': comparison({ rows: [...two, { id: 'r2', cells: ['<img src=x onerror=alert(1)>', 'EUR 20', '10'] }] }),
    'html in a label': comparison({ columns: [{ key: 'c0', label: '<b>Plan</b>', numeric: false, unit: null }, ...comparison().columns.slice(1)] }),
    'cell over 300 chars': comparison({ rows: [...two, { id: 'r2', cells: ['C'.repeat(301), 'EUR 20', '10'] }] }),
    'index out of range': comparison({ source: { kind: 'markdown_table', index: 3 } }),
    'wrong source kind': comparison({ source: { kind: 'markdown_list', index: 0 } }),
    'cells differ from the reply': comparison({ rows: [two[0], two[1], { id: 'r2', cells: ['C', 'EUR 19', '10'] }] }),
    'header differs from the reply': comparison({ columns: [comparison().columns[0], { ...comparison().columns[1], label: 'Cost' }, comparison().columns[2]] }),
    'rows missing': { ...comparison(), rows: undefined },
    'not an object': 42,
  };
  for (const [why, block] of Object.entries(bad)) assert.equal(md(REPLY, { results: set(block), messageId: 'm1' }), plain, why);
  assert.equal(md(REPLY, { results: { version: 1, blocks: [comparison(), comparison()] }, messageId: 'm1' }), plain, 'duplicate block ids');
  const huge = { ...set(comparison()), padding: 'x'.repeat(70 * 1024) };
  assert.equal(md(REPLY, { results: huge, messageId: 'm1' }), plain, 'over 64 KB');
  const badSteps = {
    'two items': steps({ items: steps().items.slice(0, 2) }),
    'item differs': steps({ items: [...steps().items.slice(0, 2), { id: 's2', text: 'Something else.' }] }),
    'unknown variant': steps({ variant: 'kanban' }),
    'done not a boolean': steps({ variant: 'checklist', items: steps().items.map((i) => ({ ...i, done: 'yes' })) }),
    'html in an item': steps({ items: [...steps().items.slice(0, 2), { id: 's2', text: '<script>x</script>' }] }),
  };
  const plainSteps = md(STEPS_REPLY);
  for (const [why, block] of Object.entries(badSteps)) assert.equal(md(STEPS_REPLY, { results: set(block), messageId: 'm1' }), plainSteps, why);
});

test('a block that throws while rendering shows the fallback; the rest of the reply still renders', async () => {
  const Boom = () => {
    throw new Error('boom');
  };
  const errors = [];
  const quiet = [console.error, console.warn];
  console.error = console.warn = (...a) => errors.push(a);
  try {
    const v = await mount(h('div', null, h(m.SmartResultBoundary, { fallback: h('p', { id: 'fb' }, 'the reply’s table') }, h(Boom)), h('p', { id: 'after' }, 'later text')));
    assert.equal(v.host.querySelector('#fb')?.textContent, 'the reply’s table');
    assert.ok(v.host.querySelector('#after'));
  } finally {
    [console.error, console.warn] = quiet;
  }
  assert.ok(errors.length > 0, 'logged, not thrown');
});

test('indexes count tables and ordered lists in document order; fenced tables don’t count', async () => {
  const reply = `\`\`\`
| Fenced | Table |
|---|---|
| x | y |
\`\`\`

| Small | Table |
|---|---|
| a | b |

${REPLY.split('\n\n').slice(1).join('\n\n')}`;
  const plainFirst = md(reply);
  assert.equal(md(reply, { results: set(comparison()), messageId: 'm1' }), plainFirst, 'index 0 is the 1-row table: mismatch, markdown');
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results: set(comparison({ source: { kind: 'markdown_table', index: 1 } })), messageId: 'm1' }));
  assert.equal(v.host.querySelectorAll('[data-smart-result="comparison"]').length, 1);
  assert.deepEqual(bodyRows(v.host), ['A', 'B', 'C'], 'table #1 became the block');
  assert.match(v.host.textContent, /Small.*Table.*a.*b/s, 'table #0 stays markdown');
  assert.match(v.host.querySelector('pre, code').textContent, /Fenced/);

  const lists = `First, some context:\n\n1. one\n2. two\n\nThen:\n\n${STEPS_REPLY}`;
  const w = await mount(h(m.MarkdownRenderer, { content: lists, results: set(steps({ source: { kind: 'markdown_list', index: 1 } })), messageId: 'm2' }));
  assert.equal(w.host.querySelectorAll('[data-smart-result="steps"]').length, 1);
  assert.match(w.host.querySelector('ol:not([role])').textContent, /one.*two/s, 'list #0 stays markdown');
});

test('a reply’s HTML is shown as text, in plain markdown and next to a block', async () => {
  const reply = `${REPLY}\n\n<img src=x onerror="window.__pwned=1"> and <b>bold</b>`;
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results: set(comparison()), messageId: 'm1' }));
  assert.equal(v.host.querySelector('img'), null);
  assert.equal(v.host.querySelector('b'), null);
  assert.match(v.host.textContent, /<b>bold<\/b>/);
  assert.equal(window.__pwned, undefined);
});
