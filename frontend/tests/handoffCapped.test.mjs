// Hand-offs past the per-message caps: one plain notice with a one-click Continue (meta kind
// "handoff_capped", POST /api/chains/:id/continue), and board owners shown with their real avatar.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  export { parseHandoffCapped, handoffCappedText, isAlreadyContinued, isChainGone, parseBudgetPaused, chainBudgetApi } from './src/chainBudget';
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { HandoffCappedNotice } from './src/components/SystemNotice';
  import { OwnerAvatar, AvatarsContext } from './src/components/MissionBoardView';
  export const notice = (props) => renderToStaticMarkup(React.createElement(HandoffCappedNotice, props));
  export const owner = (props) => renderToStaticMarkup(React.createElement(OwnerAvatar, props));
  export const ownerIn = (avatars, props) => renderToStaticMarkup(React.createElement(AvatarsContext.Provider, { value: avatars }, React.createElement(OwnerAvatar, props)));
`);

const capped = (over = {}) =>
  JSON.stringify({
    kind: 'handoff_capped',
    lead: 'k-lead',
    waiting: ['Parser', 'Frontend', 'Backend'],
    waiting_ids: ['k1', 'k2', 'k3'],
    sent: 16,
    text: 'lead reply',
    user_prompt: 'ship it',
    resumed: false,
    ...over,
  });

test('handoff_capped meta parses; other kinds and junk do not', () => {
  assert.deepEqual(m.parseHandoffCapped(capped()), { kind: 'handoff_capped', sent: 16, waiting: ['Parser', 'Frontend', 'Backend'], resumed: false });
  assert.equal(m.parseHandoffCapped(JSON.stringify({ kind: 'budget_paused', used: 1, budget: 2 })), null);
  assert.equal(m.parseHandoffCapped('not json'), null);
  assert.equal(m.parseHandoffCapped(null), null);
  // A capped notice is not mistaken for a budget pause
  assert.equal(m.parseBudgetPaused(capped()), null);
  // Bad fields fall back instead of breaking the chat
  assert.deepEqual(m.parseHandoffCapped(JSON.stringify({ kind: 'handoff_capped', sent: 'x', waiting: [3, '', 'Ada'] })), {
    kind: 'handoff_capped',
    sent: 0,
    waiting: ['Ada'],
    resumed: false,
  });
});

test('the wording is plain and asks to continue', () => {
  const t = m.handoffCappedText(m.parseHandoffCapped(capped()));
  assert.equal(t.headline, "Paused after 16 hand-offs so it doesn't loop. Continue?");
  assert.equal(t.detail, 'Waiting: Parser, Frontend, Backend');
  assert.equal(m.handoffCappedText({ kind: 'handoff_capped', sent: 1, waiting: [], resumed: false }).headline, "Paused after 1 hand-off so it doesn't loop. Continue?");
  const many = m.handoffCappedText({ kind: 'handoff_capped', sent: 4, waiting: ['A', 'B', 'C', 'D', 'E'], resumed: false });
  assert.equal(many.detail, 'Waiting: A, B, C and 2 more');
  const done = m.handoffCappedText({ kind: 'handoff_capped', sent: 4, waiting: ['A'], resumed: true });
  assert.equal(done.headline, "Paused after 4 hand-offs so it doesn't loop.");
  assert.equal(done.detail, 'Continued: handed on to A.');
  // The old "Send a new message to go on" wording is gone
  assert.doesNotMatch(JSON.stringify([t, done]), /new message|already asked/i);
});

test('the notice has one primary Continue button, and none once continued or read-only', () => {
  const html = m.notice({ messageId: 'msg1', meta: m.parseHandoffCapped(capped()), onContinue: async () => {} });
  assert.match(html, /data-handoff-capped/);
  assert.match(html, /Paused after 16 hand-offs so it doesn&#x27;t loop\. Continue\?/);
  assert.equal((html.match(/<button/g) || []).length, 1);
  assert.match(html, /data-handoff-continue[^>]*>.*Continue<\/span><\/button>/);
  assert.match(html, /aria-labelledby="capped-msg1"/);

  const resumed = m.notice({ messageId: 'msg1', meta: m.parseHandoffCapped(capped({ resumed: true })), onContinue: async () => {} });
  assert.doesNotMatch(resumed, /<button/);
  assert.match(resumed, /Continued: handed on to Parser, Frontend, Backend\./);

  assert.doesNotMatch(m.notice({ messageId: 'msg1', meta: m.parseHandoffCapped(capped()) }), /<button/);
});

test('Continue posts to the chain continue route; a 409 counts as already continued', async () => {
  const calls = [];
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push([init.method, url]);
    return new Response(JSON.stringify({ status: 'continued' }), { status: 200, headers: { 'content-type': 'application/json' } });
  };
  try {
    assert.deepEqual(await m.chainBudgetApi.continue('msg 1'), { status: 'continued' });
    assert.deepEqual(calls, [['POST', '/api/chains/msg%201/continue']]);
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'this chain was already continued' }), { status: 409 });
    const err = await m.chainBudgetApi.continue('msg1').catch((e) => e);
    assert.equal(m.isAlreadyContinued(err), true);
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'the lead of this chat is gone' }), { status: 410 });
    const gone = await m.chainBudgetApi.continue('msg1').catch((e) => e);
    assert.equal(m.isAlreadyContinued(gone), false);
    assert.equal(m.isChainGone(gone), true);
    assert.equal(gone.status, 410);
  } finally {
    globalThis.fetch = realFetch;
  }
});

test('owners show their bot avatar, with initials when the bot is unknown', () => {
  // No avatar map (outside the board page) or an unknown bot: initials
  const fallback = m.owner({ kwantId: 'k-gone', name: 'Front End' });
  assert.match(fallback, /data-owner-initials/);
  assert.match(fallback, />FE</);
  assert.doesNotMatch(fallback, /<svg/);
  assert.match(m.owner({ name: 'Backend' }), />B</);
  assert.match(m.ownerIn({ k1: 'robot' }, { kwantId: 'k-gone', name: 'Front End' }), /data-owner-initials/);
});

test('a known bot gets the same pixel avatar as the sidebar, even without a chosen sprite', () => {
  const html = m.ownerIn({ k1: 'robot', k2: '' }, { kwantId: 'k1', name: 'Frontend' });
  assert.match(html, /data-owner-avatar="k1"/);
  assert.match(html, /kw-avatar/);
  assert.match(html, /<svg/);
  assert.doesNotMatch(html, /data-owner-initials/);
  // An empty sprite still draws the name-picked creature, not initials
  assert.match(m.ownerIn({ k2: '' }, { kwantId: 'k2', name: 'Backend' }), /data-owner-avatar="k2"/);
});
