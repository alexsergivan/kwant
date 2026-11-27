// bubble-not-alert, in the chat: a bot's landed and live replies are .kw-reply-plain (no fill, border or radius) and
// carry --kw-thread in their avatar's body colour. Yours, an other's and work-ahead keep their boxes.
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

// Shell -> green via "shell", Guard -> orange via "guard".
const BODY = { green: ui.paletteCreature('green').body, orange: ui.paletteCreature('orange').body };
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
const BOX = /rounded-2xl|rounded-tl-sm|\bborder\b|\bbg-|shadow-sm|kw-bubble-tint|--kw-bubble-bg/;
const plainTag = (r) => {
  const at = r.indexOf('data-reply-plain=""');
  assert.ok(at > 0, 'data-reply-plain body');
  return r.slice(r.lastIndexOf('<div', at), r.indexOf('>', at) + 1);
};

test('the names pick the palettes this test expects', () => {
  assert.equal(ui.pickCreature('Team', 'Shell').body, BODY.green);
  assert.equal(ui.pickCreature('Team', 'Guard').body, BODY.orange);
});

const html = ui.chat(props([
  msg('s1', 'Shell'),
  msg('g1', 'Guard'),
  msg('p1', 'PlayerHero', 'user'),
  msg('o1', 'PlayerHero', 'user', j({ sent_by_bot: true, sent_by_bot_name: 'Scout', sent_by_kwant_id: 'k-scout' })),
]));

test("each bot's landed reply is plain text with its own creature thread, back to back", () => {
  for (const [id, body] of [['s1', BODY.green], ['g1', BODY.orange]]) {
    const tag = plainTag(row(html, id));
    assert.match(tag, /class="kw-reply-plain text-sm leading-relaxed text-zinc-200 w-full"/, id);
    assert.ok(tag.includes(`--kw-thread:${body}`), `${id}: ${tag}`);
    assert.doesNotMatch(tag, BOX, `${id}: no box`);
  }
});

test("your bubble and an other's bubble keep their boxes and get no thread", () => {
  for (const id of ['p1', 'o1']) {
    const r = row(html, id);
    assert.doesNotMatch(r, /kw-reply-plain|--kw-thread|data-reply-plain/, id);
  }
  assert.match(row(html, 'p1'), /px-3\.5 py-3 sm:p-5 rounded-2xl text-sm leading-relaxed shadow-sm w-full bg-zinc-800 text-zinc-100 rounded-tr-sm border border-zinc-700\/60 max-w-2xl/);
  assert.match(row(html, 'o1'), /px-3\.5 py-3 sm:p-5 rounded-2xl text-sm leading-relaxed shadow-sm w-full bg-transparent text-zinc-200 rounded-tl-sm border border-zinc-700 max-w-2xl/);
  assert.match(row(html, 'o1'), /data-other-bubble=""/);
});

test("the live reply is plain with the replying bot's thread from its first frame", () => {
  const live = ui.chat(props([msg('p2', 'PlayerHero', 'user')], { isStreaming: true, streamingSpeakerName: 'Shell', streamingText: 'Working on it' }));
  const at = live.indexOf('data-live-bubble-body');
  assert.ok(at > 0, 'live reply rendered');
  const tag = live.slice(live.lastIndexOf('<div', at), live.indexOf('>', at) + 1);
  assert.match(tag, /kw-reply-plain/);
  assert.ok(tag.includes(`--kw-thread:${BODY.green}`), tag);
  assert.doesNotMatch(tag, BOX);
});
