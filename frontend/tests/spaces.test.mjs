import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SpaceRail, NewSpaceModal } from './src/components/Spaces';
  export * from './src/spaces';
  export const rail = (props) => renderToStaticMarkup(React.createElement(SpaceRail, props));
  export const newSpace = (props) => renderToStaticMarkup(React.createElement(NewSpaceModal, props));
`);

const noop = () => {};
const space = (over = {}) => ({
  id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 6, ready_accounts: 2, unread: 0 }, ...over,
});

test('the open space rides on scoped /api calls, never on app-wide ones', () => {
  const o = 'http://kwnt.test';
  assert.equal(m.withSpace('/api/kwants', o, 'spc_b'), '/api/kwants?space=spc_b');
  assert.equal(m.withSpace('/api/channels?x=1', o, 'spc_b'), '/api/channels?x=1&space=spc_b');
  assert.equal(m.withSpace('/api/spaces', o, 'spc_b'), '/api/spaces');
  assert.equal(m.withSpace('/api/auth/status', o, 'spc_b'), '/api/auth/status');
  assert.equal(m.withSpace('/api/settings', o, 'spc_b'), '/api/settings');
  assert.equal(m.withSpace('/api/kwants?space=spc_a', o, 'spc_b'), '/api/kwants?space=spc_a');
  assert.equal(m.withSpace('https://other.test/api/kwants', o, 'spc_b'), 'https://other.test/api/kwants');
  assert.equal(m.withSpace('/api/kwants', o, null), '/api/kwants');
  assert.equal(m.eventsUrl('spc b'), '/api/events?space=spc%20b');
  assert.equal(m.eventsUrl(null), '/api/events');
});

test('addresses round-trip and the right space opens first', () => {
  assert.deepEqual(m.parseSpacePath('/s/spc_b/c/ch9'), { spaceId: 'spc_b', channelId: 'ch9' });
  assert.deepEqual(m.parseSpacePath('/s/spc_b/b/k1'), { spaceId: 'spc_b', botId: 'k1' });
  assert.deepEqual(m.parseSpacePath('/'), { spaceId: null });
  assert.equal(m.spacePath('spc_b', { channelId: 'ch9' }), '/s/spc_b/c/ch9');
  const list = [space(), space({ id: 'spc_b', name: 'B', is_default: false }), space({ id: 'spc_c', is_default: false, archived_at: 'x' })];
  assert.equal(m.pickSpace(list, 'spc_b', 'spc_default').id, 'spc_b');
  assert.equal(m.pickSpace(list, 'spc_c', null).id, 'spc_default', 'an archived space never opens');
  assert.equal(m.pickSpace(list, 'gone', 'spc_b').id, 'spc_b');
});

test('names, colours and the summary line', () => {
  assert.equal(m.spaceNameError('  ', []), 'Give the space a name.');
  assert.equal(m.spaceNameError('work', ['Work']), 'You already have a space called work.');
  assert.equal(m.spaceNameError('Home', ['Work']), null);
  assert.equal(m.firstUnusedColor([{ color: '#F59E0B' }]), '#10b981');
  assert.equal(m.firstUnusedColor([{ color: 'amber' }]), '#10b981', "the server's Default is 'amber'");
  assert.equal(m.spaceColor({ id: 'spc_default', color: 'amber' }), '#f59e0b');
  assert.equal(m.spaceIcon({ icon: 'box' }), '📦');
  assert.equal(m.spaceSummary({ bots: 1, ready_accounts: 0, unread: 0 }), '1 bot · no account yet');
  assert.equal(m.spaceSummary({ bots: 4, ready_accounts: 3, unread: 0 }), '4 bots · 3 accounts');
  assert.equal(m.spaceSummaryTitle({ bots: 4, ready_accounts: 0, unread: 0 }), '4 kwant bots · no account signed in yet');
  // QA b: the sidebar header shows about 25 characters before it cuts off
  for (const c of [{ bots: 12, ready_accounts: 0 }, { bots: 12, ready_accounts: 10 }]) assert.ok(m.spaceSummary(c).length <= 24, m.spaceSummary(c));
});

test('Laya says who she picked, from how many bots, in which space', () => {
  const meta = JSON.stringify({ routing: { kwant_name: 'Lin', candidate_count: 6, matched_terms: ['migration', 'SQLite'] }, ran_on: { account: 'Work' } });
  const r = m.parseRoutingMeta(meta);
  assert.equal(m.routingLine(r, 'Work'), "Laya picked Lin from 6 bots in Work · matched 'migration', 'SQLite'");
  assert.equal(m.routingLine({ kwant_name: 'Lin', candidate_count: 1, fallback: true }, 'B'), 'Picked by keywords (Laya is offline) · Lin from 1 bot in B');
  assert.equal(m.ranOnAccount(meta), 'Work');
  assert.equal(m.parseRoutingMeta('{"ran_on":{}}'), null);
  assert.equal(m.parseRoutingMeta('not json'), null);
});

test('the rail marks the open space and shows other spaces’ unread', () => {
  const html = m.rail({
    spaces: [space(), space({ id: 'spc_b', name: 'Side', is_default: false, counts: { bots: 1, ready_accounts: 1, unread: 3 } })],
    currentId: 'spc_default', onSwitch: noop, onCreate: noop, onOpenAppSettings: noop,
  });
  assert.match(html, /aria-current="(page|true)"/);
  assert.match(html, /Side/);
  assert.match(html, />3</);
  assert.doesNotMatch(html, /\b[Aa]gents?\b/);
});

test('a new space preselects the first unused colour and offers Copy bots as coming soon', () => {
  const html = m.newSpace({ spaces: [space()], onClose: noop, onCreated: noop });
  assert.match(html, /#10b981/);
  assert.match(html, /This machine/);
  assert.match(html, /Coming soon/);
});

test('a reload or link reopens the bot or channel in the address, and a gone one falls back to #general', () => {
  const bots = [{ id: 'k1' }, { id: 'k2' }];
  const channels = [{ id: 'ch0' }, { id: 'ch1' }, { id: 'ch2' }];
  const pick = (t) => {
    const r = m.pickInitialChat(t, bots, channels, 'ch1');
    return r && (r.kind === 'bot' ? `bot:${r.bot.id}` : `channel:${r.channel.id}`);
  };
  // The bug: a bot target lost to #general because #general exists.
  assert.equal(pick({ botId: 'k2' }), 'bot:k2');
  assert.equal(pick({ channelId: 'ch2' }), 'channel:ch2');
  assert.equal(pick({ botId: 'gone' }), 'channel:ch1');
  assert.equal(pick({ channelId: 'gone' }), 'channel:ch1');
  assert.equal(pick(null), 'channel:ch1');
  assert.equal(pick({}), 'channel:ch1');
  assert.equal(m.pickInitialChat({ botId: 'gone' }, bots, channels, 'missing')?.channel.id, 'ch0');
  assert.equal(m.pickInitialChat({ botId: 'gone' }, bots, [], 'ch1')?.bot.id, 'k1');
  assert.equal(m.pickInitialChat({ botId: 'gone' }, [], [], 'ch1'), null);
});
