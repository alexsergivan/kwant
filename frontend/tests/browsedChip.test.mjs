import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { BrowsedChip } from './src/components/BrowsedChip';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const step = (tool, detail, at = '2026-11-13T10:45:00Z') => ({ id: at + detail, kind: 'tool', status: 'ok', title: 'Opening', tool, detail, started_at: at });
const chip = (props) => m.renderToStaticMarkup(m.createElement(m.BrowsedChip, { botName: 'Ben', live: false, onOpen: () => {}, ...props }));

test('no chip when the reply opened no pages in the bot browser', () => {
  assert.equal(chip({ steps: [] }), '');
  assert.equal(chip({ steps: undefined }), '');
  assert.equal(chip({ steps: [step('WebFetch', 'https://a.example.com/')] }), '');
});

test('after the reply: "Ben opened 1 page" / "2 pages", a button that says what it shows', () => {
  const one = chip({ steps: [step('mcp__kwnt__browser_open', 'https://roastco.example.com/')] });
  assert.match(one, /^<button[^>]*type="button"/);
  assert.match(one, /Ben opened 1 page</);
  assert.match(one, /aria-label="Show the page Ben opened"/);
  assert.match(one, /data-browsed-chip="finished"/);
  const two = chip({ steps: [step('browser_open', 'https://roastco.example.com/'), step('browser_navigate', 'https://roastco.example.com/cart')] });
  assert.match(two, /Ben opened 2 pages</);
  assert.match(two, /aria-label="Show the 2 pages Ben opened"/);
});

test('while browsing: the host now, and the count only from 2 pages', () => {
  const one = chip({ live: true, steps: [step('browser_open', 'https://www.roastco.example.com/shop')] });
  assert.match(one, /Ben is on<\/span><span[^>]*>roastco\.example\.com</);
  assert.doesNotMatch(one, /pages/);
  assert.match(one, /aria-label="Watch Ben&#x27;s browser"/);
  assert.match(one, /motion-reduce:animate-none/);
  const two = chip({ live: true, steps: [step('browser_open', 'https://roastco.example.com/'), step('browser_open', 'https://beans.example.org/', '2026-11-13T10:46:00Z')] });
  assert.match(two, /beans\.example\.org<\/span><span[^>]*>· 2 pages</);
});
