// 22i (brief: autonomy/attention-shows-the-ask/brief.md): every Needs-attention row lands on and rings its
// ask; the "Needs you" strip above the composer says the whole ask, its one action and "Done, clear it",
// steps through several, shows when the message isn't rendered, and goes when the item closes.
import assert from 'node:assert/strict';
import test from 'node:test';
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
  export { NeedsYouStrip } from './src/components/NeedsYouStrip';
  export { Sidebar } from './src/components/Sidebar';
  export { needsYouRows, attentionRows } from './src/chatsTab';
  export { ringElement } from './src/ring';
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

test('Retry without the server endpoint resends the notice target, else the newest message of yours', () => {
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

test('ring: a card outside the message list gets the amber kw-flash, tagged as an ask', () => {
  const el = document.createElement('div');
  document.body.append(el);
  m.ringElement(el);
  assert.ok(el.classList.contains('kw-flash'), 'kw-flash: a static outline under reduced motion, never nothing');
  assert.equal(el.dataset.flashKind, 'ask');
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
