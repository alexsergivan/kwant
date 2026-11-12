// 0.9.5: the chat line when a bot's connection fails (release-095/tool-failure-line/brief.md).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/toolFailures';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ToolFailureLine } from './src/components/ToolFailureLine';
  export const line = (props) => renderToStaticMarkup(React.createElement(ToolFailureLine, props));
`);

// Shaped like the meta src/server/tool_failures.rs writes.
const meta = (over = {}) => ({
  kind: 'tool_failure', kwant_id: 'k1', turn_id: 't1', connection: 'Gmail', key: 'composio:gmail', server_id: 's1',
  cause: 'reconnect', tool: 'composio · GMAIL_SEND_EMAIL', detail: 'Connected account not found',
  line: 'Gmail stopped working for Ben: it needs to be connected again.', needs_you: true,
  fix: { label: 'Reconnect', opens: 'connections', connection_id: 'mcp:s1' }, resolved_at: null, ...over,
});

test('the meta is read from the saved string, and defensively', () => {
  const v = m.parseToolFailureMeta(JSON.stringify(meta()));
  assert.equal(v.connection, 'Gmail');
  assert.equal(v.cause, 'reconnect');
  assert.deepEqual(v.fix, { label: 'Reconnect', connection_id: 'mcp:s1' });
  assert.equal(v.needs_you, true);
  assert.equal(m.parseToolFailureMeta(meta({ cause: 'weird' })).cause, 'down');
  assert.equal(m.parseToolFailureMeta(meta({ fix: null })).fix, null);
  assert.equal(m.parseToolFailureMeta(meta({ line: '' })), null);
  assert.equal(m.parseToolFailureMeta({ ...meta(), kind: 'tool_note' }), null);
  assert.equal(m.parseToolFailureMeta('not json'), null);
});

test('the line says what broke and has a fix button that opens Connections', () => {
  const html = ui.line({ meta: m.parseToolFailureMeta(meta()), messageId: 'm1', onFix: () => {} });
  assert.match(html, /Gmail stopped working for Ben: it needs to be connected again\./);
  assert.match(html, /<button[^>]*>Reconnect<\/button>/);
  assert.match(html, /data-tool-failure-line="reconnect"/);
  assert.match(html, /text-amber-400/);
  // F2: an MCP server that refused its sign-in says who acts.
  const signIn = meta({ connection: 'Notion', key: 'mcp:s1', cause: 'sign_in', tool: 'Notion · search', detail: '401 Unauthorized', line: 'Notion stopped working for Ben: it needs you to sign in again.' });
  const html2 = ui.line({ meta: m.parseToolFailureMeta(signIn), messageId: 'm2', onFix: () => {} });
  assert.match(html2, /Notion stopped working for Ben: it needs you to sign in again\./);
  assert.match(html2, /data-tool-failure-line="sign_in"/);
  assert.match(html2, /<button[^>]*>Reconnect<\/button>/);
});

test('F3: a needs-you line is brighter, its button amber, and a screen reader hears "Needs you"', () => {
  const urgent = ui.line({ meta: m.parseToolFailureMeta(meta()), onFix: () => {} });
  assert.match(urgent, /^<div[^>]*text-zinc-300/);
  assert.match(urgent, /<p[^>]*><span class="sr-only">Needs you: <\/span>Gmail stopped/);
  assert.match(urgent, /<button[^>]*text-amber-200[^>]*>Reconnect<\/button>/);
  assert.match(urgent, /<button[^>]*font-medium/);
  assert.match(urgent, /<button[^>]*focus-visible:outline-sky-500/);
  const grey = ui.line({
    meta: m.parseToolFailureMeta(meta({ cause: 'down', needs_you: false, fix: { label: 'Check it', connection_id: 'mcp:s1' }, line: 'GitHub stopped responding for Ben.' })),
    onFix: () => {},
  });
  assert.match(grey, /^<div[^>]*text-zinc-400/);
  assert.doesNotMatch(grey, /sr-only|text-amber/);
  assert.match(grey, /<button[^>]*text-zinc-200[^>]*>Check it<\/button>/);
});

test('F4: a resolved line ends in a green "It works again." with a plugged icon, never amber', () => {
  const fixed = ui.line({ meta: m.parseToolFailureMeta(meta({ resolved_at: '2026-11-12T18:00:00Z' })), onFix: () => {} });
  assert.match(fixed, /^<div[^>]*text-zinc-400/);
  assert.match(fixed, /<span class="text-emerald-400"> It works again\.<\/span>/);
  assert.match(fixed, /lucide-plug[^-]/);
  assert.doesNotMatch(fixed, /lucide-unplug/);
  assert.match(fixed, /<svg[^>]*text-zinc-500/);
  assert.doesNotMatch(fixed, /amber|sr-only/);
});

test('F5 and F6: a line under another sits 4 px close; the button has an invisible 44 px hit area', () => {
  const props = { meta: m.parseToolFailureMeta(meta()), onFix: () => {} };
  assert.match(ui.line({ ...props, tight: true }), /^<div[^>]*!mt-1/);
  assert.doesNotMatch(ui.line(props), /!mt-1/);
  assert.match(ui.line(props), /<button[^>]*relative before:absolute before:content-\[(?:''|&#x27;&#x27;)\] before:-inset-x-2 before:-inset-y-\[14px\]/);
  // The phones' 44 px min-height would make the button's line of text twice as tall: it opts out
  assert.match(ui.line(props), /<button[^>]*data-compact/);
});

test('no button without a place to fix it; a fixed connection says it works again', () => {
  const own = m.parseToolFailureMeta(meta({ connection: 'kwant’s own tools', cause: 'down', fix: null, needs_you: false, line: 'kwant’s own tools stopped responding for Ben.' }));
  assert.doesNotMatch(ui.line({ meta: own, onFix: () => {} }), /<button/);
  assert.doesNotMatch(ui.line({ meta: m.parseToolFailureMeta(meta()) }), /<button/, 'no handler, no button');
  const fixed = ui.line({ meta: m.parseToolFailureMeta(meta({ resolved_at: '2026-11-12T18:00:00Z' })), onFix: () => {} });
  assert.match(fixed, /It works again\./);
  assert.doesNotMatch(fixed, /<button/);
  assert.doesNotMatch(fixed, /text-amber-400/);
});
