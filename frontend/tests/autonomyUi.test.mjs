// Autonomy UI: a card whose turn stopped waiting is still open (status `waiting`) and says so, and
// the per-space Loose ends owner setting (GET/PUT /api/zombie/owner).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  export { approvalFromAway, upsertApproval, syncApprovals, lateAnswerText } from './src/approvals';
  export { itemLabel } from './src/away';
  export { ownerChange, ownerSummary } from './src/looseEnds';
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernApprovalCard } from './src/components/ModernApprovalCard';
  import { AwayView } from './src/components/AwayView';
  import { LooseOwnerView, LooseEndsView } from './src/components/LooseEndsView';
  export const card = (approval, extra = {}) => renderToStaticMarkup(React.createElement(ModernApprovalCard, { approval, onResolve: () => {}, ...extra }));
  export const away = (props) => renderToStaticMarkup(React.createElement(AwayView, props));
  export const owner = (props) => renderToStaticMarkup(React.createElement(LooseOwnerView, props));
  export const loose = (props) => renderToStaticMarkup(React.createElement(LooseEndsView, props));
  export { ModernApprovalCard, LooseOwnerView, React, renderToStaticMarkup };
`);

// Shaped like item() in src/server/away.rs for a row from away_waiting_approvals.
const awayItem = (status) => ({
  kind: 'approval', id: 'ap1', kwant_id: 'k1', kwant_name: 'Architect', title: 'Use Bash', summary: 'command: ls',
  status, timestamp: '2026-10-22T01:00:00Z', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k1' },
  tool_name: 'Bash', details: { source: 'claude_builtin', tool: 'Bash', arguments: { command: 'ls build/' } },
});

test('a waiting away item becomes a still-open card; a pending one does not', () => {
  assert.equal(m.approvalFromAway(awayItem('waiting')).waiting, true);
  assert.equal(m.approvalFromAway(awayItem('pending')).waiting, undefined);
});

test('the server decides waiting: a live card turns waiting on sync, and back', () => {
  const live = { id: 'ap1', tool_name: 'Bash', details: { source: 'claude_builtin', tool: 'Bash', arguments: { command: 'ls build/' } }, kwant_id: 'k1', link: null };
  const synced = m.syncApprovals([live], [m.approvalFromAway(awayItem('waiting'))]);
  assert.equal(synced[0].waiting, true);
  const again = m.upsertApproval(synced, m.approvalFromAway(awayItem('pending')));
  assert.equal(again[0].waiting, undefined);
  // A summary-only server row keeps the live details but still takes the server's waiting flag.
  const summary = { ...m.approvalFromAway({ ...awayItem('waiting'), details: null }), waiting: true };
  const kept = m.upsertApproval([live], summary);
  assert.equal(kept[0].details.source, 'claude_builtin');
  assert.equal(kept[0].waiting, true);
});

test('a still-open card says so, keeps Allow and Deny, and drops Always', () => {
  const html = m.card({ ...m.approvalFromAway(awayItem('waiting')), details: { ...awayItem('waiting').details, kwant_name: 'Architect' } });
  assert.match(html, /Still open: you can still Allow/);
  assert.match(html, /Architect stopped waiting for an answer and nothing ran/);
  assert.match(html, /Allow and continue/);
  assert.match(html, />Deny</);
  assert.doesNotMatch(html, /Always/);
  assert.doesNotMatch(html, /Action Approval Required/);
  // A normal card is unchanged.
  const normal = m.card(m.approvalFromAway(awayItem('pending')));
  assert.match(normal, /Action Approval Required/);
  assert.match(normal, /Always this session/);
  assert.doesNotMatch(normal, /Still open/);
});

test('Allow and Deny on a still-open card answer with allow_once and deny', () => {
  const calls = [];
  const el = m.ModernApprovalCard({ approval: { ...m.approvalFromAway(awayItem('waiting')) }, onResolve: (id, d) => calls.push([id, d]) });
  const buttons = [];
  const walk = (n) => {
    if (!n || typeof n !== 'object') return;
    if (Array.isArray(n)) return n.forEach(walk);
    if (n.type === 'button') buttons.push(n);
    walk(n.props?.children);
  };
  walk(el);
  assert.equal(buttons.length, 2);
  for (const b of buttons) b.props.onClick();
  assert.deepEqual(calls, [['ap1', 'deny'], ['ap1', 'allow_once']]);
  // Disabled while sending: no answer goes out.
  const busy = [];
  const el2 = m.ModernApprovalCard({ approval: m.approvalFromAway(awayItem('waiting')), disabled: true, onResolve: (...a) => busy.push(a) });
  const b2 = [];
  const walk2 = (n) => { if (!n || typeof n !== 'object') return; if (Array.isArray(n)) return n.forEach(walk2); if (n.type === 'button') b2.push(n); walk2(n.props?.children); };
  walk2(el2);
  b2.forEach((b) => b.props.onClick());
  assert.equal(busy.length, 0);
});

test('the away list labels a still-open card and renders it', () => {
  assert.deepEqual(m.itemLabel({ kind: 'approval', status: 'waiting' }), { label: 'Still open', tone: 'sky' });
  assert.deepEqual(m.itemLabel({ kind: 'approval', status: 'pending' }), { label: 'Needs approval', tone: 'amber' });
  const data = { since: '2026-10-22T00:00:00Z', waiting: [awayItem('waiting')], finished: [], failed: [], unseen: { total: 1, waiting: 1, finished: 0, failed: 0 } };
  const html = m.away({ data, loading: false, error: null, onRefresh() {}, onOpen() {}, onAnswer() {}, onResolve() {} });
  assert.match(html, /Still open/);
  assert.match(html, /Allow and continue/);
});

test('after a late Allow the person hears the bot picks it back up', () => {
  assert.equal(m.lateAnswerText({ resumed: 'new_turn', status: 'allowed_once' }, 'Architect'), 'Allowed. Architect is picking the work back up.');
  assert.equal(m.lateAnswerText({ resumed: false, reason: 'the bot no longer exists' }, null), "Allowed, but The bot couldn't continue: the bot no longer exists.");
  assert.equal(m.lateAnswerText({ resolved: true, status: 'allowed_once' }, 'Architect'), null);
  assert.equal(m.lateAnswerText(null, 'Architect'), null);
});

// Shaped like owner_view in src/server/zombie_routes.rs.
const ownerRow = (over = {}) => ({ enabled: true, kwant_id: null, daily_cap: 3, owner: { id: 'k1', name: 'Architect' }, used_today: 1, ...over });

test('owner summary covers on, off, no lead, a 0 cap and a used-up cap', () => {
  assert.equal(m.ownerSummary(ownerRow()), 'Architect prepares drafts and next steps, up to 3 times a day (1 used today, UTC). Nothing is sent without you.');
  assert.match(m.ownerSummary(ownerRow({ enabled: false })), /^Off\. Reviews and deadlines still notify you/);
  assert.match(m.ownerSummary(ownerRow({ owner: null })), /no lead yet/);
  assert.match(m.ownerSummary(ownerRow({ daily_cap: 0 })), /daily limit is 0/);
  assert.match(m.ownerSummary(ownerRow({ daily_cap: 1, used_today: 4 })), /up to 1 time a day \(1 used today/);
});

test('a change keeps the other fields for the PUT body', () => {
  assert.deepEqual(m.ownerChange(ownerRow({ kwant_id: 'k2' }), { enabled: false }), { enabled: false, kwant_id: 'k2', daily_cap: 3 });
  assert.deepEqual(m.ownerChange(ownerRow(), { kwant_id: 'k2' }), { enabled: true, kwant_id: 'k2', daily_cap: 3 });
});

test('the owner setting shows the toggle, the lead by name, the bots and the cap', () => {
  const bots = [{ id: 'k1', name: 'Architect' }, { id: 'k2', name: 'Writer' }];
  const html = m.owner({ owner: ownerRow(), bots, error: null, busy: false, onChange() {} });
  assert.match(html, /Who prepares drafts/);
  assert.match(html, /<input type="checkbox"[^>]*checked=""/);
  assert.match(html, /The space lead \(Architect\)/);
  assert.match(html, />Writer</);
  assert.match(html, /up to 3 times a day/);
  // Off: the bot picker is disabled.
  const off = m.owner({ owner: ownerRow({ enabled: false }), bots, error: null, busy: false, onChange() {} });
  assert.match(off, /<select[^>]*disabled=""/);
  // Loading, load error and save error.
  assert.match(m.owner({ owner: null, bots: [], error: null, busy: false, onChange() {} }), /Loading/);
  assert.match(m.owner({ owner: null, bots: [], error: 'HTTP 500', busy: false, onChange() {} }), /Couldn’t load this setting: HTTP 500/);
  assert.match(m.owner({ owner: ownerRow(), bots, error: 'the owner must be a bot in this space', busy: false, onChange() {} }), /Couldn’t save: the owner must be a bot/);
});

test('the toggle and picker send the whole setting', () => {
  // Render through a wrapper that keeps the element tree, then call its handlers.
  let tree = null;
  const sent = [];
  const Capture = (props) => (tree = m.LooseOwnerView(props));
  m.renderToStaticMarkup(m.React.createElement(Capture, { owner: ownerRow({ kwant_id: 'k2' }), bots: [{ id: 'k2', name: 'Writer' }], error: null, busy: false, onChange: (b) => sent.push(b) }));
  const found = [];
  const walk = (n) => { if (!n || typeof n !== 'object') return; if (Array.isArray(n)) return n.forEach(walk); if (n.type === 'input' || n.type === 'select') found.push(n); walk(n.props?.children); };
  walk(tree);
  const [box, select] = found;
  box.props.onChange({ target: { checked: false } });
  select.props.onChange({ target: { value: '' } });
  select.props.onChange({ target: { value: 'k2' } });
  assert.deepEqual(sent, [
    { enabled: false, kwant_id: 'k2', daily_cap: 3 },
    { enabled: true, kwant_id: null, daily_cap: 3 },
    { enabled: true, kwant_id: 'k2', daily_cap: 3 },
  ]);
  // Busy: both controls are disabled while saving.
  m.renderToStaticMarkup(m.React.createElement(Capture, { owner: ownerRow(), bots: [], error: null, busy: true, onChange() {} }));
  found.length = 0;
  walk(tree);
  assert.ok(found.every((n) => n.props.disabled === true));
});

test('the Loose ends page shows the owner slot only once the person is confirmed', () => {
  const slot = m.React.createElement('div', { 'data-owner-slot': '1' }, 'OWNER');
  const base = { cases: [], loading: false, running: false, error: null, tab: 'open', sources: null, onTab() {}, onRefresh() {}, onScan() {}, onAction() {}, owner: slot };
  assert.match(m.loose(base), /data-owner-slot/);
  assert.doesNotMatch(m.loose({ ...base, needsToken: true }), /data-owner-slot/);
});
