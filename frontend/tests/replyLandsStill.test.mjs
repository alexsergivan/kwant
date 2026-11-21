// 16a reply-lands-still: the live reply row has the finished row's geometry, the typing cursor ends the last
// line of text (its own line after a code block, table or quote), and an ordered list keeps its start number.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'localStorage', 'sessionStorage']) {
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
`);
const { React, renderToStaticMarkup, MarkdownRenderer } = m;
const h = React.createElement;
// The HTML with the newlines between tags dropped.
const html = (content, props = {}) => renderToStaticMarkup(h(MarkdownRenderer, { content, ...props })).replace(/>\s+</g, '><');
const live = (content) => html(content, { isStreaming: true });
const CURSOR = '<span class="inline-block w-1.5 h-4 ml-0.5 align-middle bg-sky-400 animate-pulse motion-reduce:animate-none rounded-sm" aria-hidden="true"></span>';
const count = (s, sub) => s.split(sub).length - 1;

test('the cursor ends the last paragraph, not a line of its own', () => {
  const out = live('First.\n\nThe last line is still');
  assert.equal(count(out, CURSOR), 1);
  assert.ok(out.includes(`The last line is still${CURSOR}</p></div>`), out);
});

test('the cursor ends the last list item (tight, loose, nested, task, ordered)', () => {
  assert.ok(live('- one\n- two is still').includes(`two is still${CURSOR}</li>`));
  assert.ok(live('- one\n\n- two is still').includes(`two is still${CURSOR}</p>`));
  assert.ok(live('1. one\n   - inner is still').includes(`inner is still${CURSOR}</li></ul>`));
  assert.ok(live('- [ ] one\n- [x] two is still').includes(`two is still${CURSOR}</span></li>`));
  assert.ok(live('- one **bold**').includes(`<strong class="font-semibold text-zinc-100">bold</strong>${CURSOR}</li>`));
});

test('the cursor ends a heading', () => {
  assert.ok(live('## Plan so').includes(`Plan so${CURSOR}</h2>`));
});

test('after a fenced code block, a table or a quote the cursor keeps its own line', () => {
  for (const md of ['Run:\n\n```sh\nls -la\n```', '| a | b |\n|---|---|\n| 1 | 2 |', '> quoted line', '- item\n\n  ```\n  code\n  ```']) {
    const out = live(md);
    assert.equal(count(out, CURSOR), 1, md);
    assert.ok(out.endsWith(`${CURSOR}</div>`), `own line after:\n${md}\n${out}`);
  }
  // An empty reply still shows the cursor.
  assert.equal(live(''), `<div class="markdown-content text-sm leading-relaxed text-zinc-200 ">${CURSOR}</div>`);
});

test('no cursor in a finished message or a finished block', () => {
  assert.equal(count(html('Done.\n\n- a\n- b'), 'animate-pulse'), 0);
  assert.equal(count(html('Done.', { isStreaming: true, cursor: false }), 'animate-pulse'), 0);
});

test('an ordered list keeps its start number; a list from 1 renders as before', () => {
  assert.match(html('3. three\n4. four'), /<ol start="3" class="list-decimal/);
  assert.match(html('1. one\n2. two'), /<ol class="list-decimal/);
  assert.match(live('5. five'), /<ol start="5" class="list-decimal/);
});

test('the live row and the finished row share one geometry', () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  // One avatar size, one bubble padding: both rows read the same constants.
  assert.match(chat, /const REPLY_AVATAR = 36;/);
  assert.match(chat, /const REPLY_AVATAR_PHONE = 20;/);
  assert.match(chat, /const REPLY_BUBBLE = 'px-3\.5 py-3 sm:p-5 rounded-2xl text-sm leading-relaxed shadow-sm w-full';/);
  const liveRow = chat.slice(chat.indexOf('data-live-row=""'), chat.indexOf('data-live-bubble-body=""') + 30);
  assert.match(liveRow, /<span className="hidden sm:contents">\s+<ModernAvatar role=\{liveName\} name=\{liveName\} size=\{REPLY_AVATAR\} \/>/);
  assert.match(liveRow, /<span className="sm:hidden shrink-0 inline-flex">\s+<ModernAvatar role=\{liveName\} name=\{liveName\} size=\{REPLY_AVATAR_PHONE\} \/>/);
  assert.match(liveRow, /className=\{`\$\{REPLY_BUBBLE\} min-h-\[3\.5rem\] \$\{REPLY_BUBBLE_BOT\} kw-live-in`\} data-live-bubble-body=""/);
  assert.doesNotMatch(liveRow, /size=\{34\}|p-4 sm:p-5/);
  // The finished row uses the same strings.
  assert.match(chat, /size=\{REPLY_AVATAR\}\s+kwantId=\{replyKwantId\}/);
  assert.match(chat, /size=\{REPLY_AVATAR_PHONE\} kwantId=\{replyKwantId\}/);
  // 25b: the finished row picks its look by side (onYourSide), still from REPLY_BUBBLE
  assert.match(chat, /className=\{`\$\{REPLY_BUBBLE\} \$\{\s+onYourSide/);
  assert.match(chat, /: REPLY_BUBBLE_BOT\s+\}`\}/);
});

test('the stream store lives as long as the app (useState, not useMemo)', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /const \[streamStore\] = useState\(\(\) =>\s+createStreamText\(/);
  assert.doesNotMatch(app, /streamStore = useMemo/);
});
