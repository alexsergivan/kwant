// chat-scroll-holds-place: while the person reads (scrolled up by their own input), nothing moves the messages but
// their own input or a jump they pressed. The rules are pure (approvalVisibility); ChatView wires them to input.
// The whole behaviour is measured in real Chromium and WebKit by perf/scroll/probe.mjs (see perf/scroll/README.md).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/approvalVisibility';`);
const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');

test('a small step up stops following, even within 80 px of the bottom', () => {
  // A 20 px wheel step used to count as following, so the next streamed word pulled the reader back down.
  assert.equal(m.nextFollowing(true, { nearBottom: true, atBottom: false, scrollTop: 980, lastScrollTop: 1000, user: true }), false);
  assert.equal(m.nextFollowing(true, { nearBottom: true, atBottom: false, scrollTop: 990, lastScrollTop: 1000 }), false, 'unknown source counts as theirs');
});

test('content shrinking under a view pinned to the bottom is a clamp, not reading', () => {
  assert.equal(m.nextFollowing(true, { nearBottom: true, atBottom: true, scrollTop: 900, lastScrollTop: 1000, user: false }), true);
  // The same move made by the person stops following.
  assert.equal(m.nextFollowing(true, { nearBottom: true, atBottom: true, scrollTop: 900, lastScrollTop: 1000, user: true }), false);
});

test('reading, only the person scrolling down near the bottom, or the very bottom, starts following again', () => {
  assert.equal(m.nextFollowing(false, { nearBottom: true, atBottom: false, scrollTop: 1000, lastScrollTop: 900, user: true }), true, 'their scroll down');
  assert.equal(m.nextFollowing(false, { nearBottom: true, atBottom: false, scrollTop: 1000, lastScrollTop: 900, user: false }), false, 'the page holding their place');
  assert.equal(m.nextFollowing(false, { nearBottom: true, atBottom: true, scrollTop: 1000, lastScrollTop: 1000, user: false }), true, 'at the bottom');
  assert.equal(m.nextFollowing(false, { nearBottom: false, atBottom: false, scrollTop: 600, lastScrollTop: 500, user: true }), false, 'still far up');
});

test('at the bottom means within 2 px (sub-pixel positions)', () => {
  assert.equal(m.isAtBottom({ scrollHeight: 2000, scrollTop: 1218.5, clientHeight: 780 }), true);
  assert.equal(m.isAtBottom({ scrollHeight: 2000, scrollTop: 1210, clientHeight: 780 }), false);
});

test('which way the person scrolls, seen from their input', () => {
  assert.equal(m.scrollIntent({ kind: 'wheel', deltaY: -12 }), 'up');
  assert.equal(m.scrollIntent({ kind: 'wheel', deltaY: 40 }), 'down');
  assert.equal(m.scrollIntent({ kind: 'wheel', deltaY: 0 }), null);
  assert.equal(m.scrollIntent({ kind: 'touch', dy: 18 }), 'up', 'finger down the screen reads older messages');
  assert.equal(m.scrollIntent({ kind: 'touch', dy: -18 }), 'down');
  assert.equal(m.scrollIntent({ kind: 'key', key: 'PageUp' }), 'up');
  assert.equal(m.scrollIntent({ kind: 'key', key: 'Home' }), 'up');
  assert.equal(m.scrollIntent({ kind: 'key', key: ' ', shiftKey: true }), 'up');
  assert.equal(m.scrollIntent({ kind: 'key', key: 'End' }), 'down');
  assert.equal(m.scrollIntent({ kind: 'key', key: 'ArrowUp', editable: true }), null, 'typing in the composer');
  assert.equal(m.scrollIntent({ kind: 'key', key: 'a' }), null);
});

test('F1: an up-input that scrolls nothing (short chat, or at the top) keeps following', () => {
  // A wheel up, finger drag down or PageUp/Home on a chat that fits the screen makes no scroll event, so nothing
  // would turn following back on and the first reply streamed past the fold (Rigel, 1015 px gap).
  assert.equal(m.inputStopsFollowing('up', 0), false, 'nothing above to move to');
  assert.equal(m.inputStopsFollowing('up', 0.4), true, 'any room above: the scroll event follows');
  assert.equal(m.inputStopsFollowing('up', 300), true);
  assert.equal(m.inputStopsFollowing('down', 300), false);
  assert.equal(m.inputStopsFollowing(null, 300), false);
});

test('holding the place: the reader’s message goes back where it was, unless the scroller itself moved', () => {
  assert.equal(m.holdPlaceBy({ top: 300, atScrollTop: 5000 }, { top: 449, scrollTop: 5000 }), 149, 'a row above settled 149 px taller');
  assert.equal(m.holdPlaceBy({ top: 300, atScrollTop: 5000 }, { top: 227, scrollTop: 5000 }), -73, 'a row above settled shorter');
  assert.equal(m.holdPlaceBy({ top: 300, atScrollTop: 5000 }, { top: 300.3, scrollTop: 5000 }), 0, 'sub-pixel');
  assert.equal(m.holdPlaceBy({ top: 300, atScrollTop: 5000 }, { top: 449, scrollTop: 5149 }), 0, 'the browser anchored it already');
});

test('lazy rows (33q) only where they hold still: not WebKit, even with scroll anchoring', () => {
  const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
  assert.equal(m.lazyRowsHold(mac), false, 'the Mac app');
  assert.equal(m.lazyRowsHold(`${mac} Version/18.2 Safari/605.1.15`), false, 'Safari');
  assert.equal(m.lazyRowsHold('Mozilla/5.0 (iPhone; CPU iPhone OS 18_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/131.0 Mobile/15E148 Safari/604.1'), false, 'Chrome on iPhone is WebKit');
  assert.equal(m.lazyRowsHold('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0'), true, 'the Windows app');
  assert.equal(m.lazyRowsHold('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'), true, 'Chrome');
  assert.equal(m.lazyRowsHold('Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0'), true, 'Firefox');
  assert.match(css, /:not\(\.kw-no-lazy\) > \.kw-row-lazy/);
  assert.match(chat, /LAZY_ROWS \? '' : 'kw-no-lazy'/);
});

test('ChatView stops following on the person’s own input, before the scroll it makes', () => {
  for (const ev of ['wheel', 'touchmove', 'pointerdown']) assert.match(chat, new RegExp(`el\\.addEventListener\\('${ev}'`), ev);
  assert.match(chat, /document\.addEventListener\('keydown', onKey/);
  assert.match(chat, /if \(inputStopsFollowing\(dir, el\.scrollTop\)\) stickRef\.current = false;/, 'F1: only when the chat can move up');
  assert.match(chat, /jumpEndRef\.current\?\.\(\);/, 'their input ends a running jump without its late settle');
  assert.match(chat, /user: userScrolling\(\)/);
});

test('ChatView holds the reader’s place when rows change height, and 33k restores it', () => {
  assert.match(chat, /else holdPlace\(\);\s*checkApprovalVisible\(\);/, 'scroller resize');
  assert.match(chat, /else if \(!stickRef\.current\) holdPlace\(\);/, 'row resize, also a row’s first callback');
  assert.match(chat, /if \(p && placeRow\) el\.scrollTop \+= placeRow\.getBoundingClientRect\(\)\.top/, 'load earlier restores the reader’s message');
});
