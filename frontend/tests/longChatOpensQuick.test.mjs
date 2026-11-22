// 33q (brief: autonomy/long-chat-opens-quick/brief.md): opening a ~500-message chat was one long task. Two things cut it:
// every chat row wrapper carries kw-row-lazy, whose CSS (content-visibility: auto) lets far off-screen rows skip style,
// layout and paint; and a row draws only the avatar its width shows (gutter from `sm` up, name line below) instead of
// both with one hidden by CSS. JSDOM has no layout, so this pins the class, the rule and the one-avatar-per-row DOM.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
dom.window.HTMLElement.prototype.scrollIntoView = function () {};
// The width under test: `sm` (640 px and up) and the phone layout (767 px and below) follow it.
let width = 1440;
const matches = (q) => {
  const min = /min-width:\s*(\d+)px/.exec(q);
  const max = /max-width:\s*(\d+)px/.exec(q);
  if (min) return width >= Number(min[1]);
  if (max) return width <= Number(max[1]);
  return false;
};
dom.window.matchMedia = (q) => ({ matches: matches(q), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
// Records every observer, so a test can report a row's new height the way the browser would.
const observers = new Set();
dom.window.ResizeObserver = class {
  constructor(cb) { this.cb = cb; this.targets = new Set(); observers.add(this); }
  observe(t) { this.targets.add(t); }
  unobserve(t) { this.targets.delete(t); }
  disconnect() { this.targets.clear(); observers.delete(this); }
};
g.ResizeObserver = dom.window.ResizeObserver;
const resize = (el, h) => {
  for (const o of [...observers]) if (o.targets.has(el)) o.cb([{ target: el, contentRect: { height: h }, borderBoxSize: [{ blockSize: h }] }], o);
};
if (!g.CSS) g.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { ChatView } from './src/components/ChatView';
`);

const messages = [];
for (let i = 0; i < 6; i++) {
  messages.push({ id: `u${i}`, sender_role: 'user', sender_id: 'me', text: `Question ${i}`, created_at: `2026-11-19T14:0${i}:00Z`, tokens_used: 0 });
  messages.push({ id: `a${i}`, sender_role: 'kwant', sender_id: 'Mia', text: `Answer ${i}`, created_at: `2026-11-19T14:0${i}:05Z`, tokens_used: 0 });
}
const base = {
  title: 'Mia', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage() {}, onResolveApproval() {}, kwants: [], teams: [], onHighlightDone() {},
};

function render(w) {
  width = w;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.ChatView, base)));
  const rows = [...host.querySelectorAll('[role="log"] [data-message-id]')];
  return { rows, done: () => { root.unmount(); host.remove(); } };
}

for (const w of [1440, 390]) {
  test(`33q: at ${w} px every chat row wrapper is kw-row-lazy and draws only the avatar that width shows`, () => {
    const { rows, done } = render(w);
    assert.equal(rows.length, messages.length, 'one row per message');
    for (const r of rows) {
      assert.ok(r.classList.contains('kw-row-lazy'), `row ${r.getAttribute('data-message-id')} has kw-row-lazy: "${r.className}"`);
      const id = r.getAttribute('data-message-id');
      const avatars = r.querySelectorAll('.kw-avatar');
      // Below `sm` your own rows show no avatar (as before: the gutter is hidden and the name-line avatar is the bot's).
      const want = w < 640 && id.startsWith('u') ? 0 : 1;
      assert.equal(avatars.length, want, `row ${id} draws ${avatars.length} avatars, wants ${want} (both widths' avatars behind CSS was the cost)`);
    }
    done();
  });
}

test('33q: kw-row-lazy skips far rows with content-visibility, only with scroll anchoring, and not while hovered, focused or flashing', () => {
  const css = fs.readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const rule = /\.kw-row-lazy([^{]*)\{([^}]*)\}/.exec(css);
  assert.ok(rule, 'modern.css has a .kw-row-lazy rule');
  // Without scroll anchoring (WebKit) rows drawn above the screen grow from the estimate and move what is on screen.
  const before = css.slice(0, rule.index);
  const open = before.lastIndexOf('@supports (overflow-anchor: auto) {');
  assert.ok(open >= 0 && !before.slice(open).includes('}'), 'the rule sits inside @supports (overflow-anchor: auto)');
  const [, sel, body] = rule;
  for (const not of [':not(:hover)', ':not(:focus-within)', ':not(.kw-flash)', ':not(.kw-flash-quote)']) {
    assert.ok(sel.includes(not), `the rule leaves out ${not}: containment would clip the row's menus and its flash ring`);
  }
  assert.match(body, /content-visibility:\s*auto/);
  assert.match(body, /contain-intrinsic-size:\s*auto\s+\d+px/, 'rows remember their last real height (auto) after the first draw');
});

test('33q: a smooth jump is not pulled back to the bottom by a row settling, and lands again on real heights when it ends', () => {
  width = 1440;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const calls = [];
  const proto = dom.window.HTMLElement.prototype;
  const was = proto.scrollIntoView;
  proto.scrollIntoView = function (o) { calls.push({ id: this.getAttribute('data-message-id'), behavior: o?.behavior }); };
  try {
    m.flushSync(() => root.render(m.React.createElement(m.ChatView, base)));
    const log = host.querySelector('[role="log"]');
    let pins = 0;
    Object.defineProperty(log, 'scrollHeight', { configurable: true, get: () => 5000 });
    Object.defineProperty(log, 'scrollTop', { configurable: true, get: () => 0, set: (v) => { if (v === 5000) pins++; } });
    const row = log.querySelector('[data-message-id="a5"]');
    const old = log.querySelector('[data-message-id="a1"]');
    resize(row, 160);
    resize(row, 300);
    assert.ok(pins > 0, 'following the bottom, a row that changes height re-pins (the chart and lazy rows settle after the first pin)');
    pins = 0;
    resize(row, 300);
    assert.equal(pins, 0, 'the same height again (a fresh observation) does not re-pin');
    m.flushSync(() => root.render(m.React.createElement(m.ChatView, { ...base, highlightMessageId: 'a1' })));
    assert.deepEqual(calls.at(-1), { id: 'a1', behavior: 'smooth' }, 'the search hit scrolls smoothly');
    resize(old, 160);
    resize(old, 420);
    resize(row, 360);
    assert.equal(pins, 0, 'rows settling while the jump runs never pull the view back to the bottom');
    log.getBoundingClientRect = () => ({ top: 0, bottom: 600, left: 0, right: 0, width: 0, height: 600 });
    old.getBoundingClientRect = () => ({ top: 200, bottom: 300, left: 0, right: 0, width: 0, height: 100 });
    log.dispatchEvent(new dom.window.Event('scrollend'));
    assert.deepEqual(calls.at(-1), { id: 'a1', behavior: 'auto' }, 'when the smooth scroll ends, it lands once more on the real heights');
    resize(row, 400);
    assert.ok(pins > 0, 'after the jump, the re-pin works again (only while following)');
    m.flushSync(() => root.render(m.React.createElement(m.ChatView, { ...base, highlightMessageId: 'a2' })));
    const n = calls.length;
    const away = log.querySelector('[data-message-id="a2"]');
    away.getBoundingClientRect = () => ({ top: -900, bottom: -800, left: 0, right: 0, width: 0, height: 100 });
    log.getBoundingClientRect = () => ({ top: 0, bottom: 600, left: 0, right: 0, width: 0, height: 600 });
    log.dispatchEvent(new dom.window.Event('scrollend'));
    assert.equal(calls.length, n, 'a reader who scrolled away during the jump is not pulled back by the settle step');
  } finally {
    proto.scrollIntoView = was;
    root.unmount();
    host.remove();
  }
});
