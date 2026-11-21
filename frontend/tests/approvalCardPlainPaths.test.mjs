// 33a approval-card-plain-paths (Mira's ux-pass-0937 minors b and c):
// (b) Show details names a folder under a home as ~/…, with the exact folder in `title`;
// (c) Allow once in the app leaves one quiet line in the chat, live and after a reload.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

// Loaded per test, so a missing module fails its own tests only (the red run on main).
const p = await loadBundled(`export { plainPath } from './src/paths';`).catch(() => ({}));
const a = await loadBundled(`export * from './src/approvals';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { ALLOWED_FROM_TELEGRAM, ALLOWED_IN_APP } from './src/components/TelegramMarks';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  export const card = (approval) => renderToStaticMarkup(React.createElement(ModernApprovalCard, { approval, onResolve: () => {} }));
`);

test('plainPath: a folder under a home reads ~/…, anything else is unchanged', () => {
  assert.equal(p.plainPath('/home/ann/.kwnt/bot-workspaces/kw_1'), '~/.kwnt/bot-workspaces/kw_1');
  assert.equal(p.plainPath('/Users/ann/Desktop'), '~/Desktop');
  assert.equal(p.plainPath('C:\\Users\\ann\\Desktop'), '~\\Desktop', 'Windows keeps its own slash');
  assert.equal(p.plainPath('d:/Users/ann/Desktop/x'), '~/Desktop/x', 'any drive letter, either slash');
  assert.equal(p.plainPath('/home/ann'), '~');
  assert.equal(p.plainPath('/home/ann/'), '~/');
  assert.equal(p.plainPath('C:\\Users\\ann'), '~');
  for (const same of ['/tmp/x', '/root/x', '/homeless/x', '/home', '/home/', '/var/lib/kwnt', 'relative/home/ann', 'C:\\Program Files\\x', '']) {
    assert.equal(p.plainPath(same), same, same);
  }
});

const titled = (html, full, shown) => {
  const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  assert.match(html, new RegExp(`<span[^>]*title="${esc(full)}"[^>]*>${esc(shown)}</span>`), html);
};

test("the card's details show ~/… and keep the full folder in title (kwant shell, Claude Bash, file tool)", () => {
  const shell = ui.card({ id: 's1', tool_name: 'execute_shell', details: { source: 'local_shell', arguments: { command: 'ls', cwd: '/home/ann/.kwnt/bot-workspaces/kw_1' } } });
  assert.match(shell, /In folder:/);
  titled(shell, '/home/ann/.kwnt/bot-workspaces/kw_1', '~/.kwnt/bot-workspaces/kw_1');

  const bash = ui.card({ id: 'b1', tool_name: 'Bash', details: { source: 'claude_builtin', tool: 'Bash', arguments: { command: 'touch ~/Desktop/a' }, cwd: '/Users/ann/Desktop' } });
  assert.match(bash, /In folder:/);
  titled(bash, '/Users/ann/Desktop', '~/Desktop');

  const file = ui.card({ id: 'w1', tool_name: 'Write', details: { source: 'claude_builtin', tool: 'Write', arguments: { file_path: '/tmp/x', content: 'x' }, cwd: 'C:\\Users\\ann\\proj' } });
  assert.match(file, /Its folder:/);
  titled(file, 'C:\\Users\\ann\\proj', '~\\proj');

  // A folder outside any home is shown as it is.
  const tmp = ui.card({ id: 's2', tool_name: 'execute_shell', details: { source: 'local_shell', arguments: { command: 'ls', cwd: '/tmp/x' } } });
  titled(tmp, '/tmp/x', '/tmp/x');
  // The command itself is untouched.
  assert.match(bash, /touch ~\/Desktop\/a/);
});

const now = Date.parse('2026-11-21T18:00:00Z');
const thread = { thread_id: 'th-1', channel_id: null, direct_kwant_id: null };
const row = (over = {}) => ({
  id: 'ap_app', kwant_id: 'k_drafter', tool_name: 'Bash', status: 'allowed_once', answered_via: null,
  created_at: '2026-11-21T17:00:00Z', details: { kwant_name: 'Drafter' }, link: null, ...over,
});

test('after a reload: an app allowed_once row from the last day comes back, the Telegram one still does', () => {
  const rows = [ // newest first
    row({ id: 'ap_tg', answered_via: 'telegram', created_at: '2026-11-21T17:30:00Z' }),
    row({ id: 'ap_app' }),
    row({ id: 'ap_undef', answered_via: undefined, created_at: '2026-11-21T16:00:00Z', link: thread }),
    row({ id: 'ap_old', created_at: '2026-11-20T17:00:00Z' }),
    row({ id: 'ap_denied', status: 'denied' }),
    row({ id: 'ap_always', status: 'always_allowed' }),
    row({ id: 'ap_nobot', kwant_id: null, details: {} }),
  ];
  const settled = a.settledFromRecords(rows, now);
  assert.deepEqual(settled.map((r) => r.id), ['ap_undef', 'ap_app', 'ap_tg'], 'oldest first; old, denied, always and botless dropped');
  assert.equal(settled.find((r) => r.id === 'ap_app').via, 'app');
  assert.equal(settled.find((r) => r.id === 'ap_undef').via, 'app', 'no answered_via reads as the app');
  assert.equal(settled.find((r) => r.id === 'ap_tg').via, undefined, 'Telegram unchanged');
  assert.equal(settled.find((r) => r.id === 'ap_tg').link, null, 'Telegram keeps no link, as before');
  assert.deepEqual(settled.find((r) => r.id === 'ap_undef').link, thread, 'an app row stays in the chat it was asked in');
  // The thread-linked row is not drawn in the direct chat.
  assert.deepEqual(a.approvalsForChat(settled, { kind: 'kwant', kwantId: 'k_drafter', threadId: null }).map((r) => r.id), ['ap_app', 'ap_tg']);
});

const chatProps = (settledApprovals) => ({
  title: 'Drafter', subtitle: '', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], settledApprovals, onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
});

test('Allow once in the app adds one "✓ You allowed once" line in that chat and none in another', () => {
  const live = { id: 'ap_live', tool_name: 'Bash', details: { kwant_name: 'Drafter', kwant_id: 'k_drafter' }, kwant_id: 'k_drafter', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k_drafter' }, asked_at: '2026-11-21T17:59:00Z' };
  assert.deepEqual(a.keepAllowedOnce([], live, 'deny'), []);
  assert.deepEqual(a.keepAllowedOnce([], live, 'always_allow'), []);
  assert.deepEqual(a.keepAllowedOnce([], undefined, 'allow_once'), []);
  const once = a.keepAllowedOnce([], live, 'allow_once');
  assert.equal(once.length, 1);
  assert.equal(once[0].via, 'app');
  assert.equal(a.keepAllowedOnce(once, live, 'allow_once').length, 1, 'once only');
  // The reload's row for the same card is not drawn twice.
  assert.equal(a.addSettled(once, a.settledFromRecords([row({ id: 'ap_live' })], now)).length, 1);

  const here = a.approvalsForChat(once, { kind: 'kwant', kwantId: 'k_drafter', threadId: null });
  assert.equal(here.length, 1);
  assert.deepEqual(a.approvalsForChat(once, { kind: 'kwant', kwantId: 'k_other', threadId: null }), []);

  assert.equal(ui.ALLOWED_IN_APP, '✓ You allowed once');
  assert.equal(ui.ALLOWED_FROM_TELEGRAM, '✓ Allowed once from Telegram');
  const html = ui.chat(chatProps(here));
  assert.equal((html.match(/data-resolved-via="app"/g) ?? []).length, 1);
  assert.match(html, /Drafter: Bash · <b[^>]*>✓ You allowed once<\/b>/);
  assert.doesNotMatch(html, /Allowed once from Telegram/);
  assert.doesNotMatch(html.slice(html.indexOf('data-resolved-via="app"')), /^[^]*?<button[^>]*>[^<]*(Allow|Deny)/, 'nothing to answer');

  // Telegram's line is unchanged beside it.
  const tg = a.settledFromRecords([row({ id: 'ap_tg', answered_via: 'telegram' })], now);
  const both = ui.chat(chatProps([...tg, ...here]));
  assert.match(both, /data-resolved-via="telegram"[^]*Drafter: Bash · <b[^>]*>✓ Allowed once from Telegram<\/b>/);
  assert.equal((both.match(/data-resolved-via="app"/g) ?? []).length, 1);

  // App.tsx keeps the line when the app's Allow once succeeds.
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const resolve = app.slice(app.indexOf('const handleResolveApproval'), app.indexOf('const handleSelectTeam'));
  assert.match(resolve, /if \(res\.ok\) setSettledApprovals\(\(prev\) => keepAllowedOnce\(prev, card, decision\)\)/);
});
