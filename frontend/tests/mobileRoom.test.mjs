// Mobile 0.6.13 slice A (Mira's spec): A1 keyboard mode, A2 composer placeholder, A5 Answers chip,
// A6 tab bar, thread picker in the header. Every case here fails on main 3ca2bf7.
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
  "FocusEvent",
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
// Controllable media: a phone with a touch screen unless a test says otherwise.
const media = { phone: true, coarse: true };
dom.window.matchMedia = (q) => ({
  matches: q.includes("pointer: coarse") ? media.coarse : q.includes("max-width: 767px") || q.includes("max-width: 639px") ? media.phone : false,
  media: q,
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
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { PhoneTabBar } from './src/components/PhoneTabBar';
  import { ThreadPicker } from './src/components/ThreadBar';
  export * from './src/composer';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ChatView, q)));
    render(p);
    return { host, render, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const mountPicker = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ThreadPicker, p)));
    return { host, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const tabBar = (badge) =>
    renderToStaticMarkup(
      React.createElement(PhoneTabBar, {
        label: 'Demo sections',
        active: 'chats',
        onSelect: () => {},
        tabs: [
          { id: 'chats', label: 'Chats', icon: null },
          { id: 'bots', label: 'Bots', icon: null },
          { id: 'inbox', label: 'Inbox', icon: null, badge },
          { id: 'space', label: 'Space', icon: null },
        ],
      }),
    );
  export const act = (fn) => flushSync(fn);
`);

const props = (extra = {}) => ({
  title: "Dev",
  subtitle: "Backend",
  messages: [],
  streamingText: "",
  streamingThinking: "",
  isStreaming: false,
  pendingApprovals: [],
  onSendMessage: () => {},
  onResolveApproval: () => {},
  onCancelDiscussion: () => {},
  onBack: () => {},
  kwants: [],
  teams: [],
  ...extra,
});

const textarea = (h) => h.querySelector("textarea[data-composer]") ?? h.querySelector("textarea");
const focusIn = (el) => ui.act(() => { el.focus(); });
const blurOut = (el) => ui.act(() => { el.blur(); });

test("A1: focusing the composer on a touch phone turns keyboard mode on, blur turns it off", () => {
  media.phone = true;
  media.coarse = true;
  const v = ui.mount(props({ threadLine: "Direct chat" }));
  try {
    const ta = textarea(v.host);
    assert.ok(ta, "composer rendered");
    assert.equal(v.host.querySelector('[data-keyboard-mode="on"]'), null, "off before focus");
    focusIn(ta);
    assert.ok(v.host.querySelector('[data-keyboard-mode="on"]'), "on while the composer has focus");
    const header = v.host.querySelector("[data-chat-header]");
    assert.match(header.className, /\bh-12\b/, "header shrinks to 48px");
    assert.equal(header.textContent.includes("Direct chat"), false, "header line 2 hidden");
    assert.match(v.host.querySelector("[data-thread-bar]").className, /\bhidden\b/, "thread bar hidden");
    assert.match(v.host.querySelector("[data-composer-hints]").className, /\bhidden\b/, "hints row hidden");
    blurOut(ta);
    assert.equal(v.host.querySelector('[data-keyboard-mode="on"]'), null, "off after blur");
    assert.match(v.host.querySelector("[data-chat-header]").className, /\bh-14\b/);
    assert.ok(v.host.querySelector("[data-chat-header]").textContent.includes("Direct chat"), "line 2 back");
  } finally {
    v.unmount();
  }
});

test("A1: a phone-width window with a mouse (no coarse pointer) never enters keyboard mode", () => {
  media.phone = true;
  media.coarse = false;
  const v = ui.mount(props());
  try {
    focusIn(textarea(v.host));
    assert.equal(v.host.querySelector('[data-keyboard-mode="on"]'), null);
  } finally {
    v.unmount();
    media.coarse = true;
  }
});

test("A1: desktop focus changes nothing", () => {
  media.phone = false;
  media.coarse = false;
  const v = ui.mount(props());
  try {
    focusIn(textarea(v.host));
    assert.equal(v.host.querySelector('[data-keyboard-mode="on"]'), null);
    assert.equal(v.host.querySelector("[data-composer-form]").hasAttribute("data-stacked"), false, "desktop form untouched");
  } finally {
    v.unmount();
    media.phone = true;
    media.coarse = true;
  }
});

test("A2: the composer stacks (text above, tools below) only when focused or holding text", () => {
  const v = ui.mount(props());
  try {
    const form = () => v.host.querySelector("[data-composer-form]");
    assert.equal(form().getAttribute("data-stacked"), "false", "one row at rest");
    focusIn(textarea(v.host));
    assert.equal(form().getAttribute("data-stacked"), "true", "two rows while typing");
    blurOut(textarea(v.host));
    assert.equal(form().getAttribute("data-stacked"), "false");
  } finally {
    v.unmount();
  }
});

test("A2: placeholder falls back to plain \"Message\" when the name does not fit", () => {
  const per = (t) => t.length * 9;
  assert.equal(ui.phonePlaceholder("Dev", 300, per), "Message Dev");
  assert.equal(ui.phonePlaceholder("Booktun.ing Marketer with a long name", 220, per), "Message");
  assert.equal(ui.phonePlaceholder("Dev", 0, per), "Message", "unknown width: the safe short form");
  // 24px is kept free at the right edge
  const w = per("Message Dev");
  assert.equal(ui.phonePlaceholder("Dev", w + ui.PLACEHOLDER_PAD, per), "Message Dev");
  assert.equal(ui.phonePlaceholder("Dev", w + ui.PLACEHOLDER_PAD - 1, per), "Message");
});

test("A5: a direct bot chat on a phone has no Answers chip (it only repeats the header)", () => {
  const home = { kind: "bot", bot: { id: "b1", name: "Dev", title: "Backend", avatar_sprite: "" } };
  const v = ui.mount(props({ home, messages: [{ id: "m1", sender_role: "user", sender_id: "u", text: "hi", created_at: "2026-11-05T00:00:00Z" }] }));
  try {
    assert.equal(v.host.querySelector("[data-answer-target]"), null);
    assert.match(v.host.querySelector("[data-composer-hints]").className, /\bhidden\b/, "touch: no hint text either, so no row");
  } finally {
    v.unmount();
  }
});

test("A6: tab bar uses the app palette, 56px tabs, a 24x3 active bar and an 18px amber badge", () => {
  const html = ui.tabBar(3);
  assert.doesNotMatch(html, /sky-/, "no sky blue left");
  assert.match(html, /bg-\[#0e0a21\]/);
  const doc = new JSDOM(html).window.document;
  const tabs = [...doc.querySelectorAll("[data-tab]")];
  assert.equal(tabs.length, 4);
  for (const t of tabs) assert.match(t.className, /\bh-14\b/);
  const active = doc.querySelector('[aria-current="page"]');
  assert.match(active.className, /text-\[#e8e4ff\]/);
  assert.ok(active.querySelector("[data-tab-active-bar]"));
  assert.match(active.querySelector("[data-tab-active-bar]").className, /w-6 h-\[3px\].*bg-\[#9d8fff\]/);
  assert.match(doc.querySelector('[data-tab="bots"]').className, /text-\[#a39dcc\]/);
  const badge = doc.querySelector("[data-tab-badge]");
  assert.equal(badge.textContent, "3");
  assert.match(badge.className, /h-\[18px\] min-w-\[18px\]/);
  assert.match(badge.className, /bg-\[#fcd34d\] text-\[#1b1033\] text-\[11px\]/);
  assert.equal(doc.querySelector('[data-tab="inbox"]').getAttribute("aria-label"), "Inbox, 3 waiting");
  assert.equal(new JSDOM(ui.tabBar(0)).window.document.querySelector("[data-tab-badge]"), null, "no badge at 0");
});

const pickerProps = (extra = {}) => ({
  kwantName: "Dev",
  threads: [
    { id: "t1", kwant_id: "k1", title: "Fix login", archived: false, created_at: "2026-11-04T00:00:00Z", updated_at: "2026-11-04T00:00:00Z", delegation: null },
  ],
  error: null,
  openThreadId: null,
  turns: {},
  awaiting: new Set(),
  directBadge: null,
  showArchived: false,
  onToggleArchived: () => {},
  onOpenDirect: () => {},
  onOpenThread: () => {},
  onNewThread: async () => {},
  onRetry: () => {},
  ...extra,
});

test("thread picker: header line 2 names the conversation and opens a sheet listing them", () => {
  let opened = null;
  const v = ui.mountPicker(pickerProps({ onOpenThread: (t) => (opened = t.id) }));
  try {
    const btn = v.host.querySelector("[data-thread-picker]");
    assert.ok(btn);
    assert.match(btn.textContent, /Direct chat/);
    ui.act(() => btn.click());
    const sheet = document.querySelector('[role="dialog"]');
    assert.ok(sheet, "sheet open");
    const rows = [...sheet.querySelectorAll("button")].map((b) => b.textContent.trim());
    assert.ok(rows[0].startsWith("New thread"), "New thread first");
    assert.ok(rows.some((r) => r.startsWith("Direct chat")));
    const t1 = [...sheet.querySelectorAll("button")].find((b) => b.textContent.includes("Fix login"));
    ui.act(() => t1.click());
    assert.equal(opened, "t1");
    assert.equal(document.querySelector('[role="dialog"]'), null, "sheet closes after a pick");
  } finally {
    v.unmount();
  }
});
