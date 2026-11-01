// Truth Ledger, Phase C2 (docs/mission-control.md, contract §2): the board's Facts section shows the
// facts bots read, labelled unconfirmed or disputed, open disputes as Needs you (keep one, or not a
// conflict), and which tasks rely on a fact. A decision another tab made first (409) says so; changes
// arrive live. With the ledger off or in shadow, or on an older server, nothing new shows.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const b = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { FactsLedger, FactNeedsYouAction } from './src/components/FactsLedger';
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { FactsLedger, FactNeedsYouAction } from './src/components/FactsLedger';
  export * from './src/truthLedger';
  export { MissionError, errorFrom, emitMission } from './src/missions';
  export { TOAST_KINDS } from './src/continuity';
  export const ledgerHtml = (p) => renderToStaticMarkup(React.createElement(FactsLedger, p));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const NOW = Date.parse('2026-11-01T12:00:00Z');
const names = { k_sandy: 'Sandy', k_cody: 'Cody' };

const fact = (over = {}) => ({
  id: 'f1', space_id: 's1', scope: 'space', scope_id: 's1', statement: 'Launch day is 12 November.', subject: 'launch day',
  source_kind: 'bot', source_ref: null, author_kwant_id: 'k_sandy', trust: 'bot', status: 'accepted', superseded_by: null,
  valid_from: '2026-10-30T12:00:00Z', valid_until: null, memory_id: null, pinned: false, decided_by: null, decided_at: null,
  created_at: '2026-10-30T12:00:00Z', updated_at: '2026-10-30T12:00:00Z', ...over,
});
const A = fact({ id: 'fa', statement: 'Launch day is 12 November.', status: 'disputed' });
const B = fact({ id: 'fb', statement: 'Launch day is 19 November.', status: 'disputed', author_kwant_id: 'k_cody', scope: 'bot', scope_id: 'k_cody', created_at: '2026-10-31T12:00:00Z' });
const dispute = (over = {}) => ({
  id: 'd1', fact_a: 'fa', fact_b: 'fb', cause: 'numbers_differ', status: 'open', winner: null, resolved_by: null,
  created_at: '2026-11-01T11:00:00Z', resolved_at: null, facts: [A, B], ...over,
});
const P = fact({ id: 'fp', statement: 'The newsletter goes out on Fridays.', status: 'proposed', created_at: '2026-11-01T10:00:00Z' });
const OLD = fact({ id: 'fo', statement: 'Prices start at $9.', status: 'superseded' });
const facts = [fact(), A, B, P, OLD];
const initial = (over = {}) => ({ setting: 'on', facts, disputes: [dispute()], ...over });

function fakeApi(over = {}) {
  const calls = [];
  return {
    calls,
    list: async () => (calls.push(['list']), { setting: 'on', facts }),
    disputes: async (s) => (calls.push(['disputes', s]), { setting: 'on', disputes: s === 'open' ? [dispute()] : [dispute({ status: 'resolved', winner: 'fb', resolved_by: 'person' })] }),
    get: async (id) => (calls.push(['get', id]), {
      fact: facts.find((f) => f.id === id) ?? fact({ id }),
      events: [], evidence: [{ fact_id: id, evidence_id: 'e1', check_id: 'c1', message_id: null, added_by: 'check', added_at: '2026-10-30T12:00:00Z' }],
      citations: [{ fact_id: id, cited_by_kind: 'task', cited_by_id: 't1', created_at: '2026-10-31T12:00:00Z' }, { fact_id: id, cited_by_kind: 'fact', cited_by_id: 'fx', created_at: '2026-10-31T12:00:00Z' }],
      disputes: [],
    }),
    resolve: async (id, winner, key) => (calls.push(['resolve', id, winner, key]), dispute({ status: 'resolved', winner, resolved_by: 'person' })),
    dismiss: async (id, key) => (calls.push(['dismiss', id, key]), dispute({ status: 'dismissed' })),
    accept: async (id, key) => (calls.push(['accept', id, key]), { fact: { ...P, status: 'accepted' }, events: [], evidence: [], citations: [], disputes: [] }),
    reject: async (id, key) => (calls.push(['reject', id, key]), { fact: { ...P, status: 'superseded' }, events: [], evidence: [], citations: [], disputes: [] }),
    uncite: async (id, taskId) => (calls.push(['uncite', id, taskId]), { ok: true }),
    ...over,
  };
}

async function mount(Component, props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = b.createRoot(host);
  root.render(b.React.createElement(Component, props));
  await settle();
  return {
    host,
    text: () => host.textContent.replace(/\s+/g, ' '),
    click: async (sel) => {
      const el = host.querySelector(sel);
      assert.ok(el, `no ${sel}`);
      el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
      await settle();
    },
    done: () => {
      root.unmount();
      host.remove();
    },
  };
}

test('with the ledger off or in shadow, or on an older server, the board shows nothing new', async () => {
  assert.equal(b.ledgerHtml({ names, initial: initial({ setting: 'off' }), api: fakeApi() }), '');
  assert.equal(b.ledgerHtml({ names, initial: initial({ setting: 'shadow' }), api: fakeApi() }), '');
  assert.equal(b.ledgerSetting('anything'), 'off');
  // An older server answers 404: still nothing, not an error.
  const gone = new b.MissionError('Not found', 404);
  const m = await mount(b.FactsLedger, { names, api: fakeApi({ list: async () => Promise.reject(gone), disputes: async () => Promise.reject(gone) }) });
  assert.equal(m.host.innerHTML, '');
  m.done();
  // Off from the server on load: nothing either.
  const off = await mount(b.FactsLedger, { names, api: fakeApi({ list: async () => ({ setting: 'off', facts: [] }), disputes: async () => ({ setting: 'off', disputes: [] }) }) });
  assert.equal(off.host.innerHTML, '');
  off.done();
});

test('labels: unconfirmed in amber, disputed in rose, replaced in grey, none for a fact that holds', () => {
  assert.deepEqual(b.factLabel('proposed'), { text: 'Unconfirmed', tone: 'amber' });
  assert.deepEqual(b.factLabel('disputed'), { text: 'Disputed', tone: 'rose' });
  assert.deepEqual(b.factLabel('superseded'), { text: 'Replaced', tone: 'zinc' });
  assert.equal(b.factLabel('accepted'), null);
  assert.equal(b.scopeText({ scope: 'bot', scope_id: 'k_cody' }, names), 'Cody’s note');
  assert.equal(b.scopeText({ scope: 'team', scope_id: 'tm' }, names, { tm: 'Design' }), 'Team Design');
  assert.equal(b.scopeText({ scope: 'space', scope_id: 's1' }), 'Everyone in this space');
  assert.equal(b.sourceText({ source_kind: 'person', trust: 'person', author_kwant_id: null }), 'You wrote it');
  assert.equal(b.sourceText({ source_kind: 'bot', trust: 'outside', author_kwant_id: 'k_sandy' }, names), 'Sandy, from something it read outside');
  assert.equal(b.citationText([]), 'No task relies on it yet.');
});

test('an open dispute is Needs you: both facts side by side, why, keep one, or not a conflict', () => {
  const html = b.ledgerHtml({ names, now: NOW, initial: initial(), api: fakeApi() });
  const t = text(html);
  assert.match(t, /Facts 4 in force · 1 unconfirmed · 2 disputed/);
  assert.match(t, /Needs you: one dispute to settle/);
  assert.match(t, /Two facts disagree They say nearly the same thing with different numbers\./);
  assert.match(t, /First fact: “Launch day is 12 November\.” Everyone in this space · Sandy saved it/);
  assert.match(t, /Second fact: “Launch day is 19 November\.” Cody’s note · Cody saved it/);
  assert.equal((html.match(/data-keep=/g) ?? []).length, 2);
  assert.match(html, /aria-label="Keep “Launch day is 12 November\.” and replace the other"/);
  assert.match(t, /Not a conflict/);
  assert.match(t, /Until you choose, bots see both, labelled disputed\./);
  // Named for screen readers by its title; the section by its heading.
  assert.match(html, /<li aria-labelledby="[^"]+"[^>]*data-dispute="d1" data-status="open"/);
  assert.match(html, /<section aria-labelledby="[^"]+"[^>]*data-facts-ledger/);
  // The facts list is behind a disclosure.
  assert.match(html, /aria-expanded="false"[^>]*data-facts-toggle/);
  assert.doesNotMatch(html, /data-fact="/);
});

test('the facts list: disputed, then unconfirmed, labelled; replaced ones only by filter', async () => {
  const html = b.ledgerHtml({ names, now: NOW, initial: initial({ disputes: [] }), api: fakeApi(), defaultOpen: true });
  const order = [...html.matchAll(/data-fact="([^"]+)"/g)].map((x) => x[1]);
  assert.deepEqual(order, ['fb', 'fa', 'fp', 'f1']);
  assert.match(html, /data-fact="fp" data-status="proposed".*?data-fact-label="proposed"[^>]*>Unconfirmed/);
  assert.match(html, /data-fact="fa" data-status="disputed".*?data-fact-label="disputed"[^>]*>Disputed/);
  assert.doesNotMatch(html, /Prices start at/);
  assert.match(html, /aria-pressed="true"[^>]*data-filter="all"/);
  assert.doesNotMatch(text(html), /Needs you/);

  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial({ disputes: [] }), api: fakeApi(), defaultOpen: true });
  await m.click('[data-filter="proposed"]');
  assert.deepEqual([...m.host.querySelectorAll('[data-fact]')].map((x) => x.dataset.fact), ['fp']);
  await m.click('[data-filter="disputed"]');
  assert.deepEqual([...m.host.querySelectorAll('[data-fact]')].map((x) => x.dataset.fact), ['fb', 'fa']);
  m.done();

  // An empty ledger says so.
  const empty = text(b.ledgerHtml({ names, initial: initial({ facts: [], disputes: [] }), api: fakeApi(), defaultOpen: true }));
  assert.match(empty, /Facts None saved yet/);
  assert.match(empty, /No facts yet\. When a bot saves a note or you write a shared fact, it shows here\./);
});

test('details show which tasks rely on a fact, its evidence, and open the task', async () => {
  const api = fakeApi();
  const opened = [];
  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial({ disputes: [] }), api, defaultOpen: true, taskTitles: { t1: 'Write the launch post' }, onOpenTask: (id) => opened.push(id) });
  const toggle = m.host.querySelector('[data-fact="f1"] [data-fact-toggle]');
  assert.equal(toggle.getAttribute('aria-expanded'), 'false');
  toggle.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.equal(toggle.getAttribute('aria-expanded'), 'true');
  assert.deepEqual(api.calls.at(-1), ['get', 'f1']);
  const d = m.host.querySelector('[data-fact-detail="f1"]').textContent;
  // A fact citing a fact is internal: one task, not two.
  assert.match(d, /1 task relies on it\./);
  assert.match(d, /Write the launch post/);
  assert.match(d, /Backed by 1 piece of evidence, including a check\./);
  m.host.querySelector('[data-fact-detail="f1"] ul button').dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
  assert.deepEqual(opened, ['t1']);
  m.done();
});

test('keeping one fact resolves the dispute once, then shows the outcome', async () => {
  const api = fakeApi();
  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial(), api });
  await m.click('[data-keep="fb"]');
  const r = api.calls.find((c) => c[0] === 'resolve');
  assert.equal(r[1], 'd1');
  assert.equal(r[2], 'fb');
  assert.ok(r[3], 'sends an idempotency key');
  assert.match(m.text(), /Settled: “Launch day is 19 November\.” stands; the other is replaced\./);
  assert.equal(m.host.querySelector('[data-keep]'), null);
  assert.match(m.text(), /All settled\./);
  m.done();
});

test('not a conflict dismisses; a failed press retries with the same key', async () => {
  let fail = true;
  const keys = [];
  const api = fakeApi({
    dismiss: async (id, key) => {
      keys.push(key);
      if (fail) {
        fail = false;
        throw new b.MissionError('Network down', 0);
      }
      return dispute({ status: 'dismissed' });
    },
  });
  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial(), api });
  await m.click('[data-dismiss]');
  assert.match(m.host.querySelector('[data-dispute-error]').textContent, /Network down/);
  await m.click('[data-dismiss]');
  assert.equal(keys.length, 2);
  assert.equal(keys[0], keys[1]);
  assert.match(m.text(), /Not a conflict: both stand, and this pair won’t be raised again\./);
  m.done();
});

test('a dispute someone else settled first (409) says so and shows what holds now', async () => {
  const conflict = b.errorFrom(409, { error: 'this dispute is already resolved', status: 'resolved' });
  assert.equal(conflict.code, 'resolved');
  assert.equal(b.disputeConflictText(conflict), 'Someone got there first: this one is already settled.');
  assert.equal(b.disputeConflictText(new b.MissionError('the Truth Ledger isn’t on', 409)), 'The Truth Ledger was switched off, so facts can’t be changed right now.');
  const api = fakeApi({ resolve: async () => Promise.reject(conflict) });
  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial(), api });
  await m.click('[data-keep="fa"]');
  assert.equal(m.host.querySelector('[role="alert"]').textContent.trim(), 'Someone got there first: this one is already settled.');
  assert.ok(api.calls.some((c) => c[0] === 'disputes' && c[1] === 'all'));
  assert.match(m.text(), /Settled: “Launch day is 19 November\.” stands/);
  m.done();
});

test('an unconfirmed fact can be confirmed or rejected; a 409 explains', async () => {
  const api = fakeApi();
  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial({ disputes: [] }), api, defaultOpen: true });
  await m.click('[data-fact="fp"] [data-fact-toggle]');
  assert.match(m.host.querySelector('[data-fact="fp"]').innerHTML, /role="group" aria-label="Unconfirmed: The newsletter goes out on Fridays\."/);
  await m.click('[data-confirm-fact]');
  const a = api.calls.find((c) => c[0] === 'accept');
  assert.equal(a[1], 'fp');
  assert.ok(a[2]);
  assert.match(m.text(), /Confirmed: bots now read it without the label\./);
  m.done();

  assert.equal(b.factConflictText(b.errorFrom(409, { error: 'this fact is already accepted', status: 'accepted' })), 'Someone got there first: it’s already confirmed.');
  const m2 = await mount(b.FactsLedger, {
    names, now: NOW, initial: initial({ disputes: [] }), defaultOpen: true,
    api: fakeApi({ reject: async () => Promise.reject(b.errorFrom(409, { error: 'this fact is already disputed', status: 'disputed' })) }),
  });
  await m2.click('[data-fact="fp"] [data-fact-toggle]');
  await m2.click('[data-reject-fact]');
  assert.match(m2.host.querySelector('[role="alert"]').textContent, /It’s disputed now: settle the dispute instead\./);
  m2.done();
});

test('live: fact_changed and fact_dispute reload once per burst', async () => {
  const api = fakeApi();
  const m = await mount(b.FactsLedger, { names, now: NOW, initial: initial(), api });
  const before = api.calls.filter((c) => c[0] === 'list').length;
  b.emitMission({ type: 'fact_changed', data: { id: 'fa', status: 'superseded' } });
  b.emitMission({ type: 'fact_dispute', data: { id: 'd1', status: 'resolved' } });
  b.emitMission({ type: 'fact_changed', data: { id: 'fb', status: 'accepted' } });
  await new Promise((r) => setTimeout(r, 350));
  await settle();
  assert.equal(api.calls.filter((c) => c[0] === 'list').length - before, 1);
  m.done();
});

test('a task whose fact changed can stop relying on it; a disputed one points to Facts', async () => {
  const api = fakeApi();
  const m = await mount(b.FactNeedsYouAction, { taskId: 't1', item: { kind: 'fact_changed', id: 'n1', fact_id: 'fa' }, api });
  await m.click('[data-uncite]');
  assert.deepEqual(api.calls.at(-1), ['uncite', 'fa', 't1']);
  assert.match(m.text(), /Done: this task no longer relies on it\./);
  m.done();
  const d = await mount(b.FactNeedsYouAction, { taskId: 't1', item: { kind: 'rests_on_disputed_fact', id: 'n2', fact_id: 'fa' }, api });
  assert.match(d.text(), /Settle it under Facts on the board\./);
  d.done();
});

test('wiring: the board shows Facts under the week strip, tasks label fact needs-you, a dispute notification opens the board', () => {
  const board = readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8');
  // Measures (Insights) may sit between the week strip and Facts.
  assert.match(board, /<InsightsStrip [^>]*\/>\s*(?:<MeasuresPanel [^>]*\/>\s*)?<FactsLedger /);
  assert.match(board, /'A fact changed'/);
  assert.match(board, /'Rests on a disputed fact'/);
  assert.match(board, /isFactNeedsYou\(n\.kind\) \? \(\s*<FactNeedsYouAction /);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /case 'fact_changed':/);
  assert.match(app, /case 'fact_dispute':/);
  assert.match(app, /n\.kind === 'fact_disputed'\) openBoardForTier\(\)/);
  assert.ok(b.TOAST_KINDS.includes('fact_disputed'));
});
