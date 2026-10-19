import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`export * from './src/mcpSuggestions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { McpSuggestionCard } from './src/components/McpSuggestionCard';
  export const render = (props) => renderToStaticMarkup(React.createElement(McpSuggestionCard, props));
`);

const httpMeta = (over = {}) => ({
  kind: 'mcp_suggestion', suggestion_id: 's1', kwant_id: 'k1', registry_name: 'io.example/echo', version: '1.2.0',
  title: 'Echo', description: 'Repeats what you send.', reason: 'You asked me to echo things.',
  repository_url: 'https://github.com/example/echo', transport: 'http', url: 'https://echo.example/mcp',
  command: null, args: [], package: null, runs_on_your_computer: false, fixed_env: [], fixed_headers: [],
  inputs: [{ name: 'Authorization', kind: 'header', description: 'Your Echo key', required: true, secret: true, template: 'Bearer {api_key}', placeholder: null }],
  status: 'pending', server_id: null, last_error: null, ...over,
});
const stdioMeta = (over = {}) =>
  httpMeta({
    transport: 'stdio', url: null, command: 'npx', args: ['-y', '@example/echo-mcp@1.2.0'], package: 'npm @example/echo-mcp@1.2.0',
    runs_on_your_computer: true, fixed_env: ['LOG_LEVEL'],
    inputs: [{ name: 'ECHO_KEY', kind: 'env', description: 'Key', required: true, secret: true, template: null, placeholder: null },
             { name: 'ECHO_REGION', kind: 'env', description: '', required: false, secret: false, template: null, placeholder: 'eu' }],
    ...over,
  });
const noop = async () => ({});
const props = (meta, over = {}) => ({
  meta: s.parseSuggestionMeta(meta), messageId: 'm1', kwantName: 'Scout', kwantId: 'k1',
  onInstall: noop, onDismiss: async () => {}, onSignIn: async () => {}, ...over,
});

test('card meta parses from a string or object and ignores other kinds', () => {
  const m = s.parseSuggestionMeta(JSON.stringify(httpMeta()));
  assert.equal(m.title, 'Echo');
  assert.equal(m.inputs[0].template, 'Bearer {api_key}');
  assert.equal(s.parseSuggestionMeta({ kind: 'ask_user', question_id: 'q' }), null);
  assert.equal(s.parseSuggestionMeta('nope'), null);
  assert.equal(s.parseSuggestionMeta({ kind: 'mcp_suggestion' }), null, 'needs an id and a name');
  assert.equal(s.parseSuggestionMeta(httpMeta({ status: 'weird' })).status, 'pending');
  assert.equal(s.parseSuggestionMeta(httpMeta({ inputs: [{ name: 'X', kind: 'cookie' }, 3] })).inputs.length, 0);
});

test('registry links are only clickable when they are http(s)', () => {
  assert.equal(s.parseSuggestionMeta(httpMeta({ repository_url: 'javascript:alert(1)' })).repository_url, null);
  assert.equal(s.parseSuggestionMeta(httpMeta({ repository_url: 'not a url' })).repository_url, null);
  assert.equal(s.safeLink('https://github.com/x'), 'https://github.com/x');
});

test('the live record wins; a finished suggestion never goes back to pending', () => {
  const m = s.parseSuggestionMeta(httpMeta());
  const live = { id: 's1', kwant_id: 'k1', status: 'installed', server_id: 'srv', last_error: null, message_id: 'm1', created_at: 'x' };
  assert.equal(s.suggestionView(m, live).status, 'installed');
  assert.equal(s.suggestionView(m, { ...live, id: 'other' }).status, 'pending');
  let map = s.applySuggestion({}, live);
  map = s.applySuggestion(map, { ...live, status: 'pending' });
  assert.equal(map.s1.status, 'installed');
  map = s.applySuggestion({}, { ...live, status: 'installing' });
  map = s.applySuggestion(map, { ...live, status: 'pending', last_error: 'bad key' });
  assert.equal(map.s1.last_error, 'bad key', 'a failed install returns to pending with its error');
});

test('a template asks only for the key; the body carries what was typed, trimmed', () => {
  const [auth] = s.parseSuggestionMeta(httpMeta()).inputs;
  const l = s.inputLabel(auth);
  assert.equal(l.label, 'API key');
  assert.match(l.note, /Bearer <api key>/);
  assert.deepEqual(s.installPayload([auth], { 'header:Authorization': '  abc ' }), { body: { headers: { Authorization: 'abc' } }, missing: [] });
  assert.deepEqual(s.installPayload([auth], {}).missing, ['API key']);
  const env = s.parseSuggestionMeta(stdioMeta()).inputs;
  assert.deepEqual(s.installPayload(env, { 'env:ECHO_KEY': 'k', 'env:ECHO_REGION': ' ' }).body, { env: { ECHO_KEY: 'k' } }, 'blank optional fields are left out');
  assert.equal(s.inputLabel(env[0]).label, 'ECHO_KEY');
  assert.equal(s.humanizeSlot('accessToken'), 'Access token');
  assert.equal(s.templateSlot('{a} {b}'), null, 'several slots: typed as is');
});

test('http card: online service, the address, a password field for the key, Install and Not now', () => {
  const html = ui.render(props(httpMeta()));
  assert.match(html, /Scout<\/span> suggests a tool/);
  assert.match(html, /Online service/);
  assert.doesNotMatch(html, /Runs on your computer/);
  assert.match(html, /Installing connects to:/);
  assert.match(html, /https:\/\/echo\.example\/mcp/);
  assert.match(html, /You asked me to echo things\./);
  assert.match(html, /type="password"[^>]*autoComplete="new-password"|autoComplete="new-password"[^>]*type="password"/);
  assert.match(html, />API key</);
  assert.match(html, />Install</);
  assert.match(html, />Not now</);
  assert.match(html, /href="https:\/\/github\.com\/example\/echo"[^>]*rel="noopener noreferrer"/);
});

test('stdio card: "Runs on your computer" with the exact command; optional fields say so', () => {
  const html = ui.render(props(stdioMeta()));
  assert.match(html, /Runs on your computer/);
  assert.match(html, /Installing runs this command:/);
  assert.match(html, /npx -y @example\/echo-mcp@1\.2\.0/);
  assert.match(html, /Also sets: LOG_LEVEL/);
  assert.match(html, /\(optional\)/);
  assert.match(html, /type="text"[^>]*placeholder="eu"/);
});

test('a failed install keeps the form and offers to try again', () => {
  const html = ui.render(props(httpMeta({ last_error: "The server didn't start: transport error: HTTP 401 Unauthorized (sign in to this server from the Tools screen)" })));
  assert.match(html, /role="alert"[^>]*>.*It didn’t install\. The server turned down the key\. Check it and try again\./s);
  assert.doesNotMatch(html, /Tools screen|transport error/);
  assert.match(html, />Try again</);
  assert.match(html, /type="password"/, 'the field is still there');
});

test('failure wording: a refused key is named; other errors keep their detail', () => {
  const raw = "The server didn't start: transport error: HTTP 401 Unauthorized (sign in to this server from the Tools screen)";
  assert.equal(s.failureText(raw, true), 'The server turned down the key. Check it and try again.');
  assert.equal(s.failureText("The server didn't start: connection refused", true), "The server didn't start: connection refused. Check what you typed and try again.");
  assert.equal(s.failureText("The server didn't start: timed out.", false), "The server didn't start: timed out. You can try again.");
});

test('installing, installed, needs sign-in and dismissed states', () => {
  const installing = ui.render(props(httpMeta({ status: 'installing' })));
  assert.match(installing, /Installing and testing…/);
  assert.match(installing, /disabled=""/);

  const ready = ui.render(props(httpMeta({ status: 'installed', server_id: 'srv' })));
  assert.match(ready, /Ready\. Scout can use it from its next reply\./);
  assert.doesNotMatch(ready, /type="password"|>Install</);

  const signIn = ui.render(props(httpMeta({ status: 'installed', server_id: 'srv' }), { needsSignIn: true }));
  assert.match(signIn, /Sign in to Echo so Scout can use it/);
  assert.match(signIn, />Sign in</);

  const no = ui.render(props(httpMeta({ status: 'dismissed' })));
  assert.match(no, /Not installed\. Scout carries on without it\./);
  assert.doesNotMatch(no, /<form/);
});
