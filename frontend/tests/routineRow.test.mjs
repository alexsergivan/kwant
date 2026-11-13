// 0.9.7 routine row (Mira's UX audit #5, autonomy/routine-row/brief.md): a routine's run shows as a routine
// row ("Routine · <name>", clock, instructions on one line, expandable), never as the person's "You" bubble.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const r = await loadBundled(`export * from './src/routineRow';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { RoutineRow } from './src/components/RoutineRow';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  export const row = (props) => renderToStaticMarkup(React.createElement(RoutineRow, props));
`);

const INSTR = 'Check the team is still working as planned.\nList anything stuck for more than an hour.';
const routinePost = { id: 'r1', sender_role: 'user', sender_id: 'Routine: Morning check', text: INSTR, created_at: '2026-11-13T08:22:00Z' };
const own = { id: 'u1', sender_role: 'user', sender_id: 'me', text: 'Plan our spring offer.' };
const agent = { id: 'a1', sender_role: 'user', sender_id: 'me', text: 'Run the tests.', meta: JSON.stringify({ from_agent: 'Claude Code' }) };
const typed = { id: 'u2', sender_role: 'user', sender_id: 'me', text: 'Routine: Morning check, can you run it now?' };

test('routineOf: a routine post gives its name; everything else is null', () => {
  assert.equal(r.routineOf(routinePost), 'Morning check');
  assert.equal(r.routineOf(own), null, "the person's own message");
  assert.equal(r.routineOf(agent), null, 'an agent-sent user message');
  assert.equal(r.routineOf(typed), null, 'text that starts with "Routine:" is still the person: the check is on sender_id');
  assert.equal(r.routineOf({ sender_role: 'kwant', sender_id: 'Routine: Morning check' }), null, 'only user-role posts');
  assert.equal(r.routineOf({ sender_role: 'user', sender_id: 'Routine:Nightly' }), 'Nightly', 'same prefix rule as the Inbox');
  assert.equal(r.routineOf({ sender_role: 'user', sender_id: 'Routine: ' }), '', 'unnamed routine');
  assert.equal(r.routineOf({ sender_role: 'user', sender_id: null }), null);
  assert.equal(r.routineOf({ sender_role: 'user', sender_id: 'my Routine: x' }), null, 'prefix only');
});

test('label and one-line instructions', () => {
  assert.equal(r.routineLabel('Morning check'), 'Routine · Morning check');
  assert.equal(r.routineLabel(''), 'Routine');
  assert.equal(r.routineLine('  a\n\n  b\tc '), 'a b c');
  assert.equal(r.routineLine(null), '');
});

test('RoutineRow: collapsed is one button with aria-expanded=false, label, time and one line; no "You"', () => {
  const html = ui.row({ messageId: 'r1', name: 'Morning check', text: INSTR, at: routinePost.created_at });
  assert.match(html, /data-routine-row/);
  assert.match(html, /<button[^>]*aria-expanded="false"[^>]*aria-controls="routine-/);
  assert.match(html, /Routine · Morning check/);
  assert.match(html, /<time dateTime="2026-11-13T08:22:00Z"/);
  assert.match(html, /data-routine-line="">Check the team is still working as planned\. List anything stuck/);
  assert.match(html, /data-routine-body=""[^>]*hidden=""|hidden=""[^>]*data-routine-body=""/, 'body hidden while collapsed');
  assert.doesNotMatch(html, />You</);
});

test('RoutineRow: expanded shows the full instructions and aria-expanded=true', () => {
  const html = ui.row({ messageId: 'r1', name: 'Morning check', text: INSTR, defaultOpen: true });
  assert.match(html, /aria-expanded="true"/);
  assert.doesNotMatch(html, /data-routine-line/);
  assert.match(html, /data-routine-body=""[^>]*>Check the team is still working as planned\.\nList anything stuck/);
  assert.doesNotMatch(html, /data-routine-body=""[^>]*hidden/);
});

const chatProps = (messages) => ({
  title: 'Iris', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
});

test('ChatView: a routine post renders as a routine row, the others as before', () => {
  const html = ui.chat(chatProps([routinePost, own, agent, typed]));
  const rows = html.match(/data-routine-row/g) ?? [];
  assert.equal(rows.length, 1, 'only the routine post');
  assert.match(html, /data-message-id="r1"[^>]*data-routine-row/);
  assert.match(html, /Routine · Morning check/);
  // The person, the agent and the typed "Routine:" text keep their user bubbles
  assert.match(html, /data-message-id="u1"(?![^>]*data-routine-row)/);
  assert.match(html, /Claude Code \(coding agent\)/);
  assert.match(html, /Routine: Morning check, can you run it now\?/);
  const you = html.match(/<span class="text-zinc-200 shrink-0">You<\/span>/g) ?? [];
  assert.equal(you.length, 2, '"You" on the two typed messages only, never on the routine');
});
