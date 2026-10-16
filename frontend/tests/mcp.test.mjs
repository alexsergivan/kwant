import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../src/mcp.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext },
});
const mcp = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
const {
  MCP_PRESETS,
  SECRET_MASK,
  emptyMcpForm,
  formToPayload,
  serverToForm,
  splitCommandLine,
  joinCommandLine,
  validateMcpForm,
  toServerName,
  parseMcpToolName,
  parseToolCalls,
  formatArgs,
  mcpSupportFor,
} = mcp;

test('splits command lines like a README shows them', () => {
  assert.deepEqual(splitCommandLine('npx -y @scope/server /home/me'), ['npx', '-y', '@scope/server', '/home/me']);
  assert.deepEqual(splitCommandLine(`run "my folder" 'it''s' a\\ b ""`), ['run', 'my folder', 'its', 'a b', '']);
  assert.deepEqual(splitCommandLine('   '), []);
});

test('joining and splitting round-trips awkward arguments', () => {
  const words = ['kwnt', 'mcp-echo-server', 'with space', 'quo"te', 'back\\slash', ''];
  assert.deepEqual(splitCommandLine(joinCommandLine(words)), words);
  assert.equal(joinCommandLine(['npx', '-y', 'pkg']), 'npx -y pkg');
});

test('validates names, command, URL and timeout with friendly messages', () => {
  const ok = { ...emptyMcpForm(), name: 'echo', commandLine: 'kwnt mcp-echo-server' };
  assert.equal(validateMcpForm(ok), null);
  assert.match(validateMcpForm({ ...ok, name: '' }), /name/);
  assert.match(validateMcpForm({ ...ok, name: 'has space' }), /letters/);
  assert.match(validateMcpForm({ ...ok, name: 'a__b' }), /letters/);
  assert.match(validateMcpForm({ ...ok, name: 'KWNT' }), /reserved/);
  assert.match(validateMcpForm({ ...ok, commandLine: ' ' }), /command/);
  assert.match(validateMcpForm({ ...ok, env: [{ key: '1BAD', value: 'x', saved: false }] }), /variable/);
  const http = { ...emptyMcpForm('http'), name: 'remote', url: 'ftp://x' };
  assert.match(validateMcpForm(http), /http/);
  assert.equal(validateMcpForm({ ...http, url: 'https://example.com/mcp' }), null);
  assert.match(validateMcpForm({ ...ok, timeout_secs: 0 }), /Timeout/);
  assert.equal(toServerName('  My Files! '), 'my-files');
});

test('stdio payload splits the command and keeps blank saved secrets as the mask', () => {
  const payload = formToPayload({
    ...emptyMcpForm(),
    name: ' gh ',
    commandLine: 'npx -y "@x/server github"',
    env: [
      { key: 'KEEP', value: '', saved: true },
      { key: 'CHANGE', value: 'new', saved: true },
      { key: 'NEW', value: 'v', saved: false },
      { key: '  ', value: 'ignored', saved: false },
    ],
  });
  assert.equal(payload.name, 'gh');
  assert.equal(payload.command, 'npx');
  assert.deepEqual(payload.args, ['-y', '@x/server github']);
  assert.equal(payload.url, null);
  assert.deepEqual(payload.env, { KEEP: SECRET_MASK, CHANGE: 'new', NEW: 'v' });
  assert.deepEqual(payload.headers, {});
});

test('http payload sends only the URL and headers', () => {
  const payload = formToPayload({
    ...emptyMcpForm('http'),
    name: 'remote',
    commandLine: 'left over',
    url: ' https://example.com/mcp ',
    env: [{ key: 'X', value: 'y', saved: false }],
    headers: [{ key: 'Authorization', value: '', saved: true }],
  });
  assert.equal(payload.command, null);
  assert.deepEqual(payload.args, []);
  assert.equal(payload.url, 'https://example.com/mcp');
  assert.deepEqual(payload.env, {});
  assert.deepEqual(payload.headers, { Authorization: SECRET_MASK });
});

test('editing a server never puts the mask in an input, and saving unchanged keeps every secret', () => {
  const server = {
    id: '1', name: 'gh', transport: 'stdio', command: 'npx', args: ['-y', 'a b'], url: null,
    env: { TOKEN: SECRET_MASK }, headers: {}, enabled: false, timeout_secs: 12, created_at: null,
  };
  const form = serverToForm(server);
  assert.equal(form.commandLine, 'npx -y "a b"');
  assert.deepEqual(form.env, [{ key: 'TOKEN', value: '', saved: true }]);
  assert.equal(form.enabled, false);
  assert.equal(form.timeout_secs, 12);
  const payload = formToPayload(form);
  assert.deepEqual(payload.env, { TOKEN: SECRET_MASK });
  assert.deepEqual([payload.command, ...payload.args], ['npx', '-y', 'a b']);
});

test('presets are valid forms with distinct ids', () => {
  assert.ok(MCP_PRESETS.length >= 2);
  assert.equal(new Set(MCP_PRESETS.map((p) => p.id)).size, MCP_PRESETS.length);
  for (const p of MCP_PRESETS) assert.equal(validateMcpForm(p.form), null, p.id);
});

test('reads MCP tool names and recorded tool calls', () => {
  assert.deepEqual(parseMcpToolName('mcp__echo__echo'), { server: 'echo', tool: 'echo' });
  assert.deepEqual(parseMcpToolName('mcp__my-srv__read__file'), { server: 'my-srv', tool: 'read__file' });
  assert.equal(parseMcpToolName('Bash'), null);

  const raw = JSON.stringify([
    { id: 'c1', name: 'mcp__echo__add', arguments: { a: 40, b: 2 }, status: 'ok', result: '42', duration_ms: 3 },
    { name: 'mcp__echo__echo', server: 'echo', tool: 'echo', status: 'denied' },
    { nope: true },
  ]);
  const calls = parseToolCalls(raw);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].server, 'echo');
  assert.equal(calls[0].tool, 'add');
  assert.equal(calls[1].status, 'denied');
  assert.deepEqual(parseToolCalls(null), []);
  assert.deepEqual(parseToolCalls('not json'), []);
  assert.equal(parseToolCalls([{ name: 'x' }]).length, 1);
});

test('formats tool arguments for people', () => {
  assert.equal(formatArgs({ text: 'hi' }), '{\n  "text": "hi"\n}');
  assert.equal(formatArgs('{"a":1}'), '{\n  "a": 1\n}');
  assert.equal(formatArgs('plain'), 'plain');
  assert.equal(formatArgs({}), '');
  assert.equal(formatArgs(undefined), '');
});

test('says honestly which engines can use MCP tools', () => {
  const none = { anthropic: false, openai: false, xai: false };
  assert.equal(mcpSupportFor('claude', none).supported, true);
  assert.equal(mcpSupportFor('claude', { ...none, anthropic: true }).supported, false);
  assert.equal(mcpSupportFor('codex', none).supported, false);
  assert.equal(mcpSupportFor('codex', { ...none, openai: true }).supported, true);
  assert.equal(mcpSupportFor('grok', none).supported, false);
  assert.equal(mcpSupportFor('grok', { ...none, xai: true }).supported, true);
  assert.deepEqual(mcpSupportFor('local_ollama', none), { supported: true, note: null });
  const auto = mcpSupportFor('auto', none);
  assert.equal(auto.supported, true);
  assert.match(auto.note, /Laya/);
});
