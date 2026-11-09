// 0.8.0 B3 (Iris): the pre-push card (every state P0–P7, retried, the reason form, phone folding),
// the Inbox "Needs you" item, the "Checked with fresh eyes" chip (R1–R3) and the semgrep tile.
// Data is shaped like Juno's card_json (ba2157a5); see src/dev/prepushFixtures.ts.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'CustomEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export * from './src/dev/prepush';
  export * from './src/dev/freshEyes';
  export * as fx from './src/dev/prepushFixtures';
  export { parseReview } from './src/dev/types';
  export * as dfx from './src/dev/fixtures';
  export { PrepushCard } from './src/components/dev/PrepushCard';
  export { PrepushNeedsYou, PrepushThread, threadCards, usePrepushCards } from './src/components/dev/PrepushThread';
  export { FreshEyesChip } from './src/components/dev/FreshEyesChip';
  export { DevModeError } from './src/devMode';
  export { ReviewCard, semgrepTile } from './src/components/dev/ReviewCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));
const card = (raw) => {
  const c = m.parsePrepush(raw);
  assert.ok(c, 'fixture parses');
  return c;
};

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
  return { host, rerender: async (p) => { root.render(h(C, p)); await settle(); } };
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const button = (host, t) => [...host.querySelectorAll('button')].find((b) => text(b).startsWith(t));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (el, value) => {
  const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};

// ---------------------------------------------------------------------------------------------------
// Parsing and phases

test('every fixture parses and maps to Mira\'s phase', () => {
  const want = {
    PP_CHECKING: 'P1', PP_NO_CHANGE: 'P0', PP_CLEAR: 'P2', PP_BLOCKED: 'P3', PP_NEEDS_YOU: 'P4', PP_MANY: 'P3',
    PP_FAILED: 'P7', PP_REVIEW_ERROR: 'P7', PP_FAILED_NEEDS_YOU: 'P4', PP_OVERRIDDEN: 'P6', PP_RETRIED: 'RT',
  };
  for (const [k, phase] of Object.entries(want)) assert.equal(m.phaseOf(card(fx[k])), phase, k);
});

test('the parser refuses cards it cannot show honestly', () => {
  assert.equal(m.parsePrepush(null), null);
  assert.equal(m.parsePrepush({ ...fx.PP_CLEAR, status: 'pushed' }).status, 'unknown', 'a newer status is kept, read-only (Ada Later 2)');
  assert.equal(m.parsePrepush({ ...fx.PP_CLEAR, status: 7 }), null, 'a status that is not a word is refused');
  assert.equal(m.parsePrepush({ ...fx.PP_CLEAR, branch: '' }), null);
  assert.equal(m.parsePrepush({ ...fx.PP_CLEAR, checks: [...fx.PP_CLEAR.checks, { key: 'osv', state: 'pass', text: 'x' }] }), null, 'an unknown row is never dropped silently');
  assert.equal(m.parsePrepush({ ...fx.PP_CLEAR, checks: [{ key: 'secrets', state: 'queued', text: 'Waiting' }] }), null, 'no queued state in the contract');
  const c = card({ ...fx.PP_BLOCKED, needs_you: true, tries: 1 });
  assert.equal(c.needs_you, true, 'needs_you is the server\'s word');
  assert.equal(card({ ...fx.PP_CLEAR, needs_you: true }).needs_you, false, 'only blocked or failed cards can need you');
  assert.equal(card({ ...fx.PP_CLEAR, diffstat: { files: -1, add: 1, del: 1 } }).diffstat, null);
  assert.equal(card(fx.PP_CLEAR).checks.find((k) => k.key === 'semgrep').required, false, 'semgrep never counts as required');
  assert.equal(card(fx.PP_CLEAR).reviewer.name, 'Iris');
  assert.equal(card(fx.PP_CLEAR).fresh_eyes.checked, true);
  assert.equal(m.parsePrepushList({ cards: [fx.PP_CLEAR, { junk: 1 }, fx.PP_BLOCKED] }).length, 2);
});

test('status chip: plain while checking, ok when clear, rose while the bot fixes, amber only when the person must act', () => {
  const chip = (k) => m.statusChip(card(fx[k]));
  assert.equal(chip('PP_NO_CHANGE'), null);
  assert.deepEqual(chip('PP_CHECKING'), { tone: 'plain', text: 'Checking', led: 'run' });
  assert.deepEqual(chip('PP_CLEAR'), { tone: 'ok', text: 'Clear', led: null });
  assert.deepEqual(chip('PP_BLOCKED'), { tone: 'bad', text: '1 to fix', led: null });
  assert.deepEqual(chip('PP_NEEDS_YOU'), { tone: 'warn', text: 'Needs you', led: null });
  assert.deepEqual(chip('PP_FAILED_NEEDS_YOU'), { tone: 'warn', text: 'Needs you', led: null });
  assert.equal(chip('PP_FAILED').tone, 'plain');
  assert.equal(chip('PP_OVERRIDDEN').text, 'Pushed anyway');
  assert.equal(chip('PP_OVERRIDDEN').tone, 'plain');
  for (const k of Object.keys(fx.PP_ALL)) {
    const c = chip(k);
    if (c?.tone === 'warn') assert.equal(m.phaseOf(card(fx[k])), 'P4', `${k}: amber only in P4`);
  }
});

test('counts come from the server: open findings are its blockers, never a check that could not run', () => {
  assert.equal(m.openCount(card(fx.PP_NEEDS_YOU)), 1);
  assert.equal(m.openCount(card(fx.PP_FAILED)), 0, "kwant couldn't check is not a finding");
  assert.equal(m.openCount(card(fx.PP_REVIEW_ERROR)), 0, "a review that didn't finish is not a finding");
  assert.equal(m.inboxLine(card(fx.PP_NEEDS_YOU), 'Pixel'), "Pixel's push is waiting for you: 1 finding open");
  assert.equal(m.inboxLine(card(fx.PP_FAILED_NEEDS_YOU), 'Pixel'), "Pixel's push is waiting for you: the checks couldn't run");
  assert.equal(m.requiredPassed(card(fx.PP_CLEAR)), 5, 'semgrep not installed is not a passed check');
  assert.equal(m.clearedLine(card(fx.PP_CLEAR)), 'Cleared to push pixel/inbox-empty-state after 5 checks passed · 14:02');
  assert.equal(m.foldLine(card(fx.PP_NEEDS_YOU)), '4 checks passed · registry · tamper · rules · review');
  assert.equal(m.overriddenLine(card(fx.PP_OVERRIDDEN)), 'You let this push go at 14:09 with 1 finding open. It covers this commit only.');
});

test('a check that could not run never reads as passed', () => {
  for (const k of ['PP_FAILED', 'PP_REVIEW_ERROR', 'PP_FAILED_NEEDS_YOU']) {
    const c = card(fx[k]);
    for (const row of c.checks.filter((r) => r.state === 'error')) {
      assert.equal(m.STATE_LED[row.state], 'off');
      assert.equal(m.STATE_WORD[row.state], 'could not run');
      assert.ok(!m.passedRows(c).includes(row));
    }
  }
  const out = html(m.PrepushCard, { card: card(fx.PP_FAILED), bot: 'Pixel' });
  assert.doesNotMatch(out, /data-state="pass"/);
  assert.equal((out.match(/could not run:/g) || []).length, 6);
});

// ---------------------------------------------------------------------------------------------------
// Rendering, one state at a time

test('P0 nothing to push: one grey status line, no checks', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_NO_CHANGE), bot: 'Pixel' });
  assert.match(out, /data-prepush-phase="P0"/);
  assert.match(out, /role="status"/);
  assert.match(out, /has no new commits\. Nothing was pushed\./);
  assert.doesNotMatch(out, /data-check=/);
});

test('P1 running: every row runs with a static LED, the chip alone is live', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_CHECKING), bot: 'Pixel' });
  assert.match(out, /Before Pixel pushes/);
  assert.equal((out.match(/data-state="running"/g) || []).length, 6);
  assert.equal((out.match(/aria-live="polite"/g) || []).length, 1, 'only the status chip is live');
  assert.match(out, /data-prepush-chip[^>]*aria-live="polite"/);
  assert.match(out, /dv-led dv-led-still/);
  assert.match(out, /The push waits until these finish/);
  assert.doesNotMatch(out, /Push anyway/);
});

test('P2 clear: a card seen live shows Clear, then folds to P2b after about 2 s; history starts folded', async () => {
  const live = await mount(m.PrepushCard, { card: card(fx.PP_CLEAR), bot: 'Pixel', live: true });
  assert.equal(live.host.querySelector('[data-prepush-phase]').dataset.prepushPhase, 'P2');
  assert.match(text(live.host), /All clear\. Pixel can push now\./);
  assert.doesNotMatch(text(live.host), /Pushing now|Pushed /, 'kwant never sees the push itself');
  await new Promise((r) => setTimeout(r, 2100));
  await settle();
  assert.equal(live.host.querySelector('[data-prepush-phase]').dataset.prepushPhase, 'P2b');

  const hist = await mount(m.PrepushCard, { card: card(fx.PP_CLEAR), bot: 'Pixel' });
  assert.equal(hist.host.querySelector('[data-prepush-phase]').dataset.prepushPhase, 'P2b');
  assert.match(text(hist.host), /Cleared to push pixel\/inbox-empty-state after 5 checks passed · 14:02/);
  await click(button(hist.host, 'Show checks'));
  assert.equal(hist.host.querySelector('[data-prepush-phase]').dataset.prepushPhase, 'P2');
  assert.equal(hist.host.querySelectorAll('[data-check]').length, 6);
});

test('P3 blocked: rose chip, the bot is fixing, a plain Push anyway, no amber', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_BLOCKED), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  assert.match(out, /kw-chip kw-chip-bad[^"]*"[^>]*>1 to fix/);
  assert.match(out, /Pixel is fixing it\./, 'one blocker reads it (#475 Mira Later 1)');
  assert.match(out, /Try 1 of 2\./);
  assert.match(out, />Push anyway…</);
  assert.doesNotMatch(out, /amber/);
  assert.doesNotMatch(out, /role="alert"/);
  assert.doesNotMatch(out, /always|skip this check/i);
});

test('P4 needs you: amber box with role=alert, retry first, amber-outline Push anyway', async () => {
  const calls = [];
  const { host } = await mount(m.PrepushCard, { card: card(fx.PP_NEEDS_YOU), bot: 'Pixel', onRetry: async (c) => calls.push(['retry', c.id]), onOverride: async () => {} });
  const box = host.querySelector('[data-needs-you]');
  assert.equal(box.getAttribute('role'), null, 'a card that mounts in P4 does not re-announce (Mira Later 5)');
  assert.match(text(box), /Pixel couldn't clear 1 finding after 2 tries\. The push is waiting for you\. It's also in your Inbox\./);
  const btns = [...box.querySelectorAll('button')].map(text);
  assert.deepEqual(btns, ['Ask Pixel to try again', 'Push anyway…']);
  assert.match(box.querySelectorAll('button')[1].className, /border-amber-400/);
  for (const b of box.querySelectorAll('button')) assert.match(b.className, /min-h-11/, '44 px on phone');
  await click(button(host, 'Ask Pixel to try again'));
  assert.deepEqual(calls, [['retry', 'pp_needs']]);
});

test('P4 after failed checks: the primary is Run the checks again', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_FAILED_NEEDS_YOU), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  assert.match(out, /Pixel&#x27;s push couldn&#x27;t be checked after 2 tries\.<\/strong> The push is waiting for you\./);
  assert.match(out, />Run the checks again</);
});

test('P5 push anyway: required, autofocused, labelled; disabled under 10 characters; Cancel goes back', async () => {
  const sent = [];
  const { host } = await mount(m.PrepushCard, { card: card(fx.PP_NEEDS_YOU), bot: 'Pixel', onRetry: async () => {}, onOverride: async (c, r) => sent.push([c.id, r]) });
  await click(button(host, 'Push anyway…'));
  const form = host.querySelector('[data-override-form]');
  assert.ok(form);
  assert.equal(host.querySelector('[data-prepush-phase]').dataset.prepushPhase, 'P5');
  const area = form.querySelector('textarea');
  assert.equal(document.activeElement, area, 'autofocused');
  assert.ok(area.required);
  assert.match(area.className, /text-base/, 'no iOS zoom');
  assert.equal(form.querySelector(`label[for="${area.id}"]`).textContent, 'Your reason');
  const hint = document.getElementById(area.getAttribute('aria-describedby'));
  assert.match(hint.textContent, /at least 10 characters\. This push only; next time the checks run again\./);
  assert.doesNotMatch(text(form), /pull request/, 'the reason is not copied into the PR yet (contract q10)');
  const go = [...form.querySelectorAll('button')].find((b) => text(b) === 'Push anyway');
  assert.match(go.className, /bg-amber-600/);
  assert.equal(go.disabled, true);
  await type(area, '   short   ');
  assert.equal(go.disabled, true, '"short" trimmed is under 10');
  await type(area, 'revoked test key');
  assert.equal(go.disabled, false);
  await click(button(form, 'Cancel'));
  assert.equal(host.querySelector('[data-override-form]'), null);
  assert.equal(host.querySelector('[data-prepush-phase]').dataset.prepushPhase, 'P4', 'Cancel returns to P4');
  await click(button(host, 'Push anyway…'));
  await type(host.querySelector('textarea'), 'the key is a revoked test key');
  host.querySelector('[data-override-form]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(sent, [['pp_needs', 'the key is a revoked test key']]);
});

test('P5 shows the server\'s refusal and keeps the reason', async () => {
  const { host } = await mount(m.PrepushCard, {
    card: card(fx.PP_NEEDS_YOU), bot: 'Pixel', startOverride: true,
    onOverride: async () => { throw new Error('Give a reason of at least 10 characters.'); },
  });
  await type(host.querySelector('textarea'), '0123456789');
  host.querySelector('[data-override-form]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.match(text(host.querySelector('p[role="alert"]')), /at least 10 characters/);
  assert.equal(host.querySelector('textarea').value, '0123456789');
});

test('P6 pushed anyway: neutral chip, the time and the quoted reason; open findings stay rose; no actions', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_OVERRIDDEN), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  assert.match(out, /class="kw-chip shrink-0[^"]*"[^>]*>Pushed anyway/);
  assert.match(out, /You let this push go at 14:09 with 1 finding open\./);
  assert.match(out, /<blockquote[^>]*>.*The key is a revoked test key/);
  assert.match(out, /data-check="secrets" data-state="hit"/);
  assert.doesNotMatch(out, /<button[^>]*>Push anyway/);
  assert.doesNotMatch(out, /Ask Pixel/);
});

test('P7 a check could not run: plain buttons, Run the checks again', async () => {
  const calls = [];
  const { host } = await mount(m.PrepushCard, { card: card(fx.PP_FAILED), bot: 'Pixel', onRetry: async (c) => calls.push(c.id), onOverride: async () => {} });
  assert.match(text(host), /Couldn't check/);
  assert.match(text(host), /A check couldn't run, so this push can't count as checked\./);
  assert.ok(!host.innerHTML.includes('amber'));
  await click(button(host, 'Run the checks again'));
  assert.deepEqual(calls, ['pp_failed']);
});

test('retried: one grey line', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_RETRIED), bot: 'Pixel' });
  assert.match(out, /data-prepush-phase="RT"/);
  assert.match(out, /You asked for a new check of .*pixel\/inbox-empty-state.*Pixel&#x27;s next push of this commit is checked again from try 1\./);
});

test('read-only cards (no handlers) show no buttons', () => {
  for (const k of ['PP_BLOCKED', 'PP_NEEDS_YOU', 'PP_FAILED']) {
    const out = html(m.PrepushCard, { card: card(fx[k]), bot: 'Pixel' });
    assert.doesNotMatch(out, /<button[^>]*>(Push anyway…|Ask Pixel|Run the checks again)/, k);
  }
});

test('rows: names, tool lines, hidden state words, Show opens the findings behind a hit', async () => {
  const { host } = await mount(m.PrepushCard, { card: card(fx.PP_MANY), bot: 'Pixel' });
  const rows = [...host.querySelectorAll('[data-check]')];
  assert.deepEqual(rows.map((r) => r.dataset.check), ['secrets', 'pkgs', 'tests', 'risk', 'review', 'semgrep']);
  assert.match(text(rows[4]), /Fresh-eyes review.*Iris on claude.*1 finding to fix/);
  assert.match(text(rows[5]), /semgrep · optional/);
  assert.match(text(rows[0]), /^passed:/);
  assert.match(text(rows[2]), /^found a problem:/);
  assert.equal(rows[3].querySelector('button'), null, 'risk has nothing to Show');
  const show = rows[4].querySelector('button');
  assert.equal(show.getAttribute('aria-expanded'), 'false');
  await click(show);
  assert.equal(show.getAttribute('aria-expanded'), 'true');
  assert.match(text(rows[4]), /Sign-in redirect drops the return path/);
  assert.match(text(rows[4]), /frontend\/src\/auth\.ts:42/);
  await click(rows[5].querySelector('button'));
  assert.match(text(rows[5]), /innerHTML set from a variable/, 'finding text is shown as text, never HTML');
});

test('phone: passing rows fold into one, findings always show, not-installed semgrep is hidden', async () => {
  const { host } = await mount(m.PrepushCard, { card: card(fx.PP_NEEDS_YOU), bot: 'Pixel' });
  const fold = host.querySelector('[data-fold]');
  assert.match(fold.className, /sm:hidden/, 'the fold row is phone only');
  assert.match(text(fold), /4 checks passed · registry · tamper · rules · review/);
  const hidden = (k) => /max-sm:hidden/.test(host.querySelector(`[data-check="${k}"]`).className);
  assert.equal(hidden('secrets'), false, 'the finding stays');
  assert.equal(hidden('pkgs'), true);
  assert.equal(hidden('semgrep'), true);
  const t = fold.querySelector('button');
  await click(t);
  assert.equal(t.getAttribute('aria-expanded'), 'true');
  assert.equal(hidden('pkgs'), false);
  assert.equal(hidden('semgrep'), true, 'not installed stays hidden on phone');
  // P1 and P2 never fold
  assert.equal(html(m.PrepushCard, { card: card(fx.PP_CHECKING), bot: 'Pixel' }).includes('data-fold'), false);
});

test('the card never renders HTML from data', () => {
  const evil = { ...fx.PP_BLOCKED, branch: '<img src=x onerror=alert(1)>', blockers: [{ source: 'gitleaks', severity: 'critical', title: '<b>x</b>', file: null, line: null }] };
  const out = html(m.PrepushCard, { card: card(evil), bot: '<i>Pixel</i>' });
  assert.doesNotMatch(out, /<img|<b>x|<i>Pixel/);
});

// ---------------------------------------------------------------------------------------------------
// Thread, Inbox and the event

test('threadCards: this bot only, newest three, oldest first', () => {
  const cards = ['a', 'b', 'c', 'd'].map((id, i) => card({ ...fx.PP_CLEAR, id, created_at: 1000 + i }));
  cards.push(card({ ...fx.PP_CLEAR, id: 'other', kwant_id: 'k_vega', created_at: 5000 }));
  assert.deepEqual(m.threadCards(cards, 'k_pixel').map((c) => c.id), ['b', 'c', 'd']);
});

test('Inbox: one amber item per card that needs you; it resolves when the card moves on', async () => {
  const opened = [];
  const cards = [card(fx.PP_NEEDS_YOU), card(fx.PP_BLOCKED), card(fx.PP_CLEAR), card(fx.PP_FAILED_NEEDS_YOU)];
  const { host, rerender } = await mount(m.PrepushNeedsYou, { cards, names: { k_pixel: 'Pixel' }, onOpen: (c) => opened.push(c.id) });
  const items = [...host.querySelectorAll('[data-needs-prepush-item]')];
  assert.equal(items.length, 2);
  assert.match(text(items[0]), /Needs you.*Pixel's push is waiting for you/);
  await click(items[0]);
  assert.equal(opened.length, 1);
  await rerender({ cards: [card(fx.PP_OVERRIDDEN), card(fx.PP_RETRIED)], names: { k_pixel: 'Pixel' }, onOpen: () => {} });
  assert.equal(host.querySelector('[data-needs-prepush]'), null);
});

test('usePrepushCards: lists once, refetches one card per dev_prepush event, and marks a card that turned clear as live', async () => {
  const calls = [];
  let next = fx.PP_CHECKING;
  const api = {
    list: async (s) => { calls.push(['list', s]); return [card(fx.PP_CHECKING)]; },
    get: async (id) => { calls.push(['get', id]); return card({ ...next, id }); },
  };
  let state;
  const Probe = () => { state = m.usePrepushCards('s_demo', true, api); return null; };
  await mount(Probe, {});
  assert.deepEqual(calls, [['list', 's_demo']]);
  assert.equal(state.cards[0].status, 'checking');
  window.dispatchEvent(new window.CustomEvent(m.PREPUSH_EVENT, { detail: { space_id: 'other', id: 'pp_check' } }));
  await settle();
  assert.equal(calls.length, 1, 'another space is ignored');
  next = fx.PP_CLEAR;
  window.dispatchEvent(new window.CustomEvent(m.PREPUSH_EVENT, { detail: { space_id: 's_demo', id: 'pp_check', status: 'clear' } }));
  await settle();
  assert.deepEqual(calls[1], ['get', 'pp_check']);
  assert.equal(state.cards.length, 1);
  assert.equal(state.cards[0].status, 'clear');
  assert.ok(state.live.has('pp_check'));
});

test('usePrepushCards does nothing with Dev mode off', async () => {
  const calls = [];
  const api = { list: async () => { calls.push('list'); return []; }, get: async () => { calls.push('get'); return null; } };
  const Probe = () => { m.usePrepushCards('s_demo', false, api); return null; };
  await mount(Probe, {});
  window.dispatchEvent(new window.CustomEvent(m.PREPUSH_EVENT, { detail: { space_id: 's_demo', id: 'x' } }));
  await settle();
  assert.deepEqual(calls, []);
});

test('API: override trims the reason; 400 reason_needed and 409 read plainly', async () => {
  const sent = [];
  const realFetch = g.fetch;
  const respond = [];
  g.fetch = async (url, init) => {
    sent.push([String(url), init?.method, init?.body]);
    const [status, body] = respond.shift();
    return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  };
  try {
    respond.push([200, fx.PP_OVERRIDDEN]);
    const c = await m.prepushApi.override('pp_needs', '  the key is revoked  ');
    assert.equal(c.status, 'overridden');
    assert.equal(sent[0][0], '/api/dev/prepush/pp_needs/override');
    assert.equal(sent[0][1], 'POST');
    assert.deepEqual(JSON.parse(sent[0][2]), { reason: 'the key is revoked' });
    respond.push([400, { error: 'too short', code: 'reason_needed' }]);
    await assert.rejects(m.prepushApi.override('pp_needs', 'short'), /at least 10 characters/);
    respond.push([409, { error: 'only a blocked or failed card can be checked again', code: 'conflict' }]);
    await assert.rejects(m.prepushApi.retry('pp_clear'), /moved on/);
    respond.push([200, { cards: [fx.PP_CLEAR] }]);
    assert.equal((await m.prepushApi.list('s demo')).length, 1);
    assert.equal(sent.at(-1)[0], '/api/spaces/s%20demo/dev/prepush');
  } finally {
    g.fetch = realFetch;
  }
});

test('reasonOk: 10 trimmed characters, one push only', () => {
  assert.equal(m.reasonOk('123456789'), false);
  assert.equal(m.reasonOk('  123456789  '), false);
  assert.equal(m.reasonOk('1234567890'), true);
  assert.equal(m.MIN_REASON, 10);
});

// ---------------------------------------------------------------------------------------------------
// Checked with fresh eyes

test('fresh-eyes label: R1, R2 and R3 follow the contract', () => {
  const p = (v) => m.chipState(m.parseFreshEyes(v));
  assert.equal(p(fx.FE_TICKET), 'R1');
  assert.equal(p(fx.FE_SAW_CHAT), 'R2');
  assert.equal(p(fx.FE_FAILED), 'R3', 'a failed review shows no chip, never R2');
  assert.equal(p(null), 'R3');
  assert.equal(p(undefined), 'R3');
  assert.equal(p({ saw: ['diff'] }), 'R3', 'no checked flag');
  assert.deepEqual(m.parseFreshEyes({ checked: true, saw: ['diff', 'tests', 'diff', 'chat'], spec_source: 'odd' }).saw, ['diff'], 'unknown parts dropped');
});

test('popover lines come from saw and spec_source; no test-results line', () => {
  const lines = (v, o = {}) => m.sawLines(m.parseFreshEyes(v), { author: 'Pixel', ...o }).map((l) => `${l.ok ? '+' : '-'} ${l.text}`);
  assert.deepEqual(lines(fx.FE_TICKET, { ref: 'KW-212', diff: { files: 4, add: 86, del: 21 } }), [
    "+ the ticket's brief and done-when lines (KW-212)",
    '+ the diff (4 files, +86 −21)',
    "- Not Pixel's conversation or reasoning",
  ]);
  assert.deepEqual(lines(fx.FE_TASK), ["+ the task's brief", '+ the diff', "- Not Pixel's conversation or reasoning"]);
  assert.deepEqual(lines(fx.FE_PR), ["+ only the PR's title and description, in the author's own words", '+ the diff', "- Not Pixel's conversation or reasoning"]);
  assert.deepEqual(lines(fx.FE_AUDIT_WRITTEN), ["+ the owner's written spec", '+ the result', "- Not Pixel's conversation or reasoning"]);
  for (const v of [fx.FE_TICKET, fx.FE_TASK, fx.FE_PR, fx.FE_AUDIT_WRITTEN]) assert.ok(lines(v).every((l) => !/test/.test(l)));
  assert.equal(m.popoverTitle('Iris'), 'Iris reviewed with fresh eyes. They saw only what a new teammate would get:');
});

test('R1 chip: a real button; opens a labelled dialog; Esc closes it and focus returns', async () => {
  const { host } = await mount(m.FreshEyesChip, { label: m.parseFreshEyes(fx.FE_TICKET), reviewer: 'Iris', author: 'Pixel', specRef: 'KW-212' });
  const chip = host.querySelector('[data-fresh-eyes="R1"]');
  assert.equal(chip.tagName, 'BUTTON');
  assert.equal(chip.type, 'button');
  assert.equal(chip.getAttribute('aria-expanded'), 'false');
  assert.match(chip.className, /kw-chip/);
  assert.doesNotMatch(chip.className, /holo|accent|kw-chip-ok/, 'plain grey, never earned-looking');
  assert.match(chip.className, /focus-visible:ring-sky-400/);
  chip.focus();
  await click(chip);
  const pop = host.querySelector('[role="dialog"]');
  assert.equal(pop.getAttribute('aria-label'), 'How this review was run');
  assert.equal(chip.getAttribute('aria-controls'), pop.id);
  assert.equal(chip.getAttribute('aria-expanded'), 'true');
  assert.match(text(pop), /Iris reviewed with fresh eyes\. They saw only what a new teammate would get:/);
  assert.match(text(pop), /Not Pixel's conversation or reasoning/);
  document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(host.querySelector('[role="dialog"]'), null);
  assert.equal(document.activeElement, chip);
});

test('R1 chip closes on a click outside and when focus leaves it', async () => {
  const { host } = await mount(m.FreshEyesChip, { label: m.parseFreshEyes(fx.FE_PR), reviewer: 'Iris', author: 'Pixel' });
  const chip = host.querySelector('button');
  await click(chip);
  assert.ok(host.querySelector('[role="dialog"]'));
  document.body.dispatchEvent(new window.MouseEvent('mousedown', { bubbles: true }));
  await settle();
  assert.equal(host.querySelector('[role="dialog"]'), null);
  await click(chip);
  const out = document.createElement('button');
  document.body.appendChild(out);
  chip.dispatchEvent(new window.FocusEvent('focusout', { bubbles: true, relatedTarget: out }));
  await settle();
  assert.equal(host.querySelector('[role="dialog"]'), null);
});

test('R2 is a grey chip that is not a button; R3 renders nothing', () => {
  const r2 = html(m.FreshEyesChip, { label: m.parseFreshEyes(fx.FE_SAW_CHAT), reviewer: 'Iris', author: 'Pixel' });
  assert.match(r2, /^<span data-fresh-eyes="R2" class="kw-chip">Saw the author&#x27;s chat<\/span>$/);
  assert.equal(html(m.FreshEyesChip, { label: m.parseFreshEyes(fx.FE_FAILED), reviewer: 'Iris', author: 'Pixel' }), '');
  assert.equal(html(m.FreshEyesChip, { label: null, reviewer: 'Iris', author: 'Pixel' }), '');
});

test('review card: the chip sits right after the engine chip; old cards show none', () => {
  const raw = { ...m.dfx.REVIEW_294, fresh_eyes: fx.FE_TICKET };
  const r = m.parseReview(raw);
  assert.equal(r.fresh_eyes.checked, true);
  const out = html(m.ReviewCard, { card: r, now: m.dfx.FIXTURE_NOW });
  const engine = out.search(/(Different|Same) engine/);
  const fresh = out.indexOf('Checked with fresh eyes');
  assert.ok(engine > 0 && fresh > engine, 'after the engine chip');
  assert.equal(m.parseReview(m.dfx.REVIEW_294).fresh_eyes, null);
  assert.doesNotMatch(html(m.ReviewCard, { card: m.parseReview(m.dfx.REVIEW_294), now: m.dfx.FIXTURE_NOW }), /fresh eyes/);
});

test('semgrep tile: the real run, not installed is optional, and no more "in 0.7.x"', () => {
  assert.deepEqual(m.semgrepTile({ state: 'pass', count: 0, reason: null }), { state: 'pass', text: 'No matches' });
  assert.deepEqual(m.semgrepTile({ state: 'hit', count: 2, reason: null }), { state: 'hit', text: '2 matches · see finding' });
  assert.deepEqual(m.semgrepTile({ state: 'not_run', count: 0, reason: "semgrep isn't installed on the server" }), { state: 'later', text: 'Not installed. Optional.' });
  assert.deepEqual(m.semgrepTile({ state: 'not_run', count: 0, reason: 'the repo has no .semgrep.yml of its own' }), { state: 'later', text: 'Not installed. Optional.' });
  assert.deepEqual(m.semgrepTile({ state: 'not_run', count: 0, reason: 'semgrep took longer than 2 minutes' }), { state: 'not_run', text: "Didn't run: semgrep took longer than 2 minutes" });
  assert.equal(m.semgrepTile(null).state, 'later');
  const out = html(m.ReviewCard, { card: m.parseReview(m.dfx.REVIEW_294), now: m.dfx.FIXTURE_NOW });
  assert.doesNotMatch(out, /data-sec="Semgrep"[^]*?in 0\.7\.x/);
  const r = m.parseReview({ ...m.dfx.REVIEW_294, security: { ...m.dfx.REVIEW_294.security, semgrep: { state: 'hit', count: 1 } } });
  assert.deepEqual(r.security.semgrep, { state: 'hit', count: 1, reason: null });
});

// ---------------------------------------------------------------------------------------------
// #458 follow-ups (Ada review-458 Later 1–9, Mira ui-check-458 Later 1–6, Rigel qa-458 1–2)

test('Mira 5: role=alert only on the move into P4, not on mount or after Cancel', async () => {
  const { host, rerender } = await mount(m.PrepushCard, { card: card(fx.PP_BLOCKED), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  await rerender({ card: card(fx.PP_NEEDS_YOU), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  assert.equal(host.querySelector('[data-needs-you]').getAttribute('role'), 'alert');
  await click(button(host, 'Push anyway'));
  await click(button(host, 'Cancel'));
  assert.equal(host.querySelector('[data-needs-you]').getAttribute('role'), null);
});

test('Ada 3: Cancel and Esc put focus back on the Push anyway button that opened the form', async () => {
  const { host } = await mount(m.PrepushCard, { card: card(fx.PP_NEEDS_YOU), bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  await click(button(host, 'Push anyway'));
  await click(button(host, 'Cancel'));
  assert.equal(document.activeElement, button(host, 'Push anyway'));
  await click(button(host, 'Push anyway'));
  host.querySelector('textarea').dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(host.querySelector('[data-override-form]'), null);
  assert.equal(document.activeElement, button(host, 'Push anyway'));
});

test('Ada 6: the reason counts characters as the server does, not UTF-16 units', () => {
  assert.equal(m.reasonLength('🙂🙂🙂🙂🙂'), 5);
  assert.equal(m.reasonOk('🙂🙂🙂🙂🙂'), false, '5 emoji are 10 UTF-16 units but 5 characters');
  assert.equal(m.reasonOk('  revoked test key  '), true);
});

test('Ada 2: an unknown status shows as a read-only grey line, never dropped or actionable', async () => {
  const c = card({ ...fx.PP_OVERRIDDEN, status: 'pushed' });
  assert.equal(m.phaseOf(c), 'UN');
  assert.equal(m.statusChip(c), null);
  assert.equal(m.canOverride(c), false);
  assert.equal(c.needs_you, false);
  assert.equal(m.parsePrepushList({ cards: [{ ...fx.PP_CLEAR, status: 'pushed' }] }).length, 1);
  const out = html(m.PrepushCard, { card: c, bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  assert.match(out, /data-prepush-phase="UN"/);
  assert.match(out, /can&#x27;t show where it is yet/);
  assert.doesNotMatch(out, /<button/);
});

test('Mira 6: P2b renders the branch as code', () => {
  const out = html(m.PrepushCard, { card: card(fx.PP_CLEAR), bot: 'Pixel' });
  assert.match(out, /Cleared to push <code class="dv-sha dv-sha-wrap">/);
  assert.equal(m.clearedLine(card(fx.PP_CLEAR)).startsWith('Cleared to push '), true);
});

test('Mira 4: a hit the server does not count as a blocker says "doesn\'t block"; a blocker row does not', () => {
  const raw = {
    ...fx.PP_BLOCKED,
    checks: fx.PP_BLOCKED.checks.map((k) => (k.key === 'risk' ? { ...k, state: 'hit', text: 'Touches auth code' } : k)),
  };
  const c = card(raw);
  assert.equal(m.rowBlocks(c, 'risk'), false, 'risk never blocks on its own (contract §4.3)');
  assert.equal(m.rowBlocks(c, 'semgrep'), false);
  const out = html(m.PrepushCard, { card: c, bot: 'Pixel' });
  const risk = out.match(/<li data-check="risk"[\s\S]*?<\/li>/)[0];
  assert.match(risk, /doesn&#x27;t block/);
  for (const b of c.blockers) {
    const key = { gitleaks: 'secrets', packages: 'pkgs', tamper: 'tests', review: 'review' }[b.source];
    if (!key) continue;
    assert.equal(m.rowBlocks(c, key), true, `${key} holds the push`);
    const row = out.match(new RegExp(`<li data-check="${key}"[\\s\\S]*?</li>`))[0];
    assert.doesNotMatch(row, /doesn&#x27;t block/);
  }
});

test('Ada 5: a slow list never replaces a card an event already refreshed', () => {
  const old = card({ ...fx.PP_BLOCKED, updated_at: 1000 });
  const fresh = card({ ...fx.PP_BLOCKED, status: 'overridden', updated_at: 2000 });
  const other = card({ ...fx.PP_CLEAR, id: 'pp-other', updated_at: 5 });
  const merged = m.mergeList([fresh], [old, other]);
  assert.equal(merged.find((c) => c.id === fresh.id).status, 'overridden');
  assert.equal(merged.length, 2);
  assert.equal(m.mergeList([old], [fresh])[0].status, 'overridden', 'a newer list copy wins');
});

test('Ada 1: a failed list load is tried again on the next event and on window focus', async () => {
  let lists = 0;
  const api = {
    list: async () => {
      lists += 1;
      if (lists === 1) throw new Error('offline');
      return [card(fx.PP_NEEDS_YOU)];
    },
    get: async () => card(fx.PP_NEEDS_YOU),
  };
  let state;
  const Probe = () => {
    state = m.usePrepushCards('space-1', true, api);
    return null;
  };
  await mount(Probe, {});
  assert.equal(lists, 1);
  assert.equal(state.cards.length, 0);
  window.dispatchEvent(new window.Event('focus'));
  await settle();
  assert.equal(lists, 2, 'focus loads the list again after a failure');
  assert.equal(state.cards.length, 1);
  window.dispatchEvent(new window.Event('focus'));
  await settle();
  assert.equal(lists, 2, 'no reload once the list loaded');
});

test('Ada 7: when only the chat message failed, the card says the change applied', async () => {
  const { host } = await mount(m.PrepushCard, {
    card: card(fx.PP_NEEDS_YOU),
    bot: 'Pixel',
    onRetry: async () => {},
    onOverride: async () => {
      throw Object.assign(new Error(m.messageNotSent('Pixel')), { status: 0, code: 'message_not_sent' });
    },
  });
  await click(button(host, 'Push anyway'));
  await type(host.querySelector('textarea'), 'the key is a revoked test key');
  host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.match(text(host.querySelector('[role="alert"]')), /Done, but the message to Pixel didn't send\./);
  assert.equal(m.messageNotSent('Pixel'), "Done, but the message to Pixel didn't send. Tell Pixel in the chat.");
});

test('Mira 1: on desktop the popover sits under its own chip; Mira 2 and 3: chip LED gap, 44 px hit area', async () => {
  const out = html(m.FreshEyesChip, { label: card(fx.PP_CLEAR).fresh_eyes, reviewer: 'Iris', author: 'Pixel' });
  assert.match(out, /data-fresh-eyes="R1"/);
  const src = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/components/dev/FreshEyesChip.tsx', import.meta.url), 'utf8'));
  assert.doesNotMatch(src, /sm:left-0/);
  assert.match(src, /style=\{\{ left \}\}/);
  const css = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8'));
  assert.match(css, /@media \(max-width: 639px\) \{ \.fe-chip::after \{ inset: -10px -6px; \} \}/);
  assert.match(css, /\.dv-branch\.dv-branch-wrap \.h \{ white-space: normal;/, 'Ada 9: a long branch wraps instead of being cut off');
  const p1 = html(m.PrepushCard, { card: card(fx.PP_CHECKING), bot: 'Pixel' });
  assert.match(p1, /data-prepush-chip[^>]*>.*?class="[^"]*dv-led-still mx-0\.5/);
});

test('Rigel 1: refresh() fetches one card again, so a 409 leaves the card where it really is', async () => {
  let gets = 0;
  const api = { list: async () => [card(fx.PP_NEEDS_YOU)], get: async () => (gets++, card({ ...fx.PP_NEEDS_YOU, status: 'overridden', updated_at: 9e12 })) };
  let state;
  const Probe = () => {
    state = m.usePrepushCards('space-1', true, api);
    return null;
  };
  await mount(Probe, {});
  assert.equal(state.cards[0].status, 'blocked');
  await state.refresh(state.cards[0].id);
  await settle();
  assert.equal(gets, 1);
  assert.equal(state.cards[0].status, 'overridden');
});

test('#475 review 1: a 409 line survives the refetch and goes on the next change', async () => {
  const moved = 'This card moved on in the meantime. It shows where it is now.';
  const props = (c) => ({
    card: card(c),
    bot: 'Pixel',
    onRetry: async () => {
      throw new m.DevModeError(moved, 409, 'conflict');
    },
    onOverride: async () => {},
  });
  const { host, rerender } = await mount(m.PrepushCard, props(fx.PP_NEEDS_YOU));
  await click(button(host, 'Ask Pixel to try again'));
  assert.equal(text(host.querySelector('[role="alert"]')), moved);
  await rerender(props({ ...fx.PP_NEEDS_YOU, status: 'overridden', override: fx.PP_OVERRIDDEN.override }));
  assert.equal(text(host.querySelector('[role="alert"]')), moved, 'still there after the refetch');
  await rerender(props({ ...fx.PP_CHECKING, id: fx.PP_NEEDS_YOU.id }));
  assert.equal(host.querySelector('[role="alert"]'), null, 'gone on the next change (checking)');
  await rerender(props({ ...fx.PP_CLEAR, id: fx.PP_NEEDS_YOU.id }));
  assert.doesNotMatch(text(host), /moved on in the meantime/);
});

test('#475 review 1: a 409 after the card already moved, and message_not_sent, keep nothing', async () => {
  let r;
  const { host, rerender } = await mount(m.PrepushCard, {
    card: card(fx.PP_NEEDS_YOU),
    bot: 'Pixel',
    onRetry: async () => {
      await r(card({ ...fx.PP_NEEDS_YOU, status: 'overridden', override: fx.PP_OVERRIDDEN.override }));
      throw new m.DevModeError('This card moved on in the meantime. It shows where it is now.', 409, 'conflict');
    },
    onOverride: async () => {},
  });
  r = (c) => rerender({ card: c, bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  await click(button(host, 'Ask Pixel to try again'));
  await settle(); // onRetry's own rerender settles in parallel with click's; wait for the throw too
  assert.match(text(host.querySelector('[role="alert"]')), /moved on/);
  await r(card({ ...fx.PP_CHECKING, id: fx.PP_NEEDS_YOU.id }));
  assert.equal(host.querySelector('[role="alert"]'), null);
  const src = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/components/dev/PrepushCard.tsx', import.meta.url), 'utf8'));
  assert.doesNotMatch(src, /code === 'message_not_sent'/, 'message_not_sent is not kept');
});

test('#475 review 2: a retry that applied but whose message failed shows the alert on the retried line', async () => {
  let r;
  const { host, rerender } = await mount(m.PrepushCard, {
    card: card(fx.PP_NEEDS_YOU),
    bot: 'Pixel',
    onRetry: async () => {
      await r(card(fx.PP_RETRIED));
      throw new m.DevModeError(m.messageNotSent('Pixel'), 0, 'message_not_sent');
    },
    onOverride: async () => {},
  });
  r = (c) => rerender({ card: c, bot: 'Pixel', onRetry: async () => {}, onOverride: async () => {} });
  await click(button(host, 'Ask Pixel to try again'));
  await settle(); // onRetry's own rerender settles in parallel with click's; wait for the throw too
  assert.ok(host.querySelector('[data-prepush-phase="RT"]'), 'the card is the retried line');
  const alert = host.querySelector('[data-prepush-phase="RT"] [role="alert"]');
  assert.ok(alert, 'alert under the RT line');
  assert.equal(text(alert), "Done, but the message to Pixel didn't send. Tell Pixel in the chat.");
  assert.match(alert.className, /text-bad/);
  await r(card({ ...fx.PP_CHECKING, id: fx.PP_RETRIED.id }));
  assert.equal(host.querySelector('[role="alert"]'), null, 'cleared by the next change');
});

test('#475 Mira must-fix 1: the branch on one-line cards wraps; "fixing it" with one blocker', async () => {
  const long = 'pixel/inbox-empty-state-for-new-spaces-with-no-bots';
  for (const c of [fx.PP_NO_CHANGE, fx.PP_RETRIED, { ...fx.PP_CLEAR, status: 'pushed' }, fx.PP_CLEAR]) {
    const out = html(m.PrepushCard, { card: card({ ...c, branch: long }), bot: 'Pixel' });
    assert.match(out, new RegExp(`<code class="dv-sha dv-sha-wrap">${long}</code>`), c.id);
  }
  const css = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8'));
  assert.match(css, /\.dv-sha\.dv-sha-wrap \{ display: inline; height: auto; white-space: normal; overflow-wrap: anywhere;/);
  const one = card({ ...fx.PP_BLOCKED, blockers: fx.PP_BLOCKED.blockers.slice(0, 1) });
  assert.equal(m.openCount(one), 1);
  assert.match(html(m.PrepushCard, { card: one, bot: 'Pixel' }), /Pixel is fixing it\./);
  const many = card(fx.PP_MANY);
  if (m.openCount(many) > 1) assert.match(html(m.PrepushCard, { card: many, bot: 'Pixel' }), /Pixel is fixing them\./);
});
