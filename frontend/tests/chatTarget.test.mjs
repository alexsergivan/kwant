// "Answer in chat" / "Open the chat it came from" on the Mission board: the chat it points at may
// already be the selected one, under the board. It must still close the board and show that chat.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const { planOpenTarget } = await loadBundled(`export * from './src/chatTarget';`);

const exists = { channel: (id) => id === 'c1' || id === 'c2', kwant: (id) => id === 'shelly' || id === 'lead' };
const shown = (over = {}) => ({ selectedId: 'shelly', isChannel: false, openThreadId: null, ...over });

test("the owner's direct chat is already selected: reveal it instead of doing nothing", () => {
  assert.equal(planOpenTarget({ channel_id: null, kwant_id: 'shelly' }, shown(), exists), 'reveal');
});

test('a channel that is already selected is revealed too', () => {
  assert.equal(planOpenTarget({ channel_id: 'c1', kwant_id: null }, shown({ selectedId: 'c1', isChannel: true }), exists), 'reveal');
});

test('another chat, or a thread of the same bot, switches the chat pane', () => {
  assert.equal(planOpenTarget({ channel_id: null, kwant_id: 'lead' }, shown(), exists), 'select');
  assert.equal(planOpenTarget({ channel_id: 'c2', kwant_id: null }, shown({ selectedId: 'c1', isChannel: true }), exists), 'select');
  assert.equal(planOpenTarget({ channel_id: null, kwant_id: 'shelly' }, shown({ openThreadId: 't1' }), exists), 'select');
  assert.equal(planOpenTarget({ channel_id: 'c1', kwant_id: null }, shown(), exists), 'select');
});

test('a chat that no longer exists is gone', () => {
  assert.equal(planOpenTarget({ channel_id: 'nope', kwant_id: null }, shown(), exists), 'gone');
  assert.equal(planOpenTarget({ channel_id: null, kwant_id: 'nope' }, shown(), exists), 'gone');
  assert.equal(planOpenTarget({ channel_id: null, kwant_id: null }, shown(), exists), 'gone');
});

test("App's openTarget closes the board and Inbox when the chat is already selected", () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const reveal = app.match(/const revealChat = \(\) => \{([\s\S]*?)\n  \};/);
  assert.ok(reveal, 'revealChat exists');
  for (const call of ["setPage('chat')", "setPhoneView('chat')"]) {
    assert.ok(reveal[1].includes(call), `revealChat calls ${call}`);
  }
  const open = app.match(/const openTarget = \(t: ChatTarget\): boolean => \{([\s\S]*?)\n  \};/);
  assert.ok(open, 'openTarget exists');
  assert.match(open[1], /if \(plan === 'reveal'\) revealChat\(\);/);
});
