// Two contract fixes between the server's Smart Results blocks and the chat:
//  1. a findings block with detail "" (what the server writes for a bullet with no " — detail" part,
//     src/server/smart_results/items.rs `findings`) must validate and draw;
//  2. a "Shared a file" card (meta.kind "file") must draw its files block, without a second file card.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { validFindings } from './src/components/results/Findings';
  export { validFiles } from './src/components/results/Files';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  export const md = (props) => renderToStaticMarkup(React.createElement(MarkdownRenderer, props));
`);

// The block the real server stored for this reply (shape per items.rs: id "b0", ids x0..xN, the whole
// bullet text as title and detail "" when there is no " — "; refs lifted from the backticked path:line).
const REPLY = [
  'Review of the db layer:',
  '',
  '- **High:** SQL built by string concat in `src/db.rs:42`, use a bound parameter',
  '- **Low:** typo in the log line in `src/log.rs:7`',
].join('\n');
const serverFindings = () => ({
  version: 1,
  blocks: [
    {
      id: 'b0',
      type: 'findings',
      title: null,
      source: { kind: 'markdown_list', index: 0, ordered: false },
      provenance: { kind: 'reply_text' },
      items: [
        { id: 'x0', severity: 'high', severity_text: 'High', title: 'SQL built by string concat in `src/db.rs:42`, use a bound parameter', detail: '', refs: [{ path: 'src/db.rs', line: 42 }] },
        { id: 'x1', severity: 'low', severity_text: 'Low', title: 'typo in the log line in `src/log.rs:7`', detail: '', refs: [{ path: 'src/log.rs', line: 7 }] },
      ],
    },
  ],
});

test('findings: the server’s empty detail validates; title and severity stay required', () => {
  const b = serverFindings().blocks[0];
  const ok = m.validFindings(b);
  assert.ok(ok, 'detail "" is valid');
  assert.equal(ok.items[0].detail, '');
  assert.equal(ok.items.length, 2);
  // A missing detail is read as empty, never as undefined.
  const noDetail = { ...b, items: b.items.map(({ detail, ...rest }) => rest) };
  assert.equal(m.validFindings(noDetail)?.items[0].detail, '');
  assert.equal(m.validFindings({ ...b, items: b.items.map((i) => ({ ...i, title: '' })) }), null, 'blank title is still invalid');
  assert.equal(m.validFindings({ ...b, items: b.items.map((i) => ({ ...i, severity: 'urgent' })) }), null, 'severity stays an enum');
  assert.equal(m.validFindings({ ...b, items: b.items.map((i) => ({ ...i, detail: '<b>x</b>' })) }), null, 'HTML in detail is still refused');
  assert.equal(m.validFindings({ ...b, items: b.items.map((i) => ({ ...i, detail: 5 })) }), null, 'a non-string detail is invalid');
});

test('findings: server output round-trips through the renderer, with no empty detail line', () => {
  const html = m.md({ content: REPLY, results: serverFindings(), messageId: 'm1' });
  assert.match(html, /data-smart-result="findings"/, 'the block is drawn, not the markdown fallback');
  assert.match(html, /SQL built by string concat/);
  assert.match(html, /src\/db\.rs:42/);
  assert.doesNotMatch(html, /<p[^>]*text-zinc-400[^>]*><\/p>/, 'no empty paragraph for a blank detail');
});

const ATT = '3f0c1b8e-7d8a-4f6e-9a55-2d1d0c3c9a01';
const fileCard = (over = {}) => ({
  id: 'f1',
  sender_role: 'kwant',
  sender_id: 'Writer',
  text: 'Shared a file: sales.csv',
  meta: JSON.stringify({ kind: 'file', attachments: [{ id: ATT, name: 'sales.csv', mime: 'text/csv', size: 357 }] }),
  results: {
    version: 1,
    blocks: [
      {
        id: 'b0',
        type: 'files',
        title: null,
        source: { kind: 'slot' },
        provenance: { kind: 'kwant_data', ref: `attachment:${ATT}` },
        items: [
          {
            id: 'a0',
            attachment_id: ATT,
            name: 'sales.csv',
            mime: 'text/csv',
            size_bytes: 357,
            preview: { kind: 'table', columns: ['month', 'total'], rows: [['Jan', '12'], ['Feb', '15']], truncated: true, total_rows: 21 },
          },
        ],
      },
    ],
  },
  ...over,
});
const chatProps = (messages) => ({
  title: 'Writer', subtitle: 'Writer', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
});
const count = (html, re) => (html.match(re) ?? []).length;

test('a shared-file card draws the table preview and its cut note, once', () => {
  assert.ok(m.validFiles(fileCard().results.blocks[0]), 'the fixture is a valid files block');
  const html = m.chat(chatProps([fileCard()]));
  assert.match(html, /data-smart-result="files"/, 'the files block is mounted');
  assert.match(html, /<th[^>]*>month<\/th>/);
  assert.match(html, /<td[^>]*>Feb<\/td>/);
  assert.match(html, /data-preview-note/, 'the cut note shows');
  assert.equal(count(html, /aria-label="Download sales\.csv"/g), 1, 'one file tile with one Download button');
  assert.equal(count(html, /sales\.csv/g) >= 1, true);
  // No second file card: the block adds only "Preview of ..." under the tile.
  assert.equal(count(html, /text\/csv ·/g), 0, 'the block does not repeat the type and size line');
  assert.match(html, /Preview of sales\.csv/);
});

test('a shared-file card without a files block, or with an invalid one, is the plain tile as before', () => {
  const plain = m.chat(chatProps([fileCard({ results: undefined })]));
  assert.doesNotMatch(plain, /data-smart-result/);
  assert.equal(count(plain, /aria-label="Download sales\.csv"/g), 1);
  const bad = fileCard();
  bad.results.blocks[0].items[0].preview = { kind: 'table', columns: ['a'], rows: [['x', 'y']] };
  const html = m.chat(chatProps([bad]));
  assert.doesNotMatch(html, /data-smart-result="files"/, 'an invalid block draws nothing');
  assert.equal(count(html, /aria-label="Download sales\.csv"/g), 1, 'the tile is still there');
});

test('a file with no table or text preview adds nothing under its tile', () => {
  const card = fileCard();
  card.results.blocks[0].items[0].preview = null;
  const html = m.chat(chatProps([card]));
  assert.doesNotMatch(html, /data-smart-result="files"/);
  assert.equal(count(html, /aria-label="Download sales\.csv"/g), 1);
});

test('a files block on an ordinary reply is still drawn as full cards (no preview-only mode)', () => {
  const html = m.md({ content: 'Here you go.', results: fileCard().results, messageId: 'm2' });
  assert.match(html, /text\/csv · 357 B/);
  assert.doesNotMatch(html, /data-preview-only/);
});

test('a long cell cannot stretch the shared-file preview past the message (it scrolls in its wrapper)', () => {
  const card = fileCard();
  card.results.blocks[0].items[0].preview = { kind: 'table', columns: ['url'], rows: [['x'.repeat(280)]] };
  const html = m.chat(chatProps([card]));
  // The column is items-start, so the preview, its frame, list and scroll wrapper each cap at the column width.
  assert.match(html, /class="w-full min-w-0 max-w-full"/);
  assert.match(html, /data-smart-frame="files"[^>]*class="[^"]*min-w-0 max-w-full/);
  assert.match(html, /data-smart-result="files"[^>]*class="min-w-0 max-w-full"/);
  assert.match(html, /class="mt-2 min-w-0 max-w-full overflow-x-auto /);
});
