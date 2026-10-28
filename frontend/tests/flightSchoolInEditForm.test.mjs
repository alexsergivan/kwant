// Flight School tier panel inside the Edit bot form (Rigel eval-16 D1): the panel's "add a kind of
// work" row used to be a form nested in the Edit form, so Set and Enter submitted the Edit form, closed
// the dialog and saved nothing. These render the real NewAgentModal in a DOM and press Set and Enter.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1/b/k_sandy', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'SubmitEvent', 'HTMLFormElement', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'matchMedia']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;
// jsdom has no layout or audio; the modal only needs these not to throw.
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};
g.window.HTMLMediaElement && (g.window.HTMLMediaElement.prototype.play = () => Promise.resolve());

const caps = {
  flight_school: true,
  kwant_id: 'k_sandy',
  default_tier: 'sandbox',
  policies: [],
  effective: [{ task_class: 'general', tier: 'sandbox', from: 'any_class' }],
  events: [],
};
let posts = [];
const json = (body) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
g.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  if (u.startsWith('/api/capabilities') && method === 'POST') {
    const body = JSON.parse(init.body);
    posts.push({ url: u, body });
    const next = { ...caps, effective: [...caps.effective, { task_class: body.task_class, tier: body.tier, from: 'class' }] };
    return json({ capabilities: next, unchanged: false });
  }
  if (u.startsWith('/api/capabilities')) return json(caps);
  return json([]);
};
g.window.fetch = g.fetch;

const { React, createRoot, NewAgentModal } = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { NewAgentModal } from './src/components/NewAgentModal';
`);

// The bundle is React's production build, which has no act(): run the step, then let React and the
// stubbed fetches finish.
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const act = async (fn) => {
  await fn();
  await settle();
};

async function openEdit() {
  posts = [];
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const calls = { submit: 0, close: 0, nativeSubmit: 0 };
  await act(async () => {
    root.render(
      React.createElement(NewAgentModal, {
        isOpen: true,
        kwantId: 'k_sandy',
        initial: { name: 'Sandy', title: 'Writer', persona_prompt: 'p', avatar_sprite: 'architect', preferred_model: 'auto' },
        onClose: () => calls.close++,
        onSubmit: () => {
          calls.submit++;
        },
      }),
    );
  });
  await settle();
  const panel = host.querySelector('[data-flight-school]');
  assert.ok(panel, 'the tier panel shows in the Edit form with Flight School on');
  const form = panel.closest('form');
  assert.ok(form, 'the panel sits inside the Edit bot form');
  // A native submit would leave the page; count any submit that React didn't stop.
  form.addEventListener('submit', (e) => {
    if (!e.defaultPrevented) calls.nativeSubmit++;
  });
  const input = panel.querySelector('input[placeholder^="Another kind of work"]');
  const setBtn = [...panel.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Set');
  const type = async (v) => {
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, v);
      input.dispatchEvent(new window.Event('input', { bubbles: true }));
    });
  };
  const done = () => {
    root.unmount();
    host.remove();
  };
  return { panel, form, input, setBtn, type, calls, done };
}

test('the tier panel has no form of its own and every button in it is type="button"', async () => {
  const v = await openEdit();
  assert.equal(v.panel.querySelectorAll('form').length, 0, 'no form nested in the Edit form');
  const buttons = [...v.panel.querySelectorAll('button')];
  assert.ok(buttons.length >= 2);
  for (const b of buttons) assert.equal(b.getAttribute('type'), 'button', `${b.textContent.trim()} can't submit the Edit form`);
  v.done();
});

test('Set inside the Edit form saves the new kind of work and does not submit the Edit form', async () => {
  const v = await openEdit();
  await v.type('Release notes');
  assert.equal(v.setBtn.disabled, false);
  await act(async () => v.setBtn.click());
  await settle();
  assert.deepEqual(posts, [{ url: '/api/capabilities', body: { kwant_id: 'k_sandy', task_class: 'release-notes', tier: 'sandbox' } }]);
  assert.equal(v.calls.submit, 0, 'the Edit form was not saved');
  assert.equal(v.calls.nativeSubmit, 0, 'no page submit');
  assert.equal(v.calls.close, 0, 'the dialog stays open');
  assert.equal(v.input.value, '', 'the field clears after saving');
  assert.ok(v.panel.querySelector('[data-class="release-notes"][data-tier="sandbox"]'), 'the new kind of work is listed');
  v.done();
});

test('Enter in the new kind of work field saves it and does not submit the Edit form', async () => {
  const v = await openEdit();
  await v.type('deploy');
  const ev = new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  await act(async () => v.input.dispatchEvent(ev));
  await settle();
  // Browsers submit the enclosing form on Enter unless keydown is cancelled; jsdom doesn't, so check both.
  assert.equal(ev.defaultPrevented, true, 'Enter is kept from submitting the Edit form');
  assert.deepEqual(posts, [{ url: '/api/capabilities', body: { kwant_id: 'k_sandy', task_class: 'deploy', tier: 'sandbox' } }]);
  assert.equal(v.calls.submit, 0);
  assert.equal(v.calls.nativeSubmit, 0);
  assert.equal(v.calls.close, 0);
  v.done();
});

test('Enter with an empty field does nothing and still does not submit the Edit form', async () => {
  const v = await openEdit();
  const ev = new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  await act(async () => v.input.dispatchEvent(ev));
  await settle();
  assert.equal(ev.defaultPrevented, true);
  assert.deepEqual(posts, []);
  assert.equal(v.calls.submit, 0);
  v.done();
});
