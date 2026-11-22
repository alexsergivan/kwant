// 33u tool-labels-agree (autonomy/tool-labels-agree/brief.md): the in-app card and working strip (toolLabel.ts) and
// the push, Telegram and restart notes (src/server/tool_words.rs) name the same step in the same words. Both sides
// walk the one list in tests/fixtures/tool_words.json; tool_words.rs `the_shared_list_reads_the_same_here_as_in_the_app`
// is the Rust half. Tests marked "fails on 51d5874d" fail there (Read/Write/Edit/MultiEdit and agent_browser_*).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export { toolLabel, toolAction, readableStepTitle } from './src/toolLabel';`);
const cases = JSON.parse(readFileSync(new URL('../../tests/fixtures/tool_words.json', import.meta.url), 'utf8'));

test('the shared list reads the same in the app as on the server (fails on 51d5874d)', () => {
  assert.ok(cases.length >= 30, String(cases.length));
  for (const c of cases) {
    const at = `${c.tool} ${c.server ?? ''}`;
    assert.equal(m.toolLabel(c.tool, c.server ?? null), c.ing, at);
    assert.equal(m.toolAction(c.tool, c.server ?? null), c.base, at);
  }
});

test("Claude's built-in file tools read as what they do on the card (fails on 51d5874d)", () => {
  assert.equal(m.toolAction('Write'), 'write a file');
  assert.equal(m.toolAction('Read'), 'read a file');
  assert.equal(m.toolAction('Edit'), 'change a file');
  assert.equal(m.toolAction('MultiEdit'), 'change a file');
  assert.equal(m.readableStepTitle('Using Read'), 'Reading a file');
  assert.equal(m.readableStepTitle('Waiting for your approval: Using Write'), 'Waiting for your OK to write a file');
});

test('a browser step never shows its raw agent_browser name (fails on 51d5874d)', () => {
  for (const x of ['snapshot', 'click', 'open', 'get_text', 'tab_new', 'wait_for_text', 'something_new']) {
    const words = m.toolLabel(`mcp__browser__agent_browser_${x}`);
    assert.ok(!/agent|_/.test(words), `${x}: ${words}`);
  }
  assert.equal(m.readableStepTitle('Using agent_browser_snapshot (browser)'), 'Reading the browser page');
});

test("guard: an MCP server's own read/write keeps its name, and other words are unchanged", () => {
  assert.equal(m.toolLabel('mcp__fs__read'), 'using read (fs)');
  assert.equal(m.toolLabel('write', 'fs'), 'using write (fs)');
  assert.equal(m.toolLabel('mcp__github__create_pull_request'), 'opening a pull request on GitHub');
  assert.equal(m.toolLabel('bash'), 'running a command');
  assert.equal(m.toolLabel('browser_snapshot'), 'reading the browser page');
  assert.equal(m.toolAction('mcp__srv__foo_bar'), 'use foo bar (srv)');
});

test('a tool named like an object key reads as its name, never "undefined" (fails on 51d5874d)', () => {
  assert.equal(m.toolLabel('constructor'), 'using constructor');
  assert.equal(m.toolAction('toString'), 'use toString');
});
