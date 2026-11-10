// 0.8.0 drag and drop into chats (Pixel): the drag-preview helper (what a drag holds before the drop),
// the drop split (files vs folders), the announcement and the window guard that keeps a stray file
// drop from opening in place of kwant.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const d = await loadBundled(`export * from './src/dragPreview'; export { MAX_ATTACHMENTS, attachKind, previewRefusesMime } from './src/attachments';`);

const file = (type) => ({ kind: 'file', type });
const str = (type) => ({ kind: 'string', type });

const bot = { kind: 'bot', name: 'Frontend' };
const team = { kind: 'team', name: 'Kitchen' };
const card = (items, inTray = 0, extra = {}) => d.dropCard({ preview: d.previewDrag(items, inTray), recipient: bot, ...extra });

test('one image: one file goes, the card gives it to the bot', () => {
  const p = d.previewDrag([file('image/png')]);
  assert.deepEqual({ count: p.count, go: p.go, overBy: p.overBy, slotsLeft: p.slotsLeft, refused: p.refused }, { count: 1, go: 1, overBy: 0, slotsLeft: 10, refused: [] });
  assert.equal(d.whatLine(p), '1 file · 1 image');
  const c = card([file('image/png')]);
  assert.equal(c.state, 'over');
  assert.equal(d.headingText(c.heading), 'Drop to give Frontend this file');
  assert.equal(d.headingText(c.armedHeading), 'Drop here to give Frontend this file');
  assert.deepEqual(c.wont, []);
  assert.equal(c.announce, 'Drop to give Frontend 1 file: 1 image.');
});

test('three mixed files: counted by kind; a dragged link alongside is not a file; no items hides the count', () => {
  const p = d.previewDrag([file('image/jpeg'), file('image/webp'), file('application/pdf'), str('text/uri-list')]);
  assert.equal(p.count, 3);
  assert.deepEqual(p.kinds, { image: 2, video: 0, pdf: 1, text: 0, file: 0 });
  assert.equal(d.whatLine(p), '3 files · 2 images, 1 PDF');
  const q = d.previewDrag([file('video/mp4'), file('text/markdown'), file('application/json'), file('')]);
  assert.deepEqual([q.kinds.video, q.kinds.text, q.kinds.file], [1, 2, 1]);
  // Five kinds: three shown, the rest fold into "other".
  const r = d.previewDrag([file('image/png'), file('image/png'), file('video/mp4'), file('video/mp4'), file('application/pdf'), file('text/plain'), file('')]);
  assert.deepEqual(d.kindParts(r).map((k) => k.label), ['2 images', '2 videos', '1 PDF', '2 other']);
  // A browser that shows no items before the drop: no count, no badge, the heading still says who.
  const none = d.previewDrag([]);
  assert.equal(none.count, null);
  const c = d.dropCard({ preview: none, recipient: bot });
  assert.equal(c.count, null);
  assert.equal(d.headingText(c.heading), 'Drop to give Frontend your files');
  assert.equal(c.announce, 'Drop to give Frontend your files.');
});

test('headings for every recipient', () => {
  const h = (r, n = 2, armed = false) => d.headingText(d.dropHeading(r, n, armed));
  assert.equal(h(bot), 'Drop to give Frontend these files');
  assert.equal(h({ kind: 'general', name: 'General' }), 'Drop to share in General');
  assert.equal(h(team), 'Drop to share with the Kitchen team');
  assert.equal(h({ kind: 'channel', name: '#launch' }, 2, true), 'Drop here to share with #launch');
  const parts = d.dropHeading(team, 1);
  assert.deepEqual(parts, { pre: 'Drop to share with the ', name: 'Kitchen', post: ' team' }, 'the name is drawn apart');
  const c = d.dropCard({ preview: d.previewDrag([file('image/png')]), recipient: team });
  assert.equal(c.announce, 'Drop to share 1 file with the Kitchen team: 1 image.');
});

test('some won\'t go: over the limit, with the tray already holding files, and refused files beside good ones', () => {
  assert.equal(d.MAX_ATTACHMENTS, 10, 'the limit is unchanged');
  const eleven = Array.from({ length: 11 }, () => file('image/png'));
  const p = d.previewDrag(eleven);
  assert.deepEqual({ go: p.go, overBy: p.overBy }, { go: 10, overBy: 1 });
  assert.deepEqual(card(eleven).wont, ["1 more won't fit: 10 files per message."]);
  const c = card([file('image/png'), file('image/png'), file('image/png')], 8);
  assert.deepEqual({ go: c.count, wont: c.wont }, { go: 2, wont: ["1 more won't fit: 10 files per message, 8 are already in."] });
  assert.equal(c.announce, "Drop to give Frontend 2 files: 2 images. 1 more won't fit.");
  const one = card(Array.from({ length: 10 }, () => file('image/png')), 1);
  assert.match(one.wont[0], /1 is already in\.$/);
  const mixed = card([file('image/png'), file('application/zip'), file('application/zip')]);
  assert.equal(mixed.state, 'over');
  assert.deepEqual(mixed.wont, ["2 ZIPs can't go. Images, videos, PDFs and text or code files work."]);
  const odd = card([file('image/png'), file('application/zip'), file('audio/mpeg')]);
  assert.equal(odd.wont[0], "2 files can't go. Images, videos, PDFs and text or code files work.");
  // A refused file still takes a slot, as addFiles does.
  const slot = d.previewDrag([file('application/zip'), file('image/png')], 9);
  assert.deepEqual({ go: slot.go, overBy: slot.overBy, refused: slot.refused.length }, { go: 0, overBy: 1, refused: 1 });
});

test('all refused: the neutral card, in the design\'s order', () => {
  const zip = card([file('application/zip')]);
  assert.equal(zip.state, 'refused');
  assert.equal(zip.count, null);
  assert.equal(d.headingText(zip.heading), "This can't go to Frontend");
  assert.equal(zip.refusal.line1, "ZIP files can't be attached.");
  assert.equal(zip.refusal.line2, 'Images, videos, PDFs and text or code files work. Unzip it and drop the files inside.');
  assert.equal(zip.announce, "This can't go to Frontend. ZIP files can't be attached.");
  const two = d.dropCard({ preview: d.previewDrag([file('audio/mpeg'), file('application/msword')]), recipient: team });
  assert.equal(d.headingText(two.heading), "These can't go to the Kitchen team");
  assert.equal(two.refusal.line1, "These files can't be attached.");
  assert.equal(two.refusal.line2, 'Images, videos, PDFs and text or code files work.');
  const office = card([file('application/vnd.openxmlformats-officedocument.wordprocessingml.document')]);
  assert.equal(office.refusal.line1, "Office files can't be attached.");
  // Idea mode first, then a disabled composer, then a full tray.
  const idea = card([file('application/zip')], 10, { ideaMode: true, disabledReason: 'Sign in first.' });
  assert.equal(d.headingText(idea.heading), "Files can't go in the idea jar");
  assert.equal(idea.refusal.line2, 'Clear /idea from the field to send them to Frontend.');
  const off = card([file('image/png')], 10, { disabledReason: 'Sign in first.' });
  assert.deepEqual([d.headingText(off.heading), off.refusal.line1], ["Can't drop files here right now", 'Sign in first.']);
  const full = card([file('image/png')], 10);
  assert.deepEqual([d.headingText(full.heading), full.refusal.line1, full.refusal.line2], ['The tray is full', '10 files per message.', 'Send these first, then drop the rest.']);
});

test('the refusal list before the drop matches what the tray refuses after it, and never refuses .ts or SVG', () => {
  const refused = ['application/zip', 'application/x-zip-compressed', 'application/x-7z-compressed', 'audio/mpeg', 'audio/wav', 'font/woff2', 'image/heic', 'image/heif', 'image/avif', 'image/tiff', 'image/bmp',
    'image/vnd.adobe.photoshop', 'video/x-matroska', 'video/x-msvideo', 'video/mpeg', 'application/msword', 'application/vnd.ms-excel',
    'application/vnd.openxmlformats-officedocument.wordprocessingml.document', 'application/vnd.oasis.opendocument.text'];
  for (const t of refused) {
    assert.equal(d.previewRefusesMime(t), true, t);
    assert.equal(d.dragKind(t), 'refused', t);
    assert.equal(d.attachKind({ name: '', type: t }), null, `${t}: refused by the tray with no name`);
    assert.equal(d.attachKind({ name: 'x.xyz', type: t }), null, `${t}: refused by the tray with an unknown extension`);
  }
  for (const t of ['video/mp2t', 'image/svg+xml', '', 'application/octet-stream', 'application/x-made-up'])
    assert.equal(d.previewRefusesMime(t), false, `${t || '(empty)'} is never refused before the drop`);
  assert.equal(d.dragKind('video/mp2t'), 'file', '.ts files are often code, not video');
});

test('a folder: a plain file while dragging, split out on drop with the folder line', () => {
  const png = { name: 'cat.png', size: 10, type: 'image/png' };
  const items = [
    { kind: 'file', type: '', getAsFile: () => ({ name: 'Photos', size: 0, type: '' }), webkitGetAsEntry: () => ({ isDirectory: true, name: 'Photos' }) },
    { kind: 'file', type: 'image/png', getAsFile: () => png, webkitGetAsEntry: () => ({ isDirectory: false, name: 'cat.png' }) },
    { kind: 'string', type: 'text/plain', getAsFile: () => null },
  ];
  const out = d.splitDrop(items, [png]);
  assert.deepEqual(out.folders, ['Photos']);
  assert.deepEqual(out.files, [png]);
  assert.equal(d.FOLDER_NOTE, "Folders can't go: drop the files inside.");
  assert.deepEqual(d.splitDrop(null, [png]).files, [png], 'no items (an old engine): the file list as it is');
});

test('the announcement after a drop, and the pill', () => {
  assert.equal(d.dropAnnouncement(3, bot), '3 files added for Frontend. Uploading.');
  assert.equal(d.dropAnnouncement(1, team), '1 file added for the Kitchen team. Uploading.');
  assert.equal(d.dropAnnouncement(0, bot), 'No files added.');
  assert.equal(d.PILL_TEXT, 'Open a chat to drop files there');
});

// A fake window: listeners by type, timers run by hand.
const fakeWin = () => {
  const on = {};
  const timers = new Map();
  let id = 0;
  return {
    on,
    timers,
    addEventListener: (t, f, capture) => ((on[t] ??= []).push(f), assert.equal(capture, true, 'capture phase')),
    removeEventListener: (t, f) => (on[t] = (on[t] ?? []).filter((g) => g !== f)),
    setTimeout: (f, ms) => (timers.set(++id, { f, ms }), id),
    clearTimeout: (t) => timers.delete(t),
    fire: (t, e) => (on[t] ?? []).forEach((f) => f(e)),
    runTimers: () => [...timers.entries()].forEach(([k, { f }]) => (timers.delete(k), f())),
    // Ends the current task: runs the 0 ms timers only (the "a dragenter came in this task" flag).
    nextTask: () => [...timers.entries()].filter(([, t]) => t.ms === 0).forEach(([k, { f }]) => (timers.delete(k), f())),
    waitingMs: () => [...timers.values()].map((t) => t.ms).filter((ms) => ms > 0),
  };
};
const dragE = (types = ['Files'], items = [file('image/png')], relatedTarget = {}) => ({ dataTransfer: { types, items }, relatedTarget });
// A dragleave out of the window: no relatedTarget.
const leaveWindow = () => ({ dataTransfer: { types: ['Files'], items: [file('image/png')] }, relatedTarget: null });

test('the window watch: depth for enter and leave, the 1500 ms fallback, Esc, a late dragover, open chats', () => {
  const w = fakeWin();
  const watch = d.createDragWatch(w);
  assert.equal(Object.keys(w.on).length, 0, 'nothing on the window until someone listens');
  let seen = 0;
  const off = watch.subscribe(() => seen++);
  assert.deepEqual(Object.keys(w.on).sort(), ['dragend', 'dragenter', 'dragleave', 'dragover', 'drop', 'keydown', 'mousemove', 'pointermove']);
  w.fire('dragenter', dragE(['text/plain'], []));
  assert.equal(watch.get().active, false, 'text drags are not watched');
  w.fire('dragenter', dragE());
  assert.equal(watch.get().active, true);
  assert.equal(watch.get().items.length, 1);
  w.fire('dragenter', dragE());
  w.fire('dragleave', dragE());
  assert.equal(watch.get().active, true, 'moving onto a child keeps it on');
  w.fire('dragleave', dragE());
  assert.equal(watch.get().active, false, 'depth 0: left the window');
  // The fallback: armed at 1500 ms on every dragover; firing it ends the drag.
  w.fire('dragenter', dragE());
  w.nextTask();
  w.fire('dragover', dragE());
  assert.deepEqual(w.waitingMs(), [1500], 'one fallback timer, re-armed');
  w.runTimers();
  assert.equal(watch.get().active, false, 'no dragover for 1500 ms: over');
  w.fire('dragover', dragE());
  assert.equal(watch.get().active, true, 'a later dragover starts it again');
  w.fire('keydown', { key: 'a' });
  assert.equal(watch.get().active, true);
  w.fire('keydown', { key: 'Escape' });
  assert.equal(watch.get().active, false, 'Esc');
  w.fire('dragenter', dragE());
  w.fire('drop', dragE());
  assert.equal(watch.get().active, false, 'drop');
  const a = watch.registerChat();
  const b = watch.registerChat();
  assert.equal(watch.get().chats, 2);
  a();
  b();
  assert.equal(watch.get().chats, 0);
  assert.ok(seen > 0);
  off();
  assert.equal(Object.values(w.on).flat().length, 0, 'the last one out takes the listeners off');
});

// #478 M2: WebKit (and the Mac app's WKWebView) sends dragover mostly when the pointer moves.
test('a hand held still keeps the drag: no dragover for 1 s with depth > 0 stays active', () => {
  const w = fakeWin();
  const watch = d.createDragWatch(w);
  const off = watch.subscribe(() => {});
  w.fire('dragenter', dragE());
  w.nextTask();
  w.fire('dragover', dragE());
  // 1 s of nothing: only timers shorter than 1 s may fire.
  [...w.timers.entries()].filter(([, t]) => t.ms <= 1000).forEach(([k, { f }]) => (w.timers.delete(k), f()));
  assert.equal(watch.get().active, true, 'still over the chat after 1 s with no dragover');
  assert.deepEqual(w.waitingMs(), [1500], 'only the 1500 ms fallback is waiting');
  off();
});

test('the first mousemove or pointermove ends the drag', () => {
  for (const type of ['mousemove', 'pointermove']) {
    const w = fakeWin();
    const watch = d.createDragWatch(w);
    const off = watch.subscribe(() => {});
    w.fire(type, {});
    assert.equal(watch.get().active, false, 'nothing to end');
    w.fire('dragenter', dragE());
    w.fire('dragenter', dragE());
    assert.equal(watch.get().active, true);
    w.fire(type, {});
    assert.equal(watch.get().active, false, `${type}: the OS drag is over`);
    w.fire('dragover', dragE());
    w.fire('dragleave', dragE());
    assert.equal(watch.get().active, false, 'the depth started again from 0, not from the old 2');
    off();
  }
});

test('a dragleave with no relatedTarget ends the drag; a crossing in the same task does not', () => {
  const w = fakeWin();
  const watch = d.createDragWatch(w);
  const off = watch.subscribe(() => {});
  w.fire('dragenter', dragE());
  w.fire('dragenter', dragE());
  w.fire('dragenter', dragE());
  w.nextTask();
  // WebKit crossing from one element to the next: the new dragenter, then the old dragleave with no relatedTarget.
  w.fire('dragenter', dragE());
  w.fire('dragleave', leaveWindow());
  assert.equal(watch.get().active, true, 'a crossing, not an exit');
  w.nextTask();
  w.fire('dragleave', leaveWindow());
  assert.equal(watch.get().active, false, 'left the window with depth still 2');
  off();
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

// #478 polish (Mira's ui-check-478 round 2 and Rigel's qa-478).
const css = readFileSync(new URL('../src/styles/dropLayer.css', import.meta.url), 'utf8');
const rule = (sel, from = css) => {
  const at = from.indexOf(sel + ' {');
  assert.ok(at >= 0, sel);
  return from.slice(at, from.indexOf('}', at));
};

test('the armed card is .94 opaque, so chat text does not show through (#478 L5)', () => {
  assert.match(rule(".kw-dd[data-phase='armed'] .kw-dd-card"), /opacity: 0\.94;/);
});

test('the limits line balances its wrap, so "100 MB" never sits alone (#478 L6)', () => {
  assert.match(rule('.kw-dd-lim'), /text-wrap: balance;/);
});

test('reduced motion: the leave timeout matches the 120 ms fade; elsewhere it stays 100 ms (#478 L7)', () => {
  const reduced = css.slice(css.indexOf('@media (prefers-reduced-motion: reduce)'));
  const fade = Number(rule(".kw-dd[data-phase='leaving'], .kw-dd[data-phase='landing']", reduced).match(/opacity (\d+)ms/)[1]);
  const pillFade = Number(rule('.kw-dd *, .kw-dd-pill', reduced).match(/transition-duration: (\d+)ms/)[1]);
  assert.equal(d.leaveMs(true), fade);
  assert.equal(d.leaveMs(true), pillFade);
  assert.equal(d.leaveMs(false), Number(rule(".kw-dd[data-phase='leaving']").match(/opacity (\d+)ms/)[1]));
  const was = globalThis.window;
  globalThis.window = { matchMedia: (q) => ({ matches: q === '(prefers-reduced-motion: reduce)' }) };
  try {
    assert.equal(d.leaveMs(), 120, 'reads the setting by default');
  } finally {
    globalThis.window = was;
  }
  // Both exits use it: the chat's layer fade and the "Open a chat" pill.
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const layer = readFileSync(new URL('../src/components/DropLayer.tsx', import.meta.url), 'utf8');
  assert.match(chat, /setTimeout\(\(\) => setExiting\(null\), how === 'landing' \? 160 : leaveMs\(\)\)/);
  assert.match(layer, /setTimeout\(\(\) => setLeaving\(false\), leaveMs\(\)\)/);
});

test('the phone\'s Settings page is not a dialog that keeps the pill away; other modals still are (#478 L4)', () => {
  const doc = (html) => ({ querySelector: (sel) => (sel.split(/,\s*/).some((s) => matches(html, s)) ? {} : null) });
  // A tiny stand-in for querySelector over one element.
  const matches = (attrs, s) => {
    if (s === '[aria-modal="true"]:not([data-space-settings])') return attrs.modal && !attrs.settings;
    if (s === '[aria-modal="true"]') return !!attrs.modal;
    if (s === '[role="alertdialog"]') return attrs.role === 'alertdialog';
    return false;
  };
  assert.equal(d.modalOpen(doc({ modal: true, settings: true })), false);
  assert.equal(d.modalOpen(doc({ modal: true })), true);
  assert.equal(d.modalOpen(doc({ role: 'alertdialog' })), true);
});
