import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const c = await load('../src/channels.ts');

test('a typed name becomes a channel handle', () => {
  assert.equal(c.channelSlug('  #Launch Plan 2027! '), 'launch-plan-2027');
  assert.equal(c.channelSlug('###'), '');
  assert.equal(c.channelSlug('ops_team'), 'ops_team');
});

test('names that would clash or look like a team chat are refused with a reason', () => {
  const existing = [{ name: 'general' }, { name: 'launch' }];
  assert.equal(c.channelNameError('', existing), 'Give the channel a name.');
  assert.match(c.channelNameError('launch', existing), /#launch already exists/);
  assert.match(c.channelNameError('team-ops', existing), /kept for team chats/);
  assert.equal(c.channelNameError('marketing', existing), null);
});

test('the request: first bot picked leads, the lead is a member, Laya routing on', () => {
  assert.deepEqual(c.newChannelBody('launch', '  ', ['b', 'a', 'b']), {
    name: 'launch',
    lead_kwant_id: 'b',
    member_kwants: ['b', 'a'],
    routing_mode: 'auto',
  });
  assert.equal(c.newChannelBody('launch', ' For the launch ', ['a']).description, 'For the launch');
});

test('the Laya pill follows the routing rule, not the channel name', () => {
  const two = { name: 'launch', lead_kwant_id: 'a', member_kwants: '["a","b"]', routing_mode: 'auto' };
  assert.equal(c.layaRoutes(two, 3), true, 'members arrive as a JSON string');
  assert.equal(c.layaRoutes({ ...two, member_kwants: ['b'] }, 3), true, 'an older row that leaves the lead out still counts it');
  assert.equal(c.layaRoutes({ ...two, member_kwants: '["a"]' }, 3), false, 'one bot: nobody to choose between');
  assert.equal(c.layaRoutes({ ...two, routing_mode: 'lead' }, 3), false);
  assert.equal(c.layaRoutes({ name: 'general', lead_kwant_id: 'a', member_kwants: '[]', routing_mode: 'auto' }, 2), true);
  assert.equal(c.layaRoutes({ name: 'general', lead_kwant_id: 'a', member_kwants: '[]', routing_mode: 'auto' }, 1), false);
});

test('the picker explains what one bot versus several means', () => {
  assert.match(c.routingHint(0), /at least one/);
  assert.match(c.routingHint(1), /Add another and Laya picks/);
  assert.match(c.routingHint(3), /Laya picks/);
});

test('the sidebar offers New channel and the app creates it through /api/channels', async () => {
  const sidebar = await readFile(new URL('../src/components/Sidebar.tsx', import.meta.url), 'utf8');
  const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(sidebar, /New channel/);
  assert.match(sidebar, /layaRoutes\(c, kwants\.length\)/);
  assert.doesNotMatch(sidebar, /c\.name === 'general' && \(/, 'the pill is no longer #general only');
  assert.match(app, /<NewChannelModal/);
  assert.match(app, /fetch\('\/api\/channels', \{\s*method: 'POST'/);
});
