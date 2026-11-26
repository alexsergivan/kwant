// space-icons-2 (Mira, autonomy/space-icons-2/design.md): 24 icons in one radio group, and the holo foil ring
// only on the picked button and the form's preview tile. One press + foil sweep on a change, none under
// reduced motion. The rail and switcher stay plain.
import assert from 'node:assert/strict';
import test, { after, afterEach } from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'AnimationEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
let reduced = false;
g.window.matchMedia = (q) => ({ matches: reduced && q.includes('prefers-reduced-motion'), media: q, addEventListener() {}, removeEventListener() {} });
g.fetch = g.window.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { NewSpaceModal, SpaceIconPicker, SpaceRail, SpaceSwitcher } from './src/components/Spaces';
  export { SPACE_ICONS } from './src/spaces';
`);
const { React } = m;
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const roots = new Set();
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(el));
  roots.add(root);
  await settle();
  return host;
};
afterEach(() => {
  for (const r of roots) r.unmount();
  roots.clear();
  document.body.replaceChildren();
  reduced = false;
});
after(() => dom.window.close());

const space = (id, icon, color) => ({ id, name: id, icon, color, sort: 0, is_default: false, archived_at: null, created_at: '', counts: { bots: 0, ready_accounts: 0, unread: 0 } });
const newSpace = (spaces = []) => mount(React.createElement(NewSpaceModal, { spaces, onClose() {}, onCreated() {} }));
const { NewSpaceModal } = m;
const radios = (host) => [...host.querySelectorAll('[role="radio"]')];
const checked = (host) => host.querySelector('[role="radio"][aria-checked="true"]');
const key = async (el, k) => {
  el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};
const tileOf = (host) => host.querySelector('[data-space-tile-foil]');

test('one radio group named by its legend; 24 radios named "Icon: <name>"; legend names the pick', async () => {
  const host = await newSpace();
  const group = host.querySelector('[role="radiogroup"]');
  assert.ok(group);
  const legend = document.getElementById(group.getAttribute('aria-labelledby'));
  assert.equal(legend.tagName, 'LEGEND');
  assert.equal(legend.textContent, 'Icon · Work');
  assert.equal(radios(host).length, 24);
  assert.equal(radios(host)[21].getAttribute('aria-label'), 'Icon: coffee');
  assert.equal(radios(host)[21].getAttribute('title'), 'Coffee');
});

test('roving tabindex: Tab stops on the grid once (the picked button)', async () => {
  const host = await newSpace();
  const tabbable = radios(host).filter((b) => b.tabIndex === 0);
  assert.equal(tabbable.length, 1);
  assert.equal(tabbable[0].getAttribute('aria-label'), 'Icon: work');
});

test('a new space takes the first unused icon of the 24', async () => {
  const host = await newSpace([space('a', '💼', '#f59e0b'), space('b', '💻', '#10b981')]);
  assert.equal(checked(host).getAttribute('aria-label'), 'Icon: chart');
});

test('arrows, Home and End move the pick; aria-checked and focus follow; up/down use the live column count', async () => {
  const host = await newSpace();
  const grid = host.querySelector('[role="radiogroup"]');
  const at = () => checked(host).getAttribute('aria-label');
  await key(checked(host), 'ArrowRight');
  assert.equal(at(), 'Icon: code');
  assert.equal(document.activeElement, checked(host));
  assert.equal(radios(host).filter((b) => b.tabIndex === 0).length, 1);
  await key(checked(host), 'ArrowDown'); // 8 columns: code → lab
  assert.equal(at(), 'Icon: lab');
  await key(checked(host), 'End');
  assert.equal(at(), 'Icon: star');
  await key(checked(host), 'ArrowRight'); // wraps
  assert.equal(at(), 'Icon: work');
  await key(checked(host), 'ArrowLeft');
  assert.equal(at(), 'Icon: star');
  await key(checked(host), 'Home');
  assert.equal(at(), 'Icon: work');
  // A phone grid has 6 columns: down from work (0) is index 6, chat.
  grid.style.gridTemplateColumns = '1fr 1fr 1fr 1fr 1fr 1fr';
  await key(checked(host), 'ArrowDown');
  assert.equal(at(), 'Icon: chat');
  await key(checked(host), 'ArrowUp');
  assert.equal(at(), 'Icon: work');
  assert.equal(legendText(host), 'Icon · Work');
});
test('Ctrl, Meta and Alt with an arrow pass through to the browser; a plain arrow still moves the pick', async () => {
  const host = await newSpace();
  const at = () => checked(host).getAttribute('aria-label');
  const start = at();
  for (const mod of ['metaKey', 'ctrlKey', 'altKey']) {
    const ev = new window.KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true, [mod]: true });
    checked(host).dispatchEvent(ev);
    await settle();
    assert.equal(at(), start, `${mod}+ArrowRight must not change the icon`);
    assert.equal(ev.defaultPrevented, false, `${mod}+ArrowRight must not be prevented`);
  }
  const plain = new window.KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true });
  checked(host).dispatchEvent(plain);
  await settle();
  assert.notEqual(at(), start);
  assert.equal(plain.defaultPrevented, true);
});

const legendText = (host) => host.querySelector('legend').textContent;

test('foil ring: only the picked button and the preview tile wear it', async () => {
  const host = await newSpace();
  const rings = [...host.querySelectorAll('.kw-foil-ring')];
  assert.equal(rings.length, 2);
  assert.equal(rings[0].closest('[role="radio"]'), null); // the tile's, first in the form
  assert.ok(tileOf(host).contains(rings[0]));
  assert.equal(rings[1].closest('[role="radio"]'), checked(host));
  for (const r of rings) assert.ok(r.classList.contains('kw-sticker') && r.classList.contains('kw-sticker-holo'));
  assert.equal(rings[1].getAttribute('aria-hidden'), 'true');
});

test('the rail and the switcher stay plain (no foil)', async () => {
  const spaces = [space('a', '💼', '#f59e0b'), space('b', '☕', '#10b981'), space('c', '🦄', '#8b5cf6')];
  const noop = () => {};
  const rail = await mount(React.createElement(m.SpaceRail, { spaces, currentId: 'a', onSwitch: noop, onCreate: noop, onOpenAppSettings: noop }));
  assert.equal(rail.querySelectorAll('[data-space-glyph]').length, 2);
  assert.equal(rail.querySelectorAll('.kw-foil-ring, .kw-just, [data-space-tile-foil]').length, 0);
  const sw = await mount(React.createElement(m.SpaceSwitcher, { spaces, currentId: 'b', onSwitch: noop, onCreate: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onClose: noop }));
  assert.equal(sw.querySelectorAll('.kw-foil-ring, .kw-just, [data-space-tile-foil]').length, 0);
  // The header and Space ready card use SpaceTile without foil.
  for (const f of ['../src/components/Sidebar.tsx', '../src/App.tsx']) {
    const src = readFileSync(new URL(f, import.meta.url), 'utf8');
    assert.ok(!/<SpaceTile[^>]*\bfoil\b/.test(src), f);
  }
});

test('a change plays one press + sweep on the picked button and the tile; repeat does not restart; animationend clears', async () => {
  const host = await newSpace();
  assert.equal(host.querySelectorAll('.kw-just').length, 0); // nothing on first mount
  radios(host)[21].click();
  await settle();
  assert.equal(checked(host).getAttribute('aria-label'), 'Icon: coffee');
  assert.ok(checked(host).classList.contains('kw-just'));
  assert.ok(tileOf(host).classList.contains('kw-just'));
  // A second change while it plays: the pick moves, the press stays where it started.
  await key(checked(host), 'ArrowRight');
  assert.equal(checked(host).getAttribute('aria-label'), 'Icon: pets');
  assert.ok(!checked(host).classList.contains('kw-just'));
  const ev = new window.Event('animationend', { bubbles: true });
  Object.defineProperty(ev, 'animationName', { value: 'kw-foil-sweep' });
  tileOf(host).dispatchEvent(ev);
  await settle();
  assert.equal(host.querySelectorAll('.kw-just').length, 0);
  // A colour change plays it again.
  host.querySelector('[aria-label="Colour #8b5cf6"]').click();
  await settle();
  assert.ok(checked(host).classList.contains('kw-just'));
});

test('reduced motion: a change never adds kw-just, and the CSS turns its animations off', async () => {
  reduced = true;
  const host = await newSpace();
  radios(host)[5].click();
  await settle();
  assert.equal(checked(host).getAttribute('aria-label'), 'Icon: mail');
  assert.equal(host.querySelectorAll('.kw-just').length, 0);
  assert.equal(host.querySelectorAll('.kw-foil-ring').length, 2); // the ring itself stays, static
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s*\.kw-just,\s*\.kw-just > \.kw-foil-ring::before \{ animation: none; \}/);
  assert.match(css, /\.kw-just \{ --kw-tilt: 0deg; animation: kw-sticker-press 160ms var\(--kw-ease\) 1 both; \}/);
  assert.match(css, /\.kw-just > \.kw-foil-ring::before \{ animation: kw-foil-sweep 200ms var\(--kw-ease\) 1 both; \}/);
});

test('a custom icon: nothing checked, the first button is the Tab stop, the tile shows the emoji with no foil', async () => {
  const src = readFileSync(new URL('../src/components/SpaceSettings.tsx', import.meta.url), 'utf8');
  assert.match(src, /foil=\{picked\}/);
  // The picker itself, given a value outside the 24:
  const { SpaceIconPicker } = m;
  const host = await mount(React.createElement(SpaceIconPicker, { id: 't', value: '🦄', colour: '#10b981', onChange() {}, just: null, onJustEnd() {}, gridClass: '', buttonClass: '', glyphClass: '' }));
  assert.equal(host.querySelectorAll('[aria-checked="true"]').length, 0);
  assert.equal(radios(host).filter((b) => b.tabIndex === 0).length, 1);
  assert.equal(radios(host)[0].tabIndex, 0);
  assert.equal(host.querySelectorAll('.kw-foil-ring').length, 0);
  assert.equal(legendText(host), 'Icon');
});

test('saving: the grid dims and ignores the pointer; the foil stays', async () => {
  const { SpaceIconPicker } = m;
  const host = await mount(React.createElement(SpaceIconPicker, { id: 't', value: '☕', colour: '#10b981', onChange() {}, just: null, onJustEnd() {}, gridClass: '', buttonClass: '', glyphClass: '', disabled: true }));
  const grid = host.querySelector('[role="radiogroup"]');
  assert.ok(grid.classList.contains('opacity-60') && grid.classList.contains('pointer-events-none'));
  assert.equal(host.querySelectorAll('.kw-foil-ring').length, 1);
});
