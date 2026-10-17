import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/engines';
  export { claudeNeedsSignIn } from './src/claudeLogin';
  export { accountsForCard, cardOf, newAccountFor, accountUsable } from './src/profiles';
  export { EnginesPanel } from './src/components/EnginesPanel';
  export { ModelPicker } from './src/components/ModelPicker';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const h = m.createElement;

// Shaped like EngineReport with Lin's `ready` and `uses` (36b0810).
const engine = (id, over = {}) => ({
  id,
  name: { claude: 'Claude Code CLI', codex: 'Codex CLI', anthropic_api: 'Anthropic API key', ollama: 'Ollama' }[id],
  kind: id.endsWith('_api') ? 'api' : id === 'ollama' ? 'local' : 'cli',
  status: 'ready',
  detail: null,
  version: '2.1.296',
  models: [],
  checked_at: '2026-10-17T19:00:00Z',
  enabled: true,
  ready: true,
  uses: null,
  ...over,
});
// Shaped like GET /api/profiles.
const profile = (over) => ({
  id: 'prf_x', engine: 'claude', name: 'Work', kind: 'cli_home', enabled: true, is_default: false, priority: 100,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null, ...over,
});
const machine = profile({ id: 'prf_system_claude', name: 'This machine', kind: 'system', signed_in: null, priority: 0 });
const personal = profile({ id: 'prf_personal', name: 'Personal', is_default: true, priority: 10 });

const api = (engines) => ({ engines, error: null, rechecking: false, recheck: async () => {}, reload: async () => {}, setEnabled: async () => {}, apply: () => {} });
const accounts = (profiles, error = null) => ({ profiles, error, reload: async () => {} });
const NOW = Date.parse('2026-10-17T19:01:00Z');
const panel = (engines, profiles = [machine, personal], error = null) =>
  m.renderToStaticMarkup(h(m.EnginesPanel, { api: api(engines), accounts: accounts(profiles, error), now: NOW }));
const card = (html, id) => html.split(`data-engine="${id}"`)[1].split('</li></ul></div></li>')[0];
const summary = (html, id) => card(html, id).match(/data-summary="true"[^>]*>(.*?)<\/p>/)[1].replace(/<[^>]+>/g, '');

// Oleks's setup: Claude on, the host login signed out, Personal signed in and Default.
const oleksOn = engine('claude', { status: 'signed_out', detail: 'Not signed in: run `claude auth login`', uses: { profile_id: 'prf_personal', label: 'Personal' } });

test('ready comes from `ready`, not the host login or the model list', () => {
  assert.ok(m.isUsable(oleksOn));
  assert.equal(m.statusView(oleksOn).label, 'Ready');
  assert.ok(!m.isUsable({ ...oleksOn, enabled: false }));
  assert.ok(!m.isUsable(engine('claude', { ready: false })));
  // An older server without `ready` still reads the host check.
  assert.ok(m.isUsable(engine('claude', { ready: undefined })));
  assert.ok(!m.isUsable(engine('claude', { ready: undefined, status: 'signed_out' })));
  // Claude is fine through Personal, so nothing nags to sign in.
  assert.ok(!m.claudeNeedsSignIn([oleksOn]));
  assert.ok(m.claudeNeedsSignIn([{ ...oleksOn, ready: false, uses: null }]));
});

test('Oleks: one Claude card says kwants use Personal, with This machine first and signed out', () => {
  const html = panel([oleksOn]);
  assert.match(html, /Engines &amp; accounts/);
  // The summary replaces the headline: Claude runs on Claude Code CLI through Personal.
  assert.match(html, /What runs your kwants/);
  assert.match(html, /data-route="claude"[\s\S]*?Claude Code CLI[\s\S]*?>Personal</);
  assert.match(html, /No backup account is ready\./);
  assert.match(html, /data-engine="claude" data-status="ready"/);
  assert.equal(summary(html, 'claude'), 'Kwants use Personal');
  const names = [...card(html, 'claude').matchAll(/id="account-[^"]+-name"[^>]*>([^<]+)</g)].map((x) => x[1]);
  assert.deepEqual(names, ['This machine', 'Personal']);
  // This machine: not signed in, with the Batch 9 sign-in on its row; Personal is in use.
  const rows = card(html, 'claude').split('data-account=');
  assert.match(rows[1], /Needs sign-in/);
  assert.match(rows[1], /Sign in with Claude<\/button>/);
  assert.match(rows[2], /Default/);
  assert.match(rows[2], /In use/);
  assert.match(rows[2], /Ready/);
  assert.match(rows[2], /Sign out/);
  // Only one way to sign in to Claude's host login, and no "run claude auth login".
  assert.equal(html.split('Sign in with Claude</button>').length, 2);
  assert.doesNotMatch(html, /claude auth login/);
});

test('switched off with Personal signed in: "Off · Turn on to use Personal"', () => {
  const html = panel([engine('claude', { enabled: false, status: 'disabled', ready: false, uses: null })]);
  assert.match(html, /data-status="off"/);
  assert.equal(summary(html, 'claude'), 'Off · Turn on to use Personal');
  assert.match(html, /No engine can run right now\. Claude Code CLI is off: turn it on to use Personal\./);
  assert.match(html, /role="switch" aria-checked="false"/);
  assert.match(html, /Not checked while Claude Code CLI is off/);
  assert.doesNotMatch(html, /What runs your kwants/);
  // Nothing to sign in to while it's off.
  assert.doesNotMatch(html, /Sign in with Claude/);
});

test('nothing signed in: the card and headline name the next step', () => {
  const out = engine('claude', { status: 'signed_out', ready: false, uses: null });
  const html = panel([out], [machine, { ...personal, signed_in: false }]);
  assert.equal(summary(html, 'claude'), 'No account is signed in · Sign in to one below');
  assert.match(html, /No engine can run right now\. Sign in with Claude below, or add an account\./);
  assert.match(html, /Not signed in/);
  assert.match(html, /Sign in with Claude<\/button>/);
  // Personal offers its own sign-in.
  assert.match(card(html, 'claude').split('data-account=')[2], />Sign in<\/button>|<\/svg> Sign in<\/button>/);
});

test('an account out of usage says when it is back', () => {
  const out = engine('claude', { status: 'signed_out', ready: false, uses: null });
  const html = panel([out], [machine, { ...personal, limited_until: '2026-10-17T21:01:00Z' }]);
  assert.equal(summary(html, 'claude'), 'Personal is out of usage, back in 2 h · Sign in to another account below');
});

test('API-key accounts sit on the API engine card; an engine missing but with an account still shows', () => {
  const key = profile({ id: 'prf_key', name: 'Team key', kind: 'api_key', has_key: true, signed_in: null });
  assert.equal(m.cardOf(key), 'anthropic_api');
  assert.equal(m.cardOf(personal), 'claude');
  assert.deepEqual(m.newAccountFor('anthropic_api'), { engine: 'claude', kind: 'api_key' });
  assert.deepEqual(m.newAccountFor('claude'), { engine: 'claude', kind: 'cli_home' });
  assert.equal(m.newAccountFor('ollama'), null);
  const apiEngine = engine('anthropic_api', { status: 'not_installed', ready: true, uses: { profile_id: 'prf_key', label: 'Team key' } });
  const html = panel([oleksOn, apiEngine, engine('ollama', { status: 'not_installed', ready: false })], [machine, personal, key]);
  assert.match(html, /data-engine="anthropic_api" data-status="ready"/);
  assert.equal(summary(html, 'anthropic_api'), 'Kwants use Team key');
  assert.match(html, /Not available on this computer \(1\)/);
  // Ollama has no accounts section.
  assert.doesNotMatch(html, /Ollama accounts/);
});

test('accounts load and fail with a next step', () => {
  assert.match(m.renderToStaticMarkup(h(m.EnginesPanel, { api: api([oleksOn]), accounts: accounts(null), now: NOW })), /Loading accounts…/);
  const html = panel([oleksOn], null, 'Request failed (HTTP 500).');
  assert.match(html, /role="alert"[^>]*>.*Couldn’t load accounts: Request failed \(HTTP 500\)\./);
  assert.match(html, /Try again<\/button>/);
});

test('accounts list This machine first, then the order turns try them', () => {
  const list = m.accountsForCard(
    [profile({ id: 'b', name: 'B', priority: 5 }), personal, machine, profile({ id: 'a', name: 'A', priority: 1 }), profile({ id: 'k', kind: 'api_key' })],
    'claude',
  );
  assert.deepEqual(list.map((p) => p.name), ['This machine', 'Personal', 'A', 'B']);
  assert.ok(m.accountUsable(personal));
  assert.ok(!m.accountUsable({ ...personal, signed_in: null }), 'unknown sign-in is not ready, like the server');
  assert.ok(!m.accountUsable({ ...personal, enabled: false }));
});

const picker = (value, engines) =>
  m.renderToStaticMarkup(h(m.ModelPicker, { value, onChange: () => {}, engines, onOpenSettings: () => {}, onSignInClaude: () => {} }));

test('the kwant form takes Claude as ready through Personal, even with no model list', () => {
  const html = picker('claude', [oleksOn]);
  assert.doesNotMatch(html, /can&#x27;t run right now/);
  assert.match(html, /<option value="claude"[^>]*>Claude · Claude Code CLI<\/option>/);
  assert.deepEqual(m.pickHealth('auto', [oleksOn]), { kind: 'ok' });
  // Switched off: the hint says so instead of "No API key".
  const off = [{ ...oleksOn, enabled: false, status: 'disabled', ready: false }, engine('anthropic_api', { status: 'not_installed', ready: false, detail: 'No API key: add one in Settings' })];
  assert.deepEqual(m.pickHealth('claude', off), { kind: 'engine_unavailable', family: 'claude', reason: 'Switched off in Settings' });
});
