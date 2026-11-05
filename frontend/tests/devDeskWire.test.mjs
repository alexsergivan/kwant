// 0.7 Dev desk wiring (Pixel, after #314 and #315): the desk mounts Iris's real cards, "Open review" on a
// landed card switches to Reviews, scrolls to that PR's card and focuses its heading, the setup sheet's
// chip sits under the repo field on phone (Mira N3), and a failed backup from the secondary "Back up
// first" keeps keyboard focus.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo/dev', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const scrolled = [];
g.window.HTMLElement.prototype.scrollIntoView = function (opts) {
  scrolled.push([this.getAttribute('data-pr'), opts]);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as fx from './src/dev/fixtures';
  export { DevDeskView, deskStore } from './src/components/DevDesk';
  export { DevModeSetup } from './src/components/DevModeSetup';
  export { ProdFloorCard } from './src/components/dev/ProdFloorCard';
  export { parseProdFloor } from './src/dev/types';
`);
const { React, createRoot, fx } = m;
const h = React.createElement;

// Demo data only. One open finding on #289's review, so its landed card offers "Open review".
const finding = fx.REVIEW_294.findings[0];
const reviewsRoute = { cards: [fx.REVIEW_290, { ...fx.REVIEW_289, findings: [finding] }, { id: 'dvr_bad', kind: 'review', status: 'ready', title: 'No PR number' }] };
const landedRoute = { ...fx.LANDED_ROUTE, cards: [...fx.LANDED_ROUTE.cards, { id: 'dvc_bad', kind: 'landed', status: 'ready', headline: 'A card without a PR' }] };
let routes = { landed: landedRoute, reviews: reviewsRoute };
g.fetch = g.window.fetch = async (url) => {
  const u = String(url);
  const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
  if (u.includes('/dev/landed')) return json(routes.landed);
  if (u.includes('/dev/reviews')) return json(routes.reviews);
  if (u.includes('/dev_mode')) return json({ on: true, repo: 'demo-org/demo-app', branch: 'main' });
  return json({});
};

const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = [];
afterEach(() => {
  while (mounted.length) mounted.pop()();
  document.body.replaceChildren();
  scrolled.length = 0;
});
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  mounted.push(() => root.unmount());
  return host;
};
const desk = async () => {
  m.deskStore._reset();
  const host = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', now: fx.FIXTURE_NOW }));
  await settle();
  return host;
};
const byText = (host, sel, text) => [...host.querySelectorAll(sel)].find((b) => b.textContent.trim() === text);
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('slot swap: What landed draws Iris’s rollup and landed cards; a card she can’t draw gets her fallback row', async () => {
  const host = await desk();
  assert.ok(host.querySelector('[data-dev-landed-list]'), 'Iris’s LandedList');
  assert.equal(host.querySelector('[data-dev-slot]'), null, 'no placeholder cards are left');
  assert.match(host.textContent, /changes? landed on main/, 'Iris’s rollup title, with the branch');
  const prs = [...host.querySelectorAll('[data-dev-card="landed"]')].map((c) => Number(c.getAttribute('data-pr')));
  assert.deepEqual(prs.sort(), [288, 289, 290, 301, 303]);
  const fallback = host.querySelectorAll('[data-dev-card="fallback"]');
  assert.equal(fallback.length, 1);
  assert.match(fallback[0].textContent, /A card without a PR · This card couldn’t be shown\./);
});

test('slot swap: Reviews draws Iris’s review cards, one per PR, and the fallback for a bad one', async () => {
  const host = await desk();
  await click(host.querySelector('[data-dev-tab="reviews"]'));
  const cards = [...host.querySelectorAll('[data-dev-card="review"]')];
  assert.deepEqual(cards.map((c) => c.getAttribute('data-pr')), ['290', '289']);
  assert.equal(cards[0].querySelector('h3').textContent.trim().startsWith('#290'), true);
  assert.equal(host.querySelectorAll('[data-dev-card="fallback"]').length, 1);
});

test('Open review: switches to Reviews, scrolls to that PR’s card and focuses its heading', async () => {
  const host = await desk();
  const open = byText(host, 'button', 'Open review');
  assert.ok(open, 'the landed card with an open finding offers Open review');
  assert.match(open.closest('article').textContent, /#289/);
  open.focus();
  await click(open);
  const reviewsTab = host.querySelector('[data-dev-tab="reviews"]');
  assert.equal(reviewsTab.getAttribute('aria-selected'), 'true');
  const card = host.querySelector('[data-dev-card="review"][data-pr="289"]');
  const heading = card.querySelector('h3');
  assert.equal(document.activeElement, heading, 'focus is on #289’s heading, not <body>');
  assert.equal(heading.getAttribute('tabindex'), '-1', 'focusable by script, not a tab stop');
  assert.deepEqual(scrolled, [['289', { block: 'start', behavior: 'smooth' }]]);
});

test('Open review: reduced motion scrolls without the smooth animation', async () => {
  const mm = g.window.matchMedia;
  g.window.matchMedia = (q) => ({ matches: /reduce/.test(q), addEventListener() {}, removeEventListener() {} });
  try {
    const host = await desk();
    await click(byText(host, 'button', 'Open review'));
    assert.deepEqual(scrolled, [['289', { block: 'start', behavior: 'auto' }]]);
  } finally {
    g.window.matchMedia = mm;
  }
});

test('Open review: when that PR’s card isn’t on the desk, focus goes to the Reviews tab', async () => {
  const host = await desk();
  const open = byText(host, 'button', 'Open review');
  // The review closes between the press and the render.
  m.deskStore._set('spc_demo', { reviews: [] });
  await click(open);
  const reviewsTab = host.querySelector('[data-dev-tab="reviews"]');
  assert.equal(reviewsTab.getAttribute('aria-selected'), 'true');
  assert.equal(document.activeElement, reviewsTab);
  assert.deepEqual(scrolled, []);
});

test('N3: on phone the chip goes on its own line under the repo field; from sm up it sits inside it', () => {
  const html = m.renderToStaticMarkup(h(m.DevModeSetup, { space: { id: 'spc_demo', name: 'Demo' }, initial: null, onCancel() {}, onDone() {} }));
  const row = html.match(/<div class="([^"]*)" data-dv-repo-row="true">/);
  assert.ok(row, 'the repo row');
  for (const c of ['flex-col', 'sm:flex-row', 'items-start', 'sm:border', 'sm:h-10']) assert.ok(row[1].split(' ').includes(c), c);
  // The bordered box on phone holds only the prefix and the input; the chip is its sibling.
  const field = html.slice(row.index + row[0].length).match(/^<div class="([^"]*)">/);
  assert.ok(field, 'the field box is the row’s first child');
  for (const c of ['w-full', 'border', 'h-10', 'sm:border-0', 'sm:h-auto']) assert.ok(field[1].split(' ').includes(c), c);
});

test('ProdFloorCard: a backup from the secondary Back up first that fails puts focus on Try the backup again', async () => {
  const floor = m.parseProdFloor(fx.FLOOR_DEPLOY_OLD.details);
  for (const failure of ['throws', 'not ok']) {
    const host = await mount(
      h(m.ProdFloorCard, {
        id: 'ap_4',
        floor,
        bot: 'Iris',
        command: './scripts/deploy_test.sh 9a2d5f3',
        cwd: '~/work/demo',
        onResolve() {},
        onBackup: async () => {
          if (failure === 'throws') throw new Error('disk full');
          return { id: null, ok: false, path: null, bytes: null, taken_at: null, error: 'disk full' };
        },
      }),
    );
    const secondary = [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Back up first' && !b.hasAttribute('data-floor-main'));
    assert.ok(secondary, 'the old-backup deploy offers the secondary button');
    secondary.focus();
    await click(secondary);
    const main = host.querySelector('[data-floor-main]');
    assert.equal(main.getAttribute('data-floor-main'), 'retry');
    assert.equal(main.textContent.trim(), 'Try the backup again');
    assert.equal(document.activeElement, main, `${failure}: focus is on Try again, not <body>`);
    mounted.pop()();
    host.remove();
  }
});

// Round 2 (Mira's #321 review). M1: after the backup, focus moves to the main button only when it was lost
// (on <body>) or is still in the card. After a good backup that button is "Allow this deploy", so taking focus
// from the composer let one Space allow a production deploy.
const floorCard = async (outcome) => {
  let release;
  const gate = new Promise((r) => (release = r));
  const resolved = [];
  const host = await mount(
    h(m.ProdFloorCard, {
      id: 'ap_m1',
      floor: m.parseProdFloor(fx.FLOOR_DEPLOY_OLD.details),
      bot: 'Iris',
      command: './scripts/deploy_test.sh 9a2d5f3',
      cwd: '~/work/demo',
      onResolve: (id, how) => resolved.push(`${id}:${how}`),
      onBackup: async () => {
        await gate;
        if (outcome === 'throws') throw new Error('disk full');
        if (outcome === 'not ok') return { id: null, ok: false, path: null, bytes: null, taken_at: null, error: 'disk full' };
        return { id: 'bk_1', ok: true, path: '/backups/demo.tar.gz', bytes: 2048, taken_at: new Date().toISOString(), error: null };
      },
    }),
  );
  const secondary = [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Back up first' && !b.hasAttribute('data-floor-main'));
  return { host, secondary, release, resolved };
};

for (const outcome of ['ok', 'throws', 'not ok']) {
  test(`M1: focus in an outside input stays there when the backup ends (${outcome}), and Space allows nothing`, async () => {
    const { host, secondary, release, resolved } = await floorCard(outcome);
    const composer = document.createElement('textarea');
    document.body.appendChild(composer);
    secondary.focus();
    await click(secondary);
    composer.focus();
    release();
    await settle();
    assert.equal(document.activeElement, composer, 'focus is not taken from the composer');
    composer.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));
    composer.dispatchEvent(new KeyboardEvent('keyup', { key: ' ', bubbles: true }));
    await settle();
    assert.deepEqual(resolved, []);
    assert.equal(host.querySelector('[data-floor-main]').getAttribute('data-floor-main'), outcome === 'ok' ? 'allow' : 'retry');
  });
}

test('M1: with focus still in the card, a good backup lands focus on Allow this deploy', async () => {
  const { host, secondary, release } = await floorCard('ok');
  secondary.focus();
  await click(secondary);
  release();
  await settle();
  const main = host.querySelector('[data-floor-main]');
  assert.equal(main.textContent.trim(), 'Allow this deploy');
  assert.equal(document.activeElement, main);
});

test('S1: the secondary Back up first keeps focus during the backup and ignores presses while busy', async () => {
  const { host, secondary, release } = await floorCard('ok');
  secondary.focus();
  await click(secondary);
  assert.equal(secondary.hasAttribute('disabled'), false, 'not disabled, so focus does not drop to <body>');
  assert.equal(secondary.getAttribute('aria-disabled'), 'true');
  assert.equal(document.activeElement, secondary);
  assert.equal(host.querySelector('[data-floor-case]').getAttribute('data-floor-case'), 'taking');
  // A second press while busy does nothing: still one backup in flight, still taking.
  const before = host.innerHTML;
  await click(secondary);
  assert.equal(host.innerHTML, before);
  release();
  await settle();
});

test('S2: the Production keys hint sits outside the truncated line, so it wraps instead of being cut', async () => {
  const check = async () => ({ state: 'found', repo: 'demo-org/demo-app', branch: 'main', deploy_hint: 'docker' });
  const host = await mount(h(m.DevModeSetup, { space: { id: 'spc_demo', name: 'Demo' }, initial: { on: false, repo: 'demo-org/demo-app', branch: null }, check, onCancel() {}, onDone() {} }));
  await new Promise((r) => setTimeout(r, 450)); // the field's 400 ms check debounce
  await settle();
  const hint = [...host.querySelectorAll('[data-dv-tiles] p')].find((p) => p.textContent.trim() === 'Mark them in Connections');
  assert.ok(hint, 'the hint is its own line in the tile');
  assert.equal(hint.classList.contains('truncate'), false);
  assert.equal(hint.parentElement.querySelector('.truncate').textContent.includes('Mark them'), false, 'not inside the truncated value');
});

test('S3 + S4: the review heading has the violet focus ring and the card stops 12 px under the top', async () => {
  const host = await desk();
  await click(host.querySelector('[data-dev-tab="reviews"]'));
  const card = host.querySelector('[data-dev-card="review"]');
  assert.ok(card.className.split(' ').includes('scroll-mt-3'));
  const cls = card.querySelector('h3').className.split(' ');
  for (const c of ['focus:outline-none', 'focus-visible:outline-2', 'focus-visible:outline-sky-400', 'rounded-sm']) assert.ok(cls.includes(c), c);
});
