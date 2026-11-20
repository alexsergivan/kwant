// Focus after render (focus-after-render-everywhere, after #897): the shared hook focuses after the commit that
// shows its target, and a guard keeps new `setTimeout(() => ….focus())` calls out of frontend/src. A focus on a
// timer can run before React renders its target (the scheduler yields on a slow machine), and focus drops to
// <body> for good. Use useFocusAfterRender / useAfterRender from src/focusAfterRender.ts instead.
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
  export { createElement, useRef, useState } from 'react';
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
  // ProdFloorCard moves to the shared hook after #897 (in 0.9.32) merges; this item leaves the file alone.
  { file: 'components/dev/ProdFloorCard.tsx', has: 'cardRef.current?.contains(a)', why: 'takeBackup: the main button is always rendered (#897 keeps it)' },
  { file: 'components/dev/ProdFloorCard.tsx', has: 'cmdRef.current?.focus()', why: 'moves to the shared hook after #897', count: 2 },
  { file: 'components/dev/ProdFloorCard.tsx', has: 'mainRef.current?.focus()', why: 'moves to the shared hook after #897', count: 2 },
  { file: 'components/dev/ProdFloorCard.tsx', has: 'skipRef.current?.focus()', why: 'moves to the shared hook after #897' },
  { file: 'components/dev/ProdFloorCard.tsx', has: '(skipOpenerRef.current ?? mainRef.current)?.focus()', why: 'moves to the shared hook after #897' },
];

const sources = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? sources(join(dir, e.name)) : /\.(tsx?|jsx?|mjs)$/.test(e.name) ? [join(dir, e.name)] : []));

/** Comments out, line breaks kept (so line numbers still match). Good enough for this code base's comments. */
const uncomment = (s) => s.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' ')).replace(/^(\s*)\/\/.*$/gm, '$1');

/** Every `setTimeout(` / `window.setTimeout(` whose callback (its first argument) calls `.focus(`. */
export function timerFocusCalls(file, text) {
  const s = uncomment(text);
  const out = [];
  const re = /\b(?:window\.)?setTimeout\s*\(/g;
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
    if (/\.focus\s*\(/.test(cb)) out.push({ file, line: s.slice(0, mm.index).split('\n').length, cb: cb.replace(/\s+/g, ' ').trim() });
  }
  return out;
}

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
  const calls = sources(SRC).flatMap((p) => timerFocusCalls(relative(SRC, p).split('\\').join('/'), readFileSync(p, 'utf8')));
  const used = new Map();
  const unexpected = [];
  for (const c of calls) {
    const k = KEPT.find((e) => e.file === c.file && c.cb.includes(e.has));
    if (!k) unexpected.push(`${c.file}:${c.line} ${c.cb.slice(0, 90)}`);
    else used.set(k, (used.get(k) ?? 0) + 1);
  }
  assert.deepEqual(unexpected, [], 'focus on a timer can run before React renders the target; use useFocusAfterRender() (or add it to KEPT with why)');
  const stale = KEPT.filter((k) => (used.get(k) ?? 0) !== (k.count ?? 1)).map((k) => `${k.file}: ${k.has} (found ${used.get(k) ?? 0}, expected ${k.count ?? 1})`);
  assert.deepEqual(stale, [], 'KEPT lists a call that changed or is gone: update the list');
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
