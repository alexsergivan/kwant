// Focus after render (focus-after-render-everywhere, after #897): the shared hook focuses after the commit that
// shows its target, and a guard keeps new `setTimeout(() => ….focus())` calls out of frontend/src. A focus on a
// timer can run before React renders its target (the scheduler yields on a slow machine), and focus drops to
// <body> for good. Use useFocusAfterRender / useAfterRender from src/focusAfterRender.ts instead.
// raf-focus-after-render: the same guard covers `requestAnimationFrame(() => ….focus())` (KEPT_FRAME).
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { useFocusAfterRender, useAfterRender } from './src/focusAfterRender';
  export { createRoot } from 'react-dom/client';
  export { createElement, useRef, useState, useSyncExternalStore } from 'react';
`);
const h = m.createElement;

// ---------------------------------------------------------------------------------------------------
// The guard

const SRC = fileURLToPath(new URL('../src/', import.meta.url));

/**
 * The timer focus calls that stay, each with why. A new one fails the guard; one that is gone fails it too, so
 * this list never goes stale. `has` is a piece of the timer's callback, unique within its file.
 */
const KEPT = [
  { file: 'components/GitHubConnect.tsx', has: 'a.focus({ preventScroll: true })', why: 'the anchor is already on screen and this layer unmounts in the same update' },
  { file: 'components/JiraSetup.tsx', has: 'a.focus({ preventScroll: true })', why: 'the anchor is already on screen and this layer unmounts in the same update' },
  { file: 'components/JiraSetup.tsx', has: 'addrRef.current?.focus', why: 'runs after mount (field already rendered); lets the sheet’s own open focus go first' },
  { file: 'components/GetStarted.tsx', has: "'[data-composer-form] textarea'", why: 'the composer is already on screen, outside this card, and Keep can remove the card' },
  { file: 'components/DictationMic.tsx', has: "popRef.current?.querySelector<HTMLButtonElement>('button')", why: 'an effect after the commit that renders the popover' },
  { file: 'components/LooseEndsView.tsx', has: "box.current?.querySelector<HTMLElement>('input:not([disabled])", why: 'an effect after the commit that renders the form' },
  { file: 'components/dev/LineNotes.tsx', has: "'[data-ln-text]'", why: 'the text box is already on screen and the confirm that calls it unmounts' },
  { file: 'components/dev/ProdFloorCard.tsx', has: 'cardRef.current?.contains(a)', why: 'takeBackup: the main button is always rendered (#897 keeps it)' },
];

/**
 * The frame focus calls that stay (raf-focus-after-render), each with why; same rules as KEPT. A frame is safe when
 * only a click or key starts the update (React renders it before the next frame), when it runs in an effect after
 * the commit that renders its target, or when the target is already on screen.
 */
const KEPT_FRAME = [
  { file: 'components/ChatView.tsx', has: 'focusComposer()', why: 'an effect after the commit; the composer is rendered; cancelled on cleanup' },
  { file: 'components/ChatView.tsx', has: 'el.setSelectionRange(6, 6)', why: 'completeIdea: Tab, Enter or a click' },
  { file: 'components/ClaudeSignIn.tsx', has: 'triggerRef.current?.focus()', why: 'Sign out trigger is always on screen; Cancel, Escape or the backdrop' },
  { file: 'components/CodingAgents.tsx', has: 'addRef.current?.focus()', why: 'Done and Cancel: a click or key: React renders that update before the next frame', count: 2 },
  { file: 'components/DevStatusLine.tsx', has: '[data-dv-row-action], [data-dv-status-action]', why: 'focusAfter: the link form’s Cancel or Escape' },
  { file: 'components/EngineAccounts.tsx', has: 'addRef.current?.focus()', why: 'the add form’s Cancel or Escape' },
  { file: 'components/GoalsStrip.tsx', has: '[aria-invalid="true"]', why: 'a submit event: React renders the errors before the next frame' },
  { file: 'components/InboxPage.tsx', has: 'inbox-tab-', why: 'an arrow key; the tabs are always on screen' },
  { file: 'components/MissionWatch.tsx', has: 'chipRef.current?.focus()', why: 'onHide: a click; the chip is on screen' },
  { file: 'components/NewChannelModal.tsx', has: 'nameRef.current?.focus()', why: 'an effect after the commit that renders the field' },
  { file: 'components/ReviewItemView.tsx', has: 'sendBtnRef.current?.focus()', why: 'closeSend: Cancel, Escape or the sheet close; the button is on screen' },
  { file: 'components/ReviewItemView.tsx', has: 'rollBtnRef.current?.focus()', why: 'the roll-back dialog’s Keep my work or Escape; the button is on screen' },
  { file: 'components/RoutinesPanel.tsx', has: 'tzRef.current?.focus()', why: 'openTz: Change, a click' },
  { file: 'components/SettingsModal.tsx', has: 'data-switch=', why: 'an effect after the commit that renders the switch; cancelled on cleanup' },
  { file: 'components/ShowOnce.tsx', has: '.so-record-steps', why: 'focusAfterDiscard is a plain function, not a component: it can’t use the hook' },
  { file: 'components/SkillsPanel.tsx', has: 'showOnceBtn.current?.focus()', why: 'Show it once is already on screen; only the row that had focus goes' },
  { file: 'components/TeamNeedBox.tsx', has: 'ta.current?.focus({ preventScroll: true })', why: 'a mount effect: the box is rendered' },
  { file: 'components/TeamNeedBox.tsx', has: 'focusBox', why: 'the sheet’s Cancel; the box is on screen, or remounts and autofocuses' },
  { file: 'components/TeamPreviewSheet.tsx', has: 'firstRenameRef.current', why: 'an effect after the commit that renders Rename or Create' },
];

const sources = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? sources(join(dir, e.name)) : /\.(tsx?|jsx?|mjs)$/.test(e.name) ? [join(dir, e.name)] : []));

/** Comments out, line breaks kept (so line numbers still match). Good enough for this code base's comments. */
const uncomment = (s) => s.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' ')).replace(/^(\s*)\/\/.*$/gm, '$1');

/** Every call matching `re` whose callback (its first argument) passes `hit`. */
function callbackFocusCalls(file, text, re, hit) {
  const s = uncomment(text);
  const out = [];
  for (let mm; (mm = re.exec(s)); ) {
    let i = mm.index + mm[0].length;
    const start = i;
    for (let depth = 0; i < s.length; i++) {
      const c = s[i];
      if ('([{'.includes(c)) depth++;
      else if (')]}'.includes(c)) {
        if (depth === 0) break;
        depth--;
      } else if (c === ',' && depth === 0) break;
    }
    const cb = s.slice(start, i);
    if (hit.test(cb)) out.push({ file, line: s.slice(0, mm.index).split('\n').length, cb: cb.replace(/\s+/g, ' ').trim() });
  }
  return out;
}

/** Every `setTimeout(` / `window.setTimeout(` whose callback (its first argument) calls `.focus(`. */
export const timerFocusCalls = (file, text) => callbackFocusCalls(file, text, /\b(?:window\.)?setTimeout\s*\(/g, /\.focus\s*\(/);

/**
 * Every `requestAnimationFrame(` / `window.requestAnimationFrame(` whose callback calls `.focus(`, or is (or calls)
 * a `focusX` helper such as `focusBox`. A frame can come before React renders an update made after an `await`, a
 * `.then` or in an effect (raf-focus-after-render), the same race as a timer.
 */
export const frameFocusCalls = (file, text) =>
  callbackFocusCalls(file, text, /\b(?:window\.)?requestAnimationFrame\s*\(/g, /\.focus\s*\(|\bfocus[A-Z]\w*\b/);

/** Every call `find` finds in frontend/src must match a `kept` entry, and every entry must match its count. */
const checkKept = (find, kept) => {
  const calls = sources(SRC).flatMap((p) => find(relative(SRC, p).split('\\').join('/'), readFileSync(p, 'utf8')));
  const used = new Map();
  const unexpected = [];
  for (const c of calls) {
    const k = kept.find((e) => e.file === c.file && c.cb.includes(e.has));
    if (!k) unexpected.push(`${c.file}:${c.line} ${c.cb.slice(0, 90)}`);
    else used.set(k, (used.get(k) ?? 0) + 1);
  }
  const stale = kept.filter((k) => (used.get(k) ?? 0) !== (k.count ?? 1)).map((k) => `${k.file}: ${k.has} (found ${used.get(k) ?? 0}, expected ${k.count ?? 1})`);
  return { unexpected, stale };
};

test('guard: the scanner finds a timer focus on one line or many, and skips comments and other timers', () => {
  const found = timerFocusCalls(
    'x.tsx',
    [
      'window.setTimeout(() => a.current?.focus(), 0);',
      'setTimeout(() => {',
      '  const b = ref.current;',
      '  b?.focus({ preventScroll: true });',
      '}, 50);',
      'setTimeout(() => setOpen(false), 0); el.focus();',
      '// setTimeout(() => c.focus(), 0);',
      '/* window.setTimeout(() => d.focus()) */',
      'const t = setTimeout(done, 10);',
    ].join('\n'),
  );
  assert.deepEqual(
    found.map((f) => f.line),
    [1, 2],
  );
});

test('guard: no new setTimeout(… .focus()) in frontend/src; focus after render instead (src/focusAfterRender.ts)', () => {
  const { unexpected, stale } = checkKept(timerFocusCalls, KEPT);
  assert.deepEqual(unexpected, [], 'focus on a timer can run before React renders the target; use useFocusAfterRender() (or add it to KEPT with why)');
  assert.deepEqual(stale, [], 'KEPT lists a call that changed or is gone: update the list');
});

test('guard: the frame scanner finds a frame focus on one line or many, a focusX helper, and skips comments and other frames', () => {
  const found = frameFocusCalls(
    'x.tsx',
    [
      'requestAnimationFrame(() => a.current?.focus());',
      'const raf = window.requestAnimationFrame(() => {',
      '  b?.focus({ preventScroll: true });',
      '});',
      'requestAnimationFrame(focusBox);',
      'requestAnimationFrame(() => setFlash(id)); el.focus();',
      '// requestAnimationFrame(() => c.focus());',
      'requestAnimationFrame(tick);',
    ].join('\n'),
  );
  assert.deepEqual(
    found.map((f) => f.line),
    [1, 2, 5],
  );
});

test('guard: no new requestAnimationFrame(… .focus()) in frontend/src; focus after render instead (src/focusAfterRender.ts)', () => {
  const { unexpected, stale } = checkKept(frameFocusCalls, KEPT_FRAME);
  assert.deepEqual(unexpected, [], 'focus on a frame can run before React renders an update made after an await or in an effect; use useFocusAfterRender() (or add it to KEPT_FRAME with why)');
  assert.deepEqual(stale, [], 'KEPT_FRAME lists a call that changed or is gone: update the list');
});

// ---------------------------------------------------------------------------------------------------
// The hook

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};

/** A button that swaps in a field after an await (as after a save), and focuses it with the hook. */
const Swap = ({ api, onAfter }) => {
  const [open, setOpen] = m.useState(false);
  const [n, setN] = m.useState(0);
  const field = m.useRef(null);
  const focusAfterRender = m.useFocusAfterRender();
  const after = m.useAfterRender();
  const go = async () => {
    await api.save();
    setOpen(true);
    focusAfterRender(() => field.current);
    after(() => onAfter?.(field.current?.value));
  };
  const missing = () => focusAfterRender(() => null);
  return h(
    'div',
    null,
    open ? h('input', { ref: field, defaultValue: 'ready', 'data-field': '' }) : h('button', { type: 'button', onClick: () => void go(), 'data-go': '' }, 'Go'),
    h('button', { type: 'button', onClick: missing, 'data-missing': '' }, 'Missing'),
    h('button', { type: 'button', onClick: () => setN(n + 1), 'data-bump': '' }, `Bump ${n}`),
  );
};
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

test('hook: focus lands on the field that the same update swaps in, and later work sees it rendered', async () => {
  const seen = [];
  const host = await mount(h(Swap, { api: { save: async () => undefined }, onAfter: (v) => seen.push(v) }));
  await click(host.querySelector('[data-go]'));
  assert.ok(document.activeElement === host.querySelector('[data-field]'), 'focus is in the new field');
  assert.deepEqual(seen, ['ready'], 'the work ran once, after the field was rendered');
  await click(host.querySelector('[data-bump]'));
  assert.deepEqual(seen, ['ready'], 'and never again on a later render');
});

test('hook: a target that is missing after the render is skipped and cleared, never focused on a later render', async () => {
  const host = await mount(h(Swap, { api: { save: async () => undefined } }));
  const bump = host.querySelector('[data-bump]');
  bump.focus();
  await click(host.querySelector('[data-missing]'));
  bump.focus();
  await click(bump);
  assert.ok(document.activeElement === bump, 'focus stays where the person put it');
});

/** After an await, a store update (rendered first, in a sync commit) and a state update that shows the field. */
const store = { v: 0, subs: new Set(), set(v) { this.v = v; for (const f of this.subs) f(); } };
const StoreSwap = () => {
  const v = m.useSyncExternalStore((f) => (store.subs.add(f), () => store.subs.delete(f)), () => store.v);
  const [open, setOpen] = m.useState(false);
  const field = m.useRef(null);
  const focusAfterRender = m.useFocusAfterRender();
  const go = () =>
    Promise.resolve().then(() => {
      store.set(v + 1);
      setOpen(true);
      focusAfterRender(() => field.current);
    });
  return h('div', null, open ? h('input', { ref: field, 'data-field': '' }) : h('button', { type: 'button', onClick: go, 'data-go': '' }, 'Go'), h('span', null, `Saved ${v}`));
};

test('hook: work waits for the render it asked for, not an earlier store commit that does not show the target yet', async () => {
  const host = await mount(h(StoreSwap));
  await click(host.querySelector('[data-go]'));
  assert.equal(host.textContent.includes('Saved 1'), true, 'the store update rendered');
  assert.ok(document.activeElement === host.querySelector('[data-field]'), `focus is in the new field, not ${document.activeElement?.tagName.toLowerCase()}`);
});
