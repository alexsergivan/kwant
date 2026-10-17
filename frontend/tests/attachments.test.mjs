import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const a = await loadBundled(`export * from './src/attachments';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { FileTiles, UploadChip } from './src/components/Attachments';
  export const tiles = (files) => renderToStaticMarkup(React.createElement(FileTiles, { files }));
  export const chip = (item) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(UploadChip, { item, onRemove() {}, onRetry() {} })));
`);

const ID = '0b6f3c1e-2a4d-4e5f-8a9b-1c2d3e4f5a6b';
const ID2 = '1b6f3c1e-2a4d-4e5f-8a9b-1c2d3e4f5a6b';
const MB = 1024 * 1024;

test('the 20 MB limit and empty files are caught before upload', () => {
  assert.equal(a.preflightFile({ name: 'ok.pdf', size: 20 * MB }), null);
  assert.match(a.preflightFile({ name: 'big.mov', size: 20 * MB + 1 }), /big\.mov is too big \(20\.0 MB\)\. Max 20 MB\./);
  assert.match(a.preflightFile({ name: 'e.txt', size: 0 }), /empty/);
});

test('server refusals read as plain language', () => {
  assert.match(a.uploadErrorText(413), /Max 20 MB/);
  assert.match(a.uploadErrorText(415), /isn't supported/);
  assert.equal(a.uploadErrorText(400, 'bad name'), 'bad name');
  assert.match(a.uploadErrorText(0), /interrupted/);
  assert.match(a.uploadErrorText(500), /HTTP 500/);
  // Retry can't help when the server refused the file itself
  for (const st of [400, 413, 415]) assert.equal(a.uploadRetryable(st), false, `HTTP ${st}`);
  for (const st of [0, 401, 429, 500, 503]) assert.equal(a.uploadRetryable(st), true, `HTTP ${st}`);
  const e = new a.UploadError(415);
  assert.equal(e.status, 415);
  assert.match(e.message, /isn't supported/);
  assert.match(a.sendAttachmentErrorText(409), /already sent/);
  assert.equal(a.sendAttachmentErrorText(500), null);
});

test('message files parse from meta and skip malformed or unsafe ids', () => {
  const meta = JSON.stringify({ attachments: [
    { id: ID, name: 'cat.png', mime: 'image/png', size: 2048 },
    { id: '../../etc/passwd', name: 'x' },
    { id: ID2, name: 'notes' },
    null,
  ] });
  const files = a.parseAttachments(meta);
  assert.deepEqual(files.map((f) => f.id), [ID, ID2]);
  assert.equal(files[1].mime, 'application/octet-stream');
  assert.deepEqual(a.parseAttachments('nope'), []);
  assert.deepEqual(a.parseAttachments({ kind: 'ask_user' }), []);
  assert.equal(a.isSharedFileMessage(JSON.stringify({ kind: 'file', attachments: [] })), true);
  assert.equal(a.isSharedFileMessage(meta), false);
});

test('only raster images render inline; sizes and kinds are human', () => {
  assert.equal(a.isInlineImage('image/PNG'), true);
  assert.equal(a.isInlineImage('image/svg+xml'), false);
  assert.equal(a.isInlineImage('application/pdf'), false);
  assert.equal(a.formatBytes(512), '512 B');
  assert.equal(a.formatBytes(1536), '1.5 KB');
  assert.equal(a.formatBytes(3 * MB), '3.0 MB');
  assert.equal(a.fileKind('image/jpeg', 'a.jpg'), 'JPG');
  assert.equal(a.fileKind('application/octet-stream', 'data.zip'), 'ZIP');
  assert.equal(a.attachmentUrl(ID, true), `/api/attachments/${ID}/content?download=1`);
});

test('the composer sends only when every file is uploaded', () => {
  const done = { key: 'k1', name: 'a.png', size: 1, progress: 1, state: 'done', attachment: { id: ID, name: 'a.png', mime: 'image/png', size: 1 } };
  const up = { key: 'k2', name: 'b.pdf', size: 1, progress: 0.4, state: 'uploading' };
  const bad = { key: 'k3', name: 'c.exe', size: 1, progress: 0, state: 'error', error: 'nope' };
  assert.equal(a.composerReady('', []), false);
  assert.equal(a.composerReady('hi', []), true);
  assert.equal(a.composerReady('', [done]), true);
  assert.equal(a.composerReady('hi', [done, up]), false);
  assert.equal(a.composerReady('hi', [bad]), false);
  assert.deepEqual(a.readyAttachments([done, up]), [{ id: ID, name: 'a.png', mime: 'image/png', size: 1 }]);
  assert.equal(a.uploadSlotsLeft(Array(9).fill(done)), 1);
  assert.equal(a.uploadSlotsLeft(Array(12).fill(done)), 0);
});

test('tiles: images inline with a download, other files download only', () => {
  const html = ui.tiles([
    { id: ID, name: 'cat.png', mime: 'image/png', size: 2048 },
    { id: ID2, name: 'report.pdf', mime: 'application/pdf', size: 3 * MB },
  ]);
  assert.match(html, new RegExp(`<img src="/api/attachments/${ID}/content" alt="cat.png" loading="lazy"`));
  assert.doesNotMatch(html, new RegExp(`<img[^>]*${ID2}`));
  assert.match(html, /aria-label="Download report.pdf"/);
  assert.match(html, /PDF · 3.0 MB/);
  assert.match(html, /aria-label="2 attached files"/);
});

test('upload chips show progress, and failures with the reason', () => {
  const uploading = ui.chip({ key: 'k', name: 'b.pdf', size: 10, progress: 0.42, state: 'uploading' });
  assert.match(uploading, /role="progressbar"/);
  assert.match(uploading, /aria-valuenow="42"/);
  assert.match(uploading, /Uploading… 42%/);
  assert.match(uploading, /Cancel uploading b.pdf/);
  const failed = ui.chip({ key: 'k', name: 'c.exe', size: 10, progress: 0, state: 'error', error: 'Not supported' });
  assert.match(failed, /role="alert">Not supported/);
  assert.match(failed, /Retry c.exe/);
  const refused = ui.chip({ key: 'k', name: 'c.exe', size: 10, progress: 0, state: 'error', error: 'Not supported', retryable: false });
  assert.match(refused, /role="alert">Not supported/);
  assert.doesNotMatch(refused, /Retry/);
});
