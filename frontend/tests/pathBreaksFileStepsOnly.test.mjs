// 29l (autonomy/path-breaks-file-steps-only/brief.md): 29i's path breaks (<wbr> after each "/") are for a file
// step's detail only. A shell step's detail is the whole command, which may start like a path ("/usr/bin/git status",
// "./gradlew test", "~/bin/x --flag"); it renders as plain text, as before 29i, so its wrap point doesn't move and
// no lone "/" is left at a line end. The first test fails on 9cc2ec8f (#832).
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

const at = (min) => new Date(Date.parse('2026-11-18T09:00:00Z') + min * 60000).toISOString();
const step = (id, tool, t, detail) => ({
  id, kind: 'tool', status: 'ok', title: `Step ${id}`, ...(tool ? { tool } : {}), detail,
  started_at: at(t), ended_at: at(t + 0.5), duration_ms: 30000,
});
const pres = (steps) =>
  [...m.saved({ activity: m.parseActivityMeta({ activity: { steps, dropped: 0 } }), idBase: 'r1', defaultOpen: true })
    .matchAll(/<pre\b[^>]*>([\s\S]*?)<\/pre>/g)].map((x) => x[1]);
const escaped = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#x27;');
const wbrs = (inner) => (inner.match(/<wbr\/?>/g) ?? []).length;

const LONG = '/home/me/projects/spring-sale/drafts/spring-sale-plan.md';
const COMMANDS = [
  '/usr/bin/git status --short --untracked-files=all /home/me/projects/spring-sale',
  './gradlew test',
  '~/bin/x --flag',
  '../scripts/run.sh /home/me/a.md',
  'C:\\Tools\\build.exe --out C:\\out',
];

test('a shell step whose command starts with a path renders as plain text, no <wbr> (fails on 9cc2ec8f)', () => {
  for (const tool of ['Bash', 'bash', 'shell', 'exec_command', 'mcp__kwnt__run']) {
    const r = pres(COMMANDS.map((c, i) => step(`${tool}${i}`, tool, i, c)));
    COMMANDS.forEach((c, i) => assert.equal(r[i], escaped(c), `${tool}: ${c}`));
  }
});

test('a step with no tool gets no <wbr> either', () => {
  const r = pres([step('n', undefined, 0, LONG), step('c', undefined, 1, COMMANDS[0])]);
  assert.equal(r[0], escaped(LONG));
  assert.equal(r[1], escaped(COMMANDS[0]));
});

test('a Read, Write, Edit, MultiEdit or NotebookEdit step with a path still breaks after each "/", any case', () => {
  const tools = ['Read', 'Write', 'Edit', 'MultiEdit', 'NotebookEdit', 'read', 'EDIT', 'multiedit'];
  const r = pres(tools.map((t, i) => step(`f${i}`, t, i, LONG)));
  tools.forEach((t, i) => {
    assert.equal(wbrs(r[i]), 6, `${t}: ${r[i]}`);
    assert.ok(r[i].endsWith('spring-sale-plan.md'), 'the file name stays one piece');
  });
  // In the same list, a command row next to a file row stays plain.
  const mixed = pres([step('r', 'Read', 0, LONG), step('b', 'Bash', 1, COMMANDS[0])]);
  assert.equal(wbrs(mixed[0]), 6);
  assert.equal(mixed[1], escaped(COMMANDS[0]));
});

// 29m: isFileStep now uses the server's FileTool list, so read_file and mcp__x__read are file steps
// (fileStepsEveryEngine.test.mjs); shell, search and web tools stay out.
test('isFileStep knows the file tools and not shell, search or web tools', () => {
  for (const t of ['Read', 'write', 'EDIT', 'MultiEdit', 'notebookedit', 'read_file', 'mcp__x__read'])
    assert.equal(m.isFileStep({ tool: t }), true, t);
  for (const t of [undefined, '', 'Bash', 'Grep', 'Glob', 'WebFetch', 'Reader', 'mcp__kwnt__run', 'exec_command', 'shell'])
    assert.equal(m.isFileStep({ tool: t }), false, String(t));
});
