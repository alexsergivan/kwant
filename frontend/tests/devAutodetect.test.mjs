// 0.7.1 Dev mode on the existing setup (any-git S5, Pixel): GET dev_mode's `sources` and `status`
// parse (absent on an older server), no token ever reaches the screen, the Dev desk's one status line
// for each status, Switch's one-tap list and typed repo, Connect, and the first-run intro.
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

// Demo data only: no real repos, tokens or emails.
const base = { on: true, repo: null, branch: null, deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null };
const SOURCES = [
  { team: 'Web', repo: 'demo-org/demo-web', host: 'github.com', from: 'folder' },
  { team: 'API', repo: 'demo-org/demo-api', host: 'github.com', from: 'repo' },
  { team: 'Docs', repo: 'demo-docs', host: null, from: 'local' },
];
let server;
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  server.calls.push([method, u.split('?')[0], body]);
  const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
  if (server.fail && method === 'PUT') return json({ error: server.fail }, 422);
  if (server.unreadable && method === 'PUT' && body?.repo) {
    server.state = { ...server.state, on: true, repo: null, status: 'no_repo' };
    return json({ ...server.state, check: { state: server.unreadable, repo: body.repo, branch: null, deploy_hint: null } });
  }
  // Password store and approvals (B2): demo values only.
  const path = u.split('?')[0];
  if (path === '/api/passwords/store') return json(server.store ?? { store: 'local', local_count: 0, approvals_off: false });
  if (path === '/api/spaces' && method === 'GET') return json(u.includes('archived=1') ? [] : server.spaces ?? []);
  if (path === '/api/kwants' && method === 'GET') return json([]);
  if (/^\/api\/spaces\/[^/]+\/approvals$/.test(path) && method === 'PUT') {
    server.spaces = (server.spaces ?? []).map((sp) => (path.includes(sp.id) ? { ...sp, approvals: body.approvals } : sp));
    if (server.store && server.spaces.every((sp) => sp.approvals === 'ask')) server.store = { ...server.store, approvals_off: false };
    return json({ approvals: body.approvals });
  }
  if (u.includes('/dev_mode')) {
    if (method === 'PUT') server.state = { ...server.state, ...body, status: 'watching' };
    return json(server.state);
  }
  if (u.includes('/dev/landed')) return json({ cards: [], rollup: { count: 0, areas: [] }, last_tag: null, last_merge: null });
  if (u.includes('/dev/reviews')) return json({ cards: [] });
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/devMode';
  export { DevDeskView, DevDeskRow, deskStore, DEV_FEATURES, rowBadge, firstPolling } from './src/components/DevDesk';
  export { DevStatusLine } from './src/components/DevStatusLine';
  export { DevOnToast } from './src/components/DevOnToast';
  export { DevModeSwitch, DevOffNote } from './src/components/DevModeSwitch';
  export { LooseSources, AddSourceForm } from './src/components/LooseEndsView';
  export { saveGate, SAVE_FIRST } from './src/passwordStore';
  export { nextTabStop } from './src/components/DevOnToast';
  export { lastOn } from './src/components/DevModeSwitch';
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
const reset = (over = {}) => {
  server = { state: { ...base, ...over }, calls: [], fail: null };
  m.devStore._reset();
  m.deskStore._reset();
  localStorage.clear();
  document.body.innerHTML = '';
};

// ---------------------------------------------------------------------------
// Parsing

test('parseDevMode: an older server (no sources, no status) gives [] and null', () => {
  const s = m.parseDevMode(base);
  assert.deepEqual(s.sources, []);
  assert.equal(s.status, null);
  assert.equal(m.statusLine(s), null, 'no status line: the desk keeps the old repo chip');
});

test('parseDevMode: sources keep team, repo, host and from; bad rows and unknown statuses drop', () => {
  const s = m.parseDevMode({ ...base, status: 'watching', sources: [...SOURCES, { team: 'X', repo: 'a/b', from: 'ftp' }, { repo: 'a/b', from: 'repo' }, 'nope'] });
  assert.deepEqual(s.sources, SOURCES);
  assert.equal(s.status, 'watching');
  assert.equal(m.parseDevMode({ ...base, status: 'bogus' }).status, null);
  for (const st of ['watching', 'no_repo', 'not_connected', 'other_host']) assert.equal(m.parseDevMode({ ...base, status: st }).status, st);
});

test('no URL token reaches the screen: userinfo is stripped from repo and sources', () => {
  const s = m.parseDevMode({
    ...base,
    status: 'other_host',
    repo: 'https://demo-user:not-a-real-token@git.example.test/demo/app.git',
    sources: [{ team: 'Web', repo: 'https://oauth2:not-a-real-token@git.example.test/demo/app.git', host: 'git.example.test', from: 'folder' }],
  });
  assert.equal(s.repo, 'https://git.example.test/demo/app.git');
  assert.equal(s.sources[0].repo, 'https://git.example.test/demo/app.git');
  assert.doesNotMatch(JSON.stringify(m.statusLine(s)), /token|demo-user|oauth2/);
});

test('typedRepo: GitHub owner/name and URLs normalise; any git URL is kept without its userinfo', () => {
  assert.equal(m.typedRepo('https://github.com/demo-org/demo-web'), 'demo-org/demo-web');
  assert.equal(m.typedRepo('demo-org/demo-web'), 'demo-org/demo-web');
  assert.equal(m.typedRepo('https://gitlab.example.test/demo/app.git'), 'https://gitlab.example.test/demo/app.git');
  assert.equal(m.typedRepo('https://u:p@codeberg.example.test/demo/app'), 'https://codeberg.example.test/demo/app');
  assert.equal(m.typedRepo('git@gitea.example.test:demo/app.git'), 'git@gitea.example.test:demo/app.git');
  assert.equal(m.typedRepo('not a repo'), null);
  assert.equal(m.typedRepo(''), null);
});

// ---------------------------------------------------------------------------
// The status line, one per status (draft copy from any-git.md S5)

const text = (l) => l.parts.map((p) => p.text).join('');

test('watching: Mira’s copy, "Watching {repo} on {branch} · from {team}’s folder" or "’s repo link"; Switch only with several sources', () => {
  const one = m.statusLine(state({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }));
  assert.equal(text(one), 'Watching demo-org/demo-web on main · from Web’s folder');
  assert.equal(one.action, null);
  assert.deepEqual(one.parts.filter((p) => p.mono).map((p) => p.text), ['demo-org/demo-web', 'main']);
  const link = m.statusLine(state({ status: 'watching', repo: 'demo-org/demo-api', branch: 'trunk', sources: [SOURCES[1]] }));
  assert.equal(text(link), 'Watching demo-org/demo-api on trunk · from API’s repo link');
  const many = m.statusLine(state({ status: 'watching', repo: 'demo-org/demo-api', branch: 'trunk', sources: SOURCES }));
  assert.equal(text(many), 'Watching demo-org/demo-api on trunk · 1 of 3 repos found');
  assert.equal(many.action, 'switch');
  const local = m.statusLine(state({ status: 'watching', repo: 'demo-docs', sources: [SOURCES[2]] }));
  assert.equal(text(local), 'Docs’s folder has a local repo with no remote. It’s read from the next update. The floor and guards are on now.');
  const typed = m.statusLine(state({ status: 'watching', repo: 'demo-org/elsewhere', branch: null, sources: [] }));
  assert.equal(text(typed), 'Watching demo-org/elsewhere on main', 'a typed repo names no team');
});

test('not_connected: "Found {repo}. Reviews need your GitHub connection." with Connect GitHub', () => {
  const l = m.statusLine(state({ status: 'not_connected', repo: 'demo-org/demo-web', sources: [SOURCES[0]] }));
  assert.equal(text(l), 'Found demo-org/demo-web. Reviews need your GitHub connection.');
  assert.equal(l.action, 'connect');
});

test('other_host: "Found {repo}. Repos on {host} are read from the next update. The floor and guards are on now."', () => {
  const l = m.statusLine(state({ status: 'other_host', repo: 'https://gitlab.example.test/demo/app', sources: [{ team: 'Web', repo: 'https://gitlab.example.test/demo/app', host: 'gitlab.example.test', from: 'folder' }] }));
  assert.equal(text(l), 'Found https://gitlab.example.test/demo/app. Repos on gitlab.example.test are read from the next update. The floor and guards are on now.');
  assert.deepEqual(l.parts.filter((p) => p.mono).map((p) => p.text), ['https://gitlab.example.test/demo/app', 'gitlab.example.test']);
  assert.equal(l.action, null);
});

test('no_repo: "No git repo in this space’s folders yet. The floor and guards are on." with Type a repo', () => {
  const l = m.statusLine(state({ status: 'no_repo', sources: [] }));
  assert.equal(text(l), 'No git repo in this space’s folders yet. The floor and guards are on.');
  assert.equal(l.action, 'type');
});

test('flow F3: the switch-on toast by status (Mira’s flow copy §3), with its one action', () => {
  const t = (over) => {
    const o = m.onToast(over === null ? null : state(over));
    return [m.onToastText(o), o.action];
  };
  assert.deepEqual(t({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }), ['Dev mode on. Watching demo-org/demo-web on main.', 'open']);
  assert.deepEqual(t({ status: 'not_connected', repo: 'demo-org/demo-web', sources: [SOURCES[0]] }), ['Dev mode on · guards are up. Found demo-org/demo-web. Connect GitHub to get reviews.', 'connect']);
  assert.deepEqual(t({ status: 'other_host', repo: 'https://git.example.test/demo/app', sources: [{ team: 'Web', repo: 'https://git.example.test/demo/app', host: 'git.example.test', from: 'folder' }] }), [
    'Dev mode on · guards are up. git.example.test repos are read from the next update. The floor works now.',
    'open',
  ]);
  assert.deepEqual(t({ status: 'no_repo', sources: [] }), ['Dev mode on · guards are up. No git repo in this space’s folders yet. The floor works now.', 'type']);
  assert.deepEqual(t({}), ['Dev mode on. Looking for a git repo in your teams’ folders…', 'open'], 'checking: no status yet');
  assert.equal(m.onToast(state({})).checking, true);
  assert.equal(t({ status: 'watching', repo: 'demo-docs', sources: [SOURCES[2]] })[1], 'open', 'a local repo is not watched yet');
  assert.match(t({ status: 'watching', repo: 'demo-docs', sources: [SOURCES[2]] })[0], /guards are up\. Docs’s folder has a local repo with no remote/);
});

test('first-run block copy (flow copy §5): three numbered lines', () => {
  assert.deepEqual(
    m.DEV_FEATURES.map((f) => `${f.n} ${f.title} ${f.line}`),
    [
      '01 What landed. Every merge to main in plain words, and when main is ready to ship.',
      '02 Reviews. A second bot reads each pull request before you merge it.',
      '03 Production floor. Deploys and live-database commands wait for your OK.',
    ],
  );
});

// ---------------------------------------------------------------------------
// In the Dev desk

const desk = async (over, props = {}) => {
  reset(over);
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  return mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', ...props }));
};

test('DOM: each status renders one status line under the header; an older server keeps the repo chip', async () => {
  for (const [st, extra] of [
    ['watching', { repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }],
    ['not_connected', { repo: 'demo-org/demo-web', sources: [SOURCES[0]] }],
    ['other_host', { repo: 'https://gitlab.example.test/demo/app', sources: [{ team: 'Web', repo: 'https://gitlab.example.test/demo/app', host: 'gitlab.example.test', from: 'folder' }] }],
    ['no_repo', { sources: [] }],
  ]) {
    const { root } = await desk({ status: st, ...extra }, { onOpenLooseEnds() {} });
    const lines = document.querySelectorAll('[data-dv-status]');
    assert.equal(lines.length, 1, st);
    assert.equal(lines[0].dataset.dvStatus, st);
    assert.equal(lines[0].getAttribute('role'), 'status');
    assert.equal(document.querySelector('[data-dev-repo-chip]'), null, `${st}: no duplicate repo chip`);
    assert.ok(document.querySelector('[data-dev-intro]'), `${st}: first-run intro on the first visit`);
    root.unmount();
  }
  const { root } = await desk({ repo: 'demo-org/demo-web', branch: 'main' });
  assert.equal(document.querySelector('[data-dv-status]'), null);
  assert.ok(document.querySelector('[data-dev-repo-chip]'));
  root.unmount();
});

test('PX-2 (style-v2 §3/§4): header chip and subtitle, LED per status, Checked in the status line, Refresh 44 px on phone', async () => {
  const want = { watching: 'ok', not_connected: 'warn', other_host: 'off', no_repo: 'off' };
  for (const [st, extra] of [
    ['watching', { repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }],
    ['not_connected', { repo: 'demo-org/demo-web', sources: [SOURCES[0]] }],
    ['other_host', { repo: 'https://gitlab.example.test/demo/app', sources: [{ team: 'Web', repo: 'https://gitlab.example.test/demo/app', host: 'gitlab.example.test', from: 'folder' }] }],
    ['no_repo', { sources: [] }],
  ]) {
    const { root } = await desk({ status: st, ...extra }, { onOpenLooseEnds() {} });
    const line = document.querySelector('[data-dv-status]');
    assert.equal(line.querySelector('[data-dv-led]').dataset.s, want[st], `${st}: LED`);
    assert.match(line.querySelector('[data-dev-checked]').textContent, /^Checked \d\d:\d\d$/, `${st}: Checked lives in the status line`);
    assert.equal(document.querySelectorAll('[data-dev-checked]').length, 1, `${st}: not in the header too`);
    root.unmount();
  }
  const { root } = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'trunk', sources: [SOURCES[0]] });
  const glyph = document.querySelector('.kw-dv-glyph');
  assert.equal(glyph.textContent.trim(), '</>');
  const sub = [...document.querySelectorAll('[data-dev-desk] p')].find((p) => p.textContent === 'What landed and what needs a look');
  assert.ok(sub && sub.classList.contains('hidden') && sub.classList.contains('sm:block'), 'subtitle hides under 640');
  const refresh = document.querySelector('[data-dev-refresh]');
  assert.equal(refresh.getAttribute('aria-label'), 'Refresh');
  assert.ok(refresh.classList.contains('max-sm:w-11') && refresh.classList.contains('max-sm:h-11'), 'icon-only 44x44 on phone');
  const text = document.querySelector('[data-dv-status-text]');
  assert.equal(text.querySelector('b').textContent, 'demo-org/demo-web', 'repo reads as a bold name');
  assert.equal(text.querySelector('code.dv-code').textContent, 'trunk', 'branch is code');
  root.unmount();
});

test('DOM: Connect opens Loose ends; it hides when the app gives no way there', async () => {
  let opened = 0;
  const { root } = await desk({ status: 'not_connected', repo: 'demo-org/demo-web', sources: [SOURCES[0]] }, { onOpenLooseEnds: () => opened++ });
  const connect = document.querySelector('[data-dv-status-action="connect"]');
  assert.equal(connect.textContent.trim(), 'Connect GitHub');
  await click(connect);
  assert.equal(opened, 1);
  root.unmount();
  const again = await desk({ status: 'not_connected', sources: [] });
  assert.equal(document.querySelector('[data-dv-status-action]'), null);
  again.root.unmount();
});

test('DOM: Switch opens a one-tap list; a tap sends PUT {on: true, repo} and the line follows', async () => {
  const { root } = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: SOURCES });
  const sw = document.querySelector('[data-dv-status-action="switch"]');
  assert.equal(sw.getAttribute('aria-expanded'), 'false');
  await click(sw);
  assert.equal(sw.getAttribute('aria-expanded'), 'true');
  const picks = [...document.querySelectorAll('[data-dv-pick]')];
  assert.deepEqual(picks.map((p) => p.dataset.dvPick), ['demo-org/demo-web', 'demo-org/demo-api', 'demo-docs']);
  assert.equal(picks[0].getAttribute('aria-checked'), 'true');
  assert.equal(document.activeElement, picks[0], 'focus starts on the watched repo');
  assert.match(picks[1].textContent, /API’s repo link/);
  assert.match(picks[2].textContent, /Docs’s folder, no remote/);
  await click(picks[1]);
  await settle();
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , b]) => b), [{ on: true, repo: 'demo-org/demo-api' }]);
  assert.equal(document.querySelector('[data-dv-picker]'), null, 'the list closes');
  assert.equal(document.activeElement, sw, 'focus goes back to Switch');
  assert.match(document.querySelector('[data-dv-status-text]').textContent, /Watching demo-org\/demo-api on main · 1 of 3 repos found/);
  root.unmount();
});

test('DOM: tapping the watched repo just closes; Esc closes; a typed git URL is watched without its userinfo', async () => {
  const { root } = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: SOURCES });
  const sw = document.querySelector('[data-dv-status-action="switch"]');
  await click(sw);
  await click(document.querySelector('[data-dv-pick="demo-org/demo-web"]'));
  assert.equal(document.querySelector('[data-dv-picker]'), null);
  assert.equal(server.calls.filter(([meth]) => meth === 'PUT').length, 0);
  await click(sw);
  document.querySelector('[data-dv-picker]').dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(document.querySelector('[data-dv-picker]'), null);
  assert.equal(document.activeElement, sw);
  await click(sw);
  const input = document.querySelector('[data-dv-picker] input');
  const submit = document.querySelector('[data-dv-picker] button[type="submit"]');
  const type = async (v) => {
    Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, v);
    input.dispatchEvent(new window.Event('input', { bubbles: true }));
    await settle();
  };
  await type('not a repo');
  assert.equal(submit.disabled, true);
  assert.equal(input.getAttribute('aria-invalid'), 'true');
  await type('https://demo:not-a-real-token@gitea.example.test/demo/app.git');
  assert.equal(submit.disabled, false);
  input.form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , b]) => b), [{ on: true, repo: 'https://gitea.example.test/demo/app.git' }]);
  root.unmount();
});

test('DOM: a failed switch keeps the list open with the reason', async () => {
  const { root } = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: SOURCES });
  server.fail = 'Bots can’t read that repo.';
  await click(document.querySelector('[data-dv-status-action="switch"]'));
  await click(document.querySelector('[data-dv-pick="demo-org/demo-api"]'));
  await settle();
  assert.ok(document.querySelector('[data-dv-picker]'));
  assert.match(document.querySelector('[data-dv-picker]').textContent, /Bots can’t read that repo\./);
  root.unmount();
});

test('DOM: a typed repo the server can’t read (200, repo null, check) keeps the list open with why; Dev mode stays on', async () => {
  for (const [why, re] of [['not_found', /Couldn’t read demo-org\/nope\. .*Dev mode is on/], ['not_connected', /Connect GitHub so bots can read demo-org\/nope\./]]) {
    const { root } = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: SOURCES });
    server.unreadable = why;
    await click(document.querySelector('[data-dv-status-action="switch"]'));
    const input = document.querySelector('[data-dv-picker] input');
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    set.call(input, 'demo-org/nope');
    input.dispatchEvent(new window.Event('input', { bubbles: true }));
    await settle();
    input.form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    assert.deepEqual(server.calls.filter(([x]) => x === 'PUT').at(-1)[2], { on: true, repo: 'demo-org/nope' });
    assert.ok(document.querySelector('[data-dv-picker]'), why);
    assert.match(document.querySelector('[data-dv-picker]').textContent, re);
    assert.equal(m.devStore.get('spc_demo').on, true);
    assert.equal(m.devStore.get('spc_demo').repo, null);
    root.unmount();
  }
});

// ---------------------------------------------------------------------------
// The user flow (user-flow.md F3–F10)

const puts = () => server.calls.filter(([meth]) => meth === 'PUT').map(([, , b]) => b);
const noop = () => {};

const toast = async (over, props = {}) => {
  reset(over);
  const st = m.parseDevMode(server.state);
  m.devStore.set('spc_demo', st);
  return mount(h(m.DevOnToast, { state: st, onOpenDesk: noop, onConnect: noop, onTypeRepo: noop, onUndo: () => m.turnDevMode('spc_demo', false), onClose: noop, ...props }));
};

test('F3 DOM: the toast is role=status on the body, says what was found, and offers that status’s one action', async () => {
  for (const [over, title, body, action, label] of [
    [{ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }, 'Dev mode on', 'Watching demo-org/demo-web on main.', 'open', 'Open Dev desk'],
    [{ status: 'not_connected', repo: 'demo-org/demo-web', sources: [SOURCES[0]] }, 'Dev mode on · guards are up', 'Found demo-org/demo-web. Connect GitHub to get reviews.', 'connect', 'Connect GitHub'],
    [{ status: 'other_host', repo: 'https://gitlab.example.test/demo/app', sources: [{ team: 'Web', repo: 'https://gitlab.example.test/demo/app', host: 'gitlab.example.test', from: 'folder' }] }, 'Dev mode on · guards are up', 'gitlab.example.test repos are read from the next update. The floor works now.', 'open', 'Open Dev desk'],
    [{ status: 'no_repo', sources: [] }, 'Dev mode on · guards are up', 'No git repo in this space’s folders yet. The floor works now.', 'type', 'Type a repo'],
  ]) {
    let ran = '';
    const { root } = await toast(over, { onOpenDesk: () => (ran = 'open'), onConnect: () => (ran = 'connect'), onTypeRepo: () => (ran = 'type') });
    const el = document.querySelector('[data-dev-on-toast] [role="status"]');
    assert.ok(el, over.status);
    assert.equal(el.closest('[data-dev-on-toast]').parentElement, document.body, 'portalled to the body');
    assert.equal(el.querySelector('[data-dev-on-toast-title] strong').textContent, title);
    assert.match(el.querySelector('[data-dev-on-toast-title]').textContent, /^HEAD → dev/);
    assert.equal(el.querySelector('[data-dev-on-toast-text]').textContent, body);
    assert.equal(document.querySelectorAll('[data-dev-on-toast-action]').length, 1, 'one fix action');
    const btn = document.querySelector('[data-dev-on-toast-action]');
    assert.equal(btn.dataset.devOnToastAction, action);
    assert.equal(btn.textContent.trim(), label);
    assert.ok(document.querySelector('[data-dev-on-toast-undo]'), 'Undo is always there');
    assert.ok(el.querySelector('button[aria-label="Dismiss"]'));
    await click(btn);
    assert.equal(ran, action);
    root.unmount();
  }
});

test('F3 DOM: a "checking" toast updates in place when the status arrives', async () => {
  const { root } = await toast({});
  assert.equal(document.querySelector('[data-dev-on-toast]').dataset.devOnToast, 'checking');
  assert.match(document.querySelector('[data-dev-on-toast-text]').textContent, /Looking for a git repo/);
  root.render(h(m.DevOnToast, { state: state({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }), onOpenDesk: noop, onTypeRepo: noop, onUndo: async () => {}, onClose: noop }));
  await settle();
  assert.equal(document.querySelectorAll('[data-dev-on-toast]').length, 1, 'the same toast');
  assert.equal(document.querySelector('[data-dev-on-toast-text]').textContent, 'Watching demo-org/demo-web on main.');
  root.unmount();
});

test('F3 DOM: Undo turns Dev mode off in one press (PUT {on:false}) and closes; a failed Undo says it is still on', async () => {
  let closed = 0;
  const { root } = await toast({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] }, { onClose: () => closed++ });
  await click(document.querySelector('[data-dev-on-toast-undo]'));
  await settle();
  assert.deepEqual(puts(), [{ on: false }]);
  assert.equal(m.devStore.isOn('spc_demo'), false);
  assert.equal(closed, 1);
  root.unmount();
  const again = await toast({ status: 'no_repo', sources: [] }, { onClose: () => closed++ });
  server.fail = 'Offline.';
  await click(document.querySelector('[data-dev-on-toast-undo]'));
  await settle();
  assert.match(document.querySelector('[data-dev-on-toast-text]').textContent, /Couldn’t turn it off\. It’s still on\./);
  again.root.unmount();
});

test('F3 DOM: the toast goes after its time, paused while focus is inside; on a phone it sits above the chat box', async () => {
  reset({ status: 'no_repo', sources: [] });
  const form = document.createElement('form');
  form.setAttribute('data-composer-form', '');
  form.getBoundingClientRect = () => ({ top: 700, bottom: 800, height: 100, left: 0, right: 390, width: 390 });
  document.body.appendChild(form);
  Object.defineProperty(window, 'innerHeight', { value: 844, configurable: true });
  let closed = 0;
  const st = m.parseDevMode(server.state);
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.DevOnToast, { state: st, phone: true, onOpenDesk: noop, onTypeRepo: noop, onUndo: async () => {}, onClose: () => closed++, timeout: 40 }));
  await settle();
  const wrap = document.querySelector('[data-dev-on-toast]');
  assert.equal(wrap.style.bottom, '152px', '844 − 700 + 8: above the chat box, never over Send');
  const undo = wrap.querySelector('[data-dev-on-toast-undo]');
  undo.focus();
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(closed, 0, 'held while focus is inside');
  undo.blur();
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(closed, 1, 'closes once its time is up');
  root.unmount();
  form.remove();
});

test('F8 DOM: the off note’s Turn back on turns Dev mode on in one press and closes the note', async () => {
  reset({ on: false });
  let dismissed = 0;
  const { root } = await mount(h(m.DevOffNote, { approvals: 'ask', onDismiss: () => dismissed++, onTurnBackOn: () => m.turnDevMode('spc_demo', true) }));
  const b = document.querySelector('[data-dev-turn-back-on]');
  assert.equal(b.textContent.trim(), 'Turn back on');
  assert.ok(b.className.includes('min-h-[44px]'), '44 px on phones');
  await click(b);
  await settle();
  assert.deepEqual(puts(), [{ on: true }]);
  assert.equal(m.devStore.isOn('spc_demo'), true);
  assert.equal(dismissed, 1);
  root.unmount();
});

test('F10 DOM: a space already on shows no toast: onTurnedOn only follows a press that turned it on', async () => {
  reset({ status: 'watching', repo: 'demo-org/typed-by-hand', branch: 'main', sources: [SOURCES[0]] });
  let turnedOn = 0;
  const { root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' }, onTurnedOn: () => turnedOn++ }));
  await settle();
  assert.equal(document.querySelector('[data-dev-switch]').getAttribute('aria-checked'), 'true');
  assert.equal(turnedOn, 0);
  assert.equal(puts().length, 0, 'nothing is saved, so the typed repo stays');
  root.unmount();
});

test('F6: the row badge is new_reviews from the server (landed cards never count); an older server falls back to open reviews', () => {
  assert.equal(m.rowBadge(state({ new_reviews: 2 }), 5), 2);
  assert.equal(m.rowBadge(state({ new_reviews: 0 }), 5), 0);
  assert.equal(m.rowBadge(state({}), 3), 3);
  assert.equal(m.parseDevMode({ ...base, new_reviews: -4 }).new_reviews, 0);
});

test('F6 DOM: the row shows new_reviews; opening the desk posts /dev_mode/seen and clears it', async () => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]], new_reviews: 2, polled_at: '2026-11-05T09:00:00Z' });
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const row = await mount(h(m.DevDeskRow, { spaceId: 'spc_demo', active: false, onOpen: noop }));
  await settle();
  assert.equal(document.querySelector('[data-dev-row-badge]').textContent, '2');
  assert.equal(document.querySelector('[data-dev-desk-row]').getAttribute('aria-label'), 'Dev desk, 2 new reviews');
  assert.equal(document.querySelector('[data-dev-row-badge]').getAttribute('title'), '2 new reviews to look at');
  const view = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await settle();
  assert.ok(server.calls.some(([meth, u]) => meth === 'POST' && u.endsWith('/dev_mode/seen')));
  assert.equal(document.querySelector('[data-dev-row-badge]'), null, 'the badge clears at once');
  view.root.unmount();
  row.root.unmount();
});

test('F4 DOM: the Dev desk row gets one soft highlight the first time it appears, then never again', async () => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', sources: [SOURCES[0]] });
  const a = await mount(h(m.DevDeskRow, { spaceId: 'spc_demo', active: false, onOpen: noop }));
  assert.ok(document.querySelector('[data-dev-desk-row]').classList.contains('kw-dv-row-fresh'));
  a.root.unmount();
  document.body.innerHTML = '';
  const b = await mount(h(m.DevDeskRow, { spaceId: 'spc_demo', active: false, onOpen: noop }));
  assert.equal(document.querySelector('[data-dev-desk-row]').classList.contains('kw-dv-row-fresh'), false);
  b.root.unmount();
});

test('F4 DOM: while the first poll runs, still skeleton cards with "Reading the last 5 merges…", never "Nothing has landed yet"', async () => {
  assert.equal(m.firstPolling(state({ status: 'watching', repo: 'demo-org/demo-web' }), 0), true);
  assert.equal(m.firstPolling(state({ status: 'watching', repo: 'demo-org/demo-web', polled_at: '2026-11-05T09:00:00Z' }), 0), false);
  assert.equal(m.firstPolling(state({ status: 'no_repo' }), 0), false);
  const { root } = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] });
  const sk = document.querySelector('[data-dev-skeleton]');
  assert.ok(sk);
  assert.equal(sk.querySelector('[role="status"]').textContent.trim(), 'Reading the last 5 merges…');
  assert.equal(document.querySelector('[data-dev-empty]'), null);
  assert.equal(document.querySelector('.animate-pulse'), null, 'no shimmer');
  root.unmount();
  const polled = await desk({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]], polled_at: '2026-11-05T09:00:00Z' });
  assert.equal(document.querySelector('[data-dev-skeleton]'), null);
  assert.ok(document.querySelector('[data-dev-empty]'));
  polled.root.unmount();
});

test('F4 DOM: the first-run block shows on the first visit only, and Got it hides it for good', async () => {
  const one = await desk({ status: 'no_repo', sources: [] });
  const intro = document.querySelector('[data-dev-intro]');
  assert.ok(intro);
  assert.equal(intro.querySelector('h3').textContent, 'Here’s what Dev mode does');
  assert.match(intro.textContent, /HEAD → dev · guards up/);
  assert.equal(document.querySelectorAll('[data-dev-feature]').length, 3, 'three lines');
  assert.equal(intro.querySelectorAll('button').length, 1, 'Got it is the only control');
  const got = document.querySelector('[data-dev-intro-close]');
  assert.equal(got.textContent.trim(), 'Got it');
  await click(got);
  assert.equal(document.querySelector('[data-dev-intro]'), null);
  one.root.unmount();
  document.body.innerHTML = '';
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const two = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  assert.equal(document.querySelector('[data-dev-intro]'), null, 'second visit: gone');
  two.root.unmount();
});

test('PX-3 (style-v2 §9): later visits say "Nothing new on main" and "No open changes to review"; the first run reads $ kwant dev --on', async () => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]], polled_at: '2026-11-05T09:00:00Z' });
  localStorage.setItem('kwnt.dev.intro.spc_demo', '1');
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const later = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  assert.equal(document.querySelector('[data-dev-intro]'), null);
  const e = document.querySelector('[data-dev-empty-term]');
  assert.ok(e.textContent.includes('$ git log main --since="your last look"'));
  assert.ok(e.textContent.includes('(nothing new)'));
  assert.ok(e.textContent.includes('Nothing new on main'));
  assert.doesNotMatch(document.body.textContent, /No merges on main yet|Nothing has landed yet/);
  await click(document.querySelector('[data-dev-tab="reviews"]'));
  const r = document.querySelector('[data-dev-empty-term]');
  assert.ok(r.textContent.includes('$ kwant reviews --open'));
  assert.ok(r.textContent.includes('0 open changes'));
  assert.ok(r.textContent.includes('No open changes to review'));
  later.root.unmount();
  const first = await desk({ status: 'no_repo', sources: [] });
  assert.match(document.querySelector('[data-dev-intro]').textContent, /\$ kwant dev --on/);
  first.root.unmount();
});

test('F4 DOM: first-visit empty states speak git and never say "Nothing has landed yet"; 9+ caps the badge', async () => {
  for (const [over, out, title] of [
    [{ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]], polled_at: '2026-11-05T09:00:00Z' }, '(no merges yet)', 'No merges on main yet.'],
    [{ status: 'not_connected', repo: 'demo-org/demo-web', sources: [SOURCES[0]] }, '(needs a GitHub connection)', 'Connect GitHub to see what landed.'],
    [{ status: 'no_repo', sources: [] }, '(no repo yet)', 'Cards show up once there’s a repo.'],
  ]) {
    const { root } = await desk(over);
    const e = document.querySelector('[data-dev-empty-term]');
    assert.ok(e, over.status);
    assert.match(e.textContent, /\$ git log main -5/);
    assert.ok(e.textContent.includes(out), over.status);
    assert.ok(e.textContent.includes(title), over.status);
    assert.doesNotMatch(document.body.textContent, /Nothing has landed yet/);
    assert.equal(document.querySelector('[data-dev-rollup]'), null, 'no "since you last looked" line with no cards');
    root.unmount();
  }
  reset({ status: 'watching', repo: 'demo-org/demo-web', sources: [SOURCES[0]], new_reviews: 12 });
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const row = await mount(h(m.DevDeskRow, { spaceId: 'spc_demo', active: false, onOpen: noop }));
  assert.equal(document.querySelector('[data-dev-row-badge]').textContent, '9+');
  assert.equal(document.querySelector('[data-dev-desk-row]').getAttribute('aria-label'), 'Dev desk, 12 new reviews');
  row.root.unmount();
});

test('F5 DOM: no_repo’s Type a repo opens the list straight at the field; the toast’s Type a repo lands there too', async () => {
  const { root } = await desk({ status: 'no_repo', sources: [] });
  const t = document.querySelector('[data-dv-status-action="type"]');
  assert.equal(t.textContent.trim(), 'Type a repo');
  await click(t);
  const input = document.querySelector('[data-dv-picker] input');
  assert.ok(input);
  assert.equal(document.activeElement, input);
  assert.equal(document.querySelectorAll('[data-dv-pick]').length, 0);
  root.unmount();
  reset({ status: 'no_repo', sources: [] });
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  m.deskIntent.set('spc_demo', 'type');
  const again = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  assert.ok(document.querySelector('[data-dv-picker] input'), 'opened from the toast');
  assert.equal(m.deskIntent.take('spc_demo'), null, 'used once');
  again.root.unmount();
});

test('F5 DOM: Connect opens Loose ends’ add form on GitHub, and a GitHub source added says so (to return to the desk)', async () => {
  reset({});
  let added = null;
  const { root } = await mount(h(m.LooseSources, { sources: [], error: null, onChanged: noop, initialAdding: true, initialKind: 'github', onAdded: (s) => (added = s) }));
  const gh = [...document.querySelectorAll('[role="radio"]')].find((r) => /GitHub/.test(r.textContent));
  assert.equal(gh.getAttribute('aria-checked'), 'true');
  root.unmount();
  m.devConnect.start('spc_demo');
  assert.equal(m.devConnect.pending(), 'spc_demo');
  assert.equal(m.devConnect.finish(), 'spc_demo');
  assert.equal(m.devConnect.finish(), null, 'returns once');
  assert.equal(added, null);
});

test('F3 DOM: on desktop the toast starts 16 px inside the main pane, never over the sidebar', async () => {
  const over = { status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] };
  reset(over);
  const pane = document.createElement('div');
  pane.setAttribute('data-main-pane', '');
  pane.getBoundingClientRect = () => ({ left: 323, top: 0, right: 1440, bottom: 900, width: 1117, height: 900 });
  document.body.appendChild(pane);
  const st = m.parseDevMode(server.state);
  const { root } = await mount(h(m.DevOnToast, { state: st, onOpenDesk: noop, onTypeRepo: noop, onUndo: async () => {}, onClose: noop }));
  assert.equal(document.querySelector('[data-dev-on-toast]').style.left, '339px');
  root.unmount();
  const again = await toast({ status: 'no_repo', sources: [] });
  assert.equal(document.querySelector('[data-dev-on-toast]').style.left, '16px', 'no pane: 16 px');
  again.root.unmount();
});

// ---------------------------------------------------------------------------
// Round 3 (Rigel's round 2: B2, B3, K1 and the minors)

test('B2: saveGate reads the store: this server under approvals off is "store", a locked Bitwarden is "locked", else open', () => {
  assert.equal(m.saveGate(null), 'open');
  assert.equal(m.saveGate({ store: 'local', approvals_off: false, locked: false }), 'open');
  assert.equal(m.saveGate({ store: 'local', approvals_off: true, locked: false }), 'store');
  assert.equal(m.saveGate({ store: 'bitwarden', approvals_off: true, locked: true }), 'locked');
  assert.equal(m.saveGate({ store: 'bitwarden', approvals_off: true, locked: false }), 'open');
});

const frame = () => new Promise((r) => setTimeout(r, 40));
const connectForm = async (store, spaces) => {
  reset({});
  server.store = store;
  server.spaces = spaces;
  const r = await mount(h(m.LooseSources, { sources: [], error: null, onChanged: noop, initialAdding: true, initialKind: 'github', onAdded: noop, settled: true }));
  await frame();
  await settle();
  return r;
};

test('B2 DOM: under "Ask only when risky" Connect offers the two ways out before any token field, and nothing is sent', async () => {
  const { root } = await connectForm({ store: 'local', local_count: 0, approvals_off: true }, [{ id: 'spc_demo', approvals: 'smart' }]);
  const gate = document.querySelector('[data-save-first]');
  assert.ok(gate, 'the ways out show');
  assert.equal(gate.dataset.saveFirst, 'store');
  assert.equal(document.querySelector('input[type="password"]'), null, 'no token field to type into yet');
  assert.match(gate.textContent, /One step before the token/);
  assert.equal(gate.querySelector('[data-save-first-bitwarden]').textContent.trim(), 'Connect Bitwarden');
  assert.equal(gate.querySelector('[data-save-first-ask]').textContent.trim(), 'Make every bot ask first');
  assert.equal(gate.querySelector('[data-save-first-why]').textContent, 'Some bots here run commands without asking, so they could read a token saved on this server. Keep it in Bitwarden, or make every bot ask first.', 'M2: its own reason line');
  assert.equal(gate.querySelector('[data-save-first-scope]').textContent, 'Turns on “Ask before running commands” in every space. You can change it back in each space’s settings.', 'M1: the scope is said');
  assert.match(gate.querySelector('[data-save-first-scope]').className, /text-\[11px\] text-amber-100\/70/);
  for (const b of gate.querySelectorAll('[data-save-first-bitwarden], [data-save-first-ask]')) assert.match(b.className, /w-full sm:w-auto/, 'S2: equal width when stacked');
  assert.match(gate.className, /border-amber-500\/40 bg-amber-500\/10/, 'amber while it needs you');
  assert.ok(document.activeElement === gate.querySelector('[data-save-first-bitwarden]'), 'B3: focus lands on the first way out');
  assert.equal(document.querySelector('button[type="submit"]').disabled, true);
  assert.equal(server.calls.filter(([meth, u]) => meth === 'POST' && u.includes('/loose')).length, 0);
  root.unmount();
});

test('B2 DOM: "Ask before running commands" switches every space and bot to ask, then the GitHub form is back with the token focused', async () => {
  const { root } = await connectForm({ store: 'local', local_count: 0, approvals_off: true }, [{ id: 'spc_demo', approvals: 'smart' }]);
  await click(document.querySelector('[data-save-first-ask]'));
  await settle(20);
  await frame();
  assert.deepEqual(puts(), [{ approvals: 'ask' }]);
  assert.equal(document.querySelector('[data-save-first]'), null);
  const tok = document.querySelector('input[type="password"]');
  assert.ok(tok, 'the token field is back');
  assert.ok(document.activeElement === tok, 'and focused');
  assert.equal(document.querySelector('[data-save-ready]').textContent, 'Every bot now asks before running commands. Add the token below.');
  root.unmount();
});

test('B2 DOM: Connect Bitwarden opens the existing Bitwarden form in place; a locked Bitwarden offers Unlock only', async () => {
  const one = await connectForm({ store: 'local', local_count: 0, approvals_off: true }, []);
  await click(document.querySelector('[data-save-first-bitwarden]'));
  assert.equal(document.querySelector('[data-save-first-bitwarden]'), null, 'the Bitwarden form replaces the buttons');
  const open = document.querySelector('[data-save-first]');
  assert.match(open.className, /border-white\/10 bg-white\/\[0\.03\]/, 'S1: the open form is not on amber');
  assert.doesNotMatch(open.className, /bg-amber-500/);
  assert.match(open.querySelector('[data-bitwarden-form] > p').className, /text-xs font-medium text-zinc-100/, 'S1: a plain heading, not a link look');
  assert.equal(open.querySelector('[data-save-first-scope]'), null, 'the scope line goes with its button');
  assert.ok(document.querySelector('[data-save-first] form, [data-save-first] input'), 'Bitwarden sign-in fields show');
  await frame();
  assert.ok(['INPUT', 'SELECT'].includes(document.activeElement?.tagName) && document.activeElement.closest('[data-save-first]'), 'focus goes to its first field: ' + document.activeElement?.outerHTML?.slice(0, 120));
  one.root.unmount();
  const two = await connectForm({ store: 'bitwarden', local_count: 0, approvals_off: true, bitwarden: { server_url: 'https://vault.bitwarden.com', email: 'demo@example.com', login: 'password', unlocked: false } }, []);
  const gate = document.querySelector('[data-save-first]');
  assert.equal(gate.dataset.saveFirst, 'locked');
  assert.ok(document.activeElement === gate.querySelector('[data-save-first-bitwarden]'), 'Unlock is focused');
  assert.equal(gate.querySelector('[data-save-first-bitwarden]').textContent.trim(), 'Unlock Bitwarden');
  assert.equal(gate.querySelector('[data-save-first-ask]'), null);
  two.root.unmount();
});

test('B3 DOM: Connect lands on the GitHub form once the page has loaded: scrolled into view, token focused; not before', async () => {
  reset({});
  const scrolled = [];
  const proto = window.HTMLElement.prototype;
  const had = proto.scrollIntoView;
  proto.scrollIntoView = function (o) {
    scrolled.push([this.dataset.addSource, o?.block]);
  };
  const props = { sources: [], error: null, onChanged: noop, initialAdding: true, initialKind: 'github', onAdded: noop };
  const { root } = await mount(h(m.LooseSources, { ...props, settled: false }));
  assert.deepEqual(scrolled, [], 'nothing moves while the cases above are still loading');
  assert.ok(document.activeElement?.type !== 'password');
  root.render(h(m.LooseSources, { ...props, settled: true }));
  await settle(20);
  await frame();
  assert.deepEqual(scrolled, [['github', 'start']]);
  assert.ok(document.activeElement === document.querySelector('input[type="password"]'), 'focus: document.querySelector(\'input[type="password"]\')');
  root.unmount();
  proto.scrollIntoView = had;
});

test('B2/B3: Loose ends opened on its own (not from Connect) checks nothing and moves nothing', async () => {
  reset({});
  server.store = { store: 'local', local_count: 0, approvals_off: true };
  const { root } = await mount(h(m.LooseSources, { sources: [], error: null, onChanged: noop, initialAdding: true, onAdded: noop }));
  assert.equal(server.calls.filter(([, u]) => u === '/api/passwords/store').length, 0);
  assert.equal(document.querySelector('[data-save-first]'), null);
  root.unmount();
});

const withSwitch = async (props = {}) => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] });
  const page = document.createElement('div');
  page.innerHTML = '<button data-dev-switch="on" id="sw">dev</button><a href="#x" id="after">Dev desk</a><button id="later">Later</button>';
  document.body.appendChild(page);
  const st = m.parseDevMode(server.state);
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.DevOnToast, { state: st, onOpenDesk: noop, onTypeRepo: noop, onUndo: async () => {}, onClose: noop, ...props }));
  await settle();
  return { root, sw: document.getElementById('sw'), after: document.getElementById('after') };
};
const tab = (el, shift = false) => {
  const e = new window.KeyboardEvent('keydown', { key: 'Tab', shiftKey: shift, bubbles: true, cancelable: true });
  el.dispatchEvent(e);
  return e.defaultPrevented;
};

test('K1 DOM: Tab from the switch goes straight to the toast; Shift+Tab goes back; Tab from Dismiss carries on after the switch', async () => {
  const { root, sw, after } = await withSwitch();
  sw.focus();
  assert.ok(document.activeElement === sw, 'nothing steals focus when the toast opens');
  assert.equal(tab(sw), true);
  const action = document.querySelector('[data-dev-on-toast-action]');
  assert.ok(document.activeElement === action, 'one Tab: Open Dev desk');
  assert.equal(action.textContent.trim(), 'Open Dev desk');
  assert.equal(tab(action, true), true);
  assert.ok(document.activeElement === sw, 'Shift+Tab: back to the switch');
  const dismiss = document.querySelector('[data-dev-on-toast] [aria-label="Dismiss"]');
  dismiss.focus();
  assert.equal(tab(dismiss), true);
  assert.ok(document.activeElement === after, 'the next stop after the switch, as if the toast sat right after it');
  const undo = document.querySelector('[data-dev-on-toast-undo]');
  undo.focus();
  assert.equal(tab(undo), false, 'Undo → Dismiss is the normal order');
  root.unmount();
});

test('K1 DOM: the toast stays while keyboard focus or the pointer is on the switch, and goes after its time once they leave', async () => {
  let closed = 0;
  const { root, sw } = await withSwitch({ onClose: () => closed++, timeout: 40 });
  sw.focus();
  sw.dispatchEvent(new window.FocusEvent('focus'));
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(closed, 0, 'held while focus is on the switch');
  sw.blur();
  sw.dispatchEvent(new window.Event('pointerenter'));
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(closed, 0, 'held while the pointer is over the switch');
  sw.dispatchEvent(new window.Event('pointerleave'));
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(closed, 1);
  root.unmount();
});

test('K1 DOM: Dismiss from the keyboard puts focus back on the switch', async () => {
  let closed = 0;
  const { root, sw } = await withSwitch({ onClose: () => closed++ });
  const dismiss = document.querySelector('[data-dev-on-toast] [aria-label="Dismiss"]');
  dismiss.focus();
  await click(dismiss);
  assert.equal(closed, 1);
  assert.ok(document.activeElement === sw, 'focus: sw');
  root.unmount();
});

test('minor: the Dev desk row reads "Dev desk, 0 landed…" (a separator for screen readers), never "Dev desk0 landed"', async () => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] });
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
  const { root } = await mount(h(m.DevDeskRow, { spaceId: 'spc_demo', active: false, onOpen: noop }));
  await settle(20);
  const row = document.querySelector('[data-dev-desk-row]');
  assert.doesNotMatch(row.textContent, /Dev desk\d/);
  assert.match(row.textContent, /^Dev desk, /);
  assert.ok(row.querySelector('.sr-only'), 'the comma is for screen readers only');
  root.unmount();
});

test('minor: the switch starts from the last value seen here, so an on space never shows "off" while it loads', async () => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] });
  m.lastOn.set('spc_demo', true);
  let release;
  assert.equal(m.lastOn.get('spc_never'), null, 'never seen here: no guess');
  const realFetch = g.fetch;
  g.fetch = g.window.fetch = (url, init) => (String(url).includes('/dev_mode') ? new Promise((r) => (release = () => r(realFetch(url, init)))) : realFetch(url, init));
  const { root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  assert.equal(document.querySelector('[data-dev-switch]').getAttribute('aria-checked'), 'true', 'on from the first frame');
  release();
  await settle(20);
  assert.equal(document.querySelector('[data-dev-switch]').getAttribute('aria-checked'), 'true');
  g.fetch = g.window.fetch = realFetch;
  root.unmount();
  reset({ on: false });
  const off = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  await settle(20);
  assert.equal(m.lastOn.get('spc_demo'), false, 'turned off elsewhere: remembered as off');
  off.root.unmount();
});

test('F8-R3-1: a space never seen in this browser shows the switch as loading (neither on nor off) until dev_mode answers', async () => {
  reset({ status: 'watching', repo: 'demo-org/demo-web', branch: 'main', sources: [SOURCES[0]] });
  localStorage.removeItem(m.lastOn.key('spc_demo'));
  let release;
  const realFetch = g.fetch;
  g.fetch = g.window.fetch = (url, init) => (String(url).includes('/dev_mode') ? new Promise((r) => (release = () => r(realFetch(url, init)))) : realFetch(url, init));
  const { root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const sw = () => document.querySelector('[role="switch"]');
  assert.equal(sw().dataset.devSwitch, 'loading');
  assert.equal(sw().getAttribute('aria-busy'), 'true');
  assert.equal(sw().disabled, true, 'not pressable before its state is known');
  assert.equal(sw().hasAttribute('data-on'), false);
  assert.equal(sw().querySelector('.kw-dv-node').getAttribute('visibility'), 'hidden', 'no node on either side');
  release();
  await settle(20);
  assert.notEqual(sw().dataset.devSwitch, 'loading');
  assert.equal(sw().getAttribute('aria-checked'), 'true');
  assert.equal(sw().hasAttribute('aria-busy'), false);
  assert.equal(sw().disabled, false);
  assert.equal(sw().querySelector('.kw-dv-node').hasAttribute('visibility'), false);
  assert.equal(m.lastOn.get('spc_demo'), true, 'the next load starts on');
  g.fetch = g.window.fetch = realFetch;
  root.unmount();
});
