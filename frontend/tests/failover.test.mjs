import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };
// The sound module reads its mute flag on load; the list plays a click on a move.
const m = await loadBundled(`
  export * from './src/failover';
  export * from './src/failoverNotes';
  export { AccountPriority } from './src/components/AccountPriority';
  export { ReplyMeta } from './src/components/ReplyMeta';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const h = m.createElement;

const NOW = Date.parse('2026-11-02T20:00:00');
const profile = (over) => ({
  id: 'p', engine: 'claude', name: 'Work', kind: 'cli_home', enabled: true, is_default: false, priority: 100,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null, ...over,
});
const engine = (id, over = {}) => ({ id, name: id, kind: 'cli', status: 'ready', detail: null, version: '1', models: [], checked_at: null, enabled: true, ready: true, uses: null, ...over });
const list = [
  profile({ id: 'c1', name: 'Work', priority: 10, limited_until: '2026-11-03T03:00:00' }),
  profile({ id: 'x1', engine: 'codex', name: 'Personal', priority: 20 }),
  profile({ id: 'k1', engine: 'claude', name: 'API', kind: 'api_key', has_key: true, priority: 30 }),
  profile({ id: 'g1', engine: 'grok', name: 'Grok', priority: 40, signed_in: false }),
  profile({ id: 'u1', engine: 'custom', name: 'Copilot', kind: 'custom_api', priority: 50, custom: { status: 'ready', models: [], protocol: 'openai-chat', base_url: 'x', model: null, detail: null, checked_at: null, supports_tools: true } }),
];

test('one list across engines, ordered by priority with stable ties', () => {
  const shuffled = [list[3], list[0], list[4], list[2], list[1]];
  assert.deepEqual(m.failoverOrder(shuffled).map((p) => p.id), ['c1', 'x1', 'k1', 'g1', 'u1']);
  const tie = [profile({ id: 'b', name: 'B' }), profile({ id: 'a', name: 'A' }), profile({ id: 'sys', kind: 'system', name: 'This machine' })];
  assert.deepEqual(m.failoverOrder(tie).map((p) => p.id), ['sys', 'a', 'b']);
});

test('status words: Ready, Out until a local time, Signed out', () => {
  assert.deepEqual(m.priorityStatus(list[1], engine('codex'), NOW), { tone: 'ok', label: 'Ready' });
  const out = m.priorityStatus(list[0], engine('claude'), NOW);
  assert.equal(out.tone, 'warn');
  assert.match(out.label, /^Out until /);
  assert.match(out.label, /03:00/);
  assert.match(out.label, /^Out until \w{3} 03:00$/, 'a reset on another day names the day');
  assert.equal(m.priorityStatus(list[3], engine('grok'), NOW).label, 'Signed out');
  assert.equal(m.priorityStatus(profile({ last_error: 'auth' }), engine('claude'), NOW).label, 'Signed out');
  assert.equal(m.priorityStatus(profile({ enabled: false, limited_until: '2026-11-03T03:00:00' }), engine('claude'), NOW).label, 'Off');
  assert.equal(m.priorityStatus(list[4], undefined, NOW).label, 'Ready');
  // A limit that has passed is Ready again, with no action.
  assert.equal(m.priorityStatus(profile({ limited_until: '2026-11-02T19:00:00' }), engine('claude'), NOW).label, 'Ready');
  // Same-day reset is the bare time.
  assert.equal(m.priorityStatus(profile({ limited_until: '2026-11-02T23:30:00' }), engine('claude'), NOW).label, 'Out until 23:30');
});

test('moving rewrites only the priorities that change', () => {
  const order = m.failoverOrder(list);
  const next = m.moveAccount(order, 3, 1);
  assert.deepEqual(next.map((p) => p.id), ['c1', 'g1', 'x1', 'k1', 'u1']);
  assert.deepEqual(m.priorityUpdates(next), [
    { id: 'g1', priority: 20 },
    { id: 'x1', priority: 30 },
    { id: 'k1', priority: 40 },
  ]);
  assert.equal(m.moveAccount(order, 2, 2), null);
  assert.equal(m.moveAccount(order, 0, -1), null);
  assert.equal(m.moveAccount(order, 4, 5), null);
  // Equal priorities get distinct ranks once one moves.
  const ties = [profile({ id: 'a', priority: 100 }), profile({ id: 'b', priority: 100 })];
  assert.deepEqual(m.priorityUpdates([ties[1], ties[0]]), [{ id: 'b', priority: 10 }, { id: 'a', priority: 20 }]);
  assert.equal(m.moveAnnouncement('Codex', 1, 5), 'Codex moved to position 2 of 5.');
});

const markup = (props) => m.renderToStaticMarkup(h(m.AccountPriority, { profiles: list, engines: [engine('claude'), engine('codex'), engine('grok')], onChanged: async () => {}, now: NOW, ...props }));

test('the list is an ordered list with the caption, a status per row and labelled arrow buttons', () => {
  const html = markup();
  assert.match(html, /When one runs out, kwant bots move to the next that still works, in this order\./);
  assert.equal((html.match(/<li /g) ?? []).length, 5);
  assert.match(html, /<ol /);
  assert.match(html, /Claude API key/);
  assert.match(html, /Custom API/);
  assert.match(html, /Out until/);
  assert.match(html, /Signed out/);
  assert.match(html, /disabled="" aria-label="Move Work \(Claude\) up"/, 'the first row can not go up');
  assert.match(html, /disabled="" aria-label="Move Copilot \(Custom API\) down"/, 'the last row can not go down');
  assert.doesNotMatch(html, /disabled="" aria-label="Move Personal \(Codex\) up"/);
  assert.match(html, /role="status"/);
  assert.doesNotMatch(html, /[\u{1F300}-\u{1FAFF}]/u, 'no emoji');
});

test('the failover notice is one short line, naming the limit time and the model', () => {
  const e = { kwant_id: 'k', from_profile_id: 'c1', from_name: 'Work', to_profile_id: 'x1', to_name: 'Personal', class: 'quota',
    retry_at: '2026-11-02T23:30:00', from_engine: 'claude', to_engine: 'codex', to_model: 'GPT-6.1' };
  assert.equal(m.failoverNotice(e, NOW), 'Claude is out until 23:30 · answered on Codex (GPT-6.1)');
  assert.equal(m.failoverNotice({ ...e, class: 'auth', retry_at: undefined }, NOW), 'Claude is signed out · answered on Codex (GPT-6.1)');
  assert.equal(m.failoverNotice({ ...e, retry_at: undefined }, NOW), 'Claude is out of usage · answered on Codex (GPT-6.1)');
  // An older server sends no engine or model: the account names stand in.
  assert.equal(m.failoverNotice({ ...e, from_engine: undefined, to_engine: undefined, to_model: undefined, retry_at: undefined }, NOW), 'Work is out of usage · answered on Personal');
  assert.equal(m.failoverNotice({ ...e, to_model: undefined, to_model_label: 'GPT-6.1' }, NOW), 'Claude is out until 23:30 · answered on Codex (GPT-6.1)');
});

test('a chat is told once per outage', () => {
  const log = m.createOutageLog();
  const e = { kwant_id: 'k', from_profile_id: 'c1', class: 'quota', retry_at: '2026-11-03T03:00:00Z' };
  assert.equal(log.first(m.outageKey(e)), true);
  assert.equal(log.first(m.outageKey(e)), false, 'the next turn in the same outage');
  assert.equal(log.first(m.outageKey(e, 'thread-2')), true, 'another chat is told once too');
  assert.equal(log.first(m.outageKey({ ...e, retry_at: '2026-11-04T03:00:00Z' })), true, 'a new outage is a new line');
  assert.equal(log.first(m.outageKey({ ...e, class: 'auth' })), true);
});

test('a note goes under the first reply by its bot after the switch, and the live bubble until then', () => {
  const msgs = [
    { id: 'u1', sender_role: 'user', sender_id: 'me', created_at: '2026-11-02T19:59:00Z' },
    { id: 'm0', sender_role: 'kwant', sender_id: 'Architect', created_at: '2026-11-02T19:00:00Z' },
    { id: 'm1', sender_role: 'kwant', sender_id: 'Architect', created_at: '2026-11-02T20:00:30Z' },
    { id: 'm2', sender_role: 'kwant', sender_id: 'Architect', created_at: '2026-11-02T20:05:00Z' },
  ];
  const at = Date.parse('2026-11-02T20:00:10Z');
  const notes = [{ name: 'Architect', text: 'Claude is out until 03:00 · answered on Codex (GPT-6.1)', at }];
  const got = m.assignSwitchNotes(notes, msgs, (x) => x.sender_id);
  assert.deepEqual([...got.entries()], [['m1', notes[0].text]]);
  // No reply yet: the live bubble carries it.
  const none = m.assignSwitchNotes(notes, msgs.slice(0, 2), (x) => x.sender_id);
  assert.equal(none.size, 0);
  assert.equal(m.liveSwitchNote(notes, none, 'Architect', at + 1000), notes[0].text);
  assert.equal(m.liveSwitchNote(notes, got, 'Architect', at + 1000), null, 'not twice');
  assert.equal(m.liveSwitchNote(notes, none, 'Other', at + 1000), null);
});

test('the notice sits under the name and wraps, so it fits at 390 px', () => {
  const html = m.renderToStaticMarkup(h(m.ReplyMeta, { name: 'Architect', at: '2026-11-02T20:00:30Z', note: 'Claude is out until 03:00 · answered on Codex (GPT-6.1)' }));
  assert.match(html, /data-failover-note/);
  assert.match(html, /break-words/);
  assert.doesNotMatch(html, /truncate[^"]*"[^>]*data-failover-note/);
  assert.ok(html.indexOf('Architect') < html.indexOf('data-failover-note'));
  assert.doesNotMatch(m.renderToStaticMarkup(h(m.ReplyMeta, { name: 'Architect' })), /data-failover-note/);
});

test('a saved reply reads its notice from meta.failover, so it survives a reload', () => {
  const sw = { kwant_id: 'k', from_profile_id: 'c1', to_profile_id: 'x1', from_name: 'Work', to_name: 'Personal', class: 'quota', retry_at: '2026-11-03T02:00:00Z', from_engine: 'claude', to_engine: 'codex', to_model: 'gpt-6.1' };
  const now = Date.parse('2026-11-02T20:00:00Z');
  const line = m.savedFailoverNotice(JSON.stringify({ failover: [sw] }), now);
  assert.match(line, /^Claude is out until .*\d\d:\d\d · answered on Codex \(gpt-6\.1\)$/);
  // Already parsed, and a single object, read the same.
  assert.equal(m.savedFailoverNotice({ failover: sw }, now), line);
  // Two switches in one reply: first account out, last one answered.
  const two = m.savedFailoverNotice({ failover: [sw, { ...sw, from_engine: 'codex', to_engine: 'grok', to_model: 'grok-4.7' }] }, now);
  assert.match(two, /^Claude is out until .* · answered on Grok \(grok-4\.7\)$/);
  // Auth, and a reset already past.
  assert.equal(m.savedFailoverNotice({ failover: [{ ...sw, class: 'auth' }] }, now), 'Claude is signed out · answered on Codex (gpt-6.1)');
  assert.equal(m.savedFailoverNotice({ failover: [{ ...sw, retry_at: '2026-11-01T00:00:00Z' }] }, now), 'Claude is out of usage · answered on Codex (gpt-6.1)');
});

test('a reply with no or malformed failover meta shows no notice', () => {
  for (const meta of [null, undefined, '', 'not json', '{"ran_on":{}}', { failover: [] }, { failover: 'x' }, { failover: [null, 3, {}] }, { failover: [{ from_name: 'A' }] }]) {
    assert.equal(m.savedFailoverNotice(meta), null);
  }
});

// ---- Not-installed and signed-out engines are listed apart, unranked ----

const sys = (id, eng, priority) => profile({ id, engine: eng, kind: 'system', name: 'This machine', priority });
const sysList = [sys('s-claude', 'claude', 10), sys('s-codex', 'codex', 20), sys('s-grok', 'grok', 30)];
const sysEngines = [engine('claude'), engine('codex'), engine('grok', { status: 'not_installed', ready: false })];
const engineOf = (p) => sysEngines.find((e) => e.id === p.engine);

test('a not-installed engine is not ranked, a signed-out one is not ranked, an out one still is', () => {
  const { ranked, unranked } = m.splitRanked(m.failoverOrder(sysList), engineOf, NOW);
  assert.deepEqual(ranked.map((p) => p.id), ['s-claude', 's-codex']);
  assert.deepEqual(unranked.map((p) => p.id), ['s-grok']);
  const signedOut = m.splitRanked(m.failoverOrder(list), (p) => engine(p.engine), NOW);
  assert.deepEqual(signedOut.unranked.map((p) => p.id), ['g1'], 'Grok signed out');
  assert.ok(signedOut.ranked.some((p) => p.id === 'c1'), 'Out until 03:00 stays ranked');
  const off = m.splitRanked([profile({ id: 'o', enabled: false })], () => engine('claude'), NOW);
  assert.equal(off.ranked.length, 1, 'switched off stays ranked');
});

test('ranks are written for the ranked list only: an unranked account keeps its saved priority', () => {
  const { ranked, unranked } = m.splitRanked(m.failoverOrder(sysList), engineOf, NOW);
  const next = m.moveAccount(ranked, 1, 0);
  const updates = m.priorityUpdates(next);
  assert.deepEqual(updates, [{ id: 's-codex', priority: 10 }, { id: 's-claude', priority: 20 }]);
  assert.ok(!updates.some((u) => unranked.some((p) => p.id === u.id)), 'no update for Grok');
  assert.equal(unranked[0].priority, 30);
});

test('the list shows Grok under the ranked rows, with its status, no number and no arrows', () => {
  const html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: sysList, engines: sysEngines, onChanged: async () => {}, now: NOW }));
  assert.equal((html.match(/<ol /g) ?? []).length, 1);
  const ranked = html.slice(html.indexOf('<ol '), html.indexOf('</ol>'));
  assert.equal((ranked.match(/<li /g) ?? []).length, 2, 'two ranked rows');
  assert.doesNotMatch(ranked, /Grok|Not installed/);
  const rest = html.slice(html.indexOf('</ol>'));
  assert.match(rest, /Not in the order/);
  assert.match(rest, /Grok/);
  assert.match(rest, /Not installed/);
  assert.doesNotMatch(rest, /aria-label="Move /, 'not reorderable');
  assert.doesNotMatch(rest, /draggable/);
});
