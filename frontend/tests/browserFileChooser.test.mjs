// Part A: the page's own file picker, intercepted instead of Chrome drawing it natively. Covers
// the wire parsing (browserInput.ts) and the stream hook's chooser state machine
// (useBrowserStream.ts) — asks, upload/done, inline errors, cancel, and clearing on hand-back and
// on a fresh server push racing an old one. The sheet component itself (FileChooserModal) isn't
// rendered here: this suite follows the existing convention (see browserStreamAck.test.mjs) of
// testing the hook's state, not component DOM.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; sockets.push(this); }
  send(text) { this.sent.push(JSON.parse(text)); }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

const b = await loadBundled(`export * from './src/browserInput';`);
const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { useBrowserStream } from './src/useBrowserStream';
`);
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };

test('fileChooserTitle reads the accept attribute per the brief’s draft wording', () => {
  assert.equal(b.fileChooserTitle('video/*'), 'Choose a video');
  assert.equal(b.fileChooserTitle('image/*'), 'Choose a photo');
  assert.equal(b.fileChooserTitle('VIDEO/MP4'), 'Choose a video', 'case-insensitive');
  assert.equal(b.fileChooserTitle(''), 'Choose a file');
  assert.equal(b.fileChooserTitle('application/pdf'), 'Choose a file');
});

test('parseFileChooser needs a numeric id; accept/multiple default safely', () => {
  assert.deepEqual(b.parseFileChooser({ id: 3, accept: 'video/*', multiple: true }), { id: 3, accept: 'video/*', multiple: true });
  assert.deepEqual(b.parseFileChooser({ id: 3 }), { id: 3, accept: '', multiple: false });
  assert.equal(b.parseFileChooser({ accept: 'video/*' }), null, 'no id: not a chooser');
  assert.equal(b.parseFileChooser(null), null);
});

test('parseFileChooserFailure needs a numeric id; code/error default to the generic failure text', () => {
  assert.deepEqual(b.parseFileChooserFailure({ id: 3, code: 'too_big', error: "That's bigger than kwnt can send here (100 MB)." }), { code: 'too_big', error: "That's bigger than kwnt can send here (100 MB)." });
  assert.deepEqual(b.parseFileChooserFailure({ id: 3 }), { code: 'failed', error: 'Couldn’t send that file.' });
  assert.equal(b.parseFileChooserFailure({ code: 'gone' }), null, 'no id: ignored, same as a stray ack would be');
});

function Probe({ onReady }) {
  const stream = m.useBrowserStream('k1', true);
  onReady(stream);
  return null;
}

async function mountStream() {
  let stream;
  const root = m.createRoot(document.createElement('div'));
  m.flushSync(() => root.render(m.React.createElement(Probe, { onReady: (s) => { stream = s; } })));
  await settle();
  const ws = sockets.at(-1);
  ws.onopen?.();
  await settle();
  return { get stream() { return stream; }, ws, root };
}

test('a file_chooser push opens the sheet; file_chooser_done closes it and leaves the toast text', async () => {
  const { stream, ws, root } = await mountStream();
  assert.equal(stream.fileChooser, null);

  ws.receive({ type: 'file_chooser', id: 1, accept: 'video/*', multiple: false });
  await settle();
  assert.deepEqual(stream.fileChooser, { id: 1, accept: 'video/*', multiple: false });
  assert.equal(stream.fileChooserError, null);
  assert.equal(stream.fileChooserDone, null);

  stream.answerFileChooser(1, ['att_1']);
  assert.deepEqual(ws.sent.at(-1), { action: 'file_chooser', id: 1, attachment_ids: ['att_1'] }, 'answers with the chooser’s own id, not sequence.current');

  ws.receive({ type: 'file_chooser_done', id: 1, names: ['clip.mp4'] });
  await settle();
  assert.equal(stream.fileChooser, null, 'the sheet closes');
  assert.deepEqual(stream.fileChooserDone, { names: ['clip.mp4'] }, 'the toast gets the name to show');

  stream.clearFileChooserDone();
  assert.equal(stream.fileChooserDone, null);
  m.flushSync(() => root.unmount());
});

test('cancelFileChooser sends cancel:true and closes the sheet locally with no reply expected', async () => {
  const { stream, ws, root } = await mountStream();
  ws.receive({ type: 'file_chooser', id: 2, accept: '', multiple: false });
  await settle();
  assert.ok(stream.fileChooser);

  stream.cancelFileChooser(2);
  assert.deepEqual(ws.sent.at(-1), { action: 'file_chooser', id: 2, cancel: true });
  assert.equal(stream.fileChooser, null, 'the sheet closes right away, optimistically');
  m.flushSync(() => root.unmount());
});

test('file_chooser_error shows inline on the sheet it belongs to, and is ignored for a different one', async () => {
  const { stream, ws, root } = await mountStream();
  ws.receive({ type: 'file_chooser', id: 3, accept: 'image/*', multiple: true });
  await settle();

  // A stray error for some other (already-gone) chooser id doesn't touch the open sheet
  ws.receive({ type: 'file_chooser_error', id: 99, code: 'gone', error: 'The page moved on.' });
  await settle();
  assert.equal(stream.fileChooserError, null);
  assert.ok(stream.fileChooser, 'the open sheet is untouched');

  ws.receive({ type: 'file_chooser_error', id: 3, code: 'too_big', error: "That's bigger than kwnt can send here (100 MB)." });
  await settle();
  assert.deepEqual(stream.fileChooserError, { code: 'too_big', error: "That's bigger than kwnt can send here (100 MB)." });
  assert.ok(stream.fileChooser, 'the sheet itself stays open so Try again can retry');

  stream.clearFileChooserError();
  assert.equal(stream.fileChooserError, null);
  m.flushSync(() => root.unmount());
});

test('a newer file_chooser push supersedes an older one and clears its error', async () => {
  const { stream, ws, root } = await mountStream();
  ws.receive({ type: 'file_chooser', id: 4, accept: '', multiple: false });
  ws.receive({ type: 'file_chooser_error', id: 4, code: 'failed', error: "Couldn't send that file." });
  await settle();
  assert.ok(stream.fileChooserError);

  ws.receive({ type: 'file_chooser', id: 5, accept: 'video/*', multiple: false });
  await settle();
  assert.deepEqual(stream.fileChooser, { id: 5, accept: 'video/*', multiple: false });
  assert.equal(stream.fileChooserError, null, 'the new chooser starts with a clean sheet');
  m.flushSync(() => root.unmount());
});

test('hand-back clears an open chooser the same way it closes select/dialog, with no dedicated message', async () => {
  const { stream, ws, root } = await mountStream();
  ws.receive({ type: 'file_chooser', id: 6, accept: '', multiple: false });
  await settle();
  assert.ok(stream.fileChooser);

  ws.receive({ type: 'control', active: false, viewport: { width: 1280, height: 800 } });
  await settle();
  assert.equal(stream.fileChooser, null);
  assert.equal(stream.fileChooserError, null);
  assert.equal(stream.fileChooserDone, null);
  m.flushSync(() => root.unmount());
});

test('a disconnect clears the open chooser so a reconnect never shows a stale sheet', async () => {
  const { stream, ws, root } = await mountStream();
  ws.receive({ type: 'file_chooser', id: 7, accept: '', multiple: false });
  await settle();
  assert.ok(stream.fileChooser);

  ws.onclose?.();
  await settle();
  assert.equal(stream.fileChooser, null);
  m.flushSync(() => root.unmount());
});
