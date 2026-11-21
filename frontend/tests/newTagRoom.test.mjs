// 31q composer-new-tag-clear: on desktop the "New" tag on "@" sits 6 px above the field (#478). It used to poke past
// the composer's top edge into the chat and cover whatever scrolled under it. While it shows, the composer makes
// exactly that much room (+4 px clear of the hairline); once "mention" is seen the room goes. Phone is unchanged.
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
let phoneNow = false;
dom.window.matchMedia = (q) => ({
  matches: /max-width:\s*767px/.test(q) ? phoneNow : false,
  media: q,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
dom.window.ResizeObserver = globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};

// Layout as measured in a real 1440 x 900 window (Mira's rig, b232483): the field's top is at 800 and the
// tag 6 px above it (24 px box, so its top at 771); the composer's top edge at 783, less any room it makes
// (the composer is pinned to the window's bottom, so it grows upward).
const room = () => Number(document.querySelector("[data-new-tag-room]")?.dataset.newTagRoom ?? 0);
const rect = (top, h) => ({ top, bottom: top + h, height: h, left: 0, right: 40, width: 40, x: 0, y: top, toJSON() {} });
const realRect = dom.window.HTMLElement.prototype.getBoundingClientRect;
dom.window.HTMLElement.prototype.getBoundingClientRect = function () {
  if (this.matches?.('[data-new-tag="mention"]')) return rect(771, 24);
  if (this.querySelector?.(":scope > form[data-composer-form]")) return rect(783 - room(), 117 + room());
  if (this.matches?.("form[data-composer-form]")) return rect(800, 54);
  return realRect.call(this);
};

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export * from './src/whatsNew';
  export { flushSync } from 'react-dom';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ChatView, q)));
    render(p);
    return { host, render, unmount: () => { root.unmount(); host.remove(); } };
  };
`);

const props = (isStreaming = false) => ({
  title: "Pip",
  subtitle: "Analyst",
  messages: [{ id: "m1", sender_role: "user", sender_id: "u", text: "October numbers please", created_at: "2026-11-05T00:00:00Z" }],
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

test("desktop: while the mention tag shows, the composer makes room for it above the field, so it is clear of the chat", () => {
  phoneNow = false;
  assert.equal(ui.isNewThing("mention"), true, "not seen yet");
  const v = ui.mount(props());
  try {
    assert.ok(tag(v.host), "the tag shows");
    // 783 (edge) + 4 (clear) - 771 (tag top) = 16 px of room
    assert.equal(room(), 16, "room = the tag's overhang past the composer's edge + 4 px");
    const wrap = v.host.querySelector("form[data-composer-form]").parentElement;
    assert.ok(wrap.querySelector(":scope > [data-new-tag-room]"), "the room sits inside the composer, not in the chat");
    assert.equal(wrap.querySelector("[data-new-tag-room]").getAttribute("aria-hidden"), "true");
    const edge = wrap.getBoundingClientRect().top;
    assert.ok(tag(v.host).getBoundingClientRect().top >= edge + 4, "tag top is at least 4 px below the composer's edge");
    v.render(props());
    assert.equal(room(), 16, "stable on the next render");
  } finally {
    v.unmount();
  }
});

test("desktop: no room while a reply streams (the tag is hidden then), and it is back after the turn", () => {
  phoneNow = false;
  const v = ui.mount(props(true));
  try {
    assert.equal(tag(v.host), null);
    assert.equal(room(), 0);
    v.render(props(false));
    assert.equal(room(), 16);
  } finally {
    v.unmount();
  }
});

test("phone: unchanged, the tag sits in the @ button's own box and the composer makes no room", () => {
  phoneNow = true;
  const v = ui.mount(props());
  try {
    assert.ok(tag(v.host), "phone tag shows");
    assert.equal(room(), 0);
    assert.equal(v.host.querySelector("[data-new-tag-room]"), null);
  } finally {
    v.unmount();
    phoneNow = false;
  }
});

test("once mention is seen, the room goes with the tag and the composer is as before", () => {
  phoneNow = false;
  const v = ui.mount(props());
  try {
    assert.equal(room(), 16);
    ui.flushSync(() => ui.markNewSeen("mention"));
    assert.equal(v.host.querySelector("[data-new-tag-room]"), null, "room gone");
  } finally {
    v.unmount();
  }
  const w = ui.mount(props());
  try {
    assert.equal(tag(w.host), null);
    assert.equal(w.host.querySelector("[data-new-tag-room]"), null, "a fresh chat makes no room");
  } finally {
    w.unmount();
  }
});
