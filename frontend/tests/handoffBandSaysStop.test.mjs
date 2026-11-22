// 32n (autonomy/handoff-band-says-stop/brief.md; Mira's 0.9.35 UX pass, "Also seen" d and f): during a handoff chain
// the band button and the composer stop button say "Stop", like the handoff cards in the stream; only a team discussion
// says "Stop Discussion". A discussion's turn count is a cap, so it reads "turn 2, up to 4", not "turn 2 of 4".
// Renders the real ChatView synchronously (renderToStaticMarkup): no ticks or waits. Red at main d16746c0.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
`);

const view = (discussionState) => ui.chat({
  title: 'Team', subtitle: '', messages: [], streamingText: 'Writing the draft…', streamingThinking: '', isStreaming: true,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
  onContinueFrom: () => {}, onCancelDiscussion: () => {}, discussionState,
  home: { kind: 'team', team: { id: 't1', name: 'Team', members: [] } },
});
const text = (h) => h.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ');
/** The band: from its pulsing Users icon to the end of its button. */
const band = (html) => {
  const at = html.indexOf('animate-pulse shrink-0');
  assert.ok(at > 0, 'the in-flight band shows');
  const end = html.indexOf('</button>', at);
  return html.slice(at, end + 9);
};
const bandButton = (html) => text(band(html).slice(band(html).lastIndexOf('<button')));
const composerStop = (html) => {
  const m = html.match(/<button[^>]*title="([^"]*)"[^>]*aria-label="([^"]*)"[^>]*>(?:(?!<\/button>).)*<\/button>/gs) || [];
  const stops = m.map((b) => b.match(/title="([^"]*)"[^>]*aria-label="([^"]*)"/)).filter((x) => /^Stop/.test(x[1]));
  assert.equal(stops.length, 1, 'one composer stop button');
  return { title: stops[0][1], label: stops[0][2] };
};

const handoff = { inProgress: true, kind: 'handoff', currentSpeaker: 'Writer', askedBy: 'Researcher', currentTurn: 3, totalTurns: 3 };
const discussion = { inProgress: true, currentSpeaker: 'Lea', askedBy: 'Ada', phase: 'reply', currentTurn: 2, totalTurns: 4 };

test('32n: a handoff band button and the composer stop button just say Stop', () => {
  const html = view(handoff);
  assert.match(text(band(html)), /Handoff: Writer is replying \(asked by Researcher\) • 3 of 3/, 'handoff band text unchanged');
  assert.equal(bandButton(html).trim(), 'Stop', 'band button says Stop, no "Discussion"');
  assert.doesNotMatch(band(html), /Discussion/);
  assert.deepEqual(composerStop(html), { title: 'Stop', label: 'Stop' });
});

test('32n: a team discussion keeps Stop Discussion and reads "turn 2, up to 4"', () => {
  const html = view(discussion);
  assert.match(text(band(html)), /Team discussion: Lea is answering Ada • turn 2, up to 4/);
  assert.doesNotMatch(text(band(html)), /turn 2 of 4/);
  assert.equal(bandButton(html).trim(), 'Stop Discussion');
  assert.deepEqual(composerStop(html), { title: 'Stop Discussion', label: 'Stop discussion' });
});

test('32n: no run in progress -> the composer stop button says Stop and no band shows', () => {
  const html = view(undefined);
  assert.equal(html.indexOf('animate-pulse shrink-0 '), -1);
  assert.deepEqual(composerStop(html), { title: 'Stop', label: 'Stop' });
});
