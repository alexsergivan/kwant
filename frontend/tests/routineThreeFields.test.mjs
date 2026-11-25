// 36i routine-three-fields (autonomy/routine-three-fields/brief.md, Mira's simplify review #9): a new routine asks for
// Name, What to do and When; the bot and where it posts are one folded line "<Bot> · posts in <place> · Change".
// Change opens today's two selects in place; "Pick a bot." opens them too; editing keeps the bot and the place.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLSelectElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { createRoot } from 'react-dom/client';
  import { flushSync } from 'react-dom';
  import { RoutineForm } from './src/components/RoutinesPanel';
  export const form = (props) => renderToStaticMarkup(React.createElement(RoutineForm, props));
  export const mount = (props) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(RoutineForm, props)));
    return { host, unmount: () => { flushSync(() => root.unmount()); host.remove(); } };
  };
`);

const noop = () => {};
const text = (html) => html.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ');
const kwants = [
  { id: 'k1', name: 'Ben', avatar_sprite: 'ben' },
  { id: 'k2', name: 'Ava', avatar_sprite: 'ava' },
];
const channels = [{ id: 'c1', name: 'spring-launch' }];
const props = (over = {}) => ({ kwants, channels, onCancel: noop, onSaved: noop, ...over });
const tick = () => new Promise((r) => setTimeout(r, 0));
// A moment, so React has rendered what the click changed.
const frame = () => new Promise((r) => setTimeout(r, 40));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await frame();
};
const saved = (over = {}) => ({
  id: 'r1', kwant_id: 'k2', channel_id: 'c1', thread_id: null, name: 'Competitor prices', instructions: 'Check prices.',
  schedule: { type: 'daily', time: '09:00', tz: 'Europe/Kyiv' }, enabled: true, ...over,
});

test('a new routine shows Name, What to do and When, then one line for the bot and the place', () => {
  const h = ui.form(props());
  const t = text(h);
  assert.ok(t.indexOf('Name') < t.indexOf('What to do'));
  assert.ok(t.indexOf('What to do') < t.indexOf('When'));
  assert.ok(t.indexOf('When') < t.indexOf('Ben · posts in direct chat'), 'the folded line comes after When');
  assert.match(h, /data-routine-who-line[^>]*>.*Ben · posts in direct chat<\/span>/);
  assert.match(h, /aria-expanded="false"[^>]*aria-label="Change the bot and where it posts, now Ben · posts in direct chat"[^>]*data-routine-who-change[^>]*>Change</);
  // The two selects are folded away.
  assert.doesNotMatch(h, /<select/);
  assert.doesNotMatch(h, /data-routine-who-fields/);
  assert.doesNotMatch(t, /\bBot\b|Posts in/);
});

test('Change opens both selects in place with today\'s defaults, and Done folds them', async () => {
  const { host, unmount } = ui.mount(props());
  try {
    const btn = host.querySelector('[data-routine-who-change]');
    await click(btn);
    assert.equal(btn.getAttribute('aria-expanded'), 'true');
    assert.equal(btn.textContent, 'Done');
    const fields = host.querySelector('[data-routine-who-fields]');
    assert.equal(btn.getAttribute('aria-controls'), fields.id);
    const [bot, place] = fields.querySelectorAll('select');
    assert.equal(bot.value, 'k1');
    assert.equal(place.value, '');
    assert.ok(document.activeElement === bot, 'the bot select takes focus');
    await click(btn);
    assert.equal(host.querySelector('[data-routine-who-fields]'), null);
    assert.equal(btn.textContent, 'Change');
  } finally {
    unmount();
  }
});

test('"Pick a bot." opens the fold so the select is visible', async () => {
  const seed = { kwant_id: '', channel_id: null, name: 'Morning', instructions: 'Check prices.', schedule: { type: 'daily', time: '09:00', tz: 'Europe/Kyiv' } };
  const { host, unmount } = ui.mount(props({ kwants: [], seed }));
  try {
    const btn = host.querySelector('[data-routine-who-change]');
    assert.equal(btn.getAttribute('aria-expanded'), 'true', 'with no bot the selects start open');
    await click(btn);
    assert.equal(host.querySelector('[data-routine-who-fields]'), null);
    host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
    await tick();
    assert.match(host.querySelector('[role="alert"]').textContent, /Pick a bot\./);
    assert.ok(host.querySelector('[data-routine-who-fields] select'), 'the bot select is back');
    assert.equal(btn.getAttribute('aria-expanded'), 'true');
  } finally {
    unmount();
  }
});

test('editing shows the routine\'s own bot and place, and saving keeps them', async () => {
  const h = ui.form(props({ initial: saved() }));
  assert.match(text(h), /Ava · posts in #spring-launch/);
  let got = null;
  const { host, unmount } = ui.mount(props({ initial: saved(), onSubmit: async (body, placeChanged) => { got = { body, placeChanged }; } }));
  try {
    await click(host.querySelector('[data-routine-who-change]'));
    const [bot, place] = host.querySelectorAll('[data-routine-who-fields] select');
    assert.equal(bot.value, 'k2');
    assert.equal(place.value, 'c1');
    host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
    await tick();
    assert.equal(got.body.kwant_id, 'k2');
    assert.equal(got.body.channel_id, 'c1');
    assert.equal(got.placeChanged, false);
  } finally {
    unmount();
  }
});

test('with a fixed bot the line names only the place', () => {
  const h = ui.form(props({ fixedKwantId: 'k2' }));
  const t = text(h);
  assert.match(t, /Posts in direct chat/);
  assert.doesNotMatch(t, /Ava|Ben/);
  assert.match(h, /aria-label="Change where it posts, now direct chat"/);
});

test('a follow-up set up in a thread keeps its thread', async () => {
  const r = saved({ channel_id: null, thread_id: 't1' });
  assert.match(text(ui.form(props({ initial: r }))), /Ava · posts in the thread it was set up in/);
  let got = null;
  const { host, unmount } = ui.mount(props({ initial: r, onSubmit: async (body, placeChanged) => { got = { body, placeChanged }; } }));
  try {
    await click(host.querySelector('[data-routine-who-change]'));
    const place = host.querySelectorAll('[data-routine-who-fields] select')[1];
    assert.equal(place.value, '__thread__');
    assert.match(place.options[0].textContent, /The thread it was set up in/);
    host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
    await tick();
    assert.equal(got.placeChanged, false, 'the place is not sent again, so it stays in its thread');
  } finally {
    unmount();
  }
});

test('the When block\'s small labels stay labels for screen readers', () => {
  const h = ui.form(props());
  assert.match(h, /<label for="[^"]+-time"[^>]*>At<\/label>/);
  assert.match(h, /aria-labelledby="[^"]+-tz-label"/);
});

test('a folded line points at no missing id', () => {
  assert.doesNotMatch(ui.form(props()), /aria-controls/);
});
