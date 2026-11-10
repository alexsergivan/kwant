// 0.8.1 F4 (Rigel, qa-final-080 "0.8.x" item 3): a page opened while a bot replies read the running turns
// before the bot list arrived, so the working strip kept the chat's title ("@QATeam is getting started")
// for the whole turn. It now remembers which bot runs in each chat and names it once the list loads; and a
// queued turn in the same chat never names the strip over the running one.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { QueuedBeside } from './src/components/QueuedBeside';
  export * from './src/workingStrip';
  export const beside = (items) => renderToStaticMarkup(React.createElement(QueuedBeside, { items }));
`);

const bots = { 'k-ann': 'Ann', 'k-bea': 'Beatrix' };
const nameOf = (id) => bots[id];

test('a chat marked live before the bot list loaded gets its bot name once the list is there', () => {
  assert.deepEqual(m.namesToFill({ 'ch-qa': '' }, { 'ch-qa': 'k-ann' }, nameOf), [['ch-qa', 'Ann']]);
});

test('a chat that already shows a name keeps it', () => {
  assert.deepEqual(m.namesToFill({ 'ch-qa': 'Laya' }, { 'ch-qa': 'k-ann' }, nameOf), []);
});

test('nothing to fill while the bot is still unknown, or with no bot id for the chat', () => {
  assert.deepEqual(m.namesToFill({ 'ch-qa': '' }, { 'ch-qa': 'k-ann' }, () => undefined), []);
  assert.deepEqual(m.namesToFill({ 'ch-qa': '' }, {}, nameOf), []);
});

test('the running turn names the strip, not a bot waiting in line in the same chat', () => {
  const turns = [
    { turn_id: 'r1', kwant_id: 'k-ann', context_id: 'ch-qa', state: 'running' },
    { turn_id: 'q2', kwant_id: 'k-bea', context_id: 'ch-qa', state: 'queued' },
  ];
  // App gives each chat the last name it sees: replay that.
  const named = {};
  for (const t of m.liveNamingOrder(turns)) named[t.context_id] = nameOf(t.kwant_id);
  assert.equal(named['ch-qa'], 'Ann');
  // A chat with only a queued turn is still named (its strip says "waiting in line").
  const only = {};
  for (const t of m.liveNamingOrder([{ kwant_id: 'k-bea', context_id: 'ch-x', state: 'queued' }])) only[t.context_id] = nameOf(t.kwant_id);
  assert.equal(only['ch-x'], 'Beatrix');
});

test('App wires it: page-open, turn and stream events record the bot id; the bot list fills names', () => {
  const src = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.equal((src.match(/markLive\([^)]*kwantName\(/g) ?? []).length, 1, 'only markLiveKwant itself looks a name up');
  assert.ok((src.match(/markLiveKwant\(/g) ?? []).length >= 4, 'the four callers');
  assert.match(src, /liveNamingOrder\(Object\.values\(map\)\)/);
  assert.match(src, /namesToFill\(liveRef\.current, liveKwantRef\.current, kwantName\)[\s\S]{0,200}\}, \[kwants\]\);/);
  assert.match(src, /delete liveKwantRef\.current\[ctx\]/, 'cleared with the live chat');
});

test('the queued row text wraps pretty, so a last word never sits alone at 390 px (#479, Mira)', () => {
  const html = m.beside([{ position: 1, reason: 'agent_busy', speaker: 'Beatrix', named: true, turnId: 'q2' }]);
  assert.match(html, /<span class="min-w-0 \[text-wrap:pretty\]"><strong/);
});
