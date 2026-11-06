// 0.7.3 github-connect (Pixel): A3 the warning card and its one-time Save, B1 one GitHub form opened
// in place (popover on desktop, sheet on phone), B2 Team settings' repo line, B3 the helper text,
// C5 the other-cloud hint under a wrong master password, C6 the gate re-checked on focus.
// Demo data only: no real repos, tokens or emails, and nothing leaves this process.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLSelectElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
let phone = false;
window.matchMedia = (q) => ({ matches: q.includes('max-width: 767px') ? phone : false, media: q, addEventListener() {}, removeEventListener() {} });

let server;
const OLD_SPACE = { id: 'spc_old', name: 'Old', icon: 'box', color: null, archived_at: '2026-10-01T00:00:00Z', counts: { bots: 2 } };
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  const path = u.split('?')[0];
  server.calls.push([method, path, body]);
  if (path === '/api/passwords/store') return json(server.store);
  if (path === '/api/passwords/bitwarden/unlock') return json({ error: 'Wrong master password.', wrong_password: true }, 401);
  if (path === '/api/zombie/sources' && method === 'POST') {
    if (server.refuse && !body.accept_risk) return json({ error: 'Some bots here run commands without asking.', needs_bitwarden: true, can_accept: true }, 409);
    return json({ source: { id: 'src_demo', kind: 'github', label: body.label || 'GitHub', enabled: true }, test: null });
  }
  if (/\/dev_mode$/.test(path)) return json(server.dev ?? { on: false, repos: [] });
  if (path === '/api/spaces' && method === 'GET') return json([OLD_SPACE]);
  if (path === `/api/spaces/${OLD_SPACE.id}` && method === 'PATCH') {
    if (!body.accept_risk) return json({ error: 'Passwords are kept on this server and bots in this space run commands without asking.', needs_bitwarden: true, can_accept: true, local_count: 2 }, 409);
    return json({ ...OLD_SPACE, archived_at: null });
  }
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { saveGate, SAVE_FIRST, otherCloudHint, parseStoreStatus } from './src/passwordStore';
  export { GitHubConnectForm, NeedsBitwarden, GITHUB_ALSO_DEV, LooseSources } from './src/components/LooseEndsView';
  export { GitHubConnectHost, openGitHubConnect, popoverPlace, GITHUB_CONNECT_EVENT, GITHUB_CONNECTED_EVENT } from './src/components/GitHubConnect';
  export { TeamRepoDevLine, teamRepoLine, repoKeyOf, TEAM_REPO_LINES } from './src/components/TeamRepoDevLine';
  export { BitwardenLoginForm, MoveToBitwardenFirst } from './src/components/PasswordStorePanel';
  export { AddSourceForm } from './src/components/LooseEndsView';
  export { ArchivedSpacesModal } from './src/components/Spaces';
  export { acceptRiskText } from './src/passwordStore';
  export { parseDevMode, devStore } from './src/devMode';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const frame = () => new Promise((r) => setTimeout(r, 40));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const typeIn = (el, value) => {
  const proto = el.tagName === 'SELECT' ? window.HTMLSelectElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event(el.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root };
};
const noop = () => {};
const STORE_OFF = { store: 'local', locked: false, local_count: 0, bitwarden: null, approvals_off: true, exposed: false, risk_accepted: false };
const reset = (over = {}) => {
  server = { store: STORE_OFF, calls: [], refuse: false, ...over };
  m.devStore._reset();
  localStorage.clear();
  document.body.innerHTML = '';
  phone = false;
};
const posts = () => server.calls.filter(([meth, p]) => meth === 'POST' && p === '/api/zombie/sources').map(([, , b]) => b);

// ---------------------------------------------------------------- A3: the card

test('A3: the gate reads the store; once the risk is accepted the token field opens straight away', () => {
  const st = (o) => m.parseStoreStatus({ ...STORE_OFF, ...o });
  assert.equal(m.saveGate(st({})), 'store');
  assert.equal(m.saveGate(st({ risk_accepted: true })), 'open', 'accepted once for this server');
  assert.equal(m.saveGate(st({ approvals_off: false })), 'open');
  assert.equal(m.saveGate(st({ store: 'bitwarden', locked: true, bitwarden: { server_url: 'https://vault.bitwarden.com', email: 'demo@example.com', login: 'password', unlocked: false } })), 'locked');
  assert.equal(m.parseStoreStatus({ ...STORE_OFF, risk_accepted: true }).risk_accepted, true);
  assert.equal(m.parseStoreStatus({ store: 'local' }).risk_accepted, false, 'an older server: not accepted');
});

test('A3 DOM: Save it on this server opens the token field, and the save carries accept_risk once', async () => {
  reset();
  const added = [];
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: (s) => added.push(s), onCancel: noop, land: true, scroll: false }));
  await frame();
  const gate = document.querySelector('[data-save-first]');
  assert.equal(gate?.dataset.saveFirst, 'store');
  assert.match(gate.textContent, /^Before you save the token/);
  assert.equal(document.querySelector('[data-github-token]'), null);
  assert.equal(document.activeElement, gate.querySelector('[data-save-first-save]'), 'Save it on this server is focused');
  await click(gate.querySelector('[data-save-first-save]'));
  await frame();
  const tok = document.querySelector('[data-github-token]');
  assert.ok(tok, 'the token field shows');
  assert.equal(document.activeElement, tok);
  tok.value = 'demo-not-a-token';
  document.querySelector('form[data-github-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(20);
  assert.equal(posts().length, 1);
  assert.equal(posts()[0].accept_risk, true, 'the person pressed Save: sent with the save');
  assert.equal(added.length, 1);
  assert.equal(tok.value, '', 'the token leaves the field once saved');
  root.unmount();
});

test('A3 DOM: with nothing to warn about, no card and no accept_risk', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false } });
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: noop, onCancel: noop }));
  await frame();
  assert.equal(document.querySelector('[data-save-first]'), null);
  document.querySelector('[data-github-token]').value = 'demo-not-a-token';
  document.querySelector('form[data-github-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(20);
  assert.equal(posts()[0].accept_risk, undefined);
  root.unmount();
});

test('A3 DOM: a 409 with can_accept offers Save it on this server first; pressing it retries with accept_risk', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false }, refuse: true });
  const added = [];
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: (s) => added.push(s), onCancel: noop }));
  await frame();
  document.querySelector('[data-github-token]').value = 'demo-not-a-token';
  document.querySelector('form[data-github-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(20);
  const card = document.querySelector('[data-needs-bitwarden]');
  assert.ok(card, 'the refusal card shows');
  const save = card.querySelector('[data-needs-bitwarden-save]');
  assert.equal(save.textContent.trim(), 'Save it on this server');
  assert.match(save.className, /bg-sky-500/, 'Save is the primary');
  // The token stays in the field until it's saved.
  await click(save);
  await settle(20);
  assert.deepEqual(posts().map((b) => !!b.accept_risk), [false, true]);
  assert.equal(added.length, 1);
  root.unmount();
});

test('A3: the NeedsBitwarden card without can_accept keeps the old ways only', () => {
  const html = m.renderToStaticMarkup(h(m.NeedsBitwarden, { message: 'x', locked: false, onReady: noop, onCancel: noop }));
  assert.doesNotMatch(html, /data-needs-bitwarden-save/);
  const locked = m.renderToStaticMarkup(h(m.NeedsBitwarden, { message: 'x', locked: true, canAccept: true, onReady: noop, onCancel: noop }));
  assert.doesNotMatch(locked, /data-needs-bitwarden-save/, 'a locked Bitwarden is unlocked, not bypassed');
});

// ---------------------------------------------------------------- C6: re-check on focus

test('C6 DOM: Bitwarden connected elsewhere unblocks the card when the window gets focus again', async () => {
  reset();
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: noop, onCancel: noop }));
  await frame();
  assert.ok(document.querySelector('[data-save-first]'));
  const peeks = () => server.calls.filter(([, p]) => p === '/api/passwords/store').length;
  const before = peeks();
  server.store = { ...STORE_OFF, store: 'bitwarden', local_count: 0, bitwarden: { server_url: 'https://vault.bitwarden.com', email: 'demo@example.com', login: 'password', unlocked: true } };
  window.dispatchEvent(new window.Event('focus'));
  await settle(20);
  assert.equal(peeks(), before + 1, 'read again on focus');
  assert.equal(document.querySelector('[data-save-first]'), null, 'the card goes');
  assert.ok(document.querySelector('[data-github-token]'), 'the token field is there');
  assert.match(document.querySelector('[data-save-ready]').textContent, /Bitwarden/);
  root.unmount();
});

test('C6 DOM: approvals turned back on in another tab unblocks it when this tab is visible again; hidden does nothing', async () => {
  reset();
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: noop, onCancel: noop }));
  await frame();
  const peeks = () => server.calls.filter(([, p]) => p === '/api/passwords/store').length;
  const before = peeks();
  Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
  document.dispatchEvent(new window.Event('visibilitychange'));
  await settle();
  assert.equal(peeks(), before, 'not while hidden');
  server.store = { ...STORE_OFF, approvals_off: false };
  Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
  document.dispatchEvent(new window.Event('visibilitychange'));
  await settle(20);
  assert.equal(peeks(), before + 1);
  assert.equal(document.querySelector('[data-save-first]'), null);
  assert.equal(document.querySelector('[data-save-ready]').textContent, m.SAVE_FIRST.asked);
  root.unmount();
});

test('C6 DOM: still blocked after the re-check keeps the card; once open, focus reads nothing', async () => {
  reset();
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: noop, onCancel: noop }));
  await frame();
  window.dispatchEvent(new window.Event('focus'));
  await settle(20);
  assert.ok(document.querySelector('[data-save-first]'), 'nothing changed, the card stays');
  await click(document.querySelector('[data-save-first-save]'));
  const n = server.calls.length;
  window.dispatchEvent(new window.Event('focus'));
  await settle(20);
  assert.equal(server.calls.length, n, 'the form is open: no more reads');
  root.unmount();
});

test('C6 DOM: Loose ends’ own add form re-checks too (it is the same form)', async () => {
  reset();
  const { root } = await mount(h(m.LooseSources, { sources: [], error: null, onChanged: noop, initialAdding: true, initialKind: 'github', onAdded: noop, settled: true }));
  await frame();
  assert.ok(document.querySelector('[data-save-first]'));
  server.store = { ...STORE_OFF, approvals_off: false };
  window.dispatchEvent(new window.Event('focus'));
  await settle(20);
  assert.equal(document.querySelector('[data-save-first]'), null);
  root.unmount();
});

// ---------------------------------------------------------------- B1: one form, in place

test('B1: the popover sits under its button, above it when there is no room, inside the window', () => {
  assert.deepEqual(m.popoverPlace({ top: 100, bottom: 130, left: 200, right: 300 }, 1280, 800, 300), { left: 200, top: 138, width: 380 });
  assert.deepEqual(m.popoverPlace({ top: 700, bottom: 730, left: 1200, right: 1260 }, 1280, 800, 300), { left: 888, top: 392, width: 380 });
  assert.equal(m.popoverPlace(null, 1280, 800, 300).left, 450, 'no button: centred');
});

test('B1 DOM: on the desktop Connect GitHub opens the shared form as a popover by its button; Escape closes it and focus goes back', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false } });
  const { root } = await mount(h('div', null, h('button', { id: 'anchor' }, 'Connect GitHub'), h(m.GitHubConnectHost, {})));
  const anchor = document.getElementById('anchor');
  anchor.focus();
  m.openGitHubConnect({ space: 'spc_demo', anchor });
  await settle(20);
  await frame();
  const pop = document.querySelector('[data-github-connect-popover]');
  assert.ok(pop, 'a popover');
  assert.equal(document.querySelector('[data-github-connect-sheet]'), null);
  assert.equal(pop.getAttribute('role'), 'dialog');
  assert.ok(pop.querySelector('form[data-github-connect]'), 'the shared form is inside');
  assert.equal(window.location.pathname, '/s/spc_demo', 'no jump to Loose ends');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle(20);
  await frame();
  assert.equal(document.querySelector('[data-github-connect-popover]'), null, 'Escape closes it');
  assert.equal(document.activeElement, anchor, 'focus is back on the button');
  root.unmount();
});

test('B1 DOM: on a phone it is a sheet; adding the login closes it, reads Dev mode again and says so', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false } });
  phone = true;
  const connected = [];
  const heard = [];
  const on = (e) => heard.push(e.detail.space);
  window.addEventListener(m.GITHUB_CONNECTED_EVENT, on);
  const { root } = await mount(h(m.GitHubConnectHost, { onConnected: (space) => connected.push(space) }));
  m.openGitHubConnect({ space: 'spc_demo' });
  await settle(20);
  await frame();
  const sheet = document.querySelector('[data-github-connect-sheet]');
  assert.ok(sheet, 'a sheet');
  assert.equal(document.querySelector('[data-github-connect-popover]'), null);
  assert.equal(sheet.querySelector('[role="dialog"]').getAttribute('aria-modal'), 'true');
  document.querySelector('[data-github-token]').value = 'demo-not-a-token';
  document.querySelector('form[data-github-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(30);
  assert.equal(document.querySelector('[data-github-connect-sheet]'), null, 'closed once added');
  assert.deepEqual(connected, ['spc_demo']);
  assert.deepEqual(heard, ['spc_demo']);
  assert.ok(server.calls.some(([meth, p]) => meth === 'GET' && p === '/api/spaces/spc_demo/dev_mode'), 'Dev mode reads the new login');
  window.removeEventListener(m.GITHUB_CONNECTED_EVENT, on);
  root.unmount();
});

test('B1: every Connect GitHub goes through the shared form, and none opens Loose ends', () => {
  const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
  for (const f of ['components/DevStatusLine.tsx', 'components/DevDesk.tsx', 'components/DevModeSetup.tsx', 'components/TeamRepoDevLine.tsx', 'App.tsx']) assert.match(src(f), /openGitHubConnect/, f);
  assert.doesNotMatch(src('App.tsx'), /devConnect\(/);
  assert.match(src('App.tsx'), /<GitHubConnectHost/);
});

test('B1: the Loose ends GitHub row says it is also used by Dev mode', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false } });
  const src = { id: 'src_demo', kind: 'github', label: 'GitHub', enabled: true, config: {}, last_error: null, last_run_at: null };
  const { root } = await mount(h(m.LooseSources, { sources: [src], error: null, onChanged: noop, onAdded: noop, settled: true }));
  assert.equal(m.GITHUB_ALSO_DEV, 'Also used by Dev mode');
  assert.match(document.querySelector('[data-github-also-dev]')?.textContent ?? '', /Also used by Dev mode/);
  root.unmount();
});

// ---------------------------------------------------------------- B2 / B3: Team settings

const dev = (over) => m.parseDevMode({ on: true, forge: 'github', forge_login: 'demo', status: 'watching', repos: [], ...over });
const repo = (o) => ({ key: 'github.com/demo-org/demo-web', repo: 'demo-org/demo-web', host: 'github.com', team: 'Web', team_id: 'team_web', from: 'repo', provider: 'github', branch: 'main', status: 'watching', ...o });

test('B2: each of the four lines, matched by team first, then by repo', () => {
  const url = 'https://github.com/demo-org/demo-web.git';
  assert.equal(m.repoKeyOf(url), 'github.com/demo-org/demo-web');
  assert.equal(m.repoKeyOf('git@git.example.test:demo/app.git'), 'git.example.test/demo/app');
  assert.equal(m.repoKeyOf(''), null);
  assert.equal(m.teamRepoLine(dev({ repos: [repo()] }), url, 'team_web'), 'connected');
  assert.equal(m.teamRepoLine(dev({ repos: [repo({ status: 'error' })] }), url, 'team_web'), 'connected');
  assert.equal(m.teamRepoLine(dev({ repos: [repo({ status: 'not_connected' })] }), url, 'team_web'), 'connect');
  assert.equal(m.teamRepoLine(dev({ forge: null, repos: [repo()] }), url, 'team_web'), 'connect');
  assert.equal(m.teamRepoLine(dev({ on: false, repos: [] }), url, 'team_web'), 'off');
  assert.equal(m.teamRepoLine(dev({ on: false }), 'https://git.example.test/demo/app.git', null), 'off', 'off for any host');
  assert.equal(m.teamRepoLine(dev({ repos: [repo({ key: 'git.example.test/demo/app', host: 'git.example.test', provider: 'git' })] }), 'https://git.example.test/demo/app.git', 'team_web'), 'git');
  assert.equal(m.teamRepoLine(dev({}), 'https://git.example.test/demo/app.git', null), 'git', 'a new team on another host');
  // A second team on the same repo has no entry of its own: the repo's entry is used.
  assert.equal(m.teamRepoLine(dev({ repos: [repo()] }), url, 'team_other'), 'connected');
  // A URL typed over the team's saved repo is read by the new URL, not the old entry.
  const gitRow = repo({ key: '127.0.0.1/demo-org/demo-app', host: '127.0.0.1', provider: 'git' });
  assert.equal(m.teamRepoLine(dev({ forge: null, repos: [gitRow] }), 'https://github.com/demo-org/demo-app.git', 'team_web'), 'connect');
  assert.equal(m.teamRepoLine(dev({ repos: [repo({ key: '127.0.0.1/demo-org/demo-app', host: '127.0.0.1' })] }), 'http://127.0.0.1:9476/demo-org/demo-app', 'team_web'), 'connected', 'a port in the URL still matches');
  // A new team (no id yet) on GitHub reads on + forge.
  assert.equal(m.teamRepoLine(dev({}), 'demo-org/new-repo', null), 'connected');
  assert.equal(m.teamRepoLine(dev({ forge: null }), 'demo-org/new-repo', null), 'connect');
  assert.equal(m.teamRepoLine(dev({}), '', 'team_web'), null, 'no URL, no line');
  assert.equal(m.teamRepoLine(null, url, 'team_web'), null, 'unknown yet, no line');
});

test('B2 DOM: the spec’s copy for each line; Connect GitHub opens the shared form in place', async () => {
  reset();
  const url = 'https://github.com/demo-org/demo-web';
  for (const [state, line, text] of [
    [dev({ repos: [repo()] }), 'connected', 'Dev mode watches this repo · GitHub connected'],
    [dev({ repos: [repo({ status: 'not_connected' })], forge: null }), 'connect', 'Dev mode watches this repo. Connect GitHub for reviews.'],
    [dev({ on: false }), 'off', 'Dev mode can watch this repo: turn it on next to the space name.'],
    [dev({ repos: [repo({ provider: 'git' })] }), 'git', 'Dev mode reads this repo with git. Reviews need GitHub.'],
  ]) {
    m.devStore.set('spc_demo', state);
    const { root } = await mount(h(m.TeamRepoDevLine, { url, teamId: 'team_web', spaceId: 'spc_demo' }));
    const p = document.querySelector('[data-team-repo-dev]');
    assert.equal(p.dataset.teamRepoDev, line);
    assert.equal(p.textContent, text);
    assert.equal(m.TEAM_REPO_LINES[line], text);
    if (line === 'connect') {
      const got = [];
      const on = (e) => got.push(e.detail);
      window.addEventListener(m.GITHUB_CONNECT_EVENT, on);
      const btn = p.querySelector('[data-team-repo-connect]');
      await click(btn);
      window.removeEventListener(m.GITHUB_CONNECT_EVENT, on);
      assert.equal(got.length, 1);
      assert.equal(got[0].space, 'spc_demo');
      assert.equal(got[0].anchor, btn);
    }
    root.unmount();
    document.body.innerHTML = '';
  }
});

test('B2/B3: both team modals show the line under Git Repo URL, with the new helper text', () => {
  for (const f of ['NewTeamModal.tsx', 'ManageTeamModal.tsx']) {
    const src = readFileSync(new URL(`../src/components/${f}`, import.meta.url), 'utf8');
    assert.match(src, /<TeamRepoDevLine url=\{repoUrl\}/, f);
    assert.match(src, /Bots clone it into the team’s folder on first run\.\s*</, f);
    assert.doesNotMatch(src, /Dev mode watches it too/, f);
  }
  assert.match(readFileSync(new URL('../src/components/ManageTeamModal.tsx', import.meta.url), 'utf8'), /<TeamRepoDevLine url=\{repoUrl\} teamId=/);
});

// ---------------------------------------------------------------- C5: the other cloud

test('C5: the hint names the other Bitwarden cloud; nothing for self-hosted', () => {
  assert.equal(m.otherCloudHint('https://vault.bitwarden.com'), 'Is your account on bitwarden.eu? Choose it under Server.');
  assert.equal(m.otherCloudHint('https://vault.bitwarden.eu'), 'Is your account on bitwarden.com? Choose it under Server.');
  assert.equal(m.otherCloudHint('self'), null);
  assert.equal(m.otherCloudHint('https://vault.example.test'), null);
});

test('C5 SSR: shown only in connect mode under a wrong password', () => {
  const r = (p) => m.renderToStaticMarkup(h(m.BitwardenLoginForm, { mode: 'connect', status: null, onDone: noop, onCancel: noop, ...p }));
  assert.match(r({ initialProblem: { kind: 'wrong-password' } }), /data-bw-other-cloud[^>]*>Is your account on bitwarden\.eu\? Choose it under Server\./);
  assert.match(r({ initialProblem: { kind: 'wrong-password' }, initialServer: 'https://vault.bitwarden.eu' }), /Is your account on bitwarden\.com\?/);
  assert.doesNotMatch(r({ initialProblem: { kind: 'wrong-password' }, initialServer: 'self' }), /data-bw-other-cloud/);
  assert.doesNotMatch(r({}), /data-bw-other-cloud/, 'no problem, no hint');
  assert.doesNotMatch(r({ initialProblem: { kind: 'new-device-code' } }), /data-bw-other-cloud/);
  const unlock = m.renderToStaticMarkup(
    h(m.BitwardenLoginForm, { mode: 'unlock', status: m.parseStoreStatus({ store: 'bitwarden', locked: true, bitwarden: { server_url: 'https://vault.bitwarden.com', email: 'demo@example.com', login: 'password', unlocked: false } }), onDone: noop, onCancel: noop, initialProblem: { kind: 'wrong-password' } }),
  );
  assert.doesNotMatch(unlock, /data-bw-other-cloud/, 'unlock: the account is known already');
});

test('C5 DOM: a wrong password on bitwarden.eu names bitwarden.com, for the server it was tried on', async () => {
  reset();
  const { root } = await mount(h(m.BitwardenLoginForm, { mode: 'connect', status: null, onDone: noop, onCancel: noop }));
  const form = document.querySelector('[data-bitwarden-form="connect"]');
  typeIn(form.querySelector('select'), 'https://vault.bitwarden.eu');
  typeIn(form.querySelector('input[type="email"]'), 'demo@example.com');
  await settle();
  const pw = [...form.querySelectorAll('input[type="password"]')].pop();
  pw.value = 'demo-not-a-password';
  [...form.querySelectorAll('button')].find((b) => b.textContent === 'Connect').click();
  await settle(20);
  const unlock = server.calls.find(([, p]) => p === '/api/passwords/bitwarden/unlock');
  assert.equal(unlock[2].server_url, 'https://vault.bitwarden.eu');
  assert.equal(document.querySelector('[data-bw-other-cloud]')?.textContent, 'Is your account on bitwarden.com? Choose it under Server.');
  // Switching the picker afterwards doesn't change what was tried.
  typeIn(form.querySelector('select'), 'https://vault.bitwarden.com');
  await settle();
  assert.equal(document.querySelector('[data-bw-other-cloud]')?.textContent, 'Is your account on bitwarden.com? Choose it under Server.');
  root.unmount();
});

// ---------------------------------------------------------------- F1: no form inside a form

const nested = () => [...document.querySelectorAll('form')].filter((f) => f.parentElement?.closest('form'));
const unlocks = () => server.calls.filter(([meth, p]) => meth === 'POST' && p === '/api/passwords/bitwarden/unlock');
const fillBitwarden = async (bw) => {
  typeIn(bw.querySelector('input[type="email"]'), 'demo@example.com');
  await settle();
  [...bw.querySelectorAll('input[type="password"]')].pop().value = 'demo-not-a-password';
};

test('F1: the Bitwarden login form is never a <form>, so it can sit inside one', () => {
  for (const mode of ['connect', 'unlock']) {
    const html = m.renderToStaticMarkup(h(m.BitwardenLoginForm, { mode, status: null, onDone: noop, onCancel: noop }));
    assert.doesNotMatch(html, /<form/, mode);
    assert.match(html, /role="group"/);
    assert.doesNotMatch(html, /type="submit"/, `${mode}: no submit button to reach an outer form`);
  }
});

test('F1 DOM: GitHub > Use Bitwarden: no nested form; Connect and Enter each send exactly one unlock and never submit the GitHub form', async () => {
  reset();
  const submits = [];
  const onSubmit = (e) => submits.push(e.target.getAttribute('data-add-source') ?? e.target.tagName);
  document.addEventListener('submit', onSubmit, true);
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: noop, onCancel: noop, land: true, scroll: false }));
  await frame();
  await click(document.querySelector('[data-save-first-bitwarden]'));
  const bw = document.querySelector('[data-bitwarden-form="connect"]');
  assert.ok(bw.closest('form[data-github-connect]'), 'shown inside the GitHub form');
  assert.equal(bw.tagName, 'DIV');
  assert.deepEqual(nested(), []);
  await fillBitwarden(bw);
  [...bw.querySelectorAll('button')].find((b) => b.textContent === 'Connect').click();
  await settle(20);
  assert.equal(unlocks().length, 1, 'Connect: one unlock');
  // Enter in the master password field: one more unlock, and the key never reaches the GitHub form.
  const pw = [...bw.querySelectorAll('input[type="password"]')].pop();
  pw.value = 'demo-not-a-password';
  const enter = new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  pw.dispatchEvent(enter);
  await settle(20);
  assert.equal(enter.defaultPrevented, true, 'no implicit submit of the outer form');
  assert.equal(unlocks().length, 2, 'Enter: one more unlock');
  // Enter in the Server picker does nothing.
  bw.querySelector('select').dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  await settle(20);
  assert.equal(unlocks().length, 2);
  assert.deepEqual(submits, [], 'no form was submitted');
  assert.deepEqual(posts(), [], 'nothing saved to Loose ends');
  document.removeEventListener('submit', onSubmit, true);
  root.unmount();
});

test('F1 DOM: GitHub > 409 > Move to Bitwarden (NeedsBitwarden in the GitHub form): no nested form', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false }, refuse: true });
  const { root } = await mount(h(m.GitHubConnectForm, { onAdded: noop, onCancel: noop, land: true, scroll: false }));
  await frame();
  document.querySelector('[data-github-connect] input[type="password"]').value = 'demo-token-not-real';
  document.querySelector('[data-github-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(20);
  await click([...document.querySelectorAll('[data-needs-bitwarden] button')].find((b) => b.textContent === 'Move to Bitwarden'));
  assert.ok(document.querySelector('[data-github-connect] [data-bitwarden-form]'));
  assert.deepEqual(nested(), []);
  root.unmount();
});

test('F1 DOM: IMAP > 409 > Move to Bitwarden: no nested form, and Connect sends one unlock without submitting the IMAP form', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false }, refuse: true });
  const submits = [];
  const onSubmit = () => submits.push(1);
  const { root } = await mount(h(m.AddSourceForm, { onAdded: noop, onCancel: noop }));
  const form = document.querySelector('form[data-add-source="imap"]');
  const inputs = [...form.querySelectorAll('input')];
  typeIn(inputs.find((i) => i.placeholder === 'imap.gmail.com'), 'imap.example.test');
  typeIn(inputs.find((i) => i.placeholder === 'you@example.com'), 'demo@example.test');
  form.querySelector('input[type="password"]').value = 'demo-not-a-password';
  await settle();
  form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(20);
  await click([...form.querySelectorAll('[data-needs-bitwarden] button')].find((b) => b.textContent === 'Move to Bitwarden'));
  const bw = form.querySelector('[data-bitwarden-form="connect"]');
  assert.ok(bw);
  assert.deepEqual(nested(), []);
  document.addEventListener('submit', onSubmit, true);
  await fillBitwarden(bw);
  [...bw.querySelectorAll('button')].find((b) => b.textContent === 'Connect').click();
  await settle(20);
  assert.equal(unlocks().length, 1);
  assert.deepEqual(submits, []);
  document.removeEventListener('submit', onSubmit, true);
  root.unmount();
});

test('F1 DOM: Move to Bitwarden first (the approvals confirm) has no form in a form', async () => {
  reset();
  const status = m.parseStoreStatus({ ...STORE_OFF, local_count: 3 });
  const { root } = await mount(h('form', null, h(m.MoveToBitwardenFirst, { store: { status, problem: null, reload: noop, set: noop }, label: 'Move them to Bitwarden first', initialStep: 'connect' })));
  assert.ok(document.querySelector('[data-move-first] [data-bitwarden-form="connect"]'));
  assert.deepEqual(nested(), []);
  root.unmount();
});

// ---------------------------------------------------------------- F2: restoring an archived space

test('F2 DOM: restore 409 → the one-time choice (A3 copy) → Save it on this server restores with accept_risk', async () => {
  reset({ store: { ...STORE_OFF, local_count: 2 } });
  const restored = [];
  const { root } = await mount(h(m.ArchivedSpacesModal, { onClose: noop, onRestored: (s) => restored.push(s) }));
  await settle(20);
  await click([...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Restore'));
  await settle(20);
  const card = document.querySelector('[data-restore-accept-risk]');
  assert.ok(card, 'the choice, not the raw 409 sentence');
  assert.equal(document.querySelector('[role="alert"]'), null, 'no error line');
  assert.match(card.textContent, new RegExp(m.acceptRiskText(2).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  assert.equal(card.querySelector('[data-restore-save]').textContent, 'Save it on this server');
  assert.ok([...card.querySelectorAll('button')].some((b) => b.textContent.includes('Use Bitwarden')), 'Use Bitwarden');
  assert.equal(restored.length, 0);
  await click(card.querySelector('[data-restore-save]'));
  await settle(20);
  const patches = server.calls.filter(([meth, p]) => meth === 'PATCH' && p === '/api/spaces/spc_old').map(([, , b]) => b);
  assert.deepEqual(patches, [{ archived: false }, { archived: false, accept_risk: true }]);
  assert.equal(restored.length, 1);
  assert.equal(document.querySelector('[data-restore-accept-risk]'), null);
  root.unmount();
});

// ---------------------------------------------------------------- N4

test('N4 DOM: choosing GitHub in Add a source focuses its first control', async () => {
  reset({ store: { ...STORE_OFF, approvals_off: false } });
  const { root } = await mount(h(m.AddSourceForm, { onAdded: noop, onCancel: noop }));
  await click([...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'GitHub'));
  await frame();
  await settle(20);
  assert.equal(document.activeElement?.getAttribute('type'), 'password', 'the token field');
  assert.ok(document.activeElement.closest('[data-github-connect]'));
  root.unmount();
});
