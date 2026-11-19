// 29m (autonomy/file-steps-every-engine/brief.md): every engine's file step wraps its path at a folder, not only
// Claude's. isFileStep uses the server's one list (FileTool::of, src/drivers/activity.rs) and drops an
// `mcp__<server>__` prefix the way the server's tool_part does. And on a touch phone the step summary is 2 px wider
// each side (-mx-0.5 px-0.5), so its inset focus ring clears the ✓. Both tests fail on b7d44d03 (#835).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SavedActivityList } from './src/components/ActivityList';
  export { parseActivityMeta, isFileStep } from './src/activity';
  export const saved = (props) => renderToStaticMarkup(React.createElement(SavedActivityList, props));
`);

const at = (min) => new Date(Date.parse('2026-11-18T10:00:00Z') + min * 60000).toISOString();
const step = (id, tool, t, detail) => ({
  id, kind: 'tool', status: 'ok', title: `Step ${id}`, ...(tool ? { tool } : {}), detail,
  started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000,
});
const html = (steps) =>
  m.saved({ activity: m.parseActivityMeta({ activity: { steps, dropped: 0 } }), idBase: 'r1', defaultOpen: true });
const pres = (steps) => [...html(steps).matchAll(/<pre\b[^>]*>([\s\S]*?)<\/pre>/g)].map((x) => x[1]);
const escaped = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const wbrs = (inner) => (inner.match(/<wbr\/?>/g) ?? []).length;

const LONG = '/home/me/projects/spring-sale/drafts/spring-sale-plan.md';
const ENGINE_FILE_TOOLS = [
  'apply_patch', 'read_file', 'edit_file', 'create_file', 'str_replace_based_edit_tool',
  'mcp__kwnt__read_file', 'mcp__fs__write_file',
  'view', 'cat', 'str_replace', 'file_change', 'write_file', 'Read_File', 'mcp__kwnt__Edit',
];

test("Codex, grok and MCP file steps break their path after each '/' (fails on b7d44d03)", () => {
  const r = pres(ENGINE_FILE_TOOLS.map((t, i) => step(`f${i}`, t, i, LONG)));
  ENGINE_FILE_TOOLS.forEach((t, i) => {
    assert.equal(wbrs(r[i]), 6, `${t}: ${r[i]}`);
    assert.equal(r[i].replace(/<wbr\/?>/g, ''), LONG, `${t}: the copied text is the path`);
    assert.ok(r[i].endsWith('spring-sale-plan.md'), `${t}: the file name stays one piece`);
  });
});

test('a non-file MCP tool, Bash or Glob step with a path-like detail stays plain', () => {
  const tools = ['mcp__x__run', 'mcp__kwnt__run', 'Bash', 'Glob', 'exec_command', 'mcp__read_file', 'mcp__fs__'];
  const r = pres(tools.map((t, i) => step(`p${i}`, t, i, LONG)));
  tools.forEach((t, i) => assert.equal(r[i], escaped(LONG), t));
});

test('isFileStep matches the server FileTool list after dropping mcp__<server>__', () => {
  const server = ['read', 'read_file', 'view', 'cat', 'edit', 'multiedit', 'edit_file', 'str_replace',
    'str_replace_based_edit_tool', 'apply_patch', 'file_change', 'notebookedit', 'write', 'write_file', 'create_file'];
  for (const t of server) {
    for (const name of [t, t.toUpperCase(), `mcp__kwnt__${t}`, `mcp__some_server__${t}`])
      assert.equal(m.isFileStep({ tool: name }), true, name);
  }
  // tool_part: only a leading "mcp__" is dropped, then everything up to the first "__"; with no second "__" the
  // whole name is used, as on the server.
  for (const t of [undefined, '', 'Bash', 'Grep', 'Glob', 'WebFetch', 'Reader', 'shell', 'exec_command',
    'mcp__kwnt__run', 'mcp__read_file', 'x__read_file', 'MCP__kwnt__read', 'readfile', 'read_files'])
    assert.equal(m.isFileStep({ tool: t }), false, String(t));
});

test('on a touch phone the step summary is 2 px wider each side so its ring clears the ✓ (fails on b7d44d03)', () => {
  const summary = html([step('s', 'Read', 0, LONG)]).match(/<summary\b[^>]*class="([^"]*)"/)?.[1] ?? '';
  const cls = summary.split(/\s+/);
  for (const c of [
    '[@media(max-width:767px)_and_(pointer:coarse)]:-mx-0.5',
    '[@media(max-width:767px)_and_(pointer:coarse)]:px-0.5',
    // 29e/29i stay as they were.
    '[@media(max-width:767px)_and_(pointer:coarse)]:-my-1.5',
    '[@media(max-width:767px)_and_(pointer:coarse)]:py-1.5',
    '[@media(max-width:767px)_and_(pointer:coarse)]:focus-visible:-outline-offset-2',
  ]) assert.ok(cls.includes(c), `${c} in ${summary}`);
  // No horizontal padding at desktop/pointer widths: the summary's own content doesn't move there.
  assert.ok(!cls.some((c) => /^-?(mx|px)-/.test(c)), summary);
});
