// reply-bubble-bot-tint, in the chat: a bot's landed and live reply bubbles carry --kw-bubble-bg/-line for that bot's
// avatar palette (design.md table) and kw-bubble-tint, not today's indigo fill. Yours, an other's and work-ahead keep theirs.
// This file imports only modules that exist on main, so it fails there (the red proof) and passes with the tint.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  export { pickCreature, paletteCreature } from './src/components/ModernAvatar';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
`);

// design.md rows for the palettes these names pick (Shell -> green via "shell", Guard -> orange via "guard").
const TINT = {
  green: { bg: '#0c2011', line: '#194f29' },
  orange: { bg: '#271607', line: '#623605' },
};
const j = (o) => JSON.stringify(o);
const msg = (id, sender_id, role = 'kwant', meta = null) => ({ id, sender_role: role, sender_id, text: `text of ${id}`, meta, created_at: '2026-11-26T09:00:00Z' });
const props = (messages, extra = {}) => ({
  title: 'Team', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: async () => true, onResolveApproval: () => {},
  kwants: [{ id: 'k-shell', name: 'Shell', status: 'idle' }, { id: 'k-guard', name: 'Guard', status: 'idle' }], teams: [], onContinueFrom: () => {},
  ...extra,
});
const row = (html, id) => {
  const at = html.indexOf(`data-message-id="${id}"`);
  assert.ok(at > 0, `row ${id}`);
  const next = html.indexOf('data-message-id="', at + 10);
  return html.slice(html.lastIndexOf('<div', at), next < 0 ? undefined : html.lastIndexOf('<div', next));
};
const styleHas = (r, t) => r.includes(`--kw-bubble-bg:${t.bg};--kw-bubble-line:${t.line}`);

test('the names pick the palettes this test expects', () => {
  assert.equal(ui.pickCreature('Team', 'Shell').body, ui.paletteCreature('green').body);
  assert.equal(ui.pickCreature('Team', 'Guard').body, ui.paletteCreature('orange').body);
});

const html = ui.chat(props([
  msg('s1', 'Shell'),
  msg('g1', 'Guard'),
  msg('p1', 'PlayerHero', 'user'),
  msg('o1', 'PlayerHero', 'user', j({ sent_by_bot: true, sent_by_bot_name: 'Scout', sent_by_kwant_id: 'k-scout' })),
]));

test("each bot's landed reply wears its own avatar's tint, back to back", () => {
  const s1 = row(html, 's1');
  const g1 = row(html, 'g1');
  assert.ok(styleHas(s1, TINT.green), 'Shell: green tint');
  assert.ok(styleHas(g1, TINT.orange), 'Guard: orange tint');
  for (const r of [s1, g1]) {
    assert.match(r, /kw-bubble-tint text-zinc-200 rounded-tl-sm border/);
    assert.doesNotMatch(r, /bg-zinc-900\/60/, 'no indigo fill fighting the tint');
    assert.doesNotMatch(r, /border-zinc-800\/80/, 'no zinc edge fighting the tint');
  }
});

test("your bubble and an other's bubble are unchanged", () => {
  for (const id of ['p1', 'o1']) {
    const r = row(html, id);
    assert.doesNotMatch(r, /kw-bubble-tint|--kw-bubble-bg/, id);
  }
  assert.match(row(html, 'p1'), /bg-zinc-800 text-zinc-100 rounded-tr-sm/);
  assert.match(row(html, 'o1'), /data-other-bubble=""/);
});

test('the live bubble is tinted from its first frame, in the replying bot\'s colour', () => {
  const live = ui.chat(props([msg('p2', 'PlayerHero', 'user')], { isStreaming: true, streamingSpeakerName: 'Shell', streamingText: 'Working on it' }));
  const at = live.indexOf('data-live-bubble-body');
  assert.ok(at > 0, 'live bubble rendered');
  const tag = live.slice(live.lastIndexOf('<div', at), live.indexOf('>', at) + 1);
  assert.match(tag, /kw-bubble-tint/);
  assert.ok(tag.includes(`--kw-bubble-bg:${TINT.green.bg}`), tag);
  assert.doesNotMatch(tag, /bg-zinc-900\/60/);
});
