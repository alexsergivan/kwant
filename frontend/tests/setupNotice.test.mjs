import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { SetupNeededCard, parseSetupMeta } from './src/components/SetupNeededCard';
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
  assert.match(off, /This machine is off in Default/);
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
