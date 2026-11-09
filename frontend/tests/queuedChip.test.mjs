// 0.8.0 F (Rigel's 0.7.7 final check): the conversation tab's "Queued · #N" chip and the phone
// header's dot are grey like the in-chat "In line" line; amber stays for "waiting on you".
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { StatusBadge, statusDot } from './src/components/ThreadBar';
  export const badge = (b) => renderToStaticMarkup(React.createElement(StatusBadge, { badge: b }));
  export { statusDot };
`);

test('the queued tab chip is zinc, not amber', () => {
  const html = m.badge({ kind: 'queued', label: 'Queued · #2' });
  assert.match(html, /Queued · #2/);
  assert.match(html, /text-zinc-300 bg-zinc-800\/60 border-zinc-600/);
  assert.doesNotMatch(html, /amber/);
  assert.match(m.badge({ kind: 'waiting', label: 'Waiting for you' }), /text-amber-300/, 'waiting on you stays amber');
});

test('the phone header dot: grey when queued, amber when waiting on you', () => {
  assert.deepEqual(m.statusDot({ kind: 'queued', label: 'Queued · #1' }), { tone: 'queued', label: 'Queued · #1' });
  assert.deepEqual(m.statusDot({ kind: 'waiting', label: 'Waiting for you' }), { tone: 'waiting', label: 'Waiting for you' });
  assert.equal(m.statusDot({ kind: 'done', label: 'Done' }), null);
});
