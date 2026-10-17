import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const profiles = await load('../src/profiles.ts');
const auth = await load('../src/auth.ts');
const activity = await load('../src/kwantActivity.ts');

const profile = (over) => ({
  id: 'prf_x', engine: 'claude', name: 'Work', kind: 'cli_home', enabled: true, is_default: false, priority: 100,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null, ...over,
});

test('maps models to engines like the server', () => {
  const { engineOfModel } = profiles;
  assert.equal(engineOfModel('claude'), 'claude');
  assert.equal(engineOfModel('claude-opus-5-5'), 'claude');
  assert.equal(engineOfModel('grok-beta'), 'grok');
  assert.equal(engineOfModel('gpt-5'), 'codex');
  assert.equal(engineOfModel('codex'), 'codex');
  assert.equal(engineOfModel('auto'), null);
  assert.equal(engineOfModel('local_ollama'), null);
});

test('lists an engine default first, then by ascending priority', () => {
  const list = [
    profile({ id: 'a', priority: 50 }),
    profile({ id: 'b', priority: 10 }),
    profile({ id: 'sys', kind: 'system', is_default: true, priority: 100 }),
    profile({ id: 'g', engine: 'grok' }),
  ];
  assert.deepEqual(profiles.profilesForEngine(list, 'claude').map((p) => p.id), ['sys', 'b', 'a']);
  assert.deepEqual(profiles.profilesForEngine(list, null), []);
});

test('status explains usage limits, sign-in and keys without showing secrets', () => {
  const now = Date.parse('2026-10-16T12:00:00Z');
  const { profileStatus } = profiles;
  assert.deepEqual(profileStatus(profile({ limited_until: '2026-10-16T12:45:00Z' }), now), { tone: 'warn', text: 'Out of usage · back in 45 min' });
  assert.equal(profileStatus(profile({ limited_until: '2026-10-16T11:00:00Z', account_label: 'me@x.io' }), now).text, 'Signed in as me@x.io');
  assert.equal(profileStatus(profile({ last_error: 'auth' }), now).tone, 'error');
  assert.equal(profileStatus(profile({ kind: 'api_key', has_key: false }), now).text, 'No key yet');
  assert.equal(profileStatus(profile({ kind: 'api_key', has_key: true }), now).text, 'Key saved');
  assert.equal(profileStatus(profile({ enabled: false, last_error: 'quota' }), now).text, 'Turned off');
  assert.equal(profileStatus(profile({ signed_in: null, kind: 'system' }), now).tone, 'muted');
});

test('only opens https sign-in links', () => {
  assert.equal(profiles.safeExternalUrl('https://claude.ai/oauth/authorize?x=1'), 'https://claude.ai/oauth/authorize?x=1');
  assert.equal(profiles.safeExternalUrl('javascript:alert(1)'), null);
  assert.equal(profiles.safeExternalUrl('http://evil.example'), null);
  assert.equal(profiles.safeExternalUrl(null), null);
});

test('describes a profile switch for the toast', () => {
  const now = Date.parse('2026-10-16T12:00:00Z');
  const ev = { kwant_id: 'k', from_profile_id: 'a', from_name: 'Work', to_profile_id: 'b', to_name: 'Personal', class: 'quota', retry_at: '2026-10-16T14:00:00Z' };
  assert.deepEqual(profiles.describeProfileSwitch(ev, 'Ada', now), { title: 'Ada switched to Personal', detail: 'Work is out of usage until it resets in 2 h.' });
  assert.equal(profiles.describeProfileSwitch({ ...ev, class: 'auth' }, undefined, now).detail, 'Work is signed out.');
});

test('takes the token from the link and keeps other fragment parts', () => {
  const { extractHashToken } = auth;
  assert.deepEqual(extractHashToken('#kwnt_token=abc123'), { token: 'abc123', rest: '' });
  assert.deepEqual(extractHashToken('#tab=x&kwnt_token=a%2Bb'), { token: 'a+b', rest: '#tab=x' });
  assert.deepEqual(extractHashToken('#section'), { token: null, rest: '#section' });
  assert.deepEqual(extractHashToken(''), { token: null, rest: '' });
});

test('only same-origin /api calls outside /api/auth count as missing the token', () => {
  const o = 'http://127.0.0.1:8799';
  assert.equal(auth.isGuardedApiUrl('/api/kwants', o), true);
  assert.equal(auth.isGuardedApiUrl(`${o}/api/profiles/x`, o), true);
  assert.equal(auth.isGuardedApiUrl('/api/auth/session', o), false);
  assert.equal(auth.isGuardedApiUrl('https://other.example/api/kwants', o), false);
  assert.equal(auth.isGuardedApiUrl('/apiary', o), false);
});

test('a kwant works until every turn it started has ended, whatever the outcome', () => {
  const seen = [];
  const off = activity.subscribeKwant('k1', () => seen.push(activity.isKwantWorking('k1')));
  activity.turnStarted('k1', 'dm');
  activity.turnStarted('k1', 'team');
  assert.equal(activity.isKwantWorking('k1'), true);
  activity.turnEnded('k1', 'dm');
  assert.equal(activity.isKwantWorking('k1'), true);
  activity.turnEnded('k1', 'team');
  assert.equal(activity.isKwantWorking('k1'), false);
  // An end for a context we never saw still stops the animation.
  activity.turnStarted('k1', null);
  activity.turnEnded('k1', 'unknown');
  assert.equal(activity.isKwantWorking('k1'), false);
  activity.turnStarted('k1', 'x');
  activity.clearAllTurns();
  assert.equal(activity.isKwantWorking('k1'), false);
  off();
  activity.turnStarted('k1', 'y');
  activity.turnEnded('k1', 'y');
  assert.deepEqual(seen, [true, false, true, false, true, false]);
});
