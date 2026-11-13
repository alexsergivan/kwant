import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export { ModernApprovalCard, localShellView, startedByNote } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

// Shaped like approve_local_shell in src/server/computer.rs.
const shellTicket = (command, cwd = '/Users/alex/project') => ({
  id: 'a1',
  tool_name: 'execute_shell',
  details: { source: 'local_shell', tool: 'execute_shell', arguments: { command, cwd }, kwant_id: 'team-1' },
});
const render = (approval) =>
  m.renderToStaticMarkup(m.createElement(m.ModernApprovalCard, { approval, onResolve: () => {} }));

test('a local shell ticket shows the command and folder, without Always allow', () => {
  const html = render(shellTicket('ls -la && git status'));
  assert.match(html, /run a command on your computer/);
  assert.match(html, /<pre[^>]*whitespace-pre-wrap[^>]*>ls -la &amp;&amp; git status<\/pre>/);
  assert.match(html, /In folder:.*\/Users\/alex\/project/);
  assert.match(html, /Allow once/);
  assert.match(html, /Deny/);
  assert.doesNotMatch(html, /Always allow/);
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

test('a normal ticket keeps the raw details and Always allow', () => {
  const html = render({ id: 'n1', tool_name: 'write_file', details: { path: '/tmp/a.txt' } });
  assert.match(html, /write_file/);
  assert.match(html, /&quot;path&quot;: &quot;\/tmp\/a.txt&quot;/);
  assert.match(html, /Always allow/);
  assert.doesNotMatch(html, /run a command on your computer/);
});

// Shaped like BrokerApprover::classified and approve_local_shell_in with the payment floor (#110).
const stripeTicket = (extra = {}) => ({
  id: 'p1',
  tool_name: 'mcp__composio__COMPOSIO_MULTI_EXECUTE_TOOL',
  details: {
    source: 'mcp',
    server: 'composio',
    tool: 'COMPOSIO_MULTI_EXECUTE_TOOL',
    kwant_name: 'Ledger',
    arguments: { tools: [{ tool_slug: 'STRIPE_CREATE_PAYMENT_INTENT', arguments: { amount: 1200 } }] },
    payment_reason: 'composio STRIPE_CREATE_PAYMENT_INTENT is a write action of Stripe, a money app',
    ...extra,
  },
});

test('a payment card says why it is a payment and offers only Allow once and Deny', () => {
  const html = render(stripeTicket());
  assert.match(html, /This looks like a payment:<\/span> composio STRIPE_CREATE_PAYMENT_INTENT is a write action of Stripe, a money app\./);
  assert.match(html, /Payments always ask you, even with approvals off/);
  assert.match(html, /Allow once/);
  assert.match(html, /Deny/);
  assert.doesNotMatch(html, /Always allow|Always this session/);
  assert.match(html, /role="note"/);
});

test('a payment shown as a Claude tool card also drops Always this session', () => {
  const html = render({
    id: 'p2',
    tool_name: 'Bash',
    details: { source: 'claude_builtin', tool: 'Bash', arguments: { command: 'stripe charges create --amount 1' }, payment_reason: 'stripe CLI changes something.' },
  });
  assert.match(html, /This looks like a payment:<\/span> stripe CLI changes something\./, 'one full stop, not two');
  assert.doesNotMatch(html, /Always this session|Always allow/);
});

test('a payment by local shell keeps its command and shows the reason', () => {
  const t = shellTicket('curl -X POST https://api.stripe.com/v1/charges');
  t.details.payment_reason = 'POST to a payment host (api.stripe.com)';
  const html = render(t);
  assert.match(html, /This looks like a payment:<\/span> POST to a payment host \(api.stripe.com\)\./);
  assert.match(html, /run a command on your computer/);
  assert.doesNotMatch(html, /Always allow/);
});

test('cards without payment_reason are unchanged: no payment line, Always buttons as before', () => {
  for (const t of [
    { id: 'n2', tool_name: 'write_file', details: { path: '/tmp/a.txt' } },
    { id: 'n3', tool_name: 'Bash', details: { source: 'claude_builtin', tool: 'Bash', arguments: { command: 'ls' } } },
    stripeTicket({ payment_reason: undefined }),
    stripeTicket({ payment_reason: 42 }),
  ]) {
    const html = render(t);
    assert.doesNotMatch(html, /looks like a payment|role="note"/, t.id);
    assert.match(html, /Always allow|Always this session/, t.id);
  }
});

test('a still-open payment card keeps the reason and offers no always', () => {
  const html = render({ ...stripeTicket(), waiting: true });
  assert.match(html, /This looks like a payment/);
  assert.match(html, /Allow and continue/);
  assert.doesNotMatch(html, /Always/);
});

test('a payment card in its summary form (after a reload) shows the reason and no always', () => {
  const html = render({
    id: 'p3',
    tool_name: 'Bash',
    details: { source: 'summary', tool: 'Bash', summary: 'command: stripe charges create', kwant_name: 'Ledger', payment_reason: 'stripe CLI changes something' },
  });
  assert.match(html, /Ledger<\/strong> wants to run a command/);
  assert.match(html, /This looks like a payment:<\/span> stripe CLI changes something\./);
  assert.match(html, /Allow once/);
  assert.doesNotMatch(html, /Always this session|Always allow/);
  // The same summary card without a reason still offers Always this session.
  const plain = render({ id: 'n4', tool_name: 'Bash', details: { source: 'summary', tool: 'Bash', summary: 'command: ls' } });
  assert.match(plain, /Always this session/);
});

// Shaped like BrokerApprover::classified for a turn kwant started on its own (loose ends, Watch).
const startedTicket = (extra = {}) => ({
  id: 's1',
  tool_name: 'Bash',
  details: { source: 'claude_builtin', tool: 'Bash', arguments: { command: 'touch x' }, kwant_name: 'Lin', always_ask: true, ...extra },
});

test('a card from a turn kwant started says who started it, in one line under the summary', () => {
  const html = render(startedTicket({ started_by: 'Loose ends: prepare drafts and next steps' }));
  assert.match(
    html,
    /data-started-by[^>]*>Started by Loose ends, not by you, so each change asks first\. Bots on Don’t ask \(Settings › This space\) run these without a card\.<\/p>/,
  );
  // Read order: summary, then this line, then the command
  const summary = html.indexOf('data-approval-summary');
  const line = html.indexOf('data-started-by');
  const command = html.indexOf('aria-label="Command"');
  assert.ok(summary < line && line < command, 'summary → started by → command');
  assert.match(html, /data-started-by[^>]*class="mt-1 /);
  assert.doesNotMatch(render(startedTicket()), /data-started-by/, 'no label, no line');
  assert.equal(m.startedByNote(startedTicket({ started_by: '  ' })), null);
  assert.equal(m.startedByNote({ id: 'x', tool_name: 'Bash', details: { started_by: 3 } }), null);
  // A payment card keeps its own reason and doesn't add this line
  assert.doesNotMatch(render(stripeTicket({ always_ask: true, started_by: 'Loose ends' })), /data-started-by/);
});

test('internal labels map to the names people know; an unknown one shows the part before ":"', () => {
  const name = (started_by, extra = {}) => m.startedByNote(startedTicket({ started_by, ...extra }));
  assert.match(name('Verify'), /^Started by a task check, not by you, so each change asks first\./);
  assert.match(name('Audit'), /^Started by an audit, not by you/);
  assert.match(name('Goal draft'), /^Started by a goal draft, not by you/);
  assert.match(name('Watch'), /^Started by Watch, not by you/);
  assert.match(name('Something new: details'), /^Started by Something new, not by you, so each change asks first\./);
  assert.doesNotMatch(name('Loose ends: prepare drafts and next steps'), /prepare drafts/);
});

test('a strict turn (details.strict_ask) says skill drafts always ask, even on Don’t ask', () => {
  assert.equal(
    m.startedByNote(startedTicket({ started_by: 'Skill Foundry replay', strict_ask: true })),
    'Started by a skill draft’s test run, not by you. Skill drafts always ask, even on Don’t ask.',
  );
  assert.equal(
    m.startedByNote(startedTicket({ started_by: 'Skill Foundry', strict_ask: true })),
    'Started by a skill draft, not by you. Skill drafts always ask, even on Don’t ask.',
  );
  // The sentence comes from the server's flag, not from the label
  assert.match(m.startedByNote(startedTicket({ started_by: 'Skill Foundry' })), /so each change asks first/);
});

test('a card from a turn kwant started offers no "always"; an ordinary card still does', () => {
  const write = (extra = {}) => ({
    id: 'w1',
    tool_name: 'Write',
    details: { source: 'claude_builtin', tool: 'Write', arguments: { file_path: '/tmp/x', content: 'x' }, kwant_name: 'Lin', ...extra },
  });
  const plain = render(write());
  assert.match(plain, /Always this session|Always allow/);
  const started = render(write({ always_ask: true, started_by: 'Watch' }));
  assert.doesNotMatch(started, /Always this session|Always allow/);
  assert.match(started, /Allow once/);
  assert.match(started, />Deny</);
  assert.match(started, /grid-cols-1/);
  assert.doesNotMatch(render(startedTicket({ started_by: 'Watch' })), /Always this session|Always allow/);
});
