// 14b smooth-streaming: the reply streams into React at most once per frame, and finished messages don't
// re-render while it does. The store's batching and flushes, then a ChatView render count in JSDOM.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
if (!g.CSS) g.CSS = { escape: (s) => s };
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.IntersectionObserver = g.window.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
g.window.HTMLElement.prototype.scrollIntoView = function () {};
g.fetch = g.window.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });

// React DevTools' hook, reduced to counting which components rendered in each commit. A fiber rendered when
// it is new or carries PerformedWork; a subtree whose child list is unchanged bailed out and is skipped.
const renders = [];
g.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
  supportsFiber: true,
  isDisabled: false,
  renderers: new Map(),
  inject() {
    return 1;
  },
  onCommitFiberRoot(_id, root) {
    const walk = (next) => {
      const prev = next.alternate;
      if (typeof next.type === 'function' && (!prev || next.flags & 1)) renders.push({ name: next.type.name, props: next.memoizedProps });
      if (prev && next.child === prev.child) return;
      for (let c = next.child; c; c = c.sibling) walk(c);
    };
    walk(root.current);
  },
  onCommitFiberUnmount() {},
  onPostCommitFiberRoot() {},
  checkDCE() {},
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/streamText';
  export { ChatView } from './src/components/ChatView';
`);
const { React, createRoot, flushSync, createStreamText } = m;
const h = React.createElement;

/** A frame clock the test advances by hand. */
function frames() {
  let cb = null;
  let id = 0;
  return {
    frame: (f) => ((cb = f), ++id),
    cancelFrame: () => (cb = null),
    tick() {
      const f = cb;
      cb = null;
      f?.();
    },
    get waiting() {
      return cb !== null;
    },
  };
}

test('tokens before a frame land together, once, with one blip', () => {
  const clock = frames();
  const blips = [];
  const s = createStreamText({}, { ...clock, onFrame: (prev, next) => blips.push([prev.text, next.text]) });
  let seen = 0;
  s.subscribe(() => seen++);
  let buf = '';
  for (const t of ['Hel', 'lo', ' wor', 'ld']) {
    buf += t;
    s.schedule(() => ({ text: buf }));
  }
  assert.equal(s.get().text, '', 'nothing shows before the frame');
  assert.equal(seen, 0);
  clock.tick();
  assert.equal(s.get().text, 'Hello world', 'the frame shows the latest text, not the first');
  assert.equal(seen, 1, 'one update for four tokens');
  assert.deepEqual(blips, [['', 'Hello world']], 'one blip per frame');
  clock.tick();
  assert.equal(seen, 1, 'an idle frame changes nothing');
});

test('the end of the stream and Stop flush at once; a reset or chat switch drops what was waiting', () => {
  const clock = frames();
  const s = createStreamText({}, clock);
  s.schedule(() => ({ text: 'Final words' }));
  s.flush();
  assert.equal(s.get().text, 'Final words', 'flush applies without waiting for the frame');
  assert.equal(clock.waiting, false, 'and cancels the frame');
  s.schedule(() => ({ text: 'Final words and more' }));
  s.show({ text: '', thinking: '' });
  clock.tick();
  assert.deepEqual(s.get(), { text: '', thinking: '' }, 'show cancels the waiting update');
  s.show({ text: 'Other chat', thinking: 'Reading' });
  assert.deepEqual(s.get(), { text: 'Other chat', thinking: 'Reading' }, 'a chat switch shows its buffer now');
});

test('without animation frames (a hidden tab) the timer still moves the stream', async () => {
  const s = createStreamText({}, { frame: () => 1, cancelFrame: () => {} });
  s.schedule(() => ({ thinking: 'Checking the logs' }));
  assert.equal(s.get().thinking, '');
  await new Promise((r) => setTimeout(r, m.FALLBACK_MS + 30));
  assert.equal(s.get().thinking, 'Checking the logs');
});

const conversation = Array.from({ length: 12 }, (_, i) =>
  i % 2
    ? { id: `k${i}`, sender_role: 'kwant', sender_id: 'Ada', text: `## Step ${i}\n\nDone with **part ${i}**.\n\n\`\`\`ts\nconst x = ${i};\n\`\`\`\n\n- one\n- two` }
    : { id: `u${i}`, sender_role: 'user', sender_id: 'PlayerHero', text: `Next, please (${i})` },
);
const isMarkdown = (r) => /^MarkdownRenderer\d*$/.test(r.name);

test('streaming 100 tokens re-renders no finished message and the live reply at most once per frame', () => {
  const clock = frames();
  const stream = createStreamText({}, clock);
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const props = {
    title: 'Ada',
    messages: conversation,
    stream,
    pendingApprovals: [],
    isStreaming: true,
    onSendMessage: () => true,
    onResolveApproval: () => {},
    contextKey: 'k1',
  };
  flushSync(() => root.render(h(m.ChatView, props)));
  assert.ok(renders.filter(isMarkdown).length >= 6, 'the finished replies render through MarkdownRenderer');

  try {
    renders.length = 0;
    let buf = '';
    let shown = 0;
    for (let i = 0; i < 100; i++) {
      buf += i % 7 === 6 ? '\n\n' : `w${i} `;
      stream.schedule(() => ({ text: buf }));
      // 50 tokens a second against 60 frames a second is about one token a frame; four per frame is the
      // harder case for batching and still has to come out as one render per frame.
      if (i % 4 === 3) {
        flushSync(() => clock.tick());
        shown++;
      }
    }
    const finished = renders.filter((r) => isMarkdown(r) && !r.props.isStreaming);
    // 15a: the live reply renders a block at a time. Its open block (the one with the cursor) renders at most
    // once a frame; a block that finished renders once more without the cursor, and never again.
    const live = renders.filter((r) => isMarkdown(r) && r.props.isStreaming && r.props.cursor !== false);
    const closed = renders.filter((r) => isMarkdown(r) && r.props.isStreaming && r.props.cursor === false);
    const blocks = Number(host.querySelector('[data-stream-blocks]').getAttribute('data-stream-blocks'));
    const chat = renders.filter((r) => r.name === 'ChatView');
    assert.equal(shown, 25);
    assert.equal(finished.length, 0, 'finished messages stay as they are');
    assert.ok(live.length <= shown, `live reply rendered ${live.length} times for ${shown} frames`);
    assert.ok(live.length >= shown - 1, 'and it did render each frame');
    assert.ok(blocks > 5, `${blocks} blocks`);
    assert.ok(closed.length <= blocks - 1, `${closed.length} renders of ${blocks - 1} finished blocks`);
    assert.ok(chat.length <= 1, `ChatView rendered ${chat.length} times: only when the reply first has text`);
    assert.match(host.textContent, /w98/, 'the last token is on screen');
  } finally {
    flushSync(() => root.unmount());
  }
});

test('without a stream store, the streamingText prop still shows the reply (older callers and tests)', () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const props = { title: 'Ada', messages: conversation.slice(0, 2), streamingText: 'Working on **it**', streamingThinking: '', pendingApprovals: [], isStreaming: true, onSendMessage: () => true, onResolveApproval: () => {} };
  flushSync(() => root.render(h(m.ChatView, props)));
  assert.match(host.textContent, /Working on it/);
  flushSync(() => root.render(h(m.ChatView, { ...props, streamingText: 'Working on it, done' })));
  assert.match(host.textContent, /Working on it, done/);
  flushSync(() => root.unmount());
});
