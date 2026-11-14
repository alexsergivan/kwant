// Fix it "Send to…" (Mira, autonomy/fix-it-send-to/design.md; Juno's #609 /recipients API). The pure helpers,
// the review card's "Goes to" line, picker and Fix it, and the line-notes bar, against fake routes.
// Demo data only (bots Ana, Ben, Cy, Mo; team Edge); no network.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'PointerEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
if (!g.CSS) g.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/dev/recipients';
  export { parseRecipients, parseReview, finding } from './src/dev/types';
  export { sendLabel, parseChanges, parseNote } from './src/dev/lineNotes';
  export { LineNotesError } from './src/dev/lineNotesApi';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { ChangesSection } from './src/components/dev/LineNotes';
  export * as fx from './src/dev/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const click = (el) => el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
const key = (el, k) => el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));

// ---------------------------------------------------------------------------------------------------
// Demo data

const ANA = { id: 'bot_ana', name: 'Ana', team: 'Edge' };
const BEN = { id: 'bot_ben', name: 'Ben', team: 'Edge' };
const CY = { id: 'bot_cy', name: 'Cy' };
const MO = { id: 'bot_mo', name: 'Mo', team: 'Ops' };
const LEAD = { default: { ...ANA, via: 'team_lead' }, nobody: null, bots: [ANA, BEN, CY, MO] };
const AUTHOR = { default: { id: 'bot_pixel', name: 'Pixel', via: 'author' }, nobody: null, bots: [{ id: 'bot_pixel', name: 'Pixel' }, ANA, BEN] };
const NONE = { default: null, nobody: null, bots: [ANA, BEN, CY] };
const EMPTY = { default: null, nobody: 'There’s no bot in this space to send it to. Add one in Manage team.', bots: [] };

const rec = (raw) => m.parseRecipients(raw);

// ---------------------------------------------------------------------------------------------------
// Pure helpers

test('whyLabel: wrote it, lead of the team, your pick', () => {
  assert.equal(m.whyLabel(rec(AUTHOR).default, null), 'wrote it');
  assert.equal(m.whyLabel(rec(LEAD).default, null), 'lead of Edge');
  assert.equal(m.whyLabel(rec(LEAD).default, BEN), 'your pick');
  assert.equal(m.whyLabel(rec(LEAD).default, ANA), 'lead of Edge', 'picking the default is no pick');
  assert.equal(m.whyLabel(null, null), null);
  assert.equal(m.whyLabel(null, CY), 'your pick');
});

test('rowMeta and pickerRows: the default pinned first, its team owns the repo', () => {
  const r = rec({ ...LEAD, bots: [BEN, CY, ANA, MO] });
  assert.deepEqual(m.pickerRows(r).map((b) => b.name), ['Ana', 'Ben', 'Cy', 'Mo']);
  assert.equal(m.rowMeta(ANA, r.default), 'Default · lead of Edge, which owns this repo');
  assert.equal(m.rowMeta(BEN, r.default), 'Edge · owns this repo');
  assert.equal(m.rowMeta(MO, r.default), 'Ops');
  assert.equal(m.rowMeta(CY, r.default), null);
  assert.equal(m.rowMeta({ id: 'bot_pixel', name: 'Pixel' }, rec(AUTHOR).default), 'Default · wrote it');
  assert.deepEqual(m.pickerRows(rec(NONE)).map((b) => b.name), ['Ana', 'Ben', 'Cy']);
});

test('filterBots matches name or team, any case; noMatch quotes the query', () => {
  assert.deepEqual(m.filterBots([ANA, BEN, CY, MO], 'edge').map((b) => b.name), ['Ana', 'Ben']);
  assert.deepEqual(m.filterBots([ANA, BEN, CY, MO], ' mO ').map((b) => b.name), ['Mo']);
  assert.equal(m.filterBots([ANA], '').length, 1);
  assert.equal(m.noMatch(' xyz '), 'No bot matches "xyz".');
});

test('keepPick drops a pick that left the list, and a pick of the default', () => {
  const r = rec(LEAD);
  assert.deepEqual(m.keepPick(BEN, r), BEN);
  assert.equal(m.keepPick({ id: 'bot_gone', name: 'Gone' }, r), null);
  assert.equal(m.keepPick(ANA, r), null);
  assert.equal(m.keepPick(null, r), null);
});

test('parseRecipients skips bad rows and reads the default', () => {
  const r = m.parseRecipients({ default: { id: 'bot_ana', name: 'Ana', via: 'team_lead', team: 'Edge' }, nobody: null, bots: [ANA, { id: 'x' }, 3, BEN] });
  assert.equal(r.default.via, 'team_lead');
  assert.equal(r.default.team, 'Edge');
  assert.deepEqual(r.bots.map((b) => b.id), ['bot_ana', 'bot_ben']);
  assert.equal(m.parseRecipients({ nope: 1 }), null);
  assert.equal(m.parseRecipients({ default: null, nobody: 'No bots.', bots: [] }).nobody, 'No bots.');
});

test('a finding reads sent_via and sent_to_id', () => {
  const f = m.finding({ id: 'f1', severity: 'high', title: 'x', state: 'sent', sent_to: 'Ben', sent_to_id: 'bot_ben', sent_via: 'chosen', task_id: 'tsk_1' });
  assert.equal(f.sent_via, 'chosen');
  assert.equal(f.sent_to_id, 'bot_ben');
});

test('the copy: bad recipient lines, the no-default banner, the send label', () => {
  assert.equal(m.badRecipientFix('Mo'), 'Couldn’t send it: Mo isn’t in this space. Pick another bot and press Fix it again.');
  assert.equal(m.badRecipientFix(null), 'Couldn’t send it: That bot isn’t in this space. Pick another bot and press Fix it again.');
  assert.equal(m.badRecipientNotes('Mo'), 'The notes didn’t send: Mo isn’t in this space. Pick another bot.');
  assert.match(m.noDefaultNotes('sam'), /^This PR is by sam and no team owns this repo, so notes have no default bot\./);
  assert.equal(m.sendLabel(2, 'Ana'), 'Send 2 notes to Ana');
  assert.equal(m.sendLabel(2, null), 'Send 2 notes…');
  assert.equal(m.sendLabel(1, null), 'Send note…');
  assert.equal(m.goesLabel('Ana', 'lead of Edge'), 'Fixes go to Ana, lead of Edge. Change');
});

// ---------------------------------------------------------------------------------------------------
// The review card

const RAW = m.fx.REVIEW_294; // f1 and f3 open, f2 sent
const CARD = m.parseReview(RAW);
let root = null;
let host = null;

function fakeRoutes({ list = LEAD, fail = false } = {}) {
  const s = { list, fail, gets: 0, acts: [], throwOnce: null, reply: null };
  const recipients = async () => {
    s.gets++;
    if (s.fail) throw new Error('offline');
    const r = m.parseRecipients(s.list);
    return r;
  };
  const onFinding = async (cardId, fid, action) => {
    s.acts.push({ fid, action });
    if (s.throwOnce) {
      const e = s.throwOnce;
      s.throwOnce = null;
      throw e;
    }
    const to = action.to ? s.list.bots.find((b) => b.id === action.to) : s.list.default;
    return s.reply ?? { ...CARD, findings: CARD.findings.map((f) => (f.id === fid ? { ...f, state: 'sent', sent_to: to?.name ?? 'Pixel', sent_at: '2026-11-05T00:59:59Z', task_id: 'tsk_9' } : f)) };
  };
  return { s, recipients, onFinding };
}

async function card(props) {
  if (root) root.unmount();
  host?.remove();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(h(m.ReviewCard, { card: CARD, now: m.fx.FIXTURE_NOW, ...props }));
  await settle();
}
const rerender = async (props) => {
  root.render(h(m.ReviewCard, { card: CARD, now: m.fx.FIXTURE_NOW, ...props }));
  await settle();
};
const goes = (fid) => $(`[data-finding="${fid}"] [data-goes-to]`);
const fixBtn = (fid) => $(`[data-finding="${fid}"] [data-fix-it]`);
const errLine = (fid) => $(`[data-finding="${fid}"] .text-bad`)?.textContent ?? null;

test('1. the default line: wrote it, lead of Edge, and no default', async () => {
  sessionStorage.clear();
  let r = fakeRoutes({ list: AUTHOR });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  assert.equal(goes('f1').dataset.goesTo, 'author');
  assert.match(goes('f1').textContent, /Goes to\s*Pixel\s*· wrote it/);
  assert.equal(goes('f1').getAttribute('aria-label'), 'Fixes go to Pixel, wrote it. Change');
  assert.equal(goes('f1').getAttribute('aria-haspopup'), 'listbox');
  assert.ok(!goes('f2'), 'a sent finding has no line');

  r = fakeRoutes({ list: LEAD });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  assert.match(goes('f1').textContent, /Ana\s*· lead of Edge/);
  assert.equal(r.s.gets, 1, 'fetched once per card, not per finding');

  r = fakeRoutes({ list: NONE });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  assert.equal(goes('f1').dataset.goesTo, 'none');
  assert.match(goes('f1').textContent, /No default bot\s*·\s*Pick who fixes it/);

  r = fakeRoutes({ list: EMPTY });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  assert.equal(goes('f1').dataset.goesTo, 'nobody');
  assert.ok(fixBtn('f1').disabled, 'no bot at all: Fix it is off');
  assert.equal(fixBtn('f1').title, EMPTY.nobody);
});

test('2. a pick persists and covers a second finding; to is sent with it', async () => {
  sessionStorage.clear();
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(goes('f1'));
  await settle();
  const picker = $('[data-recipient-picker]');
  assert.ok(picker, 'the picker opens');
  assert.equal(goes('f1').getAttribute('aria-expanded'), 'true');
  assert.match(picker.textContent, /Who fixes this PR\?/);
  assert.ok(!picker.querySelector('input'), 'no search with 8 bots or fewer');
  const opts = $$('[role="option"]');
  assert.deepEqual(opts.map((o) => o.dataset.bot), ['bot_ana', 'bot_ben', 'bot_cy', 'bot_mo']);
  assert.equal(opts[0].getAttribute('aria-selected'), 'true', 'the default is selected');
  // Keyboard: down to Ben, Enter picks.
  const list = $('[role="listbox"]');
  key(list, 'ArrowDown');
  await settle();
  key(list, 'Enter');
  await settle();
  assert.ok(!$('[data-recipient-picker]'), 'picking closes the picker');
  assert.equal(r.s.acts.length, 0, 'picking never sends');
  assert.match($('[data-send-to-say]').textContent, /Fixes go to Ben/);
  assert.match(goes('f1').textContent, /Ben\s*· your pick/);
  assert.match(goes('f3').textContent, /Ben\s*· your pick/, 'the pick covers every finding on the card');
  assert.deepEqual(JSON.parse(sessionStorage.getItem('kwnt:fix-to:dvr_294')), BEN);

  click(fixBtn('f3'));
  await settle();
  assert.deepEqual(r.s.acts[0], { fid: 'f3', action: { action: 'fix', to: 'bot_ben' } });
  assert.match($('[data-finding="f3"]').textContent, /Sent to Ben as a task/);

  // A new mount of the same card keeps the pick.
  const r2 = fakeRoutes();
  await card({ recipients: r2.recipients, onFinding: r2.onFinding });
  assert.match(goes('f1').textContent, /Ben\s*· your pick/);
});

test('3. Fix it with no default and no pick opens the picker, and sends after a pick and a second press', async () => {
  sessionStorage.clear();
  const r = fakeRoutes({ list: NONE });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(fixBtn('f1'));
  await settle();
  assert.ok($('[data-recipient-picker]'), 'the picker opens');
  assert.equal(r.s.acts.length, 0, 'nothing sent');
  assert.match($('[data-picker-footer]').textContent, /no team owns/);
  click($('[data-bot="bot_cy"]'));
  await settle();
  assert.equal(r.s.acts.length, 0, 'the pick alone doesn’t send');
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(r.s.acts[0].action, { action: 'fix', to: 'bot_cy' });
});

test('4. to is sent only with a pick until the picker shows; picking the default clears the pick', async () => {
  sessionStorage.clear();
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(r.s.acts[0].action, { action: 'fix' });

  sessionStorage.setItem('kwnt:fix-to:dvr_294', JSON.stringify(BEN));
  const r2 = fakeRoutes();
  await card({ recipients: r2.recipients, onFinding: r2.onFinding });
  click(goes('f1'));
  await settle();
  click($('[data-bot="bot_ana"]'));
  await settle();
  assert.equal(sessionStorage.getItem('kwnt:fix-to:dvr_294'), null);
  click(fixBtn('f1'));
  await settle();
  // The picker was shown, so the press to the default names it (N1).
  assert.deepEqual(r2.s.acts[0].action, { action: 'fix', to: 'bot_ana' });
});

test('N1: once the picker has shown (or a finding went to a chosen bot), every Fix it press sends to', async () => {
  // The pure rule.
  const L = m.parseRecipients(LEAD);
  assert.equal(m.toFor(L, null, false), undefined, 'one tap to the default, picker never shown: no to');
  assert.equal(m.toFor(L, BEN, false), 'bot_ben');
  assert.equal(m.toFor(L, null, true), 'bot_ana', 'shown: the default is named');
  assert.equal(m.toFor(L, BEN, true), 'bot_ben');
  assert.equal(m.toFor(m.parseRecipients(NONE), null, true), undefined, 'nobody to name');
  assert.equal(m.toFor(null, BEN, true), 'bot_ben', 'list failed: the stored pick still goes');

  // Pick Ben, Fix it f1; then back to the default and Fix it f3 while f1 may still be waiting: f3 names Ana.
  sessionStorage.clear();
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(goes('f1'));
  await settle();
  click($('[data-bot="bot_ben"]'));
  await settle();
  assert.equal(sessionStorage.getItem('kwnt:fix-to-shown:dvr_294'), '1');
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(r.s.acts[0].action, { action: 'fix', to: 'bot_ben' });
  click(goes('f3'));
  await settle();
  click($('[data-bot="bot_ana"]'));
  await settle();
  click(fixBtn('f3'));
  await settle();
  assert.deepEqual(r.s.acts[1].action, { action: 'fix', to: 'bot_ana' });

  // A reload of the card keeps it: no pick, but the picker was shown here, so the one tap names the default.
  const r2 = fakeRoutes();
  await card({ recipients: r2.recipients, onFinding: r2.onFinding });
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(r2.s.acts[0].action, { action: 'fix', to: 'bot_ana' });

  // A fresh tab that never showed the picker, on a card with a finding already sent to a chosen bot.
  sessionStorage.clear();
  const chosen = { ...CARD, findings: CARD.findings.map((f) => (f.id === 'f2' ? { ...f, sent_via: 'chosen', sent_to_id: 'bot_ben' } : f)) };
  const r3 = fakeRoutes();
  await card({ card: chosen, recipients: r3.recipients, onFinding: r3.onFinding });
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(r3.s.acts[0].action, { action: 'fix', to: 'bot_ana' });

  // Line notes follow the same rule: shown on the card, the default is named on send.
  const n = notesApi(changes({ author_bot: { id: 'bot_ana', name: 'Ana' } }));
  await notes({ api: n.api, sendTo: sendTo(LEAD, { pinned: true }) });
  click($('[data-ln-send]'));
  await settle();
  assert.deepEqual(n.s.sends[0], { ids: null, to: 'bot_ana' });
});

test('5. bad_recipient: plain words, the list refetched, the pick back to the default', async () => {
  sessionStorage.setItem('kwnt:fix-to:dvr_294', JSON.stringify(MO));
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  assert.match(goes('f1').textContent, /Mo\s*· your pick/);
  r.s.throwOnce = Object.assign(new Error('to: no such bot in this space'), { status: 400, code: 'bad_recipient' });
  r.s.list = { ...LEAD, bots: [ANA, BEN, CY] };
  click(fixBtn('f1'));
  await settle();
  assert.equal(errLine('f1'), 'Couldn’t send it: Mo isn’t in this space. Pick another bot and press Fix it again.');
  assert.equal(r.s.gets, 2, 'refetched');
  assert.match(goes('f1').textContent, /Ana\s*· lead of Edge/);
  assert.equal(sessionStorage.getItem('kwnt:fix-to:dvr_294'), null);
});

test('6. a chosen bot deleted in the Undo window: the server’s send_error shows, the list is refetched', async () => {
  sessionStorage.setItem('kwnt:fix-to:dvr_294', JSON.stringify(BEN));
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  r.s.list = { ...LEAD, bots: [ANA, CY, MO] };
  const back = { ...CARD, findings: CARD.findings.map((f) => (f.id === 'f1' ? { ...f, state: 'open', send_error: 'Ben was removed from this space before the task started.' } : f)) };
  await rerender({ card: back, recipients: r.recipients, onFinding: r.onFinding });
  assert.equal(errLine('f1'), 'Couldn’t send it: Ben was removed from this space before the task started.');
  assert.equal(r.s.gets, 2, 'refetched');
  assert.match(goes('f1').textContent, /Ana\s*· lead of Edge/, 'the pick is gone, so the default');
});

test('7. Open task › after the Undo window, when the finding has a task', async () => {
  sessionStorage.clear();
  const r = fakeRoutes();
  const opened = [];
  const sent = { ...CARD, findings: CARD.findings.map((f) => (f.id === 'f1' ? { ...f, state: 'sent', sent_to: 'Ben', sent_at: '2026-11-05T00:50:00Z', task_id: 'tsk_7' } : f)) };
  await card({ card: sent, recipients: r.recipients, onFinding: r.onFinding, onOpenTask: (id) => opened.push(id) });
  const b = $('[data-finding="f1"] [data-open-task]');
  assert.ok(b);
  assert.equal(b.textContent, 'Open task ›');
  click(b);
  assert.deepEqual(opened, ['tsk_7']);
  assert.ok(!$('[data-finding="f2"] [data-open-task]'), 'no task id, no link');
});

test('8. the list fails: Try again shows, and Fix it still sends without to', async () => {
  sessionStorage.clear();
  const r = fakeRoutes({ fail: true });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  assert.equal(goes('f1').dataset.goesTo, 'failed');
  assert.match(goes('f1').textContent, /Goes to the PR’s usual bot · couldn’t load the list/);
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(r.s.acts[0].action, { action: 'fix' });
  r.s.fail = false;
  click(goes('f3').querySelector('button'));
  await settle();
  assert.match(goes('f3').textContent, /Ana\s*· lead of Edge/);
});

test('loading: a static skeleton, and Fix it is enabled', async () => {
  sessionStorage.clear();
  let release;
  const recipients = () => new Promise((res) => (release = res));
  const acts = [];
  await card({ recipients, onFinding: async (_c, fid, action) => (acts.push(action), CARD) });
  assert.equal(goes('f1').dataset.goesTo, 'loading');
  assert.ok(!fixBtn('f1').disabled);
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(acts[0], { action: 'fix' });
  release(rec(LEAD));
  await settle();
});

test('no recipients route (an older server or the sample card): no line, Fix it as before', async () => {
  const acts = [];
  await card({ onFinding: async (_c, fid, action) => (acts.push(action), CARD) });
  assert.ok(!goes('f1'));
  click(fixBtn('f1'));
  await settle();
  assert.deepEqual(acts[0], { action: 'fix' });
});

test('the picker: Esc closes and gives focus back; search shows from 9 bots and filters', async () => {
  sessionStorage.clear();
  const many = Array.from({ length: 9 }, (_, i) => ({ id: `bot_${i}`, name: ['Ana', 'Ben', 'Cy', 'Dot', 'Juno', 'Mo', 'Pia', 'Rex', 'Sol'][i], team: i < 3 ? 'Edge' : 'Ops' }));
  const r = fakeRoutes({ list: { default: { ...many[0], via: 'team_lead' }, nobody: null, bots: many } });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  const trigger = goes('f1');
  click(trigger);
  await settle();
  const input = $('[data-recipient-picker] input');
  assert.ok(input, 'search with more than 8 bots');
  assert.equal(document.activeElement, input);
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  set.call(input, 'ops');
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  assert.equal($$('[role="option"]').length, 6);
  set.call(input, 'zzz');
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  assert.equal($('[data-picker-empty]').textContent, 'No bot matches "zzz".');
  key(input, 'Escape');
  await settle();
  assert.ok(!$('[data-recipient-picker]'));
  assert.equal(document.activeElement, trigger, 'focus back on the trigger');
});

// ---------------------------------------------------------------------------------------------------
// Line notes

const HEAD = 'c3d9a18f00000000000000000000000000000000';
const changes = (over = {}) => ({
  card_id: 'dvr_31',
  repo: 'demo-org/edge',
  pr: 31,
  head_sha: HEAD,
  pr_url: 'https://github.com/demo-org/edge/pull/31',
  pr_state: 'open',
  author_bot: null,
  author_login: 'sam',
  files: [{ path: 'src/a.rs', status: 'modified', additions: 1, deletions: 0, hunks: [{ header: '@@ -1 +1 @@', lines: [{ kind: 'add', new: 1, text: 'let a = 1;' }] }] }],
  ...over,
});
const note = (id) => ({ id, path: 'src/a.rs', start: { side: 'new', line: 1 }, end: { side: 'new', line: 1 }, at_head: { start: { side: 'new', line: 1 }, end: { side: 'new', line: 1 } }, code: [], text: 'Name it.', state: 'draft', head_sha: HEAD, created_at: '2026-11-10T09:12:00Z', seen_sha: null });

function notesApi(c) {
  const s = { sends: [], fail: null };
  const api = {
    changes: async () => m.parseChanges(c),
    notes: async () => ({ notes: [m.parseNote(note('ln_1')), m.parseNote(note('ln_2'))], summary: null }),
    send: async (cardId, sendId, ids, to) => {
      s.sends.push({ ids: ids ?? null, to: to ?? null });
      if (s.fail) {
        const e = s.fail;
        s.fail = null;
        throw e;
      }
      return { sent: 2, message_id: 'msg_1', kwant_id: to ?? 'bot_ana', kwant_name: to === 'bot_ben' ? 'Ben' : 'Ana', sent_at: '2026-11-10T09:30:00Z', notes: [] };
    },
  };
  return { api, s };
}
const sendTo = (r, over = {}) => {
  const st = { load: { s: 'ready', r: rec(r) }, pick: null, phone: false, anchor: null, opened: [], resets: 0, ...over };
  st.open = (el, after) => st.opened.push({ el, after });
  st.retry = () => {};
  st.reset = () => st.resets++;
  st.refresh = () => {};
  return st;
};
async function notes(props) {
  if (root) root.unmount();
  host?.remove();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(h(m.ChangesSection, { cardId: 'dvr_31', stat: { files: 1, additions: 1, deletions: 0 }, defaultOpen: true, phone: false, ...props }));
  await settle();
}

test('9. line notes with no author bot but bots in the space: allowed, Pick a bot, then send with to', async () => {
  const { api, s } = notesApi(changes());
  const st = sendTo(NONE);
  await notes({ api, sendTo: st });
  assert.ok($('[data-ln-no-default]'), 'the new banner');
  assert.match($('[data-ln-no-default]').textContent, /^This PR is by sam and no team owns this repo/);
  assert.ok($('[data-ln-bar]'), 'notes can be sent');
  assert.equal($('[data-ln-to]').dataset.lnTo, 'none');
  assert.match($('[data-ln-to]').textContent, /Pick a bot/);
  const send = $('[data-ln-send]');
  assert.match(send.textContent, /Send 2 notes…/);
  click(send);
  await settle();
  assert.equal(s.sends.length, 0, 'opens the picker instead');
  assert.equal(st.opened.length, 1);
  st.opened[0].after(BEN);
  await settle();
  assert.deepEqual(s.sends[0], { ids: null, to: 'bot_ben' });
  assert.match($('[data-ln-bar]').textContent, /Sent 2 notes to Ben/);

  // Without the list (an older server), author_bot null still means no notes.
  const old = notesApi(changes());
  await notes({ api: old.api });
  assert.ok(!$('[data-ln-bar]'));
  assert.ok(!$('[data-ln-no-default]'));
});

test('10. line notes: the default needs no to, a pick sends it, bad_recipient shows the bar', async () => {
  const { api, s } = notesApi(changes({ author_bot: { id: 'bot_ana', name: 'Ana' } }));
  await notes({ api, sendTo: sendTo(LEAD) });
  assert.match($('[data-ln-send]').textContent, /Send 2 notes to Ana/);
  assert.match($('[data-ln-to]').textContent, /Ana/);
  click($('[data-ln-send]'));
  await settle();
  assert.deepEqual(s.sends[0], { ids: null, to: null });

  const b = notesApi(changes({ author_bot: { id: 'bot_ana', name: 'Ana' } }));
  const st = sendTo(LEAD, { pick: MO });
  await notes({ api: b.api, sendTo: st });
  assert.match($('[data-ln-send]').textContent, /Send 2 notes to Mo/);
  b.s.fail = new m.LineNotesError('to: no such bot', 400, 'bad_recipient');
  click($('[data-ln-send]'));
  await settle();
  assert.deepEqual(b.s.sends[0], { ids: null, to: 'bot_mo' });
  assert.equal($('[data-ln-bar]').dataset.lnBar, 'bad');
  assert.match($('[data-ln-bad]').textContent, /The notes didn’t send: Mo isn’t in this space\. Pick another bot\./);
  assert.equal(st.resets, 1, 'refetch and drop the pick');
});

// ---------------------------------------------------------------------------------------------------
// Mira's design check (design-check.md fixes 1-3)

test('fix 1: scrolling the page under the trigger closes the popover, without refocusing; the list’s own scroll doesn’t', async () => {
  sessionStorage.clear();
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(goes('f1'));
  await settle();
  const list = $('[data-recipient-picker] [role="listbox"]');
  list.dispatchEvent(new window.Event('scroll'));
  await settle();
  assert.ok($('[data-recipient-picker]'), 'scrolling inside the list keeps it open');
  const trigger = goes('f1');
  document.dispatchEvent(new window.Event('scroll'));
  await settle();
  assert.ok(!$('[data-recipient-picker]'), 'page scroll closes it');
  await new Promise((res) => setTimeout(res, 5));
  assert.notEqual(document.activeElement, trigger, 'focus is not pulled back to the trigger');
  // an ancestor of the trigger scrolling closes it too
  click(goes('f1'));
  await settle();
  assert.ok($('[data-recipient-picker]'));
  host.dispatchEvent(new window.Event('scroll'));
  await settle();
  assert.ok(!$('[data-recipient-picker]'));
});

test('fix 2: no ring when the picker opens; with no default no row is active; arrows bring the ring, the pointer takes it away', async () => {
  sessionStorage.clear();
  const r = fakeRoutes({ list: NONE });
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(goes('f1'));
  await settle();
  const list = $('[data-recipient-picker] [role="listbox"]');
  const ringed = () => $$('[role="option"]').filter((o) => o.className.includes('outline-sky-400/60')).map((o) => o.dataset.bot);
  const lit = () => $$('[role="option"]').filter((o) => o.className.includes('bg-white/[.07]')).map((o) => o.dataset.bot);
  assert.deepEqual(ringed(), []);
  assert.deepEqual(lit(), [], 'nothing preselected with no default');
  assert.equal(list.getAttribute('aria-activedescendant'), null);
  key(list, 'Enter');
  await settle();
  assert.ok($('[data-recipient-picker]'), 'Enter with nothing active does nothing');
  key(list, 'ArrowDown');
  await settle();
  assert.deepEqual(ringed(), ['bot_ana'], 'the first arrow goes to row 0, with the ring');
  $('[data-bot="bot_ben"]').dispatchEvent(new window.MouseEvent('pointermove', { bubbles: true }));
  await settle();
  assert.deepEqual(ringed(), [], 'the pointer takes the ring away');
  assert.deepEqual(lit(), ['bot_ben']);
  key(list, 'Escape');
  await settle();
  // with a default: it starts lit (its ✓ says chosen) but without the ring
  sessionStorage.clear();
  const r2 = fakeRoutes();
  await card({ recipients: r2.recipients, onFinding: r2.onFinding });
  click(goes('f1'));
  await settle();
  assert.deepEqual(lit(), ['bot_ana']);
  assert.deepEqual(ringed(), []);
});

test('fix 3: a send error waits for the list, and stays hidden when nobody can receive', async () => {
  sessionStorage.clear();
  const errCard = { ...CARD, findings: CARD.findings.map((f) => (f.id === 'f1' ? { ...f, state: 'open', send_error: 'Ben was removed from this space before the task started.' } : f)) };
  let release;
  const slow = () => new Promise((res) => (release = res));
  await card({ card: errCard, recipients: slow, onFinding: async () => errCard });
  assert.equal(goes('f1').dataset.goesTo, 'loading');
  assert.equal(errLine('f1'), null, 'hidden while the list loads');
  release(rec(LEAD));
  await settle();
  assert.equal(errLine('f1'), 'Couldn’t send it: Ben was removed from this space before the task started.');
  sessionStorage.clear();
  const r = fakeRoutes({ list: EMPTY });
  await card({ card: errCard, recipients: r.recipients, onFinding: r.onFinding });
  assert.ok(fixBtn('f1').disabled);
  assert.equal(errLine('f1'), null, 'nobody can receive: the disabled Fix it says why');
  // no recipients route at all: as before
  await card({ card: errCard, onFinding: async () => errCard });
  assert.match(errLine('f1'), /Ben was removed/);
});

test('fix 1: moving through the list never scrolls the page (that would close the popover)', async () => {
  sessionStorage.clear();
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  let calls = 0;
  const orig = window.HTMLElement.prototype.scrollIntoView;
  window.HTMLElement.prototype.scrollIntoView = function () { calls++; };
  try {
    click(goes('f1'));
    await settle();
    const list = $('[data-recipient-picker] [role="listbox"]');
    key(list, 'ArrowDown');
    await settle();
    key(list, 'End');
    await settle();
    assert.equal(calls, 0, 'no scrollIntoView: only the list scrolls');
    assert.ok($('[data-recipient-picker]'), 'still open');
  } finally {
    window.HTMLElement.prototype.scrollIntoView = orig;
  }
});

test('opening the desktop popover puts focus in it (once it is placed and visible)', async () => {
  sessionStorage.clear();
  const r = fakeRoutes();
  await card({ recipients: r.recipients, onFinding: r.onFinding });
  click(goes('f1'));
  await settle();
  const pop = $('[data-recipient-picker]');
  assert.ok(pop.style.visibility !== 'hidden', 'placed');
  assert.ok(pop.contains(document.activeElement), 'focus moved into the popover');
});
