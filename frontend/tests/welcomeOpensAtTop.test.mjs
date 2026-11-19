// 29u: a brand-new person's first screen (Get started, the welcome) opens at its top, like the empty chat's home (13d).
// Once a message is in, or a reply is on its way, the chat pins to the bottom as before.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const store = new Map();
const ls = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
dom.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.ResizeObserver ??= globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
globalThis.fetch = dom.window.fetch = async () => new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });
// jsdom lays nothing out: give every box the phone welcome's height (1342 of 619, Mira's pass) and keep what scrollTop is set to.
const tops = new WeakMap();
Object.defineProperty(dom.window.HTMLElement.prototype, 'scrollHeight', { configurable: true, get: () => 1342 });
Object.defineProperty(dom.window.HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => 619 });
Object.defineProperty(dom.window.HTMLElement.prototype, 'scrollTop', { configurable: true, get() { return tops.get(this) ?? 0; }, set(v) { tops.set(this, v); } });

const m = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ChatView, p)));
    return { host, rerender: (q) => flushSync(() => root.render(React.createElement(ChatView, q))), unmount: () => (root.unmount(), host.remove()) };
  };
  export const welcome = React.createElement('div', { 'data-test-welcome': '' }, 'Claude is ready on this computer');
`);
const settle = () => new Promise((r) => setTimeout(r, 30));
const ada = { id: 'a', name: 'Ada', title: 'Tech Lead', avatar_sprite: 'tech lead' };
const props = (messages, extra = {}) => ({
  title: '#general', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage() {}, onResolveApproval() {}, onCancelDiscussion() {},
  kwants: [ada], teams: [], suggestions: [], canRun: true, home: { kind: 'general' },
  welcome: messages.length === 0 ? m.welcome : undefined, ...extra,
});
const log = (v) => v.host.querySelector('[role="log"]');
const msg = { id: 'm1', sender_role: 'user', sender_id: 'u', text: '@helpers hi', created_at: '2026-11-19T06:00:00Z' };

test('ChatView: Get started (the welcome) opens at its top, not pinned to the bottom', async () => {
  const v = m.mount(props([]));
  try {
    await settle();
    assert.ok(v.host.querySelector('[data-test-welcome]'), 'the welcome shows');
    assert.equal(log(v).scrollTop, 0, 'the welcome opens at scrollTop 0');
  } finally {
    v.unmount();
  }
});

test('ChatView: after the first message the chat pins to the bottom as before', async () => {
  const v = m.mount(props([]));
  try {
    await settle();
    v.rerender(props([msg]));
    await settle();
    assert.equal(v.host.querySelector('[data-test-welcome]'), null, 'the welcome is gone');
    assert.equal(log(v).scrollTop, 1342, 'pinned to the bottom');
  } finally {
    v.unmount();
  }
});

test('ChatView: the welcome while a reply is on its way still follows the bottom', async () => {
  const v = m.mount(props([], { isStreaming: true, streamingText: 'Hello' }));
  try {
    await settle();
    assert.equal(log(v).scrollTop, 1342, 'the reply stays in view');
  } finally {
    v.unmount();
  }
});

test('ChatView: the empty home still opens at its top (13d)', async () => {
  const v = m.mount(props([], { welcome: undefined }));
  try {
    await settle();
    assert.ok(v.host.querySelector('[data-bot-home]'), 'the home shows');
    assert.equal(log(v).scrollTop, 0);
  } finally {
    v.unmount();
  }
});
