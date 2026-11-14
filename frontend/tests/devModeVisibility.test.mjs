// Dev mode visibility (autonomy/dev-mode-visibility/plan.md, Pixel): the Dev mode card is first in
// Settings › This space › General; Off sells (an accent card naming the found repo, its fields behind
// Details; a quiet card with a primary Turn on when nothing was found), On is unchanged; the "this space
// has code" suggestion in #general, its gating, Not now for good, and Turn on landing focused on the card.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));
g.IS_REACT_ACT_ENVIRONMENT = false;

const repo = (over = {}) => ({
  key: 'github.com/demo-org/demo-app', repo: 'demo-org/demo-app', host: 'github.com', team: 'Dev team', team_id: 't1', from: 'repo', branch: 'main',
  status: 'not_connected', poll_error: null, poll_code: null, polled_at: null, last_tag: null, added_by: 'team', provider: 'github', forge: 'github',
  forge_connected: false, fresh_days: null, old_reviews_skipped: null, ...over,
});
const off = { on: false, repo: null, branch: null, deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null };
const offFound = { ...off, repos: [repo()] };
let server;
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  server.calls.push([method, u.split('?')[0], body]);
  const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
  if (server.status !== 200) return json({ error: 'Only the person can do that.' }, server.status);
  if (u.includes('/dev_mode')) {
    if (method === 'PUT') server.state = { ...server.state, ...body, polled_at: body.on ? '2026-11-14T09:00:00Z' : server.state.polled_at };
    return json(server.state);
  }
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { devStore, devApi, parseDevMode } from './src/devMode';
  export { DevModeSettings } from './src/components/DevModeSettings';
  export { DevModeSuggestion } from './src/components/DevModeSuggestion';
  export * from './src/devSuggest';
  export { ringDevSettings, focusDevSettings } from './src/firstScreen';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const reset = (state = off, status = 200) => {
  server = { state: { ...state }, status, calls: [] };
  m.devStore._reset();
  m._resetAsked();
  localStorage.clear();
  document.body.innerHTML = '';
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root };
};
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const puts = () => server.calls.filter(([meth]) => meth === 'PUT').map(([, , b]) => b);
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const state = (s) => m.parseDevMode(s);

// ------------------------------------------------------------------ Settings › General

test('General: the Dev mode card is first, above the name/icon/colour form and Approvals', () => {
  const s = src('components/SpaceSettings.tsx');
  const general = s.slice(s.indexOf('const GeneralTab'));
  const dev = general.indexOf('<DevModeSettings');
  assert.ok(dev > 0);
  assert.ok(dev < general.indexOf('<form onSubmit={save}'), 'above the name form');
  assert.ok(dev < general.indexOf('<SpaceApprovalsSetting'), 'above approvals');
  assert.equal(general.split('<DevModeSettings').length, 2, 'once');
});

test('Off + repo found: accent card naming the repo and team, primary Turn on, fields behind Details and prefilled', async () => {
  reset(offFound);
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  const card = v.host.querySelector('[data-dev-settings]');
  assert.equal(card.getAttribute('data-dev-off'), 'found');
  assert.match(card.className, /border-sky-500\/40/);
  assert.match(card.querySelector('[data-dev-gain]').textContent, /^Found demo-org\/demo-app in Dev team\. Turn it on and your bots review pull requests, show what landed and guard deploys\.$/);
  const on = card.querySelector('[data-dev-turn-on]');
  assert.match(on.className, /bg-sky-600/, 'primary');
  assert.equal(on.textContent, 'Turn on Dev mode');
  assert.equal(card.querySelector('#dv-set-repo'), null, 'fields are behind Details');
  const details = card.querySelector('[data-dev-details]');
  assert.equal(details.getAttribute('aria-expanded'), 'false');
  assert.equal(details.getAttribute('aria-controls'), 'dv-details');
  await click(details);
  assert.equal(details.getAttribute('aria-expanded'), 'true');
  assert.equal(card.querySelector('#dv-set-repo').value, 'demo-org/demo-app', 'prefilled from the found repo');
  assert.equal(card.querySelector('#dv-set-branch').value, 'main');
  assert.equal(on.getAttribute('type'), 'submit', 'Enter in a field turns it on');
  assert.equal(card.querySelectorAll('button[type="submit"]').length, 1, 'no Save while Off');
  await click(on);
  assert.deepEqual(puts(), [{ on: true }], 'untouched prefill: just on');
  assert.equal(m.devStore.isOn('spc_demo'), true);
  assert.equal(v.host.querySelector('[data-dev-settings]').hasAttribute('data-dev-off'), false, 'now the On form');
  v.root.unmount();
});

test('Off + repo found: a changed field in Details goes with Turn on', async () => {
  reset(offFound);
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await click(v.host.querySelector('[data-dev-details]'));
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  const b = v.host.querySelector('#dv-set-branch');
  set.call(b, 'release');
  b.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  await click(v.host.querySelector('[data-dev-turn-on]'));
  assert.deepEqual(puts(), [{ on: true, branch: 'release' }], 'only the changed field: the team repo stays watched as the team\'s');
  v.root.unmount();
});

test('Off + nothing found: quiet card, a value line, the repo field with help, primary Turn on, an empty repo is refused', async () => {
  reset({ ...off, repos: [] });
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  const card = v.host.querySelector('[data-dev-settings]');
  assert.equal(card.getAttribute('data-dev-off'), 'none');
  assert.match(card.className, /border-zinc-800/);
  assert.doesNotMatch(card.className, /border-sky/);
  assert.match(card.querySelector('[data-dev-gain]').textContent, /^Link your project's repo and your bots review pull requests, show what landed and guard deploys\.$/);
  assert.equal(card.querySelector('[data-dev-details]').getAttribute('aria-expanded'), 'false', 'Details holds branch and backup');
  assert.equal(card.querySelector('#dv-set-branch'), null);
  assert.ok(card.querySelector('#dv-set-repo'), 'the repo field is shown, not behind Details');
  assert.equal(card.querySelector('#dv-set-repo').value, '');
  assert.match(card.querySelector('#dv-set-repo-help').textContent, /Team settings › Git Repo/);
  const on = card.querySelector('[data-dev-turn-on]');
  assert.match(on.className, /bg-sky-600/);
  assert.equal(card.querySelectorAll('button[type="submit"]').length, 1, 'no Save while Off');
  await click(on);
  assert.deepEqual(puts(), [], 'nothing to read yet: not turned on');
  assert.match(card.querySelector('#dv-set-repo-err').textContent, /Add a repo first\./);
  assert.equal(document.activeElement, card.querySelector('#dv-set-repo'));
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  set.call(card.querySelector('#dv-set-repo'), 'demo-org/demo-app');
  card.querySelector('#dv-set-repo').dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  await click(on);
  assert.deepEqual(puts(), [{ on: true, repo: 'demo-org/demo-app' }]);
  v.root.unmount();
});

test('On: unchanged form (repo, branch, backup, Turn off, primary Save), no accent, no Details', async () => {
  reset({ ...off, on: true, repo: 'demo-org/demo-app', branch: 'main', polled_at: 'x', repos: [repo({ status: 'watching' })] });
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  const card = v.host.querySelector('[data-dev-settings]');
  assert.equal(card.hasAttribute('data-dev-off'), false);
  assert.doesNotMatch(card.className, /border-sky/);
  assert.equal(card.querySelector('[data-dev-details]'), null);
  assert.equal(card.querySelector('[data-dev-turn-on]'), null);
  assert.ok(card.querySelector('[data-dev-turn-off]'));
  assert.equal(card.querySelector('#dv-set-repo').value, 'demo-org/demo-app');
  assert.match(card.querySelector('button[type="submit"]').className, /bg-sky-600/);
  assert.match(card.textContent, /Demo reads this repo to show what landed, review PRs and guard deploys\./);
  v.root.unmount();
});

// ------------------------------------------------------------------ The suggestion

test('Suggestion: shows only in #general, Off, a repo found, not dismissed, no getting started, and a loaded state', () => {
  const s = state(offFound);
  const base = { inGeneral: true, state: s, dismissed: false, gettingStarted: false, canRun: true };
  assert.equal(m.suggestShows(base), true);
  assert.equal(m.suggestShows({ ...base, inGeneral: false }), false, 'other chats');
  assert.equal(m.suggestShows({ ...base, dismissed: true }), false, 'Not now');
  assert.equal(m.suggestShows({ ...base, gettingStarted: true }), false, 'getting started wins');
  assert.equal(m.suggestShows({ ...base, state: null }), false, 'loading, a failed GET or a 403');
  assert.equal(m.suggestShows({ ...base, state: state({ ...offFound, on: true }) }), false, 'Dev mode on');
  assert.equal(m.suggestShows({ ...base, state: state({ ...off, repos: [] }) }), false, 'no repo found');
  const app = src('App.tsx');
  assert.match(app, /suggestShows\(\{ inGeneral: isGeneral, state: devState, dismissed: suggestDismissed\(space\.id\), gettingStarted: !!setupBanner \|\| !!getStartedCard, canRun \}\)/);
  assert.match(app, /setupBanner \?\? \(\s*<>\s*\{devSuggest\}\s*\{devSuggestStatus\}/);
  assert.match(app, /data-dev-suggestion-status>\s*\{suggestHiddenIn === space\.id \? SUGGEST_HIDDEN : ''\}/, 'the live region is there before Not now fills it');
});

test('Round 2: no AI account (canRun false) or still checking (null) hides the suggestion', () => {
  const base = { inGeneral: true, state: state(offFound), dismissed: false, gettingStarted: false, canRun: true };
  assert.equal(m.suggestShows(base), true);
  assert.equal(m.suggestShows({ ...base, canRun: false }), false, 'no account: "Connect an AI account" is the one step');
  assert.equal(m.suggestShows({ ...base, canRun: null }), false, 'still checking');
});

test('Round 2: only no_git / no_repo repos: no suggestion, and the Off card is the quiet grey one', async () => {
  const dead = { ...off, repos: [repo({ status: 'no_git' }), repo({ key: 'x/y', repo: 'demo-org/no-url', status: 'no_repo', team: 'Ops', team_id: 't2' })] };
  const s = state(dead);
  assert.equal(s.repos.length, 2);
  assert.deepEqual(m.readableRepos(s), []);
  assert.equal(m.hasFoundCode(s), false);
  assert.equal(m.foundRepo(s), null);
  assert.equal(m.suggestShows({ inGeneral: true, state: s, dismissed: false, gettingStarted: false, canRun: true }), false);
  reset(dead);
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  const card = v.host.querySelector('[data-dev-settings]');
  assert.equal(card.getAttribute('data-dev-off'), 'none', 'quiet card');
  assert.doesNotMatch(card.className, /border-sky-500\/40/, 'no accent');
  assert.doesNotMatch(card.textContent, /Found /);
  v.root.unmount();
});

test('Round 2: one readable repo among no_git / no_repo ones: shown and named, "and N more" counts readable only', async () => {
  const mixed = {
    ...off,
    repos: [
      repo({ key: 'a', repo: 'demo-org/no-git', status: 'no_git' }),
      repo({ key: 'b', repo: 'demo-org/no-url', status: 'no_repo' }),
      repo({ key: 'c', repo: 'demo-org/web', status: 'watching', team: 'Web team', team_id: 't3' }),
    ],
  };
  const s = state(mixed);
  assert.equal(m.foundRepo(s).repo, 'demo-org/web');
  assert.equal(m.suggestShows({ inGeneral: true, state: s, dismissed: false, gettingStarted: false, canRun: true }), true);
  reset(mixed);
  const sug = await mount(h(m.DevModeSuggestion, { spaceName: 'Demo', state: s, onOpen() {}, onDismiss() {} }));
  const title = sug.host.querySelector('#dv-sug-title').textContent;
  assert.equal(title, 'Demo has code: demo-org/web', 'named, and no "and 2 more"');
  sug.root.unmount();
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  const card = v.host.querySelector('[data-dev-settings]');
  assert.equal(card.getAttribute('data-dev-off'), 'found');
  assert.match(card.querySelector('[data-dev-gain]').textContent, /^Found demo-org\/web in Web team\. Turn it on/);
  v.root.unmount();
});

test('Suggestion: a failed or 403 GET leaves nothing to show, and the state is asked for once per space', async () => {
  reset(offFound, 403);
  m.ensureDevState('spc_demo');
  m.ensureDevState('spc_demo');
  await settle();
  assert.equal(server.calls.length, 1, 'once, not per render');
  assert.equal(m.devStore.get('spc_demo'), null);
  assert.equal(m.suggestShows({ inGeneral: true, state: m.devStore.get('spc_demo'), dismissed: false, gettingStarted: false, canRun: true }), false);
  reset(offFound);
  m.ensureDevState('spc_demo');
  await settle();
  assert.equal(m.suggestShows({ inGeneral: true, state: m.devStore.get('spc_demo'), dismissed: false, gettingStarted: false, canRun: true }), true);
  m.ensureDevState('spc_demo');
  await settle();
  assert.equal(server.calls.length, 1, 'cached: no second GET');
});

test('Suggestion: names the space and repo; Not now persists for that space across a reload; other spaces unaffected', async () => {
  reset(offFound);
  let opened = 0;
  let hidden = 0;
  const v = await mount(h(m.DevModeSuggestion, { spaceName: 'Demo', state: state(offFound), onOpen: () => opened++, onDismiss: () => { m.dismissSuggest('spc_demo'); hidden++; } }));
  const el = v.host.querySelector('[data-dev-suggestion]');
  assert.equal(el.tagName, 'ASIDE');
  assert.equal(el.getAttribute('aria-labelledby'), 'dv-sug-title');
  assert.equal(el.getAttribute('role'), null, 'an offer, not announced on load');
  assert.match(el.textContent, /Demo has code: demo-org\/demo-app/);
  assert.match(el.textContent, /Dev mode lets your bots review pull requests, show what landed and guard deploys\./);
  const [notNow, turnOn] = [el.querySelector('[data-dev-suggestion-dismiss]'), el.querySelector('[data-dev-suggestion-open]')];
  assert.equal(notNow.textContent, 'Not now');
  assert.equal(turnOn.textContent, 'Turn on Dev mode');
  await click(turnOn);
  assert.equal(opened, 1);
  assert.deepEqual(puts(), [], 'Turn on here never switches Dev mode on by itself');
  await click(notNow);
  assert.equal(hidden, 1);
  assert.equal(localStorage.getItem('kwnt:dev-suggest:spc_demo'), 'dismissed');
  // A reload: a fresh copy of the module (no state in memory) reads the same storage.
  const fresh = await loadBundled(`export * from './src/devSuggest';`);
  assert.equal(fresh.suggestDismissed('spc_demo'), true);
  assert.equal(fresh.suggestDismissed('spc_other'), false, 'per space');
  assert.equal(fresh.suggestShows({ inGeneral: true, state: state(offFound), dismissed: fresh.suggestDismissed('spc_demo'), gettingStarted: false, canRun: true }), false);
  v.root.unmount();
});

test('Suggestion → Settings: Turn on opens General and focuses the Dev mode card (scrolled into view)', async () => {
  reset(offFound);
  // The card mounts after Settings opens: focusDevSettings waits for it.
  m.focusDevSettings(document, 20);
  const v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await settle();
  const card = v.host.querySelector('[data-dev-settings]');
  let scrolled = 0;
  card.scrollIntoView = () => scrolled++;
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(document.activeElement, card.querySelector('[data-dev-turn-on]'), 'focus lands on the card\'s Turn on');
  assert.ok(card.contains(document.activeElement));
  assert.equal(card.hasAttribute('data-dev-arrived'), true, 'outlined');
  assert.equal(card.getAttribute('tabindex'), '-1');
  assert.equal(card.getAttribute('aria-labelledby'), 'dv-title');
  assert.equal(scrolled, 1);
  assert.equal(card.getAttribute('data-dev-off'), 'found', 'the repo is there to turn on');
  const app = src('App.tsx');
  assert.match(app, /const openDevSuggest = \(\) => \{\s*openSpaceSettings\('general'\);\s*focusDevSettings\(\);/);
  v.root.unmount();
});
