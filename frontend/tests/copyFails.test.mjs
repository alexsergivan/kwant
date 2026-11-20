// 31s copy-says-if-it-failed: the code-block Copy and the reply Copy wait for the clipboard. "Copied" only when it
// took the text; "Couldn't copy" (same 2 s, same spot, same polite live line) when it refused or there is none.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://192.168.1.20:8787/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const copied = [];
const clipboards = {
  works: { writeText: async (t) => void copied.push(t) },
  refuses: { writeText: async () => { throw new DOMException('Write permission denied.', 'NotAllowedError'); } },
  missing: undefined,
};
const useClipboard = (kind) =>
  Object.defineProperty(dom.window.navigator, 'clipboard', { value: clipboards[kind], configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { CopyGhost } from './src/components/CopyGhost';
  export { copyText } from './src/copyText';
`);
const { React, createRoot, flushSync, MarkdownRenderer, CopyGhost, copyText } = m;
const h = React.createElement;
const tick = () => new Promise((r) => setTimeout(r, 0));

const draw = (el) => {
  const box = document.createElement('div');
  document.body.appendChild(box);
  const root = createRoot(box);
  flushSync(() => root.render(el));
  return { box, done: () => { root.unmount(); box.remove(); } };
};
const click = (b) => b.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
const codeCopy = (box) => box.querySelector('button[title="Copy code"]');
const live = (box) => box.querySelector('[aria-live="polite"]');

test('copyText: true when the clipboard takes the text, false when it refuses or is missing', async () => {
  useClipboard('works');
  assert.equal(await copyText('a'), true);
  assert.equal(copied.at(-1), 'a');
  useClipboard('refuses');
  assert.equal(await copyText('b'), false);
  useClipboard('missing');
  assert.equal(await copyText('c'), false);
});

const fence = 'Run this:\n\n```bash\nnpm test\n```\n';

for (const [kind, word, cls] of [
  ['works', 'Copied', 'text-ok'],
  ['refuses', 'Couldn’t copy', 'text-bad'],
  ['missing', 'Couldn’t copy', 'text-bad'],
]) {
  test(`code-block Copy, clipboard ${kind}: says "${word}" and announces it, then back to Copy after 2 s`, async () => {
    useClipboard(kind);
    const { box, done } = draw(h(MarkdownRenderer, { content: fence }));
    try {
      const btn = codeCopy(box);
      assert.equal(btn.textContent.trim(), 'Copy');
      assert.equal(live(box).textContent, '', 'nothing announced before a click');
      const realTimeout = g.setTimeout;
      const timers = [];
      g.setTimeout = (fn, ms) => (ms === 2000 ? timers.push(fn) : realTimeout(fn, ms));
      try {
        click(btn);
        await tick();
        await tick();
      } finally {
        g.setTimeout = realTimeout;
      }
      assert.equal(btn.textContent.trim(), word);
      assert.ok(btn.querySelector(`span.${cls}`), `${word} uses ${cls}`);
      assert.equal(live(box).textContent, word, 'the polite live line says it once');
      assert.ok(!btn.contains(live(box)), 'the live line is outside the button, so its name stays one word');
      if (kind === 'works') assert.equal(copied.at(-1), 'npm test');
      assert.equal(timers.length, 1, 'one 2 s timer');
      flushSync(() => timers[0]());
      assert.equal(btn.textContent.trim(), 'Copy');
      assert.equal(live(box).textContent, '');
    } finally {
      done();
    }
  });
}

test('code-block Copy: 28 px or more on desktop by padding only (py-1.5 with -my-1 keeps the header row), phone unchanged', () => {
  useClipboard('works');
  const { box, done } = draw(h(MarkdownRenderer, { content: fence }));
  try {
    const c = codeCopy(box).className.split(/\s+/);
    for (const k of ['py-0.5', 'sm:py-1.5', 'sm:-my-1']) assert.ok(c.includes(k), k);
    assert.ok(!c.some((k) => /^(h|min-h)-/.test(k)), 'no fixed height');
  } finally {
    done();
  }
});

test('reply Copy (CopyGhost): Copied, Couldn’t copy, idle', () => {
  const at = (copied) => {
    const { box, done } = draw(h(CopyGhost, { copied, tipId: 'tip', onCopy() {} }));
    const out = { tip: box.querySelector('[role="tooltip"]').textContent, live: live(box).textContent, ok: !!box.querySelector('.text-ok'), bad: !!box.querySelector('.text-bad'), name: box.querySelector('button').getAttribute('aria-label') };
    done();
    return out;
  };
  assert.deepEqual(at('ok'), { tip: 'Copied', live: 'Copied', ok: true, bad: false, name: 'Copy reply' });
  assert.deepEqual(at('failed'), { tip: 'Couldn’t copy', live: 'Couldn’t copy', ok: false, bad: true, name: 'Copy reply' });
  assert.deepEqual(at(null), { tip: 'Copy reply', live: '', ok: false, bad: false, name: 'Copy reply' });
});

test('ChatView: the reply Copy waits for copyText and passes ok or failed to CopyGhost; sound only on success', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const fn = src.slice(src.indexOf('const handleCopy = async (text: string, id: string)'), src.indexOf('const patchUpload'));
  assert.match(fn, /const ok = await copyText\(text\);/);
  assert.match(fn, /setCopiedId\(\{ id, ok \}\);/);
  assert.match(fn, /if \(ok\) playSfx\('click'\);/);
  assert.match(fn, /setTimeout\(\(\) => setCopiedId\(null\), 2000\);/);
  assert.doesNotMatch(fn, /navigator\.clipboard/);
  assert.match(src, /<CopyGhost copied=\{copiedId\?\.id === msg\.id \? \(copiedId\.ok \? 'ok' : 'failed'\) : null\}/);
});
