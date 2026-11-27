// party-card-and-results (autonomy/party-card-and-results/brief.md, item 5): photos waiting on a wall are one
// Needs-you row, "2 photos waiting on Office holiday party" (count first, Mira fix 2), with the next photo and Approve / Hide that
// go through the wall's own host actions.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
const calls = [];
let hold = null; // a promise a POST waits on, so a test can look at the row while the action runs
g.fetch = async (url, init = {}) => {
  calls.push({ url: String(url), method: init.method ?? 'GET', body: init.body ? JSON.parse(init.body) : null });
  if (hold && (init.method ?? 'GET') === 'POST') await hold;
  return { ok: true, status: 200, headers: new Map([['content-type', 'application/json']]), json: async () => ({}), text: async () => '{}' };
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as store from './src/inboxStore';
  export { INBOX_FIXTURE_EMPTY, wallNext } from './src/api/inbox';
  export { InboxNew } from './src/components/InboxNew';
`);
const { React, store: s } = m;
const h = React.createElement;
const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

// Shaped like src/server/inbox_pockets.rs wall_needs_you + inbox.rs wall_item.
const row = {
  kind: 'pocket_wall', id: 'wall:W4LLcode', kwant_id: 'k1', kwant_name: 'Ben', pocket_code: 'W4LLcode', pocket_title: 'Office holiday party',
  title: '2 photos waiting on Office holiday party', summary: 'Next: a photo from Cleo Marsh', waiting: 2,
  next_photo: { id: 'p_01', by_name: 'Cleo Marsh' }, status: 'open', timestamp: '2026-11-27T18:00:00Z',
  link: { thread_id: null, channel_id: null, direct_kwant_id: 'k1' }, group: 'waiting_on_you', sort_key: [0, 0, 0, 'Ben', 'wall:W4LLcode'],
};
const data = (items = [row]) => {
  const d = JSON.parse(JSON.stringify(m.INBOX_FIXTURE_EMPTY));
  d.items = items;
  d.counts.waiting_on_you = items.length;
  return d;
};
const render = (props = {}, items) => {
  s.inboxStore.reset();
  s.inboxStore.patch({ data: data(items) });
  const html = m.renderToStaticMarkup(h(m.InboxNew, { noLoad: true, wide: false, onOpen: () => {}, ...props }));
  return new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
};

test('a wall row says how many photos wait, shows the next one, and offers Approve and Hide in Needs you', () => {
  const d = render();
  const li = d.querySelector('[data-row="wall:W4LLcode"]');
  assert.ok(li, 'the row is listed');
  assert.equal(li.getAttribute('data-group'), 'waiting_on_you');
  assert.match(li.textContent, /PM2 photos waiting on Office holiday partyPhotos to check/);
  assert.match(li.textContent, /Photos to check/);
  const img = li.querySelector('[data-wall-thumb="p_01"]');
  assert.equal(img.getAttribute('src'), '/api/pockets/W4LLcode/photo/p_01');
  assert.equal(img.getAttribute('alt'), 'Photo from Cleo Marsh');
  const ok = li.querySelector('[data-wall-approve="p_01"]');
  const hide = li.querySelector('[data-wall-hide="p_01"]');
  assert.equal(ok.textContent.trim(), 'Approve');
  assert.equal(hide.textContent.trim(), 'Hide');
  assert.match(ok.className, /h-11/, '44 px on a phone');
  assert.match(hide.className, /h-11/);
});

test('wide, the buttons are the compact row size', () => {
  const li = render({ wide: true }).querySelector('[data-row="wall:W4LLcode"]');
  assert.match(li.querySelector('[data-wall-approve]').className, /h-7/);
});

test('a row whose code or photo id is not safe to put in a URL gets no buttons', () => {
  assert.equal(m.wallNext({ ...row, pocket_code: '../x' }), null);
  assert.equal(m.wallNext({ ...row, next_photo: { id: 'a/b', by_name: 'x' } }), null);
  assert.equal(m.wallNext({ ...row, next_photo: null }), null);
  assert.equal(m.wallNext({ ...row, kind: 'pocket' }), null);
  assert.deepEqual(m.wallNext({ ...row, next_photo: { id: 'p_01', by_name: '  ' } }), { code: 'W4LLcode', id: 'p_01', by: null });
  const d = render({}, [{ ...row, pocket_code: '../x' }]);
  assert.equal(d.querySelector('[data-wall-approve]'), null);
  assert.match(d.body.textContent, /2 photos waiting on Office holiday party/, 'the row itself still shows');
});

for (const [action, button, said] of [
  ['approve', '[data-wall-approve="p_01"]', "Approved the photo from Cleo Marsh. It's on the wall now."],
  ['hide', '[data-wall-hide="p_01"]', "Hid the photo from Cleo Marsh. Guests won't see it."],
]) {
  test(`${action} sends the wall's own ${action} action for the next photo, says so, and reloads the Inbox`, async () => {
    calls.length = 0;
    document.body.innerHTML = '';
    s.inboxStore.reset();
    let loads = 0;
    s.inboxSource.get = async () => (loads++, data([]));
    s.inboxStore.patch({ data: data() });
    const el = document.createElement('div');
    document.body.appendChild(el);
    const root = m.createRoot(el);
    root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: true }));
    await settle();
    el.querySelector(button).dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle(40);
    const post = calls.find((c) => c.method === 'POST');
    assert.ok(post, 'one POST went out');
    assert.equal(post.url, '/api/pockets/W4LLcode/action');
    assert.deepEqual(post.body, { name: action, body: { ids: ['p_01'] } });
    assert.ok(loads >= 1, 'the Inbox reloads after');
    assert.equal(s.inboxStore.get().notice?.text, said);
    assert.equal(el.querySelector('[data-wall-row]'), null, 'no photo waits, so the row is gone');
    root.unmount();
  });
}

// Mira's design check fix 3: a tap on a slow box must show it is working, and a second tap must not land on
// the next photo after the reload.
for (const [action, tapped, other, busyWord, otherWord] of [
  ['approve', '[data-wall-approve="p_01"]', '[data-wall-hide="p_01"]', 'Approving…', 'Hide'],
  ['hide', '[data-wall-hide="p_01"]', '[data-wall-approve="p_01"]', 'Hiding…', 'Approve'],
]) {
  test(`while ${action} runs, only the tapped button says "${busyWord}" with a spinner and aria-busy; both stay disabled`, async () => {
    calls.length = 0;
    document.body.innerHTML = '';
    s.inboxStore.reset();
    s.inboxSource.get = async () => data([]);
    s.inboxStore.patch({ data: data() });
    let release;
    hold = new Promise((r) => (release = r));
    const el = document.createElement('div');
    document.body.appendChild(el);
    const root = m.createRoot(el);
    root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: true }));
    await settle();
    el.querySelector(tapped).dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    const t = el.querySelector(tapped);
    const o = el.querySelector(other);
    assert.equal(t.textContent.trim(), busyWord);
    assert.equal(t.getAttribute('aria-busy'), 'true');
    assert.ok(t.querySelector('svg.animate-spin.motion-reduce\\:animate-none'), 'the spinner, still for reduced motion');
    assert.equal(o.textContent.trim(), otherWord, 'the other button keeps its word');
    assert.equal(o.getAttribute('aria-busy'), null);
    assert.ok(t.disabled && o.disabled, 'both are disabled while it runs');
    assert.doesNotMatch(el.textContent, /Sending…/, 'no second working line under the row');
    // A second tap does nothing: still one POST.
    o.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    t.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    assert.equal(calls.filter((c) => c.method === 'POST').length, 1);
    release();
    hold = null;
    await settle(40);
    assert.equal(el.querySelector('[data-wall-row]'), null, 'done: the reload drops the row');
    root.unmount();
  });
}
