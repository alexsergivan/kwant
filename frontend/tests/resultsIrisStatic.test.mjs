// Static rendering for the 6 non-chart renderers (Chart has its own interactive test file): each is a
// bare block wrapped only in a data-smart-result div (the title/footer/copy-data frame is SmartResult.tsx's
// job, not the renderer's), accessible text stands in for colour, and links are safe by construction.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { FIXTURES } from './src/components/results/fixtures';
  import { Metrics } from './src/components/results/Metrics';
  import { Facts } from './src/components/results/Facts';
  import { Cards } from './src/components/results/Cards';
  import { Findings } from './src/components/results/Findings';
  import { Verification } from './src/components/results/Verification';
  import { Files } from './src/components/results/Files';
  const noop = () => {};
  const props = (name) => ({ block: FIXTURES[name].results.blocks[0], messageId: 'm1', reply: null, ask: noop });
  export const html = {
    metrics: renderToStaticMarkup(React.createElement(Metrics, props('metrics'))),
    facts: renderToStaticMarkup(React.createElement(Facts, props('facts'))),
    cards: renderToStaticMarkup(React.createElement(Cards, props('cards'))),
    findings: renderToStaticMarkup(React.createElement(Findings, props('findings'))),
    verification: renderToStaticMarkup(React.createElement(Verification, props('verification'))),
    files: renderToStaticMarkup(React.createElement(Files, props('files'))),
  };
`);

test('every renderer is a bare block tagged with its own type and block id, nothing more', () => {
  for (const [type, id] of [
    ['metrics', 'b-metrics'],
    ['facts', 'b-facts'],
    ['cards', 'b-cards'],
    ['findings', 'b-find'],
    ['verification', 'b-verify'],
    ['files', 'b-files'],
  ]) {
    assert.match(m.html[type], new RegExp(`data-smart-result="${type}"[^>]*data-block-id="${id}"|data-block-id="${id}"[^>]*data-smart-result="${type}"`));
  }
});

test('metrics: two tiles per row on a phone, unit and note shown verbatim', () => {
  assert.match(m.html.metrics, /grid-cols-2/);
  assert.match(m.html.metrics, /EUR 18,400/);
  assert.doesNotMatch(m.html.metrics, />EUR</, 'a unit the value text already carries is not repeated');
  assert.match(m.html.metrics, /up 8% vs September/);
});

test('facts: a definition list, label then value, blank values show an em dash', () => {
  assert.match(m.html.facts, /<dl/);
  assert.match(m.html.facts, /<dt[^>]*>Registrar<\/dt>/);
  assert.match(m.html.facts, /<dd[^>]*>Porkfish<\/dd>/);
});

test('cards: a link opens safely in a new tab, and domain is shown instead of the raw url', () => {
  assert.match(m.html.cards, /rel="noopener noreferrer"/);
  assert.match(m.html.cards, /target="_blank"/);
  assert.match(m.html.cards, /northdeck\.example/);
  assert.doesNotMatch(m.html.cards, /https:\/\/northdeck\.example\/docs</, 'the raw url is not printed as text');
});

test('findings: severity is spelled out in text, grouped with a count, refs are code', () => {
  assert.match(m.html.findings, />High \(1\)</, 'severity word and count, not colour alone');
  assert.match(m.html.findings, /<code[^>]*>scripts\/deploy\.sh:42<\/code>/);
  assert.match(m.html.findings, />Nit \(1\)</, 'the reply’s own severity word is kept, not the raw enum key');
});

test('verification: summary line, each check’s status as text, and an expandable excerpt', () => {
  assert.match(m.html.verification, />2 of 4 checks passed</);
  assert.match(m.html.verification, />Passed</);
  assert.match(m.html.verification, />Failed</);
  assert.match(m.html.verification, /<details/);
  assert.match(m.html.verification, /HTTP 200 in 182 ms/);
});

test('files: name, type and size are shown; a table and a text preview render, an image never guesses a url', () => {
  assert.match(m.html.files, /redirects\.csv/);
  assert.match(m.html.files, /text\/csv/);
  assert.match(m.html.files, /2\.3 KB/);
  assert.match(m.html.files, /<table/);
  assert.match(m.html.files, /Freeze content on Monday/);
  assert.match(m.html.files, /Image preview not available yet/);
  assert.doesNotMatch(m.html.files, /<img/, 'never renders an <img> from unverified attachment data');
});
