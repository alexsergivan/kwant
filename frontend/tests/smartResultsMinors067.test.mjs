// Client-side Smart Results minors left from 0.6.5 QA (Rigel):
//  more-M4   a card link with credentials in the URL is refused
//  typed-M3  a long (server-cut) table cell shows an ellipsis and its whole text on hover and focus
//  renderers-D4  a chart caption that the reply's own text already says is not drawn a second time
//  renderers-D6  "Preview, file is longer" only when the total is larger than what is shown
//  typed-M2 (a 20-line file plus blank lines reads "truncated, 22 lines") is decided in the server's
//  line count (src/server/smart_results/files.rs `excerpt`); the client cannot tell blank lines from
//  text it never received, so it is reported to Vega instead of papered over here.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { isHttpUrl, previewNote, repeatsReply } from './src/smartResults';
  export { validCards } from './src/components/results/Cards';
  export { Cell, LONG_CELL } from './src/components/results/Files';
  export { FIXTURES } from './src/components/results/fixtures';
  export const md = (props) => renderToStaticMarkup(React.createElement(MarkdownRenderer, props));
  export const html = (el) => renderToStaticMarkup(el);
  export { React };
`);

const count = (s, re) => (s.match(re) ?? []).length;

// ---- more-M4 -------------------------------------------------------------------------------------
test('isHttpUrl: a URL with a username or password is not a link', () => {
  assert.equal(m.isHttpUrl('https://user:pw@example.com/docs'), false, 'c08');
  assert.equal(m.isHttpUrl('https://user@example.com/docs'), false, 'username only');
  assert.equal(m.isHttpUrl('http://:pw@example.com/'), false, 'password only');
  assert.equal(m.isHttpUrl('https://example.com@evil.test/'), false, 'a host spoof through userinfo');
  assert.equal(m.isHttpUrl('https://example.com/docs?next=https://a@b.test'), true, '@ in the query is not userinfo');
  assert.equal(m.isHttpUrl('https://example.com/a@b'), true, '@ in the path is not userinfo');
  assert.equal(m.isHttpUrl('https://example.com/docs'), true);
  assert.equal(m.isHttpUrl('http://example.com:8080/x'), true);
  assert.equal(m.isHttpUrl('javascript:alert(1)'), false);
});

test('cards: a link with credentials makes the block fall back, so no link is drawn', () => {
  const b = JSON.parse(JSON.stringify(m.FIXTURES.cards.results.blocks[0]));
  assert.ok(m.validCards(b), 'the fixture is valid');
  const bad = JSON.parse(JSON.stringify(b));
  bad.items[0].url = 'https://user:pw@example.com/docs';
  assert.equal(m.validCards(bad), null);
});

// ---- typed-M3 ------------------------------------------------------------------------------------
test('a long table cell: one line, an ellipsis, the whole text on hover and focus; a short cell is unchanged', () => {
  const long = 'x'.repeat(299) + '…';
  const h = m.html(m.React.createElement(m.Cell, { text: long }));
  assert.match(h, /data-long-cell/);
  assert.match(h, /\btruncate\b/, 'clipped with a CSS ellipsis');
  assert.ok(h.includes(`title="${long}"`), 'the whole text on hover');
  assert.match(h, /tabindex="0"/i, 'reachable by keyboard');
  assert.match(h, /focus:whitespace-normal/, 'opens out on focus');
  assert.ok(h.includes(`>${long}<`), 'the whole text stays in the DOM for a screen reader');
  assert.equal(m.html(m.React.createElement(m.Cell, { text: 'short' })), 'short');
  assert.equal(m.html(m.React.createElement(m.Cell, { text: 'y'.repeat(m.LONG_CELL) })), 'y'.repeat(m.LONG_CELL), 'at the limit is not clipped');
});

test('a files preview with a 300-character cell draws it clipped, once per long cell', () => {
  const long = 'z'.repeat(299) + '…';
  const block = JSON.parse(JSON.stringify(m.FIXTURES.files.results.blocks[0]));
  block.items = [
    { id: 'a0', attachment_id: 'att', name: 'long.csv', mime: 'text/csv', size_bytes: 900, preview: { kind: 'table', columns: ['id', 'note'], rows: [['1', long], ['2', 'ok']] } },
  ];
  const h = m.md({ content: 'Here it is.', results: { version: 1, blocks: [block] }, messageId: 'm1' });
  assert.equal(count(h, /data-long-cell/g), 1);
  assert.ok(h.includes(`title="${long}"`));
});

// ---- renderers-D4 --------------------------------------------------------------------------------
test('repeatsReply: loose match on letters and digits, only for a sentence-sized caption', () => {
  const reply = 'Signups kept climbing.\n\nOctober had 2,260 signups and 455 paid.';
  assert.equal(m.repeatsReply('October had 2,260 signups and 455 paid.', reply), true);
  assert.equal(m.repeatsReply('october had 2260 signups and 455 paid', reply), true, 'punctuation and case do not matter');
  assert.equal(m.repeatsReply('Signups doubled since July.', reply), false);
  assert.equal(m.repeatsReply('Paid', 'Paid signups grew'), false, 'a short caption is not matched');
  assert.equal(m.repeatsReply('', reply), false);
  assert.equal(m.repeatsReply(null, reply), false);
  assert.equal(m.repeatsReply('October had 2,260 signups', ''), false);
});

test('chart card: the summary caption is left out when the reply says the same sentence, and kept when it does not', () => {
  const fx = m.FIXTURES.chartLine;
  const SENT = 'October had 2,260 signups and 455 paid.';
  const same = m.md({ content: fx.reply, results: fx.results, messageId: 'm1' });
  assert.match(same, /data-smart-frame="chart"/);
  // The reply's closing sentence is the only visible copy; the svg <desc> keeps the summary for screen readers.
  const visible = same.replace(/<desc[^>]*>.*?<\/desc>/g, '');
  assert.equal(count(visible, new RegExp(SENT.replace(/[.]/g, '\\.'), 'g')), 1, 'drawn once');
  // A different caption stays.
  const other = JSON.parse(JSON.stringify(fx.results));
  other.blocks[0].summary = 'Paid signups grew every month.';
  const kept = m.md({ content: fx.reply, results: other, messageId: 'm1' });
  assert.match(kept, /<p[^>]*>Paid signups grew every month\.<\/p>/);
  // A caption the reply does not repeat at all still shows when the reply is different.
  const lone = m.md({ content: 'Here is the trend.\n\n' + fx.reply.split('\n\n')[1], results: fx.results, messageId: 'm1' });
  assert.match(lone, /<p[^>]*>October had 2,260 signups and 455 paid\.<\/p>/, 'no repeat in the reply: the caption stays');
});

// ---- renderers-D6 --------------------------------------------------------------------------------
test('previewNote: "Preview, file is longer" only when the preview is cut', () => {
  const t = (cut) => ({ kind: 'table', columns: ['a', 'b'], rows: [['1', '2'], ['3', '4']], ...cut });
  const x = (cut, excerpt = 'one\ntwo') => ({ kind: 'text', excerpt, ...cut });
  assert.equal(m.previewNote(t({ total_rows: 2 })), null, 'total equals the rows shown');
  assert.equal(m.previewNote(t({ total_rows: 1 })), null, 'total below the rows shown');
  assert.equal(m.previewNote(t({ total_rows: 2, total_cols: 2 })), null);
  assert.equal(m.previewNote(x({ total_lines: 2 })), null, 'total_lines equals the lines shown');
  assert.equal(m.previewNote(x({ total_lines: 1 })), null);
  assert.equal(m.previewNote(t({})), null);
  assert.equal(m.previewNote(t({ total_rows: 143 })), 'Showing 2 of 143 rows');
  assert.equal(m.previewNote(x({ total_lines: 512 })), 'Showing 2 of 512 lines');
  assert.equal(m.previewNote(t({ truncated: true })), 'Preview, file is longer');
  assert.equal(m.previewNote(x({ truncated: true })), 'Preview, file is longer');
});
