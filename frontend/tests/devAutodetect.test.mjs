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
  export { LooseSources } from './src/components/LooseEndsView';
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
