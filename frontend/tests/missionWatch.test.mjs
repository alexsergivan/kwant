// 0.9.0 P1 "watch a mission": parsing, the chip for every state, the report's rows and text,
// the chat message and the Inbox item (p1-contract.md, designs/p1-watch-mission.md).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const w = await loadBundled(`export * from './src/missionWatch'; export * from './src/away';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { WatchChip, WatchReportBlock, WatchChatMessage } from './src/components/MissionWatch';
  export const chip = (props) => renderToStaticMarkup(React.createElement(WatchChip, props));
  export const report = (props) => renderToStaticMarkup(React.createElement(WatchReportBlock, props));
  export const chat = (props) => renderToStaticMarkup(React.createElement(WatchChatMessage, props));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const TZ = 'UTC';

const claim = (over = {}) => ({
  task_id: 't-1', task_title: 'Write the tests', owner_kwant_id: 'k-lin', owner_name: 'Lin', state: 'done',
  claimed: 'Done: tests pass', checked: 'Ran `npm test`: 3 fail', verdict: 'doesnt_hold', needs_you: false, ...over,
});
const rawReport = (over = {}) => ({
  id: 'wc-1', mission_id: 'm-1', kwant_id: 'k-rigel', kwant_name: 'Rigel', trigger: 'stop', status: 'done',
  stopped_at: '2026-11-10T18:10:00Z', started_at: '2026-11-10T18:11:00Z', finished_at: '2026-11-10T18:14:00Z',
  counts: { claims: 3, dont_hold: 1, needs_you: 1, cant_tell: 0, holds: 2 },
  claims: [
    claim({ task_id: 't-3', task_title: 'Ship the docs', verdict: 'holds', claimed: 'Done', checked: 'The page is live' }),
    claim({ task_id: 't-5', task_title: 'Deploy the site', verdict: 'holds', needs_you: true, state: 'waiting', claimed: 'Waiting for your OK', checked: 'The request is open' }),
    claim(),
  ],
  actions: [
    { kind: 'restart', task_id: 't-1', bot_id: 'k-lin', bot_name: 'Lin', reason: '3 tests fail', applied: true },
    { kind: 'needs_you', task_id: 't-5', what: 'waits for your OK to deploy', since: '2026-11-10T18:12:00Z' },
    { kind: 'refused', what: 'approve the deploy', tool: 'approve', why: 'Only you can approve' },
    { kind: 'something_new', what: 'x' },
  ],
  inbox: { open: true, line: 'Rigel: t-5 has waited for your OK to deploy since 18:12.' },
  message_id: 'msg-1', tokens: 1200, ...over,
});
const watch = (over = {}) => ({
  state: 'watching', kwant_id: 'k-rigel', kwant_name: 'Rigel', set_at: '2026-11-10T17:00:00Z', stopped: false, stopped_at: null,
  last_check: null, reason: null, can_check_now: false, ...over,
});
const lastCheck = (counts) => ({ id: 'wc-1', status: 'done', trigger: 'stop', started_at: '2026-11-10T18:11:00Z', finished_at: '2026-11-10T18:14:00Z', counts });
const mission = (wv) => ({ id: 'm-1', title: 'Launch', status: 'active', watch: wv === undefined ? undefined : w.parseWatch(wv) });

test('parseReport: a done check with matching counts parses; unknown actions are skipped', () => {
  const r = w.parseReport(rawReport());
  assert.ok(r);
  assert.equal(r.claims.length, 3);
  assert.deepEqual(r.actions.map((a) => a.kind), ['restart', 'needs_you', 'refused']);
  assert.equal(r.inbox.open, true);
});

test('parseReport: holds may count the needs-you rows or not (both contract readings)', () => {
  assert.ok(w.parseReport(rawReport({ counts: { claims: 3, dont_hold: 1, needs_you: 1, cant_tell: 0, holds: 1 } })));
  assert.ok(w.parseReport(rawReport({ counts: { claims: 3, dont_hold: 1, needs_you: 1, cant_tell: 0, holds: 2 } })));
});

test('parseReport: invalid data falls back (null) instead of drawing wrong numbers', () => {
  assert.equal(w.parseReport(rawReport({ counts: { claims: 4, dont_hold: 1, needs_you: 1, cant_tell: 0, holds: 2 } })), null);
  assert.equal(w.parseReport(rawReport({ counts: { claims: 3, dont_hold: 2, needs_you: 1, cant_tell: 0, holds: 2 } })), null);
  assert.equal(w.parseReport(rawReport({ counts: null })), null);
  assert.equal(w.parseReport(rawReport({ counts: { claims: 3, dont_hold: -1, needs_you: 1, cant_tell: 0, holds: 2 } })), null);
  assert.equal(w.parseReport(rawReport({ claims: [claim({ verdict: 'maybe' })], counts: { claims: 1, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 0 } })), null);
  assert.equal(w.parseReport(rawReport({ claims: [], counts: { claims: 0, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 0 } })), null);
  assert.equal(w.parseReport(rawReport({ status: 'weird' })), null);
  assert.equal(w.parseReport('nope'), null);
  // A failed check has no claims and still parses.
  const f = w.parseReport(rawReport({ status: 'failed', counts: null, claims: [], error: 'timed out' }));
  assert.equal(f.status, 'failed');
});

test('parseWatchMeta: only kind "watch"; a failed check carries its reason', () => {
  assert.equal(w.parseWatchMeta(JSON.stringify({ kind: 'bot_action' })), null);
  const ok = w.parseWatchMeta(JSON.stringify({ kind: 'watch', mission_id: 'm-1', check_id: 'wc-1', status: 'done', watch_report: rawReport() }));
  assert.equal(ok.missionId, 'm-1');
  assert.ok(ok.report);
  const failed = w.parseWatchMeta({ kind: 'watch', mission_id: 'm-1', status: 'failed', watch_report: rawReport({ status: 'failed', claims: [], counts: null, reason: { code: 'timeout', text: 'the turn timed out' } }) });
  assert.equal(failed.report, null);
  assert.equal(failed.failed, 'the turn timed out');
});

test('parseWatch and parseCandidates drop what they cannot read', () => {
  assert.equal(w.parseWatch({ state: 'sleeping' }), null);
  assert.equal(w.parseWatch(watch()).state, 'watching');
  const cs = w.parseCandidates([
    { kwant_id: 'k1', name: 'Rigel', relation: 'no_task', can_run: true, default: true },
    { kwant_id: 'k2', name: 'Lin', relation: 'lead', can_run: false, reason: { code: 'no_model', text: 'it has no model' } },
    { kwant_id: 'k3', relation: 'lead' },
    { kwant_id: 'k4', name: 'X', relation: 'boss' },
  ]);
  assert.deepEqual(cs.map((c) => c.kwant_id), ['k1', 'k2']);
  assert.equal(w.defaultCandidate(cs).kwant_id, 'k1');
  assert.equal(w.relationText(cs[1]), "can’t run: it has no model");
});

test('chipView: every state', () => {
  const v = (wv, status = 'active') => w.chipView({ ...mission(wv), status }, TZ);
  assert.equal(v(undefined).text, 'Watch');
  assert.equal(v(watch(), 'done'), null);
  assert.equal(v(watch({ state: 'off' })).click, 'setup');
  const nt = v(watch({ state: 'no_tasks' }));
  assert.equal(nt.disabled, true);
  assert.equal(nt.tooltip, 'Add tasks first. There’s nothing to check yet.');
  assert.equal(v(watch({ state: 'finished' })), null);
  assert.equal(v(watch()).text, 'Watching · Rigel');
  const ck = v(watch({ state: 'checking' }));
  assert.equal(ck.icon, 'loader');
  assert.equal(ck.text, 'Rigel is checking the claims');
  const dh = v(watch({ state: 'dont_hold', last_check: lastCheck({ claims: 3, dont_hold: 1, needs_you: 1, cant_tell: 0, holds: 1 }) }));
  assert.deepEqual([dh.text, dh.tone, dh.click], ['1 claim doesn’t hold · 18:14', 'rose', 'report']);
  assert.equal(v(watch({ state: 'dont_hold', last_check: lastCheck({ claims: 3, dont_hold: 2, needs_you: 0, cant_tell: 0, holds: 1 }) })).text, '2 claims don’t hold · 18:14');
  const ah = v(watch({ state: 'all_hold', last_check: lastCheck({ claims: 4, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 4 }) }));
  assert.deepEqual([ah.text, ah.tone], ['All 4 claims hold · 18:14', 'teal']);
  const ct = v(watch({ state: 'cant_tell', last_check: lastCheck({ claims: 4, dont_hold: 0, needs_you: 0, cant_tell: 1, holds: 3 }) }));
  assert.deepEqual([ct.text, ct.tone], ['4 claims checked · 18:14', 'muted']);
  assert.equal(v(watch({ state: 'same_stop', last_check: lastCheck({ claims: 1, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 1 }) })).text, 'No change since the 18:14 check');
  const cr = v(watch({ state: 'cant_run', reason: { code: 'no_model', text: 'it has no model' } }));
  assert.deepEqual([cr.text, cr.tone], ['Rigel can’t check: it has no model', 'amber']);
  assert.equal(v(watch({ state: 'over_limit' })).tone, 'amber');
  assert.equal(v(watch({ state: 'failed', can_check_now: true })).click, 'retry');
  assert.equal(v(watch({ state: 'failed', can_check_now: false })).click, 'settings');
});

test('reportOpensItself only for a "doesn’t hold" or a "needs you"', () => {
  const p = (state, counts) => w.reportOpensItself(w.parseWatch(watch({ state, last_check: lastCheck(counts) })));
  assert.equal(p('dont_hold', { claims: 2, dont_hold: 1, needs_you: 0, cant_tell: 0, holds: 1 }), true);
  assert.equal(p('all_hold', { claims: 2, dont_hold: 0, needs_you: 1, cant_tell: 0, holds: 2 }), true);
  assert.equal(p('all_hold', { claims: 2, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 2 }), false);
});

test('orderClaims, tints and summary chips follow the design and the server counts', () => {
  const r = w.parseReport(rawReport());
  const { shown, folded } = w.orderClaims(r);
  assert.deepEqual(shown.map((c) => c.task_id), ['t-1', 't-5']);
  assert.deepEqual(folded.map((c) => c.task_id), ['t-3']);
  assert.deepEqual(shown.map(w.rowTint), ['rose', 'amber']);
  assert.deepEqual(w.summaryChips(r.counts).map((c) => c.text), ['1 don’t hold', '1 needs you', '2 hold']);
});

test('chat text, needs-you lines and code parts', () => {
  const r = w.parseReport(rawReport());
  assert.equal(w.chatText(r, TZ), 'Checked 3 claims after the mission stopped at 18:10. 1 don’t hold, 1 needs you, 2 hold. I restarted Write the tests with Lin.');
  const all = w.parseReport(rawReport({ counts: { claims: 1, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 1 }, claims: [claim({ verdict: 'holds' })], actions: [] }));
  assert.equal(w.chatText(all, TZ), 'Checked 1 claim after the mission stopped at 18:10. All hold.');
  const a = w.firstNeedsYou(r);
  assert.equal(w.youText(r, a, TZ), 'Deploy the site waits for your OK to deploy since 18:12.');
  assert.deepEqual(w.codeParts('Ran `npm test` now'), [{ code: false, text: 'Ran ' }, { code: true, text: 'npm test' }, { code: false, text: ' now' }]);
  assert.equal(w.failedChatText('the turn timed out'), 'The check failed: the turn timed out. Nothing was restarted.');
});

test('the chip is a labelled button with a polite live label', () => {
  const view = w.chipView(mission(watch()), TZ);
  const html = ui.chip({ view, expanded: false, controls: 'p', onClick: () => {} });
  assert.match(html, /<button/);
  assert.match(html, /aria-label="Watching · Rigel, watch settings"/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, /aria-live="polite"/);
});

test('the report: a real table with headers, rows in order, nothing rendered as HTML', () => {
  const evil = rawReport({ claims: rawReport().claims.map((c, i) => (i === 2 ? { ...c, checked: '<img src=x onerror=alert(1)>' } : c)) });
  const html = ui.report({ report: w.parseReport(evil), phone: false, timeZone: TZ, id: 'r' });
  assert.match(html, /<table/);
  assert.match(html, /<caption/);
  assert.equal((html.match(/<th[^>]*scope="col"/g) || []).length >= 3, true);
  assert.doesNotMatch(html, /<img/);
  const t = text(html);
  assert.ok(t.indexOf('Write the tests') < t.indexOf('Deploy the site'));
  assert.match(t, /Show 1 that hold/);
  assert.match(t, /What Rigel did/);
  assert.match(t, /Rigel tried to approve the deploy\. Only you can approve, so kwant left it for you\./);
});

test('the chat message falls back to the message text when the report is invalid', () => {
  const html = ui.chat({ report: null, fallbackText: 'Checked 3 claims.', watcherName: 'Rigel', timeZone: TZ });
  assert.match(text(html), /Checked 3 claims\./);
  const ok = ui.chat({ report: w.parseReport(rawReport()), fallbackText: 'x', watcherName: 'Rigel', timeZone: TZ, onSeeClaims: () => {} });
  assert.match(text(ok), /See the claims/);
  const all = w.parseReport(rawReport({ counts: { claims: 1, dont_hold: 0, needs_you: 0, cant_tell: 0, holds: 1 }, claims: [claim({ verdict: 'holds' })], actions: [], inbox: null }));
  assert.doesNotMatch(text(ui.chat({ report: all, fallbackText: 'x', watcherName: 'Rigel', timeZone: TZ, onSeeClaims: () => {} })), /See the claims/);
});

test('Inbox: a watch item gets the card shape, an amber Watch label, and others pass through', () => {
  const raw = { kind: 'watch', id: 'wc-1', kwant_id: 'k-rigel', kwant_name: 'Rigel', title: 'Launch', summary: 'Rigel: …', status: 'open', at: '2026-11-10T18:14:00Z', mission_id: 'm-1', task_id: 't-5', thread_id: 'th-1' };
  const n = w.normalizeAwayItem(raw);
  assert.equal(n.timestamp, '2026-11-10T18:14:00Z');
  assert.deepEqual(n.link, { thread_id: 'th-1', channel_id: null, direct_kwant_id: 'k-rigel' });
  assert.equal(n.mission_id, 'm-1');
  assert.deepEqual(w.itemLabel(n), { label: 'Watch', tone: 'amber' });
  const q = { kind: 'question', id: 'q', timestamp: 't', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k' } };
  assert.equal(w.normalizeAwayItem(q), q);
});
