// pa4c-ui (autonomy/pocket-apps/pa4c-ui/brief.md): Edit questions on Juno's draft routes and the pocket row in
// Inbox › Done. Header and chips from a draft, base on every edit, 409 changed keeps the typed text and offers
// Reload, 422 lists the problems, open and closed are read only, the card swaps in the reply's card, the Done
// row (title, players, Winners only with a winner, opens its chat at the results), and with the switch off no
// Edit button and no pocket row.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'InputEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
let phone = false;
g.window.matchMedia = (q) => ({ matches: phone && /max-width/.test(q), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
g.matchMedia = g.window.matchMedia;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { Pocket, validPocket, earnedChecked, withCard, LATER } from './src/components/results/Pocket';
  export { EditPanel, EDIT_TEXT, editBody } from './src/components/results/PocketEdit';
  export { POCKET_FIXTURES, quizDraftFixture, draftCardFixture } from './src/components/results/fixtures';
  export { pocketApi, PocketApiError } from './src/pocketApi';
  export * as fx from './src/doneAwayFixture';
  export * as store from './src/inboxStore';
  export { InboxNew, pocketMeta, pocketEndedAt } from './src/components/InboxNew';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  g.fetch = realFetch;
  phone = false;
  document.body.innerHTML = '';
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
/** React's controlled inputs: set through the native setter, then fire input. */
const type = async (el, value) => {
  const proto = el.tagName === 'TEXTAREA' ? dom.window.HTMLTextAreaElement.prototype : dom.window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
  el.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle(4);
};
const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const F = m.POCKET_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const card = (name, over = {}) => mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [{ ...raw(name), ...over }] }, messageId: `m-${name}` }));
const byText = (root, text, sel = 'button, a') => [...root.querySelectorAll(sel)].find((b) => b.textContent.trim() === text);
const holos = (root) => root.querySelectorAll('.kw-sticker-holo, [data-tone="holo"]').length;

/**
 * A stateful fake of Juno's four routes. `next` lets one test answer a single request its own way.
 */
function fakeServer(over = {}) {
  let d = m.quizDraftFixture(over);
  const calls = [];
  const s = { calls, next: null, get draft() { return d; } };
  const reply = () => json({ draft: d, card: m.draftCardFixture(d) });
  const bump = () => {
    d.updated_at += 1000;
    const yours = d.questions.filter((q) => q.status === 'yours').length;
    d.counts = { questions: d.questions.length, checked: d.questions.length - yours, yours, not_checked: 0 };
    d.header = `${d.questions.length} questions · ${d.counts.checked} checked · ${yours} yours`;
  };
  g.fetch = async (url, init = {}) => {
    const u = new URL(String(url), 'http://localhost');
    const body = init.body ? JSON.parse(init.body) : null;
    const method = init.method ?? 'GET';
    calls.push({ method, path: u.pathname, search: u.search, body });
    if (s.next) {
      const r = s.next;
      s.next = null;
      return r({ method, u, body });
    }
    const mq = u.pathname.match(/^\/api\/quiz-drafts\/([^/]+)(?:\/questions(?:\/(\d+))?)?$/);
    if (!mq) return json({ error: 'not_found', message: 'Not found.' }, 404);
    if (method === 'GET') return reply();
    const n = mq[2] ? Number(mq[2]) : null;
    if (method === 'PATCH') {
      const q = d.questions.find((x) => x.n === n);
      Object.assign(q, Object.fromEntries(Object.entries(body).filter(([k]) => k !== 'base')), { status: 'yours', source_url: null });
    } else if (method === 'POST') {
      const at = d.questions.filter((x) => x.round <= body.round).length;
      d.questions.splice(at, 0, { n: 0, round: body.round, at: `R${body.round}`, kind: body.kind ?? 'text', question: body.question, answer: body.answer, accepted: body.accepted ?? [], status: 'yours', source_url: null, why: null, picture: null });
      d.questions.forEach((x, i) => (x.n = i + 1));
    } else if (method === 'DELETE') {
      d.questions = d.questions.filter((x) => x.n !== n);
      d.questions.forEach((x, i) => (x.n = i + 1));
    }
    bump();
    return reply();
  };
  return s;
}

const openEditor = async (over = {}, name = 'ready') => {
  const v = await card(name, { draft: 'qd_fq7k2m', ...over });
  const b = byText(v.host, 'Edit questions');
  assert.ok(b, 'a real Edit questions button');
  assert.equal(b.getAttribute('aria-disabled'), null, 'not disabled');
  await click(b);
  return v;
};
const panel = (v) => document.querySelector('[data-pocket-edit]');
const row = (n) => document.querySelector(`[data-q="${n}"]`);

// ---------------------------------------------------------------------------------------------------

test('header and chips come from the draft: "N questions · N checked · N yours", Checked on checked rows, Yours on own', async () => {
  const s = fakeServer({ yours: [2] });
  const v = await openEditor({ yours: 1 });
  assert.deepEqual(s.calls[0], { method: 'GET', path: '/api/quiz-drafts/qd_fq7k2m', search: s.calls[0].search, body: null });
  assert.equal(panel(v).querySelector('[data-header]').textContent, '8 questions · 7 checked · 1 yours');
  assert.match(panel(v).querySelector('h3').textContent, /Edit questions · Friday Pub Quiz/);
  // Round 1 is shown first: Q1 checked, Q2 yours.
  assert.equal(row(1).querySelector('[data-status]').dataset.status, 'checked');
  assert.equal(row(1).querySelector('[data-status]').textContent.trim(), 'Checked');
  assert.equal(row(2).querySelector('[data-status]').textContent.trim(), 'Yours, not checked');
  // Chips are plain: no holo anywhere in the editor.
  assert.equal(holos(panel(v)), 0);
  // The round list names each round and its count; a tap shows that round.
  const r3 = byText(panel(v), 'Round 3 · Pictures3');
  assert.ok(r3);
  await click(r3);
  assert.ok(row(6) && !row(1), 'round 3 rows only');
  assert.match(row(6).textContent, /Answer Kiwi· alsoKiwisKiwi bird/);
  // With a yours question the card's Checked holo is not earned and the check row says so.
  assert.equal(holos(v.host), 0);
  assert.match(v.host.textContent, /· 1 yours, not checked/);
});

test('edit one answer sends base and only what changed; the row turns Yours, the header counts it, the card swaps in the reply', async () => {
  const s = fakeServer();
  const v = await openEditor();
  assert.equal(holos(v.host), 1, 'Checked holo before the edit');
  await click(row(1).querySelector('[data-edit="1"]'));
  const form = document.querySelector('[data-edit-form="1"]');
  assert.ok(form);
  const answer = [...form.querySelectorAll('input')].find((i) => i.value === 'Fleetwood Mac');
  await type(answer, 'Fleetwood Mac (band)');
  await click(byText(form, 'Save'));
  const patch = s.calls.find((c) => c.method === 'PATCH');
  assert.equal(patch.path, '/api/quiz-drafts/qd_fq7k2m/questions/1');
  assert.deepEqual(patch.body, { answer: 'Fleetwood Mac (band)', base: 1_763_900_000_000 });
  assert.equal(row(1).querySelector('[data-status]').textContent.trim(), 'Yours, not checked');
  assert.equal(row(1).dataset.changed, 'true');
  assert.equal(panel(v).querySelector('[data-header]').textContent, '8 questions · 7 checked · 1 yours');
  assert.match(panel(v).querySelector('[data-saved]').textContent, /Saved\. The dry run passed again/);
  // The card is the reply's card: still Ready, one yours, so no Checked holo now.
  assert.equal(v.host.querySelector('[data-pocket]').dataset.pocket, 'ready');
  assert.match(v.host.textContent, /7\/7 answers checked against a source · 1 yours, not checked/);
  assert.equal(holos(v.host), 0);
  // The next edit sends the new base.
  await click(row(1).querySelector('[data-edit="1"]'));
  const f2 = document.querySelector('[data-edit-form="1"]');
  await type(f2.querySelector('textarea'), 'Which band made "Rumours"?');
  await click(byText(f2, 'Save'));
  assert.equal(s.calls.filter((c) => c.method === 'PATCH')[1].body.base, 1_763_900_001_000);
});

test('add and delete a question, each with base; delete asks first', async () => {
  const s = fakeServer();
  const v = await openEditor();
  await click(byText(panel(v), 'Add question to round 1'));
  const form = document.querySelector('[data-edit-form="add-1"]');
  await type(form.querySelector('textarea'), 'Who sang "Respect"?');
  const [answer] = [...form.querySelectorAll('input:not([type="radio"])')].filter((i) => !i.placeholder);
  await type(answer, 'Aretha Franklin');
  await click(byText(form, 'Save'));
  const post = s.calls.find((c) => c.method === 'POST');
  assert.equal(post.path, '/api/quiz-drafts/qd_fq7k2m/questions');
  assert.deepEqual(post.body, { round: 1, kind: 'text', question: 'Who sang "Respect"?', answer: 'Aretha Franklin', accepted: [], base: 1_763_900_000_000 });
  assert.equal(panel(v).querySelector('[data-header]').textContent, '9 questions · 8 checked · 1 yours');
  assert.equal(row(4).dataset.changed, 'true', 'the new question is marked');
  assert.ok(row(4).querySelector('[data-own-icon]'), 'own questions show the pen, not the book');
  assert.equal(row(1).querySelector('[data-own-icon]'), null, 'checked questions keep the book');
  // Delete Q2: asks first, then sends ?base=.
  await click(row(2).querySelector('[data-edit="2"]'));
  await click(byText(document.querySelector('[data-edit-form="2"]'), 'Delete question'));
  assert.equal(s.calls.filter((c) => c.method === 'DELETE').length, 0, 'nothing deleted before yes');
  assert.equal(document.querySelector('[role="group"][aria-label="Delete Q2"] p').textContent, 'Delete Q2? The questions after it move up one. Deleting re-runs the dry run (about 30 s).');
  await click(document.querySelector('[data-action="delete-yes"]'));
  const del = s.calls.find((c) => c.method === 'DELETE');
  assert.equal(del.path, '/api/quiz-drafts/qd_fq7k2m/questions/2');
  assert.equal(new URLSearchParams(del.search).get('base'), '1763900001000');
  assert.equal(panel(v).querySelector('[data-header]').textContent, '8 questions · 7 checked · 1 yours');
  assert.equal(row(3).dataset.changed, 'true', 'the added question moved up and keeps its mark');
});

test('409 changed: Reload shows and the typed text is kept; after Reload the save goes with the new base', async () => {
  const s = fakeServer();
  const v = await openEditor();
  await click(row(1).querySelector('[data-edit="1"]'));
  const form = () => document.querySelector('[data-edit-form="1"]');
  await type(form().querySelector('textarea'), 'My own wording');
  s.next = () => json({ error: 'changed', message: 'This quiz changed since you opened it.' }, 409);
  await click(byText(form(), 'Save'));
  const stale = panel(v).querySelector('[data-stale]');
  assert.ok(stale);
  assert.match(stale.textContent, /The Quizmaster changed this quiz\. Reload to see the new version/);
  assert.equal(form().querySelector('textarea').value, 'My own wording', 'typed text kept');
  // The Quizmaster moved on meanwhile.
  s.draft.updated_at = 1_763_900_050_000;
  await click(byText(stale, 'Reload'));
  assert.equal(panel(v).querySelector('[data-stale]'), null);
  assert.equal(form().querySelector('textarea').value, 'My own wording', 'still kept after Reload');
  await click(byText(form(), 'Save'));
  const patches = s.calls.filter((c) => c.method === 'PATCH');
  assert.equal(patches.at(-1).body.base, 1_763_900_050_000);
  assert.equal(patches.at(-1).body.question, 'My own wording');
});

test('422 shows the numbered problems under the question and keeps the form open', async () => {
  const s = fakeServer();
  await openEditor();
  await click(row(2).querySelector('[data-edit="2"]'));
  const form = document.querySelector('[data-edit-form="2"]');
  await type([...form.querySelectorAll('input')].find((i) => i.value === '4'), '');
  s.next = () => json({ error: 'invalid', message: "That question can't be saved yet.", problems: ['Q2: the answer is empty.', 'Q2: a number answer needs a number.'] }, 422);
  await click(byText(form, 'Save'));
  const alert = document.querySelector('[data-edit-form="2"] [role="alert"]');
  assert.ok(alert);
  assert.equal(alert.dataset.problems, '2');
  assert.deepEqual([...alert.querySelectorAll('li')].map((l) => l.textContent), ['Q2: the answer is empty.', 'Q2: a number answer needs a number.']);
  assert.match(alert.textContent, /can't be saved yet/);
});

test('open and closed quizzes are read only with the server words; live and ended cards say See questions', async () => {
  for (const [state, name] of [['open', 'live'], ['closed', 'ended']]) {
    fakeServer({ state });
    const v = await card(name, { draft: 'qd_fq7k2m' });
    assert.ok(!byText(v.host, 'Edit questions'), `${name}: no Edit`);
    await click(byText(v.host, 'See questions'));
    const p = panel(v);
    assert.equal(p.dataset.readOnly, 'true');
    assert.equal(p.querySelectorAll('[data-edit]').length, 0, 'no pencils');
    assert.ok(!byText(p, 'Add question to round 1'));
    assert.ok(p.querySelector('[data-read-only]'));
    v.done();
    document.body.innerHTML = '';
  }
  // A ready card whose quiz opened meanwhile: the server's 409 words, and the editor turns read only.
  const s = fakeServer();
  const v = await openEditor();
  await click(row(1).querySelector('[data-edit="1"]'));
  await type(document.querySelector('[data-edit-form="1"] textarea'), 'x');
  s.next = () => json({ error: 'quiz_open', message: 'The quiz is open now, so its questions are locked.' }, 409);
  await click(byText(document.querySelector('[data-edit-form="1"]'), 'Save'));
  assert.equal(document.querySelector('[data-edit-form="1"]'), null);
  assert.match(panel(v).querySelector('[data-read-only]').textContent, /The quiz is open now, so its questions are locked/);
  assert.equal(panel(v).querySelectorAll('[data-edit]').length, 0);
  // still_writing keeps its own words too.
  s.next = () => json({ error: 'still_writing', message: 'Quizmaster is still writing round 1.' }, 409);
});

test('writing keeps "Edit opens when round 1 is written"; no draft id says ask in chat', async () => {
  const w = await card('writing', { draft: 'qd_fq7k2m' });
  assert.equal(byText(w.host, 'Edit questions').getAttribute('title'), m.LATER.editRound1);
  w.done();
  const r = await card('ready');
  assert.equal(byText(r.host, 'Edit questions').getAttribute('title'), m.LATER.noDraft);
  assert.equal(m.LATER.edit, undefined, 'the old "comes with question editing" reason is gone');
});

test('switch off: the server says pocket_apps_off, the Edit button goes and its words show', async () => {
  g.fetch = async () => json({ error: 'pocket_apps_off', message: 'Pocket apps are off. Turn them on in Settings first.' }, 404);
  const v = await openEditor();
  assert.equal(panel(v), null);
  assert.ok(!byText(v.host, 'Edit questions'), 'no Edit button');
  assert.match(v.host.querySelector('[data-edit-off]').textContent, /Pocket apps are off/);
});

test('phone: a full-screen editor with 44 px fields and the Yours explanation; Back returns to the list', async () => {
  phone = true;
  fakeServer();
  const v = await openEditor();
  const p = document.querySelector('[data-pocket-edit="phone"]');
  assert.ok(p, 'phone layer');
  assert.equal(p.parentElement, document.body, 'portaled over the stream');
  assert.equal(p.getAttribute('role'), 'dialog');
  assert.ok([...p.querySelectorAll('[role="tab"]')].map((t) => t.textContent).join() === 'R1,R2,R3');
  await click(p.querySelector('[data-q="1"] button'));
  const form = p.querySelector('[data-edit-form="1"]');
  assert.match(p.querySelector('h2').textContent, /Q1 · Round 1/);
  for (const f of form.querySelectorAll('textarea, input:not([type="radio"])')) {
    const box = f.closest('[data-tags]') ?? f;
    assert.match(box.className, /min-h-\[44px\]/, 'a 44 px field');
  }
  assert.equal(p.querySelector('[data-yours-note]'), null, 'a checked question has no yours note');
  await click(p.querySelector('[aria-label="Back to the questions"]'));
  await click(byText(p, 'Add question to round 1'));
  assert.match(p.querySelector('[data-yours-note]').textContent, /Your own questions aren't source-checked/);
  v.done();
});

test('withCard: open → live, ready keeps a passed dry run, dry_run_stopped says what happened', () => {
  const b = m.validPocket({ ...raw('ready'), draft: 'qd_fq7k2m' });
  const d = m.quizDraftFixture({ yours: [1] });
  const c = m.draftCardFixture(d);
  const r = m.withCard(b, c);
  assert.equal(r.state, 'ready');
  assert.deepEqual(r.checks, { matched: 7, total: 7, swapped: [] });
  assert.equal(r.yours, 1);
  assert.equal(m.earnedChecked(r), false);
  assert.equal(m.withCard(b, { ...c, yours: 0 }).yours, 0);
  assert.equal(m.earnedChecked(m.withCard(b, { ...c, yours: 0 })), true);
  const stopped = m.withCard(b, { ...c, state: 'dry_run_stopped', problems: ['Q3 has no answer.'], dry_run: { ok: false, summary: 'Stopped at Q3.' } });
  assert.equal(stopped.dry_run.passed, false);
  assert.equal(stopped.dry_run.what_happened, 'Q3 has no answer.');
  assert.equal(m.withCard(b, { ...c, state: 'open' }).state, 'live');
  assert.equal(m.withCard(b, { ...c, state: 'closed' }).state, 'ended');
});

// ---------------------------------------------------------------------------------------------------
// Inbox › Done

const pocketItem = (over = {}) => ({
  kind: 'pocket',
  id: 'pocket:fq7k2m',
  kwant_id: 'k-quizmaster',
  kwant_name: 'Quizmaster',
  title: 'Friday Pub Quiz',
  summary: 'The Quizzly Bears win',
  status: 'completed',
  group: 'finished',
  sort_key: [3, 0],
  timestamp: '2026-11-21T22:41:00Z',
  unread: true,
  meta: { players: 24, ended_at: '2026-11-21T22:41:00Z' },
  winner: { name: 'The Quizzly Bears', points: 38 },
  link: { thread_id: null, channel_id: 'ch-pub', direct_kwant_id: null },
  source: { kind: 'pocket', code: 'fq7k2m', kwant_id: 'k-quizmaster', message_id: 'msg-results', channel_id: 'ch-pub', thread_id: null },
  result_message_id: 'msg-results',
  result_link: { thread_id: null, channel_id: 'ch-pub', direct_kwant_id: null },
  ...over,
});
async function mountInbox(items, onOpen = () => {}) {
  const data = structuredClone(m.fx.AWAY_INBOX);
  data.items = [...data.items, ...items];
  sessionStorage.clear();
  m.store.inboxStore.reset();
  m.store.inboxSource.get = async () => data;
  m.store.inboxStore.patch({ data });
  return mount(h(m.InboxNew, { onOpen, noLoad: true, wide: true, now: Date.parse(data.now), placeName: m.fx.awayPlaceName }));
}

test('Done row: AnyRow anatomy (kicker + ended time on top), the server title in full, "N players", Winners inline only with a winner, opens at the results', async () => {
  const opened = [];
  const v = await mountInbox(
    [
      pocketItem({ title: 'Friday Pub Quiz: Quizzly Bears win' }),
      pocketItem({ id: 'pocket:nowin', title: 'Office Quiz: Nobody joined.', winner: null, unread: false, meta: { players: 1, ended_at: '2026-11-21T20:05:00Z' } }),
    ],
    (i) => opened.push(i),
  );
  const rows = [...v.host.querySelectorAll('[data-pocket-row]')];
  assert.equal(rows.length, 2);
  const [win, none] = rows;
  const opt = win.querySelector('[role="option"]');
  assert.match(opt.className, /\bitems-start\b/, 'top-aligned like AnyRow');
  assert.equal(win.querySelector('[data-kicker]').textContent.trim(), 'Pocket app ·');
  assert.match(win.querySelector('[data-kicker]').parentElement.textContent, /Pocket app · Quizmaster/);
  const t = win.querySelector('time');
  assert.equal(t.getAttribute('dateTime'), '2026-11-21T22:41:00Z');
  assert.match(t.className, /tabular-nums/);
  assert.match(t.className, /whitespace-nowrap/);
  assert.ok(win.querySelector('[data-unread]').nextElementSibling === t, 'unread dot sits before the time');
  const title = win.querySelector('[data-pocket-title]');
  assert.equal(title.textContent.trim(), 'Friday Pub Quiz: Quizzly Bears win');
  assert.doesNotMatch(title.className, /\btruncate\b/, 'the winner name is never cut to one line');
  assert.match(title.className, /line-clamp-2/);
  assert.doesNotMatch(title.className, /(^|\s)(block|flex|inline-block)(\s|$)/, 'a display class after line-clamp-2 would cancel the clamp (QA N7)');
  const meta = win.querySelector('[data-pocket-meta]');
  assert.equal(meta.textContent, '24 players');
  assert.match(meta.className, /whitespace-nowrap/);
  assert.equal(m.pocketMeta(pocketItem()), '24 players');
  assert.equal(m.pocketEndedAt({ meta: {}, timestamp: 'x' }), 'x');
  assert.equal(holos(win), 1, 'Winners holo');
  assert.ok(meta.parentElement.contains(win.querySelector('[aria-label="Winners: The Quizzly Bears"]')), 'Winners is inline on the meta line');
  assert.ok(win.querySelector('svg, img, [data-avatar]'), 'an avatar');
  assert.equal(holos(none), 0, 'no winner, no sticker');
  assert.equal(none.querySelector('[data-pocket-meta]').textContent, '1 player');
  assert.equal(none.querySelector('[data-unread]'), null);
  assert.match(none.querySelector('time').className, /ml-auto/);
  await click(opt);
  assert.equal(opened.length, 1);
  assert.equal(opened[0].message_id, 'msg-results', 'scrolls to the results message');
  assert.deepEqual(opened[0].link, { thread_id: null, channel_id: 'ch-pub', direct_kwant_id: null });
});

test('switch off: an off server sends no pocket items, so Done has no pocket row', async () => {
  const v = await mountInbox([]);
  assert.equal(v.host.querySelectorAll('[data-pocket-row]').length, 0);
});

// Rigel's QA notes on c9513f4f (pa4c-ui/qa.md): N1 the card keeps the edit after the chat re-renders, N2 long
// words wrap, N3 marks shift after an add, N4 a bad draft id drops only Edit.

test('N1: the panel\'s first load refreshes the card, and a re-render of the same block keeps it', async () => {
  fakeServer({ yours: [2] });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const props = () => ({ content: F.ready.reply, results: { version: 1, blocks: [{ ...raw('ready'), draft: 'qd_fq7k2m' }] }, messageId: 'm-ready' });
  root.render(h(m.MarkdownRenderer, props()));
  await settle();
  mounted.add({ done: () => (root.unmount(), host.remove()) });
  assert.equal(holos(host), 1, 'the chat block says all checked');
  await click(byText(host, 'Edit questions'));
  assert.match(host.textContent, /· 1 yours, not checked/, 'the GET card shows the draft as it is');
  assert.equal(holos(host), 0);
  root.render(h(m.MarkdownRenderer, props())); // a fresh results object with the same content
  await settle();
  assert.match(host.textContent, /· 1 yours, not checked/, 'still the edited card');
  assert.equal(holos(host), 0);
});

test('N2: long answers and tags wrap anywhere instead of pushing the phone editor sideways', async () => {
  phone = true;
  fakeServer();
  await openEditor();
  const ans = row(1).querySelector('strong').parentElement;
  assert.match(ans.className, /\[overflow-wrap:anywhere\]/);
  await click(row(1).querySelector('button'));
  const tagBox = document.querySelector('[data-tags]');
  const input = tagBox.querySelector('input');
  await type(input, 'x'.repeat(48));
  input.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await settle();
  const chip = tagBox.querySelector('[aria-label^="Remove x"]')?.parentElement;
  assert.ok(chip, 'the long tag is a chip');
  assert.match(chip.className, /\[overflow-wrap:anywhere\]/);
  assert.match(chip.className, /max-w-full/);
});

test('N3: after an add, the changed marks after it move down one', async () => {
  fakeServer();
  const v = await openEditor();
  await click([...panel(v).querySelectorAll('button')].find((b) => b.textContent.trim().startsWith('Round 2')));
  await click(row(4).querySelector('[data-edit="4"]'));
  const f = document.querySelector('[data-edit-form="4"]');
  await type([...f.querySelectorAll('input')].find((i) => i.value === '15'), '15 players');
  await click(byText(f, 'Save'));
  assert.equal(row(4).dataset.changed, 'true');
  await click([...panel(v).querySelectorAll('button')].find((b) => b.textContent.trim().startsWith('Round 1')));
  await click(byText(panel(v), 'Add question to round 1'));
  const form = document.querySelector('[data-edit-form="add-1"]');
  await type(form.querySelector('textarea'), 'Who sang "Respect"?');
  const [answer] = [...form.querySelectorAll('input:not([type="radio"])')].filter((i) => !i.placeholder);
  await type(answer, 'Aretha Franklin');
  await click(byText(form, 'Save'));
  assert.equal(row(4).dataset.changed, 'true', 'the new Q4');
  await click([...panel(v).querySelectorAll('button')].find((b) => b.textContent.trim().startsWith('Round 2')));
  assert.equal(row(5).dataset.changed, 'true', 'the edited question is Q5 now');
  assert.notEqual(row(6).dataset.changed, 'true');
});

test('N4: a bad draft id or yours count drops only that field, the card still shows', () => {
  const b = m.validPocket({ ...raw('ready'), draft: 'not a valid id!', yours: -2 });
  assert.ok(b, 'the card is kept');
  assert.equal(b.draft, null);
  assert.equal(b.yours, null);
});
