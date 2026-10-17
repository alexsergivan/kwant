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
  assert.equal(row.fallback, 'If you switch off every Claude account, kwants use This machine.');
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
