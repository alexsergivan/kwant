// 0.7 Idea jar shell (Pixel): the /idea send answer and blocked lines, the Settings switch's numbers,
// the jar route, the phone tab bar's violet Jar badge, the sidebar quick-add and the drop toast.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1/jar', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'FocusEvent', 'localStorage', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;
if (!window.matchMedia) window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * from './src/ideaShell';
  export { parseView, viewSegment } from './src/nav';
  export { PhoneTabBar } from './src/components/PhoneTabBar';
  export { IdeaJarRow } from './src/components/IdeaJarRow';
  export { IdeaDropToast } from './src/components/IdeaDropToast';
`);
const h = m.React.createElement;
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  return { host, done: () => { root.unmount(); host.remove(); } };
};
const typeInto = (input, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, value);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
};
const key = (el, k) => el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));

const idea = { id: 'idea_1', space_id: 's1', text: 'A weekly summary of what the team did', status: 'new', created_at: '2026-11-05T10:00:00Z' };

test('/idea send answer: 201 drops, an old server falls back to chat, errors give the spec lines', () => {
  const ok = m.ideaSendOutcome(201, { handled: 'idea', idea });
  assert.equal(ok.kind, 'dropped');
  assert.equal(ok.idea.id, 'idea_1');
  assert.deepEqual(m.ideaSendOutcome(200, { id: 'msg_1' }), { kind: 'chat' });
  assert.deepEqual(m.ideaSendOutcome(403, { error: 'forbidden' }), { kind: 'error', line: 'Only you can add ideas.' });
  assert.deepEqual(m.ideaSendOutcome(400, { error: 'too_long', message: 'Ideas are up to 2,000 characters' }), { kind: 'error', line: 'Ideas are up to 2,000 characters.' });
  assert.deepEqual(m.ideaSendOutcome(400, { error: 'empty' }), { kind: 'error', line: m.JAR_SEND_FAILED });
  assert.deepEqual(m.ideaSendOutcome(500, null), { kind: 'error', line: m.JAR_SEND_FAILED });
  assert.deepEqual(m.ideaSendOutcome(201, { handled: 'idea', idea: null }), { kind: 'error', line: m.JAR_SEND_FAILED });
  assert.equal(m.JAR_SEND_FAILED, 'Couldn’t drop it in the jar. Try again.');
});

test('/idea blocked lines: empty, too long, a file; the counter counts only the idea', () => {
  assert.equal(m.ideaBlockedLine('/idea', 2000), 'Type your idea after /idea');
  assert.equal(m.ideaBlockedLine('/idea    ', 2000), 'Type your idea after /idea');
  assert.equal(m.ideaBlockedLine(`/idea ${'x'.repeat(2140)}`, 2000), '2,140 / 2,000 · too long for the jar');
  assert.equal(m.ideaBlockedLine('/idea a faster search', 2000, true), 'Ideas are text only. Remove the file to drop it in.');
  assert.equal(m.ideaBlockedLine('/IDEA a faster search', 2000), null);
  assert.equal(m.ideaLength('/idea hello'), 5);
});

test('Settings switch: off writes 0, on brings back the last value or 3; the read clamps to 0–10', () => {
  assert.equal(m.ideaCapForSwitch(false, 5), 0);
  assert.equal(m.ideaCapForSwitch(true, 5), 5);
  assert.equal(m.ideaCapForSwitch(true, 0), 3);
  assert.equal(m.ideaCapForSwitch(true, null), 3);
  assert.equal(m.ideaCapForSwitch(true, 40), 10);
  assert.equal(m.parseIdeaCap(0), 0);
  assert.equal(m.parseIdeaCap(undefined), 3);
  assert.equal(m.parseIdeaCap('7'), 7);
  assert.equal(m.parseIdeaCap(99), 10);
  assert.equal(m.parseIdeaCap(-2), 0);
  m.rememberIdeaCap(6);
  assert.equal(m.lastIdeaCap(), 6);
});

test('Route: /s/:id/jar parses and round-trips; older names land on the jar', () => {
  assert.deepEqual(m.parseView('jar'), { view: { kind: 'jar' }, legacy: false });
  assert.equal(m.viewSegment({ kind: 'jar' }), 'jar');
  assert.deepEqual(m.parseView('ideas').view, { kind: 'jar' });
  assert.equal(m.parseView('ideas').legacy, true);
});

test('Phone tab bar: five tabs make five columns, and the Jar badge is violet and says takes ready', () => {
  const html = m.renderToStaticMarkup(h(m.PhoneTabBar, {
    label: 'Demo sections', active: 'chats', onSelect: () => {},
    tabs: [
      { id: 'chats', label: 'Chats', icon: null },
      { id: 'bots', label: 'Bots', icon: null },
      { id: 'jar', label: 'Jar', icon: null, badge: 2, badgeTone: 'violet', badgeNoun: 'takes ready' },
      { id: 'inbox', label: 'Inbox', icon: null, badge: 1 },
      { id: 'space', label: 'Space', icon: null },
    ],
  }));
  assert.match(html, /grid-cols-5/);
  assert.match(html, /aria-label="Jar, 2 takes ready"/);
  assert.match(html, /aria-label="Inbox, 1 waiting"/);
  assert.match(html, /data-tone="violet"[^>]*class="[^"]*bg-\[#8b7bff\]/);
});

test('Sidebar quick-add: + opens the field, Enter drops and stays open, Esc closes and refocuses +', async () => {
  const dropped = [];
  const { host, done } = await mount(h(m.IdeaJarRow, { active: false, count: 2, onOpen: () => {}, onDrop: async (t) => { dropped.push(t); }, className: 'row' }));
  assert.equal(host.querySelector('[data-jar-badge]').textContent.replace(/\s+/g, ' ').trim(), '2 takes ready');
  const plus = host.querySelector('[data-jar-add]');
  assert.equal(plus.getAttribute('aria-label'), 'Drop an idea in the jar');
  plus.click();
  await settle();
  let input = host.querySelector('[data-jar-quick-input]');
  assert.ok(input, 'the field opens');
  assert.equal(plus.getAttribute('aria-expanded'), 'true');
  typeInto(input, 'A faster way to find old chats');
  await settle();
  host.querySelector('[data-jar-quick-add]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(dropped, ['A faster way to find old chats']);
  input = host.querySelector('[data-jar-quick-input]');
  assert.ok(input, 'stays open for the next idea');
  assert.equal(input.value, '');
  key(input, 'Escape');
  await settle();
  assert.equal(host.querySelector('[data-jar-quick-input]'), null, 'Esc closes it');
  assert.equal(document.activeElement, plus, 'and focus goes back to +');
  done();
});

test('Drop toast: says Dropped in the jar, Undo calls back, then reads Taken back out', async () => {
  let undone = 0;
  const { host, done } = await mount(h(m.IdeaDropToast, { state: 'dropped', onUndo: () => { undone += 1; }, onOpenJar: () => {}, onClose: () => {} }));
  const box = host.querySelector('[data-idea-toast] [role="status"]');
  assert.equal(box.getAttribute('role'), 'status');
  assert.match(box.textContent, /Dropped in the jar/);
  host.querySelector('[data-idea-undo]').click();
  assert.equal(undone, 1);
  assert.ok(host.querySelector('[data-idea-open-jar]'));
  done();
  const after = await mount(h(m.IdeaDropToast, { state: 'undone', onUndo: () => {}, onClose: () => {} }));
  assert.match(after.host.textContent, /Taken back out/);
  after.done();
});

test('Wiring: App catches idea_updated, skips the bubble for /idea; ChatView dresses the field', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /case 'idea_updated':[\s\S]{0,60}emitIdeaUpdated\(event\.data\)/);
  assert.match(app, /isIdeaCommand\(text\) && !attachments\.length/);
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(chat, /aria-label="Drop in the jar"/);
  assert.match(chat, /Goes in the idea jar, not to/);
  assert.match(chat, /Drop an idea in the jar/);
  const settings = readFileSync(new URL('../src/components/SettingsModal.tsx', import.meta.url), 'utf8');
  assert.match(settings, /idea_jar_daily_cap/);
});
