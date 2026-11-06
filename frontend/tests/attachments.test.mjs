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
  assert.match(a.preflightFile({ name: 'big.pdf', size: 20 * MB + 1 }), /big\.pdf is too big \(20\.0 MB\)\. Max 20 MB/);
  assert.match(a.preflightFile({ name: 'e.txt', size: 0 }), /empty/);
});

test('server refusals read as plain language', () => {
  assert.match(a.uploadErrorText(413), /Max 20 MB, videos 100 MB/);
  assert.match(a.uploadErrorText(415), /isn't supported/);
  assert.equal(a.uploadErrorText(400, 'bad name'), 'bad name');
  assert.match(a.uploadErrorText(0), /interrupted/);
  assert.match(a.uploadErrorText(500), /HTTP 500/);
  // Retry can't help when the server refused the file itself
  for (const st of [400, 413, 415]) assert.equal(a.uploadRetryable(st), false, `HTTP ${st}`);
  for (const st of [0, 401, 429, 500, 503]) assert.equal(a.uploadRetryable(st), true, `HTTP ${st}`);
  assert.match(a.uploadErrorText(415), /videos \(MP4, MOV, WebM\)/);
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
  assert.match(uploading, /4 of 10 B|0 of 1 KB/);
  assert.match(uploading, /Cancel uploading b.pdf/);
  const failed = ui.chip({ key: 'k', name: 'c.exe', size: 10, progress: 0, state: 'error', error: 'Not supported' });
  assert.match(failed, /role="alert">Not supported/);
  assert.match(failed, /Retry c.exe/);
  const refused = ui.chip({ key: 'k', name: 'c.exe', size: 10, progress: 0, state: 'error', error: 'Not supported', retryable: false });
  assert.match(refused, /role="alert">Not supported/);
  assert.doesNotMatch(refused, /Retry/);
});


// --- 0.7.3 video drop: P1 type table, P2 player and fallback, P3 auto-retry, P4 MB progress ---

test('P1: supported types pass, everything else is refused before upload', () => {
  const ok = [
    ['clip.mp4', 'video/mp4'], ['clip.MOV', 'video/quicktime'], ['clip.webm', 'video/webm'], ['clip.m4v', ''],
    ['phone.mov', ''], ['noext', 'video/mp4'],
    ['cat.png', 'image/png'], ['a.JPG', ''], ['b.gif', 'image/gif'], ['c.webp', 'image/webp'],
    ['doc.pdf', 'application/pdf'], ['doc', 'application/pdf'],
    ['notes.txt', 'text/plain'], ['r.md', ''], ['d.csv', 'text/csv'], ['x.json', 'application/json'],
    ['main.ts', 'video/mp2t'], ['App.tsx', ''], ['lib.rs', ''], ['run.py', 'text/x-python'], ['Makefile', ''], ['pasted', ''],
  ];
  for (const [name, type] of ok) assert.equal(a.preflightFile({ name, type, size: 1000 }), null, `${name} (${type || 'no type'})`);
  const refused = [
    ['a.zip', 'application/zip'], ['setup.exe', 'application/x-msdownload'], ['r.docx', ''], ['photo.heic', 'image/heic'],
    ['song.mp3', 'audio/mpeg'], ['film.mkv', 'video/x-matroska'], ['clip.avi', 'video/x-msvideo'], ['x.bin', 'application/octet-stream'],
    ['noext', 'application/zip'],
  ];
  for (const [name, type] of refused) {
    const why = a.preflightFile({ name, type, size: 1000 });
    assert.ok(why, `${name} should be refused`);
    assert.match(why, /can't be attached\. Images, videos \(MP4, MOV, WebM\), PDFs and text or code files work\./);
  }
  assert.equal(a.attachKind({ name: 'main.ts', type: 'video/mp2t' }), 'text');
  assert.equal(a.attachKind({ name: 'v.mov', type: 'video/quicktime' }), 'video');
});

test('P1: videos may be 100 MB, everything else 20 MB', () => {
  assert.equal(a.preflightFile({ name: 'v.mp4', type: 'video/mp4', size: 100 * MB }), null);
  assert.match(a.preflightFile({ name: 'v.mp4', type: 'video/mp4', size: 120 * MB }), /v\.mp4 is too big \(120\.0 MB\)\. Videos can be up to 100 MB\./);
  assert.match(a.preflightFile({ name: 'p.png', type: 'image/png', size: 21 * MB }), /Max 20 MB \(videos 100 MB\)/);
  assert.match(a.preflightFile({ name: 'v.webm', type: 'video/webm', size: 0 }), /empty/);
  // the type is checked first: a huge zip says the type, not the size
  assert.match(a.preflightFile({ name: 'big.zip', type: 'application/zip', size: 500 * MB }), /can't be attached/);
});

test('P4: chips show uploaded MB of total', () => {
  assert.equal(a.uploadProgressText(0.65, 8 * MB), '5.2 of 8.0 MB');
  assert.equal(a.uploadProgressText(0, 8 * MB), '0.0 of 8.0 MB');
  assert.equal(a.uploadProgressText(1.5, 8 * MB), '8.0 of 8.0 MB');
  assert.equal(a.uploadProgressText(0.5, 300 * 1024), '150 of 300 KB');
  const chip = ui.chip({ key: 'k', name: 'clip.mp4', mime: 'video/mp4', size: 8 * MB, progress: 0.65, state: 'uploading' });
  assert.match(chip, /5\.2 of 8\.0 MB/);
  assert.match(chip, /aria-valuetext="5\.2 of 8\.0 MB"/);
  const retrying = ui.chip({ key: 'k', name: 'clip.mp4', mime: 'video/mp4', size: 8 * MB, progress: 0, state: 'uploading', retrying: true });
  assert.match(retrying, /Retrying…/);
});

test('P2: a sent video is a player with no autoplay; other files keep their card', () => {
  const html = ui.tiles([
    { id: ID, name: 'clip.mp4', mime: 'video/mp4', size: 8 * MB },
    { id: ID2, name: 'report.pdf', mime: 'application/pdf', size: MB },
  ]);
  assert.match(html, new RegExp(`<video src="/api/attachments/${ID}/content" controls="" preload="metadata" playsinline=""`));
  assert.doesNotMatch(html, /autoplay/i);
  assert.match(html, /aria-label="Download clip.mp4"/);
  assert.match(html, /MP4 · 8\.0 MB/);
  assert.doesNotMatch(html, new RegExp(`<video[^>]*${ID2}`));
  assert.equal(a.isVideo('video/quicktime'), true);
  assert.equal(a.isVideo('video/x-matroska'), false);
  // composer chip: a film icon for a finished video, a file icon otherwise
  const done = ui.chip({ key: 'k', name: 'clip.mov', size: 8 * MB, progress: 1, state: 'done' });
  assert.match(done, /lucide-film/);
  assert.doesNotMatch(ui.chip({ key: 'k', name: 'a.pdf', size: 1, progress: 1, state: 'done' }), /lucide-film/);
});

test('P2: a video the browser cannot play falls back to the file card', async () => {
  const { JSDOM } = await import('jsdom');
  const dom = new JSDOM('<!doctype html><div id="root"></div>', { pretendToBeVisual: true });
  const g = globalThis;
  const saved = { window: g.window, document: g.document };
  g.window = dom.window;
  g.document = dom.window.document;
  try {
    const live = await loadBundled(`
      import React from 'react';
      import { createRoot } from 'react-dom/client';
      import { FileTiles } from './src/components/Attachments';
      export function mount(el, files) { const r = createRoot(el); r.render(React.createElement(FileTiles, { files })); return r; }
    `);
    const el = dom.window.document.getElementById('root');
    const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };
    const root = live.mount(el, [{ id: ID, name: 'phone.mov', mime: 'video/quicktime', size: 8 * MB }]);
    await settle();
    const video = el.querySelector('video');
    assert.ok(video, 'starts as a player');
    video.dispatchEvent(new dom.window.Event('error'));
    await settle();
    assert.equal(el.querySelector('video'), null, 'player gone after a decode error');
    assert.match(el.textContent, /phone\.mov/);
    assert.match(el.textContent, /MOV · 8\.0 MB/);
    assert.ok(el.querySelector('a[aria-label="Download phone.mov"]'), 'download stays');
    root.unmount();
  } finally {
    g.window = saved.window;
    g.document = saved.document;
  }
});

// A fake XMLHttpRequest that answers each request from a script: a status, or 'drop' for a lost connection.
function installFakeXhr(script) {
  const sent = [];
  class FakeXHR {
    constructor() { this.upload = {}; this.status = 0; this.responseText = ''; }
    open(method, url) { this.method = method; this.url = url; }
    abort() { this.onabort?.(); }
    send(body) {
      sent.push({ url: this.url, size: body?.size });
      const step = script.shift() ?? 201;
      queueMicrotask(() => {
        this.upload.onprogress?.({ lengthComputable: true, loaded: 4, total: 8 });
        if (step === 'drop') return this.onerror?.();
        this.status = step;
        const code = { 413: 'too_large', 415: 'unsupported_type', 400: 'empty' }[step];
        this.responseText = step === 201 ? JSON.stringify({ id: ID, name: 'clip.mp4', mime: 'video/mp4', size: 8 })
          : step === 'interrupted' ? JSON.stringify({ error: 'The upload was interrupted.', code: 'interrupted' })
          : JSON.stringify({ error: `HTTP ${step}`, code });
        if (step === 'interrupted') this.status = 400;
        this.onload?.();
      });
    }
  }
  const prev = globalThis.XMLHttpRequest;
  globalThis.XMLHttpRequest = FakeXHR;
  return { sent, restore: () => { globalThis.XMLHttpRequest = prev; } };
}

async function runUpload(script) {
  const fake = installFakeXhr(script);
  const waits = [];
  const retries = [];
  const progress = [];
  try {
    const file = Object.assign(new Blob([new Uint8Array(8)]), { name: 'clip.mp4' });
    const result = await a
      .uploadWithRetry(file, (f) => progress.push(f), undefined, { onRetrying: (n) => retries.push(n), wait: async (ms) => { waits.push(ms); } })
      .then((v) => ({ ok: v }), (e) => ({ err: e }));
    return { ...result, waits, retries, progress, sent: fake.sent.length };
  } finally {
    fake.restore();
  }
}

test('P3: a dropped connection or 502/503/504 retries by itself, after 1 s then 3 s', async () => {
  for (const blip of ['drop', 502, 503, 504, 'interrupted']) {
    const r = await runUpload([blip, 201]);
    assert.equal(r.ok?.id, ID, `${blip} then success`);
    assert.deepEqual(r.waits, [1000], `${blip}: one wait of 1 s`);
    assert.deepEqual(r.retries, [1]);
    assert.equal(r.sent, 2);
  }
  const twice = await runUpload(['drop', 503, 201]);
  assert.equal(twice.ok?.id, ID);
  assert.deepEqual(twice.waits, [1000, 3000]);
  assert.deepEqual(twice.retries, [1, 2]);
  assert.equal(twice.sent, 3);
});

test('P3: after two automatic retries it stops and shows the manual Retry', async () => {
  const r = await runUpload(['drop', 'drop', 'drop', 201]);
  assert.equal(r.sent, 3, 'one try plus two retries');
  assert.deepEqual(r.waits, [1000, 3000]);
  assert.equal(r.err?.status, 0);
  assert.equal(a.uploadRetryable(r.err.status), true, 'manual Retry still offered');
});

test('P3: 413, 415 and 400 never retry; neither do 500 or 401', async () => {
  for (const st of [413, 415, 400, 500, 401]) {
    const r = await runUpload([st, 201]);
    assert.equal(r.err?.status, st, `HTTP ${st} fails`);
    if (st === 413) assert.equal(r.err.code, 'too_large');
    assert.equal(r.sent, 1, `HTTP ${st}: no second request`);
    assert.deepEqual(r.waits, []);
  }
  for (const st of [0, 502, 503, 504]) assert.equal(a.uploadAutoRetryable(st), true);
  for (const st of [400, 401, 413, 415, 429, 500]) assert.equal(a.uploadAutoRetryable(st), false);
  assert.equal(a.uploadAutoRetryable(400, 'interrupted'), true);
  assert.equal(a.uploadRetryable(400, 'interrupted'), true);
  assert.equal(a.uploadRetryable(400, 'empty'), false);
  // api.md's 413 text is shown as the server wrote it
  assert.equal(a.uploadErrorText(413, 'Videos can be at most 100 MB.'), 'Videos can be at most 100 MB.');
});

test('P3: cancelling while it waits to retry stops it', async () => {
  const fake = installFakeXhr(['drop', 201]);
  try {
    const ctrl = new AbortController();
    const file = Object.assign(new Blob([new Uint8Array(8)]), { name: 'clip.mp4' });
    const p = a.uploadWithRetry(file, () => {}, ctrl.signal, { delays: [50, 50], onRetrying: () => ctrl.abort() });
    await assert.rejects(p, (e) => e.name === 'AbortError');
    assert.equal(fake.sent.length, 1);
  } finally {
    fake.restore();
  }
});
