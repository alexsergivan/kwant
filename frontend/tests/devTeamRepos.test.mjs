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
    const team = [...TEAMS, { id: 'tm_mkt', name: 'Marketing' }].find((t) => t.id === body.team);
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
  assert.equal(text(l), 'Watching demo-org/app · main');
  assert.deepEqual(l.rows, []);
  assert.equal(l.action, null);
  assert.equal(l.led, 'ok');
});

test('two repos: "Watching 2 repos" over two rows, open', () => {
  const l = m.statusLine(state({ repos: [APP, SITE] }), TEAMS);
  assert.equal(text(l), 'Watching 2 repos');
  assert.equal(l.collapsed, false);
  assert.deepEqual(l.rows.map((r) => [r.team, r.repo.repo, r.repo.branch, r.state.replace(/\d\d:\d\d( [AP]M)?/, 'HH:MM'), r.origin, r.action]), [
    ['App', 'demo-org/app', 'main', 'Watching · checked HH:MM', 'From App’s folder', null],
    ['Site', 'demo-org/site', 'trunk', 'Watching · checked HH:MM', 'From Site’s repo link', null],
  ]);
  assert.equal(l.rows[0].teamNote, 'Comes from App’s folder. Change it in the team’s settings.');
  assert.equal(l.rows[1].teamNote, 'Comes from Site’s repo link. Change it in the team’s settings.');
  assert.equal(m.statusLine(state({ repos: [APP, DOCS] })).rows[1].origin, 'Added by you');
  assert.equal(m.statusLine(state({ repos: [APP, DOCS] })).rows[1].teamNote, null, 'a hand row has Remove, not the team note');
});

test('four rows fold behind the summary; each status has its words, LED and at most one action', () => {
  const s = state({ repos: [APP, { ...SITE, status: 'not_connected' }, { ...DOCS, status: 'error', poll_error: 'rate limited' }, LAB, { ...APP, key: 'github.com/demo-org/eleven', repo: 'demo-org/eleven', team: 'Extra', status: 'too_many' }] });
  const l = m.statusLine(s);
  assert.equal(l.collapsed, true);
  assert.equal(text(l), 'Watching 1 of 5 repos · 2 need you', 'needs you = not_connected + error');
  assert.equal(l.led, 'bad', 'a failing repo lights the line red');
  assert.deepEqual(l.rows.map((r) => [r.state.replace(/\d\d:\d\d( [AP]M)?/, 'HH:MM'), r.led, r.action]), [
    ['Watching · checked HH:MM', 'ok', null],
    ['Needs your GitHub connection', 'warn', 'connect'],
    ['Can’t read it right now. Trying again.', 'bad', 'remove'],
    ['gitlab.example.test isn’t read yet', 'off', null],
    ['Not watched: 10 repos is the limit', 'off', null],
  ]);
  assert.equal(l.rows[2].title, 'rate limited', 'the poll error is the tooltip');
  assert.equal(m.statusLine(state({ repos: [{ ...LAB, from: 'local', host: null }, APP] })).rows[0].state, 'Local folder, no remote');
  assert.equal(m.reposSummary(state({ repos: [APP, SITE, DOCS, { ...APP, key: 'github.com/demo-org/api' }] })), 'Watching 4 repos');
  assert.equal(m.reposSummary(state({ repos: [APP, SITE, DOCS, { ...APP, key: 'github.com/demo-org/api', status: 'error' }] })), 'Watching 3 of 4 repos · 1 needs you');
  assert.equal(m.reposSummary(state({ repos: [APP, SITE, DOCS, { ...APP, key: 'github.com/demo-org/api', status: 'too_many' }] })), 'Watching 3 of 4 repos', 'too_many does not need you');
});

test('poll errors never show a URL with its token', () => {
  const l = m.statusLine(state({ repos: [APP, { ...SITE, status: 'error', poll_error: 'fetch https://demo:not-a-real-token@github.com/demo-org/site failed' }] }));
  assert.equal(l.rows[1].title, 'fetch https://github.com/demo-org/site failed');
});

test('single-line copy for error and too_many; not_connected with no repo still offers Connect', () => {
  assert.equal(text(m.statusLine(state({ repos: [{ ...APP, status: 'error', poll_error: 'rate limited' }] }))), 'demo-org/app · Can’t read it right now. Trying again.');
  assert.equal(text(m.statusLine(state({ repos: [{ ...APP, status: 'too_many' }] }))), 'demo-org/app · Not watched: 10 repos is the limit');
  assert.equal(text(m.statusLine(state({ repos: [{ ...APP, status: 'not_connected' }] }))), 'demo-org/app · Needs your GitHub connection');
  const nc = m.statusLine(state({ status: 'not_connected', repos: [] }));
  assert.equal(nc.action, 'connect');
  assert.equal(m.statusLine(state({ status: 'no_repo', repos: [] })).action, 'link');
});

test('team_id matches a row to its team even after a rename; only a team with no repo gets Link a repo', () => {
  const l2 = m.statusLine(state({ repos: [{ ...APP, team: 'App (old name)', team_id: 'tm_app', status: 'not_connected' }] }), TEAMS);
  assert.deepEqual(l2.rows.map((r) => r.id), [APP.key, 'team:tm_site']);
  assert.equal(l2.rows[0].teamId, 'tm_app');
});

test('§9b: no repo watched at all, every team with none gets a "No repo" row with Link a repo', () => {
  const l = m.statusLine(state({ repos: [{ ...APP, status: 'not_connected' }] }), TEAMS);
  assert.equal(l.rows.length, 2);
  assert.deepEqual([l.rows[1].id, l.rows[1].team, l.rows[1].state, l.rows[1].action], ['team:tm_site', 'Site', 'No repo', 'link']);
  assert.deepEqual(l.noRepoTeams, []);
});

test('§9b: with a repo watched, teams with no repo fold into one quiet line and never count as needing you', () => {
  const teams = [...TEAMS, { id: 'tm_mkt', name: 'Marketing' }, { id: 'tm_ops', name: 'Ops' }];
  const l = m.statusLine(state({ repos: [APP] }), teams);
  assert.deepEqual(l.rows, [], 'one repo stays a single line');
  assert.deepEqual(l.noRepoTeams.map((t) => t.name), ['Site', 'Marketing', 'Ops']);
  assert.equal(m.noRepoTeamsText(3), '3 teams have no repo');
  assert.equal(m.noRepoTeamsText(1), '1 team has no repo');
  assert.equal(text(m.statusLine(state({ repos: [APP, SITE, DOCS, { ...APP, key: 'github.com/demo-org/api' }] }), teams)), 'Watching 4 repos');
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
  assert.match(rows[0].textContent, /App.*demo-org\/app.*main.*Watching.*From App’s folder/);
  assert.match(rows[1].textContent, /Site.*demo-org\/site.*trunk.*Watching.*From Site’s repo link/);
  assert.equal(rows[0].querySelector('code.dv-code').textContent, 'main');
  assert.equal(rows[0].querySelectorAll('[data-action]').length, 0, 'a watched team row has no action');
  const info = rows[1].querySelector('[data-dv-team-note]');
  assert.equal(info.getAttribute('aria-label'), 'Comes from Site’s repo link. Change it in the team’s settings.');
  assert.equal(info.getAttribute('aria-expanded'), 'false');
  await click(info);
  assert.equal(info.getAttribute('aria-expanded'), 'true');
  assert.match(rows[1].querySelector('[data-dv-team-note-text]').textContent, /Change it in the team’s settings/);
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
  assert.equal(d.textContent, 'Show repos');
  await click(d);
  assert.equal(d.getAttribute('aria-expanded'), 'true');
  const list = document.querySelector('[data-dv-repos-list]');
  assert.equal(d.getAttribute('aria-controls'), list.id);
  assert.equal(list.querySelectorAll('[data-dv-repo-row]').length, 4);
  assert.equal(d.textContent, 'Hide repos');
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
  assert.match(row(SITE.key).textContent, /Can’t read it right now\. Trying again\./);
  assert.equal(row(SITE.key).querySelector('[data-dv-repo-state]').getAttribute('title'), 'rate limited');
  assert.equal(row(SITE.key).querySelectorAll('[data-action]').length, 0);
  // A team folder row has no action and no Remove: only the info note (spec §6).
  assert.equal(row(LAB.key).querySelectorAll('[data-action]').length, 0);
  assert.equal(row(LAB.key).querySelectorAll('button').length, 1);
  assert.equal(row(LAB.key).querySelectorAll('[data-dv-team-note]').length, 1);
  assert.match(row(LAB.key).textContent, /gitlab\.example\.test isn’t read yet/);
  root.unmount();
});

test('DOM: Link a repo for a team PUTs {on, repo, team}; the new row appears', async () => {
  const { root } = await line({ repos: [{ ...APP, status: 'not_connected' }] }, TEAMS);
  const btn = document.querySelector('[data-dv-row-action="team:tm_site"]');
  assert.equal(btn.textContent.trim(), 'Link a repo');
  assert.equal(btn.getAttribute('aria-label'), 'Link a repo for Site');
  assert.equal(btn.getAttribute('aria-expanded'), 'false');
  await click(btn);
  const form = document.querySelector('[data-dv-link-form="tm_site"]');
  assert.match(form.textContent, /Repo for Site/);
  assert.match(form.textContent, /Saved as this team’s repo link\. Bots only read it\./);
  assert.match(form.textContent, /Save.*Cancel/);
  const input = form.querySelector('input');
  assert.equal(input.getAttribute('placeholder'), 'owner/name or its URL');
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
  const { root } = await line({ repos: [{ ...APP, status: 'not_connected' }] }, TEAMS);
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
  assert.equal(rm.getAttribute('aria-label'), 'Stop watching demo-org/docs');
  await click(rm);
  await frame();
  assert.equal(document.querySelector('[data-dv-removed]').getAttribute('role'), 'status');
  assert.match(document.querySelector('[data-dv-removed]').textContent, /^Stopped watching demo-org\/docs\. Its cards stay\.Undo$/);
  assert.equal(document.activeElement === document.querySelector('[data-dv-undo]'), true, 'focus lands on Undo');
  const del = server.calls.filter(([x]) => x === 'DELETE');
  assert.equal(del.length, 1);
  assert.equal(new URL(del[0][1], 'http://localhost').pathname, `/api/spaces/spc_demo/dev_mode/repos/${encodeURIComponent(DOCS.key)}`);
  assert.equal(m.devStore.get('spc_demo').repos.length, 2);
  assert.equal(document.querySelectorAll('[data-dv-repo-row]').length, 2);
  // Undo puts it back by hand (PUT {repo}, no team).
  await click(document.querySelector('[data-dv-undo]'));
  const puts = server.calls.filter(([x]) => x === 'PUT').map(([, , b]) => b);
  assert.deepEqual(puts, [{ on: true, repo: 'demo-org/docs' }]);
  assert.equal(has('[data-dv-removed]'), false);
  assert.equal(document.querySelectorAll('[data-dv-repo-row]').length, 3);
  root.unmount();
});

test('DOM: a 409 team_repo on Remove shows the team note as the error', async () => {
  const { root } = await line({ repos: [APP, SITE, DOCS] });
  server.state = { ...server.state, repos: server.state.repos.map((r) => (r.key === DOCS.key ? { ...r, added_by: 'team', team: 'Docs' } : r)) };
  const realFetch = g.fetch;
  g.fetch = g.window.fetch = async (url, init = {}) =>
    init.method === 'DELETE' ? new Response(JSON.stringify({ error: 'That repo comes from a team.', code: 'team_repo' }), { status: 409, headers: { 'Content-Type': 'application/json' } }) : realFetch(url, init);
  await click(document.querySelector(`[data-dv-row-action="${DOCS.key}"]`));
  await frame();
  g.fetch = g.window.fetch = realFetch;
  assert.match(document.querySelector('[role="alert"]').textContent, /Comes from .*’s repo link\. Change it in the team’s settings\./);
  root.unmount();
});

test('DOM §9b: the quiet line "2 teams have no repo · Link a repo" opens a team picker, then the repo field', async () => {
  const teams = [...TEAMS, { id: 'tm_mkt', name: 'Marketing' }];
  const { root } = await line({ repos: [APP] }, teams);
  const q = document.querySelector('[data-dv-norepo]');
  assert.match(q.textContent, /^2 teams have no repo·Link a repo$/);
  const b = q.querySelector('[data-dv-norepo-action]');
  await click(b);
  assert.equal(b.getAttribute('aria-expanded'), 'true');
  const pick = q.querySelector('[data-dv-team-pick]');
  assert.deepEqual([...pick.options].map((o) => o.textContent), ['Site', 'Marketing']);
  await frame();
  assert.equal(document.activeElement === pick, true, 'the picker comes first');
  pick.value = 'tm_mkt';
  pick.dispatchEvent(new window.Event('change', { bubbles: true }));
  await settle();
  assert.match(q.textContent, /Repo for Marketing/);
  await typeInto(q.querySelector('input'), 'demo-org/brand');
  q.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(server.calls.filter(([x]) => x === 'PUT').map(([, , b2]) => b2), [{ on: true, repo: 'demo-org/brand', team: 'tm_mkt' }]);
  root.unmount();
});

test('§9b (Mira, final): one team says "1 team has no repo"; the line is neutral; Link a repo is 44 px on phone; counts are repos, never teams', async () => {
  const { root } = await line({ repos: [APP] }, TEAMS);
  const q = document.querySelector('[data-dv-norepo]');
  assert.match(q.textContent, /^1 team has no repo·Link a repo$/);
  assert.equal(!!q.querySelector('.dv-led, [data-s]'), false, 'no dot');
  root.unmount();
  const { readFile } = await import('node:fs/promises');
  const css = await readFile(new URL('../src/styles/devStatus.css', import.meta.url), 'utf8');
  const rule = css.match(/\.kw-dv-norepo, \.kw-dv-removed \{([^}]*)\}/)[1];
  assert.match(rule, /color: var\(--t3\)/, 'text-tertiary');
  assert.doesNotMatch(rule, /warn|amber|#f59e0b/);
  assert.match(css, /@media \(max-width: 639px\) \{[^@]*\.kw-dv-norepo-act \{ min-height: 44px;/);
  const teams3 = [...TEAMS, { id: 'tm_mkt', name: 'Marketing' }, { id: 'tm_ops', name: 'Ops' }];
  assert.equal(m.reposSummary(state({ repos: [APP, SITE] })), 'Watching 2 repos', 'teams without a repo do not change the count');
  assert.equal(m.teamsWithoutRepo(state({ repos: [APP, SITE] }), teams3).length, 2);
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
  assert.deepEqual([...document.querySelectorAll('[data-dev-card="landed"] [data-dev-card-repo]')].map((c) => [c.textContent, c.title]).sort(), [
    ['Repo demo-org/appapp', 'demo-org/app'],
    ['Repo demo-org/sitesite', 'demo-org/site'],
  ], 'the chip says the repo name; screen readers and the title get owner/name');
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

// ---------------------------------------------------------------------------
// Ada's answers (2026-11-06): 2, Dev mode off reads "Will watch" (Mira's copy), never watched; 5, Undo brings a repo back
// the way it came.

test('answer 2: Dev mode off, a watching repo is a neutral "Will watch" (no green); other statuses stay', () => {
  const off = (repos) => m.statusLine(state({ on: false, repos }), TEAMS);
  const l = off([APP, SITE, { ...DOCS, status: 'not_connected' }, { ...DOCS, key: 'github.com/demo-org/err', repo: 'demo-org/err', status: 'error' }]);
  assert.deepEqual(l.rows.map((r) => [r.state, r.led]), [
    ['Will watch', 'off'],
    ['Will watch', 'off'],
    ['Needs your GitHub connection', 'warn'],
    ['Can’t read it right now. Trying again.', 'bad'],
  ]);
  assert.equal(text(l), 'Will watch 2 of 4 repos · 2 need you');
  assert.equal(text(off([APP, SITE])), 'Will watch 2 repos');
  assert.equal(off([APP, SITE]).led, 'off', 'no green LED while off');
  const one = off([APP]);
  assert.deepEqual([text(one), one.led], ['demo-org/app · main · Will watch', 'off']);
  const tooMany = m.statusLine(state({ on: false, repos: [{ ...APP, status: 'too_many' }] }));
  assert.deepEqual([text(tooMany), tooMany.led], ['demo-org/app · Not watched: 10 repos is the limit', 'off']);
  // On, nothing changes.
  assert.equal(text(m.statusLine(state({ repos: [APP, SITE] }), TEAMS)), 'Watching 2 repos');
});

test('DOM answer 2: off, the Will watch chip is neutral and the desk header says "Dev mode is off"', async () => {
  const { root } = await line({ on: false, repos: [APP, SITE] });
  const chips = [...document.querySelectorAll('[data-dv-repo-state]')];
  assert.deepEqual(chips.map((c) => [c.textContent, c.dataset.tone]), [
    ['Will watch', 'neutral'],
    ['Will watch', 'neutral'],
  ]);
  for (const c of chips) {
    assert.equal(c.getAttribute('title'), 'Will watch when Dev mode is on');
    assert.equal(c.getAttribute('aria-label'), 'Will watch when Dev mode is on');
  }
  assert.deepEqual([...document.querySelectorAll('[data-dv-repo-row] .dv-led')].map((d) => d.dataset.s), ['off', 'off']);
  root.unmount();
  const { readFile } = await import('node:fs/promises');
  const css = await readFile(new URL('../src/styles/devStatus.css', import.meta.url), 'utf8');
  const rule = css.match(/\.kw-dv-repo-state\[data-tone='neutral'\] \{([^}]*)\}/)[1];
  assert.doesNotMatch(rule, /ok|green/, 'the Will watch chip is never green');

  reset({ on: false, repos: [APP] });
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const desk = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', now: m.fx.FIXTURE_NOW }));
  assert.equal(document.querySelector('[data-dev-desk-sub]').textContent, 'Dev mode is off');
  desk.root.unmount();
  reset({ repos: [APP] });
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const onDesk = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', now: m.fx.FIXTURE_NOW }));
  assert.equal(document.querySelector('[data-dev-desk-sub]').textContent, 'What landed and what needs a look');
  onDesk.root.unmount();
});

test('answer 5: Undo after clearing a team’s repo sends PUT {repo: the previous link, team: its id}', async () => {
  assert.deepEqual(m.undoBody({ repo: 'https://github.com/demo-org/site', team: 'tm_site' }), { repo: 'https://github.com/demo-org/site', team: 'tm_site' });
  reset({ repos: [APP] });
  await m.devApi.linkRepo('spc_demo', m.undoBody({ repo: 'https://github.com/demo-org/site', team: 'tm_site' }));
  const puts = server.calls.filter(([x]) => x === 'PUT').map(([, , b]) => b);
  assert.deepEqual(puts, [{ on: true, repo: 'https://github.com/demo-org/site', team: 'tm_site' }], 'the team link comes back, never a hand add');
  assert.equal(server.state.repos.at(-1).added_by, 'team');
});

test('answer 5: Undo for a hand row stays PUT {repo}, with no team', async () => {
  assert.deepEqual(m.undoBody({ repo: 'demo-org/docs', team: null }), { repo: 'demo-org/docs', team: null });
  reset({ repos: [APP] });
  await m.devApi.linkRepo('spc_demo', m.undoBody({ repo: 'demo-org/docs', team: null }));
  const puts = server.calls.filter(([x]) => x === 'PUT').map(([, , b]) => b);
  assert.deepEqual(puts, [{ on: true, repo: 'demo-org/docs' }]);
  assert.equal(server.state.repos.at(-1).added_by, 'hand');
});

test('answer 5: the desk’s Undo goes through undoBody, and a removed team link keeps its team id', async () => {
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(new URL('../src/components/DevStatusLine.tsx', import.meta.url), 'utf8');
  assert.match(src, /save\(spaceId, undoBody\(removed\)\)/);
  assert.doesNotMatch(src, /repo: removed\.repo, team: null/);
  assert.match(src, /added_by === 'team' && repo\.from === 'repo' \? row\.teamId : null/);
});

test('addendum 4: clearing a team’s link toasts "Cleared {team}’s repo link." with Undo; both toasts last 5 s', async () => {
  assert.equal(m.clearedText('Site'), 'Cleared Site’s repo link.');
  assert.equal(m.UNDO_TOAST_MS, 5000);
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(new URL('../src/components/DevStatusLine.tsx', import.meta.url), 'utf8');
  assert.match(src, /removed\.team \? clearedText\(removed\.teamName/);
  assert.match(src, /setTimeout\(\(\) => setRemoved\(null\), UNDO_TOAST_MS\)/);
  // The hand Remove toast goes after 5 s.
  const { root } = await line({ repos: [APP, SITE, DOCS] });
  await click(document.querySelector(`[data-dv-row-action="${DOCS.key}"]`));
  await frame();
  assert.ok(document.querySelector('[data-dv-removed]'));
  await new Promise((r) => setTimeout(r, m.UNDO_TOAST_MS + 50));
  await settle();
  assert.equal(!!document.querySelector('[data-dv-removed]'), false);
  root.unmount();
});
