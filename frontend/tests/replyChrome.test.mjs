// reply-chrome (autonomy/reply-chrome/design.md, Mira; UX audit #10): a done reply looks done, a reply asking for
// an OK gets Approve / Change…, Copy is a quiet icon, and the name line is "Name · time · Details" with routing,
// engine and cost in words inside Details.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

process.env.TZ = 'UTC';
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export * from './src/replyDone';
  export * from './src/approvalAsk';
  export { routeWhyText, routeReasonText, ROUTER_NAME } from './src/routeReason';
  export { routingLine } from './src/spaces';
  export { replyCostWords, ranOnWords, replyTokens } from './src/modelUsage';
`);
const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ReplyMeta } from './src/components/ReplyMeta';
  import { DoneMarker, DoneCheckLine } from './src/components/DoneMarker';
  import { ApproveRow } from './src/components/ApproveRow';
  import { CopyGhost } from './src/components/CopyGhost';
  export { createRoot } from 'react-dom/client';
  export const html = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export { React, flushSync, ReplyMeta, DoneMarker, DoneCheckLine, ApproveRow, CopyGhost };
`);

const mount = (Comp, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  ui.flushSync(() => root.render(ui.React.createElement(Comp, props)));
  return { host, unmount: () => (root.unmount(), host.remove()) };
};
const tick = () => new Promise((r) => setTimeout(r, 0));
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await tick();
  await tick();
};

// ---- 1. Done marker ----

const reply = (o = {}) => ({ id: 'm1', sender_role: 'kwant', text: 'Here are both prices.', meta: null, ...o });

test('doneLineOf reads the last line like keep_going::done_line', () => {
  assert.equal(m.doneLineOf('Compared them.\n\nDone: compared both shops'), 'compared both shops');
  assert.equal(m.doneLineOf('x\n**Done:** compared both shops\n\n'), 'compared both shops');
  assert.equal(m.doneLineOf('x\n> done: it ships'), 'it ships');
  assert.equal(m.doneLineOf('Done: early\nThen more words.'), null, 'only the last line counts');
  assert.equal(m.doneLineOf('Done:'), null, 'nothing after the head');
  assert.equal(m.doneLineOf('Undone: no'), null);
  assert.equal(m.doneLineOf(''), null);
});

test('doneOf: chain_done stamp, a Done: last line, or an Inbox done item; first match wins', () => {
  const stamped = m.doneOf(reply({ meta: JSON.stringify({ chain_done: { text: 'Compared both shops’ 1 kg price', at: '2026-11-13T10:00:00Z' } }) }), null);
  assert.deepEqual(stamped, { text: 'Compared both shops’ 1 kg price', checked: null });
  assert.deepEqual(m.doneOf(reply({ text: 'Both found.\nDone: priced both' }), null), { text: 'priced both', checked: null });
  const items = m.doneItemsOf([
    { group: 'finished', result_message_id: 'm1', checked: { state: 'checked', items: [], items_dropped: 0, line: 'Checked: every check passed.' } },
    { group: 'waiting_on_you', result_message_id: 'm2' },
  ]);
  const fromInbox = m.doneOf(reply(), items);
  assert.equal(fromInbox.text, null);
  assert.equal(fromInbox.checked.state, 'checked');
  assert.equal(m.doneOf(reply({ id: 'm2' }), items), null, 'a waiting item is not done');
  assert.equal(m.doneOf(reply({ id: 'm3' }), items), null, 'a plain reply has no marker');
});

test('doneOf: never on a reply that asks, a blocked or interrupted one, a temp, or a user message', () => {
  const done = { text: 'x\nDone: shipped it' };
  assert.equal(m.doneOf(reply({ ...done }), null, { asks: true }), null);
  assert.equal(m.doneOf(reply({ ...done, meta: JSON.stringify({ blocked: { text: 'needs a key' } }) }), null), null);
  assert.equal(m.doneOf(reply({ ...done, meta: JSON.stringify({ interrupted: true }) }), null), null);
  assert.equal(m.doneOf(reply({ ...done, id: 'temp-1' }), null), null);
  assert.equal(m.doneOf(reply({ ...done, sender_role: 'user' }), null), null);
  assert.equal(m.doneOf(reply({ ...done }), null, { streaming: true }), null);
});

test('doneItemsOf keeps finished and to-review items with a result message only', () => {
  const map = m.doneItemsOf([
    { group: 'to_review', result_message_id: 'a' },
    { group: 'finished', result_message_id: null },
    { group: 'failed_or_stuck', result_message_id: 'b' },
  ]);
  assert.deepEqual([...map.keys()], ['a']);
});

test('DoneMarker: a teal "Done" pill, labelled, no motion; holo only for Checked; check words match the Inbox', () => {
  const plain = ui.html(ui.DoneMarker, { done: { text: 'Priced both', checked: null } });
  assert.match(plain, /role="img"/);
  assert.match(plain, /aria-label="Done: this job is finished"/);
  assert.match(plain, /title="Priced both"/);
  assert.match(plain, /text-ok/);
  assert.match(plain, />Done</);
  assert.doesNotMatch(plain, /kw-sticker|animate|transition|tabindex|<button/);
  const chk = (state) => ({ text: null, checked: { state, items: [], items_dropped: 0, line: '' } });
  assert.match(ui.html(ui.DoneMarker, { done: chk('checked') }), /kw-sticker-holo[\s\S]*Checked/);
  for (const [state, word, tone] of [['partly', 'Partly checked', 'text-warn'], ['failed', 'Check failed', 'text-bad'], ['not_checked', 'Not checked', 'text-zinc-300']]) {
    const h = ui.html(ui.DoneMarker, { done: chk(state) });
    assert.match(h, new RegExp(`>${word}<`), state);
    assert.match(h, new RegExp(tone), state);
    assert.doesNotMatch(h, /kw-sticker/, `${state} is never holo`);
  }
});

test('DoneCheckLine: only with a check; the server line, or done-check’s default sentence', () => {
  assert.equal(ui.html(ui.DoneCheckLine, { done: { text: null, checked: null } }), '');
  const h = ui.html(ui.DoneCheckLine, { done: { text: null, checked: { state: 'partly', items: [], items_dropped: 0, line: 'Partly checked: 1 of 2 prices matched the shop page.' } } });
  assert.match(h, /data-done-check-line="partly"/);
  assert.match(h, /<b class="[^"]*text-warn[^"]*">Partly checked:<\/b> 1 of 2 prices matched the shop page\./);
  const d = ui.html(ui.DoneCheckLine, { done: { text: null, checked: { state: 'not_checked', items: [], items_dropped: 0, line: '' } } });
  assert.match(d, /Not checked:<\/b> nothing verified this result\./);
});

// ---- 2. Approve / Change… ----

test('asksApproval: the design’s true and false cases', () => {
  for (const t of ['I drafted the newsletter. It goes out once you approve the draft.', 'The post is ready. Shall I send it?', 'Waiting for your go-ahead.', 'All set. Should I publish it on Friday?', 'OK to schedule it for 9:00?', 'I’ll send it once you’ve confirmed.']) {
    assert.equal(m.asksApproval(t), true, t);
  }
  for (const t of ['I approved the refund.', 'No need to approve anything.', 'You can approve it later in Settings, but it’s live now.', 'Here are both prices.', '', null]) {
    assert.equal(m.asksApproval(t), false, String(t));
  }
});

test('asksApproval looks only at the last two sentences', () => {
  assert.equal(m.asksApproval('Shall I send it? You said yes. I sent it. It went out at 9:00.'), false);
  assert.equal(m.asksApproval('Here is the draft.\n\n**Subject:** Spring sale\n\nShall I send it?'), true);
  assert.equal(m.asksApproval('Once you approve, I send it.\n\n```\nonce you approve\n```\nIt is live now. Nothing else to do.'), false);
});

test('ApproveRow: a labelled group, Approve first, Change… second; both call back and lock while busy', async () => {
  const calls = [];
  const { host, unmount } = mount(ui.ApproveRow, { name: 'Ben', onApprove: () => calls.push('approve'), onChange: () => calls.push('change') });
  const group = host.querySelector('[role=group]');
  assert.equal(group.getAttribute('aria-label'), 'Ben is waiting for your OK');
  assert.match(group.textContent, /Ben is waiting for your OK/);
  const [a, c] = group.querySelectorAll('button');
  assert.equal(a.textContent, 'Approve');
  assert.equal(c.textContent, 'Change…');
  assert.match(a.className, /h-11 sm:h-8/);
  assert.match(a.className, /focus-visible:ring-2/);
  await click(a);
  await click(c);
  assert.deepEqual(calls, ['approve', 'change']);
  unmount();
  const busy = ui.html(ui.ApproveRow, { name: 'Ben', busy: true, onApprove() {}, onChange() {} });
  assert.equal((busy.match(/disabled=""/g) ?? []).length, 2);
});

test('approval words', () => {
  assert.equal(m.APPROVE_TEXT, 'Approved, go ahead.');
  assert.equal(m.changePlaceholder('Ben'), 'What should Ben change?');
});

// ---- 3. Copy ----

test('CopyGhost: an icon button named "Copy reply" with a tooltip, no border, 44 px on phone', () => {
  const h = ui.html(ui.CopyGhost, { copied: false, tipId: 't1', onCopy() {} });
  assert.match(h, /aria-label="Copy reply"/);
  assert.match(h, /aria-describedby="t1"/);
  assert.match(h, /id="t1" role="tooltip"/);
  assert.match(h, /h-11 w-11 sm:h-8 sm:w-8/);
  assert.doesNotMatch(h, /<button[^>]*class="[^"]*\bborder\b/, 'the button has no border');
  assert.doesNotMatch(h, /title=/);
  assert.doesNotMatch(h, /Copy markdown/);
  assert.match(h, /\[@media\(hover:hover\)\]:peer-hover:opacity-100 peer-focus-visible:opacity-100/);
  const done = ui.html(ui.CopyGhost, { copied: true, tipId: 't1', onCopy() {} });
  assert.match(done, /text-ok/);
  assert.match(done, /role="tooltip"[^>]*class="[^"]*opacity-100[^"]*">Copied</);
  assert.match(done, /aria-live="polite">Copied</);
});

test('ChatView: the action row is Reply, thumbs, Copy, then smart actions', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const row = src.slice(src.indexOf('{/* Action row (reply-chrome part 3)'));
  const at = (s) => row.indexOf(s);
  assert.ok(at('<ReplyButton') > 0 && at('<ReplyButton') < at('<ReactionButtons'));
  assert.ok(at('<ReactionButtons') < at('<CopyGhost'));
  assert.ok(at('<CopyGhost') < at('<SmartActionsSlot'));
  assert.doesNotMatch(src, /Copy markdown/);
  assert.doesNotMatch(src, /routeChipLabel/);
});

// ---- 4. Name line and Details in words ----

test('routeWhyText: a keyword fallback in words; other picks as routeReasonText', () => {
  assert.equal(
    m.routeWhyText({ kwantName: 'Cody', fallback: true, basis: 'keywords', matchedTerms: ['newsletter', 'draft'] }),
    'Laya, who picks the bot, was offline, so Cody was picked by matching words in your message: newsletter, draft.',
  );
  assert.equal(m.routeWhyText({ kwantName: 'Cody', fallback: true }), 'Laya, who picks the bot, was offline, so Cody was picked by matching words in your message.');
  assert.equal(m.routeWhyText({ kwantName: 'Ben', basis: 'ask' }), 'You picked Ben.');
  assert.equal(m.routeWhyText({ kwantName: 'Ann', basis: 'lead' }), 'No one’s role matched this, so it went to the team lead, Ann.');
  assert.equal(m.routeWhyText({ kwantName: 'Ben', basis: 'skills', reason: 'Ben compares shop prices' }), 'Ben compares shop prices.');
  assert.equal(m.routeWhyText({ kwantName: 'Ben', reason: 'Laya picked Ben from 3 bots' }), 'Laya picked Ben for this.');
  assert.doesNotMatch(m.routeWhyText({ kwantName: 'Cody', fallback: true, matchedTerms: ['x'] }), /By keywords/);
  assert.equal(m.ROUTER_NAME, 'Laya');
  assert.match(m.routingLine({ kwant_name: 'Cody', fallback: true }, 'Home'), /\(Laya is offline\)/, 'routingLine uses the same name');
});

test('ranOnWords: "Claude Sonnet 5.5, on your Work account"', () => {
  assert.equal(m.ranOnWords('Claude · Sonnet 5.5', 'Work account'), 'Claude Sonnet 5.5, on your Work account');
  assert.equal(m.ranOnWords('Claude · Sonnet 5.5', 'Booktuning'), 'Claude Sonnet 5.5, on the Booktuning account');
  assert.equal(m.ranOnWords('Codex', null), 'Codex');
  assert.equal(m.ranOnWords(null, null), null);
});

test('replyCostWords: cost and new work, cache reads as a note; never "fresh" or "cache"', () => {
  const meta = (u) => JSON.stringify({ usage: u });
  const full = m.replyCostWords(meta({ fresh_input: 9120, cache_write: 1880, output: 812, cache_read: 38400, cost_usd: 0.0412 }));
  assert.deepEqual(full, { text: 'About $0.04 · 11,812 tokens of new work', note: '(plus 38,400 re-read from earlier at a small part of the price)' });
  assert.equal(m.replyCostWords(meta({ fresh_input: 10, cache_write: 0, output: 5, cache_read: 0, cost_usd: 0.002 })).text, 'Under 1¢ · 15 tokens of new work');
  assert.deepEqual(m.replyCostWords(meta({ fresh_input: 10, cache_write: 0, output: 5, cache_read: 0 })), { text: '15 tokens of new work', note: null });
  assert.equal(m.replyCostWords(null), null);
  assert.doesNotMatch(JSON.stringify(full), /fresh|cache|input|output/);
});

test('ReplyMeta: the line is Name, Done, time, Details; no route chip or engine chip unless the engine switched', () => {
  const props = {
    name: 'Cody',
    at: '2026-11-13T16:24:00Z',
    now: Date.parse('2026-11-13T17:00:00Z'),
    ranOn: 'Claude · Sonnet 5.5',
    account: 'Work account',
    tokens: { label: '11.8k fresh', detail: '11,812 fresh tokens: …' },
    cost: { text: 'About $0.04 · 11,812 tokens of new work', note: '(plus 38,400 re-read from earlier at a small part of the price)' },
    route: { text: 'Laya, who picks the bot, was offline, so Cody was picked by matching words in your message: newsletter, draft.', fallback: true },
    done: { text: 'Drafted it', checked: null },
  };
  const line = ui.html(ui.ReplyMeta, props);
  assert.doesNotMatch(line, /By keywords|Matched words|data-routing-line|data-engine-chip|Sonnet/);
  assert.match(line, />Cody<\/span><span role="img" aria-label="Done: this job is finished"/, 'Done right after the name');
  assert.match(line, /data-reply-time/);
  const switched = ui.html(ui.ReplyMeta, { ...props, handoff: { from: 'Claude', to: 'Codex', why: 'Claude is out' } });
  assert.match(switched, /data-engine-chip/);
});

test('ReplyMeta: Details opens to Why, Ran on and Cost in words, with Wrong bot?', async () => {
  const picks = [];
  const { host, unmount } = mount(ui.ReplyMeta, {
    name: 'Cody',
    ranOn: 'Claude · Sonnet 5.5',
    account: 'Work account',
    tokens: { label: '11.8k fresh', detail: '11,812 fresh tokens: 9,120 new input' },
    cost: { text: 'About $0.04 · 11,812 tokens of new work', note: '(plus 38,400 re-read from earlier at a small part of the price)' },
    route: {
      text: 'Laya, who picks the bot, was offline, so Cody was picked by matching words in your message: newsletter, draft.',
      fallback: true,
      reroute: { key: 'm1', targets: [{ id: 'k2', name: 'Ann', title: 'Lead' }], onPick: async (id) => void picks.push(id) },
    },
  });
  const toggle = [...host.querySelectorAll('button')].find((b) => /Details/.test(b.textContent));
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  await click(toggle);
  const panel = host.querySelector('[data-reply-details]');
  const rows = [...panel.querySelectorAll('dt')].map((dt) => [dt.textContent, dt.nextElementSibling.textContent]);
  assert.deepEqual(rows.map(([k]) => k), ['Why Cody', 'Ran on', 'Cost']);
  assert.match(rows[0][1], /was offline, so Cody was picked by matching words in your message: newsletter, draft\.Wrong bot\?/);
  assert.equal(rows[1][1], 'Claude Sonnet 5.5, on your Work account');
  assert.equal(rows[2][1], 'About $0.04 · 11,812 tokens of new work (plus 38,400 re-read from earlier at a small part of the price)');
  assert.equal(panel.querySelector('dd span[title]').getAttribute('title'), '11,812 fresh tokens: 9,120 new input', 'the full split stays in the tooltip');
  assert.doesNotMatch(panel.textContent.replace('Wrong bot?', ''), /fresh|cache|By keywords/);
  await click(panel.querySelector('[data-wrong-bot]'));
  const picker = host.querySelector('[aria-label="Send to another bot"]');
  assert.ok(picker, 'Wrong bot? opens the picker');
  await click([...picker.querySelectorAll('button')].find((b) => b.textContent.startsWith('Ann')));
  assert.deepEqual(picks, ['k2']);
  assert.match(host.textContent, /Sent to Ann\./);
  unmount();
});
