// 30e (autonomy/details-no-echo-reasoning/brief.md): a CLI reply's only "reasoning" is its run line,
// "Executed via claude CLI · This machine [Effort: medium]" (src/drivers/cli.rs executed_via). Details stops showing it
// as a reasoning disclosure and keeps the effort in words next to Ran on. Real reasoning shows as before.
// Every test here fails on main 7f76f171 / bd8740d1 (no src/reasoningEcho.ts, no effort on ReplyMeta).
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

let m = {};
let loadError = null;
try {
  m = await loadBundled(`
    import React from 'react';
    import { flushSync } from 'react-dom';
    export { createRoot } from 'react-dom/client';
    export { ReplyMeta } from './src/components/ReplyMeta';
    export { cliRunLineEffort, isCliRunLine, effortWords } from './src/reasoningEcho';
    export { React, flushSync };
  `);
} catch (e) {
  loadError = e;
}
const need = () => assert.equal(loadError, null, `src/reasoningEcho.ts loads (${loadError?.message?.split('\n')[0]})`);

const CLI = 'Executed via claude CLI · This machine [Effort: medium]';

test('the CLI run line alone is hidden; its effort is kept', () => {
  need();
  assert.equal(m.isCliRunLine(CLI), true);
  assert.equal(m.isCliRunLine(`  ${CLI}\n`), true, 'surrounding whitespace is still just the line');
  assert.equal(m.isCliRunLine('Executed via codex CLI · Work account [Effort: high]'), true);
  assert.equal(m.cliRunLineEffort(CLI), 'medium');
  assert.equal(m.effortWords(CLI), 'medium effort');
  assert.equal(m.effortWords('Executed via grok CLI · The Team account [Effort: High]'), 'high effort');
});

test('real reasoning, a multi-line text and an API reply’s reasoning still show', () => {
  need();
  const shows = [
    'I compared both prices and picked the cheaper shop.',
    `${CLI}\nThen I read the file and found the bug.`,
    `Thinking first.\n${CLI}`,
    '🔮 The user wants a short answer, so I keep it to two lines.',
    'Executed via claude CLI · This machine',
    'Executed via claude CLI · This machine [Effort: medium] and then more',
    '',
    null,
    undefined,
  ];
  for (const r of shows) {
    assert.equal(m.isCliRunLine(r), false, JSON.stringify(r));
    assert.equal(m.effortWords(r), null, `no effort is invented for ${JSON.stringify(r)}`);
  }
});

const mount = (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.ReplyMeta, props)));
  return { host, unmount: () => (root.unmount(), host.remove()) };
};
const tick = () => new Promise((r) => setTimeout(r, 0));
const openDetails = async (host) => {
  const toggle = [...host.querySelectorAll('button')].find((b) => /Details/.test(b.textContent));
  toggle.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await tick();
  await tick();
  const panel = host.querySelector('[data-reply-details]');
  return Object.fromEntries([...panel.querySelectorAll('dt')].map((dt) => [dt.textContent, dt.nextElementSibling.textContent]));
};

test('Details: Ran on carries the effort in words; without one it reads as before', async () => {
  need();
  const withEffort = mount({ name: 'Mia', ranOn: 'Claude · Opus 5.5', account: null, effort: m.effortWords(CLI) });
  assert.equal((await openDetails(withEffort.host))['Ran on'], 'Claude Opus 5.5 · medium effort');
  withEffort.unmount();
  const api = mount({ name: 'Mia', ranOn: 'Claude · Opus 5.5', account: 'Work account', effort: m.effortWords('I thought about it.') });
  assert.equal((await openDetails(api.host))['Ran on'], 'Claude Opus 5.5, on your Work account');
  api.unmount();
});

test('ChatView hides the disclosure only for the run line and passes the effort to ReplyMeta', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(src, /\{msg\.reasoning_content && !isCliRunLine\(msg\.reasoning_content\) && !hasThinkingText\(savedActivity\) && \(/);
  assert.match(src, /effort=\{effortWords\(msg\.reasoning_content\)\}/);
});
