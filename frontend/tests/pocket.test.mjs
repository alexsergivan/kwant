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
  export { Pocket, validPocket, showsQr, earnedChecked, budgetLine, LATER, WARN_FILL } from './src/components/results/Pocket';
  export { POCKET_FIXTURES, FIXTURES } from './src/components/results/fixtures';
  export { REGISTRY, resolveBlock } from './src/components/results/registry';
  export { blockMarkdown, blockRows } from './src/components/results/exporters';
  export { pocketApi, PocketApiError, linkUrl } from './src/pocketApi';
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

test('buttons whose route comes later are disabled and say why', async () => {
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
  await later('writing', [['Edit questions', m.LATER.editRound1]]);
  // pa4c: with no draft id on the block, Edit asks for the chat instead.
  await later('checking', [['Edit questions', m.LATER.noDraft]]);
  await later('ready', [['Edit questions', m.LATER.noDraft]]);
  // No dead Pause or Resume on the live card: the remote pauses (Mira round 1, fix 1).
  for (const name of ['live', 'paused']) {
    const v = await card(name);
    assert.equal(button(v, 'Pause'), undefined, `${name}: no Pause`);
    assert.equal(button(v, 'Resume'), undefined, `${name}: no Resume`);
    v.done();
  }
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

test('Missed most: the warn bar fill is a class Tailwind really generates, using the --kw-warn token', async () => {
  const v = await card('results');
  const warn = [...v.host.querySelectorAll('[data-tone="warn"]')];
  assert.equal(warn.length, 3, 'all three missed-most bars are warn');
  for (const el of warn) assert.ok(el.classList.contains(m.WARN_FILL), el.className);
  assert.ok(!v.host.innerHTML.includes('bg-warn'), 'bg-warn has no CSS: warn is a text colour only');
  const { default: postcss } = await import('postcss');
  const { default: tailwind } = await import('tailwindcss');
  const { default: config } = await import('../tailwind.config.js');
  const out = await postcss([tailwind({ ...config, content: [{ raw: `<div class="${warn[0].className}"></div>` }] })]).process('@tailwind utilities;', { from: undefined });
  assert.match(out.css, /\.bg-\\\[color\\:var\\\(--kw-warn\\\)\\\]\s*\{[^}]*background-color:\s*var\(--kw-warn\)/, 'the fill rule exists');
});

test('round 2: kit glyph, Live chip, join segment, one-line buttons and count, QR gone after Stop', async () => {
  const ready = await card('ready');
  const tile = ready.host.querySelector('[data-glyph="quiz"]');
  assert.equal(tile.getAttribute('viewBox'), '0 0 9 10');
  assert.equal(tile.querySelectorAll('rect').length, 11);
  assert.ok(tile.parentElement.classList.contains('text-sky-200'));
  const seg = ready.host.querySelector('[role="radiogroup"]');
  assert.ok(seg.classList.contains('self-start') && !seg.classList.contains('w-full'));
  for (const r of seg.querySelectorAll('[role="radio"]')) assert.ok(r.classList.contains('flex-none') && r.classList.contains('text-meta') && r.classList.contains('min-h-[44px]'));
  const edit = button(ready, 'Edit questions');
  assert.ok(edit.classList.contains('whitespace-nowrap') && edit.classList.contains('text-[14px]'), edit.className);
  assert.ok(button(ready, 'Try it as a player').classList.contains('text-[15px]'), 'primary stays 15 px');
  const live = await card('live');
  const chip = live.host.querySelector('[data-chip="live"]');
  assert.ok(chip.classList.contains('bg-sky-600') && chip.classList.contains('text-white'));
  assert.ok(chip.querySelector('.kw-pulse'));
  const paused = await card('paused');
  assert.equal(paused.host.querySelector('[data-chip="live"] .kw-pulse'), null, 'no pulse while paused');
  const checking = await card('checking');
  const count = [...checking.host.querySelectorAll('strong')].find((s) => /of/.test(s.textContent) && s.classList.contains('whitespace-nowrap'));
  assert.ok(count && count.parentElement.tagName === 'SPAN', 'verb and count share one span');
  g.fetch = async () => new Response(JSON.stringify({ pocket: { code: 'fq7k2m' } }), { status: 200 });
  assert.ok(live.host.querySelector('[data-qr]'));
  await click(button(live, 'End quiz'));
  await click(button(live, 'End and show winners'));
  assert.ok(live.host.querySelector('[data-done="ok"]'));
  assert.equal(live.host.querySelector('[data-qr]'), null, 'the closed link is no longer offered');
  assert.equal(live.host.querySelector('[data-copy-link]'), null);
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

// ---------------------------------------------------------------------------------------------------
// pa4b: Open the remote and Big screen mint a fresh single-use link per press

/** The main port as the card sees it: GET the pocket (join addresses), POST …/link mints token n. */
function linkBox({ join = { mode: 'wifi', wifi_url: 'http://192.168.1.20:8790/p/fq7k2m', anywhere_url: 'https://quiz-x.trycloudflare.com/p/fq7k2m' }, fail = null } = {}) {
  const calls = [];
  let n = 0;
  g.fetch = async (url, init = {}) => {
    calls.push({ method: init.method, url, body: init.body ? JSON.parse(init.body) : undefined });
    if (init.method === 'POST' && url.endsWith('/link')) {
      if (fail) return new Response(JSON.stringify(fail), { status: 403 });
      n++;
      return new Response(JSON.stringify({ token: `tok+${n}/x`, role: JSON.parse(init.body).role, expires_at: new Date(Date.now() + 600_000).toISOString() }), { status: 200 });
    }
    return new Response(JSON.stringify({ pocket: { code: 'fq7k2m' }, summary: null, removed: 0, join }), { status: 200 });
  };
  return calls;
}
const mints = (calls) => calls.filter((c) => c.method === 'POST' && c.url.endsWith('/link'));

test('pocketApi.mintLink: POST /api/pockets/:code/link {role}; linkUrl puts the token in the fragment only', async () => {
  const calls = linkBox();
  const l = await m.pocketApi.mintLink('fq7k2m', 'host');
  assert.equal(l.token, 'tok+1/x');
  assert.deepEqual(mints(calls).map((c) => [c.url, c.body]), [['/api/pockets/fq7k2m/link', { role: 'host' }]]);
  assert.equal(m.linkUrl('http://192.168.1.20:8790/p/fq7k2m', 'host', 'tok+1/x'), 'http://192.168.1.20:8790/p/fq7k2m#remote=tok%2B1%2Fx');
  assert.equal(m.linkUrl('https://a.example/p/fq7k2m?x=1#old', 'screen', 't'), 'https://a.example/p/fq7k2m#screen=t');
  await assert.rejects(m.pocketApi.mintLink('../x', 'host'), (e) => e.status === 404);
});

test('card: Open the remote mints a host link and shows a private QR; every press mints a fresh one; no token in a query', async () => {
  const calls = linkBox();
  const v = await card('live');
  const open = button(v, 'Open the remote');
  assert.equal(open.getAttribute('aria-disabled'), null);
  assert.equal(open.getAttribute('aria-expanded'), 'false');
  await click(open);
  const panel = v.host.querySelector('[data-link-panel="host"]');
  assert.ok(panel, 'the panel shows');
  assert.equal(panel.getAttribute('data-link'), 'ready');
  assert.match(panel.textContent, /Only you see this · scan with your phone/);
  assert.match(panel.textContent, /Works once, for 10 minutes/);
  assert.equal(panel.querySelectorAll('[data-qr] svg, [data-qr] img, [data-qr] canvas').length > 0, true, 'a QR code');
  assert.equal(open.getAttribute('aria-expanded'), 'true');
  assert.deepEqual(mints(calls).map((c) => c.body), [{ role: 'host' }]);
  await click(button(v, 'Open the remote'));
  await click(button(v, 'New code'));
  assert.equal(mints(calls).length, 3, 'one fresh link per press');
  for (const c of calls) assert.ok(!/tok/.test(c.url), `no token in ${c.url}`);
  assert.ok(!v.host.querySelector('.kw-sticker'), 'no holo for a link');
  await click(v.host.querySelector('[data-link-close]'));
  assert.equal(v.host.querySelector('[data-link-panel]'), null);
  assert.equal(document.activeElement, button(v, 'Open the remote'), 'focus goes back to the button');
});

test('card on a phone (under 640 px): Open here leads, primary, and the host line says a phone cannot scan itself', async () => {
  linkBox();
  const real = window.matchMedia;
  window.matchMedia = g.matchMedia = (q) => ({ matches: q === '(max-width: 639px)', addEventListener() {}, removeEventListener() {} });
  try {
    const v = await card('live');
    await click(button(v, 'Open the remote'));
    const panel = v.host.querySelector('[data-link-panel="host"]');
    assert.equal(panel.querySelector('[data-where]').textContent, 'Only you see this · press Open here, or scan from another phone');
    const acts = [...panel.querySelectorAll('[data-action]')].map((b) => b.getAttribute('data-action'));
    assert.deepEqual(acts, ['open-here', 'new-link'], 'Open here comes first');
    assert.match(panel.querySelector('[data-action="open-here"]').className, /kw-btn-primary/);
    assert.match(panel.querySelector('[data-qr]').className, /w-\[200px\]/, 'QR stays 200 px');
    await click(button(v, 'Big screen'));
    assert.match(v.host.querySelector('[data-link-panel="screen"] [data-where]').textContent, /press Open here on the computer plugged into the TV, or scan it with the tablet or laptop/);
    v.done();
  } finally {
    window.matchMedia = g.matchMedia = real;
  }
});

test('card: Big screen mints a screen link on ready and live; Open here mints its own and opens it in a new tab', async () => {
  const calls = linkBox();
  const opened = [];
  const realOpen = window.open;
  window.open = (u, target) => {
    const tab = { location: { href: u }, opener: window, closed: false, close() { this.closed = true; } };
    opened.push({ tab, target });
    return tab;
  };
  try {
    const v = await card('ready');
    await click(button(v, 'Big screen'));
    assert.ok(v.host.querySelector('[data-link-panel="screen"]'));
    assert.deepEqual(mints(calls).map((c) => c.body), [{ role: 'screen' }]);
    await click(button(v, 'Open here'));
    assert.equal(opened.length, 1);
    assert.equal(opened[0].target, '_blank');
    assert.equal(opened[0].tab.opener, null, 'the new tab cannot reach back into the app');
    assert.match(opened[0].tab.location.href, /\/p\/fq7k2m#screen=tok%2B2%2Fx$/);
    assert.ok(!opened[0].tab.location.href.includes('?'), 'never a query string');
    assert.equal(mints(calls).length, 2, 'Open here made its own link; the QR one stays good');
    const live = await card('live');
    await click(button(live, 'Big screen'));
    assert.ok(live.host.querySelector('[data-link-panel="screen"]'));
  } finally {
    window.open = realOpen;
  }
});

test('card: a refused mint says why in plain words, closes the blank tab, and can be tried again', async () => {
  linkBox({ fail: { error: 'pocket_apps_off', message: 'Pocket apps are off.' } });
  const tabs = [];
  const realOpen = window.open;
  window.open = () => {
    const tab = { location: { href: '' }, opener: window, closed: false, close() { this.closed = true; } };
    tabs.push(tab);
    return tab;
  };
  try {
    const v = await card('live');
    await click(button(v, 'Open the remote'));
    const panel = v.host.querySelector('[data-link-panel="host"]');
    assert.equal(panel.getAttribute('data-link'), 'error');
    assert.match(panel.textContent, /Couldn't make a remote link: Pocket apps are off\./);
    assert.equal(panel.querySelector('[data-qr]'), null, 'no QR without a link');
    await click(button(v, 'Open here'));
    assert.equal(tabs[0].closed, true);
  } finally {
    window.open = realOpen;
  }
});

test('card: with no join address anywhere the panel says so and mints nothing', async () => {
  const calls = linkBox({ join: null });
  const v = await card('live', { join: null });
  await click(button(v, 'Open the remote'));
  assert.match(v.host.querySelector('[data-link-panel]').textContent, /hasn't said its address yet/);
  assert.equal(mints(calls).length, 0);
});
