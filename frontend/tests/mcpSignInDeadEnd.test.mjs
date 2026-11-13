// #574, Mira's design check: a sign-in that can never work from here (the start call returned
// no_client_registration or cimd_needs_https) has no Retry, Connect or "Try signing in again"; one grey
// next step takes the button's place, on the Tool servers screen and on the chat card.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The servers screen's module reads saved statuses when it loads.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SignInBox } from './src/components/McpServersModal';
  import { SuggestionSignIn } from './src/components/McpSuggestionCard';
  export * from './src/mcpSignInDeadEnd';
  const noop = () => {};
  export const box = (failure, over = {}) =>
    renderToStaticMarkup(React.createElement(SignInBox, { serverName: 'ElevenLabs', isConnecting: false, macApp: false, failure, onConnect: noop, ...over }));
  export const card = (error, over = {}) =>
    renderToStaticMarkup(React.createElement(SuggestionSignIn, { busy: false, disabled: false, error, onSignIn: noop, ...over }));
`);

// renderToStaticMarkup escapes apostrophes; compare against the text a person reads.
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ');

const NO_REG = 'This server only lets in apps that were set up with it in advance, so signing in from here can’t work.';
const HTTPS = 'This server’s sign-in only works from localhost or an https address, not from a plain http address like this one.';
const LOCALHOST = 'Open this app in a browser on the computer it runs on, at localhost, and sign in there. Your bots here use that sign-in too.';

test('copy per code and surface is Mira’s, word for word', () => {
  assert.deepEqual(ui.signInDeadEnd('no_client_registration', 'ElevenLabs', 'servers'), {
    code: 'no_client_registration',
    error: NO_REG,
    next: 'If ElevenLabs gives you an API key, edit this server and add the key under Headers instead.',
  });
  assert.deepEqual(ui.signInDeadEnd('no_client_registration', 'ElevenLabs', 'card'), {
    code: 'no_client_registration',
    error: `Couldn’t start the sign-in. ${NO_REG}`,
    next: 'If ElevenLabs gives you an API key, add it under Headers in Settings › Apps & accounts › Advanced › Tool servers.',
  });
  assert.deepEqual(ui.signInDeadEnd('cimd_needs_https', 'ElevenLabs', 'servers'), { code: 'cimd_needs_https', error: HTTPS, next: LOCALHOST });
  assert.deepEqual(ui.signInDeadEnd('cimd_needs_https', 'ElevenLabs', 'card'), {
    code: 'cimd_needs_https',
    error: `Couldn’t start the sign-in. ${HTTPS}`,
    next: LOCALHOST,
  });
  // Any other code, or none, keeps today's error and its button.
  for (const c of [undefined, null, '', 'invalid_redirect', 42]) assert.equal(ui.signInDeadEnd(c, 'X', 'card'), null);
  assert.equal(ui.isSignInDeadEndCode('cimd_needs_https'), true);
  assert.equal(ui.isSignInDeadEndCode('NO_CLIENT_REGISTRATION'), false);
});

test('tool servers screen, no_client_registration: no Retry or Connect, the API-key step shows', () => {
  const html = ui.box({ message: "this server only lets in apps that were set up with it in advance, so signing in from here can't work", code: 'no_client_registration' });
  assert.doesNotMatch(html, /<button/);
  assert.doesNotMatch(text(html), /\bRetry\b|\bConnect\b/);
  assert.match(text(html), /This server asks you to sign in and approve access\./);
  assert.match(html, /<p role="alert" class="flex items-start gap-1.5 text-rose-300">/);
  assert.ok(text(html).includes(NO_REG));
  assert.ok(html.includes('<p class="text-zinc-400">If ElevenLabs gives you an API key, edit this server and add the key under Headers instead.</p>'));
});

test('tool servers screen, cimd_needs_https: no Retry or Connect, the localhost step shows', () => {
  const html = ui.box({ message: 'whatever the server said', code: 'cimd_needs_https' });
  assert.doesNotMatch(html, /<button/);
  assert.ok(text(html).includes(HTTPS));
  assert.doesNotMatch(text(html), /whatever the server said/);
  assert.ok(html.includes(`<p class="text-zinc-400">${LOCALHOST}</p>`));
});

test('tool servers screen keeps Retry for other errors, and Connect with none', () => {
  const failed = ui.box({ message: 'token request failed: 500' });
  assert.match(failed, /<button[^>]*>.*Retry<\/button>/);
  assert.match(text(failed), /token request failed: 500/);
  assert.doesNotMatch(failed, /text-zinc-400/);
  assert.match(ui.box(undefined), /<button[^>]*>.*Connect<\/button>/);
  // A new attempt in progress shows the waiting state, not the old dead end.
  const waiting = ui.box({ message: 'm', code: 'cimd_needs_https' }, { isConnecting: true });
  assert.match(waiting, /Waiting…<\/button>/);
  assert.doesNotMatch(waiting, /localhost/);
});

test('chat card, no_client_registration: "Try signing in again" is gone, the API-key step shows', () => {
  const html = ui.card(ui.signInDeadEnd('no_client_registration', 'ElevenLabs', 'card'));
  assert.doesNotMatch(html, /<button/);
  assert.doesNotMatch(text(html), /Try signing in again|Sign in\b/);
  assert.ok(html.includes(`<p role="alert" class="text-rose-300 break-words">Couldn’t start the sign-in. ${NO_REG.replace("'", '&#x27;')}</p>`));
  assert.ok(html.includes('<p class="text-zinc-400 break-words">If ElevenLabs gives you an API key, add it under Headers in Settings › Apps &amp; accounts › Advanced › Tool servers.</p>'));
});

test('chat card, cimd_needs_https: "Try signing in again" is gone, the localhost step shows', () => {
  const html = ui.card(ui.signInDeadEnd('cimd_needs_https', 'ElevenLabs', 'card'));
  assert.doesNotMatch(html, /<button/);
  assert.ok(text(html).includes(`Couldn’t start the sign-in. ${HTTPS}`));
  assert.ok(html.includes(`<p class="text-zinc-400 break-words">${LOCALHOST}</p>`));
});

test('chat card keeps "Try signing in again" for a plain error, and "Sign in" before any try', () => {
  const failed = ui.card("Couldn't start the sign-in: token request failed: 500");
  assert.match(failed, /Try signing in again<\/button>/);
  assert.match(text(failed), /token request failed: 500/);
  assert.match(ui.card(null), />Sign in<\/button>/);
});
