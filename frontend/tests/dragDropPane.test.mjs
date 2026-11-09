// 0.8.0 drag and drop into chats (Pixel): the whole open chat takes a file drop (message list, header
// and composer), not only the composer box; after the drop the files sit in the same tray and the
// cursor is in the field. Paste, the paperclip, the 10-file limit and /idea mode are unchanged.
// The pane cases fail on main fd3dfafa, where only the <form> listens.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'FocusEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS', 'DOMException']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const store = new Map();
const ls = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
dom.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
// Uploads start right away; these never finish, so the chips stay "uploading".
const sent = [];
globalThis.XMLHttpRequest = class {
  upload = {};
  open(_m, url) { this.url = url; }
  send(body) { sent.push({ url: this.url, body }); }
  abort() {}
};

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ChatView, p)));
    return { host, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const act = (fn) => flushSync(fn);
`);

const props = (extra = {}) => ({
  title: 'Frontend', subtitle: 'Pixel', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, onCancelDiscussion: () => {}, kwants: [], teams: [], ...extra,
});

const mkFile = (name, type = 'image/png', size = 2048) => {
  const f = new dom.window.File(['x'.repeat(Math.min(size, 16))], name, { type });
  Object.defineProperty(f, 'size', { value: size });
  return f;
};
// jsdom has no DataTransfer: a stand-in with types, items and files, as Chrome gives them.
const dataTransfer = (files, extraTypes = []) => ({
  types: files.length ? ['Files', ...extraTypes] : extraTypes,
  files,
  items: files.map((f) => ({ kind: 'file', type: f.type, getAsFile: () => f, webkitGetAsEntry: () => ({ isDirectory: false, name: f.name }) })),
  dropEffect: 'none',
  effectAllowed: 'all',
});
// Drag events are continuous-priority in React: their state lands on the next tick.
const tick = () => new Promise((r) => setTimeout(r, 0));
const fire = async (el, type, dt) => {
  const e = new dom.window.Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(e, 'dataTransfer', { value: dt });
  ui.act(() => el.dispatchEvent(e));
  await tick();
  return e;
};
const parts = (host) => ({
  pane: host.firstElementChild,
  header: host.querySelector('[data-chat-header]'),
  list: host.querySelector('[role="log"]'),
  form: host.querySelector('form[data-composer-form]'),
  field: host.querySelector('textarea'),
});
const chips = (host) => host.querySelectorAll('ul[aria-label="Files to send"] > li').length;
const overlayOn = (host) => host.textContent.includes('Drop files to attach') || !!host.querySelector('[data-drop-summary]');

test('a file drag over the header or the message list lights up the chat, not only over the box', async () => {
  const v = ui.mount(props());
  try {
    const { header, list } = parts(v.host);
    assert.ok(header && list, 'the chat has a header and a message list');
    const enter = await fire(header, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(enter.defaultPrevented, true, 'the header accepts the drag');
    assert.ok(overlayOn(v.host), 'the drop overlay shows');
    // Moving onto a child element: enter the list, then leave the header. No flicker.
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    await fire(header, 'dragleave', dataTransfer([mkFile('a.png')]));
    assert.ok(overlayOn(v.host), 'still on while the drag is anywhere in the chat');
    const over = await fire(list, 'dragover', dataTransfer([mkFile('a.png')]));
    assert.equal(over.defaultPrevented, true);
    assert.equal(over.dataTransfer.dropEffect, 'copy');
    await fire(list, 'dragleave', dataTransfer([mkFile('a.png')]));
    assert.ok(!overlayOn(v.host), 'off once the drag leaves the chat');
  } finally {
    v.unmount();
  }
});

test('a drop on the message list lands in the tray, the cursor goes to the field and it is read out', async () => {
  sent.length = 0;
  const v = ui.mount(props());
  try {
    const { list, field } = parts(v.host);
    const files = [mkFile('one.png'), mkFile('two.pdf', 'application/pdf'), mkFile('three.md', 'text/markdown')];
    await fire(list, 'dragenter', dataTransfer(files));
    const drop = await fire(list, 'drop', dataTransfer(files));
    assert.equal(drop.defaultPrevented, true, 'the browser does not open the file');
    assert.equal(chips(v.host), 3, 'three files in the tray');
    assert.equal(sent.length, 3, 'all three upload as with the paperclip');
    assert.ok(!overlayOn(v.host));
    assert.equal(document.activeElement, field, 'the cursor is in the field');
    const live = [...v.host.querySelectorAll('[aria-live="polite"]')].map((n) => n.textContent);
    assert.ok(live.includes('3 files added'), `announced: ${JSON.stringify(live)}`);
  } finally {
    v.unmount();
  }
});

test('text and links dragged over the chat are left alone', async () => {
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    const e = await fire(list, 'dragenter', dataTransfer([], ['text/plain', 'text/uri-list']));
    assert.equal(e.defaultPrevented, false);
    assert.ok(!overlayOn(v.host));
  } finally {
    v.unmount();
  }
});

test('unchanged: the box itself still takes a drop, and eleven files keep ten with the note', async () => {
  sent.length = 0;
  const v = ui.mount(props());
  try {
    const { form } = parts(v.host);
    const eleven = Array.from({ length: 11 }, (_, i) => mkFile(`p${i}.png`));
    await fire(form, 'dragenter', dataTransfer(eleven));
    await fire(form, 'drop', dataTransfer(eleven));
    assert.equal(chips(v.host), 10);
    assert.match(v.host.textContent, /Up to 10 files per message; one was left out\./);
  } finally {
    v.unmount();
  }
});

test('unchanged: paste and the paperclip add files to the same tray, with the same checks', async () => {
  const v = ui.mount(props());
  try {
    const { field } = parts(v.host);
    const paste = new dom.window.Event('paste', { bubbles: true, cancelable: true });
    const shot = mkFile('shot.png');
    Object.defineProperty(paste, 'clipboardData', { value: { items: [{ kind: 'file', type: 'image/png', getAsFile: () => shot }], types: ['Files'] } });
    ui.act(() => field.dispatchEvent(paste));
    await tick();
    assert.equal(chips(v.host), 1);

    const input = v.host.querySelector('input[type="file"]');
    Object.defineProperty(input, 'files', { value: [mkFile('big.pdf', 'application/pdf', 21 * 1024 * 1024), mkFile('ok.txt', 'text/plain')], configurable: true });
    ui.act(() => input.dispatchEvent(new dom.window.Event('change', { bubbles: true })));
    await tick();
    assert.equal(chips(v.host), 2, 'the 20 MB file is caught before upload');
    assert.match(v.host.textContent, /big\.pdf is too big/);
  } finally {
    v.unmount();
  }
});

test('/idea mode still refuses files: a drop lands, the jar line asks to remove it', async () => {
  const v = ui.mount(props());
  try {
    const { list, field } = parts(v.host);
    const set = Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype, 'value').set;
    ui.act(() => { set.call(field, '/idea a faster search'); field.dispatchEvent(new dom.window.Event('input', { bubbles: true })); });
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    await fire(list, 'drop', dataTransfer([mkFile('a.png')]));
    assert.match(v.host.textContent, /Ideas are text only\. Remove the file to drop it in\./);
  } finally {
    v.unmount();
  }
});

test('while dragging, the chat says what is coming; a dropped folder is left out with the folder line', async () => {
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    const three = [mkFile('a.png'), mkFile('b.jpg', 'image/jpeg'), mkFile('c.pdf', 'application/pdf')];
    await fire(list, 'dragenter', dataTransfer(three));
    assert.equal(v.host.querySelector('[data-drop-summary]')?.textContent, '3 files · 2 images, 1 PDF');
    await fire(list, 'dragleave', dataTransfer(three));

    const png = mkFile('cat.png');
    const dt = dataTransfer([png]);
    dt.items.unshift({ kind: 'file', type: '', getAsFile: () => mkFile('Photos', '', 0), webkitGetAsEntry: () => ({ isDirectory: true, name: 'Photos' }) });
    await fire(list, 'dragenter', dt);
    await fire(list, 'drop', dt);
    assert.equal(chips(v.host), 1, 'only the file');
    assert.match(v.host.textContent, /Photos is a folder\. Folders can't go: drop the files inside\./);
  } finally {
    v.unmount();
  }
});
