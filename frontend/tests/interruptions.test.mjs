import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { parseInterruption, retryTargets, interruptionText, runRetryOnce } from './src/interruptions';
  export { InterruptedNotice, SystemNotice } from './src/components/SystemNotice';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

// Shaped like interrupt_running_turns in src/server/routes.rs.
const restartNote = (id, name = 'Architect') => ({
  id,
  sender_role: 'system',
  sender_id: 'kwnt',
  text: `This reply from ${name} was interrupted by a restart. Send your message again to retry.`,
  meta: JSON.stringify({ kind: 'turn_interrupted', interrupted: true, kwant_id: 'k1', kwant_name: name, turn_id: 't1', started_at: '2026-10-21T19:00:00Z' }),
});
// The stall watchdog's message in routes.rs (saved as the bot's reply with turn_stalled meta).
const taggedStall = (id, name = 'Writer', secs = 120, text = 'anything') => ({
  id,
  sender_role: 'kwant',
  sender_id: name,
  text,
  meta: JSON.stringify({ kind: 'turn_stalled', stalled: true, kwant_id: 'k2', kwant_name: name, stall_secs: secs }),
});
// Older rows, saved before the watchdog tagged its message: text only.
const stallNote = (id, name = 'Writer', secs = 120) => ({
  id,
  sender_role: 'kwant',
  sender_id: name,
  text: `⚠️ [${name}] The model sent nothing for ${secs} s, so this reply was stopped. Send your message again to retry.`,
});
const user = (id, text, meta = null) => ({ id, sender_role: 'user', sender_id: 'PlayerHero', text, meta });
const bot = (id, text) => ({ id, sender_role: 'kwant', sender_id: 'Architect', text });

test('recognises restart and stall notices, and nothing else', () => {
  assert.deepEqual(m.parseInterruption(restartNote('n1')), { kind: 'restart', botName: 'Architect' });
  assert.deepEqual(m.parseInterruption(stallNote('n2', 'Lead Dev', 90)), { kind: 'stall', botName: 'Lead Dev', secs: 90 });
  assert.equal(m.parseInterruption(bot('b', 'The model sent nothing, all fine')), null);
  assert.equal(m.parseInterruption({ ...restartNote('x'), meta: JSON.stringify({ kind: 'delegation_stalled' }) }), null);
  assert.equal(m.parseInterruption({ ...restartNote('x'), meta: 'not json' }), null);
  // A person quoting the stall line is not a notice.
  assert.equal(m.parseInterruption({ ...stallNote('s'), sender_role: 'user' }), null);
});

test('stall stops are recognised by meta first, wording only as a fallback for old rows', () => {
  // Tagged: found even if the wording changes.
  assert.deepEqual(m.parseInterruption(taggedStall('t1', 'TechLead', 45, 'Reply stopped (reworded).')), { kind: 'stall', botName: 'TechLead', secs: 45 });
  // Meta wins over the text: an untagged-looking text with stalled:false is not a notice.
  assert.equal(
    m.parseInterruption({ ...stallNote('t2'), meta: JSON.stringify({ kind: 'turn_stalled', stalled: false }) }),
    null,
  );
  // Missing secs or name: still a notice, with gentler wording.
  const bare = m.parseInterruption({ ...taggedStall('t3', 'Lead'), meta: JSON.stringify({ kind: 'turn_stalled', stalled: true }) });
  assert.deepEqual(bare, { kind: 'stall', botName: 'Lead', secs: undefined });
  assert.equal(m.interruptionText(bare), 'Lead went quiet, so the reply was stopped.');
  // Old row without meta: the wording match still works.
  assert.deepEqual(m.parseInterruption(stallNote('t4', 'Writer', 120)), { kind: 'stall', botName: 'Writer', secs: 120 });
  // Other meta kinds on a bot message don't turn into notices through the text fallback either way.
  assert.equal(m.parseInterruption({ ...bot('b', 'hi'), meta: JSON.stringify({ kind: 'turn_stalled_x' }) }), null);
  // Tagged stalls get Retry like the rest.
  assert.equal(m.retryTargets([user('u1', 'build it'), taggedStall('t5')]).get('t5').text, 'build it');
});

test('Retry targets the person message before the newest notice only', () => {
  const att = { id: '0f8fad5b-d9cb-469f-a165-70867728950e', name: 'a.png', mime: 'image/png', size: 3 };
  const msgs = [
    user('u1', 'first'),
    restartNote('n1'),
    user('u2', 'second', JSON.stringify({ attachments: [att] })),
    bot('b1', 'partial'),
    restartNote('n2'),
  ];
  const t = m.retryTargets(msgs);
  assert.deepEqual([...t.keys()], ['n2']);
  assert.deepEqual(t.get('n2'), { text: 'second', attachments: [att] });
});

test('no Retry once the person has written after the notice, or with nothing to resend', () => {
  assert.equal(m.retryTargets([user('u1', 'hi'), stallNote('n1'), user('u2', 'hi')]).size, 0);
  assert.equal(m.retryTargets([restartNote('n1')]).size, 0);
  assert.equal(m.retryTargets([user('u1', 'go'), stallNote('n1'), bot('b', 'later reply')]).get('n1').text, 'go');
});

const render = (props) => m.renderToStaticMarkup(m.createElement(m.InterruptedNotice, props));

test('the notice shows the reason and an enabled Retry button when it can retry', () => {
  const html = render({ messageId: 'n1', interruption: { kind: 'restart', botName: 'Architect' }, retry: { text: 'hi', attachments: [] }, onRetry: () => {} });
  assert.match(html, /Architect&#x27;s reply was cut off when the app restarted/);
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Retry<\/span><\/button>/);
  assert.doesNotMatch(html, /<button[^>]*\sdisabled=""/);
  const stall = render({ messageId: 'n2', interruption: { kind: 'stall', botName: 'Writer', secs: 120 }, retry: { text: 'hi', attachments: [] }, onRetry: () => {} });
  assert.match(stall, /Writer sent nothing for 120 s, so the reply was stopped/);
  assert.match(stall, /data-interrupted="stall"/);
});

test('an older or already-retried notice has no button', () => {
  const html = render({ messageId: 'n1', interruption: { kind: 'restart', botName: 'Architect' }, retry: null, onRetry: () => {} });
  assert.doesNotMatch(html, /<button/);
  assert.doesNotMatch(html, /Retry sends/);
});

test('system notes render as a notice, not a bot reply', () => {
  const html = m.renderToStaticMarkup(m.createElement(m.SystemNotice, { messageId: 's1', text: "Growth wasn't asked: inside code" }));
  assert.match(html, /role="note"/);
  assert.match(html, /data-system-notice/);
  assert.match(html, /Notice: <\/span>Growth wasn&#x27;t asked: inside code/);
});

// What a Retry click does (the button's onClick calls runRetryOnce with a ref lock).
test('Retry sends once per click and stays disabled while sending', async () => {
  const lock = { current: 'idle' };
  const states = [];
  const sent = [];
  let finish;
  const send = (t) => {
    sent.push(t.text);
    return new Promise((res) => (finish = res));
  };
  const target = { text: 'again please', attachments: [] };
  const first = m.runRetryOnce(lock, target, send, (s) => states.push(s));
  // A double click while the first send is in flight does nothing.
  assert.equal(await m.runRetryOnce(lock, target, send, (s) => states.push(s)), false);
  assert.deepEqual(sent, ['again please']);
  assert.deepEqual(states, ['sending']);
  finish();
  assert.equal(await first, true);
  assert.deepEqual(states, ['sending', 'sent']);
  // After it went through, further clicks don't resend.
  assert.equal(await m.runRetryOnce(lock, target, send, () => {}), false);
  assert.equal(sent.length, 1);
});

test('a failed Retry goes back to idle so the person can try again', async () => {
  const lock = { current: 'idle' };
  const states = [];
  let calls = 0;
  const send = () => {
    calls++;
    return calls === 1 ? Promise.reject(new Error('down')) : Promise.resolve();
  };
  assert.equal(await m.runRetryOnce(lock, { text: 'x', attachments: [] }, send, (s) => states.push(s)), false);
  assert.deepEqual(states, ['sending', 'idle']);
  assert.equal(await m.runRetryOnce(lock, { text: 'x', attachments: [] }, send, (s) => states.push(s)), true);
  assert.equal(calls, 2);
});

test('an idle Retry button is not disabled', () => {
  // The rendered button carries the real disabled attribute (not only the Tailwind variant).
  const html = render({ messageId: 'n1', interruption: { kind: 'restart', botName: 'A' }, retry: { text: 'x', attachments: [] }, onRetry: () => {} });
  assert.doesNotMatch(html, /<button[^>]*\sdisabled=""/);
});
