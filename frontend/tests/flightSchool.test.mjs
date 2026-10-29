// Flight School, Phase C3 (docs/mission-control.md §14): a bot's tier per kind of work. With the
// setting off nothing on screen changes; with it on, Settings, the bot's settings, the board and the
// approval card show the tiers, where they come from, a one-click Revoke and why a card asked.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const fs = await loadBundled(`export * from './src/flightSchool';`);
const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { TaskCard, TaskDetailBody } from './src/components/MissionBoardView';
  import { FlightSchoolPanel } from './src/components/FlightSchoolPanel';
  import { FlightSchoolSetting } from './src/components/SettingsModal';
  import { ModernApprovalCard } from './src/components/ModernApprovalCard';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
  export const detail = (p) => r(TaskDetailBody, p);
  export const panel = (p) => r(FlightSchoolPanel, p);
  export const setting = (p) => r(FlightSchoolSetting, p);
  export const approval = (a) => r(ModernApprovalCard, { approval: a, onResolve: () => {} });
`);

const NOW = Date.parse('2026-10-28T12:00:00Z');
const noop = () => {};
const names = { k_sandy: 'Sandy' };
const none = { status: 'not_checked', passed: 0, failed: 0, needs_approval: 0, required_needs_approval: 0, total: 0 };
const task = (over = {}) => ({
  id: 't1', space_id: 's1', mission_id: 'm1', mission_title: 'Release', title: 'Write the notes', acceptance: 'NOTES.md exists',
  state: 'ready', priority: 1, owner_kwant_id: 'k_sandy', owner_name: 'Sandy', suggested_kwant_id: null, budget_tokens: 40000, spent_tokens: 0,
  attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1', channel_id: 'c1', direct_kwant_id: null,
  thread_id: null, created_by: 'person', created_at: '2026-10-28T09:00:00Z', updated_at: '2026-10-28T11:00:00Z', finished_at: null,
  depends_on: [], waiting_on: [], claim: null, needs_you: [], checks_summary: none, forked_from: null, flags: [], ...over,
});
const detailProps = (t) => ({ detail: { task: t, checks: [], evidence: [], events: [] }, names, now: NOW, onAction: noop, onApproveCheck: noop, onSetClass: noop });
const caps = (over = {}) => ({
  flight_school: true,
  kwant_id: 'k_sandy',
  default_tier: 'sandbox',
  policies: [],
  effective: [
    { task_class: 'general', tier: 'sandbox', from: 'any_class' },
    { task_class: 'release', tier: 'observe', from: 'class' },
    { task_class: 'docs', tier: 'act', from: 'class' },
  ],
  events: [
    { id: 2, kwant_id: 'k_sandy', task_class: 'release', kind: 'revoked', tier: 'observe', actor: 'person', detail: 'pushed a tag', created_at: '2026-10-28T11:00:00Z' },
    { id: 1, kwant_id: 'k_sandy', task_class: '*', kind: 'new_bot_default', tier: 'sandbox', actor: 'person', detail: null, created_at: '2026-10-28T09:00:00Z' },
  ],
  ...over,
});
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const bash = (extra = {}) => ({ id: 'a1', tool_name: 'Bash', details: { source: 'claude_builtin', tool: 'Bash', kwant_name: 'Sandy', cwd: '/w', arguments: { command: 'touch a.txt' }, ...extra } });

// ---------------------------------------------------------------- setting off: nothing changes

test('with Flight School off, nothing on screen changes', async () => {
  // The server sends task_class on every task, and owner_tier null while the setting is off.
  const before = task();
  const off = task({ task_class: 'general', owner_tier: null });
  assert.equal(ui.card({ task: off, names, now: NOW, onOpen: noop }), ui.card({ task: before, names, now: NOW, onOpen: noop }), 'the task card is byte-for-byte the same');
  assert.equal(ui.detail(detailProps(off)), ui.detail(detailProps(before)), 'the task panel is byte-for-byte the same');
  assert.doesNotMatch(ui.detail(detailProps(off)), /Flight School|Kind of work|data-tier/);

  // The bot's settings: no panel with the setting off, on an older server, or while loading.
  assert.equal(ui.panel({ kwantId: 'k_sandy', kwantName: 'Sandy', initial: caps({ flight_school: false }) }), '');
  assert.equal(ui.panel({ kwantId: 'k_sandy', kwantName: 'Sandy', initial: null }), '');

  // An approval card without a tier reason looks as before.
  assert.doesNotMatch(ui.approval(bash()), /data-tier-reason|Flight School/);

  // Settings: an older server has no switch to show; off shows the switch and nothing under it.
  assert.equal(fs.parseFlightSchoolSettings({ autofix: 'off' }), null);
  const offSetting = ui.setting({ value: { flight_school: false, flight_school_new_bot_tier: 'sandbox' }, onChange: noop });
  assert.match(offSetting, /role="switch"[^>]*aria-checked="false"/);
  assert.doesNotMatch(offSetting, /flight-school-options/);
});

// ---------------------------------------------------------------- settings

test('Settings: the switch says existing bots stay at Act, payments always ask, and where new bots start', () => {
  assert.deepEqual(fs.parseFlightSchoolSettings({ flight_school: true, flight_school_new_bot_tier: 'observe' }), { flight_school: true, flight_school_new_bot_tier: 'observe' });
  assert.deepEqual(fs.parseFlightSchoolSettings({ flight_school: false, flight_school_new_bot_tier: 'nonsense' }), { flight_school: false, flight_school_new_bot_tier: 'sandbox' });
  const t = text(ui.setting({ value: { flight_school: true, flight_school_new_bot_tier: 'sandbox' }, onChange: noop }));
  assert.match(t, /Payments always ask you, whatever the tier/);
  assert.match(t, /Bots you already have stay at Act, so nothing changes until you lower one/);
  assert.match(t, /Bots you add from now on start at/);
  assert.match(ui.setting({ value: { flight_school: true, flight_school_new_bot_tier: 'sandbox' }, onChange: noop }), /<option value="sandbox" selected="">Sandbox/);
});

// ---------------------------------------------------------------- the bot's tiers

test('a bot’s tiers: each kind of work, where its tier comes from, one-click Revoke, and the recent changes', () => {
  const html = ui.panel({ kwantId: 'k_sandy', kwantName: 'Sandy', initial: caps(), now: NOW });
  const t = text(html);
  assert.match(t, /How much Sandy may do on its own, per kind of work\. A tier only adds approval cards\. Payments always ask you\./);
  assert.match(html, /data-class="general" data-tier="sandbox" data-from="any_class"/);
  assert.match(t, /General From its tier for all work/);
  // Sandbox's hint matches the gate: read-only commands in the bot's own folder may run without a card.
  assert.match(t, /Commands that can change things ask you; reading its own folder doesn’t\./);
  assert.doesNotMatch(t, /everything else ask/);
  assert.match(t, /Release Set for this kind of work/);
  // Revoke steps one tier down and says where to; Observe has nothing lower.
  assert.match(html, /aria-label="Revoke all work to Observe"/);
  assert.match(html, /data-class="docs"[\s\S]*?aria-label="Revoke Docs to Sandbox"/);
  assert.doesNotMatch(html, /Revoke release/);
  assert.match(t, /You revoked release to Observe/);
  assert.match(t, /pushed a tag/);
  assert.match(t, /Started at Sandbox as a new bot/);
  assert.equal(fs.fromText('default'), 'Not set: works as today (Act)');
  assert.equal(fs.fromText('unreadable'), 'Couldn’t be read, so it’s treated as Observe');
  assert.equal(fs.revokeTarget('act'), 'sandbox');
  assert.equal(fs.revokeTarget('observe'), null);
  assert.equal(fs.toClass('  Release Notes! '), 'release-notes');
  assert.equal(fs.validClass(''), false);
});

test('the capabilities API calls the routes Lin’s #115 serves', async () => {
  const calls = [];
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body ? JSON.parse(init.body) : undefined });
    return new Response(JSON.stringify({ unchanged: false, capabilities: caps() }), { status: 200, headers: { 'Content-Type': 'application/json' } });
  };
  try {
    await m.capabilitiesApi.get('k sandy');
    await m.capabilitiesApi.set('k_sandy', '*', 'observe');
    await m.capabilitiesApi.revoke('k_sandy', 'release', 'observe');
    await m.missionsApi.setTaskClass('t1', 'release');
  } finally {
    globalThis.fetch = realFetch;
  }
  assert.deepEqual(calls.map((c) => `${c.method} ${c.url}`), [
    'GET /api/capabilities?kwant_id=k%20sandy',
    'POST /api/capabilities',
    'POST /api/capabilities/k_sandy/revoke',
    'PATCH /api/tasks/t1',
  ]);
  assert.deepEqual(calls[1].body, { kwant_id: 'k_sandy', task_class: '*', tier: 'observe' });
  assert.deepEqual(calls[2].body, { task_class: 'release', to_tier: 'observe' });
  assert.deepEqual(calls[3].body, { task_class: 'release' });
});

// ---------------------------------------------------------------- the board

test('with Flight School on, a task shows its kind of work and its bot’s tier; Act adds no badge', () => {
  const sandboxed = task({ task_class: 'release', owner_tier: 'sandbox' });
  assert.match(ui.card({ task: sandboxed, names, now: NOW, onOpen: noop }), /data-tier-badge="sandbox"[^>]*title="Flight School: release work at Sandbox"/);
  assert.doesNotMatch(ui.card({ task: task({ task_class: 'general', owner_tier: 'act' }), names, now: NOW, onOpen: noop }), /data-tier-badge/);
  const d = ui.detail(detailProps(sandboxed));
  assert.match(d, />Kind of work<\/dt>[\s\S]*data-task-class="release"/);
  assert.match(d, />Flight School tier<\/dt>[\s\S]*data-owner-tier="sandbox"[^>]*>Sandbox</);
  assert.equal(m.eventText({ id: 'e', task_id: 't1', kind: 'class_changed', actor: 'person', actor_name: null, from_state: null, to_state: null, detail: { from: 'general', to: 'release' }, created_at: '' }), 'You changed its kind of work to release');
});

test('a task the engine guard blocked sits in Needs you and says what to do', () => {
  const blocked = task({
    state: 'blocked',
    owner_tier: 'sandbox',
    task_class: 'general',
    owner_kwant_id: 'k_cody',
    owner_name: 'Cody',
    blocked_reason: 'This engine can’t be held to its tier: switch the bot to Claude or grant Act',
    needs_you: [{ kind: 'engine_cant_sandbox', id: 't1', text: 'This engine can’t be held to its tier: switch the bot to Claude or grant Act' }],
  });
  assert.equal(m.columnOfTask(blocked), 'needs_you');
  assert.equal(m.needsYouCount([blocked, task()]), 1);
  assert.equal(m.cardStatus(blocked, names, NOW), 'Blocked: its engine can’t be held to its tier');
  const t = text(ui.detail(detailProps(blocked)));
  assert.match(t, /Engine can’t be sandboxed: Cody runs on an engine that can’t be held to a lower tier, so nothing ran\. Switch Cody to a Claude model, or set its tier for this kind of work to Act, then press Unblock\./);
});

// ---------------------------------------------------------------- the approval card

test('an approval card says when the bot’s tier asked, and a payment card keeps its own reason', () => {
  const html = ui.approval(bash({ tier_reason: 'Sandbox: commands that can change things ask' }));
  const t = text(html);
  assert.match(html, /data-tier-reason/);
  assert.match(t, /Flight School: Sandbox: commands that can change things ask\. This bot’s tier asks you here, even with approvals off\./);
  assert.match(t, /Always this session/, 'a session “always allow” still counts below Act (D9)');
  const pay = ui.approval(bash({ payment_reason: 'a request that writes to stripe.com, a payment service', tier_reason: 'Sandbox: commands that can change things ask' }));
  assert.doesNotMatch(pay, /data-tier-reason/);
  assert.match(text(pay), /This looks like a payment/);
});
