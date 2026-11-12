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
