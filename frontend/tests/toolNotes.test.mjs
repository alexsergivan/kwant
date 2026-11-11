// 0.9.0 P5: bots' notes about their tools. Contract: release-090/p5-contract.md (Lin);
// design: release-090/designs/p5-tool-notes.md (Mira).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/toolNotes';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ToolNotesSection, NoteSheet } from './src/components/ToolNotesSection';
  import { ToolNoteLine } from './src/components/ToolNoteLine';
  import { ConnectionsView, ConnectionRow } from './src/components/ConnectionsView';
  export const section = (props) => renderToStaticMarkup(React.createElement(ToolNotesSection, props));
  export const sheet = (props) => renderToStaticMarkup(React.createElement(NoteSheet, props));
  export const line = (props) => renderToStaticMarkup(React.createElement(ToolNoteLine, props));
  export const view = (props) => renderToStaticMarkup(React.createElement(ConnectionsView, props));
  export const row = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(ConnectionRow, props)));
`);

const NOW = Date.parse('2026-11-10T15:00:00Z');
// Shaped like the note in p5-contract.md §1.
const note = (over = {}) => ({
  id: 'tn_1', tool: 'github · create_pull_request', connection: 'github', builtin: false, removed: false, kind: 'broke', state: 'open',
  text: 'Fails with "head branch not found" right after a push. Waiting ~10 s and trying again works.', bot_text: 'Fails…',
  edited_by_person: false, written_by_person: false, count: 3, bots: [{ id: 'k1', name: 'Pixel' }, { id: 'k2', name: 'Iris' }],
  created_at: '2026-11-08T10:00:00Z', last_at: '2026-11-10T14:02:00Z', updated_at: null, fixed_at: null, purge_at: null,
  where: { channel_id: null, direct_kwant_id: 'k1', thread_id: null, message_id: 'm9' }, ...over,
});
const counts = (over = {}) => ({ all: 4, broke: 2, tip: 1, missing: 1, fixed: 3, ...over });
const list = (notes, c = {}) => ({ notes, counts: counts(c) });
const never = () => new Promise(() => {});
const api = { list: never, add: never, update: never, remove: never };
const props = (over = {}) => ({ api, now: NOW, initialList: list([note()]), ...over });

test('a note is read defensively: built-in when no connection, unknown kind as tip, no where without a chat', () => {
  const n = m.parseToolNote({ id: 'a', tool: 'bash', connection: null, kind: 'weird', count: 0, where: { channel_id: null, direct_kwant_id: null } });
  assert.equal(n.builtin, true);
  assert.equal(n.kind, 'tip');
  assert.equal(n.count, 1);
  assert.equal(n.where, null);
  assert.equal(m.toolLabel(n), 'Built-in · bash');
  assert.equal(m.toolLabel(m.parseToolNote(note())), 'github · create_pull_request');
  assert.equal(m.parseToolNote({ id: 'a' }), null);
  const l = m.parseToolNotesList({ notes: [note(), { nope: 1 }], counts: { all: 2 } });
  assert.equal(l.notes.length, 1);
  assert.deepEqual(l.counts, { all: 2, broke: 0, tip: 0, missing: 0, fixed: 0 });
});

test('the list query follows the filter and connection, and note=<id> asks for just that note', () => {
  assert.equal(m.notesQuery('all', null), 'state=open');
  assert.equal(m.notesQuery('broke', 'github'), 'state=open&kind=broke&connection=github');
  assert.equal(m.notesQuery('fixed', null), 'state=fixed');
  assert.equal(m.notesQuery('tip', 'github', 'tn_9'), 'note=tn_9');
});

test('the person’s text is trimmed, kept to 4 lines, and refused past 280 characters', () => {
  assert.deepEqual(m.cleanNoteText('  a\nb\nc\nd\ne  '), { text: 'a\nb\nc\nd' });
  assert.ok('error' in m.cleanNoteText('   '));
  assert.ok('error' in m.cleanNoteText('x'.repeat(281)));
  assert.deepEqual(m.cleanNoteText('x'.repeat(280)), { text: 'x'.repeat(280) });
});

test('the chat line has the design’s words for each kind and for a repeat', () => {
  assert.deepEqual(m.chatLineParts('Pixel', { note_kind: 'broke', repeat: false }), ['Pixel noted that ', ' didn’t work.']);
  assert.deepEqual(m.chatLineParts('Pixel', { note_kind: 'broke', repeat: true }), ['Pixel noted again that ', ' didn’t work.']);
  assert.deepEqual(m.chatLineParts('Lin', { note_kind: 'tip', repeat: false }), ['Lin noted a tip for ', '.']);
  assert.deepEqual(m.chatLineParts('Rigel', { note_kind: 'missing', repeat: false }), ['Rigel noted that it was missing ', '.']);
  const meta = m.parseToolNoteMeta({ kind: 'tool_note', note_id: 'tn_1', kwant_id: 'k1', tool: 'github · create_pull_request', note_kind: 'broke', repeat: false, count: 1 });
  assert.equal(meta.note_id, 'tn_1');
  assert.equal(m.parseToolNoteMeta({ kind: 'bot_action', note_id: 'x', tool: 't' }), null);
  const html = ui.line({ meta, botName: 'Pixel', messageId: 'm9', onOpen: () => {} });
  assert.match(html, /Pixel noted that <code[^>]*>github · create_pull_request<\/code> didn’t work\./);
  assert.match(html, /See the note/);
  assert.match(html, /data-message-id="m9"/);
});

test('an open note: kind tag with icon and word, mono tool, ×3, bots and last time, and the three actions', () => {
  const html = ui.section(props());
  assert.match(html, /Notes from bots/);
  assert.match(html, /4 open/);
  assert.match(html, /data-note-kind="broke"[^>]*><svg[\s\S]*?Didn’t work/);
  assert.match(html, /github · create_pull_request/);
  assert.match(html, /×3/);
  assert.match(html, /Pixel/);
  assert.match(html, /Iris/);
  assert.match(html, /last \d\d:\d\d/);
  assert.match(html, /Mark fixed/);
  assert.match(html, /aria-label="Edit note"/);
  assert.match(html, /aria-label="More actions for this note"/);
  assert.match(html, /aria-label="Didn’t work: github · create_pull_request"/);
  // The filters are a radio group with counts.
  assert.match(html, /role="radiogroup" aria-label="Filter notes"/);
  for (const [f, n] of [['all', 4], ['broke', 2], ['tip', 1], ['missing', 1], ['fixed', 3]])
    assert.match(html, new RegExp(`aria-checked="${f === 'all'}"[^>]*data-note-filter="${f}"[\\s\\S]*?>${n}<`));
  // The meta line is zinc-400, not zinc-500 (AA at 11.5 px).
  assert.match(html, /text-\[11\.5px\] text-zinc-400/);
});

test('edited and person-written notes say so; a removed connection’s tool is greyed', () => {
  const html = ui.section(props({ initialList: list([note({ id: 'a', kind: 'tip', edited_by_person: true }), note({ id: 'b', written_by_person: true, bots: [], edited_by_person: true }), note({ id: 'c', removed: true })]) }));
  assert.equal((html.match(/Edited by you/g) ?? []).length, 1);
  assert.match(html, /Written by you/);
  assert.match(html, /text-zinc-500[^>]*>github · create_pull_request<span[^>]*>\(removed\)/);
});

test('a fixed note: Fixed tag, grey text, when it was fixed, Reopen and a delete icon', () => {
  const html = ui.section(props({ initialFilter: 'fixed', initialList: list([note({ state: 'fixed', fixed_at: '2026-11-10T14:20:00Z' })]) }));
  assert.match(html, /data-note-kind="fixed"/);
  assert.match(html, /Fixed by you · \d\d:\d\d · bots no longer see it/);
  assert.match(html, /Reopen/);
  assert.match(html, /aria-label="Delete note"/);
  assert.doesNotMatch(html, /Mark fixed/);
});

test('editing: textarea with the counter, the three-way kind switch, Cancel and Save; a save error keeps the text', () => {
  const html = ui.section(props({ initialMode: { tn_1: 'edit' }, initialEditError: 'Couldn’t save the note. Try again.' }));
  assert.match(html, /<textarea/);
  assert.match(html, /92 \/ 280/);
  assert.match(html, /role="radiogroup" aria-label="Kind of note"/);
  assert.match(html, /aria-checked="true"[^>]*data-kind-option="broke"/);
  assert.match(html, /Cancel/);
  assert.match(html, />Save</);
  assert.match(html, /Couldn’t save the note\. Try again\./);
  assert.match(html, /head branch not found/);
  assert.doesNotMatch(html, /Mark fixed/);
});

test('delete asks inline, with the design’s words', () => {
  const html = ui.section(props({ initialMode: { tn_1: 'confirm' } }));
  assert.match(html, /Delete this note\? Bots stop seeing it, and it isn’t kept under Fixed\./);
  assert.match(html, /bg-rose-700 text-white/);
});

test('add a note: a tool picker of the connections plus Built-in…', () => {
  const html = ui.section(props({ initialAdding: true, connections: ['github', 'cloudflare'] }));
  assert.match(html, /<option value="github"/);
  assert.match(html, /<option value="cloudflare"/);
  assert.match(html, /Built-in…/);
  assert.match(html, />Tool</);
  assert.doesNotMatch(html, /Add a note/);
});

test('states: no notes ever, only fixed, a filter with no matches, and a load error', () => {
  assert.match(ui.section(props({ initialList: list([], { all: 0, broke: 0, tip: 0, missing: 0, fixed: 0 }) })), /No notes yet\. When a tool fails, surprises a bot or is missing, the bot writes it down here\./);
  const onlyFixed = ui.section(props({ initialList: list([], { all: 0, broke: 0, tip: 0, missing: 0, fixed: 2 }) }));
  assert.match(onlyFixed, /No open notes\./);
  assert.match(onlyFixed, /data-note-filter="fixed"[\s\S]*?>2</);
  assert.match(ui.section(props({ initialFilter: 'tip', initialList: list([], { tip: 0 }) })), /No notes of this kind\./);
  const err = ui.section(props({ initialList: null, initialError: 'Couldn’t load notes.' }));
  assert.match(err, /Couldn’t load notes\./);
  assert.match(err, /Retry/);
  // Add a note stays in every state.
  assert.match(err, /Add a note/);
});

test('more than five notes fold behind "Show {n} more"', () => {
  const html = ui.section(props({ initialList: list(Array.from({ length: 8 }, (_, i) => note({ id: `n${i}` }))) }));
  assert.equal((html.match(/data-note="/g) ?? []).length, 5);
  assert.match(html, /Show 3 more/);
});

test('a connection filter adds a removable chip; a note from the chat shows its own chip', () => {
  assert.match(ui.section(props({ connection: 'github' })), /data-note-connection="github"/);
  const focus = ui.section(props({ focusNote: 'tn_1' }));
  assert.match(focus, /Note from the chat/);
  assert.match(focus, /aria-checked="false"[^>]*data-note-filter="all"/);
});

test('read-only: the list without actions', () => {
  const html = ui.section(props({ readOnly: true }));
  assert.doesNotMatch(html, /Mark fixed|Edit note|Add a note|More actions/);
  assert.match(html, /github · create_pull_request/);
});

test('phone: one 44 px ⋯ button per row, and a sheet with the four actions', () => {
  const html = ui.section(props({ phone: true }));
  assert.match(html, /h-11 w-11[^"]*" aria-label="More actions for this note"/);
  assert.doesNotMatch(html, /aria-label="Edit note"/);
  assert.match(html, /h-9 /);
  const s = ui.sheet({ n: m.parseToolNote(note()), onClose() {}, onFix() {}, onEdit() {}, onWhere() {}, onDelete() {} });
  assert.match(s, /role="dialog"/);
  for (const w of ['Mark fixed', 'Edit', 'Open where it happened', 'Delete']) assert.match(s, new RegExp(w));
  assert.equal((s.match(/h-\[52px\]/g) ?? []).length, 4);
  const fixed = ui.sheet({ n: m.parseToolNote(note({ state: 'fixed' })), onClose() {}, onFix() {}, onEdit() {}, onDelete() {} });
  assert.match(fixed, /Reopen/);
  assert.doesNotMatch(fixed, /Open where it happened|>Edit/);
});

const conn = (over = {}) => ({
  id: 'mcp:s1', kind: 'mcp', name: 'github', detail: 'api.githubcopilot.com', status: 'connected', status_detail: null,
  last_check: '2026-11-10T14:55:00Z', used_by: [], logins_stored: 'kwnt', manage: '/api/mcp/servers/s1', ...over,
});

test('connection rows: "{n} notes", rose when any didn’t work, none without open notes or on other kinds', () => {
  const rose = ui.row({ c: conn({ notes: { open: 2, broke: 1 } }), now: NOW, onNotes: () => {} });
  assert.match(rose, /data-notes-chip="broke"/);
  assert.match(rose, /2 notes/);
  assert.match(rose, /aria-label="2 notes about github, 1 didn’t work: show them"/);
  const plain = ui.row({ c: conn({ notes: { open: 1, broke: 0 } }), now: NOW, onNotes: () => {} });
  assert.match(plain, /data-notes-chip="open"/);
  assert.match(plain, />1 note</);
  assert.doesNotMatch(ui.row({ c: conn({ notes: { open: 0, broke: 0 } }), now: NOW }), /data-notes-chip/);
  assert.doesNotMatch(ui.row({ c: conn({ kind: 'account', notes: { open: 2, broke: 0 } }), now: NOW }), /data-notes-chip/);
});

test('Connections: the section sits above the groups, and hides behind "Confirm it’s you"', () => {
  const data = { connections: [conn()], counts: {}, vault: { status: 'ok' }, catalog: [] };
  const base = { data, loading: false, error: null, now: NOW, onRefresh() {}, onAddComposio: async () => null, notes: { api, initialList: list([note()]) } };
  const html = ui.view(base);
  assert.ok(html.indexOf('data-tool-notes') > -1 && html.indexOf('data-tool-notes') < html.indexOf('Tool servers (MCP)'));
  assert.doesNotMatch(ui.view({ ...base, needsToken: true }), /data-tool-notes/);
  assert.doesNotMatch(ui.view({ ...base, data: null, loading: true }), /data-tool-notes/);
});
