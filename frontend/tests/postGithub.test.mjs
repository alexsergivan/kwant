// Post to GitHub (0.7.1, Iris): the review card footer in every state of plan/post-to-github.md, the
// parse of `posted` / `post_error`, the copy changes, and the desk call. Mock data only: nothing posts.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export * from './src/dev/types';
  export * from './src/dev/logic';
  export * as fx from './src/dev/fixtures';
  export { LandedCard, LandedList } from './src/components/dev/LandedCard';
  export { Rollup, rollupTitle, rollupTiles } from './src/components/dev/Rollup';
  export { ReviewCard } from './src/components/dev/ReviewCard';
  export { ReleaseNotesSheet } from './src/components/dev/ReleaseNotesSheet';
  export { ProdFloorCard } from './src/components/dev/ProdFloorCard';
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));
const NOW = fx.FIXTURE_NOW;

const landed = (raw, over = {}) => ({ ...m.parseLanded(raw), ...over });
const review = (raw, over = {}) => ({ ...m.parseReview(raw), ...over });

// ---------------------------------------------------------------------------------------------------
// DOM helpers

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(C, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const button = (host, text) => [...host.querySelectorAll('button')].find((b) => b.textContent.replace(/\s+/g, ' ').trim().startsWith(text));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

const base = () => review(fx.REVIEW_294);
const LOGIN = 'oleks-i';
const footer = (host) => host.querySelector('[data-review-footer]');
const flat = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const POSTED = { forge: 'github', at: '2026-11-05T19:58:00Z', login: LOGIN, url: 'https://github.com/o/r/pull/294#pullrequestreview-1', comments: 3 };

// ---------------------------------------------------------------------------------------------------
// Parse

test('parseReview keeps posted and post_error, https links only, unknown kinds as failed', () => {
  const c = m.parseReview({ ...fx.REVIEW_294, posted: POSTED, post_error: null });
  assert.deepEqual(c.posted, POSTED);
  assert.equal(c.post_error, null);
  assert.equal(m.parseReview(fx.REVIEW_294).posted, null, 'a 0.7.0 card without the field');
  assert.equal(m.parseReview({ ...fx.REVIEW_294, posted: { ...POSTED, url: 'javascript:alert(1)' } }).posted.url, null);
  assert.equal(m.parseReview({ ...fx.REVIEW_294, posted: { ...POSTED, url: 'http://github.com/x' } }).posted.url, null);
  assert.deepEqual(m.parseReview({ ...fx.REVIEW_294, post_error: { kind: 'read_only', message: 'Resource not accessible' } }).post_error, { kind: 'read_only', message: 'Resource not accessible' });
  assert.equal(m.parseReview({ ...fx.REVIEW_294, post_error: { kind: 'weird' } }).post_error.kind, 'failed');
  assert.equal(m.parseReview({ ...fx.REVIEW_294, post_error: 'nope' }).post_error, null);
});

test('the dev_mode parse reads forge and forge_login; unknown forge and a stray github_login are ignored', () => {
  // devMode.ts is bundled separately: it imports auth.
  return loadBundled(`export { parseDevMode, emptyState } from './src/devMode';`).then(({ parseDevMode, emptyState }) => {
    const d = parseDevMode({ on: true, forge: 'github', forge_login: 'octocat' });
    assert.equal(d.forge, 'github');
    assert.equal(d.forge_login, 'octocat');
    assert.equal(parseDevMode({ on: true, forge: 'gitlab' }).forge, 'gitlab');
    assert.equal(parseDevMode({ on: true, forge: 'gitea' }).forge, 'gitea');
    assert.equal(parseDevMode({ on: true, forge: 'forgejo' }).forge, 'forgejo', '0.7.4: Forgejo is its own forge');
    assert.equal(parseDevMode({ on: true, forge: 'bitbucket' }).forge, null);
    assert.equal(parseDevMode({ on: true }).forge, null);
    assert.equal(parseDevMode({ on: true }).forge_login, null);
    assert.equal(parseDevMode({ on: true, forge_login: '' }).forge_login, null);
    const old = parseDevMode({ on: true, github_login: 'octocat' });
    assert.equal(old.forge_login, null, 'github_login is gone from the contract');
    assert.ok(!('github_login' in old));
    assert.equal(emptyState().forge, null);
    assert.equal(emptyState().forge_login, null);
  });
});

// ---------------------------------------------------------------------------------------------------
// The rules

test('N counts every finding not dismissed; the label pluralises and drops a zero count', () => {
  assert.equal(m.postCount([{ state: 'open' }, { state: 'sent' }, { state: 'dismissed' }]), 2);
  assert.equal(m.postLabel(3, 'github'), 'Post to GitHub · 3 findings');
  assert.equal(m.postLabel(1, 'github'), 'Post to GitHub · 1 finding');
  assert.equal(m.postLabel(0, 'github'), 'Post to GitHub');
  assert.equal(m.postLine('octocat'), 'Nothing is posted until you press it. Posts as @octocat.');
  assert.equal(m.postLine(null), 'Nothing is posted until you press it.');
});

test('postState: posted wins, then posting, then the error; pending and the sample have no button', () => {
  const c = (o = {}) => ({ status: 'ready', posted: null, post_error: null, ...o });
  const on = { canPost: true, posting: false, forge: 'github' };
  assert.equal(m.postState(c(), on).kind, 'ready');
  assert.equal(m.postState(c(), { canPost: true, posting: true, forge: 'github' }).kind, 'posting');
  assert.equal(m.postState(c({ posted: POSTED, post_error: { kind: 'failed', message: 'x' } }), { canPost: true, posting: true, forge: 'github' }).kind, 'posted');
  assert.equal(m.postState(c({ posted: POSTED }), { canPost: false, posting: false, forge: 'github' }).kind, 'posted', 'a posted card says so even without the call');
  assert.equal(m.postState(c({ post_error: { kind: 'read_only', message: null } }), on).kind, 'read_only');
  assert.equal(m.postState(c({ post_error: { kind: 'stale', message: null } }), on).kind, 'stale');
  assert.deepEqual(m.postState(c({ post_error: { kind: 'failed', message: 'GitHub said 502' } }), on), { kind: 'failed', message: 'GitHub said 502', retry: true });
  assert.deepEqual(m.postState(c({ post_error: { kind: 'failed', message: 'no review' } }), { ...on, noReview: true }), { kind: 'failed', message: 'no review', retry: false }, 'F4: a 409 is final');
  assert.equal(m.postState(c({ status: 'failed' }), on).kind, 'no_review', 'F4: a review that failed to generate has nothing to post');
  assert.equal(m.postState(c({ status: 'failed', posted: POSTED }), on).kind, 'posted');
  assert.equal(m.postState(c({ status: 'pending' }), on).kind, 'none');
  assert.equal(m.postState(c(), { canPost: false, posting: false, forge: 'github' }).kind, 'none');
});

// ---------------------------------------------------------------------------------------------------
// Every footer state, rendered

test('ready: the line with @login in mono, primary Post to GitHub · N comments, Open on GitHub', async () => {
  const card = { ...base(), findings: [...base().findings, { ...base().findings[0], id: 'f9', state: 'dismissed' }] };
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge: 'github', forgeLogin: LOGIN, now: NOW });
  const f = footer(host);
  assert.equal(f.dataset.postState, 'ready');
  assert.match(flat(f), /Nothing is posted until you press it\. Posts as @oleks-i\./);
  assert.match(f.innerHTML, /<span class="font-mono text-zinc-100">@oleks-i<\/span>/, 'login in mono');
  assert.equal(!!f.querySelector('svg.lucide-lock'), false, 'v2: no lock icon');
  const b = button(f, 'Post to GitHub');
  assert.equal(flat(b), 'Post to GitHub · 3 findings', 'the dismissed fourth finding is not counted');
  assert.equal(b.disabled, false);
  assert.match(b.className, /kw-btn-primary/, 'the primary button');
  assert.match(f.innerHTML, /Open on GitHub/);
  assert.ok(/min-h-\[44px\]/.test(b.className) && /w-full/.test(b.className) && /sm:w-auto/.test(b.className), 'phone: full-width 44 px target under the text');
});

test('ready without a login: no "Posts as"', async () => {
  const host = await mount(m.ReviewCard, { card: base(), onFinding: async () => base(), onPost: async () => base(), forge: 'github', forgeLogin: null, now: NOW });
  assert.match(flat(footer(host)), /Nothing is posted until you press it\./);
  assert.doesNotMatch(flat(footer(host)), /Posts as/);
});

test('posting: spinner, button disabled and busy, a second press sends nothing; then posted from the reply', async () => {
  let release;
  const calls = [];
  const onPost = (id) => {
    calls.push(id);
    return new Promise((r) => (release = () => r({ ...base(), posted: POSTED })));
  };
  const host = await mount(m.ReviewCard, { card: base(), onFinding: async () => base(), onPost, forge: 'github', forgeLogin: LOGIN, now: NOW });
  await click(button(footer(host), 'Post to GitHub'));
  let f = footer(host);
  assert.equal(f.dataset.postState, 'posting');
  const b = f.querySelector('[data-post-button]');
  assert.equal(flat(b), 'Posting…');
  assert.equal(b.disabled, true);
  assert.equal(b.getAttribute('aria-busy'), 'true');
  assert.ok(b.querySelector('svg.lucide-loader-circle, svg.lucide-loader2, svg.lucide-loader'), 'spinner');
  assert.match(b.querySelector('svg').getAttribute('class'), /motion-safe:animate-spin/, 'static under reduced motion');
  await click(b);
  assert.deepEqual(calls, ['dvr_294'], 'one request only');
  release();
  await settle();
  f = footer(host);
  assert.equal(f.dataset.postState, 'posted');
  assert.match(flat(f), /^Posted 19:58 as @oleks-i · 3 inline comments\s*View on GitHub/);
  assert.equal(f.querySelector('.dv-led').dataset.s, 'ok', 'ok LED');
  const a = f.querySelector('a[href="https://github.com/o/r/pull/294#pullrequestreview-1"]');
  assert.ok(a, 'View on GitHub links to the review');
  assert.equal(a.getAttribute('target'), '_blank');
  assert.equal(!!button(f, 'Post to GitHub'), false, 'no button once posted');
  assert.equal(f.querySelector('[role="status"]').textContent, 'Posted 19:58 as @oleks-i · 3 inline comments');
});

test('posted from the server (SSE refetch): the card prop alone switches the footer', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const props = { onFinding: async () => base(), onPost: async () => base(), forge: 'github', forgeLogin: LOGIN, now: NOW };
  root.render(h(m.ReviewCard, { ...props, card: base() }));
  await settle();
  assert.equal(footer(host).dataset.postState, 'ready');
  root.render(h(m.ReviewCard, { ...props, card: { ...base(), posted: POSTED } }));
  await settle();
  assert.equal(footer(host).dataset.postState, 'posted');
  root.unmount();
});

test('posted without a url: "Posted 19:58" and no link', () => {
  const out = html(m.ReviewCard, { card: { ...base(), posted: { ...POSTED, url: null } }, onFinding: async () => base(), onPost: async () => base(), now: NOW });
  assert.match(out, /Posted 19:58/);
  assert.doesNotMatch(out, /View on GitHub/);
});

test('read_only (F1): warn LED line naming the scope, and Try again, which reaches the server once the token is fixed', async () => {
  const calls = [];
  const card = { ...base(), post_error: { kind: 'read_only', message: 'Resource not accessible by personal access token' } };
  const onPost = async (id) => (calls.push(id), { ...base(), posted: POSTED, post_error: null });
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost, forge: 'github', forgeLogin: LOGIN, now: NOW });
  let f = footer(host);
  assert.equal(f.dataset.postState, 'read_only');
  const b = button(f, 'Try again');
  assert.ok(b, 'Try again');
  assert.equal(b.getAttribute('aria-disabled'), null, 'not sticky: the press is live');
  assert.equal(!!f.querySelector('[data-post-disabled]'), false);
  const why = f.querySelector(`#${b.getAttribute('aria-describedby').replace(/([:.])/g, '\\$1')}`);
  assert.equal(flat(why), m.readOnlyLine('github'), 'screen readers hear why');
  assert.equal(flat(why), 'Your GitHub connection is read only. Give the token pull_requests: write to post.');
  assert.match(why.innerHTML, /<code class="dv-code">pull_requests: write<\/code>/);
  assert.equal(f.querySelector('.dv-led').dataset.s, 'warn');
  await click(b);
  assert.deepEqual(calls, ['dvr_294'], 'the press reaches the API');
  f = footer(host);
  assert.equal(f.dataset.postState, 'posted');
});

test('read_only that is still read only: the press comes back read_only, the line stays', async () => {
  const card = { ...base(), post_error: { kind: 'read_only', message: null } };
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge: 'github', forgeLogin: LOGIN, now: NOW });
  await click(button(footer(host), 'Try again'));
  assert.equal(footer(host).dataset.postState, 'read_only');
  assert.match(flat(footer(host)), /Your GitHub connection is read only/);
});

test('F4: a review that failed to generate has no Post footer, only its link', async () => {
  const card = { ...base(), status: 'failed', error: 'Ann couldn’t write the card’s text.' };
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge: 'github', forgeLogin: LOGIN, now: NOW });
  const f = footer(host);
  assert.equal(f.dataset.postState, 'no_review');
  assert.equal(!!host.querySelector('[data-post-button], [data-post-disabled], [data-post-retry]'), false, 'no Post, no Try again');
  assert.doesNotMatch(flat(f), /Nothing is posted|Posts as|Post to/);
  assert.match(flat(f), /^Open on GitHub/);
  const bare = await mount(m.ReviewCard, { card: { ...card, links: { ...card.links, pr: null } }, onFinding: async () => card, onPost: async () => card, forge: 'github', now: NOW });
  assert.equal(footer(bare), null, 'no link either: no footer at all');
});

test('F4: a 409 from kwant is "Couldn’t post" with no Try again', async () => {
  const calls = [];
  const err = Object.assign(new Error('This card has no review to post.'), { status: 409 });
  const host = await mount(m.ReviewCard, {
    card: base(),
    onFinding: async () => base(),
    onPost: async (id) => {
      calls.push(id);
      throw err;
    },
    forge: 'github', forgeLogin: LOGIN, now: NOW,
  });
  await click(button(footer(host), 'Post to GitHub'));
  const f = footer(host);
  assert.equal(f.dataset.postState, 'failed');
  assert.match(flat(f), /Couldn’t post: This card has no review to post\./);
  assert.equal(!!button(f, 'Try again'), false);
  assert.equal(!!button(f, 'Post to GitHub'), false);
  assert.deepEqual(calls, ['dvr_294']);
});

test('stale: run LED line and a disabled button', async () => {
  const card = { ...base(), post_error: { kind: 'stale', message: 'head moved' } };
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge: 'github', forgeLogin: LOGIN, now: NOW });
  const f = footer(host);
  assert.equal(f.dataset.postState, 'stale');
  assert.match(flat(f), /This PR has new commits\. A new review is on its way\./);
  const b = button(f, 'Post to GitHub');
  assert.equal(b.getAttribute('aria-disabled'), 'true');
  assert.equal(f.querySelector('.dv-led').dataset.s, 'run');
  assert.equal(!!button(f, 'Try again'), false);
});

test('failed: "Couldn’t post: {message}" and Try again, which posts again and clears the error', async () => {
  const calls = [];
  const card = { ...base(), post_error: { kind: 'failed', message: 'GitHub answered 502' } };
  const onPost = async (id) => (calls.push(id), { ...base(), posted: POSTED, post_error: null });
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost, forge: 'github', forgeLogin: LOGIN, now: NOW });
  let f = footer(host);
  assert.equal(f.dataset.postState, 'failed');
  assert.match(flat(f), /Couldn’t post: GitHub answered 502/);
  assert.equal(f.querySelector('.dv-led').dataset.s, 'bad');
  assert.match(button(f, 'Try again').className, /kw-btn-ghost/);
  assert.equal(!!button(f, 'Post to GitHub'), false);
  await click(button(f, 'Try again'));
  assert.deepEqual(calls, ['dvr_294']);
  f = footer(host);
  assert.equal(f.dataset.postState, 'posted');
});

test('failed from the network: a throw becomes "Couldn’t post: {message}"', async () => {
  const host = await mount(m.ReviewCard, {
    card: base(),
    onFinding: async () => base(),
    onPost: async () => {
      throw new Error('kwant is not reachable');
    },
    forge: 'github', forgeLogin: LOGIN,
    now: NOW,
  });
  await click(button(footer(host), 'Post to GitHub'));
  const f = footer(host);
  assert.equal(f.dataset.postState, 'failed');
  assert.match(flat(f), /Couldn’t post: kwant is not reachable/);
  assert.ok(button(f, 'Try again'));
});

test('a message is text, never HTML', () => {
  const out = html(m.ReviewCard, { card: { ...base(), post_error: { kind: 'failed', message: '<img src=x onerror=alert(1)>' } }, onFinding: async () => base(), onPost: async () => base(), forge: 'github', now: NOW });
  assert.doesNotMatch(out, /<img/);
  assert.match(out, /&lt;img/);
});

test('pending card: no Post button yet', () => {
  const out = html(m.ReviewCard, { card: { ...base(), status: 'pending' }, onFinding: async () => base(), onPost: async () => base(), forge: 'github', forgeLogin: LOGIN, now: NOW });
  assert.match(out, /data-post-state="none"/);
  assert.doesNotMatch(out, /Post to GitHub/);
});

// ---------------------------------------------------------------------------------------------------
// Copy changes and the desk wiring (source checks: these files need the whole app to mount)

import { readFileSync } from 'node:fs';
const src = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');

test('copy: Dev mode setup, Loose ends label and the token error say what posting needs', async () => {
  assert.match(src('src/components/DevModeSetup.tsx'), /Only you can turn this on or off\. Nothing is posted to GitHub unless you press Post\./);
  const le = 'Read-only access to issues and pull requests is enough. To post reviews from Dev mode, give it Pull requests: Read and write.';
  assert.ok(src('src/components/LooseEndsView.tsx').includes(le));
  const { githubSource } = await loadBundled(`export { githubSource } from './src/looseEnds';`);
  assert.equal(githubSource('', '', '').error, `Enter a GitHub token. ${le}`);
  assert.doesNotMatch(src('src/components/dev/ReviewCard.tsx'), /nothing is posted to GitHub in this version/);
});

test('the desk posts to /api/dev/cards/:id/post and passes forge and forge_login', () => {
  assert.match(src('src/devMode.ts'), /post: \(card: string\) => call<unknown>\(`\/api\/dev\/cards\/\$\{enc\(card\)\}\/post`, 'POST'/);
  const desk = src('src/components/DevDesk.tsx');
  assert.match(desk, /devApi\.post\(cardId\)/);
  assert.match(desk, /onPost=\{onPost\} forge=\{dev\?\.forge \?\? null\} forgeLogin=\{dev\?\.forge_login \?\? null\}/);
});

// ---------------------------------------------------------------------------------------------------
// Any forge (amendment, 21:50): the contract names the forge; posting is GitHub-only in 0.7.1

test('F7: forge null shows neither the Post button nor the Open link, in every state that would have one', async () => {
  for (const over of [{}, { post_error: { kind: 'read_only', message: null } }, { post_error: { kind: 'failed', message: 'x' } }]) {
    const card = { ...base(), ...over };
    const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge: null, forgeLogin: LOGIN, now: NOW });
    assert.equal(!!host.querySelector('[data-post-button], [data-post-disabled], [data-forge-link]'), false, 'no Post button, no link');
    assert.equal(!!button(host, 'Post to'), false);
    assert.equal(!!button(host, 'Try again'), false);
    assert.equal(!!host.querySelector('a[href="' + card.links.pr + '"]'), false, 'the PR link is not drawn anywhere on the card');
    assert.doesNotMatch(host.textContent, /Nothing is posted|Posts as|Open on|Open pull request|GitHub/, 'no lock line, no forge wording');
    assert.equal(!!host.querySelector('[data-review-footer]'), false, 'nothing to show: no empty footer rule');
  }
  // Leaving forge out is the same as null.
  const out = html(m.ReviewCard, { card: base(), onFinding: async () => base(), onPost: async () => base(), now: NOW });
  assert.doesNotMatch(out, /data-review-footer|Post to|Nothing is posted|Open on/);
  assert.equal(m.postState({ status: 'ready', posted: null, post_error: null }, { canPost: true, posting: false, forge: null }).kind, 'none');
});

test('forge null but already posted: still says Posted, with the forge from posted', () => {
  const out = html(m.ReviewCard, { card: { ...base(), posted: POSTED }, onFinding: async () => base(), now: NOW });
  assert.match(out, /Posted 19:58/);
  assert.match(out, /View on GitHub/);
});

test('the label, tooltip and links come from a map on forge (GitLab, Gitea)', async () => {
  assert.equal(m.FORGE_NAME.github, 'GitHub');
  assert.equal(m.postLabel(2, 'gitlab'), 'Post to GitLab · 2 findings');
  assert.equal(m.postLabel(0, 'gitea'), 'Post to Gitea');
  assert.equal(m.readOnlyTip('github'), 'Your GitHub connection is read only');
  assert.equal(m.readOnlyTip('gitlab'), 'Your GitLab connection is read only');
  assert.equal(m.readOnlyLine('gitea'), 'Your Gitea connection is read only. Give the token write access to post.', 'GitHub scope words only for github');
  const card = { ...base(), post_error: { kind: 'read_only', message: null } };
  const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge: 'gitlab', forgeLogin: LOGIN, now: NOW });
  const f = footer(host);
  assert.ok(button(f, 'Try again'), 'read only: Try again (F1)');
  assert.match(flat(f), /Your GitLab connection is read only/);
  assert.match(f.innerHTML, /Open on GitLab/);
  assert.doesNotMatch(flat(f), /GitHub/);
});

test('F7: the footer reads Post to {Forge} first, then the link Open on {Forge}', async () => {
  for (const [forge, name] of [['github', 'GitHub'], ['gitlab', 'GitLab'], ['gitea', 'Gitea']]) {
    for (const over of [{}]) {
      const card = { ...base(), ...over };
      const host = await mount(m.ReviewCard, { card, onFinding: async () => card, onPost: async () => card, forge, forgeLogin: LOGIN, now: NOW });
      const f = footer(host);
      const b = button(f, `Post to ${name}`);
      const a = f.querySelector('[data-forge-link] a');
      assert.ok(b, `Post to ${name}`);
      assert.ok(a, 'Open link');
      assert.equal(a.getAttribute('href'), card.links.pr);
      assert.match(flat(a), new RegExp(`^Open on ${name}`));
      assert.ok(b.compareDocumentPosition(a) & 4, 'Post comes before Open, in DOM (and so Tab) order');
    }
  }
  assert.equal(m.forgeOpen('gitea'), 'Open on Gitea');
  assert.equal(m.forgeOpen('forgejo'), 'Open on Forgejo');
  assert.equal(m.forgeOpen(null, 'commit'), 'Open commit');
});

test('no github_login or githubLogin is left in the frontend source', async () => {
  const { readdirSync, statSync } = await import('node:fs');
  const walk = (d) => readdirSync(d).flatMap((n) => (statSync(`${d}/${n}`).isDirectory() ? walk(`${d}/${n}`) : [`${d}/${n}`]));
  const root = new URL('../src', import.meta.url).pathname;
  const hits = walk(root).filter((f) => /\.(ts|tsx)$/.test(f) && /github_login|githubLogin/.test(readFileSync(f, 'utf8')));
  assert.deepEqual(hits, []);
});
