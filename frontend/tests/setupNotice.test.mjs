import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { SetupNeededCard, parseSetupMeta, ollamaDownDetail } from './src/components/SetupNeededCard';
  export { canRunNow } from './src/engines';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const card = (props) =>
  m.renderToStaticMarkup(
    m.createElement(m.SetupNeededCard, { botName: 'Architect', spaceName: 'Default', canRun: false, onOpenAccounts: () => {}, ...props }),
  );

test('only setup_needed meta becomes a setup card', () => {
  // What src/drivers/engines.rs no_engine_notice saves on the message.
  assert.deepEqual(m.parseSetupMeta({ kind: 'setup_needed', reason: 'no_account' }), { reason: 'no_account' });
  assert.deepEqual(m.parseSetupMeta({ kind: 'setup_needed', reason: 'host_off' }), { reason: 'host_off' });
  assert.deepEqual(m.parseSetupMeta({ kind: 'setup_needed' }), { reason: 'no_account' });
  // The messages API sends meta as a JSON string.
  assert.deepEqual(m.parseSetupMeta('{"kind":"setup_needed","reason":"no_account"}'), { reason: 'no_account' });
  assert.equal(m.parseSetupMeta({ kind: 'ask_user' }), null);
  assert.equal(m.parseSetupMeta(null), null);
  assert.equal(m.parseSetupMeta('setup_needed'), null);
  assert.equal(m.parseSetupMeta('{"kind":"ask_user"}'), null);
});

test('the card offers Add an account, and says where it is', () => {
  const html = card({ meta: { reason: 'no_account' } });
  assert.match(html, /Architect can(’|&#x27;|')t answer yet/);
  assert.match(html, /No account can run bots in Default yet/);
  assert.match(html, /<button[^>]*>.*Add an account<\/button>/s);
  assert.doesNotMatch(html, /Settings → Engines/);

  const off = card({ meta: { reason: 'host_off' } });
  assert.match(off, /This computer(’|&#x27;|')s AI accounts are off in Default/);
  assert.match(off, /Add an account/);
});

test('once an account works, the card says to send again and drops the button', () => {
  const html = card({ meta: { reason: 'no_account' }, canRun: true });
  assert.match(html, /Architect can answer now/);
  assert.match(html, /Send your message again/);
  assert.doesNotMatch(html, /<button/);
});

test('canRunNow is unknown while loading or checking, never a false "nothing can run"', () => {
  const e = (id, status, over = {}) => ({ id, status, enabled: true, ...over });
  assert.equal(m.canRunNow(null), null);
  assert.equal(m.canRunNow([]), null);
  assert.equal(m.canRunNow([e('claude', 'checking'), e('codex', 'signed_out')]), null);
  assert.equal(m.canRunNow([e('claude', 'signed_out'), e('codex', 'not_installed')]), false);
  assert.equal(m.canRunNow([e('claude', 'checking', { enabled: false }), e('codex', 'signed_out')]), false);
  assert.equal(m.canRunNow([e('claude', 'signed_out', { ready: true })]), true);
  assert.equal(m.canRunNow([e('claude', 'ready'), e('codex', 'checking')]), true);
  assert.equal(m.canRunNow([e('claude', 'ready', { enabled: false })]), false);
});

test('a bot pinned to Ollama that cannot reach it gets an Ollama card with Re-check, not Add an account', () => {
  // What src/drivers/engines.rs ollama_down_notice saves on the message.
  assert.deepEqual(m.parseSetupMeta({ kind: 'setup_needed', reason: 'ollama_down', url: 'http://127.0.0.1:11434' }), {
    reason: 'ollama_down',
    url: 'http://127.0.0.1:11434',
  });
  assert.deepEqual(m.parseSetupMeta({ kind: 'setup_needed', reason: 'ollama_down' }), { reason: 'ollama_down', url: undefined });
  assert.deepEqual(m.parseSetupMeta({ kind: 'setup_needed', reason: 'ollama_down', url: 42 }), { reason: 'ollama_down', url: undefined });

  const meta = { reason: 'ollama_down', url: 'http://127.0.0.1:11434' };
  const html = card({ meta, onRecheck: () => {}, ollamaReady: false });
  assert.match(html, /Architect can(’|&#x27;|')t answer yet/);
  assert.match(html, /Ollama isn(’|&#x27;|')t answering at http:\/\/127\.0\.0\.1:11434\. Start it, then try again\./);
  assert.match(html, /data-setup-recheck/);
  assert.match(html, /Re-check<\/button>/);
  assert.doesNotMatch(html, /Add an account/);
  // Another account working does not fix a bot pinned to Ollama
  assert.match(card({ meta, onRecheck: () => {}, ollamaReady: false, canRun: true }), /can(’|&#x27;|')t answer yet/);

  const checking = card({ meta, onRecheck: () => {}, rechecking: true });
  assert.match(checking, /Checking…/);
  // The spinner stops under reduced motion; the label carries the state
  assert.match(checking, /class="animate-spin motion-reduce:animate-none"/);
  // Same fallback phrase as the server's (src/drivers/engines.rs shown_url)
  assert.match(card({ meta: { reason: 'ollama_down' }, onRecheck: () => {} }), /at the address in Settings\./);
  // Without a Re-check handler there is no button at all, never Accounts
  assert.doesNotMatch(card({ meta }), /<button/);

  const fixed = card({ meta, onRecheck: () => {}, ollamaReady: true });
  assert.match(fixed, /Architect can answer now/);
  assert.match(fixed, /Ollama is answering now\. Send your message again/);
  assert.doesNotMatch(fixed, /<button/);
});

test('a Re-check that finds Ollama still down says so', () => {
  assert.equal(
    m.ollamaDownDetail('http://127.0.0.1:11434', false),
    "Ollama isn't answering at http://127.0.0.1:11434. Start it, then try again.",
  );
  assert.equal(
    m.ollamaDownDetail('http://127.0.0.1:11434', true),
    "Ollama still isn't answering at http://127.0.0.1:11434. Start it, then try again.",
  );
  assert.equal(m.ollamaDownDetail(undefined, true), "Ollama still isn't answering at the address in Settings. Start it, then try again.");
  assert.equal(m.ollamaDownDetail(undefined, false), "Ollama isn't answering at the address in Settings. Start it, then try again.");
});
