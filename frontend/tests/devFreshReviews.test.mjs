// 0.7.7 fresh reviews (dev-fresh-reviews/brief.md, "Desk line (Iris)"): Dev mode reviews only branches and pull
// requests with work in the last `fresh_days`. When the last poll skipped older ones, the Reviews tab says so in one
// quiet line, with no link and no setting. An older server sends neither field: no line. Demo data only.
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

// A GitLab repo read with plain git (Oleks's case), and a GitHub repo whose pull requests are reviewed.
const LAB = { key: 'gitlab.example.test/demo/homepage', repo: 'https://gitlab.example.test/demo/homepage', host: 'gitlab.example.test', team: 'Web', team_id: 'tm_web', from: 'repo', branch: 'master', status: 'watching', poll_error: null, polled_at: '2026-11-08T16:30:00Z', last_tag: null, added_by: 'team', provider: 'git', forge: 'gitlab', forge_connected: false };
const APP = { key: 'github.com/demo-org/app', repo: 'demo-org/app', host: 'github.com', team: 'App', team_id: 'tm_app', from: 'folder', branch: 'main', status: 'watching', poll_error: null, polled_at: '2026-11-08T16:30:00Z', last_tag: null, added_by: 'team', provider: 'github', forge: 'github', forge_connected: true };
const base = { on: true, repo: null, branch: null, deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null, sources: [], status: 'watching' };

let server;
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
  const path = u.split('?')[0];
  const repoQ = new URL(u, 'http://localhost').searchParams.get('repo');
  const of = (cards) => (repoQ ? cards.filter((c) => c.repo === repoQ) : cards);
  if (path.endsWith('/dev_mode')) return json(server.state);
  if (path.endsWith('/dev/landed')) return json({ cards: [], rollup: { count: 0, areas: [] }, last_tag: null, last_merge: null });
  if (path.endsWith('/dev/reviews')) return json({ cards: of(server.reviews) });
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/devMode';
  export { DevDeskView, deskStore } from './src/components/DevDesk';
  export * as fx from './src/dev/fixtures';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const state = (repos) => m.parseDevMode({ ...base, repos });
const line = (repos, pick = null) => m.freshReviewsLine(state(repos), pick);
const fresh = (row, skipped, days = 7) => ({ ...row, fresh_days: days, old_reviews_skipped: skipped });

// ---------------------------------------------------------------------------
// Parsing and the line

test('parseRepos: fresh_days and old_reviews_skipped are kept as whole counts; anything else reads as null', () => {
  const r = state([fresh(LAB, 24)]).repos[0];
  assert.deepEqual([r.fresh_days, r.old_reviews_skipped], [7, 24]);
  const older = state([LAB]).repos[0];
  assert.deepEqual([older.fresh_days, older.old_reviews_skipped], [null, null], 'an older server sends neither');
  for (const bad of ['24', -1, 2.5, NaN, null, true]) {
    assert.equal(state([{ ...LAB, fresh_days: 7, old_reviews_skipped: bad }]).repos[0].old_reviews_skipped, null, String(bad));
  }
  assert.deepEqual(m.parseDevMode({ ...base, repo: 'demo-org/app' }).repos.map((x) => [x.fresh_days, x.old_reviews_skipped]), [[null, null]], 'a 0.7.1 server (no repos[])');
});

test('freshReviewsLine: plain git says branches, a forge says pull requests, with the singular for 1', () => {
  assert.equal(line([fresh(LAB, 24)]), 'Reviewing branches with new commits in the last 7 days. 24 older ones skipped.');
  assert.equal(line([fresh(LAB, 1)]), 'Reviewing branches with new commits in the last 7 days. 1 older one skipped.');
  assert.equal(line([fresh(APP, 24)]), 'Reviewing pull requests updated in the last 7 days. 24 older ones skipped.');
  assert.equal(line([fresh(APP, 1)]), 'Reviewing pull requests updated in the last 7 days. 1 older one skipped.');
  assert.equal(line([fresh({ ...LAB, forge_connected: true }, 3)]), 'Reviewing pull requests updated in the last 7 days. 3 older ones skipped.', 'GitLab once connected');
  assert.equal(line([fresh(LAB, 2, 14)]), 'Reviewing branches with new commits in the last 14 days. 2 older ones skipped.', 'the server’s fresh_days, not 7');
  assert.equal(line([fresh(LAB, 2, 1)]), 'Reviewing branches with new commits in the last day. 2 older ones skipped.');
});

test('freshReviewsLine: no line for 0 skipped, an older server, or a missing fresh_days', () => {
  assert.equal(line([fresh(LAB, 0)]), null);
  assert.equal(line([LAB, APP]), null);
  assert.equal(line([{ ...LAB, old_reviews_skipped: 5 }]), null, 'no fresh_days: no number to say');
  assert.equal(line([fresh(LAB, 5, 0)]), null);
  assert.equal(m.freshReviewsLine(null), null);
});

test('freshReviewsLine: All adds the repos’ counts; a picked repo shows only its own', () => {
  const other = { ...LAB, key: 'gitlab.example.test/demo/site', repo: 'https://gitlab.example.test/demo/site' };
  assert.equal(line([fresh(LAB, 20), fresh(other, 4)]), 'Reviewing branches with new commits in the last 7 days. 24 older ones skipped.');
  assert.equal(line([fresh(LAB, 20), fresh(APP, 4)]), 'Reviewing branches and pull requests updated in the last 7 days. 24 older ones skipped.');
  assert.equal(line([fresh(LAB, 20), fresh(APP, 4)], APP.key), 'Reviewing pull requests updated in the last 7 days. 4 older ones skipped.');
  assert.equal(line([fresh(LAB, 20), fresh(APP, 0)], APP.key), null, 'the picked repo skipped none');
  assert.equal(line([fresh(LAB, 20), fresh(APP, 0)]), 'Reviewing branches with new commits in the last 7 days. 20 older ones skipped.', 'a repo with 0 adds nothing and sets no wording');
});

// ---------------------------------------------------------------------------
// The desk

const reset = (repos, reviews = []) => {
  server = { state: { ...base, repos }, reviews };
  m.devStore._reset();
  m.deskStore._reset();
  localStorage.clear();
  localStorage.setItem('kwnt.dev.intro.spc_demo', '1');
  document.body.innerHTML = '';
  m.devStore.set('spc_demo', m.parseDevMode(server.state));
};
const mount = async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', initialTab: 'reviews', now: m.fx.FIXTURE_NOW }));
  await settle();
  return root;
};
const shown = () => document.querySelector('[data-dev-fresh-line]')?.textContent ?? null;
const review = { ...m.fx.REVIEW_290, repo: LAB.key };

test('DOM desk: the Reviews tab shows the line above the cards, plain text, no link or button', async () => {
  reset([fresh(LAB, 24)], [review]);
  const root = await mount();
  assert.equal(shown(), 'Reviewing branches with new commits in the last 7 days. 24 older ones skipped.');
  const p = document.querySelector('[data-dev-fresh-line]');
  assert.equal(p.tagName, 'P');
  assert.equal(p.querySelectorAll('a, button, input').length, 0);
  assert.ok(p.closest('[role="tabpanel"]'), 'inside the Reviews panel');
  const card = document.querySelector('[data-dev-card="review"]');
  assert.ok(card, 'the review card still renders');
  assert.ok(p.compareDocumentPosition(card) & window.Node.DOCUMENT_POSITION_FOLLOWING, 'the line comes first');
  root.unmount();
});

test('DOM desk: with every review skipped the line sits above the empty state; the tab count is unchanged', async () => {
  reset([fresh(LAB, 1)]);
  const root = await mount();
  assert.equal(shown(), 'Reviewing branches with new commits in the last 7 days. 1 older one skipped.');
  assert.equal(document.querySelector('[data-dev-tab="reviews"] .kw-dv-tab-count'), null, 'no count from skipped ones');
  root.unmount();
});

test('DOM desk: no line with 0 skipped, from an older server, or on What landed', async () => {
  reset([fresh(LAB, 0)], [review]);
  let root = await mount();
  assert.equal(shown(), null);
  root.unmount();
  reset([LAB], [review]);
  root = await mount();
  assert.equal(shown(), null, 'older server');
  root.unmount();
  reset([fresh(LAB, 24)], [review]);
  root = await mount();
  document.querySelector('[data-dev-tab="landed"]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.equal(shown(), null, 'Reviews only');
  root.unmount();
});
