// The bot at work (autonomy/working-avatar/design.md §10): the scene that stands in the live bubble before the first words.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { WorkingAvatar } from './src/components/WorkingAvatar';
  export const scene = (name) => renderToStaticMarkup(React.createElement(WorkingAvatar, { name }));
`);
const scene = ui.scene;
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');
const css = read('src/styles/stream.css');
const tsx = read('src/components/WorkingAvatar.tsx');
const chat = read('src/components/ChatView.tsx');

const body = (html) => html.match(/<rect x="7" y="9" width="12" height="7" fill="([^"]+)"/)?.[1];
const paw = (html) => html.match(/<rect x="13" y="14" width="3" height="1" fill="([^"]+)" data-paw=""/)?.[1];

test('the wrapper is the status with the full label; the scene and the visible line are hidden from screen readers', () => {
  const html = scene('Builder');
  assert.match(html, /^<div class="kw-wa" role="status" aria-label="Builder is working on the answer"/);
  assert.match(html, /<svg class="kw-wa-scene"[^>]*aria-hidden="true"[^>]*focusable="false"/);
  assert.match(html, /<p class="kw-wa-text" aria-hidden="true"><span class="kw-wa-name">Builder<\/span> is working on it<span class="kw-wa-dots">/);
  assert.match(html, /<span class="sr-only">Builder is working on the answer<\/span><\/div>$/);
  assert.doesNotMatch(html, /tabindex|<a |<button|<input/, 'nothing inside takes focus');
  assert.equal((html.match(/class="kw-mood-dot/g) ?? []).length, 3, 'three dots on the mood glyph timing');
});

test('the bot is the real creature, without its ground shadow, and the paw wears its body colour', () => {
  const html = scene('Builder');
  assert.ok(body(html), 'the creature body is drawn');
  assert.equal(paw(html), body(html));
  assert.doesNotMatch(html, /fill="#261d4a"/, 'the portrait shadow is dropped');
  assert.match(html, /<g transform="translate\(-5 -2\)"><g class="kw-wa-bob">/, 'translate outside, bob inside');
});

test('a different speaker draws a different bot (team discussions follow the current speaker)', () => {
  const names = ['Builder', 'Writer', 'Designer', 'Director', 'Scout', 'Atlas', 'Quill', 'Echo'];
  const bodies = new Set(names.map((n) => body(scene(n))));
  assert.ok(bodies.size > 1, 'speakers do not all share one colour');
  for (const n of names) assert.equal(paw(scene(n)), body(scene(n)), `${n}: paw = body`);
  assert.match(chat, /<WorkingAvatar\s+name=\{\s+discussionState\?\.inProgress && discussionState\.currentSpeaker\s+\? discussionState\.currentSpeaker\s+: streamingSpeakerName \|\| title\s+\}/);
});

test('the scene replaces the grey lines; the first words keep its height and fade in once', () => {
  assert.doesNotMatch(chat, /kws-skel|Waiting for the first words/);
  assert.match(chat, /<div className="kw-wa-in kw-wa-hold">\s+<MarkdownRenderer content=\{hideStreamingDecisions\(stripDiscussionMarkers\(streamingText\)\)\} isStreaming=\{true\} \/>/);
  assert.match(css, /\.kw-wa-hold \{ min-height: 62px; \}/);
  assert.match(css, /\.kw-wa \{[^}]*height: 62px;/);
  assert.match(css, /\.kw-wa-scene \{[^}]*flex: none;[^}]*height: 54px;/);
});

test('queued and paused bubbles keep their text and do not hold the scene height', () => {
  for (const s of [
    'Paused on the browser step above. Do it in the browser panel and press Done; this reply carries on from it.',
    'Paused on the question above. Answer it there; this reply carries on from it.',
    'Paused on the approval below. Allow or deny it there; this reply carries on from it.',
  ]) assert.ok(chat.includes(s), s);
  assert.equal((chat.match(/kw-wa-hold/g) ?? []).length, 2, 'only the first-words wrapper (and its comment) name the hold');
  assert.match(chat, /\{\(!phone \|\| queued \|\| streamingText\) && \(/, 'phones keep the strip, no bubble');
});

test('motion is CSS keyframes on transform and opacity only, no JS timers, a 2.4 s loop', () => {
  assert.doesNotMatch(tsx, /setTimeout|setInterval|requestAnimationFrame|useEffect|canvas/);
  const frames = [...css.matchAll(/@keyframes (kw-wa-[\w-]+) \{([\s\S]*?)\n?\}\n/g)];
  assert.ok(frames.length >= 8, `found ${frames.length} keyframes`);
  for (const [, name, b] of frames) {
    const props = [...b.matchAll(/([a-z-]+)\s*:/g)].map((m) => m[1]).filter((p) => p !== 'animation-timing-function');
    for (const p of props) assert.ok(p === 'transform' || p === 'opacity', `${name} animates ${p}`);
  }
  for (const m of css.matchAll(/\.kw-wa-(?:bob|l1|l2|l3|paw) \{ animation: [\w-]+ ([\d.]+)s/g)) assert.ok(Number(m[1]) <= 2.4, m[0]);
});

test('reduced motion stops every animation and leaves the still frame (lines shown, spark hidden, dots "…")', () => {
  const rm = css.match(/@media \(prefers-reduced-motion: reduce\) \{\s+\.kw-wa, \.kw-wa-in, \.kw-wa-scene \*, \.kw-wa-dots > \* \{ animation: none !important; \}\s+\.kw-wa-dots > \* \{ opacity: 0\.8; \}\s+\}/);
  assert.ok(rm, 'reduced-motion block covers the wrapper, the fade, every scene part and the dots');
  assert.match(css, /\.kw-wa-spark rect \{ opacity: 0; \}/);
  const html = scene('Writer');
  for (const w of [11, 9, 6]) assert.match(html, new RegExp(`class="kw-wa-l kw-wa-l\\d" x="19" y="\\d+" width="${w}"`), `line of ${w} cells drawn at rest`);
});

test('light theme tokens exist for every dark token', () => {
  const dark = css.match(/:root \{([^}]*--kw-wa-[^}]*)\}/)[1];
  const light = css.match(/\[data-theme="light"\] \{([^}]*--kw-wa-[^}]*)\}/)[1];
  const keys = (b) => [...b.matchAll(/(--kw-wa-[\w-]+):/g)].map((m) => m[1]);
  for (const k of ['--kw-wa-ground', '--kw-wa-screen', '--kw-wa-deck', '--kw-wa-edge', '--kw-wa-spark', '--kw-wa-spark-core', '--kw-wa-text', '--kw-wa-name']) {
    assert.ok(keys(dark).includes(k) && keys(light).includes(k), k);
  }
});
