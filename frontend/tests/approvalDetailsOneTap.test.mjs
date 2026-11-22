// 34c approval-details-one-tap (brief: autonomy/approval-details-one-tap/brief.md). On a phone, the first tap on an
// approval card's "Show details" did nothing; the second opened it. Repro at 390 px on the 0.9.42 build (impl.md):
// the card never remounted. The tap took focus from the composer, and leaving keyboard mode on that blur (header
// 48 -> 56 px, thread bar back, list pinned to the bottom) moved the button 48 px down between the press and the
// release, so the release and the click landed on the card above it. Now keyboard mode ends after the tap that
// took the focus (its mouseup, then one task), so the click reaches the button the finger pressed.
// JSDOM has no layout, so this pins the order: still in keyboard mode when the tap's click arrives, out right after.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'MutationObserver', 'DOMParser', 'localStorage', 'sessionStorage', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;
// A phone with a touch screen unless a test says otherwise.
const media = { phone: true, coarse: true };
dom.window.matchMedia = (q) => ({
  matches: q.includes('pointer: coarse') ? media.coarse : q.includes('max-width') ? media.phone : q.includes('min-width') ? !media.phone : false,
  media: q,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.ResizeObserver ??= globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.IntersectionObserver ??= globalThis.IntersectionObserver;
dom.window.HTMLElement.prototype.scrollIntoView = function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
if (!globalThis.CSS) globalThis.CSS = { escape: (s) => String(s) };

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ChatView, q)));
    render(p);
    return { host, render, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const act = (fn) => flushSync(fn);
`);

const approval = { id: 'ap1', tool_name: 'execute_shell', details: { source: 'local_shell', tool: 'execute_shell', arguments: { command: 'npm install left-pad', cwd: '/tmp/shop' }, kwant_name: 'Ann' } };
const props = (extra = {}) => ({
  title: 'Ann', subtitle: '', messages: [{ id: 'u1', sender_role: 'user', sender_id: 'me', text: 'Pad the order numbers', created_at: '2026-11-22T17:00:00Z', tokens_used: 0 }],
  streamingText: '', streamingThinking: '', isStreaming: true, pendingApprovals: [approval],
  onSendMessage() {}, onResolveApproval() {}, kwants: [], teams: [], onHighlightDone() {}, threadLine: 'Direct chat',
  ...extra,
});
const tick = (ms = 20) => new Promise((r) => setTimeout(r, ms));
/** Waits until `cond()` holds or 3 s pass (a fixed wait flaked on a busy machine: Rigel's 34c QA, F1). */
const until = async (cond) => { for (const t0 = Date.now(); !cond() && Date.now() - t0 < 3000; ) await tick(10); return cond(); };
const kbdOn = (h) => !!h.querySelector('[data-keyboard-mode="on"]');
const toggle = (h) => h.querySelector('[data-approval-details-toggle]');
const opened = (h) => !!h.querySelector('[data-approval-details]');
const mouse = (el, type) => el.dispatchEvent(new dom.window.MouseEvent(type, { bubbles: true, cancelable: true, view: dom.window }));

/** A finger's tap on `el` while the composer has focus, in the order a phone browser sends it: the press takes the
 * focus (mousedown, then the composer's blur), then mouseup and click on whatever is under the finger then. */
function tapFromComposer(h, el) {
  const ta = h.querySelector('textarea[data-composer]');
  ui.act(() => { mouse(el, 'mousedown'); ta.blur(); });
  const atClick = { kbd: kbdOn(h) };
  ui.act(() => { mouse(el, 'mouseup'); mouse(el, 'click'); });
  return atClick;
}

test('34c: on a phone, one tap on Show details while typing opens it: keyboard mode lasts until the tap is done', async () => {
  media.phone = true;
  media.coarse = true;
  const v = ui.mount(props());
  try {
    assert.equal(opened(v.host), false, 'closed by default on a phone');
    assert.equal(toggle(v.host).textContent, 'Show details');
    ui.act(() => v.host.querySelector('textarea[data-composer]').focus());
    assert.ok(kbdOn(v.host), 'keyboard mode while the composer has focus');

    const at = tapFromComposer(v.host, toggle(v.host));
    assert.equal(at.kbd, true, 'the layout holds still between the press and the release: no keyboard-mode change inside the tap, so the click lands on the button pressed');
    assert.equal(opened(v.host), true, 'one tap opened the details');
    assert.equal(toggle(v.host).getAttribute('aria-expanded'), 'true');

    assert.equal(await until(() => !kbdOn(v.host)), true, 'keyboard mode ends right after the tap');
    assert.ok(v.host.querySelector('[data-chat-header]').textContent.includes('Direct chat'), 'the header line is back');
    assert.equal(opened(v.host), true, 'still open once the layout settles');

    // The same approval again, as a refetch delivers it, and the turn ending: still open.
    v.render(props({ pendingApprovals: [{ ...approval }] }));
    v.render(props({ pendingApprovals: [{ ...approval }], isStreaming: false }));
    await tick();
    assert.equal(opened(v.host), true, 'stays open across re-renders of the same approval');
  } finally {
    v.unmount();
  }
});

test('34c: focus coming back to the composer within the tap keeps keyboard mode on', async () => {
  media.phone = true;
  media.coarse = true;
  const v = ui.mount(props());
  try {
    const ta = v.host.querySelector('textarea[data-composer]');
    ui.act(() => ta.focus());
    ui.act(() => { mouse(toggle(v.host), 'mousedown'); ta.blur(); });
    ui.act(() => ta.focus());
    ui.act(() => mouse(document.body, 'mouseup'));
    await tick(150); // long enough for the deferred leave to have run, even on a busy machine
    assert.equal(kbdOn(v.host), true, 'the composer has focus again, so keyboard mode stays');
  } finally {
    v.unmount();
  }
});

test('34c: a blur that is not a tap (Tab, script) still leaves keyboard mode at once, and the defaults stay', async () => {
  media.phone = true;
  media.coarse = true;
  const v = ui.mount(props());
  try {
    const ta = v.host.querySelector('textarea[data-composer]');
    ui.act(() => ta.focus());
    ui.act(() => ta.blur());
    assert.equal(kbdOn(v.host), false, 'off at once after a plain blur');
  } finally {
    v.unmount();
  }
  media.phone = false;
  media.coarse = false;
  const d = ui.mount(props());
  try {
    assert.equal(opened(d.host), true, 'desktop: details open by default');
    assert.equal(toggle(d.host).textContent, 'Hide details');
  } finally {
    d.unmount();
    media.phone = true;
    media.coarse = true;
  }
});
