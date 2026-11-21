// 32l (autonomy/handoff-card-shows-once/brief.md): a teammate that waits on another gets two `delegation_started`
// messages with one delegation_id (queued, then "… has finished, so Writer starts …"). The stream draws the first as
// the live handoff card and later ones as the quiet one-line notice with their own text, not a second card + Stop.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  // Stands in for App.tsx renderDelegation (DelegationCard): one marked element per card drawn
  export const card = (msg, meta) =>
    React.createElement('div', { 'data-message-id': msg.id, 'data-card': meta.kind, 'data-deleg': meta.delegation_id }, meta.title);
`);

const chatProps = (messages) => ({
  title: 'Team', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
  onContinueFrom: () => {}, renderDelegation: ui.card,
  home: { kind: 'bot', bot: { id: 'k-lead', name: 'Lead', title: 'Lead', avatar_sprite: 'dev' } },
});
const deleg = (id, kind, delegation_id, text, specialist_name = 'Writer') => ({
  id, sender_role: 'kwant', sender_id: 'k-lead', text, created_at: '2026-11-21T10:00:00Z',
  meta: JSON.stringify({ kind, delegation_id, thread_id: `t-${delegation_id}`, specialist_id: `k-${specialist_name}`, specialist_name, status: 'running', title: 'Get the spring sale email ready' }),
});
const cards = (html, kind) => (html.match(new RegExp(`data-card="${kind}"`, 'g')) || []).length;
const notices = (html) => [...html.matchAll(/data-message-id="([^"]+)" data-system-notice/g)].map((x) => x[1]);

const queued = deleg('m1', 'delegation_started', 'd-w', 'Writer will start on “Get the spring sale email ready” once Researcher has finished.');
const started = deleg('m3', 'delegation_started', 'd-w', 'Researcher has finished, so Writer starts on “Get the spring sale email ready” now, with Researcher’s result.');
const result = deleg('m4', 'delegation_result', 'd-w', 'Writer finished.');
const researcher = deleg('m2', 'delegation_result', 'd-r', 'Researcher finished.', 'Researcher');

test('a second delegation_started for one delegation draws a quiet notice with its own text, not a second card', () => {
  const html = ui.chat(chatProps([queued, researcher, started, result]));
  assert.equal(cards(html, 'delegation_started'), 1, 'one live handoff card (one Stop)');
  assert.match(html, /data-message-id="m1" data-card="delegation_started"/, 'the first one is the card');
  assert.deepEqual(notices(html), ['m3'], 'the later one is the quiet line');
  assert.match(html, /Researcher has finished, so Writer starts on “Get the spring sale email ready” now, with Researcher’s result\./);
  assert.equal(cards(html, 'delegation_result'), 2, 'result cards are untouched');
  // Order kept: card, Researcher result, quiet line, Writer result
  const at = (id) => html.indexOf(`data-message-id="${id}"`);
  assert.ok(at('m1') < at('m2') && at('m2') < at('m3') && at('m3') < at('m4'));
});

test('two different delegations still draw two cards', () => {
  const other = deleg('m5', 'delegation_started', 'd-x', 'Editor starts.', 'Editor');
  const html = ui.chat(chatProps([queued, other]));
  assert.equal(cards(html, 'delegation_started'), 2);
  assert.deepEqual(notices(html), []);
});

test('older history paged out: the first started message on screen stays the card', () => {
  const html = ui.chat(chatProps([started, result]));
  assert.equal(cards(html, 'delegation_started'), 1);
  assert.match(html, /data-message-id="m3" data-card="delegation_started"/);
  assert.deepEqual(notices(html), []);
});
