// 17c live-name-line: the live reply's name line takes the finished ReplyMeta line's type and height through
// shared class strings, and the finished line's markup is unchanged.
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
  export { ReplyMeta, REPLY_NAME, replyNameLine } from './src/components/ReplyMeta';
`);
const h = m.React.createElement;

test("the shared strings are the finished line's classes", () => {
  assert.equal(m.REPLY_NAME, 'min-w-0 truncate text-[13px] font-semibold text-zinc-50');
  assert.equal(
    m.replyNameLine('sm:flex-wrap'),
    'mb-1.5 flex min-w-0 max-w-full flex-nowrap sm:flex-wrap items-center gap-x-2 gap-y-1 text-xs text-zinc-400 font-medium',
  );
  assert.equal(
    m.replyNameLine('min-h-6'),
    'mb-1.5 flex min-w-0 max-w-full flex-nowrap min-h-6 items-center gap-x-2 gap-y-1 text-xs text-zinc-400 font-medium',
  );
});

test('the finished name line renders exactly as before (b5449933)', () => {
  const out = m.renderToStaticMarkup(h(m.ReplyMeta, { name: 'Juno', avatar: h('i', null, 'a') }));
  assert.ok(
    out.startsWith(
      '<div class="w-full max-w-full min-w-0"><div class="mb-1.5 flex min-w-0 max-w-full flex-nowrap sm:flex-wrap items-center gap-x-2 gap-y-1 text-xs text-zinc-400 font-medium">' +
        '<span class="sm:hidden shrink-0 inline-flex"><i>a</i></span><span class="min-w-0 truncate text-[13px] font-semibold text-zinc-50">Juno</span>',
    ),
    out,
  );
});

test("the live name line uses the shared strings, with the Details button's height", () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(chat, /import \{ FailoverNote, REPLY_NAME, ReplyMeta, replyNameLine \} from '\.\/ReplyMeta';/);
  const liveRow = chat.slice(chat.indexOf('data-live-row=""'), chat.indexOf('data-live-bubble-body=""'));
  assert.match(liveRow, /<div className=\{replyNameLine\('min-h-6'\)\} data-live-name-line="">/);
  assert.match(liveRow, /<span className=\{REPLY_NAME\}>\{liveName\}<\/span>/);
  assert.doesNotMatch(liveRow, /text-\[11px\] text-zinc-400 font-medium|className="text-zinc-200">\{liveName\}/);
  // The chips keep their wording and colours.
  for (const chip of ['In line', 'Waiting for your OK', 'Speaking...', 'text-amber-300 text-[10px]', 'text-sky-400 text-[10px]']) assert.ok(liveRow.includes(chip), chip);
  // ReplyMeta does not import ChatView.
  const meta = readFileSync(new URL('../src/components/ReplyMeta.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(meta, /from '\.\/ChatView'/);
});
