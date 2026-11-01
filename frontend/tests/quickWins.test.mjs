// Pre-launch quick wins (ux-review/review.md, PR 1): one-line reply meta, readable quota times,
// raw commands behind Details, engine names in tooltips, a first-run welcome and "Pick one to start".
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

// Local times read in this process's zone; pin it before anything formats a date.
process.env.TZ = 'UTC';
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ReplyMeta } from './src/components/ReplyMeta';
  import { FirstRunWelcome } from './src/components/FirstRunWelcome';
  import { StartPicker } from './src/components/StartPicker';
  import { ConnectionRow } from './src/components/ConnectionsView';
  export { localClock, quotaLine } from './src/clock';
  export { engineTip, INSTALL_URL } from './src/engines';
  export { isRawCommand } from './src/connections';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const meta = r(ReplyMeta);
  export const welcome = r(FirstRunWelcome);
  export const picker = r(StartPicker);
  export const row = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(ConnectionRow, props)));
  export const el = React.createElement;
`);

const NOW = Date.parse('2026-11-02T12:00:00Z');

test('localClock: the time today, the date on another day, the year in another year', () => {
  assert.equal(m.localClock('2026-11-02T20:15:00Z', NOW, 'en-US'), '8:15 PM');
  assert.equal(m.localClock('2026-11-03T08:01:00Z', NOW, 'en-US'), 'Nov 3, 8:01 AM');
  assert.equal(m.localClock('2027-01-05T09:00:00Z', NOW, 'en-US'), 'Jan 5, 2027, 9:00 AM');
  assert.equal(m.localClock(null, NOW), null);
  assert.equal(m.localClock('not a time', NOW), null);
});

test('quotaLine: "Out of quota until …" while the limit lasts, the server words otherwise', () => {
  assert.equal(m.quotaLine('Out of quota', '2026-11-02T20:15:00Z', NOW, 'en-US'), 'Out of quota until 8:15 PM');
  assert.equal(m.quotaLine('Out of quota', '2026-11-02T10:00:00Z', NOW, 'en-US'), 'Out of quota');
  assert.equal(m.quotaLine('Not checked yet', null, NOW, 'en-US'), 'Not checked yet');
  assert.equal(m.quotaLine(null, undefined, NOW), null);
});

test('reply meta is one line, name · time, with engine, account and tokens behind Details', () => {
  const html = m.meta({
    name: 'Architect',
    at: '2026-11-02T11:42:00Z',
    now: NOW,
    ranOn: 'Claude · Opus 5.5',
    account: 'Personal',
    tokens: { label: '1.2k tokens', detail: '900 in · 300 out' },
  });
  assert.match(html, />Architect</);
  assert.match(html, /data-reply-time/);
  assert.match(html, /aria-expanded="false"/);
  assert.match(html, />Details</);
  // Folded: none of the facts show.
  assert.doesNotMatch(html, /Opus 5\.5|Personal|1\.2k tokens/);
});

test('reply meta: a failed step shows on the folded toggle; no toggle without details', () => {
  assert.match(m.meta({ name: 'Ada', ranOn: 'Codex', failed: true }), /Details · a step failed/);
  const bare = m.meta({ name: 'Ada' });
  assert.doesNotMatch(bare, /Details/);
  assert.doesNotMatch(bare, /data-reply-time/);
});

test('engineTip: the engine goes in the tooltip, Auto says Laya picks', () => {
  assert.equal(m.engineTip('auto'), 'Laya picks the model');
  assert.match(m.engineTip('claude'), /^Runs on /);
});

test('sidebar rows carry no LAYA pill or engine badge; the engine is in the name tooltip', () => {
  const src = readFileSync(new URL('../src/components/Sidebar.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(src, />\s*Laya\s*</);
  assert.match(src, /title=\{`\$\{k\.name\} · \$\{engineTip\(k\.preferred_model\)\}`\}/);
  // No account yet reads as a next step, not a warning.
  assert.doesNotMatch(src, /Nothing can run here/);
});

test('first-run welcome: what kwant bots are, the bots, one action', () => {
  let clicked = 0;
  const html = m.welcome({
    bots: [
      { id: 'a', name: 'Architect', title: 'Systems Architect', avatar_sprite: 'architect' },
      { id: 'b', name: 'TechLead' },
    ],
    actionLabel: 'Add an account',
    onAction: () => clicked++,
  });
  assert.match(html, /data-first-run-welcome/);
  assert.match(html, /Laya picks who answers/);
  assert.match(html, /Architect<\/span><span[^>]*> · Systems Architect/);
  assert.match(html, />TechLead</);
  assert.equal(html.split('<button').length, 2, 'exactly one button');
  assert.match(html, /Add an account<\/button>/);
});

const engine = (id, over = {}) => ({
  id, name: id, kind: 'cli', status: 'not_installed', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: false, uses: null, ...over,
});

test('Pick one to start: three choices; Claude opens first when it only needs a sign-in', () => {
  const html = m.picker({ engines: [engine('claude', { status: 'signed_out' })], onSignedIn() {}, onKeyAdded() {}, initial: 'claude' });
  assert.match(html, /Pick one to start/);
  for (const c of ['claude', 'codex', 'key']) assert.match(html, new RegExp(`data-start-choice="${c}"`));
  assert.match(html, /Sign in with ChatGPT/);
  assert.match(html, /Paste an API key/);
  assert.match(html, /data-start-choice="claude"[^>]*aria-checked="true"|aria-checked="true"[^>]*data-start-choice="claude"/);
  assert.match(html, /Sign in with Claude<\/button>/);
});

test('Pick one to start: an engine that is not installed offers an Install link instead', () => {
  const html = m.picker({ engines: [engine('codex')], onSignedIn() {}, onKeyAdded() {}, initial: 'codex' });
  assert.match(html, /Codex isn’t on this computer yet/);
  assert.ok(html.includes(`href="${m.INSTALL_URL.codex}"`));
  const key = m.picker({ engines: [], onSignedIn() {}, onKeyAdded() {}, initial: 'key' });
  assert.match(key, /role="radio" aria-checked="true"[^>]*>Anthropic</);
  assert.match(key, /type="password"/);
});

const conn = (over = {}) => ({
  id: 'mcp:s1', kind: 'mcp', name: 'files', detail: '/usr/bin/python3', status: 'connected', status_detail: null,
  last_check: null, used_by: [], logins_stored: 'kwnt', manage: '/api/mcp/servers/s1', ...over,
});

test('a local MCP command waits behind Details; a remote host stays in view', () => {
  assert.ok(m.isRawCommand(conn()));
  assert.ok(m.isRawCommand(conn({ detail: 'npx' })));
  assert.ok(!m.isRawCommand(conn({ detail: 'api.githubcopilot.com' })));
  assert.ok(!m.isRawCommand(conn({ kind: 'email', detail: 'me@example.com' })));
  const html = m.row({ c: conn(), now: NOW });
  assert.match(html, /<details[^>]*>.*Details<\/summary><code[^>]*>\/usr\/bin\/python3<\/code><\/details>/s);
  const remote = m.row({ c: conn({ detail: 'api.githubcopilot.com' }), now: NOW });
  assert.doesNotMatch(remote, /<details/);
});

test('an account out of quota says until when, in local time', () => {
  const html = m.row({
    c: conn({ id: 'account:p1', kind: 'account', detail: null, status: 'limited', status_detail: 'Out of quota', limited_until: '2026-11-02T20:15:00Z' }),
    now: NOW,
  });
  assert.match(html, /Out of quota until \d{1,2}:15/);
});

test('an engine that is not installed folds away with no switch or "This machine" row', async () => {
  const { panel } = await loadBundled(`
    import React from 'react';
    import { renderToStaticMarkup } from 'react-dom/server.browser';
    import { EnginesPanel } from './src/components/EnginesPanel';
    export const panel = (engines, profiles) => renderToStaticMarkup(React.createElement(EnginesPanel, {
      api: { engines, error: null, rechecking: false, recheck() {}, setEnabled() {}, reload: async () => {} },
      accounts: { profiles, error: null, reload: async () => {} },
    }));
  `);
  const machine = (eng) => ({ id: `prf_${eng}`, engine: eng, name: 'This machine', kind: 'system', is_default: true, signed_in: false, has_key: null });
  const html = panel([engine('claude', { status: 'signed_out' }), engine('codex'), engine('grok')], [machine('claude'), machine('codex'), machine('grok')]);
  assert.match(html, /Pick one to start/);
  assert.match(html, /Not available on this computer \(2\)/);
  assert.doesNotMatch(html, /role="switch"[^>]*aria-labelledby="engine-/, 'no engine switches until something runs');
  assert.doesNotMatch(html, /data-engine="codex"/, 'folded until opened');
});
