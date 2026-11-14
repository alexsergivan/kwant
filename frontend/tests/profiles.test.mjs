import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';
import { loadBundled } from './bundle.mjs';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const profiles = await load('../src/profiles.ts');
// auth.ts imports spaces.ts, so it's bundled.
const auth = await loadBundled("export * from './src/auth.ts';");
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
  assert.deepEqual(profileStatus(profile({ limited_until: '2026-10-16T12:45:00Z' }), now), { tone: 'warn', text: 'Out of usage · Back in 45 min' });
  assert.equal(profileStatus(profile({ limited_until: '2026-10-16T11:00:00Z', account_label: 'me@x.io' }), now).text, 'Ready · me@x.io');
  assert.equal(profileStatus(profile({}), now).text, 'Ready');
  assert.equal(profileStatus(profile({ last_error: 'auth' }), now).tone, 'error');
  assert.equal(profileStatus(profile({ last_error: 'auth' }), now).text, 'Needs sign-in');
  assert.equal(profileStatus(profile({ signed_in: false }), now).text, 'Needs sign-in');
  assert.equal(profileStatus(profile({ kind: 'api_key', has_key: false }), now).text, 'No key');
  assert.equal(profileStatus(profile({ kind: 'api_key', has_key: true }), now).text, 'Ready');
  assert.equal(profileStatus(profile({ enabled: false, last_error: 'quota' }), now).text, 'Off');
  assert.equal(profileStatus(profile({ signed_in: null, kind: 'system' }), now).tone, 'muted');
});

test('a Claude Code set up for another provider reads as "Other provider", never "Signed in as"', () => {
  const { accountState, isThirdPartyLabel } = profiles;
  const bedrock = profile({ account_label: 'Amazon Bedrock' });
  assert.deepEqual(accountState(bedrock), { label: 'Other provider', tone: 'warn', detail: 'Runs through Amazon Bedrock instead of its own Claude login' });
  // On This machine it's a setup, not a problem.
  assert.equal(accountState(profile({ kind: 'system', account_label: 'Google Vertex AI' })).tone, 'info');
  // An older server's raw auth method.
  assert.equal(accountState(profile({ account_label: 'third_party' })).detail, 'Runs through a third-party provider instead of its own Claude login');
  assert.equal(profiles.profileStatus(bedrock).text, 'Other provider · Runs through Amazon Bedrock instead of its own Claude login');
  assert.ok(!isThirdPartyLabel('me@x.io'));
  // Off wins over everything, so a switched-off account never looks in use.
  assert.equal(accountState(profile({ enabled: false, account_label: 'Amazon Bedrock' })).label, 'Off');
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

// ---- account-usage: 5-hour and weekly limits on Claude/Codex sign-ins (design.md §3) ----

const at = (d, h, m = 0) => new Date(2026, 10, d, h, m); // local time, Nov 2026 (13th is a Friday)
const clock = (d) => d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
const wd = (d) => d.toLocaleDateString(undefined, { weekday: 'short' });
const usage = (fiveUsed, weekUsed, over = {}) => ({
  checked_at: at(13, 20, 31).toISOString(),
  source: 'turn',
  status: 'ok',
  windows: [
    { kind: 'weekly', used_pct: weekUsed, resets_at: at(18, 0, 59).toISOString() },
    { kind: 'five_hour', used_pct: fiveUsed, resets_at: at(13, 22, 29).toISOString() },
  ],
  ...over,
});
const NOW = at(13, 20, 32).getTime();

test('usageRows: % left, 5-hour first, reset today vs. within the week vs. later', () => {
  const rows = profiles.usageRows(profile({ usage: usage(7, 56) }), NOW);
  assert.deepEqual(rows.map((r) => [r.label, r.left, r.leftText, r.tone]), [
    ['5-hour', 93, '93% left', 'ok'],
    ['Weekly', 44, '44% left', 'ok'],
  ]);
  assert.equal(rows[0].reset, `resets ${clock(at(13, 22, 29))}`);
  assert.equal(rows[1].reset, `resets ${wd(at(18, 0, 59))} ${clock(at(18, 0, 59))}`); // "resets Wed 00:59"
  const later = profiles.usageRows(profile({ usage: usage(7, 56, { windows: [{ kind: 'weekly', used_pct: 1, resets_at: at(20, 9).toISOString() }] }) }), NOW);
  assert.equal(later[0].reset, `resets ${at(20, 9).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`);
  assert.equal(profiles.usageRows(profile({ usage: usage(7, 56, { windows: [{ kind: 'five_hour', used_pct: 7, resets_at: null }] }) }), NOW)[0].reset, null);
});

test('usageRows: amber below 20% left, out at 0, clamped', () => {
  const tones = (u5, uw) => profiles.usageRows(profile({ usage: usage(u5, uw) }), NOW).map((r) => r.tone);
  assert.deepEqual(tones(80, 81), ['ok', 'low']); // 20% left is not low; 19% is
  assert.deepEqual(tones(100, 130), ['out', 'out']);
  assert.equal(profiles.usageRows(profile({ usage: usage(-5, 0) }), NOW)[0].left, 100);
  assert.equal(profiles.usageRows(profile({ usage: usage(12.4, 0) }), NOW)[0].leftText, '88% left');
});

test('usageRows: a reset that has passed shows "reset at …" with no %', () => {
  const [five, week] = profiles.usageRows(profile({ usage: usage(97, 10) }), at(13, 23, 0).getTime());
  assert.deepEqual([five.left, five.leftText, five.tone], [null, null, 'reset']);
  assert.equal(five.reset, `reset at ${clock(at(13, 22, 29))}`);
  assert.equal(week.tone, 'ok');
  const [next] = profiles.usageRows(profile({ usage: usage(97, 10) }), at(14, 9, 0).getTime());
  assert.equal(next.reset, `reset at ${wd(at(13, 22, 29))} ${clock(at(13, 22, 29))}`);
});

test('usageRows: nothing for API keys, Custom API, Grok, or never read', () => {
  assert.deepEqual(profiles.usageRows(profile({ kind: 'api_key', usage: usage(7, 56) }), NOW), []);
  assert.deepEqual(profiles.usageRows(profile({ engine: 'custom', kind: 'custom_api', usage: usage(7, 56) }), NOW), []);
  assert.deepEqual(profiles.usageRows(profile({ engine: 'grok', usage: usage(7, 56) }), NOW), []);
  assert.deepEqual(profiles.usageRows(profile({ usage: null }), NOW), []);
  assert.deepEqual(profiles.usageRows(profile({}), NOW), []);
  assert.equal(profiles.usageRows(profile({ engine: 'codex', kind: 'system', usage: usage(7, 56) }), NOW).length, 2);
});

test('usageCheckedText: the "checked … ago" wording', () => {
  const t = (s) => profiles.usageCheckedText({ checked_at: new Date(NOW - s * 1000).toISOString() }, NOW);
  assert.equal(t(10), 'checked just now');
  assert.equal(t(60), 'checked 1 min ago');
  assert.equal(t(4 * 60 + 30), 'checked 4 min ago');
  assert.equal(t(59 * 60), 'checked 59 min ago');
  assert.equal(t(6 * 3600 + 1200), 'checked 6 h ago');
  assert.equal(profiles.usageCheckedText({ checked_at: at(12, 9).toISOString() }, NOW), 'checked yesterday');
  assert.equal(profiles.usageCheckedText({ checked_at: at(11, 9).toISOString() }, NOW), `checked ${at(11, 9).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`);
  assert.equal(profiles.usageCheckedText(null, NOW), null);
  assert.equal(profiles.usageCheckedText({ checked_at: 'nope' }, NOW), null);
});

test('usageStale over 60 min, and Check now waits 60 s after a read', () => {
  const ago = (s) => ({ checked_at: new Date(NOW - s * 1000).toISOString() });
  assert.equal(profiles.usageStale(ago(3600), NOW), false);
  assert.equal(profiles.usageStale(ago(3601), NOW), true);
  assert.equal(profiles.usageStale(null, NOW), false);
  assert.equal(profiles.usageCoolingDown(ago(30), NOW), true);
  assert.equal(profiles.usageCoolingDown(ago(60), NOW), false);
  assert.equal(profiles.usageCoolingDown(null, NOW), false);
});

test('usageSummary: the badge hover line (design §5)', () => {
  const sum = (u, now = NOW, name) => profiles.usageSummary(profile({ usage: u }), now, name);
  assert.deepEqual(sum(usage(7, 56)), { text: '5-hour 93% left · Weekly 44% left', low: false });
  assert.deepEqual(sum(usage(88, 56)), { text: `5-hour 12% left, resets ${clock(at(13, 22, 29))} · Weekly 44% left`, low: true });
  assert.equal(sum(usage(100, 69)).text, `5-hour 0% left, resets ${clock(at(13, 22, 29))} · Weekly 31% left`);
  assert.equal(sum(usage(7, 56), NOW, 'Work').text, 'Work · 5-hour 93% left · Weekly 44% left');
  // Stale: the reset window drops out and the age is added; both reset: no line.
  assert.equal(sum(usage(7, 56), at(14, 3, 0).getTime()).text, 'Weekly 44% left · checked 6 h ago');
  assert.equal(sum(usage(7, 56), at(19, 3, 0).getTime()), null);
  assert.equal(profiles.usageSummary(profile({ kind: 'api_key', usage: usage(7, 56) }), NOW), null);
});
