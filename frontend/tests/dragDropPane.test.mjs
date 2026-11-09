// 0.8.0 drag and drop into chats (Pixel, Mira's design release-080/drag-drop/design.md): the whole open
// chat takes a file drop (header, message list and composer), with a card saying who gets the files
// and what is coming; after the drop the files sit in the same tray and the cursor is in the field.
// Paste, the paperclip and the 10-file limit are unchanged. The pane cases fail on main fd3dfafa,
// where only the <form> listens and there is no drop layer.
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
  import { DropPillHost } from './src/components/DropLayer';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ChatView, p)));
    return { host, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const mountPill = () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(DropPillHost)));
    return { host, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const act = (fn) => flushSync(fn);
`);

const props = (extra = {}) => ({
  title: 'Frontend', subtitle: 'Pixel', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, onCancelDiscussion: () => {}, kwants: [], teams: [],
  home: { kind: 'bot', bot: { id: 'k-front', name: 'Frontend', title: 'Frontend engineer', avatar_sprite: 'Frontend' } }, ...extra,
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
// The card itself lands in the event's own task (ChatView flushes it). What effects set afterwards (the
// live region, the exit phase) runs in React's scheduler, which Node drives with setImmediate; whether
// that or a setTimeout(0) comes first isn't fixed in Node (it flipped on CI's Node 20), so wait for both.
const tick = async () => {
  for (let i = 0; i < 3; i += 1) {
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setTimeout(r, 0));
  }
};
// A dragleave inside the page has a relatedTarget (where the pointer went); pass `out` for one that
// leaves the window, which has none (#478 M2 ends the drag on it).
const OUT = Symbol('out of the window');
const fire = async (el, type, dt, to) => {
  const e = new dom.window.Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(e, 'dataTransfer', { value: dt });
  if (type === 'dragleave') Object.defineProperty(e, 'relatedTarget', { value: to === OUT ? null : (to ?? document.body) });
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
const layer = (host) => host.querySelector('[data-drop-layer]');
const phase = (host) => layer(host)?.getAttribute('data-phase') ?? null;
const heading = (host) => host.querySelector('[data-drop-heading]')?.textContent ?? null;
const said = (host) => host.querySelector('[data-drop-announce]')?.textContent ?? '';
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
// Somewhere in the window that isn't the chat (the sidebar).
const elsewhere = () => {
  const el = document.createElement('nav');
  document.body.appendChild(el);
  return el;
};
// Ends any drag a test left in the air, so the next one starts clean.
const endDrag = async () => {
  ui.act(() => window.dispatchEvent(new dom.window.Event('dragend')));
  await wait(200);
};
const typeIn = (field, text) => {
  const set = Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype, 'value').set;
  ui.act(() => {
    set.call(field, text);
    field.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  });
};

test('a file drag over the header or the message list lights up the chat; off the chat it stays armed', async () => {
  const v = ui.mount(props());
  const side = elsewhere();
  try {
    const { header, list, form } = parts(v.host);
    assert.ok(header && list, 'the chat has a header and a message list');
    const two = () => dataTransfer([mkFile('a.png'), mkFile('b.pdf', 'application/pdf')]);
    // Into the window over the sidebar first: armed, quieter, the composer not lit.
    await fire(side, 'dragenter', two());
    assert.equal(phase(v.host), 'armed');
    assert.equal(heading(v.host), 'Drop here to give Frontend these files');
    assert.ok(!form.className.includes('kw-dd-lit'));
    assert.equal(layer(v.host).style.pointerEvents, 'none', 'armed never takes the drop');
    // Onto the header: over, lit, the layer is the drop surface.
    const enter = await fire(header, 'dragenter', two());
    await fire(side, 'dragleave', two());
    assert.equal(phase(v.host), 'over');
    assert.equal(heading(v.host), 'Drop to give Frontend these files');
    assert.equal(v.host.querySelector('[data-drop-count]')?.textContent, '2');
    assert.ok(form.className.includes('kw-dd-lit') && form.className.includes('kw-dd-raised'), 'the composer is lit above the veil');
    assert.equal(layer(v.host).style.pointerEvents, 'auto');
    assert.equal(layer(v.host).getAttribute('aria-hidden'), 'true');
    assert.equal(said(v.host), 'Drop to give Frontend 2 files: 1 image, 1 PDF.');
    void enter;
    // Moving onto a child element: enter the list, then leave the header. No flicker.
    await fire(list, 'dragenter', two());
    await fire(header, 'dragleave', two());
    assert.equal(phase(v.host), 'over', 'still on while the drag is anywhere in the chat');
    const over = await fire(list, 'dragover', two());
    assert.equal(over.defaultPrevented, true);
    assert.equal(over.dataTransfer.dropEffect, 'copy');
    // Back to the sidebar: armed again, not gone.
    await fire(side, 'dragenter', two());
    await fire(list, 'dragleave', two());
    assert.equal(phase(v.host), 'armed');
    // Out of the window: the layer fades and goes.
    await fire(side, 'dragleave', two(), OUT);
    assert.equal(phase(v.host), 'leaving');
    await wait(150);
    assert.equal(layer(v.host), null);
  } finally {
    side.remove();
    v.unmount();
    await endDrag();
  }
});

test('a drop on the message list lands in the tray, the cursor goes to the field and it is read out', async () => {
  sent.length = 0;
  const v = ui.mount(props());
  try {
    const { list, field } = parts(v.host);
    const files = [mkFile('one.png'), mkFile('two.pdf', 'application/pdf'), mkFile('three.md', 'text/markdown')];
    await fire(list, 'dragenter', dataTransfer(files));
    assert.equal(v.host.querySelector('[data-drop-summary]')?.textContent, '3 files · 1 image, 1 PDF, 1 text file');
    const drop = await fire(layer(v.host), 'drop', dataTransfer(files));
    assert.equal(drop.defaultPrevented, true, 'the browser does not open the file');
    assert.equal(chips(v.host), 3, 'three files in the tray');
    assert.equal(sent.length, 3, 'all three upload as with the paperclip');
    assert.equal(document.activeElement, field, 'the cursor is in the field');
    assert.equal(said(v.host), '3 files added for Frontend. Uploading.');
    assert.equal(phase(v.host), 'landing', 'the card hands the files down');
    assert.equal(v.host.querySelectorAll('li[data-fresh]').length, 3, 'the new chips come in tinted');
    const delays = [...v.host.querySelectorAll('li[data-fresh]')].map((li) => li.style.getPropertyValue('--kw-chip-delay'));
    assert.deepEqual(delays, ['40ms', '70ms', '100ms']);
    await wait(200);
    assert.equal(layer(v.host), null, 'gone by 160 ms');
    await wait(700);
    assert.equal(v.host.querySelectorAll('li[data-fresh]').length, 0, 'the tint is gone by 800 ms');
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('text and links dragged over the chat are left alone', async () => {
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    const e = await fire(list, 'dragenter', dataTransfer([], ['text/plain', 'text/uri-list']));
    const o = await fire(list, 'dragover', dataTransfer([], ['text/plain', 'text/uri-list']));
    assert.equal(e.defaultPrevented, false);
    assert.equal(o.defaultPrevented, false);
    assert.equal(layer(v.host), null);
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('unchanged: the box itself still takes a drop, and eleven files keep ten with the note', async () => {
  sent.length = 0;
  const v = ui.mount(props());
  try {
    const { form } = parts(v.host);
    const eleven = Array.from({ length: 11 }, (_, i) => mkFile(`p${i}.png`));
    await fire(form, 'dragenter', dataTransfer(eleven));
    assert.match(v.host.querySelector('[data-drop-warning]')?.textContent ?? '', /1 more won't fit: 10 files per message\./);
    assert.equal(v.host.querySelector('[data-drop-count]')?.textContent, '10', 'the badge counts only what will go');
    await fire(form, 'drop', dataTransfer(eleven));
    assert.equal(chips(v.host), 10);
    assert.match(v.host.textContent, /Up to 10 files per message; one was left out\./);
  } finally {
    v.unmount();
    await endDrag();
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
    assert.equal(v.host.querySelectorAll('li[data-fresh]').length, 0, 'only a drop gets the landing tint');
  } finally {
    v.unmount();
  }
});

test('/idea mode: the card says files can\'t go in the jar, the cursor says no, and the drop is ignored', async () => {
  sent.length = 0;
  const v = ui.mount(props());
  try {
    const { list, field, form } = parts(v.host);
    typeIn(field, '/idea a faster search');
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'refused');
    assert.equal(heading(v.host), "Files can't go in the idea jar");
    assert.match(v.host.querySelector('[data-drop-refusal]').textContent, /Ideas are text only\./);
    assert.match(v.host.textContent, /Clear \/idea from the field to send them to Frontend\./);
    assert.ok(!form.className.includes('kw-dd-lit'), 'not lit');
    assert.equal(v.host.querySelector('[data-drop-count]'), null, 'no badge');
    assert.equal(said(v.host), "Files can't go in the idea jar. Ideas are text only.");
    const over = await fire(list, 'dragover', dataTransfer([mkFile('a.png')]));
    assert.equal(over.dataTransfer.dropEffect, 'none');
    const drop = await fire(layer(v.host), 'drop', dataTransfer([mkFile('a.png')]));
    assert.equal(drop.defaultPrevented, true, 'never opens the file either');
    assert.equal(chips(v.host), 0);
    assert.equal(sent.length, 0);
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('all refused: a ZIP gets the neutral card and is not added', async () => {
  sent.length = 0;
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    const zip = () => dataTransfer([mkFile('site.zip', 'application/zip')]);
    await fire(list, 'dragenter', zip());
    assert.equal(phase(v.host), 'refused');
    assert.equal(layer(v.host).getAttribute('data-refused'), 'true');
    assert.equal(heading(v.host), "This can't go to Frontend");
    assert.match(v.host.textContent, /ZIP files can't be attached\./);
    assert.match(v.host.textContent, /Unzip it and drop the files inside\./);
    await fire(layer(v.host), 'drop', zip());
    assert.equal(chips(v.host), 0);
    assert.equal(sent.length, 0);
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('some won\'t go: the won\'t-go rows, and a dropped folder is left out with the folder line', async () => {
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    const twelve = [...Array.from({ length: 11 }, (_, i) => mkFile(`p${i}.png`)), mkFile('a.zip', 'application/zip')];
    await fire(list, 'dragenter', dataTransfer(twelve));
    const rows = [...v.host.querySelectorAll('[data-drop-warning] > div')].map((d) => d.textContent);
    assert.deepEqual(rows, ["1 more won't fit: 10 files per message.", "1 ZIP can't go. Images, videos, PDFs and text or code files work."]);
    assert.equal(v.host.querySelector('[data-drop-summary]')?.textContent, '10 files · 10 images');
    await endDrag();

    const png = mkFile('cat.png');
    const dt = dataTransfer([png]);
    dt.items.unshift({ kind: 'file', type: '', getAsFile: () => mkFile('Photos', '', 0), webkitGetAsEntry: () => ({ isDirectory: true, name: 'Photos' }) });
    await fire(list, 'dragenter', dt);
    assert.equal(v.host.querySelector('[data-drop-summary]')?.textContent, '2 files · 1 image, 1 file', 'a folder looks like a file before the drop');
    await fire(list, 'drop', dt);
    assert.equal(chips(v.host), 1, 'only the file');
    assert.match(v.host.querySelector('p[role="alert"]').textContent, /Folders can't go: drop the files inside\./);
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('a room shows its bots and shares with the room; an @mention gives one bot the files', async () => {
  const bots = ['Marketer', 'Scout', 'Quill', 'Echo', 'Orbit'].map((n, i) => ({ id: `k${i}`, name: n, title: n, avatar_sprite: n }));
  const v = ui.mount(props({ title: 'Booktuning Marketing', home: undefined, teamName: 'Booktuning Marketing', teamMembers: bots, kwants: bots }));
  try {
    const { list, field } = parts(v.host);
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(heading(v.host), 'Drop to share with the Booktuning Marketing team');
    assert.equal(v.host.querySelector('.kw-dd-more')?.textContent, '+1', 'four avatars, then +1');
    assert.equal(said(v.host), 'Drop to share 1 file with the Booktuning Marketing team: 1 image.');
    await endDrag();
    typeIn(field, '@Scout look at this');
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(heading(v.host), 'Drop to give Scout this file');
    assert.equal(v.host.querySelector('.kw-dd-more'), null);
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('the side pane is not a target, a modal keeps the layer away; Esc, a mouse move or leaving the window hide it, a still hand does not', async () => {
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    // A side pane next to the chat: inside the chat's root, outside its column.
    const pane = document.createElement('aside');
    list.parentElement.parentElement.appendChild(pane);
    await fire(pane, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'armed');
    const o = await fire(pane, 'dragover', dataTransfer([mkFile('a.png')]));
    assert.equal(o.defaultPrevented, true);
    assert.equal(o.dataTransfer.dropEffect, 'none');
    await fire(pane, 'drop', dataTransfer([mkFile('a.png')]));
    assert.equal(chips(v.host), 0, 'a drop on the side pane does nothing');
    pane.remove();
    await endDrag();

    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'over');
    ui.act(() => window.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape' })));
    await tick();
    assert.equal(phase(v.host), 'leaving', 'Esc hides it');
    await endDrag();

    // #478 M2: a hand held still (WebKit sends no dragover then) keeps the layer; the first mouse
    // move means the OS drag is over; a dragleave out of the window ends it too.
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'over');
    await wait(1000);
    assert.equal(phase(v.host), 'over', 'no dragover for 1 s: still over');
    ui.act(() => window.dispatchEvent(new dom.window.Event('mousemove')));
    await tick();
    assert.equal(phase(v.host), 'leaving', 'a mousemove ends it');
    await endDrag();
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    await fire(list, 'dragleave', dataTransfer([mkFile('a.png')]), OUT);
    assert.equal(phase(v.host), 'leaving', 'a dragleave with no relatedTarget ends it, depth or not');
    await endDrag();

    const modal = document.createElement('div');
    modal.setAttribute('aria-modal', 'true');
    document.body.appendChild(modal);
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(layer(v.host), null, 'never over a modal');
    modal.remove();
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('a team @mention shows that team\'s bots, not a made-up face, and gives the files to the team (#478 M4)', async () => {
  const bots = ['Chef', 'Sous', 'Baker'].map((n, i) => ({ id: `kk${i}`, name: n, title: n, avatar_sprite: n }));
  const teams = [{ id: 't-kitchen', name: 'Kitchen', member_kwants: ['kk0', 'kk2'] }, { id: 't-empty', name: 'Garden', member_kwants: ['gone'] }];
  const v = ui.mount(props({ kwants: [{ id: 'k-front', name: 'Frontend', title: 'Frontend', avatar_sprite: 'Frontend' }, ...bots], teams }));
  try {
    const { list, field } = parts(v.host);
    typeIn(field, '@Kitchen look at these');
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(heading(v.host), 'Drop to share with the Kitchen team');
    const many = v.host.querySelector('.kw-dd-many');
    assert.ok(many, 'the room stack, not one big avatar');
    assert.ok(!v.host.querySelector('.kw-dd-one'), 'no single big avatar');
    assert.deepEqual([...many.children].map((c) => c.getAttribute('aria-label') ?? c.textContent).length, 2, "Kitchen's two members");
    assert.ok(!v.host.querySelector('[data-drop-team-icon]'), 'members found: no group circle');
    await fire(list, 'drop', dataTransfer([mkFile('a.png')]));
    assert.equal(said(v.host), '1 file added for the Kitchen team. Uploading.');
    await endDrag();
    // A team whose bots aren't loaded: a plain group circle.
    typeIn(field, '@Garden here');
    await fire(list, 'dragenter', dataTransfer([mkFile('b.png')]));
    assert.equal(heading(v.host), 'Drop to share with the Garden team');
    assert.ok(v.host.querySelector('.kw-dd-many [data-drop-team-icon]'), 'the Users circle');
    assert.equal(v.host.querySelector('.kw-dd-many')?.children.length, 1, 'no bot faces');
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('the composer\'s "New" hint hides while the drop layer is up and comes back after, unseen (#478 M3)', async () => {
  const v = ui.mount(props());
  try {
    const { list } = parts(v.host);
    const shown = () => {
      const el = v.host.querySelector('[data-new-tag="mention"]');
      return !!el && !el.classList.contains('hidden');
    };
    assert.equal(shown(), true, 'the hint shows before the drag');
    await fire(list, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'over');
    assert.equal(shown(), false, 'hidden during the drag');
    await endDrag();
    assert.equal(layer(v.host), null);
    assert.equal(shown(), true, 'back after: not marked as seen');
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('the over or refused card shows in the dragenter\'s own task, with no armed frame first (#478 round 2)', async () => {
  const v = ui.mount(props());
  try {
    const { list, field } = parts(v.host);
    // No wait between the event and the read: the window watch's armed render and the pane's own change
    // land together, so the browser never paints "Drop here to…" first.
    const enter = (dt) => {
      const e = new dom.window.Event('dragenter', { bubbles: true, cancelable: true });
      Object.defineProperty(e, 'dataTransfer', { value: dt });
      ui.act(() => list.dispatchEvent(e));
    };
    enter(dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'over');
    assert.equal(heading(v.host), 'Drop to give Frontend this file');
    await endDrag();
    typeIn(field, '/idea later');
    enter(dataTransfer([mkFile('a.png')]));
    assert.equal(phase(v.host), 'refused');
  } finally {
    v.unmount();
    await endDrag();
  }
});

test('no chat open: the "Open a chat" pill shows and is read out; with a chat open it does not', async () => {
  const p = ui.mountPill();
  const side = elsewhere();
  try {
    await fire(side, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(p.host.querySelector('[data-drop-pill]')?.textContent, 'Open a chat to drop files there');
    assert.equal(p.host.querySelector('[data-drop-pill-announce]').textContent, 'Open a chat to drop files there.');
    await endDrag();
    assert.equal(p.host.querySelector('[data-drop-pill]'), null);
    const v = ui.mount(props());
    await fire(side, 'dragenter', dataTransfer([mkFile('a.png')]));
    assert.equal(p.host.querySelector('[data-drop-pill]'), null, 'the chat shows its own layer instead');
    assert.equal(phase(v.host), 'armed');
    v.unmount();
  } finally {
    side.remove();
    p.unmount();
    await endDrag();
  }
});
