// 0.7.4 forges (Pixel, on Juno's G4/G5): the GitLab/Gitea/Forgejo form in Loose ends and in place from a
// repo row, the forge on repo rows and Team settings, GitLab/Gitea review cards and no_forge branch cards.
// Demo data only: hosts are *.example.test, tokens are made up, and nothing leaves this process.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLSelectElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const STORE = { store: 'local', locked: false, local_count: 0, bitwarden: null, approvals_off: false, exposed: false, risk_accepted: false };
let server;
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init = {}) => {
  const path = String(url).split('?')[0];
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  server.calls.push([method, path, body]);
  if (path === '/api/passwords/store') return json(STORE);
  if (path === '/api/zombie/sources' && method === 'POST') {
    const next = server.answers.shift() ?? { status: 201, body: { source: { id: 'src_f', kind: 'forge', label: body.label, enabled: true, config: body.config }, test: { login: 'demo-user', forge: 'gitlab' } } };
    return json(next.body, next.status);
  }
  if (/\/dev_mode$/.test(path)) return json({ on: false, repos: [] });
  return json({});
};

const m = await loadBundled(`
  export { createElement } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as le from './src/looseEnds';
  export { GitHubConnectForm, AddSourceForm } from './src/components/LooseEndsView';
  export { GITHUB_CONNECT_EVENT, openForgeConnect, connectTitle, connectedToast } from './src/components/GitHubConnect';
  export { TeamRepoDevLine, teamRepoLine, TEAM_REPO_LINES } from './src/components/TeamRepoDevLine';
  export { DevStatusLine } from './src/components/DevStatusLine';
  export { parseDevMode, devStore, repoRows, forgeFix, forgeLabel, landedOnly, gitOnlyRepo, gitTip } from './src/devMode';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { parseReview } from './src/dev/types';
  export { postState, laterForge, prRef, prName, NO_FORGE_LINE, postLaterLine } from './src/dev/logic';
  export * as fx from './src/dev/fixtures';
`);
const h = m.createElement;
const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounts = [];
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(el);
  await settle();
  mounts.push(root);
  return host;
};
const reset = (answers = []) => {
  while (mounts.length) mounts.pop().unmount();
  document.body.innerHTML = '';
  server = { calls: [], answers };
  m.devStore._reset();
};
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const typeIn = (el, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
};
const submit = async (host) => {
  host.querySelector('form[data-forge-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(30);
};
const posts = () => server.calls.filter(([meth, p]) => meth === 'POST' && p === '/api/zombie/sources').map(([, , b]) => b);
const flat = (el) => el.textContent.replace(/\s+/g, ' ').trim();

// ------------------------------------------------------------------ the request

test('forgeSource: https added, Forgejo sent as forgejo, label defaults to the host; bad addresses refused', () => {
  assert.deepEqual(m.le.forgeSource('', 'git.example.test', null, 'tok-demo'), { kind: 'forge', label: 'git.example.test', config: { url: 'https://git.example.test' }, secret: 'tok-demo' });
  assert.equal(m.le.forgeSource('', 'https://git.example.test/', 'forgejo', 't').config.forge, 'forgejo');
  assert.equal(m.le.forgeSource('Work', 'https://git.example.test', 'gitlab', 't').config.forge, 'gitlab');
  assert.equal(m.le.forgeSource('Work', 'https://git.example.test', 'gitlab', 't').label, 'Work');
  assert.ok('error' in m.le.forgeSource('', 'https://git.example.test', null, ''), 'needs a token');
  assert.ok('error' in m.le.forgeSource('', 'http://git.example.test', null, 't'), 'http only on loopback');
  assert.ok(!('error' in m.le.forgeSource('', 'http://127.0.0.1:9474', null, 't')));
  assert.ok('error' in m.le.forgeSource('', 'https://u:p@git.example.test', null, 't'), 'no userinfo');
  assert.ok('error' in m.le.forgeSource('', 'https://git.example.test/?x=1', null, 't'), 'no query');
  assert.equal(m.le.forgeHost('https://git.example.test:8443/'), 'git.example.test:8443');
});

test('forgeConnectedText and the error readers', () => {
  assert.equal(m.le.cantTellText('kwant couldn\'t tell whether h.test runs GitLab, Gitea or Forgejo: set "forge" to gitlab or gitea'), 'kwant couldn\'t tell whether h.test runs GitLab, Gitea or Forgejo.');
  assert.equal(m.le.cantTellText('Not sure which it is.'), 'Not sure which it is.');
  assert.equal(m.le.forgeConnectedText({ login: 'demo-user', forge: 'gitlab' }, 'git.example.test'), 'Connected to GitLab at git.example.test as demo-user.');
  assert.equal(m.le.forgeConnectedText({ login: 'demo-user', forge: 'gitea' }, 'code.example.test'), 'Connected to Gitea at code.example.test as demo-user.');
  assert.equal(m.le.forgeConnectedText({ login: 'demo-user', forge: 'forgejo' }, 'codeberg.org'), 'Connected to Forgejo at codeberg.org as demo-user.');
  assert.equal(m.le.forgeConnectedText(null, 'git.example.test'), 'Connected to git.example.test.');
  assert.equal(m.le.sourceKindText({ kind: 'forge', config: { url: 'https://code.example.test', forge: 'forgejo' } }), 'Forgejo · code.example.test');
  assert.equal(m.le.sourceKindText({ kind: 'forge', config: { url: 'https://code.example.test' } }), 'GitLab, Gitea or Forgejo · code.example.test');
  assert.equal(m.le.forgeTakenText('git.example.test'), 'git.example.test is already connected. To change its token, remove that connection and add it again.');
});

// ------------------------------------------------------------------ the form

const CANT = 'kwant couldn\'t tell whether git.example.test runs GitLab, Gitea or Forgejo: set "forge" to gitlab or gitea';

test('form DOM: no choice is sent at first; a can’t-tell 400 shows the server’s words, opens the choice and focuses it; the retry carries the pick', async () => {
  reset([{ status: 400, body: { error: CANT, code: 'forge_unknown' } }]);
  const added = [];
  const host = await mount(h(m.GitHubConnectForm, { forge: {}, heading: 'Add a source', onAdded: (s, t) => added.push([s, t]), onCancel() {} }));
  assert.equal(host.querySelector('[data-forge-choice]').dataset.forgeChoice, 'closed');
  assert.equal(host.querySelector('[data-api-base]'), null, 'no API base field for a forge');
  typeIn(host.querySelector('[data-forge-url]'), 'git.example.test');
  host.querySelector('[data-forge-token]').value = 'demo-not-a-token';
  await submit(host);
  assert.equal(posts()[0].config.forge, undefined, 'the server is asked first');
  const alert = host.querySelector('[data-forge-cant-tell]');
  assert.ok(alert, 'the server’s message shows');
  assert.match(flat(alert), /couldn.t tell whether git\.example\.test/);
  assert.match(flat(alert), /Forgejo\. Pick which it is, then try again\.$/);
  assert.doesNotMatch(flat(alert), /set "forge"/, 'no API advice on screen');
  assert.equal(host.querySelector('[data-forge-choice]').dataset.forgeChoice, 'open');
  await settle();
  assert.equal(document.activeElement?.getAttribute('role'), 'radio', 'the first choice has focus');
  await click(host.querySelector('[data-forge-pick="forgejo"]'));
  host.querySelector('[data-forge-token]').value = 'demo-not-a-token';
  await submit(host);
  assert.equal(posts()[1].config.forge, 'forgejo', 'Forgejo goes as forgejo (0.7.4, fa7eed5)');
  assert.equal(host.querySelector('[data-forge-cant-tell]'), null, 'cleared on retry');
  assert.equal(added.length, 1);
  assert.deepEqual(added[0][1], { login: 'demo-user', forge: 'gitlab' }, 'the test reaches the caller');
  assert.equal(host.querySelector('[data-forge-token]')?.value ?? '', '', 'the token is never shown again');
});

test('cantTellForge keys on code forge_unknown, not on the sentence', () => {
  const E = m.le.LooseEndsError;
  assert.equal(m.le.cantTellForge(new E('Anything at all', 400, false, false, 'forge_unknown')), true, 'the code decides, whatever the words');
  assert.equal(m.le.cantTellForge(new E(CANT, 400)), false, 'the same sentence with no code is a plain error');
  assert.equal(m.le.cantTellForge(new E('GitHub is its own connection: add it as GitHub', 400)), false, 'the GitHub refusal has no code');
  assert.equal(m.le.cantTellForge(new E(CANT, 409, false, false, 'forge_unknown')), false, 'only a 400');
});

test('form DOM: a 400 with the can’t-tell sentence but no code stays a plain error, and the choice stays folded', async () => {
  reset([{ status: 400, body: { error: CANT } }]);
  const host = await mount(h(m.GitHubConnectForm, { forge: {}, onAdded() {}, onCancel() {} }));
  typeIn(host.querySelector('[data-forge-url]'), 'git.example.test');
  host.querySelector('[data-forge-token]').value = 'demo-not-a-token';
  await submit(host);
  assert.equal(host.querySelector('[data-forge-cant-tell]'), null);
  assert.equal(host.querySelector('[data-forge-choice]').dataset.forgeChoice, 'closed');
  assert.ok([...host.querySelectorAll('[role="alert"]')].some((a) => /couldn.t tell/.test(flat(a))), 'the server’s words still show');
});

test('form DOM: a 409 says the host is already connected, plainly', async () => {
  reset([{ status: 409, body: { error: 'git.example.test already has a connection: change that one' } }]);
  const host = await mount(h(m.GitHubConnectForm, { forge: { url: 'https://git.example.test' }, onAdded() {}, onCancel() {} }));
  assert.equal(host.querySelector('[data-forge-url]').value, 'https://git.example.test', 'prefilled from the row');
  host.querySelector('[data-forge-token]').value = 'demo-not-a-token';
  await submit(host);
  const err = [...host.querySelectorAll('[role="alert"]')].map(flat).join(' ');
  assert.match(err, /git\.example\.test is already connected\. To change its token, remove that connection and add it again\./);
  assert.equal(host.querySelector('[data-needs-bitwarden]'), null);
});

test('form DOM: Add a source offers GitLab or Gitea next to GitHub and email', async () => {
  reset();
  const host = await mount(h(m.AddSourceForm, { onAdded() {}, onCancel() {} }));
  const picks = [...host.querySelectorAll('[role="radio"], [data-kind]')].map(flat).join(' | ');
  assert.match(picks, /GitLab, Gitea or Forgejo/);
});

// ------------------------------------------------------------------ connect in place

test('connectTitle and the toast', () => {
  assert.equal(m.connectTitle({ forge: null }), 'Connect GitHub');
  assert.equal(m.connectTitle({ forge: { forge: 'gitlab', url: 'https://git.example.test' } }), 'Connect GitLab');
  assert.equal(m.connectTitle({ forge: { forge: 'gitea', url: 'https://code.example.test' } }), 'Connect Gitea');
  const t = m.connectedToast({ id: 's', kind: 'forge', label: 'git.example.test', enabled: true, config: { url: 'https://git.example.test' } }, { login: 'demo-user', forge: 'gitlab' });
  assert.match(`${t.title} ${t.body}`, /GitLab/);
  assert.match(`${t.title} ${t.body}`, /demo-user/);
});

const dev = (over) => m.parseDevMode({ on: true, forge: 'github', forge_login: 'demo', status: 'watching', repos: [], ...over });
const glRow = (o) => ({ key: 'git.example.test/demo/app', repo: 'demo/app', host: 'git.example.test', team: 'Web', team_id: 'team_web', from: 'repo', provider: 'git', branch: 'main', status: 'watching', forge: 'gitlab', forge_connected: false, ...o });

test('repo rows: the forge, and Connect GitLab for an unconnected GitLab host only', () => {
  const s = dev({ repos: [glRow(), glRow({ key: 'code.example.test/demo/lib', repo: 'demo/lib', host: 'code.example.test', forge: 'gitea', forge_connected: true, team: 'Lib', team_id: 'team_lib' }), { key: 'git.plain.test/demo/x', repo: 'demo/x', host: 'git.plain.test', team: 'X', team_id: 'team_x', from: 'repo', provider: 'git', branch: 'main', status: 'watching' }] });
  const rows = m.repoRows(s);
  const by = (r) => rows.find((x) => x.repo?.repo === r || x.name === r || x.label === r) ?? rows.find((x) => JSON.stringify(x).includes(r));
  assert.equal(by('demo/app').forgeName, 'GitLab');
  assert.deepEqual(by('demo/app').fix, { forge: 'gitlab', url: 'https://git.example.test', label: 'Connect GitLab' });
  assert.equal(by('demo/lib').forgeName, 'Gitea');
  assert.equal(by('demo/lib').fix, null, 'connected: no button');
  assert.equal(by('demo/x').forgeName, 'Plain git');
  assert.equal(by('demo/x').fix, null);
  assert.equal(m.landedOnly(s.repos[0]), true);
  assert.equal(m.landedOnly(s.repos[1]), false, 'a connected Gitea repo gets reviews');
  const fj = glRow({ key: 'codeberg.org/demo/fj', repo: 'demo/fj', host: 'codeberg.org', forge: 'forgejo' });
  assert.equal(m.forgeLabel(fj), 'Forgejo');
  assert.deepEqual(m.forgeFix(fj), { forge: 'forgejo', url: 'https://codeberg.org', label: 'Connect Forgejo' });
  assert.match(m.gitTip(fj), /Connect Forgejo for reviews and CI checks\./);
  assert.equal(m.parseDevMode({ on: true, repos: [fj] }).repos[0].forge, 'forgejo');
  assert.equal(m.gitOnlyRepo(s, 'code.example.test/demo/lib'), false);
  assert.match(m.gitTip(s.repos[0]), /Connect GitLab for reviews and CI checks\./);
  assert.match(m.gitTip(s.repos[2]), /work on GitHub, GitLab, Gitea and Forgejo repos\./);
  assert.equal(m.forgeFix(glRow({ status: 'not_connected' })), null, 'only a watched row');
});

test('repo rows DOM: the chip shows, and Connect GitLab opens the form in place with the host', async () => {
  reset();
  const s = dev({ repos: [glRow(), glRow({ key: 'git.example.test/demo/two', repo: 'demo/two', team: 'Two', team_id: 'team_two' })] });
  const host = await mount(h(m.DevStatusLine, { spaceId: 'spc_demo', state: s }));
  const opener = host.querySelector('[data-dv-status-fix], [data-dv-row-fix]') ?? (await (async () => {
    const more = host.querySelector('[aria-expanded]');
    if (more) await click(more);
    return document.querySelector('[data-dv-row-fix]');
  })());
  assert.ok(document.querySelector('[data-dv-repo-forge]'), 'the forge chip');
  assert.match(flat(document.querySelector('[data-dv-repo-forge]')), /GitLab/);
  assert.ok(opener, 'a Connect GitLab button');
  assert.equal(opener.getAttribute('aria-haspopup'), 'dialog');
  assert.match(flat(opener), /Connect GitLab/);
  const got = [];
  const on = (e) => got.push(e.detail);
  window.addEventListener(m.GITHUB_CONNECT_EVENT, on);
  await click(opener);
  window.removeEventListener(m.GITHUB_CONNECT_EVENT, on);
  assert.equal(got.length, 1);
  assert.equal(got[0].space, 'spc_demo');
  assert.deepEqual(got[0].forge, { forge: 'gitlab', url: 'https://git.example.test' });
});

test('Team settings: the line reads right for GitLab and Gitea', async () => {
  const url = 'https://git.example.test/demo/app.git';
  assert.equal(m.teamRepoLine(dev({ repos: [glRow()] }), url, 'team_web'), 'forge_connect');
  assert.equal(m.teamRepoLine(dev({ repos: [glRow({ forge_connected: true })] }), url, 'team_web'), 'forge_connected');
  assert.equal(m.TEAM_REPO_LINES.git, 'Dev mode reads this repo with git. Reviews need GitHub, GitLab, Gitea or Forgejo.');
  reset();
  m.devStore.set('spc_demo', dev({ repos: [glRow()] }));
  const host = await mount(h(m.TeamRepoDevLine, { url, teamId: 'team_web', spaceId: 'spc_demo' }));
  const p = host.querySelector('[data-team-repo-dev]');
  assert.equal(flat(p).startsWith('Dev mode watches this repo. Connect GitLab for reviews.'), true);
  const btn = p.querySelector('[data-team-repo-connect="gitlab"]');
  assert.ok(btn);
  const got = [];
  const on = (e) => got.push(e.detail);
  window.addEventListener(m.GITHUB_CONNECT_EVENT, on);
  await click(btn);
  window.removeEventListener(m.GITHUB_CONNECT_EVENT, on);
  assert.equal(got[0]?.forge?.forge, 'gitlab');
  m.devStore.set('spc_demo', dev({ repos: [glRow({ forge: 'gitea', forge_connected: true })] }));
  await settle();
  assert.equal(flat(host.querySelector('[data-team-repo-dev]')), 'Dev mode watches this repo · Gitea connected');
  m.devStore.set('spc_demo', dev({ repos: [glRow({ forge: 'forgejo', forge_connected: false })] }));
  await settle();
  assert.equal(flat(host.querySelector('[data-team-repo-dev]')).startsWith('Dev mode watches this repo. Connect Forgejo for reviews.'), true);
  assert.ok(host.querySelector('[data-team-repo-connect="forgejo"]'), 'Connect Forgejo in place');
  m.devStore.set('spc_demo', dev({ repos: [glRow({ forge: 'forgejo', forge_connected: true })] }));
  await settle();
  assert.equal(flat(host.querySelector('[data-team-repo-dev]')), 'Dev mode watches this repo · Forgejo connected');
});

// ------------------------------------------------------------------ cards

const review = (over) => ({ ...m.parseReview(m.fx.REVIEW_294), ...over });
const cardProps = (card) => ({ card, onFinding: async () => card, onPost: async () => card, forge: 'github', forgeLogin: 'demo', now: m.fx.FIXTURE_NOW });

test('cards: GitLab says MR !7; Post waits for a later update', () => {
  assert.equal(m.prRef('gitlab', 7), '!7');
  assert.equal(m.prName('gitlab', 7), 'MR !7');
  assert.equal(m.prRef('gitea', 7), '#7');
  assert.equal(m.prRef('forgejo', 7), '#7');
  assert.equal(m.laterForge({ forge: 'forgejo', no_forge: false }), 'forgejo');
  assert.equal(m.postLaterLine('forgejo'), 'Posting to Forgejo comes in a later update.');
  assert.equal(m.connectTitle({ forge: { forge: 'forgejo', url: 'https://codeberg.org' } }), 'Connect Forgejo');
  assert.equal(m.parseReview({ ...m.fx.REVIEW_294, forge: 'forgejo' }).forge, 'forgejo');
  assert.equal(m.laterForge({ forge: 'gitlab', no_forge: false }), 'gitlab');
  assert.equal(m.laterForge({ forge: 'gitea', no_forge: true }), null);
  assert.equal(m.laterForge({ forge: null, no_forge: false }), null);
  assert.equal(m.postLaterLine('gitlab'), 'Posting to GitLab comes in a later update.');
  const raw = m.parseReview({ ...m.fx.REVIEW_294, forge: 'gitlab', no_forge: false, pr: 7 });
  assert.equal(raw.forge, 'gitlab');
  assert.equal(raw.no_forge, false);
});

test('cards DOM: a GitLab card reads MR !7 and its footer says later, with no Post button', async () => {
  reset();
  const card = review({ forge: 'gitlab', no_forge: false, pr: 7 });
  const host = await mount(h(m.ReviewCard, cardProps(card)));
  const h3 = host.querySelector('h3');
  assert.match(flat(h3), /^MR !7 /, 'screen readers hear MR');
  assert.match(h3.querySelector('[aria-hidden="true"]').textContent, /^!7$/);
  const foot = host.querySelector('[data-review-footer]');
  assert.equal(foot.dataset.postState, 'later');
  assert.match(flat(foot), /Posting to GitLab comes in a later update\./);
  assert.equal([...foot.querySelectorAll('button')].some((b) => /Post to/.test(b.textContent)), false, 'not a dead button');
});

test('cards DOM: a Gitea card is #7 with the same later line; a GitHub card is unchanged', async () => {
  reset();
  let host = await mount(h(m.ReviewCard, cardProps(review({ forge: 'gitea', no_forge: false, pr: 7 }))));
  assert.match(flat(host.querySelector('h3')), /^#7 /);
  assert.match(flat(host.querySelector('[data-review-footer]')), /Posting to Gitea comes in a later update\./);
  reset();
  host = await mount(h(m.ReviewCard, cardProps(review({ forge: 'github', no_forge: false }))));
  assert.match(flat(host.querySelector('h3')), /^#294 /);
  assert.notEqual(host.querySelector('[data-review-footer]').dataset.postState, 'later');
});

test('cards DOM: a no_forge card reads as a pushed branch: no PR number, no Post, one short line', async () => {
  reset();
  const card = review({ forge: null, no_forge: true, pr: null, branch: 'demo-branch' });
  const host = await mount(h(m.ReviewCard, cardProps(card)));
  const art = host.firstElementChild;
  assert.doesNotMatch(flat(host.querySelector('h3')), /#294|PR #|MR !/);
  const foot = host.querySelector('[data-review-footer]');
  assert.equal(foot.dataset.postState, 'no_forge');
  assert.match(flat(foot), new RegExp(m.NO_FORGE_LINE.replace(/[.:]/g, '\\$&')));
  assert.equal([...host.querySelectorAll('button')].some((b) => /Post to/.test(b.textContent)), false);
  assert.ok(art.querySelector('[data-no-forge]') || art.matches('[data-no-forge]'));
});
