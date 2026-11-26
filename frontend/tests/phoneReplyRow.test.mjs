// 19f phone-reply-row: on a phone a reply's buttons sit in two tidy rows, no empty browsed row, and the 14 px
// Ask-chip avatars get a 6 px dot instead of the mood plate.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { AvatarMoodMark } from './src/components/AvatarMood';
  export { evidenceClass } from './src/components/results/SmartActions';
  export { pagesOfReply } from './src/browserPages';
  export const mark = (mood, size) => renderToStaticMarkup(React.createElement(AvatarMoodMark, { mood, size }));
`);
const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');
const chat = read('src/components/ChatView.tsx');
const css = read('src/styles/modern.css');

test('the phone browsed row renders only when the reply opened pages', () => {
  assert.match(chat, /phone && savedActivity && pagesOfReply\(savedActivity\.steps\)\.length > 0 && \(\s*<div className="mt-1 flex items-center min-h-\[44px\][^"]*" data-phone-browsed-row/);
  assert.equal(m.pagesOfReply([]).length, 0);
  assert.equal(m.pagesOfReply(null).length, 0);
});

test('on a phone the ghost row does not wrap and the smart actions get their own row after it', () => {
  const at = chat.indexOf('data-reply-actions=');
  assert.ok(at > 0);
  const row = chat.slice(at - 300, at + 60);
  assert.match(row, /\$\{phone \? 'flex-nowrap' : 'flex-wrap'\}/);
  const tail = chat.slice(at, at + 2500);
  assert.match(tail, /\{!phone && msg\.results != null && <SmartActionsSlot messageId=\{msg\.id\} \/>\}\s*<\/div>/);
  assert.match(tail, /\{phone && msg\.results != null && \(\s*<div className="mt-1 flex flex-wrap items-center gap-2[^"]*" data-smart-row="">\s*<SmartActionsSlot messageId=\{msg\.id\} \/>/);
  assert.ok(tail.indexOf('data-smart-row') > tail.indexOf('<CopyGhost'), 'the smart row comes after the ghost row');
  assert.match(css, /\[data-smart-row\]:has\(> \[data-smart-actions\]:empty\) \{\s*display: none;/);
});

test('the source line ("From task checks") is a quiet caption at every width, not a chip', () => {
  // no phone-only override any more: phone and desktop share evidenceClass
  assert.doesNotMatch(css, /\[data-evidence\]/);
  const cls = m.evidenceClass.split(' ');
  for (const chip of ['border', 'border-hairline', 'rounded-lg', 'px-3', 'min-h-9', 'font-medium']) {
    assert.ok(!cls.includes(chip), `no chip class ${chip}`);
  }
  assert.ok(!cls.some((c) => c.startsWith('bg-') || c.startsWith('hover:') || c.startsWith('active:')), 'no background or hover');
  for (const quiet of ['text-zinc-500', 'font-normal', 'cursor-default', 'items-center', 'self-center', 'min-w-0', '[&>svg]:text-zinc-500']) {
    assert.ok(cls.includes(quiet), `has ${quiet}`);
  }
});

test('a 14 px avatar that needs you gets the 6 px dot, not the 7×7 plate; 24 px keeps the plate', () => {
  const small = m.mark('needs_you', 14);
  assert.match(small, /data-mood-dot/);
  assert.match(small, /width:6px;height:6px/);
  assert.match(small, /#ffb020/);
  // not .kw-mood-dot: that class is the thinking dots' 0.25–1 opacity pulse
  assert.doesNotMatch(small, /class="kw-mood-dot\b/);
  assert.doesNotMatch(small, /viewBox="0 0 7 7"/);
  assert.equal(m.mark('thinking', 14), '');
  assert.equal(m.mark('failed', 19), '');
  const big = m.mark('needs_you', 24);
  assert.match(big, /viewBox="0 0 7 7"/);
  assert.match(big, /width="14" height="14"/);
  assert.doesNotMatch(big, /data-mood-dot/);
  assert.match(m.mark('needs_you', 20), /viewBox="0 0 7 7"/);
  assert.match(read('src/components/ModernAvatar.tsx'), /<AvatarMoodMark mood=\{mood\} size=\{size\} \/>/);
});
