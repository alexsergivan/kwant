// The token screen: one command up front, other setups folded away, steps that work without kwnt on the PATH.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { OWN_TOKEN_HINT, OtherSetups, TokenForm, WRONG_TOKEN, isLoopback, selectForRetry, tokenFromPaste, tunnelCommand } from './src/components/AuthGate';
  export { OWN_TOKEN_HINT, WRONG_TOKEN, isLoopback, selectForRetry, tokenFromPaste, tunnelCommand };
  export const form = () => renderToStaticMarkup(React.createElement(TokenForm, { notice: null, onSignedIn: () => {} }));
  export const other = (loc, docker = false) => renderToStaticMarkup(React.createElement(OtherSetups, { loc, docker }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/\s+/g, ' ');
const http = (hostname, port = '8799') => ({ protocol: 'http:', hostname, port });

test('the default view: one command, the token box and a folded "Other setups", no picker', () => {
  const html = m.form();
  const t = text(html);
  assert.match(t, /Find it on the machine running kwant bots:/);
  assert.match(t, /cat ~\/\.kwnt\/api-token/);
  assert.match(html, /placeholder="Paste the token or the sign-in link"/);
  assert.match(html, /<details class="[^"]*">/, 'closed by default');
  assert.match(t, /Other setups/);
  assert.match(t, /Stays signed in for a year\. Anyone with the token controls this server\./);
  assert.ok(!/Where does kwant bots run|On a server|type="radio"/.test(html));
  assert.ok(!t.includes(m.OWN_TOKEN_HINT.slice(0, 30)), 'the own-token hint only shows with a wrong token');
});

test('other setups: Docker, systemd and a source build, so it works without kwnt on the PATH', () => {
  const t = text(m.other(http('vps.example.com')));
  assert.match(t, /docker compose exec kwnt kwnt token --raw/);
  assert.match(t, /sudo cat \/var\/lib\/kwnt\/api-token/);
  assert.match(t, /\.\/target\/release\/kwnt token --raw/);
});

test('on loopback (Docker) the Docker command leads and the file command moves into other setups', () => {
  const t = text(m.other(http('127.0.0.1'), true));
  assert.match(t, /cat ~\/\.kwnt\/api-token/);
  assert.ok(!t.includes('docker compose'));
});

test('SSH tunnel: on plain http at a network address with a port, using that port', () => {
  const t = text(m.other(http('vps.example.com', '9000')));
  assert.match(t, /ssh -N -L 9000:127\.0\.0\.1:9000 user@vps\.example\.com/);
  assert.match(t, /http:\/\/127\.0\.0\.1:9000/);
  assert.equal(m.tunnelCommand(http('2001:db8::1')).command, 'ssh -N -L 8799:127.0.0.1:8799 user@[2001:db8::1]');
});

test('SSH tunnel: left out behind https, without a port, and on loopback', () => {
  for (const loc of [
    { protocol: 'https:', hostname: 'kwant.jumpin.games', port: '' },
    { protocol: 'https:', hostname: 'vps.example.com', port: '8443' },
    http('vps.example.com', ''),
    http('127.0.0.1'),
    http('localhost'),
  ]) {
    assert.equal(m.tunnelCommand(loc), null, JSON.stringify(loc));
    assert.ok(!text(m.other(loc)).includes('ssh -N'), JSON.stringify(loc));
  }
});

test('loopback hosts', () => {
  for (const h of ['127.0.0.1', 'localhost', '[::1]', '::1']) assert.ok(m.isLoopback(h), h);
  for (const h of ['vps.example.com', '10.0.0.5', 'mac.local']) assert.ok(!m.isLoopback(h), h);
});

test('pasting the whole sign-in link works as well as the token', () => {
  assert.equal(m.tokenFromPaste('  abc123def456abc123  '), 'abc123def456abc123');
  assert.equal(m.tokenFromPaste('http://1.2.3.4:8799/#kwnt_token=abc123def456abc123'), 'abc123def456abc123');
  assert.equal(m.tokenFromPaste('http://<this-host>:8799/#kwnt_token=tok'), 'tok');
});

test('a token from another computer: the error says where it has to come from', () => {
  assert.match(m.WRONG_TOKEN, /machine running kwant bots, as the user running it/);
  assert.match(m.OWN_TOKEN_HINT, /one copied from another computer won't work here/);
});

test('after a wrong token the box is focused and selected, so the next paste replaces it', () => {
  const calls = [];
  m.selectForRetry({ focus: () => calls.push('focus'), select: () => calls.push('select') });
  assert.deepEqual(calls, ['focus', 'select']);
  assert.doesNotThrow(() => m.selectForRetry(null));
});
