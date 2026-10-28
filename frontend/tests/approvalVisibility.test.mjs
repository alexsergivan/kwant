import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/approvalVisibility';
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

test('following the chat means near the bottom, within the same 80 px as before', () => {
  assert.equal(m.isNearBottom({ scrollHeight: 2000, scrollTop: 1200, clientHeight: 780 }), true);
  assert.equal(m.isNearBottom({ scrollHeight: 2000, scrollTop: 1000, clientHeight: 780 }), false);
});

test('a card that just started waiting is new; one already shown is not', () => {
  assert.deepEqual(m.newApprovalIds(new Set(['a']), ['a', 'b']), ['b']);
  assert.deepEqual(m.newApprovalIds(new Set(['a', 'b']), ['a', 'b']), []);
});

test('buttons count as visible only when the whole row is inside the stream', () => {
  const view = { top: 60, bottom: 760 };
  assert.equal(m.fullyVisible({ top: 700, bottom: 740 }, view), true);
  assert.equal(m.fullyVisible({ top: 740, bottom: 780 }, view), false, 'cut by the composer');
  assert.equal(m.fullyVisible({ top: 20, bottom: 70 }, view), false, 'scrolled above');
});

test('following: keep the bottom pinned, also when a waiting card grows afterwards', () => {
  assert.equal(m.approvalScrollAction({ waiting: 1, following: true, buttonsVisible: false }), 'follow');
  assert.equal(m.approvalScrollAction({ waiting: 0, following: true, buttonsVisible: false }), 'follow');
});

test('scrolled up to read: never steal the scroll, offer "Approval waiting" instead', () => {
  assert.equal(m.approvalScrollAction({ waiting: 1, following: false, buttonsVisible: false }), 'notify');
  assert.equal(m.approvalScrollAction({ waiting: 1, following: false, buttonsVisible: true }), 'none');
  assert.equal(m.approvalScrollAction({ waiting: 0, following: false, buttonsVisible: false }), 'none');
});

test('a card marks itself and its button row so the chat can find them', () => {
  const html = m.renderToStaticMarkup(
    m.createElement(m.ModernApprovalCard, {
      approval: { id: 'pay-1', tool_name: 'x', details: { payment_reason: 'charges a card' } },
      onResolve: () => {},
    }),
  );
  assert.match(html, /data-approval-card="pay-1"/);
  assert.match(html, /data-approval-actions="true"[^>]*>[\s\S]*Deny[\s\S]*Allow Once/);
});

test('only scrolling up stops following; a card growing under a pinned view does not', () => {
  // Card grew by 300 px before the view was re-pinned: not near the bottom, but nobody scrolled.
  assert.equal(m.nextFollowing(true, { nearBottom: false, scrollTop: 1200, lastScrollTop: 1200 }), true);
  assert.equal(m.nextFollowing(true, { nearBottom: false, scrollTop: 900, lastScrollTop: 1200 }), false, 'scrolled up');
  assert.equal(m.nextFollowing(false, { nearBottom: false, scrollTop: 1000, lastScrollTop: 900 }), false, 'still reading');
  assert.equal(m.nextFollowing(false, { nearBottom: true, scrollTop: 1500, lastScrollTop: 900 }), true, 'back at the bottom');
});
