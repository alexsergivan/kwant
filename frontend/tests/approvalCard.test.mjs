import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { ModernApprovalCard, localShellView } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

// Shaped like approve_local_shell in src/server/computer.rs.
const shellTicket = (command, cwd = '/Users/oleks/project') => ({
  id: 'a1',
  tool_name: 'execute_shell',
  details: { source: 'local_shell', tool: 'execute_shell', arguments: { command, cwd }, kwant_id: 'team-1' },
});
const render = (approval) =>
  m.renderToStaticMarkup(m.createElement(m.ModernApprovalCard, { approval, onResolve: () => {} }));

test('a local shell ticket shows the command and folder, without Always Allow', () => {
  const html = render(shellTicket('ls -la && git status'));
  assert.match(html, /run a command on your computer/);
  assert.match(html, /<pre[^>]*whitespace-pre-wrap[^>]*>ls -la &amp;&amp; git status<\/pre>/);
  assert.match(html, /In folder:.*\/Users\/oleks\/project/);
  assert.match(html, /Allow Once/);
  assert.match(html, /Deny/);
  assert.doesNotMatch(html, /Always Allow/);
  assert.doesNotMatch(html, /kwant_id|&quot;source&quot;/, 'no raw JSON dump');
});

test('a very long command is shown in full, scrollable rather than clipped', () => {
  const long = `echo ${'x'.repeat(5000)} | wc -c`;
  const html = render(shellTicket(long));
  assert.ok(html.includes(long), 'whole command is in the markup');
  assert.match(html, /<pre[^>]*overflow-y-auto/);
  assert.doesNotMatch(html, /<pre[^>]*overflow-hidden/);
});

test('localShellView tolerates missing fields and ignores other tools', () => {
  assert.deepEqual(m.localShellView({ id: 'x', tool_name: 'execute_shell', details: { source: 'local_shell' } }), { command: '', cwd: null });
  assert.equal(m.localShellView({ id: 'x', tool_name: 'write_file', details: { path: '/tmp/a' } }), null);
  assert.equal(m.localShellView({ id: 'x', tool_name: 'execute_shell', details: null }), null);
  const html = render({ id: 'x', tool_name: 'execute_shell', details: { source: 'local_shell', arguments: { command: 'pwd' } } });
  assert.doesNotMatch(html, /In folder:/);
});

test('a normal ticket keeps the raw details and Always Allow', () => {
  const html = render({ id: 'n1', tool_name: 'write_file', details: { path: '/tmp/a.txt' } });
  assert.match(html, /Tool:<\/span> write_file/);
  assert.match(html, /&quot;path&quot;: &quot;\/tmp\/a.txt&quot;/);
  assert.match(html, /Always Allow/);
  assert.doesNotMatch(html, /run a command on your computer/);
});
