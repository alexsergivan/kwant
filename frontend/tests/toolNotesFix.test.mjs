// 0.9.0 P5 QA fixes (Rigel, qa-p5-ui.md): M1 the server's own sentence on a refused save (D11 409,
// 400, 403), S1 keyboard focus back on "Edit note" after Esc, Cancel or Save.
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
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export { ToolNotesSection, noteProblem } from './src/components/ToolNotesSection';
  export { ConnectionsError } from './src/connections';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);

const NOW = Date.parse('2026-11-10T15:00:00Z');
const note = (over = {}) => ({
  id: 'tn_1', tool: 'github · create_pull_request', connection: 'github', builtin: false, removed: false, kind: 'broke', state: 'open',
  text: 'Fails right after a push.', bot_text: 'Fails…', edited_by_person: false, written_by_person: false, count: 1,
  bots: [{ id: 'k1', name: 'Pixel' }], created_at: '2026-11-08T10:00:00Z', last_at: '2026-11-10T14:02:00Z', updated_at: null,
  fixed_at: null, purge_at: null, where: null, ...over,
});
const list = (notes) => ({ notes, counts: { all: notes.length, broke: notes.length, tip: 0, missing: 0, fixed: 0 } });
const act = async (fn) => {
  await fn();
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const ALREADY = 'There is already an open “Tip” note for github · create_pull_request.';

async function mount(api, notes = [note()]) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  await act(async () => root.render(m.createElement(m.ToolNotesSection, { api, now: NOW, initialList: list(notes) })));
  const q = (sel) => host.querySelector(sel);
  const byText = (t) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === t);
  const click = (el) => act(async () => el.dispatchEvent(new MouseEvent('click', { bubbles: true })));
  const done = async () => {
    await act(async () => root.unmount());
    host.remove();
  };
  return { host, q, byText, click, done };
}
const tick = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};

test('noteProblem: server refusals and offline say their own sentence, anything else the generic line', () => {
  const E = m.ConnectionsError;
  const generic = 'Couldn’t save the note. Try again.';
  assert.equal(m.noteProblem(new E(ALREADY, 409), generic), ALREADY);
  assert.equal(m.noteProblem(new E('Text is 280 characters at most.', 400), generic), 'Text is 280 characters at most.');
  assert.equal(m.noteProblem(new E('Only you can see and change connections.', 403), generic), 'Only you can see and change connections.');
  assert.equal(m.noteProblem(new E('Couldn’t reach the server. Check your connection and try again.', 0), generic), 'Couldn’t reach the server. Check your connection and try again.');
  assert.equal(m.noteProblem(new E('Something went wrong (HTTP 500).', 500), generic), generic);
  assert.equal(m.noteProblem(new Error('boom'), generic), generic);
  assert.equal(m.noteProblem(new E('  ', 409), generic), generic);
});

test('a kind change the server refuses (409) shows its sentence; the editor stays open with the text', async () => {
  let updates = 0;
  const api = {
    list: async () => list([note()]),
    add: async () => { throw new Error('unused'); },
    update: async () => { updates++; throw new m.ConnectionsError(ALREADY, 409); },
    remove: async () => {},
  };
  const v = await mount(api);
  await v.click(v.q('button[aria-label="Edit note"]'));
  const area = v.q('textarea');
  assert.ok(area, 'editor open');
  await v.click(v.byText('Tip'));
  await v.click(v.byText('Save'));
  await tick();
  assert.equal(updates, 1);
  assert.equal(v.q('[role="alert"]').textContent, ALREADY);
  assert.ok(!v.host.textContent.includes('Couldn’t save the note'));
  assert.ok(v.q('textarea'), 'editor still open');
  assert.equal(v.q('textarea').value, 'Fails right after a push.');
  assert.equal(v.q('li[data-note="tn_1"]').querySelector('[data-note-editor]') !== null, true);
  await v.done();
});

test('Reopen refused with 409 shows the server sentence in the row error', async () => {
  const api = {
    list: async () => list([note({ state: 'fixed' })]),
    add: async () => {},
    update: async () => { throw new m.ConnectionsError(ALREADY, 409); },
    remove: async () => {},
  };
  const v = await mount(api, [note({ state: 'fixed', fixed_at: '2026-11-10T14:00:00Z' })]);
  await v.click(v.byText('Reopen'));
  await tick();
  assert.ok(v.host.textContent.includes(ALREADY));
  assert.ok(!v.host.textContent.includes('Couldn’t save the note'));
  await v.done();
});

for (const how of ['Esc', 'Cancel', 'Save']) {
  test(`after ${how} closes the editor, focus is back on that note’s "Edit note"`, async () => {
    const api = {
      list: async () => list([note({ text: 'Changed.' })]),
      add: async () => {},
      update: async (_id, patch) => note({ ...patch }),
      remove: async () => {},
    };
    const v = await mount(api);
    await v.click(v.q('button[aria-label="Edit note"]'));
    const area = v.q('textarea');
    assert.ok(document.activeElement === area, `focus starts in the textarea, not ${document.activeElement?.tagName}`);
    if (how === 'Esc') {
      await act(async () => area.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })));
    } else if (how === 'Cancel') {
      await v.click(v.byText('Cancel'));
    } else {
      await act(async () => {
        const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
        set.call(area, 'Changed.');
        area.dispatchEvent(new Event('input', { bubbles: true }));
      });
      await v.click(v.byText('Save'));
      await tick();
    }
    assert.ok(v.q('textarea') === null, 'editor closed');
    const edit = v.q('li[data-note="tn_1"] button[aria-label="Edit note"]');
    assert.ok(edit);
    assert.ok(document.activeElement === edit, `focus is on ${document.activeElement?.tagName} ${document.activeElement?.getAttribute?.('aria-label') ?? ''}`);
    await v.done();
  });
}
