// 0.8.0 drag and drop into chats (Pixel): the drag-preview helper (what a drag holds before the drop),
// the drop split (files vs folders), the announcement and the window guard that keeps a stray file
// drop from opening in place of kwant.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const d = await loadBundled(`export * from './src/dragPreview'; export { MAX_ATTACHMENTS } from './src/attachments';`);

const file = (type) => ({ kind: 'file', type });
const str = (type) => ({ kind: 'string', type });

test('one image: one file, one image, it fits', () => {
  const p = d.previewDrag([file('image/png')]);
  assert.equal(p.count, 1);
  assert.equal(p.kinds.image, 1);
  assert.deepEqual({ slotsLeft: p.slotsLeft, overBy: p.overBy, fits: p.fits }, { slotsLeft: 10, overBy: 0, fits: 1 });
  assert.equal(d.dragSummary(p), '1 file · 1 image');
  assert.equal(d.dragWarning(p), null);
});

test('three mixed files: counted by kind; a dragged link alongside is not a file', () => {
  const p = d.previewDrag([file('image/jpeg'), file('image/webp'), file('application/pdf'), str('text/uri-list')]);
  assert.equal(p.count, 3);
  assert.deepEqual(p.kinds, { image: 2, video: 0, pdf: 1, text: 0, refused: 0, unknown: 0 });
  assert.equal(d.dragSummary(p), '3 files · 2 images, 1 PDF');
  const q = d.previewDrag([file('video/mp4'), file('text/markdown'), file('application/json')]);
  assert.deepEqual([q.kinds.video, q.kinds.text], [1, 2]);
});

test('eleven files: one too many, and the tray already holding files leaves less room', () => {
  const eleven = Array.from({ length: 11 }, () => file('image/png'));
  const p = d.previewDrag(eleven);
  assert.equal(d.MAX_ATTACHMENTS, 10, 'the limit is unchanged');
  assert.deepEqual({ count: p.count, fits: p.fits, overBy: p.overBy }, { count: 11, fits: 10, overBy: 1 });
  assert.equal(d.dragWarning(p), 'Only 10 files fit: up to 10 per message.');
  const withEight = d.previewDrag([file('image/png'), file('image/png'), file('image/png')], 8);
  assert.deepEqual({ slotsLeft: withEight.slotsLeft, fits: withEight.fits, overBy: withEight.overBy }, { slotsLeft: 2, fits: 2, overBy: 1 });
  const full = d.previewDrag([file('image/png')], 10);
  assert.equal(d.dragWarning(full), 'This message already has 10 files.');
});

test('a ZIP is refused before the drop; Office, audio and HEIC too', () => {
  const p = d.previewDrag([file('application/zip')]);
  assert.equal(p.kinds.refused, 1);
  assert.match(d.dragSummary(p), /1 that can't be attached/);
  for (const t of ['application/x-zip-compressed', 'audio/mpeg', 'image/heic', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'])
    assert.equal(d.dragKind(t), 'refused', t);
});

test('a folder: unknown while dragging (no type), split out on drop with the folder line', () => {
  const p = d.previewDrag([file('')]);
  assert.equal(p.kinds.unknown, 1, 'a folder, a .ts file and an unknown format look alike before the drop');
  assert.equal(d.dragKind('video/mp2t'), 'unknown', '.ts files are often code, not video');
  assert.equal(d.dragSummary(p), '1 file');

  const png = { name: 'cat.png', size: 10, type: 'image/png' };
  const items = [
    { kind: 'file', type: '', getAsFile: () => ({ name: 'Photos', size: 0, type: '' }), webkitGetAsEntry: () => ({ isDirectory: true, name: 'Photos' }) },
    { kind: 'file', type: 'image/png', getAsFile: () => png, webkitGetAsEntry: () => ({ isDirectory: false, name: 'cat.png' }) },
    { kind: 'string', type: 'text/plain', getAsFile: () => null },
  ];
  const out = d.splitDrop(items, [png]);
  assert.deepEqual(out.folders, ['Photos']);
  assert.deepEqual(out.files, [png]);
  assert.equal(d.folderNote(out.folders), "Photos is a folder. Folders can't go: drop the files inside.");
  assert.equal(d.folderNote(['a', 'b']), "Folders can't go: drop the files inside.");
  // No items (an old engine): the file list is used as it is.
  assert.deepEqual(d.splitDrop(null, [png]).files, [png]);
});

test('the announcement after a drop', () => {
  assert.equal(d.dropAnnouncement(3), '3 files added');
  assert.equal(d.dropAnnouncement(1), '1 file added');
  assert.equal(d.dropAnnouncement(0), 'No files added');
});

const ev = (type, types, prevented = false) => {
  const e = { type, defaultPrevented: prevented, dataTransfer: { types, dropEffect: 'copy' }, prevented: 0 };
  e.preventDefault = () => { e.prevented += 1; e.defaultPrevented = true; };
  return e;
};

test('a file drop outside a chat is prevented and shows the no-drop cursor', () => {
  const over = ev('dragover', ['Files']);
  assert.equal(d.guardFileDrag(over), true);
  assert.equal(over.prevented, 1);
  assert.equal(over.dataTransfer.dropEffect, 'none');
  const drop = ev('drop', ['Files']);
  assert.equal(d.guardFileDrag(drop), true);
  assert.equal(drop.prevented, 1);
});

test('the guard leaves text, links and drops a target already took alone', () => {
  for (const types of [['text/plain'], ['text/uri-list', 'text/html'], []]) {
    const e = ev('drop', types);
    assert.equal(d.guardFileDrag(e), false, types.join());
    assert.equal(e.prevented, 0);
  }
  // The chat pane or the Import dialog called preventDefault: copy cursor stays, nothing more.
  const taken = ev('dragover', ['Files'], true);
  assert.equal(d.guardFileDrag(taken), false);
  assert.equal(taken.dataTransfer.dropEffect, 'copy');
});

test('the guard goes on the window in bubble phase and comes off', () => {
  const calls = [];
  const win = { addEventListener: (t, f, o) => calls.push(['add', t, o]), removeEventListener: (t) => calls.push(['remove', t]) };
  const off = d.installFileDropGuard(win);
  assert.deepEqual(calls, [['add', 'dragover', undefined], ['add', 'drop', undefined]]);
  off();
  assert.deepEqual(calls.slice(2), [['remove', 'dragover'], ['remove', 'drop']]);
  const main = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8');
  assert.match(main, /installFileDropGuard\(window\)/, 'the app installs it at start');
});

test('the Mac and Windows place window hands drops to the page', () => {
  const rs = readFileSync(new URL('../../apps/mac/src/main.rs', import.meta.url), 'utf8');
  const start = rs.indexOf('fn make_window<');
  const body = rs.slice(start, rs.indexOf('\n}\n', start));
  assert.match(body, /\.disable_drag_drop_handler\(\)/);
  assert.equal(rs.match(/disable_drag_drop_handler/g).length, 1, 'only the place window; connect and nudge keep Tauri\'s');
});
