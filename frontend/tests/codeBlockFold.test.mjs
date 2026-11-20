// 31l long-log-folds: a code or log block over 20 lines folds to 16 lines with one "Show all N lines" /
// "Show less" toggle under it. Copy still copies every line; blocks of 20 lines or fewer render as before.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
const copied = [];
Object.defineProperty(dom.window.navigator, 'clipboard', { value: { writeText: async (t) => void copied.push(t) }, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
`);
const { React, createRoot, flushSync, MarkdownRenderer } = m;
const h = React.createElement;

const lines = (n, label = 'PASS test') => Array.from({ length: n }, (_, i) => `${label} ${i + 1}`).join('\n');
const fence = (body, lang = 'text') => `Fixed it. Now all 418 pass.\n\n\`\`\`${lang}\n${body}\n\`\`\`\n\nAfter the log.\n`;

const draw = (content) => {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  flushSync(() => root.render(h(MarkdownRenderer, { content })));
  return { el, root, done: () => { root.unmount(); el.remove(); } };
};
const toggleOf = (el) => [...el.querySelectorAll('button')].filter((b) => b.hasAttribute('aria-expanded'));
const click = (b) => flushSync(() => b.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true })));

test('a 69-line log folds to 16 lines with "Show all 69 lines", and opens to the full log with "Show less"', () => {
  const body = lines(69);
  const { el, done } = draw(fence(body));
  try {
    const pre = el.querySelector('pre');
    assert.equal(pre.querySelector('code').textContent, body, 'all 69 lines are in the DOM (folded by height only)');
    assert.match(pre.className, /\bmax-h-80\b/, 'folded to 320 px = 16 lines');
    assert.match(pre.className, /\boverflow-y-hidden\b/);
    const fade = pre.parentElement.querySelector('[aria-hidden="true"]');
    assert.ok(fade, 'a decorative fade sits at the cut');
    const [btn, ...more] = toggleOf(el);
    assert.ok(btn, 'one toggle under the block');
    assert.equal(more.length, 0, 'only one toggle');
    assert.equal(btn.getAttribute('type'), 'button');
    assert.equal(btn.getAttribute('aria-expanded'), 'false');
    assert.equal(btn.textContent.trim(), 'Show all 69 lines');

    click(btn);
    assert.equal(btn.getAttribute('aria-expanded'), 'true');
    assert.equal(btn.textContent.trim(), 'Show less');
    assert.doesNotMatch(el.querySelector('pre').className, /max-h-80|overflow-y-hidden/, 'open: full height');
    assert.equal(el.querySelector('pre').parentElement.querySelector('[aria-hidden="true"]'), null, 'no fade when open');

    click(btn);
    assert.equal(btn.getAttribute('aria-expanded'), 'false');
    assert.equal(btn.textContent.trim(), 'Show all 69 lines');
    assert.match(el.querySelector('pre').className, /\bmax-h-80\b/);
  } finally {
    done();
  }
});

test('Copy on a folded 69-line log copies all 69 lines', () => {
  copied.length = 0;
  const body = lines(69);
  const { el, done } = draw(fence(body));
  try {
    const copy = [...el.querySelectorAll('button')].find((b) => b.getAttribute('title') === 'Copy code');
    click(copy);
    assert.equal(copied.length, 1);
    assert.equal(copied[0], body);
    assert.equal(copied[0].split('\n').length, 69);
  } finally {
    done();
  }
});

test('a 9-line diff and a 20-line block have no toggle and no fold; 21 lines folds', () => {
  for (const [n, lang] of [[9, 'diff'], [20, 'text'], [20, '']]) {
    const { el, done } = draw(fence(lines(n, '+ line'), lang));
    try {
      assert.equal(toggleOf(el).length, 0, `${n} lines (${lang || 'no language'}): no toggle`);
      assert.doesNotMatch(el.querySelector('pre').className, /max-h-80|overflow-y-hidden/);
      assert.equal(el.querySelector('pre').parentElement.querySelector('[aria-hidden="true"]'), null);
    } finally {
      done();
    }
  }
  const { el, done } = draw(fence(lines(21)));
  try {
    assert.equal(toggleOf(el)[0]?.textContent.trim(), 'Show all 21 lines');
  } finally {
    done();
  }
});
