import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export * from './src/imports';
  export { SourceList, PreviewView, DoneView, ErrorBox, RecentImports } from './src/components/ImportDialog';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const html = (C, props) => m.renderToStaticMarkup(m.createElement(C, props));
const text = (s) => s.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ');

// Shaped like the server's preview in IMPORT-API.md.
const preview = (over = {}) => ({
  bundle: {
    format: 'kwant.bundle/1',
    source: { kind: 'openmaus', label: 'OpenMausBot', exported_at: null },
    bots: [
      { ref: 'lead', name: 'Ada', title: 'Tech lead', persona_prompt: 'Plan small steps.\n\nCheck the work. '.repeat(20), memories: ['a', 'b'], skills: [{ name: 'plan', description: '', body: 'x' }], routines: [] },
      { ref: 'qa', name: 'Rigel', title: 'QA', persona_prompt: 'Test it.', memories: [], skills: [], routines: [{ name: 'Friday', instructions: 'x', schedule: {} }] },
    ],
    teams: [{ name: 'Eng', lead: 'lead', members: ['lead', 'qa'] }],
    skills: [],
    omitted: ['Rooms'],
  },
  warnings: [{ bot: 'lead', message: 'We removed things that looked like API keys.' }, { message: 'Model set to auto.' }],
  dropped: [{ bot: 'qa', message: '1 conversation starter(s): no place for them yet.' }],
  problems: [],
  renames: [{ ref: 'lead', from: 'Ada', to: 'Ada-2' }],
  can_apply: true,
  ...over,
});

test('preview rows: renamed names, per-bot notices and a short excerpt', () => {
  const { bots, general } = m.previewRows(preview());
  assert.equal(bots[0].name, 'Ada-2');
  assert.equal(bots[0].renamedFrom, 'Ada');
  assert.equal(bots[1].renamedFrom, null);
  assert.deepEqual([bots[0].memories, bots[0].skills, bots[1].routines], [2, 1, 1]);
  assert.ok(bots[0].excerpt.length <= 180 && bots[0].excerpt.endsWith('…'));
  assert.ok(!bots[0].excerpt.includes('\n'));
  assert.deepEqual(bots[0].warnings, ['We removed things that looked like API keys.']);
  assert.deepEqual(bots[1].dropped, ['1 conversation starter(s): no place for them yet.']);
  assert.deepEqual(general.warnings, ['Model set to auto.']);
});

test('Import needs the server to agree and at least one bot left', () => {
  assert.equal(m.canImport(preview(), new Set()), true);
  assert.equal(m.canImport(preview(), new Set(['lead'])), true);
  assert.equal(m.canImport(preview(), new Set(['lead', 'qa'])), false);
  assert.equal(m.canImport(preview({ can_apply: false }), new Set()), false);
});

test('the preview shows the rename, the notices, the safety note and the count left', () => {
  const out = text(html(m.PreviewView, { preview: preview(), skip: new Set(['qa']), onToggle: () => {} }));
  assert.match(out, /1 bot will be created from OpenMausBot\./);
  // A bot name in source.label is not where it came from.
  const card = text(html(m.PreviewView, { preview: preview({ bundle: { ...preview().bundle, source: { kind: 'kwant', label: 'Recipe Pal' } } }), skip: new Set(), onToggle() {} }));
  assert.match(card, /2 bots will be created\. Untick/);
  assert.match(out, /Ada-2/);
  assert.match(out, /Renamed from Ada: that name is already taken here/);
  assert.match(out, /Worth a look/);
  assert.match(out, /Not brought over .*Rooms/);
  assert.match(out, /1 team and|1 team \(Eng\)/);
  assert.match(out, /start with no tools, their skills wait for your OK, and routines start paused/);
});

test('problems block the import and are shown first', () => {
  const p = preview({ problems: [{ bot: 'qa', message: 'Its instructions are over 16,000 characters.' }], can_apply: false });
  const out = text(html(m.PreviewView, { preview: p, skip: new Set(), onToggle: () => {} }));
  assert.match(out, /Must be fixed before importing .*over 16,000 characters/);
  assert.equal(m.canImport(p, new Set()), false);
});

test('Bring a bot from… lists the sources in the server order, then Upload a file', () => {
  const prompts = ['grok', 'chatgpt', 'claude', 'other'].map((id, i) => ({ id, label: ['Grok', 'ChatGPT', 'Claude', 'Another app'][i], where_to_send: '', prompt: 'p' }));
  const out = text(html(m.SourceList, { prompts, error: null, onPick: () => {}, onUpload: () => {}, onRetry: () => {} }));
  const order = ['Grok', 'ChatGPT', 'Claude', 'Another app', 'Upload a file'].map((w) => out.indexOf(w));
  assert.ok(order.every((n) => n >= 0), out);
  assert.deepEqual([...order].sort((a, b) => a - b), order);
  assert.match(text(html(m.SourceList, { prompts: null, error: null, onPick() {}, onUpload() {}, onRetry() {} })), /Loading/);
  const failed = text(html(m.SourceList, { prompts: null, error: new m.ImportError('', 'network', 0), onPick() {}, onUpload() {}, onRetry() {} }));
  assert.match(failed, /No connection.*Try again/);
});

test('every error code gets a plain heading, and the server’s sentence when it has one', () => {
  const E = (code, msg = '') => m.importErrorText(new m.ImportError(msg, code, 400));
  assert.equal(E('too_large').title, 'That’s too big to import');
  assert.match(E('too_large').detail, /20 MB/);
  assert.equal(E('unsupported').title, 'We can’t read that kind of file');
  assert.equal(E('invalid', 'This zip has a file path that points outside it.').detail, 'This zip has a file path that points outside it.');
  assert.equal(E('empty').title, 'Nothing to import');
  assert.equal(E('problems').title, 'Fix these first');
  assert.equal(E('name_taken').title, 'A name was just taken');
  assert.equal(E('not_found').title, 'That import is gone');
  // A database error never shows its details.
  assert.equal(E('internal', 'sqlite: disk I/O').detail, 'The server couldn’t finish this. Try again in a moment.');
  assert.equal(m.importErrorText(new Error('x')).title, 'Something went wrong');
  const box = text(html(m.ErrorBox, { error: new m.ImportError('', 'problems', 400, [], [{ message: 'Too many bots.' }]) }));
  assert.match(box, /Fix these first.*Too many bots/);
});

test('import errors keep the code and the names the server sends', async () => {
  const real = globalThis.fetch;
  try {
    globalThis.fetch = async () =>
      new Response(JSON.stringify({ code: 'edited', edited: ['Ada'], error: 'Ada changed since the import.' }), { status: 409 });
    await assert.rejects(m.importApi.undo('x'), (e) => e instanceof m.ImportError && e.code === 'edited' && e.names[0] === 'Ada');
    globalThis.fetch = async () => new Response('gateway', { status: 413 });
    await assert.rejects(m.importApi.previewFile(new Blob(['x']), 'a.zip'), (e) => e.code === 'too_large');
    globalThis.fetch = async () => {
      throw new TypeError('offline');
    };
    await assert.rejects(m.importApi.prompts(), (e) => e.code === 'network');
    const calls = [];
    globalThis.fetch = async (url, init) => {
      calls.push([url, init]);
      return new Response('{}', { status: 200 });
    };
    await m.importApi.previewFile(new Blob(['x']), 'my card.png');
    await m.importApi.undo('id1', true);
    assert.equal(calls[0][0], '/api/import/preview?filename=my%20card.png');
    assert.equal(calls[0][1].headers['Content-Type'], 'application/octet-stream');
    assert.equal(calls[1][1].body, '{"force":true}');
  } finally {
    globalThis.fetch = real;
  }
});

test('size checks match the server limits', () => {
  assert.equal(m.fileTooBig(20 * 1024 * 1024), false);
  assert.equal(m.fileTooBig(20 * 1024 * 1024 + 1), true);
  assert.equal(m.textTooLong('x'.repeat(256 * 1024 + 1)), true);
});

const applied = {
  import: {
    id: 'i1', space_id: 'spc_default', source_kind: 'openmaus', status: 'applied', created_at: '2026-10-19T00:00:00Z', undone_at: null,
    created: { kwants: [{ ref: 'lead', id: 'k1', name: 'Ada-2' }, { ref: 'qa', id: 'k2', name: 'Rigel' }], skills: ['s'], routines: ['r'], teams: ['t'], channels: ['c'], memories: 2 },
  },
  warnings: [], dropped: [], renames: [],
};

test('after Import: what was made, then each undo state', () => {
  const view = (undo) => text(html(m.DoneView, { applied, undo, onUndo() {}, onKeep() {} }));
  assert.match(view({ kind: 'idle' }), /Imported 2 bots Ada-2, Rigel.*1 team, 1 skill to review, 1 paused routine, 2 memories/);
  const edited = view({ kind: 'confirm_edited', names: ['Ada-2'] });
  assert.match(edited, /Ada-2 changed since the import\. Undo anyway removes it with your changes/);
  assert.match(edited, /Keep them.*Undo anyway/);
  assert.match(view({ kind: 'in_use', names: ['Rigel'] }), /Rigel has already been used, so this import can’t be undone in one step/);
  assert.match(view({ kind: 'undone' }), /Import undone.*Ada-2, Rigel were removed/);
});

test('recent imports offer Undo only while applied', () => {
  const out = html(m.RecentImports, { records: [applied.import, { ...applied.import, id: 'i2', status: 'undone' }], onUndo() {}, busyId: null });
  assert.equal((out.match(/aria-label="Undo import/g) ?? []).length, 1);
  assert.match(text(out), /undone/);
  assert.match(text(html(m.RecentImports, { records: [], onUndo() {}, busyId: null })), /No imports yet/);
});

test('the download name comes from Content-Disposition', () => {
  assert.equal(m.downloadName('attachment; filename="Ada-Fixture.kwant.json"', 'x'), 'Ada-Fixture.kwant.json');
  assert.equal(m.downloadName("attachment; filename*=UTF-8''Caf%C3%A9.kwant.json", 'x'), 'Café.kwant.json');
  assert.equal(m.downloadName(null, 'export.kwant.json'), 'export.kwant.json');
});

test('focus trap: Tab wraps from the last item to the first, Shift+Tab the other way', () => {
  assert.equal(m.trapFocusTarget(3, 2, false), 0);
  assert.equal(m.trapFocusTarget(3, 0, true), 2);
  assert.equal(m.trapFocusTarget(3, 1, false), null);
  assert.equal(m.trapFocusTarget(3, 1, true), null);
  assert.equal(m.trapFocusTarget(3, -1, false), 0);
  assert.equal(m.trapFocusTarget(3, -1, true), 2);
  assert.equal(m.trapFocusTarget(0, -1, false), null);
});

test('success state says tools added later reach the imported bots', () => {
  const applied = { import: { id: 'imp_1', created: { kwants: [{ id: 'k1', name: 'Recipe-Pal' }], teams: [], skills: [], routines: [], memories: 0 } }, warnings: [] };
  const out = text(html(m.DoneView, { applied, undo: { kind: 'idle' }, onUndo: () => {}, onKeep: () => {} }));
  assert.match(out, /Tools you add to this space later will be on for these bots too\./);
});
