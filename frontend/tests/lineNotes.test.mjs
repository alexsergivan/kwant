// 0.9.0 G1 line notes on a diff (Mira, release-090/designs/g1-line-notes.md; Juno, release-090/g1-api.md).
// The parsers, the range logic, the copy, the API client against a fake caller, and the Changes section
// against an injected fake api. Demo data only; no network.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'PointerEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
if (!g.CSS) g.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/dev/lineNotes';
  export * from './src/dev/lineNotesApi';
  export { ChangesSection } from './src/components/dev/LineNotes';
  export { ReviewCard } from './src/components/DevDeskSlots';
  export { parseReview } from './src/dev/types';
  export * as fx from './src/dev/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

// ---------------------------------------------------------------------------------------------------
// Demo data

const HEAD = 'c3d9a18f00000000000000000000000000000000';
const NEXT = 'e81f2c4a00000000000000000000000000000000';
const rawChanges = (over = {}) => ({
  card_id: 'dvr_312',
  repo: 'demo-org/app',
  pr: 312,
  head_sha: HEAD,
  pr_url: 'https://github.com/demo-org/app/pull/312',
  pr_state: 'open',
  author_bot: { id: 'bot_vega', name: 'Vega' },
  author_login: 'vega-bot',
  files: [
    {
      path: 'src/server/auth.rs',
      status: 'modified',
      additions: 3,
      deletions: 1,
      hunks: [
        {
          header: '@@ -40,4 +40,6 @@ fn check_token',
          lines: [
            { kind: 'ctx', old: 40, new: 40, text: 'fn check_token(t: &str) -> bool {' },
            { kind: 'del', old: 41, text: '    t.len() > 8' },
            { kind: 'add', new: 41, text: '    let t = t.trim();' },
            { kind: 'add', new: 42, text: '    t.len() >= 12' },
            { kind: 'add', new: 43, text: '        && t.is_ascii()' },
            { kind: 'ctx', old: 42, new: 44, text: '}' },
          ],
        },
        {
          header: '@@ -90,2 +92,2 @@',
          lines: [
            { kind: 'ctx', old: 90, new: 92, text: '// end' },
            { kind: 'ctx', old: 91, new: 93, text: '' },
          ],
        },
      ],
    },
    { path: 'Cargo.lock', status: 'modified', additions: 40, deletions: 12, big: false, hunks: [{ header: '@@ -1 +1 @@', lines: [{ kind: 'ctx', old: 1, new: 1, text: '# lock' }] }] },
    { path: 'docs/logo.png', status: 'added', additions: 0, deletions: 0, omitted: 'binary', hunks: [] },
  ],
  ...over,
});
// Juno's final Note: every note has at_head (its place in the current diff); by default its own lines.
const rawNote = (over = {}) => {
  const n = {
    id: 'ln_1',
    path: 'src/server/auth.rs',
    start: { side: 'new', line: 42 },
    end: { side: 'new', line: 42 },
    code: [{ kind: 'add', old: null, new: 42, text: '    t.len() >= 12' }],
    text: 'Use a constant for the minimum length.',
    state: 'draft',
    head_sha: HEAD,
    created_at: '2026-11-10T09:12:00Z',
    seen_sha: null,
    ...over,
  };
  return 'at_head' in over ? n : { ...n, at_head: { start: n.start, end: n.end } };
};

/** An in-memory api: records every call, can be told to fail. */
function fakeApi({ changes = rawChanges(), notes = [], summary = null } = {}) {
  const calls = [];
  const s = { changes, notes: notes.map((n) => m.parseNote(n)), summary, fail: {}, seq: 1, deleted: new Map() };
  const maybe = async (name) => {
    calls.push(name);
    const f = s.fail[name];
    if (f) {
      if (typeof f === 'number') s.fail[name] = f - 1 || undefined;
      throw new m.LineNotesError('Nope.', 500, 'boom');
    }
  };
  const api = {
    async changes() {
      await maybe('changes');
      return m.parseChanges(s.changes);
    },
    async notes() {
      await maybe('notes');
      return { notes: s.notes.slice(), summary: s.summary };
    },
    async create(cardId, n) {
      await maybe('create');
      calls.push(['create', n]);
      const note = m.parseNote({ id: `ln_new${s.seq++}`, path: n.path, start: n.start, end: n.end, code: [], text: n.text, state: 'draft', head_sha: n.head_sha, at_head: { start: n.start, end: n.end } });
      s.notes.push(note);
      return note;
    },
    async edit(id, text) {
      await maybe('edit');
      const i = s.notes.findIndex((x) => x.id === id);
      s.notes[i] = { ...s.notes[i], text };
      return s.notes[i];
    },
    async remove(id) {
      await maybe('remove');
      s.deleted.set(id, s.notes.find((x) => x.id === id));
      s.notes = s.notes.filter((x) => x.id !== id);
    },
    async restore(id) {
      await maybe('restore');
      const n = s.deleted.get(id);
      s.notes.push(n);
      return n;
    },
    async discardAll() {
      await maybe('discardAll');
      const n = s.notes.filter((x) => x.state === 'draft').length;
      s.notes = s.notes.filter((x) => x.state !== 'draft');
      return n;
    },
    async send(cardId, sendId, ids) {
      calls.push(['send', sendId, ids ?? null]);
      await maybe('send');
      const out = s.notes.filter((x) => x.state === 'draft' && (!ids || ids.includes(x.id))).map((x) => ({ ...x, state: 'sent', sent_at: '2026-11-10T09:30:00Z' }));
      s.notes = s.notes.map((x) => out.find((o) => o.id === x.id) ?? x);
      return { sent: out.length, message_id: 'msg_1', kwant_id: 'bot_vega', kwant_name: 'Vega', sent_at: '2026-11-10T09:30:00Z', notes: out };
    },
  };
  return { api, calls, s };
}

let root = null;
let host = null;
async function mount(props) {
  if (root) root.unmount();
  host?.remove();
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(h(m.ChangesSection, { cardId: 'dvr_312', stat: { files: 3, additions: 43, deletions: 13 }, ...props }));
  await settle();
  return host;
}
const $ = (sel) => host.querySelector(sel);
const $$ = (sel) => [...host.querySelectorAll(sel)];
const click = async (el, init = {}) => {
  assert.ok(el, 'element to click');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true, ...init }));
  await settle();
};
const key = async (el, k, init = {}) => {
  el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }));
  await settle();
};
const byText = (text, sel = 'button') => $$(sel).find((b) => b.textContent.trim() === text);
/** React's controlled textarea: set through the native setter so onChange fires. */
const type = async (ta, value) => {
  const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(ta, value);
  ta.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const plus = (side, line) => $(`[data-ln-point="${side}:${line}"] button`);

// ---------------------------------------------------------------------------------------------------
// Parsers

test('parseChanges: keeps files, lines and counts; drops lines without their side number; reads omitted and the cut', () => {
  const c = m.parseChanges(rawChanges({ cut: { files_not_read: 2 } }));
  assert.equal(c.files.length, 3);
  assert.deepEqual(c.author_bot, { id: 'bot_vega', name: 'Vega' });
  const [auth, , png] = c.files;
  assert.deepEqual(auth.hunks[0].lines[1], { kind: 'del', old: 41, new: null, text: '    t.len() > 8' });
  assert.equal(auth.hunks[1].lines[1].text, '', 'a blank code line stays a line');
  assert.equal(png.omitted, 'binary');
  assert.deepEqual(c.cut, { files_not_read: 2 });
  const bad = m.parseChanges(rawChanges({ files: [{ path: 'a', hunks: [{ header: '', lines: [{ kind: 'add', text: 'x' }, { kind: 'ctx', new: 1, text: 'y' }, { kind: 'zap', new: 1, old: 1, text: 'z' }] }] }] }));
  assert.equal(bad.files[0].hunks.length, 0, 'every line missing its number is dropped');
  assert.equal(m.parseChanges(rawChanges({ files: [{ path: 'b', omitted: 'too_large' }] })).files[0].omitted, 'too_large');
  assert.equal(m.parseChanges(rawChanges({ cut: { files_not_read: 0 } })).cut, null, 'nothing cut');
  assert.equal(m.parseChanges(rawChanges({ pr_url: 'javascript:alert(1)' })).pr_url, null, 'only https links');
  assert.equal(m.parseChanges(rawChanges({ pr_state: 'weird' })).pr_state, 'open');
  assert.equal(m.parseChanges(rawChanges({ author_bot: null })).author_bot, null);
  assert.equal(m.parseChanges({ files: 'no' }), null);
  assert.equal(m.parseChanges(null), null);
});

test('parseNote: start/end objects, code as diff lines, at_head and seen_sha make moved and gone', () => {
  const a = m.parseNote(rawNote());
  assert.deepEqual([a.start, a.end, a.at_head.end.line, a.moved, a.gone], [{ side: 'new', line: 42 }, { side: 'new', line: 42 }, 42, false, false]);
  assert.deepEqual(a.code, [{ kind: 'add', old: null, new: 42, text: '    t.len() >= 12' }]);
  assert.equal(m.codeText([...a.code, { kind: 'del', old: 41, new: null, text: 'x' }]), '+     t.len() >= 12\n- x');
  assert.equal(m.parseNote({ id: 'x', path: 'p', side: 'old', start_line: 3, end_line: 5, text: 't' }), null, 'draft 1 anchors are gone');
  assert.equal(m.parseNote({ id: 'x', path: 'p' }), null);
  assert.equal(m.parseNote(rawNote({ at_head: null })).at_head, null, 'the server says it is not in the diff');
  const moved = m.parseNote(rawNote({ seen_sha: NEXT, at_head: { start: { side: 'new', line: 44 }, end: { side: 'new', line: 44 } } }));
  assert.deepEqual([moved.moved, moved.gone, moved.at_head.end.line, moved.seen_sha], [true, false, 44, NEXT]);
  const gone = m.parseNote(rawNote({ seen_sha: NEXT, at_head: null }));
  assert.deepEqual([gone.gone, gone.at_head, gone.moved], [true, null, false]);
  assert.equal(m.parseNote(rawNote({ state: 'resolved', seen_sha: NEXT, at_head: null })).gone, false, 'only drafts are gone');
  assert.equal(m.parseNote(rawNote({ state: 'nope' })).state, 'draft');
  assert.equal(m.parseNote(rawNote({ text: '<b>hi</b>' })).text, '<b>hi</b>', 'text stays text');
});

test('parseNotes, parseSendResult and the card counts', () => {
  const p = m.parseNotes({ notes: [rawNote(), { bad: 1 }], summary: { total: 3, changed: 2, open: 1, sha: NEXT } });
  assert.equal(p.notes.length, 1);
  assert.deepEqual(p.summary, { total: 3, changed: 2, open: 1, sha: NEXT });
  assert.equal(m.parseNotes({ notes: [] }).summary, null);
  assert.equal(m.parseNotes({}), null);
  const r = m.parseSendResult({ sent: 2, message_id: 'msg', kwant_id: 'bot_vega', kwant_name: 'Vega', sent_at: 'x', notes: [rawNote({ state: 'sent' })] });
  assert.deepEqual([r.sent, r.notes[0].state], [2, 'sent']);
  assert.equal(m.parseSendResult({ sent: -1 }), null);
  assert.deepEqual(m.parseCardChanges({ files: 3, additions: 43, deletions: 13 }), { files: 3, additions: 43, deletions: 13 });
  assert.equal(m.parseCardChanges({ files: 1.5 }), null);
  assert.equal(m.parseCardChanges(undefined), null);
  assert.deepEqual(m.parseCardNotes({ drafts: 2, total: 5 }), { drafts: 2, total: 5 });
  assert.equal(m.parseCardNotes({}), null);
});

test('parseReview: head_sha, changes and notes (counts) are optional; an older card has none and no Changes section', () => {
  const older = m.parseReview(m.fx.REVIEW_312 ?? m.fx.REVIEW_289);
  assert.deepEqual([older.head_sha, older.changes, older.line_notes], [null, null, null]);
  const raw = { ...(m.fx.REVIEW_312 ?? m.fx.REVIEW_289), head_sha: HEAD, changes: { files: 3, additions: 43, deletions: 13 }, notes: { drafts: 1, total: 1 } };
  const c = m.parseReview(raw);
  assert.deepEqual([c.head_sha, c.changes.files, c.line_notes.drafts], [HEAD, 3, 1]);
});

// ---------------------------------------------------------------------------------------------------
// Lines, ranges and copy

test('clampRange stays in the anchor hunk, orders the ends and caps at 40 lines', () => {
  const lines = m.flatLines(m.parseChanges(rawChanges()).files[0]);
  assert.equal(lines.length, 8);
  assert.deepEqual(m.clampRange(lines, 2, 4), [2, 4]);
  assert.deepEqual(m.clampRange(lines, 4, 1), [1, 4]);
  assert.deepEqual(m.clampRange(lines, 3, 7), [3, 5], 'stops at the end of its hunk');
  assert.deepEqual(m.clampRange(lines, 6, 0), [6, 6], 'does not cross into the hunk above');
  const long = m.flatLines(m.parseChanges(rawChanges({ files: [{ path: 'a', hunks: [{ header: '', lines: Array.from({ length: 60 }, (_, i) => ({ kind: 'add', new: i + 1, text: 'x' })) }] }] })).files[0]);
  assert.deepEqual(m.clampRange(long, 0, 59), [0, 39]);
  assert.deepEqual(m.clampRange(long, 59, 0), [20, 59]);
});

test('positionIn: a note sits where at_head says, even across a removed and an added line', () => {
  const lines = m.flatLines(m.parseChanges(rawChanges()).files[0]);
  assert.deepEqual(m.positionIn(lines, m.parseNote(rawNote())), [3, 3]);
  const placed = m.parseNote(rawNote({ head_sha: 'old', at_head: { start: { side: 'old', line: 41 }, end: { side: 'new', line: 42 } } }));
  assert.deepEqual(m.positionIn(lines, placed), [1, 3], 'a range across a removed and an added line');
  assert.equal(m.positionIn(lines, m.parseNote(rawNote({ at_head: { start: { side: 'new', line: 999 }, end: { side: 'new', line: 999 } } }))), null);
  assert.equal(m.positionIn(lines, m.parseNote(rawNote({ at_head: null }))), null);
});

test('copy: Mira’s words, singular for 1, short shas, no emoji', () => {
  assert.equal(m.sendLabel(1, 'Vega'), 'Send note to Vega');
  assert.equal(m.sendLabel(3, 'Vega'), 'Send 3 notes to Vega');
  assert.equal(m.sendShort(3), 'Send 3');
  assert.equal(m.sentLine(3, 'Vega', '09:30'), 'Sent 3 notes to Vega · 09:30');
  assert.equal(m.discardAsk(1), 'Discard 1 draft?');
  assert.equal(m.discardAsk(4), 'Discard 4 drafts?');
  assert.equal(m.changedIn(NEXT), 'Changed in e81f2c4');
  assert.equal(m.stillOpen(NEXT), 'Still open · line unchanged in e81f2c4');
  assert.equal(m.lineWords({ side: 'new', line: 40 }, { side: 'new', line: 42 }), 'lines 40–42');
  assert.equal(m.lineWords({ side: 'new', line: 40 }, { side: 'new', line: 40 }), 'line 40');
  assert.equal(m.addAria({ kind: 'del', old: 41, new: null, text: '' }), 'Add a note on removed line 41');
  assert.equal(m.summaryLine({ total: 3, changed: 2, open: 1, sha: NEXT }), '3 notes · 2 changed in e81f2c4 · 1 still open');
  assert.equal(m.movedLine('Vega', NEXT, 44), 'Vega pushed e81f2c4 after you wrote this. The line is now 44.');
  assert.equal(m.goneLine(NEXT), 'This line is gone in e81f2c4.');
  assert.equal(m.cutLine({ files_not_read: 1, text: null }), '1 file not read: the diff was cut.');
  assert.equal(m.showLines(1284), 'Show 1,284 lines');
  assert.equal(m.showLines(1), 'Show 1 line');
  assert.ok(m.startsCollapsed({ path: 'web/package-lock.json', big: false }));
  assert.ok(!m.startsCollapsed({ path: 'src/lock.rs', big: false }));
  const all = [m.SEND_FAILED, m.TOO_MANY, m.LOAD_FAILED, m.NO_DIFF, m.notBotLine('oleks'), m.readOnlyLine('merged'), m.sentNoteLine('09:30', 'Vega')].join(' ');
  assert.doesNotMatch(all, /\p{Extended_Pictographic}/u);
});

// ---------------------------------------------------------------------------------------------------
// The API client

test('lineNotesApi: routes, bodies and send_id; errors carry the server code', async () => {
  const seen = [];
  const answers = {
    'GET /api/dev/cards/dvr_312/changes': rawChanges(),
    'GET /api/dev/cards/dvr_312/notes': { notes: [rawNote()] },
    'POST /api/dev/cards/dvr_312/notes': rawNote({ id: 'ln_9' }),
    'PATCH /api/dev/notes/ln_9': rawNote({ id: 'ln_9', text: 'new' }),
    'DELETE /api/dev/notes/ln_9': { ok: true },
    'POST /api/dev/notes/ln_9/restore': rawNote({ id: 'ln_9' }),
    'DELETE /api/dev/cards/dvr_312/notes': { discarded: 2 },
    'POST /api/dev/cards/dvr_312/notes/send': { sent: 1, notes: [] },
  };
  const api = m.lineNotesApi(async (url, method, body) => {
    seen.push([method, url, body]);
    return answers[`${method} ${url}`];
  });
  assert.equal((await api.changes('dvr_312')).files.length, 3);
  assert.equal((await api.notes('dvr_312')).notes.length, 1);
  await api.create('dvr_312', { path: 'a.rs', start: { side: 'old', line: 41 }, end: { side: 'new', line: 42 }, head_sha: HEAD, text: 'x' });
  assert.deepEqual(seen.at(-1)[2], { path: 'a.rs', start: { side: 'old', line: 41 }, end: { side: 'new', line: 42 }, head_sha: HEAD, text: 'x' });
  assert.equal((await api.edit('ln_9', 'new')).text, 'new');
  await api.remove('ln_9');
  assert.equal((await api.restore('ln_9')).id, 'ln_9');
  assert.equal(await api.discardAll('dvr_312'), 2);
  await api.send('dvr_312', 'snd_1');
  assert.deepEqual(seen.at(-1)[2], { send_id: 'snd_1' });
  await api.send('dvr_312', 'snd_2', ['ln_9']);
  assert.deepEqual(seen.at(-1)[2], { ids: ['ln_9'], send_id: 'snd_2' });
  await assert.rejects(m.lineNotesApi(async () => ({ nope: 1 })).notes('x'), (e) => e.code === 'bad_shape');

  const http = m.fetchCaller(async () => new Response(JSON.stringify({ error: 'stale_head', code: 'stale_head' }), { status: 409 }));
  await assert.rejects(http('/x', 'POST', {}), (e) => e instanceof m.LineNotesError && e.status === 409 && e.code === 'stale_head' && /pushed again/.test(e.message));
  const down = m.fetchCaller(async () => {
    throw new TypeError('fetch failed');
  });
  await assert.rejects(down('/x', 'GET'), (e) => e.status === 0 && /reach the server/.test(e.message));
  const plain = m.fetchCaller(async () => new Response(JSON.stringify({ error: 'The PR was force-pushed.' }), { status: 422 }));
  await assert.rejects(plain('/x', 'GET'), (e) => e.message === 'The PR was force-pushed.' && e.code === null);
  assert.notEqual(m.newSendId(), m.newSendId());
});

// ---------------------------------------------------------------------------------------------------
// The section

test('collapsed: the card’s own counts, no fetch; Show loads the diff and Hide keeps it', async () => {
  const { api, calls } = fakeApi();
  await mount({ api, notesCount: { drafts: 2, total: 2 } });
  const head = $('h4').textContent;
  assert.match(head, /Changes/);
  assert.match(head, /3 files/);
  assert.match(head, /2 notes/);
  assert.equal(calls.length, 0, 'nothing is fetched until it opens');
  const toggle = byText('Show the changes');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  await click(toggle);
  assert.equal(toggle.getAttribute('aria-expanded'), 'true');
  assert.deepEqual(calls.slice().sort(), ['changes', 'notes']);
  assert.ok($('[data-ln-grid="src/server/auth.rs"]'));
  assert.equal($('[data-ln-grid="src/server/auth.rs"]').getAttribute('aria-label'), 'Changes in src/server/auth.rs');
  assert.ok(!$('[data-ln-grid="Cargo.lock"]'), 'lockfiles start collapsed');
  assert.match($('[data-ln-file="docs/logo.png"]').textContent, /Binary file, not shown/);
  await click(byText('Hide the changes'));
  assert.ok(document.getElementById(toggle.getAttribute('aria-controls')).hidden);
  await click(byText('Show the changes'));
  assert.equal(calls.length, 2, 'opening again does not refetch');
});

test('load error: Couldn’t load the changes, Try again refetches', async () => {
  const { api, s, calls } = fakeApi();
  s.fail.changes = 1;
  await mount({ api, defaultOpen: true });
  assert.match($('[data-ln-error]').textContent, /Couldn’t load the changes\./);
  await click(byText('Try again'));
  assert.ok(!$('[data-ln-error]'));
  assert.equal(calls.filter((c) => c === 'changes').length, 2);
});

test('gutter +: one tab stop per file, arrows move, Enter opens the box; ⌘/Ctrl+Enter adds a draft', async () => {
  const { api, calls } = fakeApi();
  await mount({ api, defaultOpen: true });
  const grid = $('[data-ln-grid="src/server/auth.rs"]');
  const tabbable = [...grid.querySelectorAll('button')].filter((b) => b.tabIndex === 0);
  assert.equal(tabbable.length, 1, 'roving tabindex');
  assert.equal(plus('old', 41).getAttribute('aria-label'), 'Add a note on removed line 41');
  tabbable[0].focus();
  await key(tabbable[0], 'ArrowDown');
  await key(document.activeElement, 'ArrowDown');
  await key(document.activeElement, 'ArrowDown');
  assert.equal(document.activeElement, plus('new', 42));
  await click(document.activeElement);
  const box = $('[data-ln-writing]');
  assert.ok(box, 'the writing box opens under the line');
  assert.match(box.textContent, /Note for Vega line 42/);
  const ta = box.querySelector('textarea');
  assert.equal(byText('Add note').disabled, true, 'nothing to add yet');
  await type(ta, 'Use a constant.');
  await key(ta, 'Enter', { ctrlKey: true });
  const create = calls.find((c) => Array.isArray(c) && c[0] === 'create')[1];
  assert.deepEqual([create.path, create.start, create.end, create.head_sha, create.text], ['src/server/auth.rs', { side: 'new', line: 42 }, { side: 'new', line: 42 }, HEAD, 'Use a constant.']);
  assert.ok(!$('[data-ln-writing]'));
  const note = $('[data-ln-state="draft"]');
  assert.ok(note);
  assert.equal(note.getAttribute('aria-label'), 'Draft note on line 42');
  assert.match($('[data-ln-live]').textContent, /Note added on line 42\. 1 note for Vega\./);
  assert.match(plus('new', 42).getAttribute('aria-label'), /It has a note/);
});

test('ranges: Shift+Down widens inside the hunk, Shift-click ends it; the box says lines a–b', async () => {
  const { api, calls } = fakeApi();
  await mount({ api, defaultOpen: true });
  const first = plus('new', 41);
  first.focus();
  first.dispatchEvent(new window.FocusEvent('focus', { bubbles: true }));
  await settle();
  // Focus the line, then widen.
  await click(first);
  await click(byText('Cancel'));
  plus('new', 41).focus();
  await key(plus('new', 41), 'ArrowDown', { shiftKey: true });
  await key(document.activeElement, 'ArrowDown', { shiftKey: true });
  assert.deepEqual($$('[aria-selected="true"]').map((r) => r.dataset.lnPoint), ['new:41', 'new:42', 'new:43']);
  await click(document.activeElement);
  assert.match($('[data-ln-writing]').textContent, /lines 41–43/);
  await type($('[data-ln-writing] textarea'), 'Split this.');
  await click(byText('Add note'));
  const create = calls.filter((c) => Array.isArray(c) && c[0] === 'create').at(-1)[1];
  assert.deepEqual([create.start.line, create.end.line], [41, 43]);
  // Shift-click from a selected line stops at the hunk's end.
  await click(plus('new', 40));
  await click(plus('new', 93), { shiftKey: true });
  assert.match($('[data-ln-writing]').textContent, /lines 40–44/, 'a range never crosses hunks');
});

test('Esc in a box with text asks first; Keep writing returns, Throw away closes', async () => {
  const { api } = fakeApi();
  await mount({ api, defaultOpen: true });
  await click(plus('new', 43));
  const ta = $('[data-ln-writing] textarea');
  await type(ta, 'half a thought');
  await key(ta, 'Escape');
  assert.equal($('[role="alertdialog"]').getAttribute('aria-label'), 'Throw away this note?');
  assert.equal(document.activeElement, byText('Keep writing'), 'the safe choice has focus');
  await click(byText('Keep writing'));
  assert.ok(!$('[role="alertdialog"]'));
  assert.equal($('[data-ln-writing] textarea').value, 'half a thought');
  await key($('[data-ln-writing] textarea'), 'Escape');
  await click(byText('Throw away'));
  assert.ok(!$('[data-ln-writing]'));
  await click(plus('new', 43));
  await key($('[data-ln-writing] textarea'), 'Escape');
  assert.ok(!$('[data-ln-writing]'), 'an empty box closes at once');
});

test('send bar: Send {n} notes to {bot}, one send with a send_id, then Sent with Open chat', async () => {
  const { api, calls } = fakeApi({ notes: [rawNote(), rawNote({ id: 'ln_2', start: { side: 'new', line: 43 }, end: { side: 'new', line: 43 } })] });
  const opened = [];
  await mount({ api, defaultOpen: true, onOpenChat: (id) => opened.push(id) });
  const bar = $('[data-ln-bar]');
  assert.equal(bar.getAttribute('aria-label'), 'Notes to send');
  assert.match(bar.textContent, /2 notes for Vega · drafts, only you see them/);
  const send = $('[data-ln-send]');
  assert.equal(send.textContent.trim(), 'Send 2 notes to Vega');
  await click(send);
  const sends = calls.filter((c) => Array.isArray(c) && c[0] === 'send');
  assert.equal(sends.length, 1);
  assert.ok(sends[0][1], 'a send id');
  assert.equal($('[data-ln-bar]').dataset.lnBar, 'sent');
  assert.match($('[data-ln-bar]').textContent, /Sent 2 notes to Vega/);
  assert.equal($$('[data-ln-state="sent"]').length, 2);
  assert.match($('[data-ln-state="sent"]').textContent, /waiting for Vega’s next push/);
  await click(byText('Open Vega’s chat'));
  assert.deepEqual(opened, ['bot_vega']);
});

test('a failed send keeps the drafts; Try again reuses the same send_id', async () => {
  const { api, s, calls } = fakeApi({ notes: [rawNote()] });
  s.fail.send = 1;
  await mount({ api, defaultOpen: true });
  await click($('[data-ln-send]'));
  assert.equal($('[data-ln-bar]').dataset.lnBar, 'failed');
  assert.match($('[data-ln-bar]').textContent, /The notes didn’t send\. They’re still here\./);
  assert.equal($$('[data-ln-state="draft"]').length, 1);
  await click(byText('Try again'));
  const sends = calls.filter((c) => Array.isArray(c) && c[0] === 'send');
  assert.equal(sends.length, 2);
  assert.equal(sends[0][1], sends[1][1], 'same send id, so the batch posts once');
  assert.equal($('[data-ln-bar]').dataset.lnBar, 'sent');
});

test('more than 20 drafts: Send is off and the bar says why', async () => {
  const many = Array.from({ length: 21 }, (_, i) => rawNote({ id: `ln_${i}`, start: { side: 'new', line: 41 + (i % 3) }, end: { side: 'new', line: 41 + (i % 3) } }));
  const { api } = fakeApi({ notes: many });
  await mount({ api, defaultOpen: true });
  assert.equal($('[data-ln-send]').disabled, true);
  assert.match($('[data-ln-bar]').textContent, /Up to 20 notes per send/);
});

test('Discard all asks first; Keep keeps them', async () => {
  const { api, calls } = fakeApi({ notes: [rawNote(), rawNote({ id: 'ln_2' })] });
  await mount({ api, defaultOpen: true });
  await click(byText('Discard all'));
  assert.match($('[data-ln-bar]').textContent, /Discard 2 drafts\?/);
  assert.equal(document.activeElement, byText('Keep'));
  await click(byText('Keep'));
  assert.equal($$('[data-ln-state="draft"]').length, 2);
  await click(byText('Discard all'));
  await click(byText('Discard'));
  assert.ok(calls.includes('discardAll'));
  assert.equal($$('[data-ln-state="draft"]').length, 0);
  assert.ok(!$('[data-ln-bar]'), 'no drafts, no bar');
});

test('a draft: E edits in place, Delete deletes with a 5 s Undo that restores it', async () => {
  const { api, calls } = fakeApi({ notes: [rawNote()] });
  await mount({ api, defaultOpen: true });
  const note = $('[data-ln-note="ln_1"]');
  assert.equal(note.tabIndex, 0);
  note.focus();
  await key(note, 'e');
  const ta = $('[data-ln-writing] textarea');
  assert.equal(ta.value, 'Use a constant for the minimum length.');
  await type(ta, 'Use MIN_TOKEN_LEN.');
  await click(byText('Save'));
  assert.ok(calls.includes('edit'));
  assert.match($('[data-ln-note="ln_1"]').textContent, /Use MIN_TOKEN_LEN\./);
  await key($('[data-ln-note="ln_1"]'), 'Delete');
  assert.ok(!$('[data-ln-note="ln_1"]'));
  assert.match($('[data-ln-undo]').textContent, /Note deleted/);
  await click(byText('Undo'));
  assert.ok(calls.includes('restore'));
  assert.ok($('[data-ln-note="ln_1"]'));
});

test('after the bot’s push: Changed in {sha} and Still open, with the summary line', async () => {
  const { api } = fakeApi({
    notes: [
      rawNote({ state: 'resolved', checked_sha: NEXT, sent_at: '2026-11-10T09:30:00Z' }),
      rawNote({ id: 'ln_2', start: { side: 'new', line: 43 }, end: { side: 'new', line: 43 }, state: 'open', checked_sha: NEXT }),
    ],
    summary: { total: 2, changed: 1, open: 1, sha: NEXT },
  });
  await mount({ api, defaultOpen: true });
  assert.equal($('[data-ln-summary]').textContent, '2 notes · 1 changed in e81f2c4 · 1 still open');
  const done = $('[data-ln-state="resolved"]');
  assert.match(done.textContent, /Changed in e81f2c4/);
  assert.equal(done.getAttribute('aria-label'), 'Note on line 42, Changed in e81f2c4');
  assert.match($('[data-ln-state="open"]').textContent, /Still open · line unchanged in e81f2c4/);
  assert.equal(done.tabIndex, -1, 'sent notes are not editable');
  assert.ok(!$('[data-ln-bar]'), 'no drafts, no bar');
});

test('a dev_card event for this card refetches the notes and announces the result', async () => {
  const { api, s, calls } = fakeApi({ notes: [rawNote({ state: 'sent' })], summary: { total: 1, changed: 0, open: 0, sha: HEAD } });
  await mount({ api, defaultOpen: true });
  s.notes = [m.parseNote(rawNote({ state: 'resolved', checked_sha: NEXT }))];
  s.summary = { total: 1, changed: 1, open: 0, sha: NEXT };
  window.dispatchEvent(new window.CustomEvent('kwant:dev-card', { detail: { id: 'other' } }));
  await settle();
  assert.equal(calls.filter((c) => c === 'notes').length, 1, 'another card: nothing');
  window.dispatchEvent(new window.CustomEvent('kwant:dev-card', { detail: { id: 'dvr_312' } }));
  await settle();
  assert.equal(calls.filter((c) => c === 'notes').length, 2);
  assert.ok($('[data-ln-state="resolved"]'));
  assert.equal($('[data-ln-live]').textContent, '1 note changed in e81f2c4, 0 still open.');
});

test('a draft whose line moved or went: it says so; a gone draft can be sent alone', async () => {
  const { api, calls } = fakeApi({
    notes: [
      rawNote({ seen_sha: NEXT, at_head: { start: { side: 'new', line: 43 }, end: { side: 'new', line: 43 } } }),
      rawNote({ id: 'ln_2', seen_sha: NEXT, at_head: null }),
    ],
  });
  await mount({ api, defaultOpen: true });
  assert.match($('[data-ln-note="ln_1"]').textContent, /Vega pushed e81f2c4 after you wrote this\. The line is now 43\./);
  const gone = $('[data-ln-note="ln_2"]');
  assert.match(gone.textContent, /This line is gone in e81f2c4\./);
  await click([...gone.querySelectorAll('button')].find((b) => b.textContent === 'Send it anyway'));
  const send = calls.find((c) => Array.isArray(c) && c[0] === 'send');
  assert.deepEqual(send[2], ['ln_2']);
});

test('read-only: a merged PR and a PR not by a bot have no + and no bar', async () => {
  const { api } = fakeApi({ changes: rawChanges({ pr_state: 'merged' }), notes: [rawNote({ state: 'sent' })] });
  await mount({ api, defaultOpen: true });
  assert.match($('[data-ln-banner]').textContent, /This PR is merged\. Notes are read-only\./);
  assert.equal($$('[data-ln-grid] button').length, 0);
  assert.ok(!$('[data-ln-bar]'));
  assert.equal($('[data-ln-grid]').tabIndex, 0, 'the read-only diff still scrolls by keyboard');

  const other = fakeApi({ changes: rawChanges({ author_bot: null, author_login: 'oleks' }) });
  await mount({ api: other.api, defaultOpen: true });
  assert.equal($('[data-ln-banner]').textContent, 'Notes go to bots in this space. This PR is by oleks, so there’s nothing to send them to.');
  assert.equal($('[data-ln-banner] strong').textContent, 'oleks');
  assert.equal($$('[data-ln-grid] button').length, 0);
});

test('a stale head on create keeps the text and reloads the diff', async () => {
  const { api, calls } = fakeApi();
  api.create = async () => {
    calls.push('create');
    throw new m.LineNotesError('The bot pushed again. The changes reloaded; your note is still here.', 409, 'stale_head');
  };
  await mount({ api, defaultOpen: true });
  await click(plus('new', 42));
  await type($('[data-ln-writing] textarea'), 'keep me');
  await click(byText('Add note'));
  assert.equal($('[data-ln-writing] [role="alert"]').textContent, 'The bot pushed again. The changes reloaded; your note is still here.');
  assert.equal($('[data-ln-writing] textarea').value, 'keep me');
  assert.equal(calls.filter((c) => c === 'changes').length, 2);
});

test('phone: tapping a line opens a sheet; the bar is short with Discard in a menu', async () => {
  const { api } = fakeApi({ notes: [rawNote()] });
  await mount({ api, defaultOpen: true, phone: true });
  assert.equal($('[data-ln-send]').textContent.trim(), 'Send 1');
  assert.equal($('[data-ln-send]').getAttribute('aria-label'), 'Send note to Vega');
  assert.ok(!byText('Discard all'));
  await click($('[aria-label="More"]'));
  assert.ok(byText('Discard all'));
  await click(plus('new', 43));
  const sheet = $('[data-ln-sheet]');
  assert.equal(sheet.getAttribute('role'), 'dialog');
  assert.match(sheet.textContent, /Note for Vega/);
  assert.match(sheet.textContent, /auth\.rs · line 43/);
  assert.ok(!$('[data-ln-writing]'), 'no inline box on the phone');
  await click(byText('Extend to line…'));
  assert.match($('[data-ln-range-bar]').textContent, /tap the last line/);
  await click(plus('new', 42));
  // Tapping above the anchor makes the range 42–43.
  assert.match($('[data-ln-sheet]').textContent, /lines 42–43/);
});

test('the review card shows Changes only with an api and the card’s counts', async () => {
  const raw = { ...(m.fx.REVIEW_312 ?? m.fx.REVIEW_289), head_sha: HEAD, changes: { files: 3, additions: 43, deletions: 13 } };
  const { api } = fakeApi();
  const render = async (props) => {
    if (root) root.unmount();
    host?.remove();
    host = document.createElement('div');
    document.body.appendChild(host);
    root = createRoot(host);
    root.render(h(m.ReviewCard, { onFinding: async () => raw, ...props }));
    await settle();
  };
  await render({ card: raw, lineNotes: api });
  assert.ok($('[data-line-notes]'));
  await render({ card: raw });
  assert.ok(!$('[data-line-notes]'), 'no api, no section');
  await render({ card: { ...raw, changes: undefined }, lineNotes: api });
  assert.ok(!$('[data-line-notes]'), 'an older server: no section');
  root.unmount();
  root = null;
});
