// 31u code-copy-focus-ring: the code-block Copy shows CopyGhost's keyboard focus ring, and both Copy buttons keep one
// 2 s result timer: a second click restarts the window (the first click's timer can't end it early), unmount clears it.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost:8787/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
Object.defineProperty(dom.window.navigator, 'clipboard', { value: { writeText: async () => {} }, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
`);
const { React, createRoot, flushSync, MarkdownRenderer } = m;
const h = React.createElement;
const realSet = g.setTimeout;
const realClear = g.clearTimeout;
const tick = () => new Promise((r) => realSet(r, 0));

// A fake clock for the 2 s result timers only; everything else (React's scheduler, tick) keeps real timers.
const clock = () => {
  let now = 0;
  let next = 1;
  const pending = new Map();
  g.setTimeout = (fn, ms, ...a) => {
    if (ms !== 2000) return realSet(fn, ms, ...a);
    const id = { fake: next++ };
    pending.set(id, { at: now + ms, fn });
    return id;
  };
  g.clearTimeout = (id) => (id && typeof id === 'object' && 'fake' in id ? void pending.delete(id) : realClear(id));
  return {
    pending,
    advance(ms) {
      now += ms;
      for (const [id, t] of [...pending].sort((a, b) => a[1].at - b[1].at)) {
        if (t.at > now || !pending.has(id)) continue;
        pending.delete(id);
        flushSync(() => t.fn());
      }
    },
    restore() {
      g.setTimeout = realSet;
      g.clearTimeout = realClear;
    },
  };
};

const fence = 'Run this:\n\n```bash\nnpm test\n```\n';
const draw = () => {
  const box = document.createElement('div');
  document.body.appendChild(box);
  const root = createRoot(box);
  flushSync(() => root.render(h(MarkdownRenderer, { content: fence })));
  return { box, btn: box.querySelector('button[title="Copy code"]'), done: () => { flushSync(() => root.unmount()); box.remove(); } };
};
const click = async (b) => {
  b.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await tick();
  await tick();
};

test('code-block Copy has CopyGhost’s focus-visible ring; nothing else about its size changes', () => {
  const { btn, done } = draw();
  try {
    const c = btn.className.split(/\s+/);
    for (const k of ['focus-visible:outline-none', 'focus-visible:ring-2', 'focus-visible:ring-sky-400']) assert.ok(c.includes(k), k);
    for (const k of ['px-2', 'py-0.5', 'sm:py-1.5', 'sm:-my-1', 'rounded']) assert.ok(c.includes(k), `${k} kept`);
    const ghost = readFileSync(new URL('../src/components/CopyGhost.tsx', import.meta.url), 'utf8');
    assert.match(ghost, /focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400/, 'same classes as the reply Copy');
  } finally {
    done();
  }
});

test('code-block Copy: a second click 1.5 s after the first keeps "Copied" until 2 s after the second', async () => {
  const c = clock();
  const { btn, done } = draw();
  try {
    await click(btn);
    assert.equal(btn.textContent.trim(), 'Copied');
    c.advance(1500);
    await click(btn);
    assert.equal(c.pending.size, 1, 'one result timer, not two');
    c.advance(500); // 2.0 s after the first click
    assert.equal(btn.textContent.trim(), 'Copied', 'the first click’s timer does not end the second window');
    c.advance(1400); // 1.9 s after the second
    assert.equal(btn.textContent.trim(), 'Copied');
    c.advance(100); // 2.0 s after the second
    assert.equal(btn.textContent.trim(), 'Copy');
  } finally {
    done();
    c.restore();
  }
});

test('code-block Copy: unmount clears the pending result timer', async () => {
  const c = clock();
  const { btn, done } = draw();
  try {
    await click(btn);
    assert.equal(c.pending.size, 1);
    done();
    assert.equal(c.pending.size, 0, 'cleared on unmount');
  } finally {
    c.restore();
  }
});

test('ChatView reply Copy: one timer ref for the list, restarted by each click, cleared on unmount', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const fn = src.slice(src.indexOf('const handleCopy = async (text: string, id: string)'), src.indexOf('const patchUpload'));
  assert.match(fn, /clearTimeout\(copyTimer\.current\);\s*copyTimer\.current = setTimeout\(\(\) => setCopiedId\(null\), 2000\);/);
  assert.match(src, /const copyTimer = useRef<ReturnType<typeof setTimeout>>\(\);/);
  assert.match(src, /useEffect\(\(\) => \(\) => clearTimeout\(copyTimer\.current\), \[\]\);/);
});
