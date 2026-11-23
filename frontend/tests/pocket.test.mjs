// Pocket card (pa4, autonomy/pocket-apps/pa0a/design.md §1 and §6): validPocket (a good block, each
// state, missing fields, junk), no QR code unless ready or live, the ask before Stop / End quiz, the
// buttons that come with pa4b say why, holo at most once and only where earned, the Quizmaster avatar,
// and pocketApi's routes against Lin's pa1 host API.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.matchMedia = g.window.matchMedia;
const clipboard = { text: null, fail: false, writeText: async (t) => {
  if (clipboard.fail) throw new Error('denied');
  clipboard.text = t;
} };
Object.defineProperty(g.navigator, 'clipboard', { value: clipboard, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { COMPOSE_PREFILL_EVENT } from './src/components/results/SmartResult';
  export { Pocket, validPocket, showsQr, earnedChecked, budgetLine, LATER } from './src/components/results/Pocket';
  export { POCKET_FIXTURES, FIXTURES } from './src/components/results/fixtures';
  export { REGISTRY, resolveBlock } from './src/components/results/registry';
  export { blockMarkdown, blockRows } from './src/components/results/exporters';
  export { pocketApi, PocketApiError } from './src/pocketApi';
  export { pickCreature, AVATAR_STYLES, paletteCreature, isPalette } from './src/components/ModernAvatar';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
const realFetch = g.fetch;
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  localStorage.clear();
  clipboard.text = null;
  clipboard.fail = false;
  g.fetch = realFetch;
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = { host, done: () => (root.unmount(), host.remove()) };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (el, k) => {
  el.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: k, bubbles: true }));
  await settle();
};
const F = m.POCKET_FIXTURES;
const raw = (name) => structuredClone(F[name].results.blocks[0]);
const card = (name, over = {}) => mount(h(m.MarkdownRenderer, { content: F[name].reply, results: { version: 1, blocks: [{ ...raw(name), ...over }] }, messageId: `m-${name}` }));
const button = (v, text) => [...v.host.querySelectorAll('button, a')].find((b) => b.textContent.trim() === text);
const holos = (v) => v.host.querySelectorAll('.kw-sticker-holo, [data-tone="holo"]').length;

// ---------------------------------------------------------------------------------------------------
// The validator

test('every fixture is a good block: each state validates and renders its own card', async () => {
  const states = { writing: 'writing', checking: 'checking', dryRunStopped: 'dry_run_stopped', ready: 'ready', readyAnywhere: 'ready', live: 'live', paused: 'live', ended: 'ended', results: 'results' };
  for (const [name, state] of Object.entries(states)) {
    const b = m.validPocket(raw(name));
    assert.ok(b, `${name} validates`);
    assert.ok(m.resolveBlock(raw(name), null), `${name} resolves as a slot block`);
    const v = await card(name);
    assert.equal(v.host.querySelector('[data-pocket]')?.getAttribute('data-pocket'), state, name);
    assert.match(v.host.textContent, /Friday Pub Quiz/, `${name}: its name`);
    assert.equal(v.host.querySelector('[data-evidence]'), null, `${name}: no Smart Results footer, the card has its own`);
    v.done();
  }
  assert.equal(m.FIXTURES.pocket, F.ready, 'the registry-wide fixture list carries one pocket block');
});

test('missing optional fields fall back to plain words', () => {
  const min = { id: 'p', type: 'pocket', source: { kind: 'slot' }, provenance: { kind: 'reply_text' }, code: 'fq7k2m', kit: 'quiz', name: 'Quiz', state: 'checking' };
  const b = m.validPocket(min);
  assert.ok(b);
  assert.deepEqual([b.when, b.progress, b.checks, b.dry_run, b.live, b.join, b.ended, b.budget, b.results], [null, null, null, null, null, null, null, null, null]);
  assert.deepEqual(b.rounds, []);
  assert.equal(b.paused, false);
  assert.equal(m.budgetLine(b), "Bot calls so far aren't counted yet. Live play uses none unless a near miss needs judging.");
  assert.equal(m.budgetLine({ ...b, budget: { calls: 15, usd: 0.04 } }), 'About 15 bot calls so far, about $0.04 on your account. Live play uses none unless a near miss needs judging.');
  // paused counts only while live.
  assert.equal(m.validPocket({ ...raw('ready'), paused: true }).paused, false);
  assert.equal(m.validPocket(raw('paused')).paused, true);
});

test('dry run stopped without words says what happened in its own fallback', async () => {
  const v = await card('dryRunStopped', { dry_run: { players: 30, passed: false } });
  assert.match(v.host.textContent, /Dry run stopped before the end/);
  assert.match(v.host.textContent, /What happened:.*couldn't finish the quiz/);
  assert.match(v.host.textContent, /What I'm doing:.*looking into it/);
});

test('junk and contradictions are rejected, so the reply shows as markdown', () => {
  const bad = [
    null,
    'pocket',
    [],
    { ...raw('ready'), source: { kind: 'markdown_table', index: 0 } },
    { ...raw('ready'), state: 'open' },
    { ...raw('ready'), code: 'no spaces!' },
    { ...raw('ready'), code: undefined },
    { ...raw('ready'), kit: 'Quiz Night' },
    { ...raw('ready'), name: '' },
    { ...raw('ready'), name: 42 },
    { ...raw('ready'), paused: 'yes' },
    { ...raw('ready'), dry_run: null },
    { ...raw('ready'), dry_run: { players: 30, passed: false } },
    { ...raw('dryRunStopped'), dry_run: { players: 30, passed: true } },
    { ...raw('live'), results: raw('results').results },
    { ...raw('ready'), join: { mode: 'bluetooth', wifi_url: null, anywhere_url: null } },
    { ...raw('ready'), join: { mode: 'wifi', wifi_url: 'javascript:alert(1)', anywhere_url: null } },
    { ...raw('writing'), progress: { done: 41, total: 40 } },
    { ...raw('writing'), rounds: Array.from({ length: 9 }, () => ({ label: 'R', topic: null, written: 0, total: 1 })) },
    { ...raw('writing'), rounds: [{ label: 'R', written: 3, total: 2 }] },
    { ...raw('live'), live: { ...raw('live').live, palettes: ['hotpink'] } },
    { ...raw('live'), live: { ...raw('live').live, players: -1 } },
    { ...raw('ready'), budget: { calls: 1, usd: -2 } },
    { ...raw('results'), results: { ...raw('results').results, teams: [] } },
    { ...raw('results'), results: { ...raw('results').results, missed: [{ label: 'Q1', question: 'x', answer: null, pct_right: 140 }] } },
    { ...raw('results'), results: { ...raw('results').results, near_misses: 1, near_misses_accepted: 2 } },
  ];
  bad.forEach((b, i) => assert.equal(m.validPocket(b), null, `case ${i}`));
  assert.equal(m.resolveBlock(raw('ready'), { kind: 'list', items: [], start: 1, ordered: false, task: false }), null, 'never placed into the reply body');
});

// ---------------------------------------------------------------------------------------------------
// No QR code until a dry run passed

test('no QR unless ready (dry run passed) or live', async () => {
  const qr = (b) => m.showsQr(b);
  assert.equal(qr({ state: 'writing', dry_run: null }), false);
  assert.equal(qr({ state: 'checking', dry_run: null }), false);
  assert.equal(qr({ state: 'dry_run_stopped', dry_run: { passed: false } }), false);
  assert.equal(qr({ state: 'ready', dry_run: { passed: false } }), false);
  assert.equal(qr({ state: 'ready', dry_run: { passed: true } }), true);
  assert.equal(qr({ state: 'live', dry_run: null }), true);
  assert.equal(qr({ state: 'ended', dry_run: { passed: true } }), false);
  // Even with join links in the block, the earlier states draw no code and no link.
  const join = raw('ready').join;
  for (const name of ['writing', 'checking', 'dryRunStopped', 'ended', 'results']) {
    const v = await card(name, { join });
    assert.equal(v.host.querySelector('[data-qr]'), null, `${name}: no QR`);
    assert.equal(v.host.querySelector('[data-join]'), null, `${name}: no join box`);
    assert.doesNotMatch(v.host.textContent, /192\.168|trycloudflare/, `${name}: no join link`);
    v.done();
  }
  const ready = await card('ready');
  assert.equal(ready.host.querySelectorAll('[data-qr]').length, 1);
  assert.equal(ready.host.querySelector('[data-join]').getAttribute('data-join'), 'full', 'the full join box');
  const live = await card('live');
  assert.equal(live.host.querySelector('[data-join]').getAttribute('data-join'), 'compact', 'live: the smaller code for late arrivals');
  assert.match(live.host.textContent, /Same Wi-Fi · late arrivals can still scan in/);
  // Show as text follows the same rule.
  assert.doesNotMatch(m.blockMarkdown(m.validPocket({ ...raw('checking'), join })), /192\.168/);
  assert.match(m.blockMarkdown(m.validPocket(raw('ready'))), /192\.168\.1\.20/);
});

test('join mode: a radio group with arrow keys; the link follows; a missing link says so and draws no code', async () => {
  const v = await card('ready');
  const radios = [...v.host.querySelectorAll('[role="radio"]')];
  assert.deepEqual(radios.map((r) => r.getAttribute('aria-checked')), ['true', 'false']);
  assert.equal(v.host.querySelector('input[aria-label="Join link"]').value, 'http://192.168.1.20:7480/p/fq7k2m');
  await key(radios[0], 'ArrowRight');
  assert.deepEqual([...v.host.querySelectorAll('[role="radio"]')].map((r) => r.getAttribute('aria-checked')), ['false', 'true']);
  assert.equal(v.host.querySelector('input[aria-label="Join link"]').value, 'https://quiet-otter-lamp.trycloudflare.com/p/fq7k2m');
  assert.match(v.host.textContent, /free Cloudflare quick link/);
  assert.equal(button(v, 'Try it as a player').getAttribute('href'), 'https://quiet-otter-lamp.trycloudflare.com/p/fq7k2m');
  v.done();

  const none = await card('readyAnywhere', { join: { mode: 'anywhere', wifi_url: 'http://192.168.1.20:7480/p/fq7k2m', anywhere_url: null } });
  assert.equal(none.host.querySelector('[data-qr]'), null);
  assert.match(none.host.querySelector('[data-no-link]').textContent, /from-anywhere link isn't set up on this box yet/);
});

test('Copy link copies, says Copied, and says so when the browser refuses', async () => {
  const v = await card('ready');
  await click(v.host.querySelector('[data-copy-link]'));
  assert.equal(clipboard.text, 'http://192.168.1.20:7480/p/fq7k2m');
  assert.equal(v.host.querySelector('[data-copy-link]').textContent, 'Copied');
  clipboard.fail = true;
  await click(v.host.querySelector('[data-copy-link]'));
  assert.equal(v.host.querySelector('[data-copy-link]').textContent, "Couldn't copy");
});

// ---------------------------------------------------------------------------------------------------
// Stop and End quiz ask first

test('Stop asks first; Escape keeps going; yes ends the pocket on the box', async () => {
  const calls = [];
  g.fetch = async (url, init) => (calls.push([init.method, url]), new Response(JSON.stringify({ pocket: { code: 'fq7k2m' } }), { status: 200 }));
  const v = await card('checking');
  await click(button(v, 'Stop'));
  assert.equal(calls.length, 0, 'nothing happens on the first tap');
  const dialog = v.host.querySelector('[data-confirm="stop"]');
  assert.ok(dialog);
  assert.equal(document.activeElement.textContent, 'Stop the quiz', 'focus moves into the question');
  await key(dialog, 'Escape');
  assert.equal(v.host.querySelector('[data-confirm]'), null);
  await click(button(v, 'Stop'));
  await click(button(v, 'Stop the quiz'));
  assert.deepEqual(calls, [['POST', '/api/pockets/fq7k2m/end']]);
  assert.match(v.host.querySelector('[data-done="ok"]').textContent, /Stopped/);
});

test('End quiz asks first, names the leader, and shows the box’s own error', async () => {
  g.fetch = async () => new Response(JSON.stringify({ error: 'pocket_apps_off', message: 'Pocket apps are off.' }), { status: 404 });
  const v = await card('live');
  await click(button(v, 'End quiz'));
  assert.match(v.host.querySelector('[data-confirm="end"]').textContent, /Quizzly Bears win with 1,840/);
  await click(button(v, 'End and show winners'));
  assert.match(v.host.querySelector('[data-done="error"]').textContent, /Couldn't end the quiz: Pocket apps are off\. Try again\./);
  assert.ok(v.host.querySelector('[data-confirm="end"]'), 'the question stays so it can be tried again');
});

// ---------------------------------------------------------------------------------------------------
// pa4b buttons, prefilled asks, holo

test('buttons that come with pa4b are disabled and say why', async () => {
  const later = async (name, labels, over = {}) => {
    const v = await card(name, over);
    for (const [text, why] of labels) {
      const b = button(v, text);
      assert.ok(b, `${name}: ${text}`);
      assert.equal(b.getAttribute('aria-disabled'), 'true', `${name}: ${text} disabled`);
      assert.equal(b.getAttribute('title'), why, `${name}: ${text} says why`);
    }
    v.done();
  };
  const r1 = { label: 'Round 1', topic: 'Music', written: 6, total: 10 };
  await later('writing', [['Edit questions', m.LATER.editRound1]], { rounds: [r1] });
  await later('writing', [['Edit questions', m.LATER.edit]]);
  await later('checking', [['Edit questions', m.LATER.edit]]);
  await later('ready', [['Big screen', m.LATER.bigScreen], ['Edit questions', m.LATER.edit]]);
  await later('live', [['Open the remote', m.LATER.remote], ['Big screen', m.LATER.bigScreen], ['Pause', m.LATER.pause]]);
  await later('paused', [['Resume', m.LATER.pause]]);
  await later('ended', [['Keep the results', m.LATER.keep]]);
  await later('results', [['Keep the results', m.LATER.keep]]);
});

test('See results, the dry run report and Run it again prefill the composer, never send', async () => {
  const asked = [];
  const on = (e) => asked.push(e.detail.text);
  window.addEventListener(m.COMPOSE_PREFILL_EVENT, on);
  try {
    let v = await card('ended');
    await click(button(v, 'See results'));
    await click(button(v, 'Run it again next Friday'));
    v.done();
    v = await card('dryRunStopped');
    await click(button(v, 'See the dry run report'));
    v.done();
    v = await card('results');
    await click(button(v, 'Reuse with new questions'));
  } finally {
    window.removeEventListener(m.COMPOSE_PREFILL_EVENT, on);
  }
  assert.deepEqual(asked, [
    'Show me the results of Friday Pub Quiz',
    'Run Friday Pub Quiz again next Friday with new questions',
    'Show me the dry run report for Friday Pub Quiz',
    'Reuse Friday Pub Quiz with new questions',
  ]);
});

test('holo is earned and at most one per card: Checked on ready, Winners at the end', async () => {
  const want = { writing: 0, checking: 0, dryRunStopped: 0, ready: 1, readyAnywhere: 1, live: 0, paused: 0, ended: 1, results: 1 };
  for (const [name, n] of Object.entries(want)) {
    const v = await card(name);
    assert.equal(holos(v), n, name);
    v.done();
  }
  // A ready quiz with an unchecked answer earns no "Checked".
  const partial = m.validPocket({ ...raw('ready'), checks: { matched: 39, total: 40, swapped: [] } });
  assert.equal(m.earnedChecked(partial), false);
  const v = await card('ready', { checks: { matched: 39, total: 40, swapped: [] } });
  assert.equal(holos(v), 0);
  // An ended quiz with no winner shows none.
  v.done();
  const nw = await card('ended', { ended: { at: '10:12 PM', winner: null, stats: null, deletes_on: null } });
  assert.equal(holos(nw), 0);
  assert.match(nw.host.textContent, /deleted 7 days after the quiz/);
});

test('the results card: winner strip, scores best first, missed most, the deletion line', async () => {
  const v = await card('results');
  assert.match(v.host.querySelector('[data-winner]').textContent, /Quizzly Bears win.*Ana, Ben, Chloe, Dev · 3,420 points · 240 ahead/);
  assert.deepEqual([...v.host.querySelectorAll('tbody tr')].map((tr) => tr.children[1].textContent), ['Quizzly Bears', 'Les Quizerables', 'Agatha Quiztie', 'The Know-It-Ales', 'Trivia Newton-John', 'Table 6']);
  assert.equal(v.host.querySelectorAll('[role="progressbar"]').length, 3);
  assert.match(v.host.textContent, /5 near misses settled, you accepted 3/);
  assert.match(v.host.textContent, /19 bot calls in all, about \$0\.05/);
  assert.match(v.host.textContent, /Names and answers are deleted on Fri, Dec 4\. Keeping the results saves names and scores only\./);
  assert.equal(m.blockRows(m.validPocket(raw('results'))).filter((r) => /^\d\./.test(r.what)).length, 6, 'Show as text lists the scores');
});

// ---------------------------------------------------------------------------------------------------
// Quizmaster, pocketApi

test('Quizmaster: a picker style, the host look, and the quiz name hint', () => {
  assert.ok(m.AVATAR_STYLES.some((s) => s.id === 'quizmaster' && s.label === 'Quizmaster'));
  const style = m.pickCreature('quizmaster', 'Anything');
  assert.equal(style.accessory, 'host');
  assert.equal(m.pickCreature('default', 'Quizmaster').accessory, 'host', 'a bot named for quizzes gets the host look');
  assert.equal(m.pickCreature('default', 'Quiz night helper').accessory, 'host');
  assert.equal(m.paletteCreature('coral').accessory, 'none', 'team creatures are plain guests');
  assert.equal(m.isPalette('coral'), true);
  assert.equal(m.isPalette('constructor'), false);
});

test('pocketApi: the four routes, same-origin, and plain errors', async () => {
  const calls = [];
  g.fetch = async (url, init) => {
    calls.push([init.method, url, init.credentials]);
    if (init.method === 'DELETE') return new Response(null, { status: 204 });
    return new Response(JSON.stringify({ pockets: [] }), { status: 200 });
  };
  await m.pocketApi.list();
  await m.pocketApi.read('fq7k2m');
  await m.pocketApi.end('fq7k2m');
  assert.equal(await m.pocketApi.remove('fq7k2m'), undefined);
  assert.deepEqual(calls, [
    ['GET', '/api/pockets', 'same-origin'],
    ['GET', '/api/pockets/fq7k2m', 'same-origin'],
    ['POST', '/api/pockets/fq7k2m/end', 'same-origin'],
    ['DELETE', '/api/pockets/fq7k2m', 'same-origin'],
  ]);
  await assert.rejects(m.pocketApi.read('../x'), (e) => e instanceof m.PocketApiError && e.status === 404, 'a bad code never reaches the network');
  assert.equal(calls.length, 4);
  g.fetch = async () => new Response('', { status: 404 });
  await assert.rejects(m.pocketApi.list(), (e) => e.message === 'This pocket app no longer exists.');
  g.fetch = async () => new Response(JSON.stringify({ error: 'pocket_apps_off' }), { status: 404 });
  await assert.rejects(m.pocketApi.list(), (e) => e.code === 'pocket_apps_off' && /Turn them on in Settings/.test(e.message));
  g.fetch = async () => {
    throw new TypeError('offline');
  };
  await assert.rejects(m.pocketApi.end('fq7k2m'), (e) => e.status === 0 && /didn’t answer/.test(e.message));
});
