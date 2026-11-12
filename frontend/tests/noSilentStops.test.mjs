// 0.9.5 "Runs never stop silently": stopped items in Needs you and the Inbox, Retry on a failed
// reply, and Retry on an approval that expired unanswered.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { parseInterruption, retryTargets, interruptionText, failedDetail, parseExpiredApproval } from './src/interruptions';
  export { itemLabel, stoppedAction, normalizeAwayItem } from './src/away';
  export { needsYouRows } from './src/chatsTab';
  export { ExpiredApprovalNotice, InterruptedNotice } from './src/components/SystemNotice';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

// Shaped like away::stopped_items in src/server/away.rs.
const stopped = (id, kwant_id, title, extra = {}) => ({
  kind: 'stopped',
  id,
  kwant_id,
  kwant_name: 'Ben',
  title,
  summary: title,
  status: 'open',
  timestamp: '2026-11-12T10:00:00Z',
  message_id: 'm1',
  retry: null,
  link: { thread_id: null, channel_id: null, direct_kwant_id: kwant_id },
  ...extra,
});

test('a stopped item is labelled Stopped and keeps its own words', () => {
  assert.deepEqual(m.itemLabel({ kind: 'stopped', status: 'open' }), { label: 'Stopped', tone: 'amber' });
  const item = stopped('w1', 'k1', 'Ben needs you to sign in to tiktok.com in the browser panel, then it continues.');
  assert.equal(m.normalizeAwayItem(item), item, 'passes through untouched');
});

test('Needs you lists a stopped bot with the line that says what resumes it', () => {
  const line = "Ben's reply was cut off when kwant restarted. Retry in the chat sends your message again.";
  const rows = m.needsYouRows([stopped('w1', 'k1', line)], {}, new Set(['k1']));
  assert.equal(rows.length, 1);
  assert.equal(rows[0].line, line);
  assert.equal(rows[0].item.id, 'w1');
});

test('an approval outranks a stopped item for the same bot; a browser step alone ranks below it', () => {
  const approval = { kind: 'approval', id: 'a1', kwant_id: 'k1', kwant_name: 'Ben', title: 't', summary: '', status: 'pending', timestamp: '2026-11-12T11:00:00Z', link: {}, tool_name: 'bash' };
  const rows = m.needsYouRows([stopped('w1', 'k1', 'Stopped'), approval], {}, new Set(['k1']));
  assert.equal(rows[0].line, 'Wants to run a command');
  // The oldest ask's time stays (the stopped item came first)
  assert.equal(rows[0].at, Date.parse('2026-11-12T10:00:00Z'));
  const withStep = m.needsYouRows([stopped('w1', 'k1', 'Ben needs you to sign in to x.com in the browser panel, then it continues.')], { k1: 1 }, new Set(['k1']));
  assert.match(withStep[0].line, /sign in to x\.com/, 'the specific ask wins over "Needs you in the browser"');
});

test('a stopped item offers Retry in place only for an expired approval', () => {
  assert.equal(m.stoppedAction({ kind: 'approval', retry: null }), null);
  assert.deepEqual(m.stoppedAction({ kind: 'stopped', retry: null }), { kind: 'open_chat' });
  assert.deepEqual(m.stoppedAction({ kind: 'stopped', retry: { type: 'continue' } }), { kind: 'open_chat' });
  assert.deepEqual(m.stoppedAction({ kind: 'stopped', retry: { type: 'approval', approval_id: 'ap1' } }), { kind: 'retry_approval', approvalId: 'ap1' });
});

// The failed reply saved by run_kwant_turn_inner (meta from stopped::gave_up_meta or turn_failed_text).
const failed = (id, text, meta = { kind: 'turn_failed', gave_up: true }) => ({ id, sender_role: 'kwant', sender_id: 'Ben', text, meta: JSON.stringify(meta) });
const user = (id, text) => ({ id, sender_role: 'user', sender_id: 'me', text, meta: null });

test('a failed reply is an interruption with its own words and gets Retry', () => {
  const msg = failed('f1', "⚠️ [Ben] Couldn't finish this reply: the model returned an error.\nDetails: 529");
  const i = m.parseInterruption(msg);
  assert.equal(i.kind, 'failed');
  assert.equal(i.botName, 'Ben');
  assert.equal(m.interruptionText(i), "Couldn't finish this reply: the model returned an error.\nDetails: 529");
  const targets = m.retryTargets([user('u1', 'make the report'), msg]);
  assert.deepEqual(targets.get('f1'), { text: 'make the report', attachments: [] });
  // Once the person writes again, the old notice has no Retry
  assert.equal(m.retryTargets([user('u1', 'a'), msg, user('u2', 'b')]).size, 0);
  // turn_failed without gave_up (a driver error) is the same
  assert.equal(m.parseInterruption(failed('f2', '⚠️ [Ben] boom', { kind: 'turn_failed' })).kind, 'failed');
  assert.equal(m.interruptionText({ kind: 'failed', botName: 'Ben' }), "Ben couldn't finish this reply.");
});

test('failedDetail drops only the bot tag', () => {
  assert.equal(m.failedDetail('⚠️ [Ben] boom', 'Ben'), 'boom');
  assert.equal(m.failedDetail('⚠️ [Other bot] boom', 'Ben'), 'boom');
  assert.equal(m.failedDetail('[draft] kept', 'Ben'), 'kept');
  assert.equal(m.failedDetail('no tag at all', 'Ben'), 'no tag at all');
});

test('the failed notice shows every line and a Retry button', () => {
  const html = m.renderToStaticMarkup(
    m.createElement(m.InterruptedNotice, { messageId: 'f1', interruption: { kind: 'failed', botName: 'Ben', detail: 'line one\nline two' }, retry: { text: 'x', attachments: [] }, onRetry: () => {} }),
  );
  assert.match(html, /data-interrupted="failed"/);
  assert.match(html, /whitespace-pre-line/);
  assert.match(html, />Retry</);
});

// The note mcp_runtime::expire_unanswered posts.
const expiredNote = (id, approval_id = 'ap1') => ({
  id,
  sender_role: 'system',
  sender_id: 'kwnt',
  text: "Ben stopped waiting for your OK to use bash (no answer in 24 h), so that step didn't run. Retry asks you again.",
  meta: JSON.stringify({ kind: 'approval_expired', approval_id, kwant_id: 'k1', kwant_name: 'Ben', tool_name: 'bash' }),
});

test('an expired approval note is parsed; other notes are not', () => {
  assert.deepEqual(m.parseExpiredApproval(expiredNote('n1')), { approvalId: 'ap1', botName: 'Ben', toolName: 'bash' });
  assert.equal(m.parseExpiredApproval({ ...expiredNote('n1'), sender_role: 'kwant' }), null);
  assert.equal(m.parseExpiredApproval({ ...expiredNote('n1'), meta: JSON.stringify({ kind: 'approval_expired' }) }), null);
  assert.equal(m.parseInterruption(expiredNote('n1')), null, 'not a resend-your-message Retry');
});

test('the expired approval notice has Retry, and none in a read-only view', () => {
  const props = { messageId: 'n1', text: 'Ben stopped waiting…', expired: { approvalId: 'ap1', botName: 'Ben', toolName: 'bash' } };
  const withRetry = m.renderToStaticMarkup(m.createElement(m.ExpiredApprovalNotice, { ...props, onRetry: async () => ({}) }));
  assert.match(withRetry, /data-approval-expired="ap1"/);
  assert.match(withRetry, /data-approval-retry/);
  assert.match(withRetry, />Retry</);
  const readOnly = m.renderToStaticMarkup(m.createElement(m.ExpiredApprovalNotice, props));
  assert.doesNotMatch(readOnly, /data-approval-retry/);
});
