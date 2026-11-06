// 0.7.2 team repos (plan: dev-mode/plan/team-repos.md, Frontend; T8): Dev mode watches every
// team's repo. The status line becomes a repo list, cards get a repo chip, and both lists filter
// by repo through `?repo=`. Demo data only: demo-org/app and demo-org/site, no real repos or tokens.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const APP = { key: 'github.com/demo-org/app', repo: 'demo-org/app', host: 'github.com', team: 'App', team_id: 'tm_app', from: 'folder', branch: 'main', status: 'watching', poll_error: null, polled_at: '2026-11-06T09:00:00Z', last_tag: 'v1.2.0', added_by: 'team' };
const SITE = { key: 'github.com/demo-org/site', repo: 'demo-org/site', host: 'github.com', team: 'Site', team_id: 'tm_site', from: 'repo', branch: 'trunk', status: 'watching', poll_error: null, polled_at: '2026-11-06T09:00:00Z', last_tag: null, added_by: 'team' };
const DOCS = { key: 'github.com/demo-org/docs', repo: 'demo-org/docs', host: 'github.com', team: null, team_id: null, from: 'hand', branch: 'main', status: 'watching', poll_error: null, polled_at: null, last_tag: null, added_by: 'hand' };
const LAB = { key: 'gitlab.example.test/demo/lab', repo: 'https://gitlab.example.test/demo/lab', host: 'gitlab.example.test', team: 'Lab', team_id: 'tm_lab', from: 'folder', branch: null, status: 'other_host', poll_error: null, polled_at: null, last_tag: null, added_by: null };
const base = { on: true, repo: null, branch: null, deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null, sources: [], status: 'watching' };
const TEAMS = [
  { id: 'tm_app', name: 'App' },
  { id: 'tm_site', name: 'Site' },
];

let server;
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  server.calls.push([method, u, body]);
  const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
  const path = u.split('?')[0];
  const repoQ = new URL(u, 'http://localhost').searchParams.get('repo');
  if (method === 'DELETE' && /\/dev_mode\/repos\//.test(path)) {
    const key = decodeURIComponent(path.split('/dev_mode/repos/')[1]);
    const row = server.state.repos.find((r) => r.key === key);
    if (!row || row.added_by !== 'hand') return json({ error: 'That repo comes from a team. Change the team’s repo instead.' }, 409);
    server.state = { ...server.state, repos: server.state.repos.filter((r) => r.key !== key) };
    return json(server.state);
  }
  if (path.endsWith('/dev_mode') && method === 'PUT') {
    if (server.fail) return json({ error: server.fail }, 422);
    const repo = body.repo;
    const team = TEAMS.find((t) => t.id === body.team);
    const row = { ...APP, key: `github.com/${repo}`.toLowerCase(), repo, team: team?.name ?? null, team_id: team?.id ?? null, from: team ? 'repo' : 'hand', added_by: team ? 'team' : 'hand', polled_at: null };
    server.state = { ...server.state, repos: [...server.state.repos, row] };
    return json(server.state);
  }
  if (path.endsWith('/dev_mode')) return json(server.state);
  const of = (cards) => (repoQ ? cards.filter((c) => c.repo === repoQ) : cards);
  if (path.endsWith('/dev/landed')) return json({ cards: of(server.landed), rollup: { count: 0, areas: [] }, last_tag: null, last_merge: null });
  if (path.endsWith('/dev/reviews')) return json({ cards: of(server.reviews) });
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/devMode';
  export { DevDeskView, deskStore } from './src/components/DevDesk';
  export { DevStatusLine } from './src/components/DevStatusLine';
  export { useDevMode } from './src/components/DevModeSwitch';
  export * as fx from './src/dev/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root };
};
const state = (over) => m.parseDevMode({ ...base, ...over });
const text = (l) => l.parts.map((p) => p.text).join('');
const reset = (over = {}) => {
  server = { state: { ...base, ...over }, calls: [], landed: [], reviews: [] };
  m.devStore._reset();
  m.deskStore._reset();
  localStorage.clear();
  localStorage.setItem('kwnt.dev.intro.spc_demo', '1');
  document.body.innerHTML = '';
};
const line = async (over, teams = []) => {
  reset(over);
  const st = m.parseDevMode(server.state);
  m.devStore.set('spc_demo', st);
  // Live, as in the desk: the line follows the store after Link and Remove.
  const Live = () => h(m.DevStatusLine, { spaceId: 'spc_demo', state: m.useDevMode('spc_demo') ?? st, teams, onConnect() {} });
  return mount(h(Live));
};
const frame = () => new Promise((r) => setTimeout(r, 50));
const typeInto = async (input, v) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, v);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
// An assertion on a DOM node prints the whole tree when it fails: compare booleans and strings.
const has = (sel) => document.querySelector(sel) !== null;

// ---------------------------------------------------------------------------
// Parsing

test('parseDevMode: repos[] keep key, team, branch and status; bad rows, unknown statuses and duplicate keys drop', () => {
  const s = state({ repos: [APP, SITE, { ...APP }, { repo: 'demo-org/x', status: 'nope' }, { status: 'watching' }, 'junk', null] });
  assert.deepEqual(s.repos.map((r) => [r.key, r.team, r.branch, r.status, r.added_by]), [
    ['github.com/demo-org/app', 'App', 'main', 'watching', 'team'],
    ['github.com/demo-org/site', 'Site', 'trunk', 'watching', 'team'],
  ]);
});

test('parseDevMode: no URL token reaches the screen from repos[]; only hand (or migrated) rows read as hand', () => {
  const s = state({ repos: [{ repo: 'https://demo:not-a-real-token@gitea.example.test/demo/app.git', host: 'gitea.example.test', status: 'other_host', added_by: null }] });
  assert.equal(s.repos[0].repo, 'https://gitea.example.test/demo/app.git');
  assert.equal(s.repos[0].key, 'gitea.example.test/demo/app');
  assert.equal(s.repos[0].added_by, null, 'no row: nothing to remove');
  assert.equal(state({ repos: [{ ...DOCS, added_by: 'migrated' }] }).repos[0].added_by, 'hand');
  assert.equal(state({ repos: [{ ...DOCS, from: 'hand' }] }).repos[0].from, 'hand');
  assert.ok(!JSON.stringify(s).includes('not-a-real-token'));
});

test('parseDevMode: an older server (no repos[]) falls back to one row from the deprecated repo and branch', () => {
  const s = state({ repo: 'demo-org/app', branch: 'main', sources: [{ team: 'App', repo: 'demo-org/app', host: 'github.com', from: 'folder' }] });
  assert.equal(s.repos.length, 1);
  assert.deepEqual([s.repos[0].key, s.repos[0].team, s.repos[0].branch, s.repos[0].status], ['github.com/demo-org/app', 'App', 'main', 'watching']);
  assert.equal(state({ repo: 'demo-org/typed', sources: [] }).repos[0].added_by, null, 'an older server has no DELETE: no Remove');
  assert.deepEqual(state({ status: 'no_repo' }).repos, []);
  assert.equal(state({ repo: 'demo-org/app', repos: [] }).repos.length, 0, 'an empty repos[] wins over the old field');
});

// ---------------------------------------------------------------------------
// The list states

test('one repo: today’s single line, no list', () => {
  const l = m.statusLine(state({ repos: [APP] }));
  assert.equal(text(l), 'Watching demo-org/app on main · from App’s folder');
  assert.deepEqual(l.rows, []);
  assert.equal(l.action, null);
  assert.equal(l.led, 'ok');
});

test('two repos: "Watching 2 repos" over two rows, open', () => {
  const l = m.statusLine(state({ repos: [APP, SITE] }), TEAMS);
  assert.equal(text(l), 'Watching 2 repos');
  assert.equal(l.collapsed, false);
  assert.deepEqual(l.rows.map((r) => [r.team, r.repo.repo, r.repo.branch, r.state, r.action]), [
    ['App', 'demo-org/app', 'main', 'Watching', null],
    ['Site', 'demo-org/site', 'trunk', 'Watching', null],
  ]);
});

test('four rows fold behind the summary; each status has its words, LED and at most one action', () => {
  const s = state({ repos: [APP, { ...SITE, status: 'not_connected' }, { ...DOCS, status: 'error', poll_error: 'rate limited' }, LAB, { ...APP, key: 'github.com/demo-org/eleven', repo: 'demo-org/eleven', team: 'Extra', status: 'too_many' }] });
  const l = m.statusLine(s);
  assert.equal(l.collapsed, true);
  assert.equal(text(l), 'Watching 1 repo');
  assert.equal(l.led, 'warn', 'any warn row lights the line amber');
  assert.deepEqual(l.rows.map((r) => [r.state, r.led, r.action]), [
    ['Watching', 'ok', null],
    ['Needs GitHub', 'warn', 'connect'],
    ['Can’t read: rate limited', 'warn', 'remove'],
    ['Read from the next update', 'off', null],
    ['Not watched: 10 repos is the limit', 'off', null],
  ]);
});

test('single-line copy for error and too_many; not_connected with no repo still offers Connect', () => {
  assert.equal(text(m.statusLine(state({ repos: [{ ...APP, status: 'error', poll_error: 'rate limited' }] }))), 'Can’t read demo-org/app: rate limited. The floor and guards are on.');
  assert.equal(text(m.statusLine(state({ repos: [{ ...APP, status: 'too_many' }] }))), 'Found demo-org/app. Dev mode watches 10 repos at most.');
  const nc = m.statusLine(state({ status: 'not_connected', repos: [] }));
  assert.equal(nc.action, 'connect');
  assert.equal(m.statusLine(state({ status: 'no_repo', repos: [] })).action, 'link');
});

test('team_id matches a row to its team even after a rename; only a team with no repo gets Link a repo', () => {
  const l2 = m.statusLine(state({ repos: [{ ...APP, team: 'App (old name)', team_id: 'tm_app' }] }), TEAMS);
  assert.deepEqual(l2.rows.map((r) => r.id), [APP.key, 'team:tm_site']);
  assert.equal(l2.rows[0].teamId, 'tm_app');
});

test('a team with no repo gets a "No repo yet" row with Link a repo', () => {
  const l = m.statusLine(state({ repos: [APP] }), TEAMS);
  assert.equal(l.rows.length, 2);
  assert.deepEqual([l.rows[1].id, l.rows[1].team, l.rows[1].state, l.rows[1].action], ['team:tm_site', 'Site', 'No repo yet', 'link']);
});

// ---------------------------------------------------------------------------
// DOM: the list

test('DOM 1 repo: one role=status line, no list, no disclosure', async () => {
  const { root } = await line({ repos: [APP] });
  assert.equal(document.querySelectorAll('[data-dv-status]').length, 1);
  assert.equal(document.querySelector('[data-dv-status]').getAttribute('role'), 'status');
  assert.equal(has('[data-dv-repos-list]'), false);
  assert.equal(has('[data-dv-disclosure]'), false);
  root.unmount();
});

test('DOM 2 repos: a labelled list, one row each: team · owner/name · branch · status', async () => {
  const { root } = await line({ repos: [APP, SITE] });
  const list = document.querySelector('[data-dv-repos-list]');
  assert.equal(list.tagName, 'UL');
  assert.equal(list.getAttribute('aria-label'), 'Repos Dev mode watches');
  const rows = [...list.querySelectorAll('[data-dv-repo-row]')];
  assert.equal(rows.length, 2);
  assert.match(rows[0].textContent, /App.*demo-org\/app.*main.*Watching/);
  assert.match(rows[1].textContent, /Site.*demo-org\/site.*trunk.*Watching/);
  assert.equal(rows[0].querySelector('code.dv-code').textContent, 'main');
  assert.equal(rows[0].querySelectorAll('button').length, 0, 'a watched team row has no action');
  assert.equal(has('[data-dv-disclosure]'), false, 'two rows stay open');
  root.unmount();
});

test('DOM 4 repos: folded behind "Watching N repos"; the disclosure works by keyboard and names its list', async () => {
  const four = [APP, SITE, DOCS, { ...APP, key: 'github.com/demo-org/api', repo: 'demo-org/api', team: 'API' }];
  const { root } = await line({ repos: four });
  const d = document.querySelector('[data-dv-disclosure]');
  assert.equal(d.tagName, 'BUTTON', 'a real button: Enter and Space work');
  assert.equal(d.getAttribute('aria-expanded'), 'false');
  assert.equal(has('[data-dv-repos-list]'), false);
  assert.match(document.querySelector('[data-dv-status-text]').textContent, /Watching 4 repos/);
  await click(d);
  assert.equal(d.getAttribute('aria-expanded'), 'true');
  const list = document.querySelector('[data-dv-repos-list]');
  assert.equal(d.getAttribute('aria-controls'), list.id);
  assert.equal(list.querySelectorAll('[data-dv-repo-row]').length, 4);
  await click(d);
  assert.equal(has('[data-dv-repos-list]'), false);
  root.unmount();
});

test('DOM states: not_connected rows offer Connect GitHub, error shows why, other_host and too_many have no action', async () => {
  let connected = 0;
  reset({ repos: [{ ...APP, status: 'not_connected' }, { ...SITE, status: 'error', poll_error: 'rate limited' }, LAB] });
  const st = m.parseDevMode(server.state);
  m.devStore.set('spc_demo', st);
  const { root } = await mount(h(m.DevStatusLine, { spaceId: 'spc_demo', state: st, onConnect: () => connected++ }));
  const row = (k) => document.querySelector(`[data-dv-repo-row="${k}"]`);
  assert.equal(row(APP.key).querySelector('[data-action]').textContent.trim(), 'Connect GitHub');
  await click(row(APP.key).querySelector('[data-action]'));
  assert.equal(connected, 1);
  assert.match(row(SITE.key).textContent, /Can’t read: rate limited/);
  assert.equal(row(SITE.key).querySelectorAll('button').length, 0);
  assert.equal(row(LAB.key).querySelectorAll('button').length, 0);
  assert.match(row(LAB.key).textContent, /Read from the next update/);
  root.unmount();
});

test('DOM: Link a repo for a team PUTs {on, repo, team}; the new row appears', async () => {
  const { root } = await line({ repos: [APP] }, TEAMS);
  const btn = document.querySelector('[data-dv-row-action="team:tm_site"]');
  assert.equal(btn.textContent.trim(), 'Link a repo');
  assert.equal(btn.getAttribute('aria-label'), 'Link a repo for Site');
  assert.equal(btn.getAttribute('aria-expanded'), 'false');
  await click(btn);
  const form = document.querySelector('[data-dv-link-form="tm_site"]');
  assert.match(form.textContent, /Repo for Site/);
  const input = form.querySelector('input');
  assert.equal(document.activeElement === input, true, 'focus goes to the field');
  await typeInto(input, 'demo-org/site');
  input.form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  const puts = server.calls.filter(([x]) => x === 'PUT').map(([, , b]) => b);
  assert.deepEqual(puts, [{ on: true, repo: 'demo-org/site', team: 'tm_site' }]);
  assert.equal(m.devStore.get('spc_demo').repos.length, 2);
  root.unmount();
});

test('DOM: Esc closes the Link field and puts focus back on its button', async () => {
  const { root } = await line({ repos: [APP] }, TEAMS);
  const btn = document.querySelector('[data-dv-row-action="team:tm_site"]');
  await click(btn);
  document.querySelector('[data-dv-link-form] input').dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  await frame();
  assert.equal(has('[data-dv-link-form]'), false);
  assert.equal(document.activeElement === document.querySelector('[data-dv-row-action="team:tm_site"]'), true);
  root.unmount();
});

test('DOM: Remove on a hand row sends DELETE …/dev_mode/repos/:key; team rows have no Remove', async () => {
  const { root } = await line({ repos: [APP, SITE, DOCS] });
  assert.equal(document.querySelectorAll('[data-action="remove"]').length, 1);
  const rm = document.querySelector(`[data-dv-row-action="${DOCS.key}"]`);
  assert.equal(rm.getAttribute('aria-label'), 'Remove demo-org/docs');
  await click(rm);
  await frame();
  const del = server.calls.filter(([x]) => x === 'DELETE');
  assert.equal(del.length, 1);
  assert.equal(new URL(del[0][1], 'http://localhost').pathname, `/api/spaces/spc_demo/dev_mode/repos/${encodeURIComponent(DOCS.key)}`);
  assert.equal(m.devStore.get('spc_demo').repos.length, 2);
  assert.equal(document.querySelectorAll('[data-dv-repo-row]').length, 2);
  root.unmount();
});

// ---------------------------------------------------------------------------
// Chips and filters

test('repoFilters: short names, owner/name when two share one; none below 2 readable repos', () => {
  assert.deepEqual(m.repoFilters(state({ repos: [APP, SITE] })).map((f) => [f.key, f.label]), [
    ['github.com/demo-org/app', 'app'],
    ['github.com/demo-org/site', 'site'],
  ]);
  const twin = { ...SITE, key: 'github.com/other-org/app', repo: 'other-org/app' };
  assert.deepEqual(m.repoFilters(state({ repos: [APP, twin] })).map((f) => f.label), ['demo-org/app', 'other-org/app']);
  assert.deepEqual(m.repoFilters(state({ repos: [APP] })), []);
  assert.deepEqual(m.repoFilters(state({ repos: [APP, LAB] })), [], 'other_host has no cards to filter');
});

test('cardRepoLabel: only with more than one repo; unknown keys drop their host', () => {
  assert.equal(m.cardRepoLabel(state({ repos: [APP] }), APP.key), null);
  assert.equal(m.cardRepoLabel(state({ repos: [APP, SITE] }), 'GitHub.com/Demo-Org/Site'), 'demo-org/site');
  assert.equal(m.cardRepoLabel(state({ repos: [APP, SITE] }), 'github.com/demo-org/gone'), 'demo-org/gone');
  assert.equal(m.cardRepoLabel(state({ repos: [APP, SITE] }), null), null);
  assert.equal(m.cardRepoLabel(state({ repos: [APP, SITE] }), 'github.com/demo-org/gone', 'demo-org/gone-renamed'), 'demo-org/gone-renamed', 'the card’s repo_name wins');
});

test('cardsOfRepo: keeps that repo’s cards and cards with no repo (an older server); null keeps all', () => {
  const cards = [{ id: 'a', repo: APP.key }, { id: 's', repo: SITE.key }, { id: 'old' }];
  assert.deepEqual(m.cardsOfRepo(cards, APP.key).map((c) => c.id), ['a', 'old']);
  assert.equal(m.cardsOfRepo(cards, null).length, 3);
});

const withRepo = (card, repo) => ({ ...card, repo, repo_name: repo.split('/').slice(1).join('/') });

test('DOM desk, 2 repos: All · app · site filter chips; a pick loads both lists with ?repo= and shows only that repo', async () => {
  reset({ repos: [APP, SITE] });
  server.landed = [withRepo(m.fx.LANDED_288, APP.key), withRepo(m.fx.LANDED_289, SITE.key)];
  server.reviews = [withRepo(m.fx.REVIEW_290, APP.key), withRepo(m.fx.REVIEW_301, SITE.key)];
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const { root } = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', teams: TEAMS, now: m.fx.FIXTURE_NOW }));
  const group = document.querySelector('[data-dev-repo-filters]');
  assert.equal(group.getAttribute('role'), 'group');
  assert.equal(group.getAttribute('aria-label'), 'Show cards from');
  const chips = [...group.querySelectorAll('button')];
  assert.deepEqual(chips.map((c) => c.textContent), ['All', 'app', 'site']);
  assert.deepEqual(chips.map((c) => c.getAttribute('aria-pressed')), ['true', 'false', 'false']);
  assert.equal(document.querySelectorAll('[data-dev-card="landed"]').length, 2);
  // Each card names its repo.
  assert.deepEqual([...document.querySelectorAll('[data-dev-card="landed"] [data-dev-card-repo]')].map((c) => c.textContent).sort(), ['Repo demo-org/app', 'Repo demo-org/site']);
  assert.equal(document.querySelectorAll('[data-dev-card="review"] [data-dev-card-repo]').length, 0, 'reviews are on the other tab');
  server.calls = [];
  await click(chips[2]);
  await settle();
  assert.equal(chips[2].getAttribute('aria-pressed'), 'true');
  const gets = server.calls.filter(([x, u]) => x === 'GET' && /\/dev\/(landed|reviews)/.test(u)).map(([, u]) => new URL(u, 'http://localhost').searchParams.get('repo'));
  assert.deepEqual(gets, [SITE.key, SITE.key]);
  const landed = [...document.querySelectorAll('[data-dev-card="landed"]')];
  assert.deepEqual(landed.map((c) => c.dataset.repo), [SITE.key]);
  await click(chips[0]);
  assert.equal(document.querySelectorAll('[data-dev-card="landed"]').length, 2, 'All shows the space again');
  root.unmount();
});

test('DOM desk, 1 repo: no filter chips and no repo chip on cards', async () => {
  reset({ repos: [APP] });
  server.landed = [withRepo(m.fx.LANDED_288, APP.key)];
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const { root } = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', now: m.fx.FIXTURE_NOW }));
  assert.equal(document.querySelectorAll('[data-dev-card="landed"]').length, 1);
  assert.equal(has('[data-dev-repo-filters]'), false);
  assert.equal(has('[data-dev-card-repo]'), false);
  assert.equal(server.calls.some(([, u]) => u.includes('repo=')), false);
  root.unmount();
});

test('devApi: Draft release notes, landed and reviews send ?repo= when a repo is picked, nothing otherwise', async () => {
  reset({ repos: [APP, SITE] });
  await m.devApi.releaseNotes('spc_demo', SITE.key).catch(() => null);
  await m.devApi.releaseNotes('spc_demo').catch(() => null);
  await m.devApi.landed('spc_demo', undefined, SITE.key);
  await m.devApi.reviews('spc_demo', null);
  const qs = server.calls.map(([meth, u]) => `${meth} ${new URL(u, 'http://localhost').pathname.split('/').pop()} ${new URL(u, 'http://localhost').searchParams.get('repo')}`);
  assert.deepEqual(qs, [`POST release_notes ${SITE.key}`, 'POST release_notes null', `GET landed ${SITE.key}`, 'GET reviews null']);
});
