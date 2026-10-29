import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const p = await loadBundled(`export * from './src/decideFirst';`);
const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');

// The reply from Oleks's #general screenshot, as the channel round saved it
const REPLY = 'I updated the "Daily morning overview" with both. Next I expect you\'ll want a pick-up route.';
const BLOCK =
  '[DECISIONS]\n' +
  'question: Arrival buffer for "Leave by" | options: 5 min; 10 min; 15 min | picked: 10 min | why: leaves time to park and walk in before 08:10\n' +
  'question: Traffic section on days without school | options: Run Maps anyway; One-line "no school run" note | picked: One-line "no school run" note | why: next week is Herbstferien\n' +
  '[/DECISIONS]';

test('a raw block is hidden and its lines become read-only chips', () => {
  const r = p.messageDecisions(`${REPLY}\n\n${BLOCK}`, null);
  assert.equal(r.text, REPLY);
  assert.equal(r.readOnly, true);
  assert.equal(r.decisions.length, 2);
  assert.deepEqual(r.decisions[0], {
    index: 0,
    question: 'Arrival buffer for "Leave by"',
    options: ['5 min', '10 min', '15 min'],
    picked: '10 min',
    why: 'leaves time to park and walk in before 08:10',
    status: 'decided',
    changed_to: null,
  });
  assert.equal(r.decisions[1].picked, 'One-line "no school run" note');
});

test('decisions saved on the message win and stay changeable; the block is still hidden', () => {
  const saved = { index: 0, question: 'Q', options: ['A', 'B'], picked: 'B', why: '', status: 'decided', changed_to: null };
  const r = p.messageDecisions(`${REPLY}\n\n${BLOCK}`, JSON.stringify({ decisions: [saved] }));
  assert.equal(r.text, REPLY);
  assert.equal(r.readOnly, false);
  assert.deepEqual(r.decisions, [saved]);
});

test('two decisions run together on one line are split at "question:"', () => {
  const oneLine = BLOCK.replace('08:10\nquestion:', '08:10 question:');
  const r = p.takeDecisionsBlock(`${REPLY} ${oneLine}`);
  assert.equal(r.text, REPLY);
  assert.deepEqual(r.decisions.map((d) => d.picked), ['10 min', 'One-line "no school run" note']);
});

test('a malformed or unclosed block is hidden without chips, never shown raw', () => {
  for (const bad of [
    '[DECISIONS]\nquestion: Q | options: only one | picked: only one\n[/DECISIONS]',
    '[DECISIONS]\nquestion: Q | options: A; B | picked: C\n[/DECISIONS]',
    '[DECISIONS]\njust some words\n[/DECISIONS]',
    '[DECISIONS]\nquestion: Q | options: A; B',
    '[decisions] question: Q | options: A; B | picked: A [/decisions]',
  ]) {
    const r = p.messageDecisions(`${REPLY}\n\n${bad}`, null);
    assert.equal(r.text, REPLY, bad);
    assert.doesNotMatch(r.text, /DECISIONS/i);
  }
  // The last case is valid apart from the lowercase tags
  assert.equal(p.takeDecisionsBlock('[decisions] question: Q | options: A; B | picked: A [/decisions]').decisions.length, 1);
});

test('text after the block is kept, and normal text is untouched', () => {
  assert.equal(p.takeDecisionsBlock(`Before.\n${BLOCK}\nAfter.`).text, 'Before.\n\nAfter.');
  for (const t of ['Plain reply.', 'A [link](https://x.y) and [brackets]', '']) {
    const r = p.messageDecisions(t, null);
    assert.equal(r.text, t);
    assert.deepEqual(r.decisions, []);
  }
  // At most six chips, like the server
  const many = `[DECISIONS]\n${Array.from({ length: 8 }, (_, i) => `question: Q${i} | options: A; B | picked: A`).join('\n')}\n[/DECISIONS]`;
  assert.equal(p.takeDecisionsBlock(many).decisions.length, 6);
});

test('a block being streamed in is hidden, including a half-typed tag', () => {
  assert.equal(p.hideStreamingDecisions(`${REPLY}\n\n[DECISIONS]\nquestion: Arr`), REPLY);
  assert.equal(p.hideStreamingDecisions(`${REPLY}\n\n[DECIS`), REPLY);
  assert.equal(p.hideStreamingDecisions('See [the docs'), 'See [the docs');
  assert.equal(p.hideStreamingDecisions(REPLY), REPLY);
});

test('the chat renders the cleaned text and keeps client-read chips read-only', () => {
  assert.match(src, /messageDecisions\(msg\.text, msg\.meta\)/);
  assert.match(src, /<MarkdownRenderer content=\{text\} \/>/);
  assert.match(src, /onChangeDecision && !shownDecisions\?\.readOnly/);
  assert.match(src, /hideStreamingDecisions\(stripDiscussionMarkers\(streamingText\)\)/);
  assert.match(src, /handleCopy\(text, msg\.id\)/);
});

test('client-read chips render without a Change button', async () => {
  globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };
  const ui = await loadBundled(`
    import React from 'react';
    import { renderToStaticMarkup } from 'react-dom/server.browser';
    import { DecisionChips } from './src/components/Proactive';
    export const chips = (props) => renderToStaticMarkup(React.createElement(DecisionChips, props));
  `);
  const { decisions } = p.messageDecisions(`${REPLY}\n\n${BLOCK}`, null);
  const html = ui.chips({ messageId: 'm1', decisions });
  assert.match(html, /Decided: 10 min/);
  assert.match(html, /Decided: One-line &quot;no school run&quot; note/);
  assert.doesNotMatch(html, /Change/);
  assert.doesNotMatch(html, /DECISIONS/);
});
