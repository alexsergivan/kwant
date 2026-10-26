import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const g = await loadBundled(`export * from './src/browserGestures';`);
const o = await loadBundled(`export { textEdits, sendPageInput } from './src/components/BrowserControlOverlay';`);

const phone = { width: 390, height: 700 };

test('fit: the page fills the width, top-aligned, no band above', () => {
  const v = g.fitView(phone);
  assert.equal(v.scale, 390 / 1280);
  assert.equal(v.tx, 0);
  assert.equal(v.ty, 0);
  // Landscape: height limits, centred across
  const l = g.fitView({ width: 844, height: 300 });
  assert.equal(l.scale, 300 / 800);
  assert.equal(l.tx, (844 - 1280 * l.scale) / 2);
});

test('clamp: never smaller than fit or past 4x, page edges stay on the stage', () => {
  assert.equal(g.clampView({ scale: 0.01, tx: 50, ty: 50 }, phone).scale, 390 / 1280);
  assert.equal(g.clampView({ scale: 10, tx: 0, ty: 0 }, phone).scale, g.MAX_SCALE);
  const v = g.clampView({ scale: 1, tx: 100, ty: -5000 }, phone);
  assert.equal(v.tx, 0);
  assert.equal(v.ty, 700 - 800);
});

test('zoom to a box: it fills the stage with a margin, centred', () => {
  const box = { x: 460, y: 180, width: 360, height: 380 };
  const v = g.zoomToBox(box, phone);
  assert.equal(v.scale, Math.min((390 - 32) / 360, (700 - 32) / 380));
  const centre = { x: (box.x + box.width / 2) * v.scale + v.tx, y: (box.y + box.height / 2) * v.scale + v.ty };
  assert.ok(Math.abs(centre.x - 195) < 1 && Math.abs(centre.y - 350) < 1, JSON.stringify(centre));
  // Box corners are on screen
  assert.ok(box.x * v.scale + v.tx >= 0 && (box.x + box.width) * v.scale + v.tx <= 390);
  // A tiny box doesn't zoom past the max
  assert.equal(g.zoomToBox({ x: 10, y: 10, width: 5, height: 5 }, phone).scale, g.MAX_SCALE);
});

test('stage ↔ page round-trips; off the page is null, clamped holds the edge', () => {
  const v = { scale: 2, tx: -100, ty: -40 };
  assert.deepEqual(g.stageToPage(2 * 510 - 100, 2 * 500 - 40, v), { x: 510, y: 500 });
  assert.equal(g.stageToPage(10, 10, g.fitView(phone)) !== null, true);
  assert.equal(g.stageToPage(10, 600, g.fitView(phone)), null);
  assert.deepEqual(g.stageToPageClamped(-50, 9999, g.fitView(phone)), { x: 0, y: 799 });
});

test('pinch keeps the page point under the fingers', () => {
  const start = { scale: 0.5, tx: 0, ty: 0 };
  const v = g.pinchView(start, { x: 100, y: 100 }, { x: 200, y: 100 }, { x: 50, y: 100 }, { x: 250, y: 100 });
  assert.equal(v.scale, 1);
  // Midpoint 150,100 was page (300,200); still there
  assert.equal(300 * v.scale + v.tx, 150);
  assert.equal(200 * v.scale + v.ty, 100);
});

function harness({ zoomable = true, zoomed = false } = {}) {
  const sent = [];
  let t = 0;
  const timers = [];
  let view = { scale: 0.5, tx: 0, ty: 0 };
  const host = {
    toPage: (p) => g.stageToPage(p.x, p.y, view),
    toPageClamped: (p) => g.stageToPageClamped(p.x, p.y, view),
    send: (i) => sent.push(i),
    now: () => t,
    later: (fn, ms) => {
      const timer = { at: t + ms, fn, live: true };
      timers.push(timer);
      return () => (timer.live = false);
    },
  };
  if (zoomable) host.view = { get: () => view, set: (v) => (view = v), zoomed: () => zoomed };
  const tick = (ms) => {
    t += ms;
    for (const tm of timers) if (tm.live && tm.at <= t) { tm.live = false; tm.fn(); }
  };
  return { tr: g.createGestureTracker(host), sent, tick, view: () => view };
}

test('a tap is one click at the page point', () => {
  const { tr, sent } = harness();
  tr.down(1, { x: 100, y: 50 });
  tr.move(1, { x: 102, y: 51 });
  tr.up(1, { x: 102, y: 51 });
  assert.deepEqual(sent, [{ action: 'click', x: 204, y: 102 }]);
});

test('a drag presses at the start, moves at most every 16ms, flushes and releases', () => {
  const { tr, sent, tick } = harness();
  tr.down(1, { x: 255, y: 250 });
  tick(5);
  tr.move(1, { x: 265, y: 250 });
  assert.deepEqual(sent[0], { action: 'pointer_down', x: 510, y: 500 });
  // Many moves inside one interval: one goes out now or after the wait, with the newest point
  for (let i = 1; i <= 5; i++) { tick(2); tr.move(1, { x: 265 + i * 10, y: 250 }); }
  const movesBefore = sent.filter((e) => e.action === 'pointer_move').length;
  tick(16);
  const moves = sent.filter((e) => e.action === 'pointer_move');
  assert.ok(moves.length <= movesBefore + 1 && moves.length <= 2, JSON.stringify(sent));
  assert.deepEqual(moves.at(-1), { action: 'pointer_move', x: 630, y: 500 });
  tr.move(1, { x: 400, y: 250 });
  tr.up(1, { x: 400, y: 250 });
  assert.deepEqual(sent.at(-2), { action: 'pointer_move', x: 800, y: 500 });
  assert.deepEqual(sent.at(-1), { action: 'pointer_up', x: 800, y: 500 });
  assert.ok(!sent.some((e) => e.action === 'click'));
  assert.equal(tr.dragging(), false);
});

test('a drag that leaves the page keeps going, held at the edge', () => {
  const { tr, sent } = harness();
  tr.down(1, { x: 600, y: 10 });
  tr.move(1, { x: 700, y: 10 });
  tr.up(1, { x: 900, y: 10 });
  assert.deepEqual(sent.at(-1), { action: 'pointer_up', x: 1279, y: 20 });
});

test('cancel releases a drag without a click; a cancelled press sends nothing', () => {
  let h = harness();
  h.tr.down(1, { x: 100, y: 100 });
  h.tr.move(1, { x: 130, y: 100 });
  h.tr.cancel(1);
  assert.deepEqual(h.sent.map((e) => e.action), ['pointer_down', 'pointer_move', 'pointer_up']);
  h = harness();
  h.tr.down(1, { x: 100, y: 100 });
  h.tr.cancel(1);
  assert.deepEqual(h.sent, []);
});

test('two fingers spreading zoom the view and send nothing', () => {
  const { tr, sent, view } = harness();
  tr.down(1, { x: 100, y: 100 });
  tr.down(2, { x: 200, y: 100 }, false);
  tr.move(1, { x: 50, y: 100 });
  tr.move(2, { x: 250, y: 100 });
  tr.up(1);
  tr.up(2);
  assert.deepEqual(sent, []);
  assert.equal(view().scale, 1);
});

test('two fingers moving up together scroll the page down (at fit)', () => {
  const { tr, sent } = harness();
  tr.down(1, { x: 100, y: 300 });
  tr.down(2, { x: 200, y: 300 }, false);
  tr.move(1, { x: 100, y: 260 });
  tr.move(2, { x: 200, y: 260 });
  tr.up(1);
  tr.up(2);
  const scrolls = sent.filter((e) => e.action === 'scroll');
  assert.ok(scrolls.length >= 1, JSON.stringify(sent));
  assert.ok(scrolls.every((s) => s.delta_y > 0));
  assert.ok(!sent.some((e) => e.action === 'click'));
});

test('zoomed in, two fingers moving together pan the view instead', () => {
  const { tr, sent, view } = harness({ zoomed: true });
  tr.down(1, { x: 100, y: 300 });
  tr.down(2, { x: 200, y: 300 }, false);
  tr.move(1, { x: 100, y: 260 });
  tr.move(2, { x: 200, y: 260 });
  assert.deepEqual(sent, []);
  assert.equal(view().ty, -40);
});

test('a finger added mid-drag is ignored; the drag ends cleanly', () => {
  const { tr, sent } = harness();
  tr.down(1, { x: 100, y: 100 });
  tr.move(1, { x: 140, y: 100 });
  tr.down(2, { x: 300, y: 300 }, false);
  tr.move(2, { x: 340, y: 340 });
  tr.up(2);
  tr.up(1, { x: 160, y: 100 });
  assert.deepEqual(sent.map((e) => e.action).filter((a) => a !== 'pointer_move'), ['pointer_down', 'pointer_up']);
});

test('a press that starts off the page does nothing', () => {
  const { tr, sent } = harness();
  tr.down(1, { x: 100, y: 600 });
  tr.move(1, { x: 150, y: 600 });
  tr.up(1);
  assert.deepEqual(sent, []);
});

test('desktop (no zoom): two fingers always scroll', () => {
  const { tr, sent } = harness({ zoomable: false });
  tr.down(1, { x: 100, y: 300 });
  tr.down(2, { x: 200, y: 300 }, false);
  tr.move(1, { x: 60, y: 200 });
  tr.move(2, { x: 240, y: 200 });
  assert.ok(sent.some((e) => e.action === 'scroll'));
});

test('phone keyboard text becomes type and Backspace events (autocorrect replaces words)', () => {
  assert.deepEqual(o.textEdits('', 'hi'), [{ action: 'type', text: 'hi' }]);
  assert.deepEqual(o.textEdits('hi', 'h'), [{ action: 'key', key: 'Backspace' }]);
  assert.deepEqual(o.textEdits('teh', 'the '), [{ action: 'key', key: 'Backspace' }, { action: 'key', key: 'Backspace' }, { action: 'type', text: 'he ' }]);
  assert.deepEqual(o.textEdits('same', 'same'), []);
});

test('only moves skip the ack; non-scroll inputs carry zero deltas', () => {
  const calls = [];
  const send = (p, input) => calls.push([p, input]);
  o.sendPageInput(send, { action: 'pointer_move', x: 1, y: 2 });
  o.sendPageInput(send, { action: 'pointer_down', x: 1, y: 2 });
  o.sendPageInput(send, { action: 'scroll', x: 1, y: 2, delta_x: 0, delta_y: 30 });
  assert.deepEqual(calls.map((c) => c[1]), [false, true, true]);
  assert.equal(calls[0][0].delta_x, 0);
  assert.equal(calls[2][0].delta_y, 30);
});
