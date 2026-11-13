// 0.7.1: a turn paused on an approval card (the production floor in the D6 footage) showed the grey
// "first words" skeleton above the card for as long as the card waited. It isn't loading: it waits for you.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const w = await loadBundled(`export * from './src/workingStrip';`);

const step = (over = {}) => ({ id: 's1', kind: 'tool', status: 'running', title: 'Running `ls`', started_at: '2026-11-05T20:00:00Z', ...over });
const approval = step({ id: 'ap', kind: 'approval', status: 'waiting', title: 'Run `./deploy.sh`' });

test('a turn waiting on an approval card shows "waiting for your OK", not the loading skeleton', () => {
  const live = { turnId: 't', steps: [step({ id: 'a', status: 'ok' }), approval] };
  assert.equal(w.liveBubble({ live }), 'approval');
  // The strip and the bubble agree on whose move it is.
  assert.equal(w.workingLabel({ live }), 'waiting for your OK');
  assert.equal(w.waitingOn({ live }), 'approval');
});

test('the skeleton stays for a turn that is really starting, and settled approvals don’t hold it', () => {
  assert.equal(w.liveBubble({}), 'starting');
  assert.equal(w.liveBubble({ live: { turnId: 't', steps: [step()] } }), 'starting');
  // Allowed: the step is no longer waiting, so the bot is working again.
  assert.equal(w.liveBubble({ live: { turnId: 't', steps: [{ ...approval, status: 'ok' }] } }), 'starting');
});

test('words, the queue and questions win over the approval line', () => {
  const live = { turnId: 't', steps: [approval] };
  assert.equal(w.liveBubble({ live, writing: true }), 'text');
  assert.equal(w.liveBubble({ live, queued: true }), 'queued');
  assert.equal(w.liveBubble({ live, awaitingAnswer: true }), 'question');
  assert.equal(w.liveBubble({ live, awaitingAnswer: true, awaitingBrowser: true }), 'browser');
});

test('ChatView draws the approval line before it falls back to the bot at work', () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const line = chat.indexOf("bubble === 'approval' ? (");
  const skel = chat.indexOf('<WorkingAvatar');
  assert.ok(line > 0 && skel > line, 'the approval branch comes before the bot at work');
  assert.match(chat, /Paused on the approval below\. Allow or deny it there; this reply carries on from it\./);
  assert.match(chat, /bubble === 'approval' \? 'Waiting for your OK'/);
});
