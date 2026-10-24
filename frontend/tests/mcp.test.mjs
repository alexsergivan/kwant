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
  MCP_PRESET_CATEGORIES,
  searchPresets,
  groupPresets,
  COMPOSIO_ONLY_APPS,
  searchComposioOnly,
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
  toStoredStatus,
  sanitizeStatuses,
  loadStatuses,
  saveStatuses,
  serverStatusKind,
  isSignedIn,
  readOAuthMessage,
  kwantUsage,
  defaultKwantServerIds,
  isBlankRequired,
  blankRequiredKeys,
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

test('presets are valid forms with distinct ids and names', () => {
  assert.ok(MCP_PRESETS.length >= 12);
  assert.equal(new Set(MCP_PRESETS.map((p) => p.id)).size, MCP_PRESETS.length);
  assert.equal(new Set(MCP_PRESETS.map((p) => p.form.name)).size, MCP_PRESETS.length);
  assert.equal(new Set(MCP_PRESETS.map((p) => p.label)).size, MCP_PRESETS.length);
  const categories = new Set(MCP_PRESET_CATEGORIES.map((c) => c.id));
  for (const p of MCP_PRESETS) {
    assert.equal(validateMcpForm(p.form), null, p.id);
    assert.ok(categories.has(p.category), `${p.id} category`);
    assert.match(p.source, /^https:\/\//, `${p.id} source`);
    assert.ok(p.blurb.length > 0 && p.blurb.length <= 100, `${p.id} blurb`);
    // Secrets start blank: the user brings their own.
    for (const row of [...p.form.env, ...p.form.headers]) {
      assert.equal(row.value, '', `${p.id} ${row.key}`);
      assert.equal(row.saved, false, `${p.id} ${row.key}`);
    }
    // URL servers are https and keep their keys in headers, never in the address.
    if (p.form.transport === 'http') assert.match(p.form.url, /^https:\/\/[^?#]+$/, p.id);
    // What a preset needs matches how it runs.
    const cmd = splitCommandLine(p.form.commandLine)[0];
    if (cmd === 'npx') assert.ok(p.needs.includes('Node.js'), p.id);
    if (cmd === 'uvx') assert.ok(p.needs.includes('uv'), p.id);
    if (p.needs.includes('Sign-in')) assert.equal(p.form.transport, 'http', p.id);
  }
});

test('presets avoid deprecated reference servers', () => {
  const deprecated = ['server-github', 'server-brave-search', 'server-postgres', 'server-slack', 'mcp-server-sqlite'];
  for (const p of MCP_PRESETS) {
    for (const d of deprecated) assert.ok(!p.form.commandLine.includes(d), `${p.id} uses ${d}`);
  }
});

test('a preset form copies into the payload kwnt expects', () => {
  const github = MCP_PRESETS.find((p) => p.id === 'github');
  const payload = formToPayload({ ...github.form, headers: [{ ...github.form.headers[0], value: 'Bearer t' }] });
  assert.deepEqual(payload.headers, { Authorization: 'Bearer t' });
  assert.equal(payload.url, 'https://api.githubcopilot.com/mcp/');
  const pg = formToPayload(MCP_PRESETS.find((p) => p.id === 'postgres').form);
  assert.deepEqual([pg.command, ...pg.args], ['uvx', 'postgres-mcp', '--access-mode=restricted']);
  assert.deepEqual(Object.keys(pg.env), ['DATABASE_URI']);
});

test('searches presets by name, blurb, group and needs', () => {
  assert.equal(searchPresets('').length, MCP_PRESETS.length);
  assert.equal(searchPresets('   ').length, MCP_PRESETS.length);
  assert.deepEqual(searchPresets('NOTION').map((p) => p.id), ['notion']);
  assert.ok(searchPresets('database').some((p) => p.id === 'postgres'));
  assert.ok(searchPresets('sign-in').every((p) => p.needs.includes('Sign-in')));
  assert.ok(searchPresets('dev tools').some((p) => p.id === 'sentry'));
  assert.deepEqual(searchPresets('zzz-nothing'), []);
});

test('groups presets in category order and drops empty groups', () => {
  const groups = groupPresets(MCP_PRESETS);
  assert.deepEqual(groups.map((g) => g.id), MCP_PRESET_CATEGORIES.map((c) => c.id));
  assert.equal(groups.reduce((n, g) => n + g.presets.length, 0), MCP_PRESETS.length);
  assert.deepEqual(groupPresets(searchPresets('notion')).map((g) => g.id), ['productivity']);
  assert.deepEqual(groupPresets([]), []);
  assert.ok(groups.every((g) => Array.isArray(g.composio) && g.composio.length === 0));
});

test('apps without a usable official server point to Composio instead of an invented preset', () => {
  const presetIds = new Set(MCP_PRESETS.map((p) => p.id));
  const ids = COMPOSIO_ONLY_APPS.map((a) => a.id);
  assert.equal(new Set(ids).size, ids.length);
  for (const a of COMPOSIO_ONLY_APPS) {
    assert.ok(!presetIds.has(a.id), `${a.id} is both a preset and Composio-only`);
    assert.match(a.toolkit, /^[a-z0-9_-]+$/, a.id);
    assert.match(a.source, /^https:\/\/docs\.composio\.dev\/toolkits\//, a.id);
    assert.ok(a.why.length > 10 && a.blurb.length <= 100, a.id);
    assert.ok(MCP_PRESET_CATEGORIES.some((c) => c.id === a.category), a.id);
  }
  assert.ok(ids.includes('gmail'));
});

test('the asked-for apps are offered: Cloudflare and Picsart as presets, Gmail via Composio', () => {
  for (const id of ['cloudflare', 'picsart']) {
    const p = MCP_PRESETS.find((x) => x.id === id);
    assert.ok(p && p.needs.includes('Sign-in') && p.form.transport === 'http', id);
    assert.match(p.source, /^https:\/\//, id);
  }
  assert.deepEqual(searchComposioOnly('gmail').map((a) => a.id), ['gmail']);
  assert.deepEqual(searchPresets('gmail'), []);
});

test('search finds Composio-only apps, and groups put them with their category', () => {
  assert.deepEqual(searchComposioOnly('').length, COMPOSIO_ONLY_APPS.length);
  assert.ok(searchComposioOnly('composio').length === COMPOSIO_ONLY_APPS.length);
  assert.deepEqual(searchComposioOnly('googledrive').map((a) => a.id), ['google-drive']);
  assert.deepEqual(searchComposioOnly('zzz-nothing'), []);
  const groups = groupPresets(searchPresets('figma'), searchComposioOnly('figma'));
  assert.deepEqual(groups.map((g) => [g.id, g.composio.map((a) => a.id)]), [['creative', ['figma']]]);
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

const memoryStorage = (initial = {}) => {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => (k in data ? data[k] : null),
    setItem: (k, v) => { data[k] = String(v); },
    removeItem: (k) => { delete data[k]; },
  };
};

test('stored test status keeps only ok, tool names and time, never error text', () => {
  const failed = toStoredStatus({ ok: false, error: 'bad token SECRET-123', stderr_tail: 'SECRET-123', tools: undefined }, '2026-10-16T00:00:00Z');
  assert.deepEqual(failed, { ok: false, tools: [], at: '2026-10-16T00:00:00Z' });
  const ok = toStoredStatus({ ok: true, tools: [{ name: 'echo', description: 'x' }, { name: 'add' }] }, 't');
  assert.deepEqual(ok, { ok: true, tools: ['echo', 'add'], at: 't' });

  globalThis.localStorage = memoryStorage();
  saveStatuses({ a: { ok: false, tools: [], at: 't', error: 'SECRET-123' } });
  assert.deepEqual(JSON.parse(localStorage.data['kwnt.mcp.lastTest']), { a: { ok: false, tools: [], at: 't' } });
  assert.ok(!localStorage.data['kwnt.mcp.lastTest'].includes('SECRET'));
});

test('old stored statuses with error text are cleaned on load and rewritten', () => {
  const old = {
    a: { ok: false, tools: [], error: 'Authorization: Bearer SECRET-456', at: 't1' },
    b: { ok: true, tools: ['echo', 7], at: 't2' },
    junk: 'nope',
    noOk: { tools: ['x'] },
  };
  globalThis.localStorage = memoryStorage({ 'kwnt.mcp.lastTest': JSON.stringify(old) });
  const loaded = loadStatuses();
  assert.deepEqual(loaded, { a: { ok: false, tools: [], at: 't1' }, b: { ok: true, tools: ['echo'], at: 't2' } });
  assert.ok(!localStorage.data['kwnt.mcp.lastTest'].includes('SECRET'));
  assert.deepEqual(JSON.parse(localStorage.data['kwnt.mcp.lastTest']), loaded);

  globalThis.localStorage = memoryStorage({ 'kwnt.mcp.lastTest': '{not json' });
  assert.deepEqual(loadStatuses(), {});
  assert.equal(localStorage.data['kwnt.mcp.lastTest'], undefined);

  assert.deepEqual(sanitizeStatuses(null), {});
  assert.deepEqual(sanitizeStatuses([{ ok: true }]), {});
  delete globalThis.localStorage;
});

test('maps a server and its last test to what the card shows', () => {
  const http = { enabled: true, transport: 'http' };
  const stdio = { enabled: true, transport: 'stdio' };
  assert.equal(serverStatusKind(http, undefined), 'untested');
  assert.equal(serverStatusKind(http, { ok: true }), 'ok');
  assert.equal(serverStatusKind(http, { ok: false, auth_required: true }), 'needs_sign_in');
  assert.equal(serverStatusKind(http, { ok: false }), 'error', 'a URL server that is just down is not a sign-in problem');
  assert.equal(serverStatusKind(stdio, { ok: false, auth_required: true }), 'error');
  assert.equal(serverStatusKind(http, { ok: false, auth_required: true }, true), 'testing');
  assert.equal(serverStatusKind({ ...http, enabled: false }, { ok: false, auth_required: true }), 'paused');

  assert.equal(isSignedIn({ transport: 'http', oauth: { connected: true, expires_at: null } }), true);
  assert.equal(isSignedIn({ transport: 'http', oauth: { connected: false, expires_at: null } }), false);
  assert.equal(isSignedIn({ transport: 'http' }), false, 'older servers without the field');
  assert.equal(isSignedIn({ transport: 'stdio', oauth: { connected: true, expires_at: null } }), false);
});

test('stored status keeps auth_required, still without error text', () => {
  const needs = toStoredStatus({ ok: false, auth_required: true, error: 'Sign-in required' }, 't');
  assert.deepEqual(needs, { ok: false, tools: [], at: 't', auth_required: true });
  assert.deepEqual(toStoredStatus({ ok: true, auth_required: true, tools: [] }, 't'), { ok: true, tools: [], at: 't' });
  assert.deepEqual(sanitizeStatuses({ a: { ok: false, tools: [], at: 't', auth_required: true, error: 'x' } }), {
    a: { ok: false, tools: [], at: 't', auth_required: true },
  });
  assert.deepEqual(sanitizeStatuses({ a: { ok: false, tools: [], at: 't', auth_required: 'yes' } }), { a: { ok: false, tools: [], at: 't' } });
});

test('accepts sign-in results only from this origin and in the expected shape', () => {
  const origin = 'https://kwnt.example.com';
  const data = { type: 'kwnt-mcp-oauth', server_id: 's1', ok: true };
  assert.deepEqual(readOAuthMessage({ origin, data }, origin), { server_id: 's1', ok: true });
  assert.deepEqual(readOAuthMessage({ origin, data: { ...data, ok: false, error: 'access_denied' } }, origin), {
    server_id: 's1',
    ok: false,
    error: 'access_denied',
  });
  assert.deepEqual(readOAuthMessage({ origin, data: { ...data, server_id: null, ok: false } }, origin), { server_id: null, ok: false });

  assert.equal(readOAuthMessage({ origin: 'https://evil.example', data }, origin), null);
  assert.equal(readOAuthMessage({ origin: 'https://kwnt.example.com:8443', data }, origin), null);
  assert.equal(readOAuthMessage({ origin: 'null', data }, origin), null);
  assert.equal(readOAuthMessage({ origin, data: { ...data, type: 'other' } }, origin), null);
  assert.equal(readOAuthMessage({ origin, data: { ...data, ok: 'true' } }, origin), null);
  assert.equal(readOAuthMessage({ origin, data: { ...data, server_id: 7 } }, origin), null);
  assert.equal(readOAuthMessage({ origin, data: 'kwnt-mcp-oauth' }, origin), null);
  assert.equal(readOAuthMessage({ origin, data: null }, origin), null);
});

test('a server card says which kwants use it', () => {
  const kwants = [
    { id: 'k1', name: 'Architect' },
    { id: 'k2', name: 'SmartDataBeast' },
  ];
  assert.deepEqual(kwantUsage({ disabled_for_kwants: [] }, kwants), { allKwants: true, offFor: [] });
  // Older backends leave the field out: that still means every kwant.
  assert.deepEqual(kwantUsage({}, kwants), { allKwants: true, offFor: [] });
  // Keeps the backend's name order and names only kwants this screen knows.
  assert.deepEqual(kwantUsage({ disabled_for_kwants: ['k1', 'k2', 'gone'] }, kwants), {
    allKwants: false,
    offFor: ['Architect', 'SmartDataBeast'],
  });
});

test('a new kwant starts with every server on, paused ones included', () => {
  assert.deepEqual(defaultKwantServerIds([]), []);
  assert.deepEqual(
    defaultKwantServerIds([
      { id: 'a', enabled: true },
      { id: 'b', enabled: false },
    ]),
    ['a', 'b'],
  );
});

const presetForm = (id) => {
  const f = MCP_PRESETS.find((p) => p.id === id).form;
  return { ...f, env: f.env.map((r) => ({ ...r })), headers: f.headers.map((r) => ({ ...r })) };
};

test('only presets that need a key or token mark it required', () => {
  const marked = Object.fromEntries(
    MCP_PRESETS.map((p) => [p.id, [...p.form.env, ...p.form.headers].filter((r) => r.required).map((r) => r.key)]).filter(([, keys]) => keys.length > 0),
  );
  assert.deepEqual(marked, { github: ['Authorization'], 'brave-search': ['BRAVE_API_KEY'], postgres: ['DATABASE_URI'] });
  // Every preset tagged as needing a key or token has a required field, and no Sign-in preset does.
  for (const p of MCP_PRESETS) {
    const hasRequired = [...p.form.env, ...p.form.headers].some((r) => r.required);
    assert.equal(hasRequired, p.needs.includes('Token') || p.needs.includes('API key') || p.id === 'postgres', p.id);
    if (p.needs.includes('Sign-in')) assert.equal(hasRequired, false, p.id);
  }
});

test('warns about required keys that are still blank, and only those', () => {
  const github = presetForm('github');
  assert.deepEqual(blankRequiredKeys(github), ['Authorization']);
  for (const blank of ['', '   ', 'Bearer', 'bearer ', 'Basic']) {
    github.headers[0].value = blank;
    assert.deepEqual(blankRequiredKeys(github), ['Authorization'], JSON.stringify(blank));
  }
  github.headers[0].value = 'Bearer ghp_abc123';
  assert.deepEqual(blankRequiredKeys(github), []);

  const brave = presetForm('brave-search');
  assert.deepEqual(blankRequiredKeys(brave), ['BRAVE_API_KEY']);
  brave.env[0].value = 'BSA123';
  assert.deepEqual(blankRequiredKeys(brave), []);
  assert.deepEqual(blankRequiredKeys(presetForm('postgres')), ['DATABASE_URI']);

  // Sign-in, no-key and custom servers never warn.
  for (const id of ['notion', 'context7', 'files', 'supabase']) assert.deepEqual(blankRequiredKeys(presetForm(id)), [], id);
  const custom = { ...emptyMcpForm('http'), headers: [{ key: 'Authorization', value: '', saved: false }] };
  assert.deepEqual(blankRequiredKeys(custom), []);
});

test('a saved key left blank is kept, so it never warns', () => {
  assert.equal(isBlankRequired({ key: 'Authorization', value: '', saved: true, required: true }), false);
  assert.equal(isBlankRequired({ key: 'Authorization', value: '', saved: false, required: true }), true);
  assert.equal(isBlankRequired({ key: 'Authorization', value: '', saved: false }), false);
});

test('a blank required key still saves, and the flag stays out of the request', () => {
  const github = presetForm('github');
  assert.equal(validateMcpForm(github), null);
  const payload = formToPayload(github);
  assert.deepEqual(payload.headers, { Authorization: '' });
  assert.equal(JSON.stringify(payload).includes('required'), false);
});
