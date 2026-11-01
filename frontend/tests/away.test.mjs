import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// Sidebar's sound settings read localStorage on import.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const a = await loadBundled(`export * from './src/away';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { AwayView } from './src/components/AwayView';
  import { Sidebar } from './src/components/Sidebar';
  export const render = (props) => renderToStaticMarkup(React.createElement(AwayView, props));
  export const sidebar = (props) => renderToStaticMarkup(React.createElement(Sidebar, props));
`);

const NOW = Date.parse('2026-10-17T12:47:00Z');
const item = (over = {}) => ({
  kind: 'delegation', id: 'd1', kwant_id: 'k2', kwant_name: 'Scout', title: 'Write the brief', summary: 'Brief',
  status: 'completed', timestamp: '2026-10-17T10:15:00Z', link: { thread_id: 't1', channel_id: null, direct_kwant_id: null }, ...over,
});
const question = item({ kind: 'question', id: 'q1', title: 'Ship tonight?', summary: 'Choices: Yes · No', status: 'open', choices: ['Yes', 'No'],
  link: { thread_id: null, channel_id: 'c1', direct_kwant_id: null } });
const approval = item({ kind: 'approval', id: 'a1', kwant_id: 'k1', kwant_name: 'Lead', title: 'Use send (mail)', status: 'pending',
  summary: 'api_key: [redacted]', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k1' } });
const resp = (over = {}) => ({
  since: '2026-10-10T12:47:00Z', last_seen: null, now: '2026-10-17T12:47:00Z', stuck_after_minutes: 30,
  waiting: [question, approval], finished: [item()], failed: [item({ id: 'd3', status: 'stuck', title: 'Long crawl' })],
  unseen: { waiting: 2, finished: 1, failed: 1, total: 4 }, ...over,
});

test('relative time reads naturally and survives bad input', () => {
  const ago = (s) => new Date(NOW - s * 1000).toISOString();
  assert.equal(a.relativeTime(ago(5), NOW), 'just now');
  assert.equal(a.relativeTime(ago(-30), NOW), 'just now', 'clock skew is not "in the future"');
  assert.equal(a.relativeTime(ago(5 * 60), NOW), '5 min ago');
  assert.equal(a.relativeTime(ago(3 * 3600 + 59), NOW), '3 h ago');
  assert.equal(a.relativeTime(ago(30 * 3600), NOW), 'yesterday');
  assert.equal(a.relativeTime(ago(4 * 86400), NOW), '4 days ago');
  assert.doesNotMatch(a.relativeTime(ago(9 * 86400), NOW), /ago/);
  assert.equal(a.relativeTime('nope', NOW), '');
});

test('the subtitle names the default week or the last visit', () => {
  assert.equal(a.windowLabel('2026-10-10T12:47:00Z', NOW), 'From the last 7 days');
  assert.equal(a.windowLabel('2026-10-17T09:47:00Z', NOW), 'Since you last looked, 3 h ago');
  assert.equal(a.windowLabel('bad', NOW), '');
});

test('labels per kind and status', () => {
  const l = (kind, status) => a.itemLabel({ kind, status });
  assert.deepEqual(l('question', 'open'), { label: 'Question', tone: 'amber' });
  assert.deepEqual(l('approval', 'pending'), { label: 'Needs approval', tone: 'amber' });
  assert.deepEqual(l('delegation', 'completed'), { label: 'Handoff done', tone: 'emerald' });
  assert.deepEqual(l('delegation', 'failed'), { label: 'Handoff failed', tone: 'rose' });
  assert.deepEqual(l('delegation', 'interrupted'), { label: 'Interrupted', tone: 'rose' });
  assert.deepEqual(l('delegation', 'stuck'), { label: 'Stuck', tone: 'amber' });
  assert.deepEqual(l('routine_run', 'ok'), { label: 'Routine ran', tone: 'emerald' });
  assert.deepEqual(l('routine_run', 'error'), { label: 'Routine failed', tone: 'rose' });
  assert.equal(l('mystery', 'x').tone, 'zinc');
});

test('badge text, empty check and the event filter', () => {
  assert.equal(a.badgeText(0), '');
  assert.equal(a.badgeText(7), '7');
  assert.equal(a.badgeText(120), '99+');
  assert.equal(a.isAwayEmpty(null), false, 'not loaded is not "all caught up"');
  assert.equal(a.isAwayEmpty(resp({ waiting: [], finished: [], failed: [] })), true);
  assert.equal(a.isAwayEmpty(resp()), false);
  assert.ok(a.affectsAway('question_updated') && a.affectsAway('ApprovalRequired') && a.affectsAway('RoutineRun'));
  assert.equal(a.affectsAway('TextDelta'), false);
  assert.equal(a.affectsAway(undefined), false);
});

test('each item opens its thread, channel or direct chat', () => {
  assert.deepEqual(a.awayTarget(item()), { thread: 't1', kwant: 'k2' });
  assert.deepEqual(a.awayTarget(question), { channel: 'c1' });
  assert.deepEqual(a.awayTarget(approval), { direct: 'k1' });
  assert.deepEqual(a.awayTarget(item({ link: { thread_id: null, channel_id: null, direct_kwant_id: null } })), { direct: 'k2' });
});

test('an acted-on item leaves the list and the waiting count right away', () => {
  const r = resp();
  const next = a.withoutItem(r, 'waiting', 'q1');
  assert.deepEqual(next.waiting.map((i) => i.id), ['a1']);
  assert.deepEqual(next.unseen, { waiting: 1, finished: 1, failed: 1, total: 3 });
  assert.equal(a.withoutItem(r, 'waiting', 'nope'), r);
});

test('a lapsed request reads "This request expired"', () => {
  assert.equal(a.actionErrorText(409, 'x'), 'This request expired');
  assert.equal(a.actionErrorText(404, 'x'), 'This request expired');
  assert.equal(a.actionErrorText(500, 'Couldn’t send it'), 'Couldn’t send it');
});

test("an expired card's 409 shows the server's sentence as a plain notice, not an error", () => {
  const msg = 'This card expired: the bot is no longer waiting for it. Ask the bot again.';
  assert.deepEqual(a.actionFailureNotice(409, msg, 'Use send (mail)'), { text: `${msg} (“Use send (mail)”)`, error: false, info: true });
  // Other failures stay errors, with the old wording.
  assert.deepEqual(a.actionFailureNotice(409, 'Already answered', 'X'), { text: 'This request expired: “X”', error: true });
  assert.deepEqual(a.actionFailureNotice(500, 'Boom', 'X'), { text: "Couldn't send it: Boom: “X”", error: true });
  const html = ui.render({ data: resp(), loading: false, error: null, busy: null, notice: a.actionFailureNotice(409, msg, 'X'), onRefresh() {}, onOpen() {}, onAnswer() {}, onResolve() {} });
  assert.match(html, /This card expired/);
  assert.match(html, /bg-slate-500\/10/);
  assert.doesNotMatch(html, /bg-amber-500\/10 text-amber-200[^>]*>This card expired/);
});

const noop = () => {};
const view = (over = {}) => ui.render({ data: resp(), loading: false, error: null, now: NOW, onRefresh: noop, onOpen: noop, onAnswer: noop, onResolve: noop, ...over });

test('three sections with agent, title, summary and time; one-tap actions', () => {
  const html = view();
  const order = ['Waiting on you', 'Finished', 'Failed or stuck'].map((s) => html.indexOf(s));
  assert.ok(order.every((i) => i > 0) && order[0] < order[1] && order[1] < order[2]);
  assert.match(html, /Scout/);
  assert.match(html, /Write the brief/);
  assert.match(html, /Brief/, 'a summary line');
  assert.doesNotMatch(html, /Choices: Yes · No/, 'a question shows its choices as buttons, not a summary');
  assert.match(html, /2 h ago/);
  assert.match(html, /From the last 7 days/);
  assert.match(html, />Yes<\/button>/);
  assert.match(html, />No<\/button>/);
  // An approval is the same card as in the chat, with its labels.
  for (const b of ['Allow Once', 'Always Allow', 'Deny']) assert.match(html, new RegExp(b));
  assert.match(html, /Lead<\/span> wants to use <span[^>]*>send<\/span>/);
  assert.match(html, /Handoff done/);
  assert.match(html, /Stuck/);
  assert.doesNotMatch(html, /all caught up/);
});

test('empty, loading, error and expired states', () => {
  assert.match(view({ data: resp({ waiting: [], finished: [], failed: [] }) }), /You&#x27;re all caught up/);
  const loading = view({ data: null, loading: true });
  assert.match(loading, /Loading…/);
  assert.doesNotMatch(loading, /caught up/);
  const err = view({ data: null, error: 'Request failed (HTTP 500).' });
  assert.match(err, /role="alert"/);
  assert.match(err, /Retry/);
  assert.match(view({ notice: { text: 'This request expired', error: true } }), /This request expired/);
  const busy = view({ busy: 'a1' });
  assert.match(busy, /Sending…/);
  assert.ok((busy.match(/disabled=""/g) || []).length >= 5, 'every action is disabled while one is sending');
});

test('on a phone the page has a way back, since it hides the tab bar', () => {
  assert.match(view({ onBack: () => {} }), /aria-label="Back to chats"/);
  assert.doesNotMatch(view({}), /Back to chats/);
});

test('sidebar entry shows the unseen badge only when there is something new', () => {
  const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
    created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 } };
  const base = { space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop,
    onOpenSearch: noop,
    kwants: [], channels: [], teams: [], selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop,
    onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop };
  const withNew = ui.sidebar({ ...base, inboxCount: 7 });
  assert.match(withNew, />Inbox</);
  assert.doesNotMatch(withNew, /While you were away/);
  assert.match(withNew, />7<span class="sr-only"> new<\/span>/);
  assert.doesNotMatch(ui.sidebar({ ...base, inboxCount: 0 }), /sr-only"> new/);
  assert.match(ui.sidebar({ ...base, inboxCount: 3, inboxActive: true }), /aria-current="page"/);
});
