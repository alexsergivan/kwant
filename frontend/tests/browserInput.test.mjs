import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const b = await loadBundled(`export * from './src/browserInput';`);
const st = await loadBundled(`export { splitAddress, RELAYING_DELAY_MS } from './src/components/BrowserTakeoverStage';`);
const hs = await loadBundled(`export { openStepFor } from './src/humanSteps';`);
const vm = await loadBundled(`export { TAKEOVER_RETRY_MS, TAKEOVER_RETRY_FOR_MS, STEP_NEED, STEP_DONE } from './src/components/VirtualMachinePane';`);

const key = (over = {}) => ({ type: 'keydown', key: 'a', code: 'KeyA', altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, ...over });

test('resizeFor asks for the stage size, clamped to the server limits', () => {
  assert.deepEqual(b.resizeFor({ width: 1280.7, height: 760.2 }, 2), { action: 'resize', width: 1280, height: 760, dpr: 2 });
  assert.deepEqual(b.resizeFor({ width: 200, height: 3000 }, 3), { action: 'resize', width: 360, height: 1200, dpr: 2 });
  assert.equal(b.resizeFor({ width: 0, height: 500 }), null);
});

test('stageLayout draws 1:1 when it fits and scales down when it does not', () => {
  assert.deepEqual(b.stageLayout({ width: 1000, height: 800 }, { width: 800, height: 600 }), { scale: 1, left: 100, top: 100, width: 800, height: 600 });
  const l = b.stageLayout({ width: 400, height: 600 }, { width: 800, height: 600 });
  assert.equal(l.scale, 0.5);
  assert.equal(l.top, 150);
});

test('pagePoint maps client points to page pixels, null outside unless clamped', () => {
  const drawn = { left: 100, top: 50, width: 400, height: 300 };
  const page = { width: 800, height: 600 };
  assert.deepEqual(b.pagePoint(300, 200, drawn, page), { x: 400, y: 300 });
  assert.deepEqual(b.pagePoint(100, 50, drawn, page), { x: 0, y: 0 });
  assert.equal(b.pagePoint(99, 60, drawn, page), null);
  assert.equal(b.pagePoint(500, 60, drawn, page), null);
  assert.deepEqual(b.pagePoint(900, -10, drawn, page, true), { x: 799, y: 0 });
});

test('mouse buttons and modifiers', () => {
  assert.equal(b.buttonName(0), 'left');
  assert.equal(b.buttonName(1), 'middle');
  assert.equal(b.buttonName(2), 'right');
  assert.equal(b.mouseModifiers({ altKey: false, ctrlKey: true, metaKey: false, shiftKey: true }), b.MOD.ctrl | b.MOD.shift);
});

test('frame coalescer sends only the last move per frame and flushes before a press', () => {
  const queue = [];
  const sent = [];
  const c = b.createFrameCoalescer((fn) => queue.push(fn), (v) => sent.push(v));
  c.push(1); c.push(2); c.push(3);
  assert.equal(queue.length, 1);
  queue.shift()();
  assert.deepEqual(sent, [3]);
  c.push(4);
  c.flush();
  assert.deepEqual(sent, [3, 4]);
  queue.shift()(); // the scheduled run finds nothing left
  assert.deepEqual(sent, [3, 4]);
  c.push(5); c.drop(); queue.shift()();
  assert.deepEqual(sent, [3, 4]);
});

test('wheel: lines and pages become pixels; steps in a frame add up and fractions carry', () => {
  assert.deepEqual(b.wheelPixels({ deltaX: 0, deltaY: 3, deltaMode: 1 }, 700), { dx: 0, dy: 120 });
  assert.deepEqual(b.wheelPixels({ deltaX: 0, deltaY: 1, deltaMode: 2 }, 700), { dx: 0, dy: 700 });
  const queue = [];
  const sent = [];
  const w = b.createWheelCoalescer((fn) => queue.push(fn), (m) => sent.push(m));
  w.add({ x: 10, y: 20 }, { dx: 0, dy: 4.25 });
  w.add({ x: 11, y: 21 }, { dx: 0, dy: 4.5 }, b.MOD.shift);
  assert.equal(queue.length, 1);
  queue.shift()();
  assert.deepEqual(sent, [{ action: 'wheel', x: 11, y: 21, delta_x: 0, delta_y: 8, modifiers: b.MOD.shift }]);
  w.add({ x: 11, y: 21 }, { dx: 0, dy: 0.25 });
  queue.shift()();
  assert.equal(sent[1].delta_y, 1, 'the 0.75 left over plus 0.25 makes one pixel');
});

test('planKey: printable keys carry text, Enter carries \\r, composition is skipped', () => {
  const p = b.planKey(key(), false);
  assert.equal(p.kind, 'send');
  assert.equal(p.messages[0].text, 'a');
  assert.equal(b.planKey(key({ key: 'Enter', code: 'Enter' }), false).messages[0].text, '\r');
  assert.equal(b.planKey(key({ isComposing: true }), false).kind, 'skip');
  assert.equal(b.planKey(key({ key: 'Process', keyCode: 229 }), false).kind, 'skip');
  assert.equal(b.planKey(key({ type: 'keyup' }), false).messages[0].text, undefined);
});

test('planKey: Mac Cmd becomes Ctrl, Cmd+arrows Home/End, Option+arrows word moves', () => {
  const cmdA = b.planKey(key({ metaKey: true }), true).messages[0];
  assert.equal(cmdA.modifiers, b.MOD.ctrl);
  assert.equal(cmdA.text, undefined);
  const home = b.planKey(key({ key: 'ArrowLeft', code: 'ArrowLeft', metaKey: true }), true).messages[0];
  assert.deepEqual([home.key, home.modifiers], ['Home', 0]);
  const top = b.planKey(key({ key: 'ArrowUp', code: 'ArrowUp', metaKey: true }), true).messages[0];
  assert.deepEqual([top.key, top.modifiers], ['Home', b.MOD.ctrl]);
  const word = b.planKey(key({ key: 'ArrowLeft', code: 'ArrowLeft', altKey: true }), true).messages[0];
  assert.equal(word.modifiers, b.MOD.ctrl);
  const accent = b.planKey(key({ key: 'é', code: 'KeyE', altKey: true }), true).messages[0];
  assert.deepEqual([accent.text, accent.modifiers], ['é', 0]);
  const cmd = b.planKey(key({ key: 'Meta', code: 'MetaRight', metaKey: true }), true).messages[0];
  assert.deepEqual([cmd.key, cmd.code], ['Control', 'ControlRight']);
});

test('planKey: paste is left to the paste event, copy also asks for the selection', () => {
  assert.equal(b.planKey(key({ key: 'v', code: 'KeyV', ctrlKey: true }), false).kind, 'paste');
  assert.equal(b.planKey(key({ key: 'v', code: 'KeyV', metaKey: true }), true).kind, 'paste');
  assert.equal(b.planKey(key({ key: 'c', code: 'KeyC', ctrlKey: true }), false).copy, true);
  assert.equal(b.planKey(key({ key: 'x', code: 'KeyX', metaKey: true }), true).copy, true);
});

test('createKeyboard: Mac Cmd chords go up at once, held keys go up on blur, stray ups are dropped', () => {
  const sent = [];
  const kb = b.createKeyboard(true, (m) => sent.push(m));
  kb.handle(key({ key: 'c', code: 'KeyC', metaKey: true }));
  assert.deepEqual(sent.map((m) => m.action === 'key' ? `${m.type}:${m.code}` : m.action), ['down:KeyC', 'up:KeyC', 'copy']);
  sent.length = 0;
  kb.handle(key({ key: 'b', code: 'KeyB' }));
  assert.deepEqual(kb.held(), ['KeyB']);
  kb.releaseAll();
  assert.deepEqual(sent.map((m) => `${m.type}:${m.code}`), ['down:KeyB', 'up:KeyB']);
  sent.length = 0;
  kb.handle(key({ type: 'keyup', key: 'q', code: 'KeyQ' }));
  assert.equal(sent.length, 0);
  assert.equal(kb.handle(key({ key: 'v', code: 'KeyV', metaKey: true })), false, 'paste keeps its default');
});

test('typeMessages splits text into 16 KB chunks without breaking characters', () => {
  assert.deepEqual(b.typeMessages(''), []);
  const parts = b.typeMessages('ab😀cd', 5);
  assert.deepEqual(parts.map((p) => p.text), ['ab', '😀c', 'd']);
});

test('parseSelect and parseDialog read server messages defensively', () => {
  const s = b.parseSelect({ type: 'select', id: 's1', options: [{ value: 'a', label: 'A' }], selected: [0], anchor: { left: 1, top: 2, width: 3, height: 4 }, label: '  Country  ' });
  assert.ok(s);
  assert.equal(s.label, 'Country');
  assert.equal(b.parseSelect({ type: 'select' }), null);
  assert.equal(b.parseDialog({ type: 'dialog', kind: 'weird', message: 'Hi' }).kind, 'alert');
});

test('matchOption: type-ahead skips disabled options and cycles on a repeated letter', () => {
  const opts = [{ label: 'Austria' }, { label: 'Belgium', disabled: true }, { label: 'Brazil' }, { label: 'Bulgaria' }, { label: 'Canada' }].map((o, i) => ({ value: String(i), ...o }));
  assert.equal(b.matchOption(opts, 'b', 0), 2);
  assert.equal(b.matchOption(opts, 'b', 2), 3);
  assert.equal(b.matchOption(opts, 'bb', 3), 2, 'bb cycles back to Brazil');
  assert.equal(b.matchOption(opts, 'bu', 0), 3);
  assert.equal(b.matchOption(opts, 'z', 4), 4, 'no match keeps the active row');
  assert.equal(b.nextEnabled(opts, 0, 1), 2);
  assert.equal(b.nextEnabled(opts, 4, 1), 4);
});

test('popoverPlacement follows spec §4: 4 px below, 180–360 wide, 8 px inset, flips above', () => {
  const stage = { width: 1200, height: 800 };
  const p = b.popoverPlacement({ left: 100, top: 100, width: 120, height: 32 }, stage, 200);
  assert.deepEqual(p, { left: 100, top: 136, width: 180, maxHeight: 320 });
  assert.equal(b.popoverPlacement({ left: 100, top: 100, width: 600, height: 32 }, stage, 200).width, 360);
  assert.equal(b.popoverPlacement({ left: 1150, top: 100, width: 200, height: 32 }, stage, 200).left, 1200 - 8 - 200);
  const up = b.popoverPlacement({ left: 100, top: 700, width: 200, height: 32 }, stage, 200);
  assert.equal(up.top, 700 - 4 - 200);
  assert.ok(up.maxHeight <= 320);
});

test('splitAddress shows the host bright and the path muted', () => {
  assert.deepEqual(st.splitAddress('https://github.com/login?x=1'), { secure: true, host: 'github.com', rest: '/login?x=1' });
  assert.deepEqual(st.splitAddress('http://example.com/'), { secure: false, host: 'example.com', rest: '' });
  assert.deepEqual(st.splitAddress('about:blank'), { secure: false, host: 'about:blank', rest: '' });
  assert.equal(st.RELAYING_DELAY_MS, 300);
});

test('openStepFor picks the newest open step on this browser', () => {
  const s = (id, over) => ({ id, kind: 'sign_in', status: 'open', sandbox_target: 'vm', created_at: '2026-11-03T10:00:00Z', ...over });
  const map = { a: s('a'), b: s('b', { created_at: '2026-11-03T11:00:00Z', kind: 'two_factor' }), c: s('c', { sandbox_target: 'other', created_at: '2026-11-03T12:00:00Z' }), d: s('d', { status: 'done', created_at: '2026-11-03T13:00:00Z' }) };
  assert.equal(hs.openStepFor(map, 'vm').id, 'b');
  assert.equal(hs.openStepFor(map, undefined), null);
  assert.equal(hs.openStepFor(map, 'none'), null);
});

test('busy retry is every second for ten seconds; step copy per kind', () => {
  assert.equal(vm.TAKEOVER_RETRY_MS, 1000);
  assert.equal(vm.TAKEOVER_RETRY_FOR_MS, 10_000);
  assert.equal(vm.STEP_DONE.sign_in, 'Done, I’m signed in');
  assert.equal(vm.STEP_NEED.captcha, 'needs you to solve a puzzle');
});
