// 0.7.7: pick several files in the importer. They go up as one STORED zip ("bots.zip") that the
// server's zip path reads into one preview. The writer, the checks before upload, and the dialog.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/importZip';
  export { ImportDialog, pickedLabel } from './src/components/ImportDialog';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = async (n = 6) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const bytes = (s) => new TextEncoder().encode(s);
const str = (b) => new TextDecoder().decode(b);

/** A small independent zip reader: walks the central directory, checks each local header and CRC. */
function readZip(buf) {
  const v = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const eocd = buf.length - 22;
  assert.equal(v.getUint32(eocd, true), 0x06054b50, 'end of central directory');
  const count = v.getUint16(eocd + 10, true);
  const cdSize = v.getUint32(eocd + 12, true);
  let p = v.getUint32(eocd + 16, true);
  assert.equal(p + cdSize, eocd, 'central directory ends where the end record starts');
  const out = [];
  for (let i = 0; i < count; i++) {
    assert.equal(v.getUint32(p, true), 0x02014b50, 'central header');
    const flags = v.getUint16(p + 8, true);
    const method = v.getUint16(p + 10, true);
    const crc = v.getUint32(p + 16, true);
    const csize = v.getUint32(p + 20, true);
    const usize = v.getUint32(p + 24, true);
    const nlen = v.getUint16(p + 28, true);
    const elen = v.getUint16(p + 30, true);
    const clen = v.getUint16(p + 32, true);
    const off = v.getUint32(p + 42, true);
    const name = str(buf.subarray(p + 46, p + 46 + nlen));
    assert.equal(v.getUint32(off, true), 0x04034b50, 'local header');
    assert.equal(v.getUint16(off + 8, true), method);
    assert.equal(v.getUint32(off + 14, true), crc);
    assert.equal(v.getUint32(off + 18, true), csize);
    const lnlen = v.getUint16(off + 26, true);
    const lelen = v.getUint16(off + 28, true);
    assert.equal(str(buf.subarray(off + 30, off + 30 + lnlen)), name, 'same name in both headers');
    const data = buf.subarray(off + 30 + lnlen + lelen, off + 30 + lnlen + lelen + csize);
    out.push({ name, flags, method, crc, csize, usize, data });
    p += 46 + nlen + elen + clen;
  }
  return out;
}

test('crc32 matches the standard check values', () => {
  assert.equal(m.crc32(bytes('123456789')), 0xcbf43926);
  assert.equal(m.crc32(new Uint8Array()), 0);
  assert.equal(m.crc32(bytes('The quick brown fox jumps over the lazy dog')), 0x414fa339);
});

test('storedZip: STORED entries, UTF-8 names, right CRCs, and the bytes read back', () => {
  const files = [
    { name: 'grok-a.json', data: bytes('{"name":"A"}') },
    { name: 'agent ü.md', data: bytes('---\nname: x\n---\nhi') },
    { name: 'empty.txt', data: new Uint8Array() },
  ];
  const zip = m.storedZip(files);
  const back = readZip(zip);
  assert.deepEqual(back.map((e) => e.name), files.map((f) => f.name));
  back.forEach((e, i) => {
    assert.equal(e.method, 0, 'STORED');
    assert.equal(e.flags & 0x0800, 0x0800, 'UTF-8 flag');
    assert.equal(e.csize, e.usize);
    assert.equal(e.crc, m.crc32(files[i].data));
    assert.deepEqual([...e.data], [...files[i].data]);
  });
  assert.equal(zip.length, files.reduce((n, f) => n + f.data.length, 0) + m.zipOverhead(files.map((f) => f.name)));
});

test('zipNames: base names only, duplicates get " (2)", a second SKILL.md gets its own folder', () => {
  assert.deepEqual(m.zipNames(['a/b/bot.json', 'C:\\x\\bot.json', 'bot.json', 'README', 'readme']), ['bot.json', 'bot (2).json', 'bot (3).json', 'README', 'readme (2)']);
  assert.deepEqual(m.zipNames(['one/SKILL.md', 'two/SKILL.md']), ['SKILL.md', 'skill-2/SKILL.md']);
});

test('pickProblem: one file is checked as before; several files: no archives, ≤500, ≤4 MB each, ≤20 MB in all', () => {
  const MB = 1024 * 1024;
  assert.equal(m.pickProblem([]), null);
  assert.equal(m.pickProblem([{ name: 'big.zip', size: 19 * MB }]), null, 'one file goes the old way');
  assert.equal(m.pickProblem([{ name: 'a.json', size: 10 }, { name: 'b.md', size: 10 }]), null);
  assert.equal(m.pickProblem([{ name: 'a.json', size: 10 }, { name: 'more.ZIP', size: 10 }]), 'Pick one zip, or several files that aren’t zips.');
  assert.equal(m.pickProblem([{ name: 'a.json', size: 10 }, { name: 'x.tar.gz', size: 10 }]), 'Pick one zip, or several files that aren’t zips.');
  assert.match(m.pickProblem(Array.from({ length: 501 }, (_, i) => ({ name: `${i}.json`, size: 1 }))), /^That’s 501 files\. Pick up to 500 at once\.$/);
  assert.equal(m.pickProblem(Array.from({ length: 500 }, (_, i) => ({ name: `${i}.json`, size: 1 }))), null);
  assert.equal(m.pickProblem([{ name: 'a.json', size: 10 }, { name: 'huge.png', size: 4 * MB + 1 }]), 'huge.png is over 4 MB, so it can’t be imported.');
  assert.equal(m.pickProblem(Array.from({ length: 6 }, (_, i) => ({ name: `${i}.png`, size: 3.5 * MB }))), 'These files add up to more than 20 MB. Pick fewer at a time.');
});

test('uploadFor: one file goes up unchanged; several become one bots.zip', async () => {
  const one = new File(['{"a":1}'], 'grok.json');
  const up1 = await m.uploadFor([one]);
  assert.equal(up1.body, one);
  assert.equal(up1.filename, 'grok.json');
  const up3 = await m.uploadFor([one, new File(['{"b":2}'], 'grok.json'), new File(['---\nname: r\n---\n'], 'reviewer.md')]);
  assert.equal(up3.filename, 'bots.zip');
  const back = readZip(new Uint8Array(await up3.body.arrayBuffer()));
  assert.deepEqual(back.map((e) => [e.name, str(e.data)]), [['grok.json', '{"a":1}'], ['grok (2).json', '{"b":2}'], ['reviewer.md', '---\nname: r\n---\n']]);
});

test('pickedLabel names one file, or counts several', () => {
  assert.equal(m.pickedLabel([{ name: 'a.json' }]), 'a.json');
  assert.equal(m.pickedLabel([{ name: 'a.json' }, { name: 'b.md' }]), '2 files: a.json and b.md');
  assert.equal(m.pickedLabel([{ name: 'a' }, { name: 'b' }, { name: 'c' }]), '3 files: a, b and c');
  assert.equal(m.pickedLabel([{ name: 'a' }, { name: 'b' }, { name: 'c' }, { name: 'd' }, { name: 'e' }]), '5 files: a, b, c and 2 more');
});

// ------------------------------------------------------------------ the dialog

function dialog() {
  const calls = [];
  const preview = { bundle: { format: 'kwant.bundle/1', source: { kind: 'zip' }, bots: [], teams: [], skills: [], omitted: [] }, warnings: [], dropped: [], problems: [], renames: [], can_apply: false };
  g.fetch = g.window.fetch = async (url, init = {}) => {
    calls.push({ url: String(url), method: init.method ?? 'GET', body: init.body });
    const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
    if (String(url) === '/api/import/prompts') return json([]);
    if (String(url).startsWith('/api/import/preview')) return json(preview);
    return new Response('{}', { status: 404 });
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() => root.render(h(m.ImportDialog, { spaceName: 'Home', onClose() {}, onChanged() {} })));
  const btn = (re) => [...document.querySelectorAll('[role="dialog"] button')].find((b) => re.test(b.textContent));
  const pick = (files) => {
    const input = document.querySelector('[role="dialog"] input[type="file"]');
    Object.defineProperty(input, 'files', { value: files, configurable: true });
    input.dispatchEvent(new window.Event('change', { bubbles: true }));
  };
  return { calls, host, btn, pick, done: () => (root.unmount(), host.remove()) };
}

test('dialog: the picker takes several files; one file uploads exactly as before', async () => {
  const d = dialog();
  await settle();
  d.btn(/Upload a file/).click();
  await settle();
  const input = document.querySelector('[role="dialog"] input[type="file"]');
  assert.equal(input.multiple, true);
  assert.match(document.querySelector('[role="dialog"]').textContent, /Several of these at once/);
  const f = new File(['{"name":"A"}'], 'grok a.json');
  d.pick([f]);
  await settle();
  assert.equal(document.querySelector('[data-picked]').textContent, 'grok a.json');
  d.btn(/Preview/).click();
  await settle();
  const up = d.calls.find((c) => c.url.startsWith('/api/import/preview'));
  assert.equal(up.url, '/api/import/preview?filename=grok%20a.json');
  assert.equal(up.body, f, 'the file itself, not a zip');
  d.done();
});

test('dialog: three files go up as one bots.zip with all three inside', async () => {
  const d = dialog();
  await settle();
  d.btn(/Upload a file/).click();
  await settle();
  d.pick([new File(['{"a":1}'], 'a.json'), new File(['{"b":1}'], 'b.json'), new File(['---\nname: r\n---\nx'], 'reviewer.md')]);
  await settle();
  assert.equal(document.querySelector('[data-picked]').textContent, '3 files: a.json, b.json and reviewer.md');
  d.btn(/Preview/).click();
  await settle(12);
  const up = d.calls.find((c) => c.url.startsWith('/api/import/preview'));
  assert.equal(up.url, '/api/import/preview?filename=bots.zip');
  const back = readZip(new Uint8Array(await up.body.arrayBuffer()));
  assert.deepEqual(back.map((e) => e.name), ['a.json', 'b.json', 'reviewer.md']);
  d.done();
});

test('dialog: a zip among several files is refused before anything is sent', async () => {
  const d = dialog();
  await settle();
  d.btn(/Upload a file/).click();
  await settle();
  d.pick([new File(['x'], 'a.json'), new File(['PK'], 'more.zip')]);
  await settle();
  assert.equal(document.querySelector('[role="dialog"] [role="alert"]').textContent, 'Pick one zip, or several files that aren’t zips.');
  assert.equal(d.btn(/Preview/).disabled, true);
  assert.equal(d.calls.some((c) => c.url.startsWith('/api/import/preview')), false);
  d.done();
});

test('preview of several picked files says "from 3 files" and "Files that weren\'t read", not "a zip"', async () => {
  const { renderToStaticMarkup } = await loadBundled(`export { renderToStaticMarkup } from 'react-dom/server.browser';`);
  const pv = await loadBundled(`export { PreviewView } from './src/components/ImportDialog'; export { createElement } from 'react';`);
  const bot = (ref) => ({ ref, name: ref, title: '', persona_prompt: 'x', memories: [], skills: [], routines: [] });
  const preview = { bundle: { format: 'kwant.bundle/1', source: { kind: 'zip' }, bots: [bot('a'), bot('b'), bot('c')], teams: [], skills: [], omitted: ["Files in the zip that weren't read: notes.txt"] }, warnings: [], dropped: [], problems: [], renames: [], can_apply: true };
  const out = (n) => renderToStaticMarkup(pv.createElement(pv.PreviewView, { preview, skip: new Set(), onToggle() {}, pickedFiles: n })).replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'");
  assert.match(out(3), /3 bots will be created from 3 files\./);
  assert.match(out(3), /Files that weren't read: notes\.txt/);
  assert.match(out(0), /Files in the zip that weren't read: notes\.txt/, 'an uploaded zip keeps the server wording');
});
