// 0.7.3 any-git desk (#366; copy: Mira's dev-mode/any-git/copy-073.md): the Dev desk knows `no_git`
// and refused-address `no_repo` rows and the too-big error, reads plain-git repos (GitLab, Gitea, a folder
// with no remote) as watching "landed changes only" with a quiet tooltip line, and gives their cards no CI tile, no Post button and no forge link.
// `other_host` is retired but an older server still sends it. Demo data only: no real repos or tokens.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export * from './src/devMode';
  export { parseLanded } from './src/dev/types';
  export * as fx from './src/dev/fixtures';
  export { LandedCard } from './src/components/dev/LandedCard';
  export { DevStatusLine } from './src/components/DevStatusLine';
  export { Rollup } from './src/components/DevDeskSlots';
  export { parseLandedPage } from './src/dev/shellTypes';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));

const row = (over) => ({ team: null, team_id: null, from: 'folder', branch: 'main', status: 'watching', poll_error: null, polled_at: '2026-11-06T09:00:00Z', last_tag: null, added_by: 'team', ...over });
const APP = row({ provider: 'github', key: 'github.com/demo-org/app', repo: 'demo-org/app', host: 'github.com', team: 'App', team_id: 'tm_app' });
const LAB = row({ provider: 'git', key: 'gitlab.example.test/demo/lab', repo: 'https://gitlab.example.test/demo/lab', host: 'gitlab.example.test', team: 'Lab', team_id: 'tm_lab', branch: 'trunk' });
const NOTES = row({ provider: 'git', key: 'local:notes', repo: 'notes', host: null, team: 'Notes', team_id: 'tm_notes', from: 'local' });
const NO_GIT = row({ provider: 'git', key: 'gitea.example.test/demo/wiki', repo: 'https://gitea.example.test/demo/wiki', host: 'gitea.example.test', team: 'Wiki', team_id: 'tm_wiki', branch: null, status: 'no_git', polled_at: null, added_by: null });
const NO_FETCH = row({ provider: 'git', key: 'local:ops', repo: 'file:///srv/ops.git', host: null, team: 'Ops', team_id: 'tm_ops', branch: null, status: 'no_repo', polled_at: null, added_by: null });
const OLD_LAB = row({ ...LAB, provider: undefined, branch: null, status: 'other_host', polled_at: null, added_by: null });
const OLD_LOCAL = row({ ...NOTES, provider: undefined, branch: null, status: 'other_host', polled_at: null, added_by: null });
// A server before 6dd0243: no provider, the desk guesses from the host.
const noProvider = ({ provider, ...r }) => r;
const HINT = 'Install git 2.31 or newer on the server.';
// Mira's copy-073.md §1a–§1d.
const NO_GIT_TIP = 'Dev mode reads this repo with git. Install git 2.31 or newer on the server. Then press Refresh.';
const NO_GIT_TIP_OLD = 'Dev mode reads this repo with git. Install git 2.31 or newer on the server, then press Refresh.';
const GIT_TIP = 'Watching with git: shows what lands on trunk. Reviews and CI checks work on GitHub, GitLab, Gitea and Forgejo repos\.';
const LOCAL_TIP = 'Read from Notes’s folder on the server: shows what lands on main. It has no remote, so there are no reviews or CI checks.';
const CANT_USE_TIP = 'Dev mode only reads https, ssh or git addresses on another computer. This one points at this server, or isn’t a git address. Change it in Ops’s settings or in the folder’s git remote.';

const state = (over = {}) => m.parseDevMode({ on: true, repo: null, branch: null, sources: [], status: 'watching', forge: 'github', forge_login: 'kwant-bot', ...over });
const texts = (parts) => parts.map((p) => p.text);

test('parse: no_git and no_repo rows are kept, install_hint and poll_code are read, other_host still parses', () => {
  const s = state({ status: 'no_git', install_hint: HINT, repos: [NO_GIT, NO_FETCH, OLD_LAB, { ...APP, key: 'github.com/demo-org/x', status: 'bogus' }] });
  assert.equal(s.status, 'no_git');
  assert.equal(s.install_hint, HINT);
  assert.deepEqual(s.repos.map((r) => [r.key, r.status]), [
    ['gitea.example.test/demo/wiki', 'no_git'],
    ['local:ops', 'no_repo'],
    ['gitlab.example.test/demo/lab', 'other_host'],
  ]);
  assert.equal(state({ status: 'other_host', repos: [OLD_LAB] }).status, 'other_host');
  assert.equal(state({ repos: [APP] }).install_hint, null);
  assert.equal(state({ repos: [{ ...LAB, status: 'error', poll_code: 'too_big' }] }).repos[0].poll_code, 'too_big');
  assert.equal(state({ repos: [APP] }).repos[0].poll_code, null);
});

test('provider: the server’s value wins; else git for a local folder or any host but github.com', () => {
  const [app, lab, notes] = state({ repos: [APP, LAB, NOTES] }).repos;
  assert.deepEqual([app.provider, lab.provider, notes.provider], ['github', 'git', 'git']);
  assert.equal(state({ repos: [{ ...APP, provider: 'git' }] }).repos[0].provider, 'git');
  // Fallback for a server before 6dd0243 (no provider): the host guess.
  const [oApp, oLab, oNotes] = state({ repos: [APP, LAB, NOTES].map(noProvider) }).repos;
  assert.deepEqual([oApp.provider, oLab.provider, oNotes.provider], ['github', 'git', 'git']);
  assert.equal(state({ repos: [{ ...LAB, provider: 'gitlab' }] }).repos[0].provider, 'git', 'an unknown value falls back to the guess');
});

test('provider beats the host guess: a GitHub Enterprise host the server calls github keeps CI, Post and its link', () => {
  const GHE = row({ provider: 'github', key: 'ghe.example.test/demo/core', repo: 'demo/core', host: 'ghe.example.test', team: 'Core', team_id: 'tm_core' });
  const s = state({ repos: [GHE] });
  assert.equal(s.repos[0].provider, 'github');
  assert.equal(m.gitOnlyRepo(s, 'ghe.example.test/demo/core'), false);
  assert.equal(m.statusLine(s).note, null, 'no plain-git line');
  assert.deepEqual(texts(m.statusLine(s).parts), ['Watching ', 'demo/core', ' · ', 'main']);
  assert.equal(m.repoRows(state({ repos: [APP, GHE] }))[1].state, 'Watching · checked 09:00');
  // The same row from an older server is guessed as plain git.
  assert.ok(m.gitOnlyRepo(state({ repos: [noProvider(GHE)] }), 'ghe.example.test/demo/core'));
  // Rendered: what the desk passes to the card for this repo keeps the CI tile and Post.
  const card = { ...m.parseLanded(m.fx.LANDED_DIRECT), repo: 'ghe.example.test/demo/core' };
  const out = html(m.LandedCard, { card, forge: 'github', gitOnly: m.gitOnlyRepo(s, card.repo) });
  assert.match(out, /data-tests="passed"/);
  assert.match(out, /Open on GitHub/);
  assert.doesNotMatch(out, /data-git-only/);
});

test('watching (plain git): "Watching {repo} · {branch} · landed changes only", the tooltip line, no action', () => {
  const s = state({ repos: [LAB] });
  const l = m.statusLine(s);
  assert.equal(l.status, 'watching');
  assert.equal(l.led, 'ok');
  assert.deepEqual(texts(l.parts), ['Watching ', 'https://gitlab.example.test/demo/lab', ' · ', 'trunk', ' · landed changes only']);
  assert.equal(l.note, GIT_TIP);
  assert.equal(l.action, null);
  assert.ok(m.gitOnlyRepo(s, 'gitlab.example.test/demo/lab'));
  assert.ok(m.gitOnlyRepo(s, 'GitLab.example.test/demo/lab'), 'keys match case-blind');
  assert.deepEqual(texts(m.devSubLine(s).parts), ['Watching ', 'https://gitlab.example.test/demo/lab', ' · ', 'trunk']);
  assert.deepEqual(texts(m.onToast(s).body), ['Watching ', 'https://gitlab.example.test/demo/lab', ' on ', 'trunk', '. Landed changes only for now.']);
  assert.equal(m.onToast(s).title, 'Dev mode on');
  const out = html(m.DevStatusLine, { spaceId: 'spc_demo', state: s });
  assert.match(out, /data-dv-line-note="true">Watching with git: shows what lands on trunk\. Reviews and CI checks work on GitHub, GitLab, Gitea and Forgejo repos\.</);
  assert.match(out, /aria-describedby="[^"]+-note"/, 'the line is described by its note');
  assert.doesNotMatch(out, /Connect GitHub|Link a repo/);
  // No branch name (the remote's default): no branch anywhere, never "main".
  const bare = state({ repos: [{ ...LAB, branch: '' }] });
  assert.deepEqual(texts(m.statusLine(bare).parts), ['Watching ', 'https://gitlab.example.test/demo/lab', ' · landed changes only']);
  assert.deepEqual(texts(m.devSubLine(bare).parts), ['Watching ', 'https://gitlab.example.test/demo/lab']);
  assert.deepEqual(texts(m.onToast(bare).body), ['Watching ', 'https://gitlab.example.test/demo/lab', '. Landed changes only for now.']);
  assert.match(m.statusLine(bare).note, /shows what lands on its default branch\./);
});

test('watching (GitHub): no quiet line, cards keep CI and Post', () => {
  const s = state({ repos: [APP] });
  assert.equal(m.statusLine(s).note, null);
  assert.deepEqual(texts(m.statusLine(s).parts), ['Watching ', 'demo-org/app', ' · ', 'main']);
  assert.equal(m.gitOnlyRepo(s, 'github.com/demo-org/app'), false);
  assert.equal(m.gitOnlyRepo(s, null), false);
  assert.deepEqual(texts(m.onToast(s).body), ['Watching ', 'demo-org/app', ' on ', 'main', '.']);
  assert.doesNotMatch(html(m.DevStatusLine, { spaceId: 'spc_demo', state: s }), /data-dv-line-note/);
});

test('watching (a folder with no remote): read with git, "Landed changes only", counted as watched', () => {
  const s = state({ repos: [APP, NOTES] });
  const rows = m.repoRows(s);
  assert.equal(rows[1].state, 'Landed changes only · checked 09:00');
  assert.equal(rows[1].led, 'ok');
  assert.equal(rows[1].note, LOCAL_TIP);
  assert.equal(rows[1].origin, 'From Notes’s folder');
  assert.equal(rows[0].state, 'Watching · checked 09:00');
  assert.equal(rows[0].note, null);
  assert.equal(m.repoRows(state({ repos: [APP, { ...NOTES, polled_at: null }] }))[1].state, 'Landed changes only');
  assert.equal(m.reposSummary(s), 'Watching 2 repos');
  assert.equal(m.devSubLine(s).state, 'several');
  assert.deepEqual(m.repoFilters(s).map((f) => f.label), ['app', 'notes']);
  const out = html(m.DevStatusLine, { spaceId: 'spc_demo', state: s });
  assert.match(out, /data-dv-repo-note="true">Read from Notes’s folder on the server/);
  assert.match(out, /data-dv-repo-state="true" aria-describedby="([^"]+-note-1)">Landed changes only · checked 09:00<\/span>/);
  assert.doesNotMatch(out, /Local folder, no remote/);
  // The 0.7.1 shape (no repos[]): a local source is no longer turned into other_host.
  const legacy = m.parseDevMode({ on: true, status: 'watching', repo: 'notes', sources: [{ team: 'Notes', repo: 'notes', host: null, from: 'local' }] });
  assert.equal(legacy.repos[0].status, 'watching');
  assert.equal(legacy.repos[0].provider, 'git');
});

test('no_git: "Needs git on the server", the install_hint tooltip line, needs you, no button', () => {
  const s = state({ status: 'no_git', install_hint: HINT, repos: [NO_GIT] });
  const l = m.statusLine(s);
  assert.equal(l.status, 'no_git');
  assert.equal(l.led, 'warn');
  assert.deepEqual(texts(l.parts), ['https://gitea.example.test/demo/wiki', ' · Needs git on the server']);
  assert.equal(l.note, NO_GIT_TIP);
  assert.equal(l.action, null);
  assert.equal(m.needsYouCount(s), 1);
  assert.deepEqual([m.devSubLine(s).state, m.devSubLine(s).led, m.devSubLine(s).parts[0].text], ['no_git', 'warn', 'Needs git on the server']);
  const t = m.onToast(s);
  assert.equal(t.title, 'Dev mode on · guards are up');
  assert.deepEqual(texts(t.body), ['Needs git on the server to read ', 'https://gitea.example.test/demo/wiki', '. The floor works now.']);
  assert.equal(t.action, 'open');
  assert.deepEqual(m.repoFilters(state({ repos: [APP, NO_GIT] })), [], 'a repo Dev mode can’t read gets no filter chip');
  // The tooltip follows the server's hint (an early #366 sent it lower-case), and an older server's falls back to Mira's table.
  assert.equal(m.noGitTip('install git 2.40 or newer on the server'), 'Dev mode reads this repo with git. Install git 2.40 or newer on the server. Then press Refresh.');
  assert.equal(m.noGitTip(null), NO_GIT_TIP_OLD);
  assert.equal(m.statusLine(state({ status: 'no_git', repos: [NO_GIT] })).note, NO_GIT_TIP_OLD);
  // No rows at all: the line says it, the tooltip goes under it.
  const bare = m.statusLine(state({ status: 'no_git', install_hint: HINT, repos: [] }));
  assert.deepEqual([bare.status, bare.parts[0].text, bare.note, bare.action], ['no_git', 'Needs git on the server', NO_GIT_TIP, null]);
  // In a list: the row's state and its quiet line, next to a watched repo (partial, 1 needs you).
  const both = state({ status: 'watching', install_hint: HINT, repos: [APP, NO_GIT] });
  assert.equal(m.reposSummary(both), 'Watching 1 of 2 repos · 1 needs you');
  assert.equal(m.repoRows(both)[1].note, NO_GIT_TIP);
  const out = html(m.DevStatusLine, { spaceId: 'spc_demo', state: both });
  assert.match(out, />Needs git on the server<\/span>/);
  assert.match(out, /Dev mode reads this repo with git\. Install git 2\.31 or newer on the server\. Then press Refresh\./);
  assert.doesNotMatch(out, /Connect GitHub|Link a repo/, 'nothing in the app fixes it: no button');
});

test('no_repo row (a refused address): "Can’t use this repo address", needs you, Link a repo for its team', () => {
  const s = state({ status: 'no_repo', repos: [NO_FETCH] });
  const r = m.repoRows(s)[0];
  assert.deepEqual([r.state, r.led, r.action, r.note], ['Can’t use this repo address', 'warn', 'link', CANT_USE_TIP]);
  assert.equal(r.teamNote, 'Comes from Ops’s folder. Change it in the team’s settings.');
  const l = m.statusLine(s);
  assert.deepEqual([texts(l.parts), l.action, l.led], [['file:///srv/ops.git', ' · Can’t use this repo address'], 'link', 'warn']);
  assert.equal(m.needsYouCount(s), 1);
  assert.deepEqual([m.devSubLine(s).state, m.devSubLine(s).led, m.devSubLine(s).parts[0].text], ['no_address', 'warn', 'Can’t use the repo address']);
  const t = m.onToast(s);
  assert.deepEqual([t.title, texts(t.body), t.action], ['Dev mode on · guards are up', ['Can’t use ', 'file:///srv/ops.git', '’s address. The floor works now.'], 'open']);
  // The true "no repo anywhere" keeps the 0.7.2 copy.
  assert.equal(m.devSubLine(state({ status: 'no_repo', repos: [] })).parts[0].text, 'No repo yet');
  assert.match(texts(m.onToast(state({ status: 'no_repo', repos: [] })).body).join(''), /No git repo in this space’s folders yet/);
  // Sidebar order: no_git wins over a refused address.
  assert.equal(m.devSubLine(state({ status: 'no_git', repos: [NO_FETCH, NO_GIT] })).state, 'no_git');
  const out = html(m.DevStatusLine, { spaceId: 'spc_demo', state: s, teams: [{ id: 'tm_ops', name: 'Ops' }] });
  assert.match(out, /Can’t use this repo address/);
  assert.match(out, /Dev mode only reads https, ssh or git addresses/);
  assert.match(out, /Link a repo/);
});

test('too big (error): the row always reads "Too big to read"; the quiet line is the server\'s own words', () => {
  const CAP = 'This repo is too big for Dev mode to read (over 1 GB).';
  const ONE = 'This repo is too big for Dev mode to read.';
  // 679fe98: both too-big errors carry poll_code too_big. The 1 GB mirror cap:
  const cap = { ...LAB, status: 'error', poll_code: 'too_big', poll_error: CAP };
  const r = m.repoRows(state({ repos: [APP, cap] }))[1];
  assert.deepEqual([r.state, r.led, r.note, r.title], ['Too big to read', 'bad', CAP, CAP]);
  assert.deepEqual(texts(m.statusLine(state({ repos: [cap] })).parts), ['https://gitlab.example.test/demo/lab', ' · Too big to read']);
  assert.equal(m.statusLine(state({ repos: [cap] })).note, CAP);
  // A single git answer over 16 MB: same row, no size anywhere.
  const one = m.repoRows(state({ repos: [APP, { ...cap, poll_error: ONE }] }))[1];
  assert.deepEqual([one.state, one.note, one.title], ['Too big to read', ONE, ONE]);
  // Keyed off the code, not the words; empty words fall back to Mira's no-number sentence.
  assert.equal(m.repoRows(state({ repos: [APP, { ...cap, poll_error: 'Some other wording.' }] }))[1].state, 'Too big to read');
  for (const e of ['', '   ', null]) {
    const x = m.repoRows(state({ repos: [APP, { ...cap, poll_error: e }] }))[1];
    assert.deepEqual([x.state, x.note, x.title], ['Too big to read', ONE, ONE]);
  }
  // Fallback, older server with no code: the exact sentence, and the row text is still just "Too big to read".
  for (const e of [CAP, ONE]) {
    const old = m.repoRows(state({ repos: [APP, { ...LAB, status: 'error', poll_error: e }] }))[1];
    assert.deepEqual([old.state, old.note], ['Too big to read', e]);
  }
  // Nothing the desk writes in the row says "(over 1 GB)".
  for (const rows of [[APP, cap], [cap]]) {
    const s = state({ repos: rows });
    const out = html(m.DevStatusLine, { spaceId: 'spc_demo', state: s, teams: [] });
    assert.doesNotMatch([...m.repoRows(s).map((x) => x.state), ...texts(m.statusLine(s).parts)].join(' '), /over 1 GB/);
    assert.match(out, /Too big to read/);
  }
  // A code on a non-error row changes nothing.
  assert.equal(m.repoRows(state({ repos: [APP, { ...LAB, poll_code: 'too_big' }] }))[1].state, 'Landed changes only · checked 09:00');
  // Any other error keeps "Can't read it right now".
  assert.equal(m.repoRows(state({ repos: [APP, { ...LAB, status: 'error', poll_error: 'Couldn’t reach the repo’s host.' }] }))[1].state, 'Can’t read it right now. Trying again.');
  assert.equal(m.devSubLine(state({ repos: [cap] })).parts[0].text, 'Can’t read a repo');
});

test('other_host from an older server shows as before: "{host} isn’t read yet", a folder "Local folder, no remote"', () => {
  const s = state({ status: 'other_host', repos: [OLD_LAB] });
  assert.deepEqual(m.statusLine(s).parts.map((p) => p.text), ['https://gitlab.example.test/demo/lab', ' · gitlab.example.test isn’t read yet']);
  assert.equal(m.devSubLine(s).state, 'other_host');
  assert.equal(m.onToast(s).title, 'Dev mode on · guards are up');
  const local = state({ status: 'other_host', repos: [OLD_LOCAL] });
  assert.match(m.statusLine(local).parts.map((p) => p.text).join(''), /Local folder, no remote/);
  assert.equal(m.devSubLine(local).parts[0].text, 'Local folder, no remote');
  assert.equal(m.statusLine(local).note, null, 'no plain-git line: the older server isn’t reading it');
});

test('a plain-git repo’s landed card: no CI tile, no "Open on GitHub", no Diff link; GitHub cards keep them', () => {
  const card = { ...m.parseLanded(m.fx.LANDED_DIRECT), repo: 'gitlab.example.test/demo/lab' };
  const git = html(m.LandedCard, { card, forge: 'github', gitOnly: true });
  assert.match(git, /data-git-only="true"/);
  assert.doesNotMatch(git, /data-tests=/);
  assert.doesNotMatch(git, /Tests passed|see CI/);
  assert.doesNotMatch(git, /Open on GitHub|Open commit|>Diff</);
  assert.match(git, /Fix typo in README/, 'the card still says what landed');
  const gh = html(m.LandedCard, { card, forge: 'github' });
  assert.match(gh, /data-tests="passed"/);
  assert.match(gh, /Open on GitHub/);
  assert.doesNotMatch(gh, /data-git-only/);
});

test('main_green null (6dd0243: only git-only repos landed): no green or red chip, nothing faked; true and false still show', () => {
  const page = (main_green, repo) => m.parseLandedPage({
    cards: [{ ...m.fx.LANDED_DIRECT, repo }],
    rollup: { since: '2026-11-06T00:00:00Z', until: '2026-11-06T15:00:00Z', count: 1, needs_you: 0, main_green, areas: [{ area: 'Docs', count: 1, first_headline: 'Fix typo' }] },
    last_tag: null, last_merge: null,
  });
  const chip = (p) => html(m.Rollup, { rollup: p.rollup, last_merge: null, branch: 'trunk', cards: p.cards });
  const gitOnly = page(null, 'gitlab.example.test/demo/lab');
  assert.equal(gitOnly.rollup.main_green, null);
  const out = chip(gitOnly);
  assert.match(out, /data-dev-rollup=/);
  assert.doesNotMatch(out, /is green|is red|kw-chip-ok|kw-chip-bad/);
  assert.equal(m.parseLandedPage({ rollup: { count: 0, needs_you: 0, areas: [] } }).rollup.main_green, null, 'a missing field reads as null, not false');
  assert.match(chip(page(true, 'github.com/demo-org/app')), /trunk is green/);
  assert.match(chip(page(false, 'github.com/demo-org/app')), /trunk is red/);
});

test('no_git with 6dd0243’s full-sentence install_hint: one full stop, never two', () => {
  const tip = m.statusLine(state({ status: 'no_git', install_hint: 'Install git 2.31 or newer on the server.', repos: [NO_GIT] })).note;
  assert.equal(tip, NO_GIT_TIP);
  assert.doesNotMatch(tip, /\.\./);
  assert.equal(m.noGitTip('Install git 2.31 or newer on the server. '), NO_GIT_TIP);
});
