// 22i (brief: autonomy/attention-shows-the-ask/brief.md): every Needs-attention row lands on and rings its
// ask; the "Needs you" strip above the composer says the whole ask, its one action and "Done, clear it",
// steps through several, shows when the message isn't rendered, and goes when the item closes.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * from './src/askHere';
  export { approvalLine } from './src/approvals';
  export { NeedsYouStrip } from './src/components/NeedsYouStrip';
  export { Sidebar } from './src/components/Sidebar';
  export { needsYouRows, attentionRows } from './src/chatsTab';
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
`);
const h = m.React.createElement;
const noop = () => {};

const link = (over = {}) => ({ thread_id: null, channel_id: null, direct_kwant_id: null, ...over });
const item = (kind, over = {}) => ({
  kind, id: `${kind}-1`, kwant_id: 'quant', kwant_name: 'Quant', title: '', summary: '', status: 'open',
  timestamp: '2026-11-15T17:40:00Z', link: link({ direct_kwant_id: 'quant' }), ...over,
});
// Oleks's row: the title is cut for one line; the summary is the whole ask.
const STALL = 'The model sent nothing for 120 s, so this reply was stopped. Retry in the chat sends your message again.';
const stopped = item('stopped', { id: 'w1', title: 'The model sent nothing for 120 s, so this…', summary: STALL, message_id: 'm-notice', retry: null });

test('open target: every kind lands on its own chat and message, not only approvals', () => {
  // approval: its card id, in a channel
  assert.deepEqual(m.openTarget(item('approval', { id: 'ap1', link: link({ channel_id: 'c1' }) })), {
    channel_id: 'c1', kwant_id: 'quant', thread_id: null, focus: 'ap1', focusKind: 'approval',
  });
  // question: the card message from the open questions when the item doesn't name it; in a thread
  const q = item('question', { id: 'q1', link: link({ thread_id: 't9' }) });
  assert.deepEqual(m.openTarget(q, { questionMessage: (id) => (id === 'q1' ? 'm-q' : null) }), {
    channel_id: null, kwant_id: 'quant', thread_id: 't9', focus: 'm-q', focusKind: 'question',
  });
  // a question with no card known: the chat, nothing to ring
  assert.equal(m.openTarget(q).focus, null);
  // stopped: the notice that carries Retry, in the direct chat
  assert.deepEqual(m.openTarget(stopped), { channel_id: null, kwant_id: 'quant', thread_id: null, focus: 'm-notice', focusKind: 'message' });
  // stopped on a browser step: the step's card (from the step when the item has no message_id)
  const step = item('stopped', { id: 'hs1', human_step_id: 'hs1', message_id: null });
  assert.deepEqual(m.openTarget(step, { stepMessage: (id) => (id === 'hs1' ? 'm-step' : null) }), {
    channel_id: null, kwant_id: 'quant', thread_id: null, focus: 'm-step', focusKind: 'browser_step',
  });
  // direct chat named by another bot's id (a lead's chat), and an Inbox item with a message
  assert.equal(m.openTarget(item('stopped', { link: link({ direct_kwant_id: 'lead' }) })).kwant_id, 'lead');
  assert.deepEqual(m.openTarget(item('delegation', { message_id: 'm-r', link: link({ thread_id: 't2' }) })), {
    channel_id: null, kwant_id: 'quant', thread_id: 't2', focus: 'm-r', focusKind: 'message',
  });
});

test('one action per item: the server says it, else it is inferred from the kind', () => {
  assert.equal(m.askAction(item('question')), 'answer');
  assert.equal(m.askAction(item('approval')), 'decide');
  assert.equal(m.askAction(stopped), 'retry');
  assert.equal(m.askAction(item('stopped', { retry: { type: 'continue' } })), 'continue');
  assert.equal(m.askAction(item('stopped', { retry: { type: 'approval', approval_id: 'a9' } })), 'ask_again');
  assert.equal(m.askAction(item('stopped', { human_step_id: 'hs1' })), 'browser');
  assert.equal(m.askAction({ ...stopped, action: 'continue' }), 'continue', "Lin's action wins");
  assert.equal(m.askAction({ ...stopped, action: 'bogus' }), 'retry', 'an unknown action is ignored (older shape)');
  assert.equal(m.askAction({ ...stopped, action: null }), null, 'an explicit null: nothing resolves it but Done');
  assert.equal(m.askAction(item('delegation')), null);
  assert.deepEqual(Object.values(m.ACTION_LABEL), ['Retry', 'Continue', 'Ask again', 'Go to the question', 'Go to the approval', 'Open the browser step']);
});

test('the full line: the title whole; a title cut for a row gives way to the summary; an approval its question', () => {
  assert.equal(m.askLine(stopped), STALL);
  assert.equal(m.askLine(item('stopped', { title: 'The model sent nothing for 120 s, so this reply was stopped.', summary: 'Short.' })), 'The model sent nothing for 120 s, so this reply was stopped.');
  assert.equal(m.askLine(item('approval', { title: 'Wants to run a command', question: 'Run “deploy.sh prod”?' })), 'Run “deploy.sh prod”?');
  assert.equal(m.askLine(item('approval', { title: 'Wants to run a command' })), 'Wants to run a command');
  assert.equal(m.askLine(item('question', { title: 'Ship tonight?', summary: 'Choices: Yes · No' })), 'Ship tonight?');
  assert.equal(m.askLine(item('question', { title: '', summary: 'Pick one' })), 'Pick one');
});

test("the strip's words: kind, hint, the button's name and the error lines (design.md §2, §5)", () => {
  const rows = [
    [stopped, 'Stopped', 'Retry sends your last message to Quant again.', 'Retry Quant’s reply'],
    [{ ...stopped, status: 'failed' }, 'Failed', 'Retry runs it again from the start.', 'Retry Quant’s reply'],
    [item('stopped', { retry: { type: 'continue' } }), 'Paused', 'Continue picks up where Quant stopped. Nothing is sent again.', 'Continue Quant’s reply'],
    [item('stopped', { retry: { type: 'approval', approval_id: 'a' } }), 'Approval ran out', 'Ask again starts a new turn. Nothing runs until you say yes.', 'Ask Quant again'],
    [item('question'), 'Question', 'Answer it on the card in the chat.', 'Go to Quant’s question'],
    [item('approval'), 'Approval', 'Allow or Deny it on the card in the chat.', 'Go to Quant’s approval'],
    [item('stopped', { human_step_id: 'h' }), 'Browser step', 'Do it in Quant’s browser, then press Done there.', 'Open Quant’s browser step'],
    [item('delegation'), 'Stuck', 'There’s no button for this one. Write to Quant, or clear it once it’s handled.', null],
  ];
  for (const [it, kind, hint, name] of rows) {
    const a = m.askAction(it);
    assert.equal(m.askKind(it, a), kind);
    assert.equal(m.askHint(it, a, 'Quant'), hint);
    assert.equal(a ? m.actionName(a, 'Quant') : null, name);
  }
  assert.equal(m.failLine('retry', 'Quant’s engine is out until 20:50.', 'Quant'), 'Couldn’t retry: Quant’s engine is out until 20:50. Try again, or write to Quant.');
  assert.equal(m.failLine('continue', 'no budget', 'Quant'), 'Couldn’t continue: no budget. Try again, or write to Quant.');
  assert.equal(m.failLine('ask_again', 'gone', 'Quant'), 'Couldn’t ask again: gone. Try again, or write to Quant.');
  assert.equal(m.failLine('retry', null, 'Quant'), 'Couldn’t retry. Try again, or write to Quant.');
  assert.equal(m.failLine('dismiss', 'not yours', 'Quant'), 'Couldn’t clear it: not yours.');
});

test('asks here: only the open chat (thread, channel or direct), oldest first, minus cleared', () => {
  const a = item('approval', { id: 'a', timestamp: '2026-11-15T17:41:00Z' });
  const t = item('question', { id: 't', link: link({ thread_id: 'th' }) });
  const c = item('question', { id: 'c', link: link({ channel_id: 'ch' }) });
  const w = item('watch', { id: 'w', timestamp: '2026-11-15T17:50:00Z' });
  const sign = item('connection', { id: 'x' });
  const all = [a, stopped, t, c, w, sign];
  assert.deepEqual(m.asksHere(all, { direct: 'quant' }).map((i) => i.id), ['w1', 'a', 'w'], 'a sign-in to redo stays in the Inbox');
  assert.deepEqual(m.asksHere(all, { thread: 'th', kwant: 'quant' }).map((i) => i.id), ['t']);
  assert.deepEqual(m.asksHere(all, { channel: 'ch' }).map((i) => i.id), ['c']);
  assert.deepEqual(m.asksHere(all, { direct: 'other' }), []);
  assert.deepEqual(m.asksHere(all, null), []);
  assert.deepEqual(m.asksHere(all, { direct: 'quant' }, new Set(['w1'])).map((i) => i.id), ['a', 'w']);
  assert.equal(m.stripHeading([stopped]), 'Quant needs you');
  assert.equal(m.stripHeading([stopped, a]), '2 things need you');
  // the strip keeps the ask it showed when the list changes, else the same place
  assert.equal(m.stripIndex([{ id: 'x' }, { id: 'y' }], 'y', 0), 1);
  assert.equal(m.stripIndex([{ id: 'x' }], 'gone', 1), 0);
  assert.equal(m.stripIndex([], null, 3), 0);
});

test('Retry of a stopped turn resends the notice target, else the newest message of yours', () => {
  const you = (id, text) => ({ id, sender_role: 'user', sender_id: 'you', text, meta: null, created_at: '' });
  const bot = (id, text, meta = null) => ({ id, sender_role: 'kwant', sender_id: 'quant', text, meta, created_at: '' });
  // The notice isn't loaded (Oleks's case): the newest message of the person's
  assert.deepEqual(m.resendTarget([you('u1', 'first'), bot('b1', 'ok'), you('u2', 'Fix what the pre-push check found'), bot('b2', 'step failed')], 'm-notice'), {
    text: 'Fix what the pre-push check found', attachments: [],
  });
  assert.equal(m.resendTarget([bot('b1', 'hi')], 'm-notice'), null);
});

// ---- the strip, rendered ----
const mount = (props) => {
  const host = document.createElement('div');
  document.body.append(host);
  const root = m.createRoot(host);
  let last = props;
  const render = (p) => {
    last = { ...last, ...p };
    m.flushSync(() => root.render(h(m.NeedsYouStrip, { onAction: noop, onDismiss: async () => {}, ...last })));
  };
  render({});
  return { host, render, unmount: () => { root.unmount(); host.remove(); } };
};
const tick = () => new Promise((r) => setTimeout(r, 0));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const parse = (el) => new DOMParser().parseFromString(m.renderToStaticMarkup(el), 'text/html');
const NOW = Date.parse('2026-11-15T17:45:00Z');

test('strip, one ask in each state A1–A8: eyebrow, kind, the whole line, hint, "Done, clear it" then the action', () => {
  const cases = [
    [stopped, 'retry', 'Stopped', 'Retry'],
    [{ ...stopped, status: 'failed' }, 'retry', 'Failed', 'Retry'],
    [item('stopped', { retry: { type: 'continue' }, title: 'Quant stopped after 40 steps, the most one reply can take.' }), 'continue', 'Paused', 'Continue'],
    [item('stopped', { retry: { type: 'approval', approval_id: 'a9' }, title: 'Your OK ran out after 2 hours.' }), 'ask_again', 'Approval ran out', 'Ask again'],
    [item('question', { title: 'Should I rebalance into BTC now, or wait for the 4-hour close at 20:00?' }), 'answer', 'Question', 'Go to the question'],
    [item('approval', { title: 'Wants to run a command', question: 'Run “npm run deploy” on production?' }), 'decide', 'Approval', 'Go to the approval'],
    [item('stopped', { human_step_id: 'hs1', title: 'Quant needs you to sign in to binance.com.' }), 'browser', 'Browser step', 'Open the browser step'],
    [item('delegation', { title: 'Stuck: Quant has been on the same step for 25 minutes.' }), 'none', 'Stuck', null],
  ];
  for (const [it, action, kind, label] of cases) {
    const d = parse(h(m.NeedsYouStrip, { asks: [it], onAction: noop, onDismiss: async () => {}, onShow: noop, now: NOW }));
    const s = d.querySelector('[data-needs-you-strip]');
    assert.equal(s.getAttribute('data-action'), action);
    assert.equal(s.getAttribute('tabindex'), '-1', 'a keyboard open can focus it');
    const head = d.getElementById(s.getAttribute('aria-labelledby'));
    assert.equal(head.textContent, 'Quant needs you', 'named by its eyebrow');
    assert.match(s.querySelector('[data-strip-kind]').textContent, new RegExp(`^${kind} · `));
    assert.doesNotMatch(s.querySelector('[data-strip-kind]').textContent, /Quant/, 'one bot in view: no name in the kind line');
    assert.equal(s.querySelector('[data-strip-line]').textContent, m.askLine(it), 'the whole line');
    assert.doesNotMatch(s.querySelector('[data-strip-line]').getAttribute('class'), /truncate|line-clamp/, 'never cut');
    assert.equal(s.querySelector('[data-strip-hint]').textContent, m.askHint(it, m.askAction(it), 'Quant'));
    const buttons = [...s.querySelectorAll('[data-strip-buttons] button')];
    assert.ok(buttons[0].hasAttribute('data-strip-dismiss'), 'Done, clear it always shows, left of the action');
    assert.equal(buttons[0].textContent, 'Done, clear it');
    assert.equal(buttons[0].getAttribute('aria-label'), 'Done, clear it from Needs attention');
    if (label) {
      assert.equal(buttons[1].getAttribute('data-strip-action'), action);
      assert.equal(buttons[1].textContent, label);
      assert.equal(buttons[1].getAttribute('aria-label'), m.actionName(action, 'Quant'));
      assert.match(buttons[1].className, /kw-btn-primary/);
    } else assert.equal(buttons.length, 1, 'A8: only Done, clear it');
    assert.equal(s.querySelector('[data-strip-steps]'), null, 'one ask: no stepping');
  }
  const none = parse(h(m.NeedsYouStrip, { asks: [], onAction: noop, onDismiss: async () => {} }));
  assert.equal(none.querySelector('[data-needs-you-strip]'), null, 'nothing open: no strip');
});

test('strip: it reads the list, not the chat, so it shows when the notice is not rendered', () => {
  // No messages anywhere: the strip still says the ask (Oleks's row opened onto nothing)
  const d = parse(h(m.NeedsYouStrip, { asks: [stopped], onAction: noop, onDismiss: async () => {} }));
  assert.equal(d.querySelector('[data-strip-line]').textContent, STALL);
});

test('strip: steps through several asks (M1, M2), rings each one shown, and goes live when they close', async () => {
  const shown = [];
  const two = [stopped, item('question', { id: 'q2', title: 'Should I rebalance now?', timestamp: '2026-11-15T17:44:00Z' })];
  const { host, render, unmount } = mount({ asks: two, onShow: (it, o) => (shown.push(`${it.id}:${!!o?.quiet}`), true), now: NOW });
  const q = (s) => host.querySelector(s);
  const head = () => host.querySelector(`#${q('[data-needs-you-strip]').getAttribute('aria-labelledby')}`).textContent;
  assert.equal(head(), '2 things need you');
  assert.equal(q('[data-strip-count]').textContent, '1 of 2');
  assert.equal(q('[data-strip-count]').getAttribute('aria-live'), 'polite');
  assert.ok(q('[aria-label="Previous ask"]').disabled, 'at the start: Previous is disabled, not hidden');
  assert.match(q('[data-strip-kind]').textContent, /^Stopped · Quant · /, 'several asks: the bot is named');
  m.flushSync(() => q('[aria-label="Next ask"]').click());
  assert.equal(q('[data-strip-count]').textContent, '2 of 2');
  assert.equal(q('[data-needs-you-strip]').getAttribute('data-needs-you-strip'), 'q2');
  assert.ok(q('[aria-label="Next ask"]').disabled, 'at the end: Next is disabled');
  assert.deepEqual(shown, ['q2:true'], 'stepping rings that ask if its card is rendered');
  // A new, older ask arrives: the strip stays on the question it shows
  render({ asks: [item('question', { id: 'q0', timestamp: '2026-11-15T17:00:00Z' }), ...two] });
  await tick();
  assert.equal(q('[data-needs-you-strip]').getAttribute('data-needs-you-strip'), 'q2');
  assert.equal(q('[data-strip-count]').textContent, '3 of 3');
  // The shown one closes and it was last: the strip moves to the one before it
  render({ asks: [item('question', { id: 'q0', timestamp: '2026-11-15T17:00:00Z' }), stopped] });
  await tick();
  assert.equal(q('[data-needs-you-strip]').getAttribute('data-needs-you-strip'), 'w1');
  render({ asks: [stopped] });
  await tick();
  assert.equal(head(), 'Quant needs you');
  assert.equal(q('[data-strip-steps]'), null);
  render({ asks: [] });
  assert.equal(q('[data-needs-you-strip]'), null, 'gone without a reload');
  unmount();
});

test('strip: Retry sends once, waits for the close (busy), and comes back with a note when it stays open', async () => {
  const calls = [];
  const { host, unmount } = mount({
    asks: [stopped],
    sentWaitMs: 30,
    onAction: async (it, action) => void calls.push(`${action}:${it.id}`),
    onDismiss: async (it) => void calls.push(`dismiss:${it.id}`),
  });
  const q = (s) => host.querySelector(s);
  q('[data-strip-action]').click();
  q('[data-strip-action]').click(); // a double press sends once
  await tick();
  await tick();
  assert.deepEqual(calls, ['retry:w1']);
  // A1b: busy until the close event, Done, clear it still pressable
  assert.equal(q('[data-strip-action]').textContent, 'Retrying…');
  assert.equal(q('[data-strip-action]').getAttribute('aria-busy'), 'true');
  assert.ok(q('[data-strip-action]').disabled);
  assert.ok(!q('[data-strip-dismiss]').disabled);
  await wait(60);
  await tick();
  assert.equal(q('[data-strip-action]').textContent, 'Retry', 'still open after the wait: the button comes back');
  assert.equal(q('[data-strip-hint]').textContent, 'Sent. This clears once Quant answers.');
  unmount();
});

test('strip: a failed press says why in the server’s words (A1c), and clearing shows Clearing… (A9)', async () => {
  let release;
  const { host, unmount } = mount({
    asks: [stopped],
    onAction: async () => {
      throw new Error('Quant’s engine is out until 20:50.');
    },
    onDismiss: () => new Promise((r) => (release = r)),
  });
  const q = (s) => host.querySelector(s);
  q('[data-strip-action]').click();
  await tick();
  await tick();
  assert.equal(q('[data-strip-error]').textContent, 'Couldn’t retry: Quant’s engine is out until 20:50. Try again, or write to Quant.');
  assert.equal(q('[data-strip-error]').getAttribute('role'), 'alert');
  assert.equal(q('[data-strip-action]').textContent, 'Retry', 'the button is back');
  q('[data-strip-dismiss]').click();
  await tick();
  assert.equal(q('[data-strip-dismiss]').textContent, 'Clearing…');
  assert.ok(q('[data-strip-dismiss]').disabled && q('[data-strip-action]').disabled, 'clearing stops both');
  assert.equal(q('[data-strip-error]'), null, 'the error clears on the next try');
  release();
  await tick();
  unmount();
});

test('strip: Go to the question rings it; a card that is gone offers Open in Inbox (A5b)', async () => {
  const shown = [];
  let found = true;
  const opened = [];
  const ap = item('approval', { id: 'ap', title: 'Run deploy?' });
  const { host, unmount } = mount({ asks: [ap], onShow: async (it) => (shown.push(it.id), found), onOpenInbox: (it) => opened.push(it.id) });
  const q = (s) => host.querySelector(s);
  q('[data-strip-action]').click();
  await tick();
  await tick();
  assert.deepEqual(shown, ['ap']);
  assert.equal(q('[data-strip-action]').textContent, 'Go to the approval', 'found: the strip stays until it is answered');
  found = false;
  q('[data-strip-action]').click();
  await tick();
  await tick();
  assert.equal(q('[data-strip-hint]').textContent, 'Its card isn’t in this chat anymore.');
  assert.equal(q('[data-strip-action]').textContent, 'Open in Inbox');
  assert.equal(q('[data-strip-action]').getAttribute('data-strip-action'), 'inbox');
  q('[data-strip-action]').click();
  await tick();
  assert.deepEqual(opened, ['ap']);
  unmount();
});

test('strip: a keyboard open focuses it; when the last ask closes, focus goes to the composer; it says so', async () => {
  const composer = document.createElement('textarea');
  composer.setAttribute('data-composer', '');
  document.body.append(composer);
  const { host, render, unmount } = mount({ asks: [], hereKey: 'd:quant' });
  const live = () => host.querySelector('[data-needs-you-live]').textContent;
  // An ask arrives live in the open chat: one polite line, bot plus button
  render({ asks: [stopped] });
  await tick();
  assert.equal(live(), 'Quant needs you: Retry');
  render({ focusKey: 1 });
  await tick();
  assert.equal(document.activeElement, host.querySelector('[data-needs-you-strip]'));
  host.querySelector('[data-strip-action]').focus();
  render({ asks: [] });
  await tick();
  assert.equal(host.querySelector('[data-needs-you-strip]'), null);
  assert.equal(document.activeElement, composer, 'never dropped to the page body');
  // Opening another chat with asks in it says nothing; a clear says "Cleared." at once
  render({ asks: [stopped], hereKey: 'd:other' });
  await tick();
  host.querySelector('[data-strip-dismiss]').click();
  await tick();
  render({ asks: [] });
  await tick();
  assert.equal(live(), 'Cleared.');
  unmount();
  composer.remove();
});

// ---- the pinned row ----
const space = { id: 'spc_default', name: 'Binan', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });

test('pinned row: hover and focus give the whole ask in a tooltip the row is described by', () => {
  const kwants = [bot('quant', 'Quant'), bot('builder', 'Builder')];
  const waiting = [stopped, item('approval', { id: 'ap', kwant_id: 'builder', kwant_name: 'Builder', tool_name: 'bash', question: 'Run “deploy.sh prod”?', link: link({ direct_kwant_id: 'builder' }) })];
  const needs = m.needsYouRows(waiting, {}, new Set(kwants.map((k) => k.id)));
  const html = m.renderToStaticMarkup(h(m.Sidebar, {
    space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
    kwants, channels: [], teams: [], selectedId: null, isChannel: false, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
    onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop, busyKwants: {}, botStatus: {},
    attention: m.attentionRows({ kwants, needs, now: Date.parse('2026-11-15T17:45:00Z') }),
  }));
  const d = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const row = d.querySelector('[data-pin-row="quant"]');
  assert.equal(row.getAttribute('title'), null, 'no native title: it never shows on focus');
  assert.equal(row.getAttribute('aria-label'), null, 'the row keeps its name');
  const tip = d.getElementById('pin-tip-quant');
  assert.equal(tip.getAttribute('role'), 'tooltip');
  assert.ok(row.getAttribute('aria-describedby').split(' ').includes('pin-tip-quant'));
  assert.ok(tip.hasAttribute('hidden'), 'hidden until hover or focus');
  const [top, line] = tip.querySelectorAll('span');
  assert.match(top.textContent, /^Quant · Stopped · /);
  assert.equal(line.textContent, STALL, 'the whole line, not the row’s cut one');
  assert.equal(row.querySelector('[data-bot-busy]').getAttribute('title'), null);
  assert.equal(d.querySelector('#pin-tip-builder span:last-child').textContent, 'Run “deploy.sh prod”?');
  assert.match(d.querySelector('#pin-tip-builder span').textContent, /^Builder · Approval · /);
  // The bot's own row in the list is not a pinned copy: no tooltip there
  assert.equal(d.querySelector('[data-bot-row="quant"] [role="tooltip"]'), null);
});

test('pinned row tooltip: shows at once on keyboard focus, after 300 ms on hover, hides on Esc', async () => {
  const kwants = [bot('quant', 'Quant')];
  const needs = m.needsYouRows([stopped], {}, new Set(['quant']));
  const host = document.createElement('div');
  document.body.append(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(h(m.Sidebar, {
    space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
    kwants, channels: [], teams: [], selectedId: null, isChannel: false, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
    onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop, busyKwants: {}, botStatus: {},
    attention: m.attentionRows({ kwants, needs, now: Date.parse('2026-11-15T17:45:00Z') }),
  })));
  const row = host.querySelector('[data-pin-row="quant"]');
  const tip = () => host.querySelector('#pin-tip-quant');
  row.dispatchEvent(new window.MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body }));
  await tick();
  assert.ok(tip().hidden, 'not yet: hover waits 300 ms');
  await wait(330);
  assert.ok(!tip().hidden, 'shown after hover');
  row.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await tick();
  assert.ok(tip().hidden, 'Esc hides it');
  m.flushSync(() => root.unmount());
  host.remove();
});

// ---- round 2: Lin's #700 API (docs/live-activity.md §3) ----

test('open target: link.thread_kwant_id opens the thread under the bot that owns it (Oleks’s Quant case)', () => {
  // The thread belongs to Builder, not to the item's bot: looking it up under Quant found nothing.
  const it = item('stopped', { id: 'w9', message_id: 'm-n', link: link({ thread_id: 't-b', thread_kwant_id: 'builder' }) });
  assert.deepEqual(m.openTarget(it), { channel_id: null, kwant_id: 'builder', thread_id: 't-b', focus: 'm-n', focusKind: 'message' });
  // Absent (the bot's own thread, or an older server): the item's kwant_id, as before
  assert.equal(m.openTarget(item('stopped', { link: link({ thread_id: 't-q' }) })).kwant_id, 'quant');
  assert.equal(m.openTarget(item('stopped', { link: link({ thread_id: 't-q', thread_kwant_id: null }) })).kwant_id, 'quant');
  // The strip still finds it in that thread
  assert.ok(m.isHere(it, { thread: 't-b', kwant: 'builder' }));
});

test('Done, clear it follows dismissable; older servers keep it; the body names the kind', () => {
  assert.equal(m.canClear({ dismissable: true }), true);
  assert.equal(m.canClear({ dismissable: false }), false);
  assert.equal(m.canClear({}), true, 'missing (an older server): the round-1 inference, always offered');
  assert.equal(m.dismissKind(stopped), 'stopped');
  assert.equal(m.dismissKind(item('stopped', { human_step_id: 'hs' })), 'browser');
  assert.equal(m.dismissKind(item('routine_run')), 'routine_run');
});

test('a 409 use_* dismiss switches to the ask’s own button; other codes are errors', () => {
  assert.equal(m.switchFor('use_question_answer'), 'answer');
  assert.equal(m.switchFor('use_approval_decide'), 'decide');
  assert.equal(m.switchFor('use_watch_decide'), 'decide');
  assert.equal(m.switchFor('use_step_decline'), 'browser');
  for (const c of ['person_only', 'not_found', 'bad_kind', null, undefined]) assert.equal(m.switchFor(c), null);
});

test('Retry never goes to /api/waiting/:id/retry: approval, task, or resend', () => {
  assert.deepEqual(m.retryRoute(item('stopped', { retry: { type: 'approval', approval_id: 'ap9' } })), { via: 'approval', id: 'ap9' });
  assert.deepEqual(m.retryRoute(item('routine_run', { status: 'error', task_id: 'tk1' })), { via: 'task', id: 'tk1' });
  assert.deepEqual(m.retryRoute(item('delegation', { status: 'failed', task_id: 'tk2' })), { via: 'task', id: 'tk2' });
  assert.equal(m.retryRoute(item('delegation', { status: 'failed' })), null, 'no task to retry: write to the bot');
  assert.deepEqual(m.retryRoute(stopped), { via: 'resend' });
  // The client has no waiting-retry call at all.
  const src = readFileSync(new URL('../src/away.ts', import.meta.url), 'utf8');
  assert.ok(!/waiting\/\$\{[^}]+\}\/retry/.test(src) && !/retryWaiting/.test(src));
});

test('waiting_updated: a closed item drops at once; an open one waits for the refetch', () => {
  const list = [stopped, item('question', { id: 'q1' })];
  const ev = (status, id = 'w1') => ({ item: { id, kind: 'stopped', status, reason: status === 'closed' ? 'person_wrote' : null } });
  assert.equal(m.closedId(ev('closed')), 'w1');
  assert.deepEqual(m.dropClosed(list, m.closedId(ev('closed'))).map((i) => i.id), ['q1']);
  assert.equal(m.closedId(ev('open')), null);
  assert.equal(m.dropClosed(list, m.closedId(ev('open'))), list, 'open: unchanged (same list)');
  assert.equal(m.dropClosed(list, m.closedId(ev('closed', 'other'))), list, 'not in the list: unchanged');
  assert.equal(m.closedId(null), null);
  assert.equal(m.closedId({}), null);
});

test('strip: Done hidden when not dismissable; a 409 use_* switches the button and drops Done', async () => {
  const shown = [];
  const step = item('stopped', { id: 'hs2', human_step_id: 'hs2', action: 'browser', dismissable: false, title: 'Sign in to the exchange' });
  let r1 = mount({ asks: [step] });
  assert.equal(r1.host.querySelector('[data-strip-dismiss]'), null, 'not dismissable: no Done');
  assert.equal(r1.host.querySelector('[data-strip-action]').getAttribute('data-strip-action'), 'browser');
  r1.unmount();
  // An older-server stall with no action the client knows; the server says use the question
  const odd = item('stopped', { id: 'w5', action: null, title: 'Waiting on your answer' });
  const { host, unmount } = mount({ asks: [odd], onDismiss: async () => ({ use: 'answer' }), onShow: async (it) => (shown.push(it.id), true) });
  const q = (s) => host.querySelector(s);
  assert.equal(q('[data-strip-action]'), null);
  q('[data-strip-dismiss]').click();
  await tick();
  await tick();
  assert.equal(q('[data-strip-dismiss]'), null, 'Done goes');
  assert.equal(q('[data-strip-action]').textContent, 'Go to the question');
  assert.equal(q('[data-strip-error]'), null, 'not an error');
  assert.equal(q('[data-strip-hint]').textContent, 'This one clears when you answer it on its card.', 'says why Done went (fix 5)');
  q('[data-strip-action]').click();
  await tick();
  assert.deepEqual(shown, ['w5']);
  unmount();
});

// ---- round 2 addition (Ada, #700 decision): failed hand-off rows ----
const handOff = (over = {}) => item('delegation', { id: 'd1', status: 'failed', kwant_name: 'Builder', title: 'The hand-off to Builder failed', link: link({ thread_id: 't-h' }), ...over });

test('failed hand-off with task_id: Retry, and it retries the board task', async () => {
  const h = handOff({ action: 'retry', task_id: 'tk7', dismissable: true });
  assert.equal(m.askAction(h), 'retry');
  assert.deepEqual(m.retryRoute(h), { via: 'task', id: 'tk7' });
  const calls = [];
  const { host, unmount } = mount({ asks: [h], onAction: async (it, a) => void calls.push(`${a}:${it.id}`) });
  assert.equal(host.querySelector('[data-strip-action]').textContent, 'Retry');
  assert.ok(host.querySelector('[data-strip-dismiss]'));
  host.querySelector('[data-strip-action]').click();
  await tick();
  assert.deepEqual(calls, ['retry:d1']);
  unmount();
});

test('failed hand-off without task_id: no action button, only Done, and the hint says ask again here', () => {
  const h = handOff({ action: null, dismissable: true });
  assert.equal(m.askAction(h), null);
  const { host, unmount } = mount({ asks: [h] });
  assert.equal(host.querySelector('[data-strip-action]'), null);
  assert.ok(host.querySelector('[data-strip-dismiss]'), 'Done, clear it');
  assert.equal(host.querySelector('[data-strip-hint]').textContent, 'There’s no button for this one. Ask Builder again here, or clear it once it’s handled.');
  assert.equal(m.askKind(h, null), 'Failed');
  unmount();
  // the bot's later reply closes it live (waiting_updated, reason later_reply)
  assert.deepEqual(m.dropClosed([h], m.closedId({ item: { id: 'd1', kind: 'delegation', status: 'closed', reason: 'later_reply' } })), []);
});

test('failed routine run without task_id: the hint points at Run now in Routines, not at asking the bot again', () => {
  const r = item('routine_run', { id: 'r1', status: 'error', kwant_name: 'Quant' });
  assert.equal(m.askAction(r), null);
  assert.equal(m.askHint(r, null, 'Quant'), 'There’s no button for this one. Press Run now on it in Routines, or clear it once it’s handled.');
});

test('older server: a hand-off row with no task_id never gets an inferred Retry', () => {
  assert.equal(m.askAction(handOff()), null, 'no action field, no task');
  assert.equal(m.askAction(handOff({ status: 'interrupted' })), null);
  assert.equal(m.askAction(handOff({ action: 'retry' })), null, 'even a retry the server sent needs the task');
  assert.equal(m.askAction(item('routine_run', { status: 'error' })), null);
  assert.equal(m.retryRoute(handOff()), null);
});

// ---- round 2: Mira's design check (design-check.md, fixes 1-9) ----
test('fix 1: the line drops a Failed: / Stuck: prefix the kind word already says', () => {
  assert.equal(m.askLine(item('delegation', { title: 'Stuck: Quant has been on the same step for 25 minutes.' })), 'Quant has been on the same step for 25 minutes.');
  assert.equal(m.askLine(item('stopped', { status: 'failed', title: 'Failed: Backtest maker-first-rebalance.' })), 'Backtest maker-first-rebalance.');
  assert.equal(m.askLine(item('stopped', { title: 'Failed:' })), 'Failed:', 'nothing left: keep it');
  assert.equal(m.askLine(item('delegation', { title: 'Failed: the hand-off stopped.' })), 'The hand-off stopped.', 'starts with a capital');
  assert.equal(m.askLine(stopped), STALL, 'other lines unchanged');
});

test('fixes 3 and 4: no Retry with nowhere to go; no action and not dismissable: no button row and its own hint', async () => {
  // fix 3: the server says retry, but there is no task: no button, only Done
  let r = mount({ asks: [handOff({ action: 'retry', dismissable: true })] });
  assert.equal(r.host.querySelector('[data-strip-action]'), null);
  assert.ok(r.host.querySelector('[data-strip-dismiss]'));
  r.unmount();
  // fix 4
  const lone = item('delegation', { id: 'z', action: null, dismissable: false, title: 'Quant is still on it.' });
  assert.equal(m.askHint(lone, null, 'Quant', false), 'Write to Quant in the chat. This clears once it’s handled.');
  r = mount({ asks: [lone] });
  assert.equal(r.host.querySelector('[data-strip-buttons]'), null, 'no empty button row');
  assert.equal(r.host.querySelector('[data-strip-hint]').textContent, 'Write to Quant in the chat. This clears once it’s handled.');
  r.unmount();
});

test('fixes 2, 7, 8: phone browser step stacks, icons never shrink, compact steppers, a focus ring on the strip', () => {
  const step = item('stopped', { id: 'hs3', human_step_id: 'hs3', action: 'browser', dismissable: true, title: 'Sign in' });
  let r = mount({ asks: [step], phone: true });
  assert.match(r.host.querySelector('[data-strip-buttons]').className, /flex-col-reverse/);
  assert.match(r.host.querySelector('[data-strip-action] svg').getAttribute('class'), /shrink-0/);
  assert.match(r.host.querySelector('[data-needs-you-strip]').className, /kw-focus/);
  assert.doesNotMatch(r.host.querySelector('[data-needs-you-strip]').className, /outline-none/);
  r.unmount();
  r = mount({ asks: [stopped, item('question', { id: 'q2' })], phone: true });
  for (const l of ['Previous ask', 'Next ask']) assert.ok(r.host.querySelector(`[aria-label="${l}"]`).hasAttribute('data-compact'));
  r.unmount();
});

// ha-sheet-polish (b): a Home Assistant lock ask in the strip and the pin tip says the card's sentence.
const lockAsk = item('approval', {
  id: 'h1', kwant_id: 'house', kwant_name: 'House', title: 'Use intent__HassTurnOff (home-assistant)',
  tool_name: 'mcp__home-assistant__intent__HassTurnOff', link: link({ direct_kwant_id: 'house' }),
  details: { source: 'mcp', server: 'home-assistant', tool: 'intent__HassTurnOff', arguments: { name: 'Front door', area: 'Hallway', domain: ['lock'] } },
});

test('strip: a Home Assistant lock ask reads "House wants to unlock Front door", not the raw tool call', () => {
  const d = parse(h(m.NeedsYouStrip, { asks: [lockAsk], onAction: noop, onDismiss: async () => {}, onShow: noop, now: NOW }));
  const line = d.querySelector('[data-strip-line]').textContent;
  assert.equal(line, 'House wants to unlock Front door');
  assert.ok(!/intent__|home-assistant|^Use /.test(line));
  // Other approvals keep their line (askLine): the question, or a title already in words
  const q = item('approval', { title: 'Wants to run a command', question: 'Run “deploy.sh prod”?' });
  assert.equal(m.approvalLine(q) ?? m.askLine(q), 'Run “deploy.sh prod”?');
  const w = item('approval', { title: 'Wants to run a command' });
  assert.equal(m.approvalLine(w) ?? m.askLine(w), 'Wants to run a command');
});

test('pinned row tooltip: a Home Assistant lock ask says the card sentence too', () => {
  const kwants = [bot('house', 'House')];
  const needs = m.needsYouRows([lockAsk], {}, new Set(['house']));
  const html = m.renderToStaticMarkup(h(m.Sidebar, {
    space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
    kwants, channels: [], teams: [], selectedId: null, isChannel: false, onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop,
    onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, onOpenNeedsYou: noop, busyKwants: {}, botStatus: {},
    attention: m.attentionRows({ kwants, needs, now: Date.parse('2026-11-15T17:45:00Z') }),
  }));
  const d = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  assert.equal(d.querySelector('#pin-tip-house span:last-child').textContent, 'House wants to unlock Front door');
});

// 29w (brief: autonomy/needs-you-strip-steps-aside/brief.md): while the chat's only ask is a live approval and its
// card's Allow / Deny row is fully on screen, the Needs-you strip steps aside (it would only repeat the card).
const live = { id: 'ap1', kind: 'approval', kwant_id: 'k1', kwant_name: 'Mia', title: 'Mia wants to run a command', timestamp: '2026-11-19T09:00:00Z' };
const asideWhen = ({ asks, cardInView }) => cardInView && m.onlyLiveApproval(asks);

test('29w: one live approval with its card in view: the strip steps aside; card scrolled off: it is back', () => {
  assert.equal(typeof m.onlyLiveApproval, 'function');
  assert.equal(m.onlyLiveApproval([live]), true);
  assert.equal(m.onlyLiveApproval([{ ...live, action: 'decide' }]), true);
  assert.equal(asideWhen({ asks: [live], cardInView: true }), true);
  assert.equal(asideWhen({ asks: [live], cardInView: false }), false);
});

test('29w: an approval plus a second ask, a run-out approval, or no asks: the strip is not hidden', () => {
  const q = { id: 'q1', kind: 'question', kwant_id: 'k1', title: 'Which folder?', timestamp: '2026-11-19T09:01:00Z' };
  const ranOut = { id: 's1', kind: 'stopped', kwant_id: 'k1', title: 'Approval ran out', retry: { type: 'approval', approval_id: 'ap0' }, timestamp: '2026-11-19T09:00:00Z' };
  assert.equal(m.onlyLiveApproval([live, q]), false);
  assert.equal(m.onlyLiveApproval([live, { ...live, id: 'ap2' }]), false);
  assert.equal(m.onlyLiveApproval([ranOut]), false);
  assert.equal(m.onlyLiveApproval([{ ...live, action: 'ask_again' }]), false);
  assert.equal(m.onlyLiveApproval([q]), false);
  assert.equal(m.onlyLiveApproval([]), false);
  assert.equal(asideWhen({ asks: [live, q], cardInView: true }), false);
  assert.equal(asideWhen({ asks: [ranOut], cardInView: true }), false);
});

test('29w: App passes stripOnlyApproval; ChatView keeps approvalCardInView from the same measurement and gates the strip', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(app, /stripOnlyApproval=\{onlyLiveApproval\(asks\)\}/);
  // Set in checkApprovalVisible from the same buttonsVisible the "Approval waiting" pill uses: no new listeners.
  const check = chat.slice(chat.indexOf('const checkApprovalVisible'), chat.indexOf('const onScroll'));
  assert.match(check, /const buttonsVisible = rows\.some\(\(r\) => fullyVisible\(/);
  assert.match(check, /setApprovalCardInView\(buttonsVisible\)/);
  assert.match(chat, /const stripAside = stripStepsAside\(\{ strip: !!needsYouStrip, onlyApproval: stripOnlyApproval, cardInView: approvalCardInView, followingCard \}\);/);
  // Round 3 (Rigel): followingCard comes from the same measurement, after onScroll has updated stickRef.
  assert.match(check, /setFollowingCard\(stickRef\.current && rows\.length > 0\)/);
  // Kept mounted (no re-enter animation, its state stays) and hidden at once: no new motion.
  assert.match(chat, /className=\{stripAside \? 'hidden' : 'contents'\}.*\n\s*\{needsYouStrip\}\n\s*<\/div>/);
  assert.doesNotMatch(chat, /\n\s*\{needsYouStrip\}\n\n\s*<form/);
  // Round 2 (Mira): only keyboard focus hands off to the composer; a tap (Android focuses the button) must not
  // raise the soft keyboard over Allow / Deny.
  assert.match(chat, /onFocus=\{\(e\) => \(stripHadFocusRef\.current = \(e\.target as HTMLElement\)\.matches\(':focus-visible'\)\)\}/);
  assert.doesNotMatch(chat, /stripHadFocusRef\.current = true/);
});

// Round 3 (Rigel's QA at 6168ce1b): bringing the strip back within the follow zone shrank the chat, the follow pin
// snapped to the bottom, the card was in view again and the strip hid: a slow scroll up could not leave the bottom.
test('29w: while the chat follows the bottom with the card in it, the strip stays aside; past the follow zone it is back', () => {
  const base = { strip: true, onlyApproval: true };
  assert.equal(typeof m.stripStepsAside, 'function');
  // At the bottom: card in view.
  assert.equal(m.stripStepsAside({ ...base, cardInView: true, followingCard: true }), true);
  // 32 px up: the buttons are clipped but the chat still follows: stays aside (was: back, then snapped down).
  assert.equal(m.stripStepsAside({ ...base, cardInView: false, followingCard: true }), true);
  // Past 80 px up: no longer following, buttons off screen: back.
  assert.equal(m.stripStepsAside({ ...base, cardInView: false, followingCard: false }), false);
  // Read up with the card still fully in view: aside, as before.
  assert.equal(m.stripStepsAside({ ...base, cardInView: true, followingCard: false }), true);
  // Never aside without a strip, or when the ask is not one live approval.
  assert.equal(m.stripStepsAside({ ...base, strip: false, cardInView: true, followingCard: true }), false);
  assert.equal(m.stripStepsAside({ ...base, onlyApproval: false, cardInView: true, followingCard: true }), false);
});

// 30a (brief: autonomy/approval-one-pointer/brief.md; Mira's 29w design check, small things a and b).
// b: one pointer per card. The "↓ Approval waiting" pill steps aside while the Needs-you strip offers
// "↑ Go to the approval" for this chat's live approval, as the working line's "Answer ↑" already does.
test('30a: the "Approval waiting" pill steps aside while the strip offers Go to the approval', () => {
  assert.equal(typeof m.approvalPillShows, 'function');
  const base = { offscreen: true, waiting: 1, phoneAnswerUp: false, stripHasApproval: false };
  // No strip about it: the pill shows while the card's buttons are off screen (old 22i / 0.8.1 F6 behaviour).
  assert.equal(m.approvalPillShows(base), true);
  assert.equal(m.approvalPillShows({ ...base, waiting: 2 }), true);
  // b-1440: the strip says "Go to the approval": the pill steps aside (it pointed the other way at the same card).
  assert.equal(m.approvalPillShows({ ...base, stripHasApproval: true }), false);
  // Phone: "Answer ↑" stands in for it, as before.
  assert.equal(m.approvalPillShows({ ...base, phoneAnswerUp: true }), false);
  // Buttons in view, or nothing waiting: no pill.
  assert.equal(m.approvalPillShows({ ...base, offscreen: false }), false);
  assert.equal(m.approvalPillShows({ ...base, waiting: 0 }), false);
  // Which strips count: a live approval of this chat, alone or with other asks. A question alone, a run-out
  // approval (its Ask again is a different action) or no asks leave the pill where it was.
  const q = { id: 'q1', kind: 'question', kwant_id: 'k1', title: 'Which folder?', timestamp: '2026-11-19T09:01:00Z' };
  const ranOut = { id: 's1', kind: 'stopped', kwant_id: 'k1', title: 'Approval ran out', retry: { type: 'approval', approval_id: 'ap0' }, timestamp: '2026-11-19T09:00:00Z' };
  assert.equal(m.stripHasLiveApproval([live]), true);
  assert.equal(m.stripHasLiveApproval([q, live]), true);
  assert.equal(m.stripHasLiveApproval([q]), false);
  assert.equal(m.stripHasLiveApproval([ranOut]), false);
  assert.equal(m.stripHasLiveApproval([{ ...live, action: 'ask_again' }]), false);
  assert.equal(m.stripHasLiveApproval([]), false);
});

// a: the approval card is the landing place: script-focusable only, named by its own heading and ask (no new copy).
test('30a: the approval card can take focus without a tab stop, and is named by its heading and ask', () => {
  const approval = { id: 'ap1', tool_name: 'execute_shell', details: { source: 'local_shell', tool: 'execute_shell', arguments: { command: 'curl -sI https://example.com', cwd: '/tmp' }, kwant_name: 'Mia' } };
  const html = m.renderToStaticMarkup(h(m.ModernApprovalCard, { approval, onResolve: noop }));
  const d = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const card = d.querySelector('[data-approval-card="ap1"]');
  assert.equal(card.getAttribute('tabindex'), '-1');
  assert.equal(card.getAttribute('role'), 'group');
  const name = card.getAttribute('aria-labelledby').split(' ').map((id) => d.getElementById(id)?.textContent.trim());
  assert.equal(name.length, 2);
  assert.equal(name[0], 'Approval needed');
  assert.equal(name[1], d.querySelector('[data-approval-summary]').textContent.trim());
  assert.match(name[1], /wants to/);
  // Allow / Deny stay ordinary tab stops after the card (no tabindex games on them).
  for (const b of card.querySelectorAll('[data-approval-actions] button')) assert.equal(b.getAttribute('tabindex'), null);
});

test('30a: keyboard "Go to the approval" (strip, pill, Answer ↑) focuses the card, never Allow / Deny; a tap moves nothing', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const floor = readFileSync(new URL('../src/components/dev/ProdFloorCard.tsx', import.meta.url), 'utf8');
  assert.match(app, /stripHasApproval=\{stripHasLiveApproval\(asks\)\}/);
  // The pill's show rule is the helper (one pointer per card).
  assert.match(chat, /\{approvalPillShows\(\{ offscreen: approvalOffscreen, waiting: pendingApprovals\.length, phoneAnswerUp: phone && stripWaitsOn === 'approval', stripHasApproval \}\) && \(/);
  // Pill and "Answer ↑" go through showApproval: focus the card only when a button had keyboard focus.
  const show = chat.slice(chat.indexOf('const showApproval = () =>'), chat.indexOf('const stripAside ='));
  assert.match(show, /const keyboard = from instanceof HTMLButtonElement && from\.matches\(':focus-visible'\);/);
  assert.match(show, /if \(keyboard\) card\.focus\(\{ preventScroll: true \}\);/);
  // The strip's Go to goes through the highlight effect: only its own decide button with keyboard focus moves it.
  const hl = chat.slice(chat.indexOf('// 0.8.1 F6: an approval\'s card'), chat.indexOf('if (!messages.some((m) => m.id === highlightMessageId)) return;'));
  assert.match(hl, /stripSlotRef\.current\?\.contains\(from\) && from\.matches\('\[data-strip-action="decide"\]:focus-visible'\)\) card\.focus\(\{ preventScroll: true \}\)/);
  // Never Allow / Deny: nothing in these paths queries the card's buttons to focus them.
  assert.doesNotMatch(show + hl, /data-approval-actions|button.*\.focus\(/);
  // The 29w hand-off to the composer still only runs when focus was lost (body) or is still in the hidden strip.
  assert.match(chat, /if \(!a \|\| a === document\.body \|\| stripSlotRef\.current\?\.contains\(a\)\) textareaRef\.current\?\.focus\(\{ preventScroll: true \}\);/);
  // The flash wrapper is always there, so flashing the card on landing doesn't remount it (and drop its focus).
  // 30j: the flash only toggles kw-flash on the same wrapper (see the 30h test for the exact classes).
  assert.match(chat, /<div\s+key=\{approval\.id\}\s+className=\{`[^`]*\$\{flashId === approval\.id \? ' kw-flash' : ''\}`\}[^>]*>\n\s*<ModernApprovalCard approval=\{approval\} onResolve=\{onResolveApproval\} \/>\n\s*<\/div>/);
  assert.doesNotMatch(chat, /<ModernApprovalCard key=\{approval\.id\}/);
  // The dev production-floor card is a landing place too.
  assert.match(floor, /data-approval-card=\{id\}[\s\S]{0,400}tabIndex=\{-1\}/);
});

// 30h (brief: autonomy/approval-flash-fits-card/brief.md; Mira's 30a design check "Small things c", Rigel's 30a QA
// observation 1): at 1440 the amber flash box ran to the chat column's edge while the card stops at its max width.
test('30h: the approval flash box is as wide as the card, and adds nothing to its height', () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const card = readFileSync(new URL('../src/components/ModernApprovalCard.tsx', import.meta.url), 'utf8');
  const flash = chat.match(/<div\s+key=\{approval\.id\}\s+className=\{`([^`$]*)\$\{flashId === approval\.id \? '([^']*)' : ''\}`\}[^>]*>\s*<ModernApprovalCard /);
  assert.ok(flash, 'the approval card sits in the flash wrapper');
  const [, idle, toggled] = flash;
  // 30j: one box idle and flashing; only kw-flash toggles (the brief's "no jump" rule; behaviour in approvalFlashNoJump).
  assert.equal(toggled, ' kw-flash');
  const on = `${idle}${toggled}`;
  const onCls = on.split(/\s+/).filter(Boolean);
  assert.ok(onCls.includes('kw-flash') && onCls.includes('rounded-xl'), on);
  assert.ok(onCls.includes('!mt-0'), 'the wrapper cancels the list gap, so the card keeps its idle spot');
  // The card root's own max width, read from the card, is on the flashing wrapper too.
  const root = card.match(/data-approval-card=\{approval\.id\}[\s\S]{0,300}?className=\{`([^`$]*)/);
  assert.ok(root, 'card root class');
  const maxW = root[1].split(/\s+/).filter((c) => /^max-w-/.test(c));
  assert.equal(maxW.length, 1, root[1]);
  assert.ok(onCls.includes(maxW[0]), `flash wrapper "${on}" lacks the card's ${maxW[0]}`);
  // Top and bottom: the card's my-3 collapses through the wrapper only while the wrapper has no margin, padding,
  // border or its own formatting context, so none of those may join the wrapper (its one margin class is !mt-0).
  for (const c of onCls.filter((c) => c !== '!mt-0')) assert.doesNotMatch(c, /^!?(-?m[trblxy]?|p[trblxy]?|border|ring|flex|grid|inline|overflow|flow-root|w)-|^!?(flex|grid|inline-block|flow-root|border|contents)$/, c);
});
