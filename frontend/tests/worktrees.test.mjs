// 0.9.0 H1 + P3 (Pixel): the Dev desk's Worktrees tab. Copy helpers, the parser, the list with every row
// state, the P3 line, New worktree (new and existing branch, inline errors) and Remove (a dirty checkout
// needs "Remove anyway"; the branch is never deleted). The server is a mocked fetch on Lin's contract v2.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach, beforeEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLSelectElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export * from './src/dev/worktrees';
  export * as fx from './src/dev/worktreeFixtures';
  export { WorktreesPanel } from './src/components/dev/WorktreesTab';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
  export { act } from 'react';
`);
const { fx } = m;
const h = m.createElement;

// ---------------------------------------------------------------------------------------------------
// Mocked server

let calls = [];
let routes = {};
const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
beforeEach(() => {
  calls = [];
  routes = {
    'GET /api/spaces/sp1/dev/worktrees': () => json(200, fx.LIST),
    'GET /api/spaces/sp1/dev/worktrees/branches': () => json(200, fx.BRANCHES),
  };
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const path = String(url).split('?')[0];
    calls.push({ method, url: String(url), body: init.body ? JSON.parse(init.body) : undefined });
    const r = routes[`${method} ${path}`];
    return r ? r(String(url), init) : json(404, {});
  };
  m.worktreeStore._reset();
});

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const Panel = (props) => {
  const data = m.worktreeStore.get('sp1');
  return h(m.WorktreesPanel, { space: 'sp1', data, bots: fx.BOTS, now: fx.NOW, home: '/home/ann', ...props });
};
let rerender = () => undefined;
const mount = async (props = {}) => {
  await m.worktreeStore.load('sp1');
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  rerender = () => root.render(h(Panel, props));
  const stop = m.worktreeStore.subscribe(() => rerender());
  rerender();
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    stop();
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const click = async (el) => {
  assert.ok(el, 'element to click exists');
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const setValue = async (el, v) => {
  const proto = el.tagName === 'SELECT' ? window.HTMLSelectElement.prototype : window.HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
  el.dispatchEvent(new window.Event(el.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
  await settle();
};
const tick = async (el) => {
  el.click();
  await settle();
};
const row = (host, id) => host.querySelector(`[data-wt-row="${id}"]`);
const text = (el) => {
  // Block boundaries read as a space, the way a screen reader or a person reads the row.
  if (!el.querySelectorAll) return el.textContent.replace(/\s+/g, ' ').trim();
  const c = el.cloneNode(true);
  for (const b of c.querySelectorAll('p, h2, h3, div, li, ul, button, label, select, input, span, code')) { b.before(' '); b.after(' '); }
  return c.textContent.replace(/\s+/g, ' ').replace(/ ([.,?’)])/g, '$1').replace(/\( /g, '(').trim();
};

// ---------------------------------------------------------------------------------------------------
// Helpers

test('parseList keeps repos, drops removed and malformed rows, and reads other worktrees', () => {
  const l = m.parseList({ ...fx.LIST, repos: [{ ...fx.LIST.repos[0], worktrees: [...fx.LIST.repos[0].worktrees, { id: 'x' }, fx.wt('gone', 'gone', { state: 'removed' })] }] });
  assert.equal(l.repos.length, 1);
  assert.equal(l.repos[0].worktrees.length, 4);
  assert.equal(l.repos[0].other[0].branch, 'hotfix/1.4.2');
  assert.equal(m.worktreeCount(l), 5);
  assert.equal(m.parseList({ nope: 1 }), null);
  assert.equal(m.parseList({ repos: [] }).clean_after_minutes, 60);
});

test('row status and time copy', () => {
  const W = (o) => m.parseWorktree(o);
  assert.deepEqual(m.rowStatus(W(fx.WORKING), fx.NOW), { led: 'run', text: 'is working here', kind: 'working' });
  assert.equal(m.rowStatus(W(fx.FINISHED_DUE), fx.NOW).text, 'finished 13:42');
  assert.equal(m.rowStatus(W(fx.FINISHED_DUE), fx.NOW).led, 'ok');
  assert.equal(m.rowStatus(W(fx.CLEANED), fx.NOW).text, 'finished yesterday');
  assert.equal(m.rowStatus(W(fx.MINE), fx.NOW).text, 'made by you');
  assert.equal(m.rowStatus(W(fx.MISSING), fx.NOW).kind, 'missing');
  assert.equal(m.rowTime(W(fx.WORKING), fx.NOW), 'active now');
  assert.equal(m.rowTask(W(fx.MINE), fx.NOW), 'No task. Made from the Dev desk 09:12');
  assert.equal(m.changesLabel(0), 'No uncommitted changes');
  assert.equal(m.changesLabel(1), '1 uncommitted file');
  assert.equal(m.changesLabel(3), '3 uncommitted files');
  assert.equal(m.changesLabel(null), null);
});

test('P3 states: due, kept, cleaning, cleaned with kept notes, failed', () => {
  const W = (o) => m.parseWorktree(o);
  assert.deepEqual(m.p3State(W(fx.FINISHED_DUE)), { kind: 'due', at: '14:42', dirs: [] });
  assert.deepEqual(m.p3State(W({ ...fx.FINISHED_DUE, keep_build: true })), { kind: 'kept' });
  assert.deepEqual(m.p3State(W(fx.FINISHED_DUE), { cleaning: true }), { kind: 'cleaning' });
  assert.deepEqual(m.p3State(W(fx.CLEANED)), { kind: 'cleaned', size: '1.2 GB', at: '18:40', dirs: ['.vite', 'target'] });
  assert.deepEqual(m.keptNotes(W(fx.CLEANED)), ['Kept dist: Git tracks a file in it.']);
  assert.equal(m.p3State(W(fx.WORKING)), null);
  assert.equal(m.p3State(W(fx.MISSING)), null);
  assert.deepEqual(m.p3State(W(fx.FINISHED_DUE), { failed: 'Lin is working here' }), { kind: 'failed', reason: 'Lin is working here' });
  assert.equal(m.sizeLabel(512), '512 bytes');
  assert.equal(m.sizeLabel(340_000_000), '340 MB');
});

test('branch names: slug matches the server, spaces get a suggestion, bad refs are refused', () => {
  assert.equal(m.branchSlug('Pixel/Inbox Empty.State'), 'pixel-inbox-empty-state');
  assert.equal(m.branchSlug('a'.repeat(80)).length, 60);
  assert.equal(m.branchSlug('///'), 'worktree');
  assert.deepEqual(m.branchProblem('fix the inbox'), { kind: 'spaces', suggestion: 'fix-the-inbox' });
  for (const bad of ['-x', 'a..b', 'a/', 'a.lock', 'x~1', 'a:b', '@', 'a//b', '.hidden/x']) assert.deepEqual(m.branchProblem(bad), { kind: 'invalid' }, bad);
  assert.equal(m.branchProblem('pixel/inbox-empty'), null);
  assert.equal(m.suggestBranch('Pixel', 'Inbox: empty state & keyboard order now'), 'pixel/inbox-empty-state-keyboard');
  assert.equal(m.suggestBranch(null, null), '');
});

test('error copy names the problem and says nothing changed', () => {
  assert.equal(m.createErrorCopy({ code: 'branch_exists', message: '' }, { branch: 'a', path: 'p' }).field, true);
  assert.match(m.createErrorCopy({ code: 'git_failed', message: '' }, { branch: 'a', path: 'p' }).text, /Nothing was changed\.$/);
  assert.equal(m.removeErrorCopy({ code: 'working', message: '' }, 'Lin'), 'Lin is working here. Stop Lin first.');
  assert.match(m.removeErrorCopy({ code: null, message: 'Boom.' }, null), /^Couldn’t remove it: Boom\. Nothing was deleted\.$/);
  assert.equal(m.shortPath('/home/ann/code/shop', '/home/ann'), '~/code/shop');
  assert.equal(m.shortPath('/srv/x'), '/srv/x');
});

test('store: a 404 without a code means the server has no routes yet (the tab hides)', async () => {
  routes['GET /api/spaces/sp1/dev/worktrees'] = () => json(404, {});
  await m.worktreeStore.load('sp1');
  assert.equal(m.worktreeStore.get('sp1').missing, true);
  assert.equal(m.worktreeStore.get('sp1').error, null);
  routes['GET /api/spaces/sp1/dev/worktrees'] = () => json(500, { error: 'Disk is full.' });
  await m.worktreeStore.load('sp1');
  assert.equal(m.worktreeStore.get('sp1').missing, false);
  assert.equal(m.worktreeStore.get('sp1').error, 'Disk is full.');
});

// ---------------------------------------------------------------------------------------------------
// The list

test('the list groups by repo with counts, rows in every state, and an empty repo', async () => {
  const host = await mount();
  const shop = host.querySelector('[data-wt-repo="shop"]');
  assert.match(text(shop.querySelector('h3')), /shop 5 worktrees ~\/code\/shop · main checkout/);
  const w1 = row(host, 'w1');
  assert.equal(w1.dataset.wtState, 'working');
  assert.match(text(w1), /Pixel is working here pixel\/inbox-empty-state/);
  assert.match(text(w1), /3 uncommitted files/);
  assert.match(text(w1), /↑2/);
  assert.equal(w1.querySelector('[data-wt-act="remove"]').getAttribute('aria-disabled'), 'true');
  assert.match(text(w1), /Pixel is working here\. Stop Pixel first\./);
  assert.match(text(row(host, 'w2')), /Lin finished 13:42/);
  assert.match(text(row(host, 'w2')), /up to date|↑4/);
  assert.match(text(row(host, 'w4')), /You made by you try-vite-6/);
  assert.match(text(row(host, 'w4')), /No task\. Made from the Dev desk 09:12/);
  assert.match(text(row(host, '/home/ann/code/shop-hotfix')), /shop-hotfix made outside kwant hotfix\/1\.4\.2/);
  assert.equal(row(host, '/home/ann/code/shop-hotfix').querySelector('[data-wt-act="remove"]'), null, 'outside rows are read-only');
  const site = host.querySelector('[data-wt-repo="site"]');
  assert.ok(site.querySelector('[data-wt-empty="empty"]'));
  assert.match(text(site), /Each bot task can get its own folder\./);
});

test('P3 line: Keep it, then Clean now; a refused clean says why and nothing else was touched', async () => {
  routes['POST /api/dev/worktrees/w2/keep'] = (_u, init) => json(200, { ...fx.FINISHED_DUE, keep_build: JSON.parse(init.body).keep });
  routes['POST /api/dev/worktrees/w2/clean'] = () => json(409, { code: 'working', error: 'A bot is working here.' });
  const host = await mount();
  const p3 = () => row(host, 'w2').querySelector('[data-wt-p3]');
  assert.equal(p3().dataset.wtP3, 'due');
  assert.match(text(p3()), /Build output is cleaned at 14:42 · Keep it/);
  await click(p3().querySelector('[data-wt-keep]'));
  assert.deepEqual(calls.at(-2).body ?? calls.find((c) => c.url.endsWith('/keep')).body, { keep: true });
  assert.equal(p3().dataset.wtP3, 'kept');
  await click(p3().querySelector('[data-wt-clean]'));
  assert.equal(p3().dataset.wtP3, 'failed');
  assert.match(text(p3()), /Couldn’t clean build output: Lin is working here\. Nothing else was touched\. · Try again/);
  const cleaned = row(host, 'w3').querySelector('[data-wt-p3]');
  assert.match(text(cleaned), /Cleaned 1\.2 GB of build output at 18:40 \(\.vite, target\) · Kept dist: Git tracks a file in it\./);
});

test('missing folder: Forget it removes the record', async () => {
  routes['GET /api/spaces/sp1/dev/worktrees'] = () => json(200, { ...fx.LIST, repos: [{ ...fx.LIST.repos[0], worktrees: [fx.MISSING] }] });
  routes['DELETE /api/dev/worktrees/w5'] = () => json(200, { ok: true });
  const host = await mount();
  assert.match(text(row(host, 'w5')), /Folder missing old\/spike Git still lists ~\/\.kwant\/spaces\/sp1\/worktrees\/shop\/old-spike, but the folder is gone\./);
  await click(row(host, 'w5').querySelector('[data-wt-act="forget"]'));
  assert.ok(calls.some((c) => c.method === 'DELETE' && c.url.endsWith('/w5?force=false')));
});

test('loading, no repo, a whole-list error with Try again, and a repo that can’t be read', async () => {
  m.worktreeStore._set('sp1', { loading: true });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(Panel));
  await settle();
  assert.ok(host.querySelector('[data-wt-loading] [role="status"]'));
  m.worktreeStore._set('sp1', { loading: false, list: { repos: [], clean_after_minutes: 60, build_dirs: [] } });
  root.render(h(Panel));
  await settle();
  assert.ok(host.querySelector('[data-wt-empty="no-repo"]'));
  m.worktreeStore._set('sp1', { list: null, error: 'Disk is full.' });
  root.render(h(Panel));
  await settle();
  assert.match(text(host.querySelector('[data-wt-error]')), /Couldn’t read the worktrees\. Disk is full\. Try again/);
  m.worktreeStore._set('sp1', { error: null, list: m.parseList({ ...fx.LIST, repos: [{ ...fx.LIST.repos[1], error: true }] }) });
  root.render(h(Panel));
  await settle();
  assert.match(text(host.querySelector('[data-wt-repo-error]')), /Couldn’t read the worktrees of site\. Try again/);
  assert.equal(host.querySelector('[data-wt-new]'), null, 'no New worktree on a repo that can’t be read');
  root.unmount();
});

test('Copy path announces politely', async () => {
  let copied = null;
  Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (t) => void (copied = t) }, configurable: true });
  const host = await mount();
  await click(row(host, 'w2').querySelector('[data-wt-act="copy"]'));
  assert.equal(copied, `${fx.ROOT}/shop/lin-worktree-routes`);
  assert.equal(host.querySelector('[data-wt-live]').getAttribute('aria-live'), 'polite');
  assert.equal(text(host.querySelector('[data-wt-live]')), 'Path copied');
});

// ---------------------------------------------------------------------------------------------------
// New worktree

test('create a new branch: default bot, suggested name, preview, request body, toast with Open chat', async () => {
  let created = null;
  routes['POST /api/spaces/sp1/dev/worktrees'] = (_u, init) => json(201, (created = fx.wt('w9', JSON.parse(init.body).branch, { kwant_id: 'k-lin', thread_id: 'th-lin', base: 'main' })));
  const opened = [];
  const host = await mount({ defaultBotId: 'k-lin', threadFor: (id) => `th-${id}`, onOpenChat: (b, t) => opened.push([b, t]) });
  const newBtn = host.querySelector('[data-wt-repo="shop"] [data-wt-new]');
  await click(newBtn);
  const dlg = document.querySelector('[data-wt-dialog="create"]');
  assert.equal(dlg.getAttribute('role'), 'dialog');
  assert.equal(dlg.getAttribute('aria-modal'), 'true');
  assert.equal(document.activeElement, dlg.querySelector('[data-wt-for]'), 'focus starts on For');
  assert.equal(dlg.querySelector('[data-wt-for]').value, 'k-lin');
  const input = dlg.querySelector('[data-wt-branch-input]');
  assert.equal(input.value, 'lin');
  await setValue(input, 'lin/search-index');
  assert.match(text(dlg.querySelector('[data-wt-preview]')), /folder ~\/\.kwant\/spaces\/sp1\/worktrees\/shop\/lin-search-index branch lin\/search-index \(new, from main\)/);
  const base = dlg.querySelector('[data-wt-base]');
  assert.equal(base.tagName, 'SELECT', 'base is a picker when the branch list exists');
  await setValue(base, 'release/0.9');
  dlg.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  const post = calls.find((c) => c.method === 'POST');
  assert.deepEqual(post.body, { team_id: 't-shop', branch: 'lin/search-index', base: 'release/0.9', kwant_id: 'k-lin', thread_id: 'th-k-lin' });
  assert.ok(created);
  assert.equal(document.querySelector('[data-wt-dialog]'), null);
  assert.equal(document.activeElement, newBtn, 'focus returns to New worktree');
  const t = host.querySelector('[data-wt-toast]');
  assert.match(text(t), /Worktree ready for Lin Open chat/);
  await click(t.querySelector('button'));
  assert.deepEqual(opened, [['k-lin', 'th-lin']]);
});

test('create: inline errors for spaces, a branch with a worktree, an existing branch, and the server', async () => {
  routes['POST /api/spaces/sp1/dev/worktrees'] = () => json(409, { code: 'folder_taken', error: 'Folder taken.' });
  const host = await mount();
  await click(host.querySelector('[data-wt-repo="shop"] [data-wt-new]'));
  const dlg = document.querySelector('[data-wt-dialog="create"]');
  const input = dlg.querySelector('[data-wt-branch-input]');
  const create = dlg.querySelector('[data-wt-create]');
  const err = () => text(dlg.querySelector('[data-wt-field-error]') ?? { textContent: '' });
  await setValue(input, 'fix the inbox');
  assert.equal(err(), 'Branch names can’t have spaces. Try fix-the-inbox.');
  assert.equal(input.getAttribute('aria-invalid'), 'true');
  assert.ok(input.getAttribute('aria-describedby').includes(dlg.querySelector('[data-wt-field-error]').id));
  assert.equal(create.disabled, true);
  await setValue(input, 'pixel/inbox-empty-state');
  assert.equal(err(), 'pixel/inbox-empty-state already has a worktree: ~/.kwant/spaces/sp1/worktrees/shop/pixel-inbox-empty-state.');
  await setValue(input, 'release/0.9');
  assert.equal(err(), 'release/0.9 already exists. Pick Existing branch to use it, or choose another name.');
  await setValue(input, 'pixel/new-thing');
  assert.equal(err(), '');
  assert.equal(create.disabled, false);
  await click(create);
  assert.equal(err(), '~/.kwant/spaces/sp1/worktrees/shop/pixel-new-thing already exists and isn’t empty. kwant won’t write into it.');
  assert.ok(document.querySelector('[data-wt-dialog="create"]'), 'the dialog stays open');
});

test('create from an existing branch: checked-out branches are disabled, no base is sent', async () => {
  routes['POST /api/spaces/sp1/dev/worktrees'] = (_u, init) => json(201, fx.wt('w8', JSON.parse(init.body).branch));
  const host = await mount();
  await click(host.querySelector('[data-wt-repo="shop"] [data-wt-new]'));
  const dlg = document.querySelector('[data-wt-dialog="create"]');
  await setValue(dlg.querySelector('[data-wt-for]'), '');
  await click(dlg.querySelector('[data-wt-mode="existing"]'));
  assert.equal(dlg.querySelector('[data-wt-mode="existing"]').getAttribute('aria-checked'), 'true');
  const pick = dlg.querySelector('[data-wt-pick]');
  const opt = (v) => [...pick.options].find((o) => o.value === v);
  assert.equal(opt('main').disabled, true);
  assert.match(opt('main').textContent, /checked out at ~\/code\/shop/);
  assert.equal(dlg.querySelector('[data-wt-create]').disabled, true);
  await setValue(pick, 'feature/search');
  assert.match(text(dlg.querySelector('[data-wt-preview]')), /branch feature\/search \(existing\)/);
  await click(dlg.querySelector('[data-wt-create]'));
  assert.deepEqual(calls.find((c) => c.method === 'POST').body, { team_id: 't-shop', branch: 'feature/search' });
  assert.match(text(host.querySelector('[data-wt-toast]')), /^Worktree ready$/);
});

test('create without the branch route: the base and branch become text fields', async () => {
  delete routes['GET /api/spaces/sp1/dev/worktrees/branches'];
  const host = await mount();
  await click(host.querySelector('[data-wt-repo="shop"] [data-wt-new]'));
  const dlg = document.querySelector('[data-wt-dialog="create"]');
  assert.equal(dlg.querySelector('[data-wt-base]').tagName, 'INPUT');
  assert.equal(dlg.querySelector('[data-wt-base]').value, 'main');
  await click(dlg.querySelector('[data-wt-mode="existing"]'));
  assert.equal(dlg.querySelector('[data-wt-pick]').tagName, 'INPUT');
});

test('create dialog: Esc closes, Tab wraps', async () => {
  const host = await mount();
  await click(host.querySelector('[data-wt-repo="shop"] [data-wt-new]'));
  const dlg = document.querySelector('[data-wt-dialog="create"]');
  const items = [...dlg.querySelectorAll('button:not([disabled]), input:not([disabled]), select:not([disabled])')];
  items.at(-1).focus();
  items.at(-1).dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
  await settle();
  assert.equal(document.activeElement, items[0]);
  dlg.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(document.querySelector('[data-wt-dialog]'), null);
});

// ---------------------------------------------------------------------------------------------------
// Remove

test('remove a clean worktree: alertdialog, focus on Cancel, the branch stays', async () => {
  routes['DELETE /api/dev/worktrees/w2'] = () => json(200, { ok: true });
  const host = await mount();
  await click(row(host, 'w2').querySelector('[data-wt-act="remove"]'));
  const dlg = document.querySelector('[data-wt-dialog="remove"]');
  assert.equal(dlg.getAttribute('role'), 'alertdialog');
  assert.equal(document.activeElement, dlg.querySelector('[data-wt-cancel]'));
  assert.match(text(dlg), /Remove Lin’s worktree\? This deletes the folder ~\/\.kwant\/spaces\/sp1\/worktrees\/shop\/lin-worktree-routes\. The branch lin\/worktree-routes stays, with its 4 commits\./);
  assert.match(text(dlg), /No uncommitted changes\./);
  assert.equal(dlg.querySelector('[data-wt-force]'), null);
  await click(dlg.querySelector('[data-wt-remove]'));
  assert.ok(calls.some((c) => c.method === 'DELETE' && c.url.endsWith('/w2?force=false')));
  assert.match(text(host.querySelector('[data-wt-toast]')), /Removed Lin’s worktree\. The branch lin\/worktree-routes stays\./);
});

test('remove a dirty worktree: refused until "Remove anyway" is ticked, then force=true', async () => {
  routes['DELETE /api/dev/worktrees/w4'] = (u) => (u.endsWith('force=true') ? json(200, { ok: true }) : json(409, { code: 'dirty', dirty: 1, changes: [{ status: 'M', path: 'package.json' }] }));
  const host = await mount();
  await click(row(host, 'w4').querySelector('[data-wt-act="remove"]'));
  const dlg = document.querySelector('[data-wt-dialog="remove"]');
  assert.match(text(dlg), /1 file has changes nobody committed\. Removing loses it\./);
  assert.match(text(dlg.querySelector('[data-wt-changes]')), /M package\.json/);
  const go = dlg.querySelector('[data-wt-remove]');
  assert.equal(go.disabled, true);
  await click(go);
  assert.equal(calls.filter((c) => c.method === 'DELETE').length, 0);
  await tick(dlg.querySelector('[data-wt-force]'));
  assert.equal(go.disabled, false);
  await click(go);
  assert.deepEqual(calls.filter((c) => c.method === 'DELETE').map((c) => c.url), ['/api/dev/worktrees/w4?force=true']);
  assert.match(text(host.querySelector('[data-wt-toast]')), /The branch try-vite-6 stays\./);
});

test('remove: a checkout that got dirty since the list was read shows the server’s changes and asks again', async () => {
  routes['DELETE /api/dev/worktrees/w2'] = () =>
    json(409, { code: 'dirty', dirty: 2, changes: [{ status: 'M', path: 'src/a.rs' }, { status: '?', path: 'notes.md' }] });
  const host = await mount();
  await click(row(host, 'w2').querySelector('[data-wt-act="remove"]'));
  const dlg = document.querySelector('[data-wt-dialog="remove"]');
  await click(dlg.querySelector('[data-wt-remove]'));
  assert.match(text(dlg), /2 files have changes nobody committed\. Removing loses them\./);
  assert.match(text(dlg), /Remove anyway and lose these 2 changes/);
  assert.equal(dlg.querySelector('[data-wt-remove]').disabled, true);
  assert.ok(document.querySelector('[data-wt-dialog="remove"]'));
});

test('remove refused by the server says why and nothing was deleted', async () => {
  routes['DELETE /api/dev/worktrees/w2'] = () => json(409, { code: 'working', error: 'busy' });
  const host = await mount();
  await click(row(host, 'w2').querySelector('[data-wt-act="remove"]'));
  const dlg = document.querySelector('[data-wt-dialog="remove"]');
  await click(dlg.querySelector('[data-wt-remove]'));
  assert.equal(text(dlg.querySelector('[data-wt-dialog-error]')), 'Lin is working here. Stop Lin first.');
});

// ---------------------------------------------------------------------------------------------------
// Phone

test('phone: rows get a ⋯ sheet with Open chat, Copy path, Keep, Clean now and Remove', async () => {
  const opened = [];
  const host = await mount({ phone: true, onOpenChat: (b, t) => opened.push([b, t]) });
  assert.equal(row(host, 'w2').querySelector('[data-wt-act="remove"]'), null);
  assert.equal(host.querySelector('[data-wt-repo="shop"] [data-wt-new]').getAttribute('aria-label'), 'New worktree in shop');
  await click(row(host, 'w2').querySelector('[data-wt-more]'));
  const sheet = document.querySelector('[data-wt-dialog="row-menu"]');
  assert.deepEqual([...sheet.querySelectorAll('button')].map((b) => text(b)), ['Open chat', 'Copy path', 'Keep build output', 'Clean now', 'Remove']);
  await click([...sheet.querySelectorAll('button')].find((b) => text(b) === 'Remove'));
  assert.ok(document.querySelector('[data-wt-dialog="remove"]'));
  assert.equal(document.querySelector('[data-wt-dialog="row-menu"]'), null);
  await click(document.querySelector('[data-wt-cancel]'));
  await click(row(host, 'w1').querySelector('[data-wt-more]'));
  const s2 = document.querySelector('[data-wt-dialog="row-menu"]');
  assert.equal([...s2.querySelectorAll('button')].find((b) => text(b) === 'Remove').disabled, true);
  assert.match(text(s2), /Pixel is working here\. Stop Pixel first\./);
});
