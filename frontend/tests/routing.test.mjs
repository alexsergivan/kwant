import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/routing';`);

const NOW = Date.parse('2026-10-17T19:01:00Z');
const engine = (id, over = {}) => ({
  id,
  name: { claude: 'Claude Code CLI', codex: 'Codex CLI', anthropic_api: 'Anthropic API key', ollama: 'Ollama' }[id],
  kind: id.endsWith('_api') ? 'api' : id === 'ollama' ? 'local' : 'cli',
  status: 'ready', detail: null, version: null, checked_at: '2026-10-17T19:00:00Z', enabled: true, ready: true, uses: null,
  models: id === 'claude' ? [{ id: 'sonnet', label: 'Claude Sonnet', default: true }] : [],
  ...over,
});
const profile = (over) => ({
  id: 'prf_x', engine: 'claude', name: 'Work', kind: 'cli_home', enabled: true, is_default: false, priority: 100,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null, ...over,
});
// Oleks's setup: This machine off and on Bedrock, Personal default and signed in.
const machine = profile({ id: 'prf_system_claude', name: 'This machine', kind: 'system', enabled: false, priority: 0, account_label: 'Amazon Bedrock' });
const personal = profile({ id: 'prf_personal', name: 'Personal', is_default: true, priority: 100, account_label: 'oleks@example.com' });
const work = profile({ id: 'prf_work', name: 'Work', priority: 50, account_label: 'oleks@work.io' });

test('turns try the default first, then by priority, at most 3, skipping off accounts', () => {
  const extra = profile({ id: 'prf_z', name: 'Zed', priority: 200 });
  const names = m.tryOrder([machine, personal, work, extra, profile({ id: 'p4', name: 'Four', priority: 300 })], 'claude', NOW).map((p) => p.name);
  assert.deepEqual(names, ['Personal', 'Work', 'Zed']);
  assert.deepEqual(m.tryOrder([machine], 'claude', NOW), []);
});

test('an unusable default is skipped when a backup can run', () => {
  const out = { ...personal, limited_until: '2026-10-17T21:00:00Z' };
  assert.deepEqual(m.tryOrder([out, work], 'claude', NOW).map((p) => p.name), ['Work']);
  // Nothing else to try: the default stays, so the turn reports why.
  assert.deepEqual(m.tryOrder([out], 'claude', NOW).map((p) => p.name), ['Personal']);
});

test("Oleks: Claude runs on Claude Code CLI through Personal, with the This machine fallback spelled out", () => {
  const [row, ...rest] = m.routeSummary([engine('claude'), engine('codex', { ready: false, status: 'signed_out' })], [machine, personal], NOW);
  assert.equal(rest.length, 0);
  assert.equal(row.label, 'Claude');
  assert.equal(row.engine, 'Claude Code CLI');
  assert.deepEqual(row.account, { name: 'Personal', detail: 'oleks@example.com', otherProvider: false });
  assert.deepEqual(row.then, []);
  assert.equal(row.defaultModel, 'Claude Sonnet');
  assert.equal(row.fallback, 'If you switch off every Claude account, bots in this space use This machine.');
});

test('backups are listed in order', () => {
  const [row] = m.routeSummary([engine('claude')], [machine, personal, work], NOW);
  assert.equal(row.account.name, 'Personal');
  assert.deepEqual(row.then, ['Work']);
});

test('an account on another provider is marked as such', () => {
  const bedrock = { ...personal, account_label: 'Amazon Bedrock' };
  const [row] = m.routeSummary([engine('claude')], [machine, bedrock], NOW);
  assert.deepEqual(row.account, { name: 'Personal', detail: 'via Amazon Bedrock', otherProvider: true });
});

test('with every account off, turns use This machine', () => {
  const [row] = m.routeSummary([engine('claude')], [machine, { ...personal, enabled: false }], NOW);
  assert.equal(row.account.name, 'This machine');
  assert.equal(row.account.otherProvider, true);
  assert.equal(row.engine, 'Claude Code CLI');
});

test('nothing ready means no rows; an API key from Settings shows as such', () => {
  assert.deepEqual(m.routeSummary([engine('claude', { ready: false })], [machine, personal], NOW), []);
  const [row] = m.routeSummary([engine('anthropic_api')], [], NOW);
  assert.equal(row.engine, 'Anthropic API key');
  assert.equal(row.account.name, 'API key in Settings');
});

// Rigel's edge run: the only switched-on accounts are a signed-out Work and an unchecked one.
test('when no account is ready, the summary names the one the server still tries, with why', () => {
  const out = { ...work, signed_in: false, is_default: true };
  const [row] = m.routeSummary([engine('claude', { uses: { profile_id: 'prf_personal', label: 'Personal' } })], [machine, out, { ...personal, enabled: false }], NOW);
  assert.equal(row.account.name, 'Work');
  assert.match(row.note, /needs sign-in/);
  assert.deepEqual(row.then, []);
  const limited = { ...personal, limited_until: '2026-10-17T21:00:00Z' };
  const [r2] = m.routeSummary([engine('claude')], [machine, limited], NOW);
  assert.equal(r2.account.name, 'Personal');
  assert.match(r2.note, /out of usage/);
  // A ready account carries no note.
  assert.equal(m.routeSummary([engine('claude')], [machine, personal], NOW)[0].note, null);
});

test('an account whose sign-in is not checked yet is tried, as on the server', () => {
  const unchecked = profile({ id: 'prf_new', name: 'New', priority: 10, signed_in: null });
  const out = { ...work, signed_in: false };
  assert.deepEqual(m.tryOrder([machine, personal, unchecked, out], 'claude', NOW).map((p) => p.name), ['Personal', 'New']);
  const [row] = m.routeSummary([engine('claude')], [machine, { ...unchecked, is_default: true }, out], NOW);
  assert.equal(row.account.name, 'New');
  assert.match(row.note, /hasn’t been checked/);
  // A refused turn's "auth" error alone doesn't take it out; only a signed-out check does.
  assert.deepEqual(m.tryOrder([{ ...personal, last_error: 'auth' }, work], 'claude', NOW).map((p) => p.name), ['Personal', 'Work']);
});

test('equal priorities keep the API order (created_at, then id), not the name', () => {
  // The API lists Zed before Alpha: Zed was added first.
  const zed = profile({ id: 'prf_a1', name: 'Zed', priority: 100 });
  const alpha = profile({ id: 'prf_b2', name: 'Alpha', priority: 100 });
  assert.deepEqual(m.tryOrder([machine, zed, alpha], 'claude', NOW).map((p) => p.name), ['Zed', 'Alpha']);
  assert.deepEqual(m.tryOrder([machine, alpha, zed], 'claude', NOW).map((p) => p.name), ['Alpha', 'Zed']);
});

test('an API-key account on an engine that is off drops out of the route', () => {
  const key = profile({ id: 'prf_key', name: 'Key', kind: 'api_key', has_key: true, priority: 10 });
  const [row] = m.routeSummary([engine('claude')], [machine, key, personal], NOW);
  assert.equal(row.account.name, 'Personal');
  assert.deepEqual(row.then, []);
});
