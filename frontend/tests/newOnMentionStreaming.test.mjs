// Mira's #266 fix 3: the "New" tag on the composer's "@" is hidden while a reply streams (so it never sits on
// the working strip's edge) and comes back after the turn, because hiding it does not mark it as seen.
import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { loadBundled } from "./bundle.mjs";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost/",
  pretendToBeVisual: true,
});
for (const k of [
  "window",
  "document",
  "navigator",
  "HTMLElement",
  "HTMLButtonElement",
  "Element",
  "Node",
  "Event",
  "KeyboardEvent",
  "MutationObserver",
  "requestAnimationFrame",
  "cancelAnimationFrame",
  "getComputedStyle",
  "CSS",
]) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, {
    value: dom.window[k],
    configurable: true,
    writable: true,
  });
}
const store = new Map();
const ls = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => void store.set(k, String(v)),
  removeItem: (k) => void store.delete(k),
};
Object.defineProperty(globalThis, "localStorage", {
  value: ls,
  configurable: true,
  writable: true,
});
Object.defineProperty(dom.window, "localStorage", {
  value: ls,
  configurable: true,
});
dom.window.matchMedia ??= () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
dom.window.ResizeObserver ??= globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export * from './src/whatsNew';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ChatView, q)));
    render(p);
    return { host, render, unmount: () => root.unmount() };
  };
`);

const props = (isStreaming) => ({
  title: "Dev",
  subtitle: "Backend",
  messages: [],
  streamingText: "",
  streamingThinking: "",
  isStreaming,
  pendingApprovals: [],
  onSendMessage: () => {},
  onResolveApproval: () => {},
  onCancelDiscussion: () => {},
  kwants: [],
  teams: [],
});
const tag = (host) => host.querySelector('[data-new-tag="mention"]');

test('"New" on @ is not rendered while a reply streams, and is back after the turn when not yet seen', () => {
  assert.equal(ui.isNewThing("mention"), true, "not seen yet");
  const v = ui.mount(props(true));
  try {
    assert.equal(tag(v.host), null, "hidden while streaming");
    assert.equal(
      ui.isNewThing("mention"),
      true,
      "streaming does not mark it as seen",
    );
    v.render(props(false));
    assert.ok(tag(v.host), "back after the turn");
    assert.equal(tag(v.host).hasAttribute("data-leaving"), false);
    v.render(props(true));
    assert.equal(tag(v.host), null, "hidden again on the next turn");
  } finally {
    v.unmount();
  }
});

test('once seen, "New" on @ stays gone after the turn too', () => {
  ui.markNewSeen("mention");
  const v = ui.mount(props(false));
  try {
    assert.equal(tag(v.host), null);
  } finally {
    v.unmount();
  }
});
