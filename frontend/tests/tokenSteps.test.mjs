// The token screen: steps that work for where kwant runs, without kwnt on the PATH.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { TokenSteps, guessPlace, tokenFromPaste } from './src/components/AuthGate';
  export { guessPlace, tokenFromPaste };
  export const steps = (place, hostname = 'vps.example.com', port = '8799') =>
    renderToStaticMarkup(React.createElement(TokenSteps, { place, hostname, port }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/\s+/g, ' ');

test('server: the token file and a source-build path, so it works without kwnt on the PATH', () => {
  const t = text(m.steps('server'));
  assert.match(t, /as the user that runs kwant bots/);
  assert.match(t, /cat ~\/\.kwnt\/api-token/);
  assert.match(t, /sudo cat \/var\/lib\/kwnt\/api-token/);
  assert.match(t, /\.\/target\/release\/kwnt token --raw/);
});

test('server: an SSH tunnel to this host and port skips the token', () => {
  const t = text(m.steps('server', 'vps.example.com', '9000'));
  assert.match(t, /ssh -N -L 9000:127\.0\.0\.1:9000 user@vps\.example\.com/);
  assert.match(t, /http:\/\/127\.0\.0\.1:9000/);
  assert.match(t, /Not for Docker/);
  assert.match(text(m.steps('server', '2001:db8::1')), /user@\[2001:db8::1\]/);
});

test('docker: compose and plain docker, no tunnel', () => {
  const t = text(m.steps('docker'));
  assert.match(t, /docker compose exec kwnt kwnt token --raw/);
  assert.match(t, /docker exec <container> kwnt token --raw/);
  assert.ok(!t.includes('ssh -N'));
});

test('this computer: open 127.0.0.1, which never asks', () => {
  const html = m.steps('here', '192.168.1.20', '8799');
  assert.match(html, /href="http:\/\/127\.0\.0\.1:8799"/);
  assert.match(text(html), /not at this computer's network address/);
});

test('a locked page on loopback is Docker; anywhere else a server', () => {
  for (const h of ['127.0.0.1', 'localhost', '[::1]']) assert.equal(m.guessPlace(h), 'docker', h);
  for (const h of ['vps.example.com', '10.0.0.5', 'mac.local']) assert.equal(m.guessPlace(h), 'server', h);
});

test('pasting the whole sign-in link works as well as the token', () => {
  assert.equal(m.tokenFromPaste('  abc123def456abc123  '), 'abc123def456abc123');
  assert.equal(m.tokenFromPaste('http://1.2.3.4:8799/#kwnt_token=abc123def456abc123'), 'abc123def456abc123');
  assert.equal(m.tokenFromPaste('http://<this-host>:8799/#kwnt_token=tok'), 'tok');
});
