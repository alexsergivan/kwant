// 22o: the bot browser's preview flashed light grey while a page swapped in. Chrome's screencast sends a
// blank white picture while a tab commits a new page (before the page paints its own background); the
// pane showed it. The live view now keeps the last good picture while such a frame is in, for at most
// 1.5 s, so a page that really is white still shows. These tests fail if the blank frame shows again.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { isBlankLight, steadyDecision, useSteadyFrame, HOLD_LIGHT_MS, PROBE_MAX_CHARS } from './src/steadyFrame';
`);
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };

const pixels = (fill) => { const a = new Uint8ClampedArray(32 * 20 * 4); for (let i = 0; i < a.length; i += 4) { const [r, gg, b] = fill(i / 4); a[i] = r; a[i + 1] = gg; a[i + 2] = b; a[i + 3] = 255; } return a; };

test('a flat white or light grey picture is blank; a dark page or a white page with content is not', () => {
  assert.equal(m.isBlankLight(pixels(() => [255, 255, 255])), true, "Chrome's blank frame");
  assert.equal(m.isBlankLight(pixels(() => [0xbf, 0xbf, 0xbf])), true, 'the #bfbfbf Mira saw');
  assert.equal(m.isBlankLight(pixels((i) => (i % 7 ? [252, 252, 252] : [246, 246, 246]))), true, 'JPEG noise on white');
  assert.equal(m.isBlankLight(pixels(() => [17, 20, 42])), false, 'the dark tent page');
  assert.equal(m.isBlankLight(pixels((i) => (i % 32 < 6 && i < 200 ? [30, 30, 30] : [255, 255, 255]))), false, 'a white page with a logo');
});

test('a blank frame is held, but never past 1.5 s', () => {
  assert.equal(m.HOLD_LIGHT_MS, 1500);
  assert.equal(m.steadyDecision(false, null, 0), 'show');
  assert.equal(m.steadyDecision(true, null, 0), 'hold');
  assert.equal(m.steadyDecision(true, 1000, 2499), 'hold');
  assert.equal(m.steadyDecision(true, 1000, 2500), 'show', 'a page that is really white shows');
});

test('the live view keeps the last good picture while a blank one is in, and acks the held one', async () => {
  const blank = new Set(['WHITE', 'WHITE2']);
  const probe = async (image) => blank.has(image);
  const acks = [];
  const ack = (id) => acks.push(id);
  let out;
  function Pane({ frame }) { out = m.useSteadyFrame(frame, ack, probe); return null; }
  const root = m.createRoot(document.createElement('div'));
  const show = async (frame) => { m.flushSync(() => root.render(m.React.createElement(Pane, { frame }))); await settle(); };
  const f = (id, image) => ({ id, image, width: 1280, height: 800 });

  await show(f(1, 'PAGE'));
  assert.equal(out.frame?.image, 'PAGE');
  await show(f(2, 'WHITE'));
  assert.equal(out.frame?.image, 'PAGE', 'the blank frame never reaches the <img>');
  assert.deepEqual(acks, [2], 'held, so the hook acknowledges it (no image load will)');
  await show(f(3, 'PAGE'));
  assert.equal(out.frame?.image, 'PAGE');
  assert.deepEqual(acks, [2, 3], 'the same picture again after a held one: no image load, so the hook acks it');
  await show(f(6, 'PAGE2'));
  assert.equal(out.frame?.image, 'PAGE2', 'the next real picture shows at once');
  assert.deepEqual(acks, [2, 3], 'a new picture is acked by its image load, not here');

  // A fresh stream (another tab) whose first picture is blank: the dark pane, not white, not "Browser idle"
  await show(null);
  assert.equal(out.frame, null);
  await show(f(4, 'WHITE2'));
  assert.equal(out.frame, null);
  assert.equal(out.holding, true);
  await new Promise((r) => setTimeout(r, m.HOLD_LIGHT_MS + 100));
  await settle();
  assert.equal(out.frame?.image, 'WHITE2', 'after 1.5 s a white page shows: nothing is held forever');

  // A big picture is a real page: it never waits for a check
  const big = 'x'.repeat(m.PROBE_MAX_CHARS + 1);
  m.flushSync(() => root.render(m.React.createElement(Pane, { frame: f(5, big) })));
  assert.equal(out.frame?.image, big);
  m.flushSync(() => root.unmount());
});

test('every browser view gets its picture through the steady frame, on the dark surface', () => {
  const src = readFileSync(new URL('../src/components/VirtualMachinePane.tsx', import.meta.url), 'utf8');
  assert.match(src, /const steady = useSteadyFrame\(stream\.frame, stream\.acknowledgeFrame\)/);
  assert.match(src, /const currentScreenshot = isBlankPage\(pageUrl\) \? null : steady\.frame\?\.image/);
  assert.match(src, /paneFrameId\.current = steady\.frame\?\.id/);
  assert.doesNotMatch(src, /stream\.frame\?\.image/, 'no view reads the raw frame picture');
  const watch = src.slice(src.indexOf('data-browser-watch'), src.indexOf('data-browser-watch') + 400);
  assert.match(watch, /bg-\[#0a0719\]/, 'the pane behind the picture stays dark');
  assert.doesNotMatch(watch, /bg-(white|zinc-[1-4]00|gray-|neutral-[1-4])/);
});
