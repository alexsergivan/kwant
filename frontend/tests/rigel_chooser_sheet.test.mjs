// Rigel QA for #578 (Round 2): the FileChooserModal sheet itself, rendered with jsdom + react-dom.
// Adapted from her original evidence (rigel-evidence/rigel_chooser_sheet.test.mjs) for Mira's
// Round 2 fix: a "gone" failure now shows a single focused OK button, not Cancel + Try again.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id=root></div></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'DragEvent', 'File', 'FileList', 'DataTransfer', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
const m = await loadBundled(`
  import React from 'react';
  import { createRoot } from 'react-dom/client';
  import { flushSync } from 'react-dom';
  import { FileChooserModal } from './src/components/BrowserPopups';
  export { React, createRoot, flushSync, FileChooserModal };
`);
const { React, createRoot, flushSync, FileChooserModal } = m;
// the shared bundle helper builds React in production mode, where act() is unavailable: flush by hand
async function act(fn) { let p; flushSync(() => { p = fn(); }); await p; await new Promise((r) => setTimeout(r, 30)); }

async function mount(props) {
  const host = document.getElementById('root'); host.innerHTML = '';
  const root = createRoot(host);
  const calls = { answer: [], cancel: 0, clear: 0 };
  const el = (p) => React.createElement(FileChooserModal, { request: { id: 1, accept: 'video/*', multiple: false }, failure: null, onAnswer: (ids) => calls.answer.push(ids), onCancel: () => calls.cancel++, onClearFailure: () => calls.clear++, ...p, ...props });
  await act(async () => { root.render(el()); });
  return { host, calls, root, rerender: (p) => act(async () => { root.render(el(p)); }) };
}
const buttons = (host) => [...host.querySelectorAll('button')].map((b) => b.textContent.trim());

test('asks: title from accept, Choose file… focused first, Cancel present', async () => {
  const { host } = await mount();
  assert.match(host.textContent, /Choose a video/);
  assert.deepEqual(buttons(host), ['Cancel', 'Choose file…']);
  assert.equal(document.activeElement?.textContent?.trim(), 'Choose file…', 'focus starts on the primary button');
});
test('titles for image and other', async () => {
  let { host } = await mount({ request: { id: 2, accept: 'image/png', multiple: false } });
  assert.match(host.textContent, /Choose a photo/);
  ({ host } = await mount({ request: { id: 3, accept: '', multiple: false } }));
  assert.match(host.textContent, /Choose a file/);
});
test('Escape cancels', async () => {
  const { host, calls } = await mount();
  const dlg = host.querySelector('[role=dialog]');
  await act(async () => { dlg.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
  assert.equal(calls.cancel, 1);
});
test('"gone": a single focused OK button, no Try again (Mira item 2)', async () => {
  const { host } = await mount({ failure: { code: 'gone', error: 'The page moved on.' } });
  assert.match(host.textContent, /The page moved on and closed its file picker/);
  assert.deepEqual(buttons(host), ['OK']);
  assert.equal(document.activeElement?.textContent?.trim(), 'OK', 'OK is focused');
});
test('a recoverable failure (not gone): message + Try again returns to picking', async () => {
  const { host, calls, rerender } = await mount();
  await rerender({ failure: { code: 'failed', error: 'Couldn’t send that file to the page.' } });
  assert.match(host.textContent, /Couldn’t send that file to the page\./);
  assert.deepEqual(buttons(host), ['Cancel', 'Try again']);
  const retry = [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Try again');
  await act(async () => { retry.click(); });
  assert.equal(calls.clear, 1);
});
test('client-side preflight: a .docx is refused inline with the supported list', async () => {
  const { host } = await mount({ request: { id: 4, accept: '', multiple: false } });
  const input = host.querySelector('input[type=file]');
  const f = new dom.window.File(['x'.repeat(10)], 'cv.docx', { type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' });
  Object.defineProperty(input, 'files', { value: [f], configurable: true });
  await act(async () => { input.dispatchEvent(new dom.window.Event('change', { bubbles: true })); });
  assert.ok(host.querySelector('[role=alert]'));
});
test('input accept/multiple follow the request', async () => {
  const { host } = await mount({ request: { id: 5, accept: 'video/*', multiple: true } });
  const input = host.querySelector('input[type=file]');
  assert.equal(input.getAttribute('accept'), 'video/*');
  assert.equal(input.multiple, true);
});
test('dropping a mismatched file on the sheet is refused inline (Mira item 1)', async () => {
  const { host } = await mount({ request: { id: 6, accept: 'video/*', multiple: false } });
  const dlg = host.querySelector('[role=dialog]');
  const f = new dom.window.File(['x'], 'photo.jpg', { type: 'image/jpeg' });
  const ev = new dom.window.Event('drop', { bubbles: true, cancelable: true });
  ev.dataTransfer = { files: [f], items: [{ kind: 'file', getAsFile: () => f }], types: ['Files'] };
  await act(async () => { dlg.dispatchEvent(ev); });
  assert.ok(ev.defaultPrevented, 'the drop is handled, not left to the browser');
  assert.match(host.textContent, /This page asks for a video\./);
});
