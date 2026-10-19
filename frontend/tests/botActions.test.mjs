import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const b = await loadBundled(`export * from './src/botActions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { BotActionCard } from './src/components/BotActionCard';
  import { HireCard } from './src/components/HireCard';
  export const action = (props) => renderToStaticMarkup(React.createElement(BotActionCard, props));
  export const hire = (props) => renderToStaticMarkup(React.createElement(HireCard, props));
`);

// Visible text only, so assertions don't match class names or ids.
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((m) => text(m[1]));

const routineMeta = (over = {}) => ({
  kind: 'bot_action', action_id: 'a1', action: 'routine_create', kwant_id: 'k1', target_kind: 'routine', target_id: 'r1',
  summary: "created routine 'Morning motivation', daily 07:00 Europe/Berlin, delivering to #general",
  details: {
    routine_id: 'r1', name: 'Morning motivation', kwant_id: 'k1', bot_name: 'Coach',
    schedule: { type: 'daily', time: '07:00', tz: 'Europe/Berlin' }, schedule_text: 'daily 07:00 Europe/Berlin', enabled: true,
    next_run_at: '2026-10-19T05:00:00Z', channel_id: 'c1', delivers_to: '#general',
  },
  status: 'applied', can_undo: true, needs_confirmation: false, created_at: '2026-10-18T10:00:00Z', resolved_at: null, ...over,
});
const actionProps = (meta, over = {}) => ({
  meta: b.parseBotActionMeta(meta), messageId: 'm1', kwantName: 'Coach', kwantId: 'k1',
  onDecide: async () => {}, onOpenRoutine: () => {}, ...over,
});

test('bot action meta parses and finished actions never offer buttons', () => {
  const m = b.parseBotActionMeta(JSON.stringify(routineMeta()));
  assert.equal(m.action_id, 'a1');
  assert.equal(m.can_undo, true);
  assert.equal(b.parseBotActionMeta(routineMeta({ status: 'undone', can_undo: true })).can_undo, false);
  assert.equal(b.parseBotActionMeta(routineMeta({ status: 'confirmed', needs_confirmation: true })).needs_confirmation, false);
  assert.equal(b.parseBotActionMeta({ kind: 'hire_proposal', hire_id: 'h' }), null);
  assert.equal(b.parseBotActionMeta({ kind: 'bot_action' }), null);
  assert.equal(b.parseBotActionMeta('nope'), null);
});

test('applied routine card: summary, routine facts, Undo, Edit and Pause', () => {
  const html = ui.action(actionProps(routineMeta()));
  const t = text(html);
  assert.match(t, /Coach set up a routine/);
  assert.match(t, /Created routine 'Morning motivation'/);
  assert.match(t, /When Every day at 07:00 \(Europe\/Berlin\)/);
  assert.match(t, /Runs as Coach/);
  assert.match(t, /Posts in #general/);
  assert.match(t, /Next run/);
  assert.match(t, /Done\./);
  assert.deepEqual(buttons(html), ['Undo', 'Edit', 'Pause']);
});

test('undone card says Undone, not Done, and offers nothing', () => {
  const html = ui.action(actionProps(routineMeta({ status: 'undone', can_undo: false, resolved_at: '2026-10-18T10:05:00Z' })));
  const t = text(html);
  assert.match(t, /Undone\. It's back the way it was\./);
  assert.doesNotMatch(t, /\bDone\./);
  assert.doesNotMatch(t, /Next run/);
  assert.match(html, /line-through/);
  assert.deepEqual(buttons(html), []);
});

test('a live update wins over the message meta, and a resolved card never goes back', () => {
  const applied = b.parseBotActionMeta(routineMeta());
  const undone = b.parseBotActionMeta(routineMeta({ status: 'undone', can_undo: false }));
  assert.equal(b.botActionView(applied, undone).status, 'undone');
  assert.equal(b.botActionView(undone, applied).status, 'undone');
  assert.equal(b.botActionView(applied, b.parseBotActionMeta(routineMeta({ action_id: 'other', status: 'undone' }))).status, 'applied');

  let map = b.applyBotAction({}, { id: 'a1', status: 'undone', card: routineMeta({ status: 'undone' }) });
  map = b.applyBotAction(map, { id: 'a1', status: 'applied', card: routineMeta() });
  assert.equal(map.a1.status, 'undone');
  assert.equal(b.applyBotAction(map, { id: 'x', card: null }), map);

  const html = ui.action(actionProps(routineMeta(), { live: undone }));
  assert.match(text(html), /Undone\./);
  assert.deepEqual(buttons(html), []);
});

test('pending action: reason, Confirm and Decline; then Confirmed or Declined', () => {
  const pending = routineMeta({
    action: 'routine_delete', status: 'pending', can_undo: false, needs_confirmation: true,
    summary: "delete routine 'Morning motivation'", details: { ...routineMeta().details, reason: 'You said you no longer need it.' },
  });
  const html = ui.action(actionProps(pending));
  const t = text(html);
  assert.match(t, /Needs your OK\. Nothing changes until you confirm\./);
  assert.match(t, /Why: You said you no longer need it\./);
  assert.deepEqual(buttons(html), ['Confirm', 'Decline']);

  const confirmed = ui.action(actionProps({ ...pending, status: 'confirmed', needs_confirmation: false }));
  assert.match(text(confirmed), /Confirmed\. It's done\./);
  assert.doesNotMatch(text(confirmed), /Why:/);
  assert.deepEqual(buttons(confirmed), []);

  const declined = ui.action(actionProps({ ...pending, status: 'declined', needs_confirmation: false }));
  assert.match(text(declined), /Declined\. Nothing changed\./);
  assert.deepEqual(buttons(declined), []);
});

test('run now cannot be undone and shows the run, not the next run', () => {
  const html = ui.action(
    actionProps(routineMeta({ action: 'routine_run_now', can_undo: false, details: { ...routineMeta().details, run_id: 'run1', run_status: 'ok' } })),
  );
  const t = text(html);
  assert.match(t, /Done\. This can't be undone\./);
  assert.match(t, /This run Finished/);
  assert.doesNotMatch(t, /Next run/);
  assert.deepEqual(buttons(html), ['Edit', 'Pause']);
});

test('Edit and Pause only while the routine exists; Pause hides when it is paused', () => {
  const r = (over) => b.routineButtons(b.parseBotActionMeta(routineMeta(over)));
  assert.deepEqual(r({}), { edit: true, pause: true });
  assert.deepEqual(r({ details: { ...routineMeta().details, enabled: false } }), { edit: true, pause: false });
  assert.deepEqual(r({ action: 'routine_pause' }), { edit: true, pause: false });
  assert.deepEqual(r({ action: 'routine_delete' }), { edit: false, pause: false });
  assert.deepEqual(r({ status: 'undone' }), { edit: false, pause: false });
  assert.deepEqual(r({ target_kind: 'bot' }), { edit: false, pause: false });
  const paused = text(ui.action(actionProps(routineMeta({ action: 'routine_pause', details: { ...routineMeta().details, enabled: false } }))));
  assert.match(paused, /Next run Paused/);
});

test('non-routine actions: no routine facts, verb fallback', () => {
  const html = ui.action(
    actionProps(routineMeta({ action: 'bot_update', target_kind: 'bot', target_id: 'k2', summary: 'renamed Scout to Finder', details: { name: 'Finder' } })),
  );
  const t = text(html);
  assert.match(t, /Renamed Scout to Finder/);
  assert.doesNotMatch(t, /Runs as|When/);
  assert.deepEqual(buttons(html), ['Undo']);
  assert.equal(b.actionVerb('something_new'), 'changed something');
});

test('schedules in words: ours for known shapes, the server text otherwise', () => {
  assert.equal(b.scheduleWords({ type: 'monthly', time: '09:00', days: [1, 'last'], tz: 'UTC' }), 'Every month on the 1st and last day at 09:00 (UTC)');
  assert.equal(b.scheduleWords({ type: 'cron', expression: '0 9 * * MON#2', tz: 'UTC' }), 'Custom rule 0 9 * * MON#2 (UTC)');
  assert.equal(b.scheduleWords({ type: 'lunar' }, 'when the moon is full'), 'When the moon is full');
  assert.equal(b.scheduleWords(null, ''), 'Unknown schedule');
  assert.equal(b.asSchedule({ type: 'monthly', time: '09:00', days: [0, 5, 'last', 'x'], tz: 'UTC' }).days.join(), '5,last');
  assert.equal(b.asSchedule({ type: 'weekly', time: '09:00', weekdays: ['mon', 'xyz'], tz: 'UTC' }).weekdays.join(), 'mon');
  assert.equal(b.asSchedule({ type: 'cron' }), null);
});

// ---------------------------------------------------------------------------
// Hiring

const hireMeta = (over = {}) => ({
  kind: 'hire_proposal', hire_id: 'h1', kwant_id: 'k1', proposer_name: 'Ada', name: 'Ledger', role: 'Bookkeeper',
  instructions: 'Keep the books.\nReconcile weekly.', model: 'auto', tools: [{ id: 's1', name: 'Sheets' }, { id: 's2', name: null }],
  team: { id: 't1', name: 'Finance' }, reason: 'You keep asking for invoices.',
  evidence: { basis: 'repeated', similar_requests: ['m1', 'm2', 'm3'] }, requested_by_person: false,
  status: 'pending', auto: false, choices: ['deny', 'accept_once', 'always_accept'],
  hired_kwant_id: null, action_id: null, can_undo: false, policy: 'ask', note: null, created_at: '2026-10-18T10:00:00Z', resolved_at: null, ...over,
});
const hireProps = (meta, over = {}) => ({ meta: b.parseHireMeta(meta), messageId: 'm9', kwantName: 'Ada', kwantId: 'k1', onDecide: async () => {}, ...over });

test('pending hire: who proposes, the bot, why, evidence, and the three choices', () => {
  const html = ui.hire(hireProps(hireMeta()));
  const t = text(html);
  assert.match(t, /Ada proposes hiring a new bot/);
  assert.match(t, /Ledger Bookkeeper/);
  assert.match(t, /Why: You keep asking for invoices\./);
  assert.match(t, /Based on: You asked for this kind of work 3 times in this space\./);
  assert.match(t, /Team Finance/);
  assert.match(t, /Model Picked automatically/);
  assert.match(t, /Tools Sheets, Unknown tool/);
  assert.match(t, /Waiting for you\. Ada carries on with your request meanwhile\./);
  assert.match(t, /up to 3 a day, each with Undo/);
  assert.deepEqual(buttons(html), ['Show its instructions', 'Accept once', 'Always accept', 'Deny']);
  // Instructions start collapsed.
  assert.doesNotMatch(t, /Keep the books/);
  assert.match(html, /aria-expanded="false"/);
});

test('accepted hire: Hired, joined the team, with Undo', () => {
  const html = ui.hire(hireProps(hireMeta({ status: 'accepted', choices: [], hired_kwant_id: 'k9', action_id: 'a9', can_undo: true })));
  assert.match(text(html), /Hired\. Ledger joined Finance\./);
  assert.deepEqual(buttons(html), ['Show its instructions', 'Undo']);
  // can_undo without an action id can't be honoured.
  assert.equal(b.parseHireMeta(hireMeta({ status: 'accepted', can_undo: true })).can_undo, false);
});

test('auto-accepted hire shows why it happened at once, the note, and Undo', () => {
  const html = ui.hire(
    hireProps(hireMeta({ status: 'auto_accepted', auto: true, choices: [], action_id: 'a9', can_undo: true, policy: 'auto', note: '2 of 3 automatic hires used today.' })),
  );
  const t = text(html);
  assert.match(t, /Ada hired a new bot/);
  assert.match(t, /Hired at once, because you chose Always accept\./);
  assert.match(t, /2 of 3 automatic hires used today\./);
  assert.ok(buttons(html).includes('Undo'));
});

test('denied and undone hires offer nothing more', () => {
  const denied = ui.hire(hireProps(hireMeta({ status: 'denied', choices: ['deny'] })));
  assert.match(text(denied), /Not hired\. Bots here won't suggest this kind of bot again unless you ask\./);
  assert.deepEqual(buttons(denied), ['Show its instructions']);
  const undone = ui.hire(hireProps(hireMeta({ status: 'undone', action_id: 'a9', can_undo: true })));
  assert.match(text(undone), /Hire undone\. Ledger was removed\./);
  assert.deepEqual(buttons(undone), ['Show its instructions']);
});

test('a hire live update wins unless it is older', () => {
  const pending = b.parseHireMeta(hireMeta());
  const accepted = b.parseHireMeta(hireMeta({ status: 'accepted', action_id: 'a9', can_undo: true }));
  const undone = b.parseHireMeta(hireMeta({ status: 'undone' }));
  assert.equal(b.hireView(pending, accepted).status, 'accepted');
  assert.equal(b.hireView(accepted, pending).status, 'accepted');
  assert.equal(b.hireView(accepted, undone).status, 'undone');
  let map = b.applyHire({}, { id: 'h1', status: 'undone', card: hireMeta({ status: 'undone' }) });
  map = b.applyHire(map, { id: 'h1', status: 'accepted', card: hireMeta({ status: 'accepted' }) });
  assert.equal(map.h1.status, 'undone');
  const html = ui.hire(hireProps(hireMeta(), { live: accepted }));
  assert.match(text(html), /Hired\./);
});

test('evidence in plain words', () => {
  assert.equal(b.evidenceText({ basis: 'specialist', needs: ['tools', 'schedule'], gap: 'no bot here can read invoices' }, false),
    'This work needs its own tools and a schedule. No bot here can read invoices.');
  assert.equal(b.evidenceText({ basis: 'repeated', similar_requests: ['m1'] }, false), 'You asked for this kind of work 1 time in this space.');
  assert.equal(b.evidenceText(null, true), 'You asked for this kind of bot.');
  assert.equal(b.parseHireMeta(hireMeta({ evidence: 'Plain text reason' })).evidence.text, 'Plain text reason');
  assert.equal(b.modelText('claude-sonnet-5-5'), 'claude-sonnet-5-5');
  assert.equal(b.parseHireMeta(hireMeta({ team: null })).team, null);
  assert.equal(b.parseHireMeta(hireMeta({ name: '' })), null);
});

test('409 and 403 read as plain words', () => {
  assert.equal(b.decisionError(new b.ApiError('hire is not pending', 409), 'hire it'), 'Couldn’t hire it: it was already decided or changed (hire is not pending).');
  assert.equal(b.decisionError(new b.ApiError('Request failed (HTTP 409).', 409), 'undo it'), 'Couldn’t undo it: it was already decided or changed.');
  assert.match(b.decisionError(new b.ApiError('nope', 403), 'deny it'), /only you can do this/);
  assert.equal(b.decisionError(new Error('Network down'), 'deny it'), 'Couldn’t deny it: Network down');
});

test('hire routes send {} for Accept once and {always:true} for Always accept', async () => {
  const calls = [];
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push([init.method, url, init.body]);
    if (url.endsWith('/deny')) return new Response(JSON.stringify({ error: 'hire is not pending' }), { status: 409 });
    return new Response(JSON.stringify({ id: 'h1', status: 'accepted' }), { status: 200 });
  };
  try {
    await b.hiresApi.accept('h1');
    await b.hiresApi.accept('h1', true);
    await b.hiresApi.setPolicy('sp 1', { clear_denials: true });
    await assert.rejects(b.hiresApi.deny('h1'), (e) => e instanceof b.ApiError && e.status === 409 && e.message === 'hire is not pending');
  } finally {
    globalThis.fetch = realFetch;
  }
  assert.deepEqual(calls, [
    ['POST', '/api/hires/h1/accept', '{}'],
    ['POST', '/api/hires/h1/accept', '{"always":true}'],
    ['PUT', '/api/spaces/sp%201/hire-policy', '{"clear_denials":true}'],
    ['POST', '/api/hires/h1/deny', undefined],
  ]);
});

test('hire policy parses with safe defaults', () => {
  const p = b.parseHirePolicy({
    space_id: 's1', policy: 'auto', auto_hires_last_24h: 2, auto_hires_per_day: 3,
    denied: [{ hire_id: 'h2', name: 'Spammer', role: 'Outreach', denied_at: '2026-10-17T10:00:00Z' }, { hire_id: 'h3' }, null],
  });
  assert.equal(p.policy, 'auto');
  assert.equal(p.auto_hires_last_24h, 2);
  assert.deepEqual(p.denied.map((d) => d.name), ['Spammer']);
  const d = b.parseHirePolicy({ policy: 'sometimes' });
  assert.equal(d.policy, 'ask');
  assert.equal(d.auto_hires_per_day, 3);
  assert.deepEqual(d.denied, []);
  assert.equal(b.parseHirePolicy(null), null);
});

test('bot and hire events that add or remove a bot ask the sidebar to reload', () => {
  const action = (over) => ({ id: 'a9', status: over.status ?? 'applied', card: routineMeta({ action_id: 'a9', ...over }) });
  const hireEv = (status) => ({ id: 'h9', status, card: { kind: 'hire_proposal', hire_id: 'h9', kwant_id: 'k1', name: 'Chef', role: 'Cook', status } });
  // A bot made, renamed, hired, undone or deleted: reload.
  assert.equal(b.changesBots('bot_action_updated', { action: action({ action: 'bot_create', target_kind: 'bot', target_id: 'k2' }) }), true);
  assert.equal(b.changesBots('bot_action_updated', { action: action({ action: 'bot_hire', target_kind: 'bot', status: 'undone' }) }), true);
  assert.equal(b.changesBots('BotActionUpdated', { action: action({ action: 'bot_delete', target_kind: 'bot', status: 'confirmed' }) }), true);
  assert.equal(b.changesBots('hire_updated', { hire: hireEv('accepted') }), true);
  assert.equal(b.changesBots('HireUpdated', { hire: hireEv('auto_accepted') }), true);
  assert.equal(b.changesBots('hire_updated', { hire: hireEv('undone') }), true);
  // Nothing about the bot list changed yet, or not a bot at all: leave it.
  assert.equal(b.changesBots('bot_action_updated', { action: action({ action: 'bot_delete', target_kind: 'bot', status: 'pending' }) }), false);
  assert.equal(b.changesBots('bot_action_updated', { action: action({ action: 'bot_delete', target_kind: 'bot', status: 'declined' }) }), false);
  assert.equal(b.changesBots('bot_action_updated', { action: action({}) }), false);
  assert.equal(b.changesBots('hire_updated', { hire: hireEv('pending') }), false);
  assert.equal(b.changesBots('hire_updated', { hire: hireEv('denied') }), false);
  assert.equal(b.changesBots('message_added', { message: {} }), false);
  assert.equal(b.changesBots('hire_updated', null), false);
});
