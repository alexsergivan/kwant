import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`export * from './src/skills';`);

const skill = (over = {}) => ({
  id: 'a', kwant_id: 'cto', name: 'weekly-report', description: 'Friday report', body: '1. Pull\n2. Write',
  status: 'pending', source: 'agent', proposed_by: 'cto', replaces_id: null,
  created_at: '2026-10-17T10:00:00Z', updated_at: '2026-10-17T10:00:00Z', reviewed_at: null, ...over,
});

test('drafts follow the server rules', () => {
  const ok = { name: 'weekly-report', description: 'x', body: 'steps' };
  assert.equal(s.skillDraftError(ok), null);
  assert.match(s.skillDraftError({ ...ok, name: '' }), /name/);
  assert.match(s.skillDraftError({ ...ok, name: 'Weekly Report' }), /lowercase/);
  assert.match(s.skillDraftError({ ...ok, name: '-x' }), /lowercase/);
  assert.match(s.skillDraftError({ ...ok, name: 'a'.repeat(49) }), /48/);
  assert.match(s.skillDraftError({ ...ok, description: 'd'.repeat(201) }), /200/);
  assert.match(s.skillDraftError({ ...ok, body: '   ' }), /steps/);
  assert.match(s.skillDraftError({ ...ok, body: 'b'.repeat(8001) }), /8000/);
});

test('names are slugified on blur', () => {
  assert.equal(s.slugifySkillName('  Weekly Report! v2 '), 'weekly-report-v2');
  assert.equal(s.slugifySkillName('---'), '');
  assert.equal(s.slugifySkillName('a'.repeat(47) + ' b').length <= 48, true);
  assert.equal(s.slugifySkillName('a'.repeat(47) + ' b').endsWith('-'), false);
});

test('skill_updated upserts, filters by agent and handles deletes', () => {
  let list = [];
  list = s.applySkillEvent(list, skill(), 'cto');
  assert.equal(list.length, 1);
  list = s.applySkillEvent(list, skill({ id: 'b', kwant_id: null, status: 'enabled' }), 'cto');
  assert.deepEqual(list.map((x) => x.id), ['b', 'a'], 'shared skills count, newest first');
  const same = s.applySkillEvent(list, skill({ id: 'c', kwant_id: 'other' }), 'cto');
  assert.equal(same, list, 'another agent’s skill leaves the list alone');
  list = s.applySkillEvent(list, skill({ status: 'enabled' }), 'cto');
  assert.equal(list.find((x) => x.id === 'a').status, 'enabled', 'updated in place');
  list = s.applySkillEvent(list, { id: 'a', kwant_id: 'cto', deleted: true }, 'cto');
  assert.deepEqual(list.map((x) => x.id), ['b']);
  assert.equal(s.applySkillEvent(list, { id: 'zz', kwant_id: null, deleted: true }), list);
  // Moved to another agent: it leaves this list
  list = s.applySkillEvent(list, skill({ id: 'b', kwant_id: 'other' }), 'cto');
  assert.equal(list.length, 0);
});

test('groups: pending, then in use (enabled before disabled), then history', () => {
  const g = s.groupSkills([
    skill({ id: '1', status: 'pending' }),
    skill({ id: '2', status: 'disabled', name: 'a-off' }),
    skill({ id: '3', status: 'enabled', name: 'z-on' }),
    skill({ id: '4', status: 'enabled', name: 'b-on' }),
    skill({ id: '5', status: 'rejected' }),
    skill({ id: '6', status: 'superseded' }),
  ]);
  assert.deepEqual(g.pending.map((x) => x.id), ['1']);
  assert.deepEqual(g.active.map((x) => x.id), ['4', '3', '2']);
  assert.deepEqual(g.history.map((x) => x.id), ['5', '6']);
});

test('pending counts per agent', () => {
  const c = s.pendingByKwant([skill(), skill({ id: 'b' }), skill({ id: 'c', kwant_id: 'ellis' }), skill({ id: 'd', status: 'enabled' }), skill({ id: 'e', kwant_id: null })]);
  assert.deepEqual(c, { cto: 2, ellis: 1 });
});

test('the API sends the documented requests', async () => {
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    calls.push([init.method, url, init.body ?? null]);
    return init.method === 'DELETE' ? new Response(null, { status: 204 }) : new Response('{}', { status: 200 });
  };
  await s.skillsApi.list({ kwant_id: 'cto', status: 'pending' });
  await s.skillsApi.approve('id/1', { body: 'x' });
  await s.skillsApi.patch('id1', { enabled: false });
  assert.equal(await s.skillsApi.remove('id1'), undefined, '204 has no body');
  assert.deepEqual(calls, [
    ['GET', '/api/skills?kwant_id=cto&status=pending', null],
    ['POST', '/api/skills/id%2F1/approve', '{"body":"x"}'],
    ['PATCH', '/api/skills/id1', '{"enabled":false}'],
    ['DELETE', '/api/skills/id1', null],
  ]);
});
