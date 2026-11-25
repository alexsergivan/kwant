// 32e empty-card-never-blinks: the live reply bubble never shows its frame with nothing inside it. The bubble fades
// in as one piece with its content; inside a bubble already on screen, the scene and the first words never start
// from opacity 0; and the scene stays until the text has words the bubble would show (not a bare marker or a
// decisions block being written). The frame-by-frame proof in a browser is in the item's impl.md.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'localStorage', 'sessionStorage']) {
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
dom.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createStreamText, useStreamWriting } from './src/streamText';
  export { stripDiscussionMarkers } from './src/discussionThreads';
  export { hideStreamingDecisions } from './src/decideFirst';
  export { ChatView } from './src/components/ChatView';
`);
const h = m.React.createElement;
const css = readFileSync(new URL('../src/styles/stream.css', import.meta.url), 'utf8');
const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');

const shown = (t) => m.hideStreamingDecisions(m.stripDiscussionMarkers(t));
const writingOf = (text, fn) => {
  const s = m.createStreamText({ text });
  const W = () => String(fn ? m.useStreamWriting(s, fn) : m.useStreamWriting(s));
  return m.renderToStaticMarkup(h(W)) === 'true';
};

test('writing waits for words the bubble would show', () => {
  for (const [text, want] of [
    ['', false],
    ['[PASS]', false],
    ['[ASK_USER] ', false],
    ['[DECIS', false],
    ['Hello', true],
    ['[PASS] Nothing to add from me.', true],
    ['Plan:\n\n[DECISIONS', true],
  ]) assert.equal(writingOf(text, shown), want, JSON.stringify(text));
  // Without a filter it still means "any text", so other callers keep their meaning.
  assert.equal(writingOf('[PASS]'), true);
  assert.equal(writingOf(''), false);
});

// The real ChatView, live: what the bubble holds for a given stream text (desktop, matchMedia false).
const noop = () => {};
const liveBubble = (text) => {
  const stream = m.createStreamText({ text });
  const props = new Proxy(
    { title: 'Ben', subtitle: '', messages: [], pendingApprovals: [], isStreaming: true, streamingSpeakerName: 'Ben', stream, teamMembers: [] },
    { get: (t, k) => (k in t ? t[k] : typeof k === 'string' && k.startsWith('on') ? noop : undefined) },
  );
  const html = m.renderToStaticMarkup(h(m.ChatView, props));
  const at = html.indexOf('data-live-bubble-body=""');
  assert.ok(at > 0, 'the live bubble renders');
  const open = html.lastIndexOf('<div', at);
  // 36d: rest = the whole bubble on: the scene is long, and the words come after it.
  return { tag: html.slice(open, html.indexOf('>', at) + 1), inside: html.slice(at, at + 600), rest: html.slice(at) };
};

test('a marker-only reply keeps the working scene in the bubble (no empty bubble)', () => {
  for (const text of ['[PASS]', '[ASK_USER]', '[DECISIONS']) {
    const b = liveBubble(text);
    assert.match(b.inside, /data-working-avatar=""/, `${text}: the scene stays`);
    assert.doesNotMatch(b.inside, /data-stream-blocks/, `${text}: no empty words block`);
  }
  // 36d: with words the scene stays on top (it is the one working status) and the words stream under it.
  const words = liveBubble('[PASS] Nothing to add from me.');
  assert.match(words.inside, /data-working-avatar=""[^>]*data-writing=""/);
  assert.match(words.rest, /data-scene-words="">[\s\S]*Nothing to add from me\./);
});

test('the bubble itself fades in, so its frame and what is inside it appear together', () => {
  assert.match(liveBubble('').tag, /class="[^"]*\bkw-live-in\b[^"]*"/);
  const anim = (sel) => css.match(new RegExp(`(?:^|\\n)${sel.replace('.', '\\.')} \\{[^}]*animation: ([\\w-]+) `))?.[1];
  const fromOpacity = (name) => Number(css.match(new RegExp(`@keyframes ${name} \\{ from \\{ opacity: ([\\d.]+); \\}`))?.[1]);
  assert.equal(anim('.kw-live-in'), 'kw-wa-in');
  assert.equal(fromOpacity('kw-wa-in'), 0, 'the bubble comes up from nothing');
  // Inside it, nothing starts from 0 (a frame of an empty bubble). 36d: the scene row no longer fades on its own
  // (no swap: it stays above the words), so only the half-fade class is left.
  assert.ok(!anim('.kw-wa'), 'the scene row does not fade by itself');
  for (const sel of ['.kw-wa-in']) {
    const name = anim(sel);
    assert.ok(name, `${sel} has an animation`);
    assert.ok(fromOpacity(name) >= 0.5, `${sel} starts at ${fromOpacity(name)}`);
  }
  // Reduced motion: no fade at all, the resting frame.
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s+[^{]*\.kw-live-in[^{]*\{ animation: none !important; \}/);
});

test('ChatView decides writing from the shown words, the same text StreamPart draws', () => {
  assert.match(chat, /useStreamWriting\(stream, shownWords\)/);
  assert.match(chat, /<StreamMarkdown content=\{shownWords\(streamingText\)\} \/>/);
  assert.match(chat, /!streamProp && !!shownWords\(streamingTextProp\)\.trim\(\)/);
});
