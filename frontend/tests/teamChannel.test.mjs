import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const { isTeamChannel, teamChannelOf, teamOfChannel } = await loadBundled(`export * from './src/teamChannel';`);

const startup = { id: 't1', name: 'startup' };
const general = { id: 'c0', name: 'general' };
const oldChat = { id: 'c1', name: 'team-Startup' };

test('a chat left under a differently-cased name is still the team chat', () => {
  // What Oleks saw: @startup in Teams, and its chat listed again as #team-Startup.
  assert.equal(isTeamChannel(oldChat, startup), true);
  assert.equal(teamChannelOf([general, oldChat], startup), oldChat);
  assert.equal(teamOfChannel([startup], oldChat), startup);
  assert.equal(isTeamChannel(general, startup), false);
});

test('an exact name wins over one that differs only in case', () => {
  const exact = { id: 'c2', name: 'team-startup' };
  assert.equal(teamChannelOf([oldChat, exact], startup), exact);
  assert.equal(teamOfChannel([{ id: 't2', name: 'Startup' }, startup], exact), startup);
});

test('an id match counts, other teams do not', () => {
  assert.equal(teamChannelOf([{ id: 't1', name: 'whatever' }], startup)?.id, 't1');
  assert.equal(teamChannelOf([{ id: 'c3', name: 'team-startups' }], startup), undefined);
  assert.equal(teamOfChannel([startup], { id: 'c4', name: 'team-other' }), undefined);
});

test('the sidebar hides every team chat and nothing else', async () => {
  const src = (await import('node:fs')).readFileSync(new URL('../src/components/Sidebar.tsx', import.meta.url), 'utf8');
  assert.match(src, /teamChannelOf\(channels, t\)\?\.id === c\.id/);
  const channels = [general, oldChat, { id: 'c5', name: 'team-ghost' }];
  const shown = channels.filter((c) => ![startup].some((t) => teamChannelOf(channels, t)?.id === c.id));
  assert.deepEqual(shown.map((c) => c.name), ['general', 'team-ghost']);
});

test("saving Manage Team keeps a hired team's name as it is", async () => {
  const { teamNameToSave } = await loadBundled(`export * from './src/teamChannel';`);
  // The bug: any save turned "Startup" into "startup", and the team lost its chat.
  assert.equal(teamNameToSave('Startup', 'Startup'), 'Startup');
  assert.equal(teamNameToSave(' Startup ', 'Startup'), 'Startup');
  // A name the person actually typed is still made into a handle.
  assert.equal(teamNameToSave('Launch Crew', 'Startup'), 'launch-crew');
  const src = (await import('node:fs')).readFileSync(new URL('../src/components/ManageTeamModal.tsx', import.meta.url), 'utf8');
  assert.match(src, /teamNameToSave\(name, team\.name\)/);
});
