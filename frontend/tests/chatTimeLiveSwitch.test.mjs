// #1278 (Rigel D1 on #1281): one ChatView serves every chat, so the live start belongs to the chat and the bot it was
// taken for. Mounts the real ChatView in jsdom with a client root, as the app does (renderToStaticMarkup renders once).
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body><div id="r"></div></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'ResizeObserver', 'IntersectionObserver', 'MutationObserver']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;
g.window.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
g.ResizeObserver ??= g.window.ResizeObserver;
g.IS_REACT_ACT_ENVIRONMENT = false;
g.window.scrollTo = () => {}; g.Element.prototype.scrollTo = () => {}; g.Element.prototype.scrollIntoView = () => {};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { ChatView } from './src/components/ChatView';
  export { localClock } from './src/clock';
`);
const h = m.React.createElement;
const realNow = Date.now;
let clock = Date.parse('2026-11-26T08:06:10Z');
Date.now = () => clock;
const iso = (t) => new Date(t).toISOString();

const SCOUT = { id: 's', name: 'Scout', status: 'idle' };
const props = (extra = {}) => ({
  title: 'Support', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: async () => true, onResolveApproval: () => {},
  kwants: [{ id: 'k-support', name: 'Support', status: 'idle' }, SCOUT], teams: [], onContinueFrom: () => {},
  home: { kind: 'bot', bot: { id: 'k-support', name: 'Support' } }, ...extra,
});
const mount = () => {
  const root = m.createRoot(document.getElementById('r'));
  const render = (p) => { m.flushSync(() => root.render(h(m.ChatView, p))); };
  return { root, render };
};
const live = () => {
  const row = document.querySelector('[data-live-row]');
  const t = row?.querySelector('time[data-message-time="live"]');
  return { row: !!row, text: t?.textContent.replace('Started ', '').replace('· ', '') ?? null, iso: t?.getAttribute('datetime') ?? null, name: row?.querySelector('[data-live-name-line]')?.textContent ?? null };
};

const at = (s) => Date.parse(`2026-11-26T${s}Z`);

test('opening another working chat retakes the start; the same chat keeps it', (t) => {
  const { render, root } = mount();
  t.after(() => root.unmount());
  clock = at('11:00:00');
  render(props({ isStreaming: true, streamingSpeakerName: 'Support', contextKey: 'chat-a' }));
  assert.equal(live().iso, iso(at('11:00:00')));
  clock = at('11:05:00');
  render(props({ isStreaming: true, streamingSpeakerName: 'Support', contextKey: 'chat-a', streamingText: 'hi' }));
  assert.equal(live().iso, iso(at('11:00:00')), 'same chat: never ticks');
  clock = at('11:20:00');
  render(props({ isStreaming: true, streamingSpeakerName: 'Scout', contextKey: 'chat-b' }));
  assert.equal(live().iso, iso(at('11:20:00')), "chat B doesn't show chat A's 11:00");
});

test('a hand-over to another bot in the same run retakes the start', (t) => {
  const { render, root } = mount();
  t.after(() => root.unmount());
  clock = at('12:00:00');
  render(props({ isStreaming: true, streamingSpeakerName: 'Support', contextKey: 'chat-a' }));
  assert.equal(live().iso, iso(at('12:00:00')));
  clock = at('12:03:00');
  render(props({ isStreaming: true, streamingSpeakerName: 'Scout', contextKey: 'chat-a' }));
  assert.equal(live().iso, iso(at('12:03:00')));
});

test.after(() => { Date.now = realNow; });
