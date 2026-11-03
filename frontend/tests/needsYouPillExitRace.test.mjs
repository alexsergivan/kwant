// The in-app pill's slide-out race (Lin found it in the Mac pill, #268; Mira's note for #267): an exit
// must only clear the pill it is exiting. Something that arrives during the 160 ms slide-out drops in as
// a fresh pill instead of being swallowed, and an item answered elsewhere during it doesn't bring it back.
import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
dom.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;

const ui = await loadBundled(`
  export { NeedsYouPill } from './src/components/NeedsYouPill';
  export { nudgeFromAway, pillAdd, pillPrune, EMPTY_PILL } from './src/attention';
  export { createElement } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
`);

const item = (id) => ({
  kind: 'question', id, kwant_id: 'k1', kwant_name: 'Juno', title: `Title ${id}`, summary: '', status: 'open',
  timestamp: '2026-11-03T21:00:00Z', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k1' },
});
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r)); };

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  let pill = ui.EMPTY_PILL;
  const draw = () =>
    ui.flushSync(() => {
      root.render(ui.createElement(ui.NeedsYouPill, {
        items: pill.items, rev: pill.rev, spriteOf: () => 'violet', onOpen: () => {},
        // The same guard App's dismissPill has: only clear the rev that left.
        onDismiss: (rev) => { if (pill.rev === rev) { pill = ui.EMPTY_PILL; draw(); } },
      }));
    });
  return {
    async arrive(id) { pill = ui.pillAdd(pill, [ui.nudgeFromAway(item(id))]); draw(); await settle(); },
    async prune(keep) { pill = ui.pillPrune(pill, new Set(keep.map((id) => `question:${id}`))); draw(); await settle(); },
    async act(fn) { ui.flushSync(fn); await settle(); },
    async tick(ms) { ui.flushSync(() => mock.timers.tick(ms)); await settle(); },
    button: () => host.querySelector('button.kw-nudge'),
    showing() { const b = host.querySelector('button.kw-nudge'); return !!b && !b.classList.contains('is-leaving'); },
    count: () => host.querySelector('button.kw-nudge')?.dataset.count,
    items: () => pill.items.length,
    unmount() { ui.flushSync(() => root.unmount()); host.remove(); },
  };
}

test('an arrival during the timed slide-out mounts a fresh pill with a full 5 s', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  const p = mount();
  try {
    await p.arrive('q0');
    const first = p.button();
    await p.tick(5000);
    assert.ok(!p.showing(), 'sliding out at 5 s');
    await p.tick(80);
    await p.arrive('q1');
    assert.ok(p.showing(), 'back up, not leaving');
    assert.notEqual(p.button(), first, 'a fresh pill, not the one that was leaving');
    await p.tick(1000);
    assert.ok(p.showing(), 'the old exit timer did not remove it');
    assert.equal(p.items(), 2, 'the app still has what arrived');
    await p.tick(3900);
    assert.ok(p.showing(), 'still up 4.9 s after it came back');
    await p.tick(100);
    assert.ok(!p.showing());
    await p.tick(200);
    assert.equal(p.button(), null);
    assert.equal(p.items(), 0);
  } finally { p.unmount(); mock.timers.reset(); }
});

test('an arrival during an Esc slide-out is not swallowed either', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  const p = mount();
  try {
    await p.arrive('q0');
    await p.act(() => p.button().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    assert.ok(!p.showing(), 'Esc starts the slide-out');
    await p.tick(100);
    await p.arrive('q1');
    await p.tick(500);
    assert.ok(p.showing());
    assert.equal(p.items(), 2);
  } finally { p.unmount(); mock.timers.reset(); }
});

test('an item answered elsewhere during the slide-out does not bring the pill back', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  const p = mount();
  try {
    await p.arrive('q0');
    await p.arrive('q1');
    await p.tick(5000);
    assert.ok(!p.showing());
    await p.tick(80);
    await p.prune(['q1']);
    assert.ok(!p.showing(), 'still leaving');
    await p.tick(200);
    assert.equal(p.button(), null, 'gone');
    assert.equal(p.items(), 0, 'and cleared');
  } finally { p.unmount(); mock.timers.reset(); }
});
