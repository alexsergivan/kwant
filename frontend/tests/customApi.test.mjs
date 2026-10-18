import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/engines';
  export { accountState, engineOfModel, readyCustomAccounts, accountUsable, CUSTOM_PRESETS } from './src/profiles';
  export { EnginesPanel } from './src/components/EnginesPanel';
  export { CustomServerForm } from './src/components/EngineAccounts';
  export { ModelPicker } from './src/components/ModelPicker';
  export { mcpSupportFor } from './src/mcp';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const h = m.createElement;

// Shaped like GET /api/engines for custom_api (f053aac): never installed, ready through accounts.
const customEngine = (over = {}) => ({
  id: 'custom_api', name: 'Custom API', kind: 'api', status: 'not_installed',
  detail: 'Runs on this space’s Custom API accounts: add one under Space settings → Accounts',
  version: null, models: [], checked_at: '2026-10-18T09:00:00Z', enabled: true, ready: false, uses: null, ...over,
});
const server = (over = {}) => ({
  protocol: 'openai-chat', base_url: 'http://localhost:23333/api/openai/v1', model: null, status: 'ready',
  detail: 'Connected · 2 models', checked_at: '2026-10-18T09:00:00Z', supports_tools: true,
  models: [{ id: 'gpt-4.1', label: 'gpt-4.1' }, { id: 'claude-sonnet-4', label: 'claude-sonnet-4' }], ...over,
});
const account = (over = {}, custom = {}) => ({
  id: 'prf_copilot', engine: 'custom', name: 'Copilot', kind: 'custom_api', enabled: true, is_default: false, priority: 100,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null,
  custom: server(custom), ...over,
});
const copilot = account();
const claudeFmt = account(
  { id: 'prf_anth', name: 'Copilot (Anthropic)' },
  { protocol: 'anthropic-messages', base_url: 'http://localhost:23333/api/anthropic/v1', supports_tools: false, models: [{ id: 'claude-sonnet-4', label: 'claude-sonnet-4' }], detail: 'Connected · 1 model' },
);
const down = account({ id: 'prf_down', name: 'Laptop', signed_in: false }, { status: 'unreachable', detail: 'Couldn’t reach http://localhost:9 (connection refused)', models: [] });
const badKey = account({ id: 'prf_bad', name: 'OpenRouter', has_key: true, signed_in: false }, { status: 'key_rejected', detail: 'The server refused the key (401 Unauthorized)', models: [] });

const api = (engines) => ({ engines, error: null, rechecking: false, recheck: async () => {}, reload: async () => {}, setEnabled: async () => {}, apply: () => {} });
const panel = (engines, profiles) =>
  m.renderToStaticMarkup(h(m.EnginesPanel, { api: api(engines), accounts: { profiles, error: null, reload: async () => {} }, now: Date.parse('2026-10-18T09:01:00Z') }));

test('custom pins parse and encode as custom:<model>', () => {
  assert.deepEqual(m.parsePick('custom:gpt-4.1'), { family: 'custom', model: 'gpt-4.1' });
  assert.deepEqual(m.parsePick('custom/gpt-4.1'), { family: 'custom', model: 'gpt-4.1' });
  assert.deepEqual(m.parsePick('custom'), { family: 'custom', model: null });
  assert.equal(m.encodePick('custom', 'gpt-4.1'), 'custom:gpt-4.1');
  assert.equal(m.encodePick('custom', null), 'custom');
  assert.equal(m.engineOfModel('custom:gpt-4.1'), 'custom');
  // A model id that merely starts with "custom" stays where it was.
  assert.notEqual(m.engineOfModel('customer-model'), 'custom');
});

test('each server status reads plainly, with the server detail verbatim', () => {
  assert.deepEqual([m.accountState(copilot).label, m.accountState(copilot).tone], ['Ready', 'ok']);
  assert.equal(m.accountState(copilot).detail, 'Connected · 2 models');
  assert.equal(m.accountState(down).label, 'Can’t connect');
  assert.equal(m.accountState(down).detail, 'Couldn’t reach http://localhost:9 (connection refused)');
  assert.equal(m.accountState(badKey).label, 'Key rejected');
  assert.equal(m.accountState(account({}, { status: 'error', detail: 'HTTP 500' })).label, 'Error');
  assert.equal(m.accountState(account({}, { status: null, detail: null })).label, 'Not checked');
  assert.ok(m.accountUsable(copilot));
  assert.ok(!m.accountUsable(down));
  assert.deepEqual(m.readyCustomAccounts([down, copilot, badKey, claudeFmt]).map((p) => p.id).sort(), ['prf_anth', 'prf_copilot']);
});

test('the Custom API card lists several servers with status, checks and the no-tools badge', () => {
  const html = panel([customEngine({ ready: true })], [copilot, claudeFmt, down, badKey]);
  assert.match(html, /data-engine="custom_api"/);
  assert.match(html, /Add a custom server/);
  assert.match(html, /Connected · 2 models/);
  assert.match(html, /Couldn’t reach http:\/\/localhost:9 \(connection refused\)/);
  assert.match(html, /The server refused the key \(401 Unauthorized\)/);
  assert.match(html, /Key rejected/);
  assert.match(html, /Can’t connect/);
  assert.equal((html.match(/Check again/g) ?? []).length, 4);
  // Only the Anthropic-format server says it has no tools.
  assert.equal((html.match(/data-no-tools/g) ?? []).length, 1);
  assert.match(html, /No tools \(Anthropic format\)/);
  assert.match(html, /data-custom-url="true"[^>]*>http:\/\/localhost:23333\/api\/anthropic\/v1</);
});

test('without servers the card invites one, naming Agent Maestro', () => {
  const html = panel([customEngine()], []);
  assert.match(html, /No servers yet\./);
  assert.match(html, /Agent Maestro \(GitHub Copilot\)/);
});

test('the form offers both formats, the Agent Maestro preset and a password key field', () => {
  const html = m.renderToStaticMarkup(h(m.CustomServerForm, { onCancel: () => {}, onSaved: () => {} }));
  assert.match(html, /data-custom-form="new"/);
  assert.match(html, /Use Agent Maestro/);
  assert.match(html, /OpenAI-compatible/);
  assert.match(html, /Anthropic-compatible/);
  assert.match(html, /type="password"/);
  assert.equal(m.CUSTOM_PRESETS.find((p) => p.id === 'maestro').urls['openai-chat'], 'http://localhost:23333/api/openai/v1');
  // Editing a ready server picks its model from the server's own list.
  const edit = m.renderToStaticMarkup(h(m.CustomServerForm, { account: copilot, onCancel: () => {}, onSaved: () => {} }));
  assert.match(edit, /data-custom-form="edit"/);
  assert.match(edit, /<option value="gpt-4.1">gpt-4.1<\/option>/);
  assert.doesNotMatch(edit, /type="password"/);
});

test('the bot editor offers custom models grouped and labelled by account', () => {
  const html = m.renderToStaticMarkup(h(m.ModelPicker, {
    value: 'custom:claude-sonnet-4', onChange: () => {}, engines: [customEngine({ ready: true })],
    profiles: [copilot, claudeFmt, down], profileId: 'prf_anth', onPickAccount: () => {},
  }));
  assert.match(html, /<optgroup label="Copilot">/);
  assert.match(html, /<optgroup label="Copilot \(Anthropic\)">/);
  assert.doesNotMatch(html, /label="Laptop"/);
  assert.match(html, /gpt-4.1 · Copilot</);
  // The pinned account's copy of a shared model is the one selected.
  assert.match(html, /value="prf_anth\nclaude-sonnet-4" selected=""/);
});

test('a custom pin never blames the Codex or Claude CLI for tools', () => {
  const keys = { anthropic: true, openai: false, xai: false };
  for (const pin of ['custom:gpt-4.1', 'custom:claude-sonnet-4', 'custom']) {
    const s = m.mcpSupportFor(pin, keys);
    assert.ok(s.supported, pin);
    assert.match(s.note, /OpenAI-compatible server/);
  }
});

test('ready accounts keep Custom API on offer while the engine list is still the old one', () => {
  const opts = m.familyOptions([customEngine({ ready: false })], [{ name: 'Copilot', models: [{ id: 'gpt-4.1', label: 'gpt-4.1' }] }]);
  assert.deepEqual(opts.map((o) => [o.id, o.via]), [['custom', ['Copilot']]]);
  assert.deepEqual(m.familyOptions([customEngine({ ready: false })]), []);
});
