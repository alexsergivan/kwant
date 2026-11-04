// The in-app "needs you" pill's timing (Mira's spec §2, Ada's call on the cap): 5 s from the last arrival,
// a hard 12 s cap on arrival restarts, and hover or focus holds it past the cap until 2 s after it leaves.
import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
// Width queries answer against window.innerWidth, the way a browser does; anything else (reduced motion) is off.
dom.window.matchMedia = (q) => {
  const max = /\(max-width:\s*(\d+)px\)/.exec(q);
  const min = /\(min-width:\s*(\d+)px\)/.exec(q);
  const matches = max ? dom.window.innerWidth <= Number(max[1]) : min ? dom.window.innerWidth >= Number(min[1]) : false;
  return { matches, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} };
};
globalThis.matchMedia = dom.window.matchMedia;

const ui = await loadBundled(`
  export { NeedsYouPill } from './src/components/NeedsYouPill';
  export { nudgeFromAway, pillAdd, EMPTY_PILL } from './src/attention';
  export { PHONE_QUERY } from './src/layout';
  export { createElement } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
`);

const item = (id) => ({
  kind: 'question', id, kwant_id: 'k1', kwant_name: 'Juno', title: `Title ${id}`, summary: '', status: 'open',
  timestamp: '2026-11-03T21:00:00Z', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k1' },
});

// React runs effects and continuous-priority updates (hover) on its scheduler, which uses setImmediate:
// the mocked clock leaves that alone, so each step lets it drain.
const settle = async () => { for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r)); };

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  let pill = ui.EMPTY_PILL;
  let dismissed = false;
  const draw = () =>
    ui.flushSync(() => {
      root.render(ui.createElement(ui.NeedsYouPill, {
        items: pill.items, rev: pill.rev, spriteOf: () => 'violet', onOpen: () => {},
        onDismiss: () => { pill = ui.EMPTY_PILL; dismissed = true; },
      }));
    });
  return {
    async arrive(id) { pill = ui.pillAdd(pill, [ui.nudgeFromAway(item(id))]); draw(); await settle(); },
    async act(fn) { ui.flushSync(fn); await settle(); },
    button: () => host.querySelector('button.kw-nudge'),
    showing() { const b = host.querySelector('button.kw-nudge'); return !!b && !b.classList.contains('is-leaving'); },
    count: () => host.querySelector('button.kw-nudge')?.dataset.count,
    async tick(ms) {
      ui.flushSync(() => mock.timers.tick(ms));
      await settle();
      // The app clears its pill state when the pill says it has gone; do the same here.
      if (dismissed) { dismissed = false; draw(); await settle(); }
    },
    unmount() { ui.flushSync(() => root.unmount()); host.remove(); },
  };
}

test('arrivals at 0, 4, 8 and 11 s swap in place but the pill is gone by about 12 s', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  const p = mount();
  try {
    await p.arrive('q0');
    const first = p.button();
    await p.tick(4000); await p.arrive('q4');
    await p.tick(4000); await p.arrive('q8');
    await p.tick(3000); await p.arrive('q11');
    assert.ok(p.showing(), 'still up at 11 s');
    assert.equal(p.button(), first, 'coalescing swaps the content in the same pill');
    assert.equal(p.count(), '4');
    await p.tick(900);
    assert.ok(p.showing(), 'still up at 11.9 s');
    await p.tick(100);
    assert.ok(!p.showing(), 'leaving at 12 s, not 16 s');
    await p.tick(200);
    assert.equal(p.button(), null, 'gone after the slide out');
    // An arrival after it has gone springs a fresh pill with a full 5 s.
    await p.arrive('q13');
    assert.ok(p.showing());
    assert.equal(p.count(), '1');
    await p.tick(4900);
    assert.ok(p.showing());
    await p.tick(100);
    assert.ok(!p.showing());
  } finally {
    p.unmount();
    mock.timers.reset();
  }
});

test('with focus inside it stays past the 12 s cap and goes 2 s after blur', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  const p = mount();
  try {
    await p.arrive('q0');
    await p.act(() => p.button().focus());
    await p.tick(4000); await p.arrive('q4');
    await p.tick(9000);
    assert.ok(p.showing(), 'still there at 13 s with focus inside');
    await p.act(() => p.button().blur());
    await p.tick(1900);
    assert.ok(p.showing(), 'still there 1.9 s after blur');
    await p.tick(100);
    assert.ok(!p.showing(), 'goes 2 s after blur');
  } finally {
    p.unmount();
    mock.timers.reset();
  }
});

test('hover holds it past the cap too, and it goes 2 s after the pointer leaves', async () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 0 });
  const p = mount();
  try {
    await p.arrive('q0');
    // React listens for mouseover/mouseout to synthesize enter and leave.
    await p.act(() => p.button().dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body })));
    await p.tick(15000);
    assert.ok(p.showing(), 'still there at 15 s while hovered');
    await p.act(() => p.button().dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body })));
    await p.tick(1900);
    assert.ok(p.showing());
    await p.tick(100);
    assert.ok(!p.showing());
  } finally {
    p.unmount();
    mock.timers.reset();
  }
});

test('desktop: the pill is centred on the main pane, starting where the sidebar ends; phone: the whole width', async () => {
  const side = document.createElement('aside');
  side.setAttribute('data-kw-sidebar', '');
  side.getBoundingClientRect = () => ({ left: 56, right: 312, width: 256, top: 0, bottom: 900, height: 900 });
  document.body.appendChild(side);
  const region = () => document.querySelector('.kw-nudge-region');
  try {
    for (const [w, want] of [[1440, '312px'], [390, '']]) {
      window.innerWidth = w;
      const p = mount();
      try {
        await p.arrive('q' + w);
        assert.equal(region().style.getPropertyValue('--kw-nudge-left'), want);
      } finally {
        p.unmount();
      }
    }
  } finally {
    side.remove();
  }
});

// Rigel's R-1: between 640 and 767 px the app is in its phone layout (no sidebar shown), but the pill still
// measured the sidebar and slid off-screen. The pill and its CSS now switch at the app's own breakpoint.
test('R-1: at 700 px (phone layout) the pill is full width, not placed from the sidebar; at 768 px it is centred on the main pane', async () => {
  const side = document.createElement('aside');
  side.setAttribute('data-kw-sidebar', '');
  // In the phone layout the sidebar can still be in the DOM with a size (an off-canvas drawer): it must not count.
  side.getBoundingClientRect = () => ({ left: 56, right: 312, width: 256, top: 0, bottom: 900, height: 900 });
  document.body.appendChild(side);
  const region = () => document.querySelector('.kw-nudge-region');
  try {
    for (const [w, want] of [[700, ''], [767, ''], [768, '312px']]) {
      window.innerWidth = w;
      const p = mount();
      try {
        await p.arrive('r1-' + w);
        assert.equal(region().style.getPropertyValue('--kw-nudge-left'), want, `at ${w} px`);
      } finally {
        p.unmount();
      }
    }
  } finally {
    side.remove();
  }
});

test('R-1: the pill CSS switches to its full-width phone style at the same breakpoint as the app layout', () => {
  const css = readFileSync(new URL('../src/styles/needsYou.css', import.meta.url), 'utf8');
  const phone = /@media\s*(\(max-width:\s*\d+px\))\s*\{\s*\.kw-nudge-region\s*\{[^}]*left:\s*0/.exec(css);
  assert.ok(phone, 'the phone block resets the region to the full width');
  assert.equal(phone[1], ui.PHONE_QUERY);
});

// Rigel's R-2: widening from the phone layout while the pill shows fires `resize` before React swaps the layout,
// so the full-width chat list still sits where the sidebar goes. The pill must measure again once the layout
// has settled, and never be placed at the window's right edge.
test('R-2: widening 700 -> 900 and 767 -> 768 while the pill shows keeps it on the main pane, not off-screen', async () => {
  const side = document.createElement('aside');
  side.setAttribute('data-kw-sidebar', '');
  document.body.appendChild(side);
  const region = () => document.querySelector('.kw-nudge-region');
  // Animation frames run when the test says so, as a browser paints after React commits.
  let frames = [];
  const origRaf = window.requestAnimationFrame;
  const origCaf = window.cancelAnimationFrame;
  window.requestAnimationFrame = (f) => { frames.push(f); return frames.length; };
  window.cancelAnimationFrame = () => {};
  const paint = async () => {
    for (let i = 0; i < 4 && frames.length; i++) { const run = frames; frames = []; ui.flushSync(() => run.forEach((f) => f(0))); }
    await settle();
  };
  try {
    for (const [from, to] of [[700, 900], [767, 768]]) {
      window.innerWidth = from;
      // Phone layout: the chat list fills the window where the sidebar goes.
      side.getBoundingClientRect = () => ({ left: 0, right: from, width: from, top: 0, bottom: 900, height: 900 });
      const p = mount();
      try {
        await p.arrive(`r2-${from}`);
        assert.equal(region().style.getPropertyValue('--kw-nudge-left'), '', `at ${from} px`);
        // Widen: resize fires while the phone layout is still on screen.
        window.innerWidth = to;
        side.getBoundingClientRect = () => ({ left: 0, right: to, width: to, top: 0, bottom: 900, height: 900 });
        await p.act(() => window.dispatchEvent(new window.Event('resize')));
        const early = region().style.getPropertyValue('--kw-nudge-left');
        assert.ok(early === '' || parseInt(early, 10) < to, `${from} -> ${to}: not at the right edge right after resize (got ${early})`);
        // React swaps in the desktop layout: a 256 px sidebar after the 56 px rail.
        side.getBoundingClientRect = () => ({ left: 56, right: 312, width: 256, top: 0, bottom: 900, height: 900 });
        await paint();
        assert.equal(region().style.getPropertyValue('--kw-nudge-left'), '312px', `${from} -> ${to}: centred on the main pane`);
        assert.ok(p.showing(), 'still showing');
      } finally {
        p.unmount();
      }
    }
  } finally {
    window.requestAnimationFrame = origRaf;
    window.cancelAnimationFrame = origCaf;
    side.remove();
  }
});
