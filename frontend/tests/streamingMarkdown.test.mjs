// 15a streaming-markdown: a streaming reply is cut into finished blocks plus one open block, so each frame
// parses only the open block. The splitter's cases, "the blocks join back to the text", "the blocks render as
// the whole text would", and a render count: each finished block renders once with its final text.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

// React DevTools' hook, reduced to counting which components rendered in each commit (as in smoothStreaming).
const renders = [];
g.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
  supportsFiber: true,
  isDisabled: false,
  renderers: new Map(),
  inject() {
    return 1;
  },
  onCommitFiberRoot(_id, root) {
    const walk = (next) => {
      const prev = next.alternate;
      if (typeof next.type === 'function' && (!prev || next.flags & 1)) renders.push({ name: next.type.name, props: next.memoizedProps });
      if (prev && next.child === prev.child) return;
      for (let c = next.child; c; c = c.sibling) walk(c);
    };
    walk(root.current);
  },
  onCommitFiberUnmount() {},
  onPostCommitFiberRoot() {},
  checkDCE() {},
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { splitStreamBlocks } from './src/streamBlocks';
  export { StreamMarkdown } from './src/components/StreamMarkdown';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
`);
const { React, createRoot, flushSync, renderToStaticMarkup, splitStreamBlocks, StreamMarkdown, MarkdownRenderer } = m;
const h = React.createElement;

/** Splits, checks the blocks join back to the text, and returns them. */
const split = (text) => {
  const blocks = splitStreamBlocks(text);
  assert.equal(blocks.join(''), text, 'the joined blocks equal the text');
  return blocks;
};

/** The markdown's HTML inside its wrapper(s), with the whitespace between tags dropped. */
const inner = (html) => html.replace(/<div class="markdown-content[^"]*">/g, '').replace(/<\/div>$/g, '').replace(/>\s+</g, '><');
const whole = (text) => inner(renderToStaticMarkup(h(MarkdownRenderer, { content: text, isStreaming: true })));
const byBlocks = (text) =>
  split(text)
    .map((b, i, all) => inner(renderToStaticMarkup(h(MarkdownRenderer, { content: b, isStreaming: true, cursor: i === all.length - 1 }))))
    .join('');
/** The blocks, rendered one by one, give the same HTML as the whole text. */
const sameAsWhole = (text) => assert.equal(byBlocks(text), whole(text), `renders as one text would:\n${text}`);

test('paragraphs: each one is a block, the last one open', () => {
  assert.deepEqual(split('One.\n\nTwo.\n\nThree is still'), ['One.\n\n', 'Two.\n\n', 'Three is still']);
  assert.deepEqual(split(''), []);
  assert.deepEqual(split('Only one line'), ['Only one line']);
  sameAsWhole('One.\n\nTwo.\n\nThree.\n');
});

test('a code fence with blank lines inside stays one block (``` and ~~~, any length)', () => {
  const text = 'Intro:\n\n```ts\nconst a = 1;\n\nconst b = 2;\n```\n\nAfter it.\n';
  assert.deepEqual(split(text), ['Intro:\n\n', '```ts\nconst a = 1;\n\nconst b = 2;\n```\n\n', 'After it.\n']);
  sameAsWhole(text);
  // A shorter fence does not close a longer one, and a ~~~ fence is not closed by ```.
  const tildes = 'Intro.\n\n~~~~\nx\n\n~~~\n\n```\n\ny\n~~~~\n\nEnd.\n';
  assert.deepEqual(split(tildes), ['Intro.\n\n', '~~~~\nx\n\n~~~\n\n```\n\ny\n~~~~\n\n', 'End.\n']);
  sameAsWhole(tildes);
  // An indented fence inside a list item.
  const nested = '- step:\n\n  ```sh\n  npm test\n\n  npm run build\n  ```\n\nDone.\n';
  assert.deepEqual(split(nested), ['- step:\n\n  ```sh\n  npm test\n\n  npm run build\n  ```\n\n', 'Done.\n']);
  sameAsWhole(nested);
});

test('a fence left open at the end is all in the open block', () => {
  const text = 'Intro.\n\n```py\nx = 1\n\ny = 2\n\n';
  assert.deepEqual(split(text), ['Intro.\n\n', '```py\nx = 1\n\ny = 2\n\n']);
  sameAsWhole(text);
});

test('a loose list stays one block; a tight one ends at its blank line', () => {
  const loose = '- one\n\n- two\n\n- three\n\nDone.\n';
  assert.deepEqual(split(loose), ['- one\n\n- two\n\n- three\n\n', 'Done.\n']);
  sameAsWhole(loose);
  assert.match(whole(loose), /<li[^>]*><p/, 'loose: items hold paragraphs');
  const tight = 'Plan:\n\n1. read\n2. fix\n3. test\n\nThen ship.\n';
  assert.deepEqual(split(tight), ['Plan:\n\n', '1. read\n2. fix\n3. test\n\n', 'Then ship.\n']);
  sameAsWhole(tight);
});

test('an ordered list with a start number keeps it, loose or tight', () => {
  const text = 'From step 3:\n\n3. three\n\n4. four\n\n5) five\n\nEnd.\n';
  const blocks = split(text);
  // "5)" starts a new list (another delimiter); it stays in the same block, which is never wrong.
  assert.equal(blocks[1], '3. three\n\n4. four\n\n5) five\n\n');
  sameAsWhole(text);
});

test('a nested list and an item continued after a blank line stay with their list', () => {
  const text = '- a\n  - b\n\n  - c\n- d\n\n  more about d\n\nAfter.\n';
  assert.deepEqual(split(text), ['- a\n  - b\n\n  - c\n- d\n\n  more about d\n\n', 'After.\n']);
  sameAsWhole(text);
});

test('a table is one block, and a row after a blank line does not split from it', () => {
  const text = 'Totals:\n\n| Month | Total |\n|---|---:|\n| Jan | 2,310.00 |\n| Feb | 1,980.50 |\n\nMarch is short.\n';
  assert.deepEqual(split(text), ['Totals:\n\n', '| Month | Total |\n|---|---:|\n| Jan | 2,310.00 |\n| Feb | 1,980.50 |\n\n', 'March is short.\n']);
  sameAsWhole(text);
  assert.equal(split('| a | b |\n|---|---|\n| 1 | 2 |\n\n| 3 | 4 |\n').length, 1);
});

test('a block quote goes on across a blank line; indented code is not cut', () => {
  const quote = '> one\n\n> two\n\nAfter.\n';
  assert.deepEqual(split(quote), ['> one\n\n> two\n\n', 'After.\n']);
  sameAsWhole(quote);
  const indented = 'Text.\n\n    code\n\n    more\n\nEnd.\n';
  assert.deepEqual(split(indented), ['Text.\n\n    code\n\n    more\n\n', 'End.\n']);
  sameAsWhole(indented);
});

test('when unsure it does not split: a line still undecided, and reference definitions', () => {
  // "2" could still become "2. item" of the list above.
  assert.deepEqual(split('- a\n\n2'), ['- a\n\n2']);
  assert.deepEqual(split('- a\n\n2. b'), ['- a\n\n2. b']);
  // A first word typed out says enough.
  assert.deepEqual(split('Para.\n\nNext words'), ['Para.\n\n', 'Next words']);
  // A link reference or footnote definition can reach back to an earlier block.
  assert.deepEqual(split('See [the spec].\n\n[the spec]: https://example.com\n'), ['See [the spec].\n\n[the spec]: https://example.com\n']);
  assert.equal(split('A note[^1].\n\nMore.\n\n[^1]: The note.\n').length, 1);
});

// A reply like the harness streams: prose with a list and a quote, a code block, a task list and a table.
const REPLY = (() => {
  const prose = (i) => `**Step ${i}.** I read the notes. Three things stand out:\n\n1. The *rates cache* is read early.\n2. The list is sorted twice.\n3. The period ends a day early, see [the spec](https://example.com/spec).\n\n> Keep the old order for exports.\n\nI changed the first two.`;
  const code = (i) => `Here is the fix for step ${i}:\n\n\`\`\`ts\nexport function total(rows: Row[]): number {\n  let sum = 0;\n\n  for (const r of rows) sum += r.amount;\n  return sum;\n}\n\`\`\`\n\nRun \`npm test\` after it.`;
  const list = (i) => `Checked ${i} items:\n\n- [x] invoices match\n- [x] totals add up\n- [ ] the March receipt is missing\n\nEverything else is in place.`;
  const table = (i) => `Totals for run ${i}:\n\n| Month | Invoices | Total |\n|---|---:|---:|\n| January | 14 | 2,310.00 |\n| March | 9 | 1,402.75 |\n\nMarch is short one receipt.`;
  let s = '';
  for (let i = 0; i < 12; i++) s += `${[prose, code, list, table][i % 4](i)}\n\n`;
  return s;
})();

test('as a reply grows, finished blocks never change, and the whole reply renders as one text would', () => {
  let finished = [];
  for (let n = 1; n <= REPLY.length; n++) {
    const blocks = split(REPLY.slice(0, n));
    const done = blocks.slice(0, -1);
    assert.deepEqual(done.slice(0, finished.length), finished, `at ${n} characters a finished block changed`);
    finished = done;
  }
  assert.ok(finished.length >= 30, `${finished.length} finished blocks`);
  sameAsWhole(REPLY);
  // And at each line end on the way (a reply cut anywhere mid-stream).
  for (let n = REPLY.indexOf('\n'); n > 0; n = REPLY.indexOf('\n', n + 1)) if (n % 5 === 0) sameAsWhole(REPLY.slice(0, n + 1));
});

test('StreamMarkdown renders each finished block once with its final text; only the open block each frame', () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const markdown = () => renders.filter((r) => /^MarkdownRenderer\d*$/.test(r.name));
  try {
    renders.length = 0;
    let frames = 0;
    for (let n = 40; n < REPLY.length; n += 40) {
      flushSync(() => root.render(h(StreamMarkdown, { content: REPLY.slice(0, n) })));
      frames++;
    }
    flushSync(() => root.render(h(StreamMarkdown, { content: REPLY })));
    frames++;
    const blocks = splitStreamBlocks(REPLY);
    const all = markdown();
    for (const [i, b] of blocks.slice(0, -1).entries()) {
      // Once finished (no cursor) it never renders again; before that it was the open block.
      // (The sample repeats some blocks word for word: each copy renders once.)
      const copies = blocks.slice(0, -1).filter((x) => x === b).length;
      const n = all.filter((r) => r.props.content === b && r.props.cursor === false).length;
      assert.equal(n, copies, `finished block ${i} rendered ${n} times once finished (${copies} copies)`);
    }
    // Each frame: the open block, plus a block that just finished (re-cut once) and one just opened.
    assert.ok(all.length <= frames + 2 * blocks.length, `${all.length} renders for ${frames} frames and ${blocks.length} blocks`);
    assert.ok(all.every((r) => r.props.isStreaming === true), 'streaming blocks render as streaming (no smart results)');
    assert.equal(host.querySelectorAll('.animate-pulse').length, 1, 'one typing cursor, after the open block');
    assert.ok(host.querySelector('.markdown-content:last-child .animate-pulse'));
    assert.equal(host.querySelectorAll('.markdown-content').length, blocks.length);
    assert.equal(host.querySelector('[data-stream-blocks]').getAttribute('data-stream-blocks'), String(blocks.length));
    assert.match(host.textContent, /March is short one receipt\./);
  } finally {
    flushSync(() => root.unmount());
  }
});

test('a paragraph ending a finished block keeps its gap below (the class the blocks wrapper carries)', () => {
  const html = renderToStaticMarkup(h(StreamMarkdown, { content: 'One.\n\nTwo words.' }));
  assert.match(html, /^<div class="\[&amp;&gt;\.markdown-content:not\(:last-child\)&gt;p:last-child\]:mb-3" data-stream-blocks="2">/);
});
