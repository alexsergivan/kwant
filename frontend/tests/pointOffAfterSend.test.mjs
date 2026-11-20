// 30o point-overlay-fixes (autonomy/point-overlay-fixes/design.md, Fix B): once the person sends a message that
// carries a pick, Point turns off (layer, bar and "Pointing · n picked" go, the button is off) and focus stays in
// the composer. A send without picks leaves Point on. Real ChatView with its browser pane; the pane's socket is faked
// the way the server answers the point actions. Fails on main 43c2aa89, where Point stays on after the send.
// Element checks are booleans: a failing assert must never try to print a jsdom node.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLImageElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS', 'DOMException']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
const store = new Map();
const ls = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };
Object.defineProperty(g, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
Object.defineProperty(g, 'sessionStorage', { value: ls, configurable: true, writable: true });
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.IntersectionObserver = g.window.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
g.fetch = g.window.fetch = async () => ({ ok: false, status: 404, json: async () => ({}), text: async () => '' });

const box = { x: 15, y: 303, width: 328, height: 36 };
const sockets = [];
class FakeWebSocket {
  static OPEN = 1;
  constructor(url) { this.url = url; this.readyState = 1; this.sent = []; this.bufferedAmount = 0; sockets.push(this); }
  send(text) {
    const p = JSON.parse(text);
    this.sent.push(p);
    if (typeof p.id !== 'number') return;
    const el = { handle: 'k3f9.12', label: 'button.pay', box, viewport: { width: 1280, height: 800 }, text: 'Pay' };
    if (p.action === 'point_neighbour' || p.action === 'point_at') setTimeout(() => this.receive({ type: 'ack', id: p.id, el }), 0);
    if (p.action === 'point_pick') {
      setTimeout(() => this.receive({ type: 'ack', id: p.id, pick: { attachment_id: '6b0c1d2e-3f40-4a5b-8c6d-7e8f90a1b2c3', picture: true, url: 'https://shop.example.com/', host: 'shop.example.com', path: '/', label: 'button.pay', box, viewport: { width: 1280, height: 800 }, captured_at: '2026-11-15T05:00:00Z' } }), 0);
    }
  }
  close() { this.readyState = 3; }
  receive(message) { this.onmessage?.({ data: JSON.stringify(message) }); }
}
g.WebSocket = g.window.WebSocket = FakeWebSocket;

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ChatView, p)));
    return { host, unmount: () => { root.unmount(); host.remove(); } };
  };
`);
const settle = async () => {
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setImmediate(r));
    await new Promise((r) => setTimeout(r, 0));
  }
};
const key = (target, k) => target.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));
const type = (ta, text) => {
  Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(ta, text);
  ta.dispatchEvent(new window.Event('input', { bubbles: true }));
};
const textarea = (host) => host.querySelector('form textarea');
const send = async (host) => {
  host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};

async function pointingInChat() {
  const sends = [];
  const { host, unmount } = ui.mount({
    title: 'Ben', subtitle: 'Browser', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
    pendingApprovals: [], onSendMessage: (text, atts) => { sends.push({ text, atts }); }, onResolveApproval: () => {}, onCancelDiscussion: () => {},
    kwants: [], teams: [], teamId: 'k1', teamName: 'Ben', isVmPaneOpen: true, onToggleVmPane: () => {},
    home: { kind: 'bot', bot: { id: 'k1', name: 'Ben', title: 'Browser', avatar_sprite: 'Frontend' } },
  });
  try {
    await settle();
    const ws = sockets.at(-1);
    assert.ok(ws, 'the browser pane opened its socket');
    ws.onopen?.();
    ws.receive({ type: 'location', url: 'https://shop.example.com/', title: 'Shop' });
    ws.receive({ type: 'frame', id: 1, image: 'AAAA', width: 1280, height: 800 });
    await settle();
    const button = host.querySelector('[data-point-button]');
    assert.ok(button, 'Point at it shows once a page is on screen');
    button.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    await settle();
    assert.ok(host.querySelector('[data-point-layer]'), 'Point is on');
    return { host, unmount, ws, sends };
  } catch (e) {
    unmount();
    throw e;
  }
}

test('a send that carries a pick turns Point off; focus stays in the composer', async () => {
  const { host, unmount, ws, sends } = await pointingInChat();
  try {
    const view = host.querySelector('[data-point-layer]');
    key(view, 'ArrowUp');
    await settle();
    key(view, 'Enter');
    await settle();
    assert.ok(ws.sent.some((s) => s.action === 'point_pick'), 'the pick went to the browser');
    assert.ok(host.querySelector('[data-point-layer]'), 'a click keeps Point on, to pick more');
    const ta = textarea(host);
    type(ta, 'Why is this button grey?');
    ta.focus();
    await settle();
    await send(host);
    assert.equal(sends.length, 1, 'the message went out');
    assert.equal(sends[0].atts.filter((a) => a.point).length, 1, 'with the pick');
    assert.ok(!host.querySelector('[data-point-layer]'), 'Point turned off: no ring, bar or outline');
    assert.ok(!host.querySelector('[data-point-footer]'), 'no "Pointing · 0 picked" footer');
    const button = host.querySelector('[data-point-button]');
    assert.ok(button, 'the Point button stays');
    assert.equal(button.getAttribute('aria-pressed'), 'false', 'the Point button is off');
    assert.ok(document.activeElement === textarea(host), 'focus stays in the composer (not moved to the Point button)');
  } finally {
    unmount();
  }
});

test('a send without picks leaves Point on', async () => {
  const { host, unmount, sends } = await pointingInChat();
  try {
    type(textarea(host), 'Keep going');
    await settle();
    await send(host);
    assert.equal(sends.length, 1, 'the message went out');
    assert.equal(sends[0].atts.length, 0, 'no attachments');
    assert.ok(host.querySelector('[data-point-layer]'), 'Point stays on');
    assert.ok(host.querySelector('[data-point-footer]'), 'the footer still says Pointing');
  } finally {
    unmount();
  }
});
