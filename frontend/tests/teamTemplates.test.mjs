// 0.7.7 Ready-made teams (T) and Suggest members from the code (D), Pixel's side: the client rules, the
// preview sheet against api.md's shapes, the Needs card and the entry points. Example data only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLSelectElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'KeyboardEvent',
  'FocusEvent',
  'CustomEvent',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
  'sessionStorage',
]) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
let phone = false;
window.matchMedia = (q) => ({ matches: q.includes('max-width: 767px') ? phone : false, media: q, addEventListener() {}, removeEventListener() {} });
g.fetch = g.window.fetch = async () => new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/teams/teamTemplates';
  export * as fx from './src/teams/suggestFixtures';
  export { TEMPLATES_FIXTURE, TEMPLATES_WIRE } from './src/teams/templateFixtures';
  export { TeamPreviewSheet } from './src/components/TeamPreviewSheet';
  export { TeamNeedsCard, TeamCreatedToast, UndoTeamConfirm } from './src/components/TeamNeedsCard';
  export { TemplateWelcome, TemplateTiles, TemplateSidebarBox } from './src/components/TeamTemplatePicks';
  export { ReadyTeamsProvider, madeFromRow } from './src/components/ReadyTeams';
  export { ReadyTeamsTiles, SuggestMembersButton, ReadyTeamsSidebarBox } from './src/components/ReadyTeamsEntry';
  export { ManageTeamModal } from './src/components/ManageTeamModal';
`);
const { React, createRoot, fx } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const typeIn = (el, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
};
const key = async (el, k) => {
  el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root, rerender: async (e) => (root.render(e), await settle()), unmount: () => (root.unmount(), host.remove()) };
};
const byText = (root, sel, re) => [...root.querySelectorAll(sel)].find((el) => re.test(el.textContent ?? ''));
const primary = (root) => byText(root, 'button', /^(Create team|Add \d|Try again|Creating)/);
const use = (opts) => {
  const calls = [];
  m.setTeamsTransport(fx.fixtureTransport(opts, calls));
  return calls;
};
const creates = (calls) => calls.filter((c) => c.url.startsWith('/api/teams/from-template'));

const sheet = (mode, extra = {}) => {
  const out = { created: null, closed: 0, coding: 0 };
  const el = h(m.TeamPreviewSheet, {
    open: true,
    space: 'spc_demo',
    mode,
    templates: m.TEMPLATES_FIXTURE,
    takenNames: [],
    takenHandles: [],
    spaceBots: [{ id: 'kw_frontend', name: 'Frontend', role: 'tech_lead' }],
    onClose: () => out.closed++,
    onCreated: (r, t, handle) => (out.created = { r, t, handle }),
    onUseCoding: () => out.coding++,
    ...extra,
  });
  return { el, out };
};

// ---------------------------------------------------------------------------------------------------
// Pure rules and wire shapes

test('wire → internal: bots carry a key (§0.1); reason_parts and string reasons both become parts; the stack prefix goes', () => {
  const t = m.normTemplate(m.TEMPLATES_WIRE[0]);
  assert.deepEqual(t.bots.map((b) => b.key), ['inboxmanager', 'drafter', 'researcher']);
  const keys = m.TEMPLATES_WIRE.flatMap((x) => x.bots.map((b) => b.key));
  assert.deepEqual(keys, ['inboxmanager', 'drafter', 'researcher', 'saleslead', 'prospector', 'outreachwriter', 'techlead', 'frontend', 'backend', 'qa']);
  // §0.2 and §0.9: skill owners and the routine's bot are keys, not names.
  assert.equal(t.skills[0].owner, 'inboxmanager');
  assert.equal(t.skills[0].owner_name, 'InboxManager');
  assert.equal(t.routine.bot, 'inboxmanager');
  assert.equal(m.botNameByKey(t, { drafter: 'ReplyWriter' }, 'drafter'), 'ReplyWriter');
  assert.equal(m.botNameByKey(t, {}, 'inboxmanager'), 'InboxManager');
  const s = fx.SHOP_SUGGESTION;
  assert.equal(s.stack_line, 'Rust (axum), React 18 + TypeScript (Vite), Postgres, Docker');
  assert.deepEqual(s.bots[0].reason[1], { path: 'server/Cargo.toml' });
  assert.deepEqual(s.also_possible[0].reason, [{ text: 'Found 38 .sql files in server/migrations/. Migrations alone are a weak sign.' }]);
  assert.equal(s.fallback, false);
  assert.equal(fx.UNKNOWN_SUGGESTION.fallback, true);
});

test('createBody sends only the documented fields, with idempotency_key in the body (§0.3)', () => {
  const k = 'k1';
  assert.deepEqual(m.createBody({ kind: 'template', space: 's', template_id: 'inbox', handle: 'inbox', names: {} }, k), { template_id: 'inbox', handle: 'inbox', names: {}, idempotency_key: k });
  assert.deepEqual(m.createBody({ kind: 'suggestion', space: 's', suggestion_id: 'sug_1', rows: { qa: { on: false } } }, k), { suggestion_id: 'sug_1', edits: { rows: { qa: { on: false } } }, idempotency_key: k });
  assert.deepEqual(m.createBody({ kind: 'suggestion', space: 's', suggestion_id: 'sug_1', handle: 'shop', rows: {} }, k), { suggestion_id: 'sug_1', edits: { rows: {}, handle: 'shop' }, idempotency_key: k });
  // A18: team_id at the top level, and never a handle with it.
  assert.deepEqual(m.createBody({ kind: 'suggestion', space: 's', suggestion_id: 'sug_1', team_id: 'team_1', handle: 'x', rows: {} }, k), { suggestion_id: 'sug_1', edits: { rows: {} }, idempotency_key: k, team_id: 'team_1' });
});

test('§0.6 existing object flattens onto the row; §0.7 partial "file" has its own copy; §0.5 only 424 is can’t read', () => {
  const fe = fx.SHOP_SUGGESTION.bots.find((b) => b.key === 'frontend');
  assert.equal(fe.existing_kwant_id, 'kw_frontend');
  assert.equal(fe.existing_name, 'Frontend');
  assert.equal(fe.existing_avatar, 'tech_lead');
  assert.equal('existing' in fe, false);
  assert.equal(fx.SHOP_SUGGESTION.bots[0].existing_kwant_id, null);
  assert.equal(m.partialHead('file'), 'One manifest was too big to read.');
  assert.match(m.partialBody('file'), /256 KB/);
  assert.equal(m.cantRead(new m.TeamsError('x', 424, 'cant_read')), true);
  assert.equal(m.cantRead(new m.TeamsError('only the person can do this', 403)), false);
  assert.equal(m.cantRead(new m.TeamsError('x', 502, 'read_failed')), false);
});

test('fieldKey reads the row out of a 409 field', () => {
  assert.equal(m.fieldKey('names.drafter'), 'drafter');
  assert.equal(m.fieldKey('edits.rows.qa.name'), 'qa');
  assert.equal(m.fieldKey('handle'), null);
  assert.equal(m.fieldKey(null), null);
});

test('names, handles and clashes (T9)', () => {
  assert.equal(m.cleanHandle(' Inbox  Team-- '), 'inbox-team');
  assert.equal(m.nextFree('Drafter', ['Drafter', 'Drafter2']), 'Drafter3');
  assert.match(m.nameError('', []), /name/i);
  assert.ok(m.nameError('Drafter', ['drafter']));
  assert.equal(m.nameError('Drafter', ['Researcher']), null);
  const c = m.resolveClashes(m.TEMPLATES_FIXTURE[0].bots, 'inbox', ['Drafter'], ['inbox']);
  assert.equal(c.names.drafter, 'Drafter2');
  assert.equal(c.handle, 'inbox2');
  assert.ok(c.notes.drafter && c.handleNote);
});

test('D rules: initial ticks, the existing bot, size limits, carrying edits after a 410', () => {
  const e = m.initialEdits(fx.SHOP_SUGGESTION, []);
  assert.equal(m.tickedCount(e), 4);
  assert.equal(e.frontend.useExisting, 'kw_frontend');
  assert.equal(e.data.ticked, false);
  assert.equal(m.newBotCount(e), 3);
  assert.equal(m.sizeNote(1), 'Pick at least 2 bots.');
  assert.match(m.sizeNote(6), /6 bots at most/);
  assert.equal(m.sizeNote(4), null);
  const edited = { ...e, qa: { ...e.qa, ticked: false }, sre: { ...e.sre, name: 'Ops', renamed: true } };
  const carried = m.carryEdits(edited, m.initialEdits(fx.PARTIAL_SUGGESTION, []), fx.SHOP_SUGGESTION, fx.PARTIAL_SUGGESTION);
  assert.equal(carried.qa.ticked, false);
  assert.equal(carried.sre.name, 'Ops');
  assert.equal(m.partialHead('paths'), 'Read the first 20,000 files.');
  assert.equal(m.partialHead('bytes'), 'Some manifests were too big to read.');
  assert.equal(m.sourceName('git.example.com/acme/shop.git'), 'shop');
  assert.equal(m.sourceName('~/projects/shop/'), 'shop');
});

test('needs: each opener reads its own state; open count adds "Add an account" when nothing can run', () => {
  const inbox = m.TEMPLATES_FIXTURE.find((t) => t.id === 'inbox');
  const ctx = { canRun: true, imap: { state: 'none' }, apps: [], repo: { state: 'none' }, personPosted: false };
  const email = inbox.needs.find((n) => n.id === 'email');
  assert.equal(m.needView(email, ctx).state, 'req');
  assert.equal(m.needView(email, { ...ctx, imap: { state: 'checking', detail: 'mail.example.com' } }).text, 'Checking mail.example.com…');
  assert.equal(m.needView(email, { ...ctx, imap: { state: 'ok', detail: 'ops@mail.example.com' } }).state, 'done');
  assert.equal(m.needView(email, { ...ctx, imap: { state: 'failed' } }).state, 'bad');
  assert.equal(m.needView(inbox.needs.find((n) => n.id === 'mail_app'), { ...ctx, apps: ['gmail'] }).state, 'done');
  assert.equal(m.openCount(inbox, ctx), 1);
  assert.equal(m.openCount(inbox, { ...ctx, canRun: false }), 2);
  assert.match(m.needsSub(inbox, { ...ctx, imap: { state: 'ok' } }), /It has what it needs\.$/);
});

// ---------------------------------------------------------------------------------------------------
// T: the template preview

test('T: Create sends template_id, handle and only changed names by key, with idempotency_key in the body', async () => {
  const calls = use({ create: 'ok' });
  const { el, out } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  assert.ok(root, 'sheet renders');
  assert.match(primary(root).textContent, /Create team · 3 bots/);
  await click(root.querySelector('[aria-label="Rename Drafter"]'));
  const input = root.querySelector('[aria-label^="Name for"]');
  typeIn(input, 'ReplyWriter');
  await settle();
  await key(input, 'Enter');
  await click(primary(root));
  const [c] = creates(calls);
  assert.equal(c.url, '/api/teams/from-template?space=spc_demo');
  const { idempotency_key, ...rest } = c.body;
  assert.deepEqual(rest, { template_id: 'inbox', handle: 'inbox', names: { drafter: 'ReplyWriter' } });
  assert.match(idempotency_key, /^[0-9a-f-]{36}$|^k/);
  assert.equal(c.headers, undefined);
  assert.equal(out.created.handle, 'inbox');
  assert.equal(out.created.r.channel_id, 'ch_team_new');
  v.unmount();
});

test('T: a name already in the space is renamed before Create and says so', async () => {
  use({ create: 'ok' });
  const { el } = sheet({ kind: 'template', templateId: 'inbox' }, { takenNames: ['Drafter'], takenHandles: ['inbox'] });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  assert.match(root.textContent, /Drafter2/);
  assert.match(root.textContent, /inbox2/);
  v.unmount();
});

test('T: a 409 clash takes the server suggestion, names the row, and the retry keeps the sheet’s key', async () => {
  const calls = use({ create: 'clash' });
  const { el, out } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  await click(primary(root));
  assert.equal(out.created, null);
  assert.match(root.textContent, /Drafter is taken in this space, so this one is Drafter3\./);
  m.setTeamsTransport(fx.fixtureTransport({ create: 'ok' }, calls));
  await click(primary(root));
  const [a, b] = creates(calls);
  assert.equal(b.body.names.drafter, 'Drafter3');
  assert.equal(a.body.idempotency_key, b.body.idempotency_key);
  assert.ok(out.created);
  v.unmount();
});

test('T: a 409 clash on the handle moves the handle and keeps the sheet', async () => {
  use({ create: 'clash-handle' });
  const { el } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  await click(primary(root));
  assert.equal(root.querySelector('[aria-label="Team handle"]').value, 'inbox3');
  assert.match(root.textContent, /@inbox is taken here, so this team is @inbox3\./);
  v.unmount();
});

test('T: a failed create says nothing was created; Try again reuses the same idempotency_key', async () => {
  const calls = use({ create: 'fail' });
  const { el, out } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  await click(primary(root));
  assert.match(primary(root).textContent, /Try again/);
  m.setTeamsTransport(fx.fixtureTransport({ create: 'ok' }, calls));
  await click(primary(root));
  const [a, b] = creates(calls);
  assert.equal(a.body.idempotency_key, b.body.idempotency_key);
  assert.ok(out.created);
  v.unmount();
});

test('T: a 409 with several clashes (§0.4) applies every one, not just the first', async () => {
  const calls = use({ create: 'clash-many' });
  const { el, out } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  await click(primary(root));
  assert.equal(out.created, null);
  assert.equal(root.querySelector('[aria-label="Team handle"]').value, 'inbox3');
  assert.match(root.textContent, /Drafter is taken in this space, so this one is Drafter3\./);
  assert.match(root.textContent, /Researcher is taken in this space, so this one is Researcher2\./);
  assert.match(root.textContent, /3 names were taken/);
  m.setTeamsTransport(fx.fixtureTransport({ create: 'ok' }, calls));
  await click(primary(root));
  const b = creates(calls)[1].body;
  assert.equal(b.handle, 'inbox3');
  assert.deepEqual(b.names, { drafter: 'Drafter3', researcher: 'Researcher2' });
  v.unmount();
});

test('T: the server preview (§0.10) sets the free names before Create; a 404 keeps the client’s guess', async () => {
  const preview = { template_id: 'inbox', handle: 'inbox4', handle_wanted: 'inbox', names: { inboxmanager: 'InboxManager', drafter: 'Drafter5', researcher: 'Researcher' }, clashes: [] };
  const calls = use({ create: 'ok', preview });
  const { el } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  assert.ok(calls.some((c) => c.url === '/api/team-templates/inbox/preview?space=spc_demo'));
  assert.equal(root.querySelector('[aria-label="Team handle"]').value, 'inbox4');
  assert.match(root.textContent, /Drafter is taken in this space, so this one is Drafter5\./);
  await click(primary(root));
  assert.deepEqual(creates(calls)[0].body.names, { drafter: 'Drafter5' });
  v.unmount();

  use({ create: 'ok' });
  const w = await mount(sheet({ kind: 'template', templateId: 'inbox' }, { takenNames: ['Drafter'] }).el);
  assert.match(document.querySelector('[data-team-sheet="inbox"]').textContent, /Drafter2/);
  w.unmount();
});

test('T: skills and the routine show the owner by key, after a rename', async () => {
  use({ create: 'ok' });
  const v = await mount(sheet({ kind: 'template', templateId: 'inbox' }).el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  assert.match(root.textContent, /InboxManager · on · Turns new mail/);
  assert.match(root.textContent, /Weekdays at 8:00 · InboxManager\./);
  await click(root.querySelector('[aria-label="Rename Drafter"]'));
  const input = root.querySelector('[aria-label^="Name for"]');
  typeIn(input, 'ReplyWriter');
  await settle();
  await key(input, 'Enter');
  assert.match(root.textContent, /ReplyWriter · on · Writes a reply/);
  v.unmount();
});

// ---------------------------------------------------------------------------------------------------
// D: suggest members from the code

test('D: reads one source, shows the stack, the reasons with paths, the existing bot and also possible', async () => {
  const calls = use({ suggest: 'shop' });
  const { el } = sheet({ kind: 'suggest', workspace_path: '~/projects/shop' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="suggest"]');
  const s = calls.find((c) => c.url.startsWith('/api/teams/suggest'));
  assert.equal(s.url, '/api/teams/suggest?space=spc_demo');
  assert.deepEqual(s.body, { workspace_path: '~/projects/shop' });
  assert.match(root.textContent, /Suggested team for shop/);
  assert.match(root.textContent, /Rust \(axum\)/);
  assert.match(root.textContent, /server\/Cargo\.toml/);
  assert.ok(root.querySelector('[data-row="frontend"]'));
  assert.ok(root.querySelector('[data-row="security"]'));
  assert.match(primary(root).textContent, /Create team · 4 bots/);
  v.unmount();
});

test('D: unticking and the existing bot go out as on / use_existing rows; the handle seeds from the folder', async () => {
  const calls = use({ suggest: 'shop', create: 'ok' });
  const { el, out } = sheet({ kind: 'suggest', workspace_path: '~/projects/shop' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="suggest"]');
  await click(root.querySelector('[data-row="sre"] [role="checkbox"]:not([aria-disabled])'));
  assert.match(primary(root).textContent, /Create team · 3 bots/);
  await click(primary(root));
  const [c] = creates(calls);
  assert.equal(c.body.suggestion_id, 'sug_demo_shop');
  assert.equal(c.body.edits.handle, 'shop');
  assert.deepEqual(c.body.edits.rows.sre, { on: false });
  assert.deepEqual(c.body.edits.rows.frontend, { on: true, use_existing: 'kw_frontend' });
  assert.deepEqual(c.body.edits.rows.qa, { on: true, name: 'QA' });
  assert.deepEqual(c.body.edits.rows.data, { on: false });
  for (const row of Object.values(c.body.edits.rows)) for (const k of Object.keys(row)) assert.ok(['on', 'name', 'preset', 'use_existing'].includes(k), k);
  assert.ok(out.created);
  v.unmount();
});

test('D: a 410 reads the project again and keeps the edits on rows that come back', async () => {
  const calls = use({ suggest: 'shop', create: 'expired-once' });
  const { el, out } = sheet({ kind: 'suggest', workspace_path: '~/projects/shop' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="suggest"]');
  await click(root.querySelector('[data-row="sre"] [role="checkbox"]:not([aria-disabled])'));
  await click(primary(root));
  assert.equal(out.created, null);
  assert.match(root.textContent, /Read the project again, it had been an hour\./);
  assert.equal(calls.filter((c) => c.url.startsWith('/api/teams/suggest')).length, 2);
  assert.match(primary(root).textContent, /Create team · 3 bots/);
  await click(primary(root));
  assert.deepEqual(creates(calls)[1].body.edits.rows.sre, { on: false });
  v.unmount();
});

test('D: Manage team sends team_id, no handle, and labels the button by new bots', async () => {
  const calls = use({ suggest: 'manage', create: 'ok' });
  const { el } = sheet({ kind: 'suggest', team: { id: 'team_shop', name: 'shop' }, workspace_path: '~/projects/shop' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="suggest"]');
  assert.deepEqual(calls[0].body, { team_id: 'team_shop' });
  assert.match(root.textContent, /Suggested for @shop/);
  assert.match(primary(root).textContent, /Add 2 bots/);
  await click(primary(root));
  const body = creates(calls)[0].body;
  assert.equal('handle' in body.edits, false);
  assert.equal(body.team_id, 'team_shop');
  v.unmount();
});

test('D7: can’t read, read failed and couldn’t tell each offer the Coding roster; nothing created', async () => {
  for (const [sc, re] of [
    ['cantread', /Can’t read this repo yet/],
    ['error', /Couldn’t read the project\./],
  ]) {
    use({ suggest: sc });
    const { el, out } = sheet({ kind: 'suggest', repo_url: 'git.example.com/acme/shop' });
    const v = await mount(el);
    const root = document.querySelector('[data-team-sheet="suggest"]');
    assert.match(root.textContent, re);
    // Mira #396 must-fix 5: nothing will be created, so the footer's left side is empty (no Undo icon).
    const note = root.querySelector('footer p');
    assert.equal(note.textContent, '');
    assert.equal(note.querySelector('svg'), null);
    await click(byText(root, 'button', /^Use the Coding roster$/));
    assert.equal(out.coding, 1);
    v.unmount();
  }
  use({ suggest: 'unknown' });
  const { el } = sheet({ kind: 'suggest', workspace_path: '~/projects/notes' });
  const v = await mount(el);
  assert.match(document.querySelector('[data-team-sheet="suggest"]').textContent, /Couldn’t tell what this project uses\./);
  v.unmount();
});

test('D7 can’t read: "Pick a local folder instead" shows when the picker is passed; partial uses CircleAlert, not WifiOff', async () => {
  use({ suggest: 'cantread' });
  let picked = 0;
  const { el } = sheet({ kind: 'suggest', repo_url: 'git.example.com/acme/shop' }, { onPickFolder: () => picked++ });
  let v = await mount(el);
  let root = document.querySelector('[data-team-sheet="suggest"]');
  assert.match(root.textContent, /so kwant can read file names and manifests/);
  await click(byText(root, 'button', /^Pick a local folder instead$/));
  assert.equal(picked, 1);
  v.unmount();
  use({ suggest: 'partial-file' });
  v = await mount(sheet({ kind: 'suggest', workspace_path: '~/projects/shop' }).el);
  root = document.querySelector('[data-team-sheet="suggest"]');
  const banner = byText(root, '[role="status"]', /One manifest was too big to read/);
  assert.ok(banner);
  assert.match(banner.querySelector('svg').getAttribute('class'), /lucide-circle-alert|lucide-alert-circle/);
  v.unmount();
});

test('A8 "Pick a repo": Manage team opens on Team Workspace; Git Repo + repo URL focused with no workspace, its own tab otherwise', async () => {
  const scrolled = [];
  const orig = window.HTMLElement.prototype.scrollIntoView;
  window.HTMLElement.prototype.scrollIntoView = function (o) {
    scrolled.push([this.getAttribute('data-team-workspace') !== null, o?.block]);
  };
  const kwants = [{ id: 'kw_a', name: 'Ada', avatar_sprite: 'architect' }];
  const base = { isOpen: true, kwants, onClose() {}, onUpdateTeam: async () => {}, onDeleteTeam: async () => {}, focusWorkspace: true };
  let v = await mount(h(m.ManageTeamModal, { ...base, team: { id: 'team_x', name: 'x', lead_kwant_id: 'kw_a', member_kwants: ['kw_a'] } }));
  await settle(30);
  assert.deepEqual(scrolled.at(-1), [true, 'center']);
  assert.equal(document.activeElement?.getAttribute('aria-label'), 'Repo URL');
  v.unmount();
  v = await mount(h(m.ManageTeamModal, { ...base, team: { id: 'team_y', name: 'y', workspace_path: '~/projects/y', lead_kwant_id: 'kw_a', member_kwants: ['kw_a'] } }));
  await settle(30);
  assert.equal(document.activeElement?.value, '~/projects/y');
  v.unmount();
  window.HTMLElement.prototype.scrollIntoView = orig;
});

test('the sheet closes on Esc and from Close, and not while creating', async () => {
  use({});
  const { el, out } = sheet({ kind: 'template', templateId: 'inbox' });
  const v = await mount(el);
  const root = document.querySelector('[data-team-sheet="inbox"]');
  await key(root, 'Escape');
  assert.equal(out.closed, 1);
  await click(root.querySelector('[aria-label="Close"]'));
  assert.equal(out.closed, 2);
  v.unmount();
});

// ---------------------------------------------------------------------------------------------------
// T5: the Needs card, the toast and Undo

test('§0.8 team rows say which template made them; Undo info comes from template-undo (§0.10)', async () => {
  const base = { id: 'team_1', name: 'inbox', created_at: '2026-11-07T10:00:00Z' };
  assert.equal(m.madeFromRow(base, null), undefined, 'an older server: no fields, the local record decides');
  assert.equal(m.madeFromRow({ ...base, created_from_template: null, template_action_id: null }, null), null);
  const made = m.madeFromRow({ ...base, created_from_template: 'inbox', template_action_id: 'act_1' }, null);
  assert.deepEqual(made, { templateId: 'inbox', handle: 'inbox', createdAt: Date.parse('2026-11-07T10:00:00Z'), actionId: 'act_1', undoUntil: Date.parse('2026-11-08T10:00:00Z') });
  assert.equal(m.madeFromRow({ ...base, created_from_template: 'suggested', template_action_id: 'act_2' }, null).templateId, null);

  const calls = [];
  m.setTeamsTransport(async (url, method) => {
    calls.push({ url, method });
    return { status: 200, body: { template_id: 'inbox', action_id: 'act_1', created_at: '', undo_until: '', can_undo: true, blocked: [], removes: { bots: ['Drafter'], skills: 1, routines: 0, handle: 'inbox' }, chat_messages: 0, also_in: [{ bot: 'Drafter', team: 'support' }], edited_skills: ['Draft a reply'] } };
  });
  const info = await m.teamsApi.templateInfo('team_1', 'spc_demo');
  assert.equal(calls[0].url, '/api/teams/team_1/template-undo?space=spc_demo');
  assert.deepEqual(info.other_teams, [{ bot: 'Drafter', team: 'support' }]);
  assert.equal(m.undoWindowOver(new m.TeamsError('x', 409, 'undo_window_over')), true);
  assert.equal(m.undoWindowOver({ code: 'undo_window_over', status: 409 }), true);
  assert.equal(m.undoWindowOver(new m.TeamsError('x', 409, 'bot_leads_elsewhere')), false);
});

test('the Undo confirm names edited skills and other teams, and skips the archive line for an empty chat', async () => {
  const v = await mount(h(m.UndoTeamConfirm, { label: 'Inbox', handle: 'inbox', botNames: ['InboxManager', 'Drafter'], skills: 2, routine: true, messages: 0, alsoIn: [{ bot: 'Drafter', team: 'support' }], editedSkills: ['Draft a reply'], onKeep() {}, onUndo() {} }));
  const d = document.querySelector('[data-undo-confirm]');
  assert.match(d.textContent, /Drafter is also in @support; it will leave that team\./);
  assert.match(d.textContent, /You edited Draft a reply\. It’s removed too\./);
  assert.doesNotMatch(d.textContent, /archived/);
  v.unmount();
});

test('T5: the Needs card lists what is open, opens each, and offers Undo for a day', async () => {
  const inbox = m.TEMPLATES_FIXTURE.find((t) => t.id === 'inbox');
  const opened = [];
  let undone = 0;
  const now = Date.parse('2026-11-07T10:00:00Z');
  const ctx = { canRun: true, imap: { state: 'none' }, apps: [], repo: { state: 'none' }, personPosted: false };
  const props = { template: inbox, handle: 'inbox', teamId: 'team_new', createdAt: now - 60_000, ctx, now, onOpen: (n) => opened.push(n.id), onAddAccount() {}, onFill() {}, onUndo: () => undone++ };
  const v = await mount(h(m.TeamNeedsCard, props));
  const card = document.querySelector('[data-needs-card="team_new"]');
  assert.ok(card);
  assert.equal(card.querySelector('[data-need="email"]').dataset.s, 'req');
  assert.equal(card.querySelector('[data-need="mail_app"]').dataset.s, 'opt');
  await click(card.querySelector('[data-need="email"] button'));
  assert.deepEqual(opened, ['email']);
  await click(card.querySelector('[aria-label="Undo team @inbox"]'));
  assert.equal(undone, 1);
  await v.rerender(h(m.TeamNeedsCard, { ...props, ctx: { ...ctx, imap: { state: 'checking', detail: 'mail.example.com' } } }));
  assert.equal(document.querySelector('[data-need="email"]').dataset.s, 'checking');
  await v.rerender(h(m.TeamNeedsCard, { ...props, createdAt: now - 2 * 86_400_000 }));
  assert.equal(document.querySelector('[aria-label="Undo team @inbox"]'), null);
  v.unmount();
});

test('the created toast shows counts and Undo', async () => {
  let undo = 0;
  const v = await mount(h(m.TeamCreatedToast, { text: 'Created @inbox: 3 bots, 2 skills, 1 routine (off).', onUndo: () => undo++, onDismiss() {} }));
  const t = document.querySelector('[data-team-toast]');
  assert.match(t.textContent, /Created @inbox: 3 bots, 2 skills, 1 routine \(off\)\./);
  await click(byText(t, 'button', /Undo/));
  assert.equal(undo, 1);
  v.unmount();
});

// ---------------------------------------------------------------------------------------------------
// T7 and D1: the entry points through the provider

test('T7/D1 entry points: tiles, sidebar box (hide sticks per space), Suggest only with a folder or repo', async () => {
  const calls = use({ suggest: 'shop' });
  localStorage.clear();
  let hidden = 0;
  let shown = 0;
  const tree = (path) =>
    h(
      m.ReadyTeamsProvider,
      { space: 'spc_demo', bots: [], teams: [], onChanged() {}, onOpenTeam() {} },
      h('div', { id: 'tiles' }, h(m.ReadyTeamsTiles, { source: { workspace_path: path }, onHide: () => hidden++, onShow: () => shown++, onDone() {} })),
      h('div', { id: 'suggest' }, h(m.SuggestMembersButton, { workspace_path: path, onHide: () => hidden++, onShow: () => shown++ })),
      h('div', { id: 'box' }, h(m.ReadyTeamsSidebarBox, { teamCount: 0 })),
    );
  const v = await mount(tree(''));
  assert.ok(calls.some((c) => c.url === '/api/team-templates?space=spc_demo'));
  assert.equal(document.querySelectorAll('#tiles [data-template]').length, 3);
  assert.equal(document.querySelector('#suggest [data-suggest-members]'), null);
  assert.ok(document.querySelector('#box [data-template-sidebar]'));
  await click(document.querySelector('[aria-label="Hide ready-made teams"]'));
  assert.equal(document.querySelector('#box [data-template-sidebar]'), null);
  assert.equal(m.readyTeamsHidden('spc_demo'), true);

  await v.rerender(tree('~/projects/shop'));
  const btn = byText(document.querySelector('#suggest'), 'button', /Suggest members from the code/);
  await click(btn);
  assert.equal(hidden, 1);
  const s = document.querySelector('[data-team-sheet="suggest"]');
  assert.ok(s, 'the sheet opens from Suggest');
  await key(s, 'Escape');
  assert.equal(shown, 1);
  assert.equal(document.querySelector('[data-team-sheet]'), null);

  await click(document.querySelector('#tiles [data-template="coding"]'));
  assert.equal(document.querySelector('[data-team-sheet="suggest"]') !== null, true, 'Coding with a folder reads the code first');
  v.unmount();
});

test('no templates route (404): every entry point renders nothing', async () => {
  m.setTeamsTransport(async () => ({ status: 404, body: { error: 'not found' } }));
  const v = await mount(
    h(
      m.ReadyTeamsProvider,
      { space: 'spc_old', bots: [], teams: [], onChanged() {}, onOpenTeam() {} },
      h('div', { id: 'all' }, h(m.ReadyTeamsTiles, { source: {}, onHide() {}, onShow() {}, onDone() {} }), h(m.SuggestMembersButton, { workspace_path: '~/projects/shop' }), h(m.ReadyTeamsSidebarBox, { teamCount: 0 })),
    ),
  );
  assert.equal(document.querySelector('#all').children.length, 0);
  v.unmount();
});
