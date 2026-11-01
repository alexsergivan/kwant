// Flight School, Phase C4 (docs/mission-control.md §14): a bot that earned the next tier asks for it,
// and only the person grants or declines. Requests show on the board's Needs-you list and in the bot's
// settings with its scorecard; a decision another tab made first (409) says so; revoking stays one tap;
// changes arrive live. With Flight School off nothing new shows.
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
  import { TierRequests, ScorecardView, ScorecardDisclosure } from './src/components/TierProposals';
  import { FlightSchoolPanel } from './src/components/FlightSchoolPanel';
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { TierRequests, ScorecardDisclosure } from './src/components/TierProposals';
  export { FlightSchoolPanel } from './src/components/FlightSchoolPanel';
  export * from './src/flightSchool';
  export { MissionError, errorFrom, emitMission } from './src/missions';
  export { notifyKindsFor, notifyOnToSave, TOAST_KINDS } from './src/continuity';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const requestsHtml = (p) => r(TierRequests, p);
  export const scorecardHtml = (p) => r(ScorecardView, p);
  export const panelHtml = (p) => r(FlightSchoolPanel, p);
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const NOW = Date.parse('2026-11-01T12:00:00Z');
const names = { k_sandy: 'Sandy', k_cody: 'Cody' };

const card = (over = {}) => ({
  kwant_id: 'k_sandy', task_class: 'code', window_days: 30, since: '2026-10-02T12:00:00Z', tier: 'observe', tier_from: 'class', next_tier: 'sandbox',
  eligible: true, missing: [], tasks: { settled: 6, verified: 5, failed: 1, cancelled: 0 }, first_try: { k: 4, n: 5, rate: 0.8 },
  verified_rate: { k: 5, n: 6, rate: 5 / 6 }, false_success: { k: 0, n: 5, rate: 0 }, cards: { shown: 9, approved: 9, denied: 0, approved_rate: { k: 9, n: 9, rate: 1 } },
  recent_denials: { n: 5, of: 5, with_denial: 0 }, ...over,
});
const proposal = (over = {}) => ({
  kind: 'tier_proposal', id: 'p1', space_id: 's1', kwant_id: 'k_sandy', task_class: 'code', from_tier: 'observe', to_tier: 'sandbox',
  reason: 'Five verified code tasks this month, none denied.', status: 'proposed', note: null, decided_at: null, created_at: '2026-11-01T11:00:00Z',
  scorecard: card(), ...over,
});
const caps = (over = {}) => ({
  flight_school: true, kwant_id: 'k_sandy', default_tier: 'observe', policies: [],
  effective: [{ task_class: 'general', tier: 'observe', from: 'any_class' }, { task_class: 'code', tier: 'observe', from: 'class' }],
  events: [], proposals: [proposal()], ...over,
});

function fakeApi(over = {}) {
  const calls = [];
  const api = {
    calls,
    get: async (id) => (calls.push(['get', id]), caps()),
    set: async () => ({ unchanged: false, capabilities: caps() }),
    revoke: async () => ({ unchanged: false, capabilities: caps() }),
    scorecard: async (id, cls) => (calls.push(['scorecard', id, cls]), card({ task_class: cls })),
    proposals: async (status) => (calls.push(['proposals', status ?? null]), []),
    grant: async (id, key) => (calls.push(['grant', id, key]), { proposal: proposal({ status: 'granted' }), capabilities: caps({ proposals: [] }) }),
    decline: async (id, key, note) => (calls.push(['decline', id, key, note ?? null]), { proposal: proposal({ status: 'declined', note }), capabilities: caps({ proposals: [] }) }),
    ...over,
  };
  return api;
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

test('with nobody asking (always the case with Flight School off) the board shows nothing new', () => {
  assert.equal(b.requestsHtml({ names, initial: [], api: fakeApi() }), '');
  // Decided ones alone don't make a list either.
  assert.equal(b.requestsHtml({ names, initial: [proposal({ status: 'granted' })], api: fakeApi() }), '');
  // The bot's settings stay as before with Flight School off, even if old proposals exist.
  assert.equal(b.panelHtml({ kwantId: 'k_sandy', kwantName: 'Sandy', initial: caps({ flight_school: false }), api: fakeApi() }), '');
});

test('a request reads as who asks for what, why, its scorecard, and Grant / Decline', () => {
  const html = b.requestsHtml({ names, now: NOW, initial: [proposal()], api: fakeApi() });
  const t = text(html);
  assert.match(t, /Needs you: a bot asks for more freedom/);
  assert.match(t, /Sandy asks for Sandbox on Code/);
  assert.match(t, /Observe → Sandbox\./);
  assert.match(t, /asked 1 ?h ago/);
  assert.match(t, /Sandy says: Five verified code tasks this month, none denied\./);
  assert.match(t, /Its scorecard when it asked/);
  assert.match(t, /Verified 5 of 6 settled/);
  assert.match(t, /Grant Sandbox/);
  assert.match(t, /Decline…/);
  assert.match(t, /Payments always ask you/);
  // Named for screen readers: the card by its title, the buttons as one decision.
  assert.match(html, /<li aria-labelledby="[^"]+"[^>]*data-proposal="p1" data-status="proposed"/);
  assert.match(html, /role="group" aria-label="Decide: Sandy asks for Sandbox on Code"/);
  // Several bots asking say so; an unknown bot still reads.
  const two = text(b.requestsHtml({ names: {}, now: NOW, initial: [proposal(), proposal({ id: 'p2', kwant_id: 'k_x', created_at: '2026-11-01T10:00:00Z' })], api: fakeApi() }));
  assert.match(two, /2 bots ask for more freedom/);
  assert.match(two, /A bot asks for Sandbox on Code/);
});

test('scorecards: plain rows, what holds it back in rose, and what it still needs', () => {
  const ready = b.scorecardHtml({ card: card() });
  assert.match(text(ready), /Verified on the first try 4 of 5 \(80%\)/);
  assert.match(text(ready), /Said done, check failed 0 of 5/);
  assert.match(text(ready), /Denied in its last 5 0 of 5/);
  assert.match(text(ready), /Approval cards 9 approved, 0 denied/);
  assert.match(ready, /data-scorecard-status="ready"[^>]*>Qualifies to ask for Sandbox\. Only you can grant it\./);
  assert.match(text(ready), /From the board’s own records of the last 30 days/);

  const notYet = b.scorecardHtml({ card: card({ eligible: false, false_success: { k: 1, n: 5, rate: 0.2 }, missing: ['2 more verified task(s) (has 3, needs 5).', 'no false successes'] }) });
  assert.match(notYet, /text-rose-300[^>]*>1 of 5/);
  assert.match(text(notYet), /Not yet ready for Sandbox: needs 2 more verified task\(s\) \(has 3, needs 5\); no false successes\./);

  assert.match(text(b.scorecardHtml({ card: card({ tier: 'act', next_tier: null, recent_denials: null }) })), /Act is the top tier: nothing more to earn here\./);
  assert.match(text(b.scorecardHtml({ card: { task_class: 'docs' } })), /No settled work in this kind yet\./);
});

test('a 409 names the state the server found, and stale says why', () => {
  const e = b.errorFrom(409, { error: 'this proposal is already declined', status: 'declined' });
  assert.equal(e.status, 409);
  assert.equal(e.code, 'declined');
  assert.equal(b.proposalConflictText(e.code, e.message), 'Someone got there first: it’s already declined.');
  assert.equal(b.proposalConflictText('granted', 'x'), 'Someone got there first: it’s already granted.');
  const stale = b.errorFrom(409, { error: 'this proposal is stale: the bot is already at Sandbox', status: 'stale' });
  assert.equal(b.proposalConflictText(stale.code, stale.message), 'This request no longer holds: the bot is already at Sandbox. The bot has to ask again.');
  // A 409 with neither code nor status keeps the server's words.
  const off = b.errorFrom(409, { error: 'Flight School is off; switch it on to grant tiers' });
  assert.equal(off.code, null);
  assert.equal(b.proposalConflictText(off.code, off.message), off.message);
});

test('Grant is one tap and says how to undo it; a retry reuses the same key', async () => {
  let fail = true;
  const api = fakeApi();
  const grant = api.grant;
  api.grant = async (id, key) => {
    if (fail) {
      fail = false;
      api.calls.push(['grant-failed', id, key]);
      throw new Error('Couldn’t reach kwnt.');
    }
    return grant(id, key);
  };
  const v = await mount(b.TierRequests, { names, now: NOW, initial: [proposal()], api });
  await v.click('[data-grant]');
  assert.match(v.host.querySelector('[data-proposal-error]').textContent, /Couldn’t reach/);
  await v.click('[data-grant]');
  const [first, second] = api.calls.filter((c) => c[0].startsWith('grant'));
  assert.equal(first[2], second[2], 'the same Idempotency-Key, so a lost answer replays');
  assert.equal(v.host.querySelector('[data-proposal-error]'), null);
  assert.match(v.text(), /Granted: Sandy is now Sandbox on Code\. You can revoke it in one tap from Sandy’s settings\./);
  assert.equal(v.host.querySelector('[data-grant]'), null);
  assert.match(v.text(), /Nothing else is waiting\./);
  v.done();
});

test('Decline asks for an optional note, Escape backs out, and the note is sent', async () => {
  const api = fakeApi();
  const v = await mount(b.TierRequests, { names, now: NOW, initial: [proposal()], api });
  await v.click('[data-decline]');
  const input = v.host.querySelector('[data-decline-form] input');
  assert.equal(document.activeElement, input, 'the note field takes focus');
  assert.equal(v.host.querySelector('[data-decline-form] label').textContent.trim(), 'Why not? Optional, Sandy sees it');
  input.dispatchEvent(new dom.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(v.host.querySelector('[data-decline-form]'), null);
  assert.ok(v.host.querySelector('[data-grant]'));

  await v.click('[data-decline]');
  const field = v.host.querySelector('[data-decline-form] input');
  const setter = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set;
  setter.call(field, '  Not on the release branch yet ');
  field.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
  await settle();
  await v.click('[data-confirm-decline]');
  const call = api.calls.find((c) => c[0] === 'decline');
  assert.equal(call[3], 'Not on the release branch yet');
  assert.match(v.text(), /Declined\. Sandy can ask again in 7 days\./);
  v.done();
});

test('someone got there first: the card says so and shows what holds now', async () => {
  const api = fakeApi({
    grant: async () => {
      throw new b.MissionError('This proposal is already declined.', 409, 'declined');
    },
    proposals: async (status) => (status ? [] : [proposal({ status: 'declined', note: 'Not yet' })]),
  });
  const v = await mount(b.TierRequests, { names, now: NOW, initial: [proposal()], api });
  await v.click('[data-grant]');
  const err = v.host.querySelector('[data-proposal-error]');
  assert.equal(err.getAttribute('role'), 'alert');
  assert.match(err.textContent, /Someone got there first: it’s already declined\./);
  assert.equal(v.host.querySelector('[data-proposal]').dataset.status, 'declined');
  assert.match(v.text(), /Declined\. Sandy can ask again in 7 days\./);
  assert.equal(v.host.querySelector('[data-grant]'), null);
  v.done();
});

test('requests arrive and leave live', async () => {
  const api = fakeApi();
  const v = await mount(b.TierRequests, { names, now: NOW, initial: [], api });
  assert.equal(v.host.innerHTML, '');
  b.emitMission({ type: 'capability_changed', data: { capability: { kwant_id: 'k_cody', proposal: proposal({ id: 'p9', kwant_id: 'k_cody', task_class: 'docs' }) } } });
  await settle();
  assert.match(v.text(), /Cody asks for Sandbox on Docs/);
  // Decided elsewhere: it leaves the list.
  b.emitMission({ type: 'capability_changed', data: { capability: { kwant_id: 'k_cody', proposal: proposal({ id: 'p9', kwant_id: 'k_cody', status: 'granted' }) } } });
  await settle();
  assert.equal(v.host.innerHTML, '');
  // A change without a proposal (a revoke) reads the list again.
  b.emitMission({ type: 'capability_changed', data: { capability: { kwant_id: 'k_cody' } } });
  await settle();
  assert.deepEqual(api.calls.at(-1), ['proposals', 'proposed']);
  v.done();
});

test('the bot’s settings: its request, a scorecard per kind of work, one-tap Revoke, and live reloads', async () => {
  const api = fakeApi();
  const v = await mount(b.FlightSchoolPanel, { kwantId: 'k_sandy', kwantName: 'Sandy', now: NOW, initial: caps({ default_tier: 'sandbox' }), api });
  assert.match(v.text(), /Sandy asks for Sandbox on Code/);
  assert.ok(v.host.querySelector('[data-revoke]'), 'Revoke is still one tap');
  // No scorecard for "all work": scorecards are per kind of work.
  const toggles = [...v.host.querySelectorAll('[data-scorecard-toggle]')].map((x) => x.dataset.scorecardToggle);
  assert.deepEqual(toggles, ['general', 'code']);
  const code = v.host.querySelector('[data-scorecard-toggle="code"]');
  assert.equal(code.getAttribute('aria-expanded'), 'false');
  assert.match(code.textContent, /Scorecard for Code/);
  await v.click('[data-scorecard-toggle="code"]');
  assert.equal(code.getAttribute('aria-expanded'), 'true');
  assert.deepEqual(api.calls.filter((c) => c[0] === 'scorecard'), [['scorecard', 'k_sandy', 'code']]);
  assert.ok(v.host.querySelector('[data-scorecard="code"]'));

  // Granted elsewhere: tiers read again and the open scorecard too. Another bot's change is ignored.
  b.emitMission({ type: 'capability_changed', data: { capability: { kwant_id: 'k_cody' } } });
  await settle();
  assert.equal(api.calls.filter((c) => c[0] === 'get').length, 0);
  b.emitMission({ type: 'capability_changed', data: { capability: { kwant_id: 'k_sandy' } } });
  await settle();
  assert.equal(api.calls.filter((c) => c[0] === 'get').length, 1);
  assert.equal(api.calls.filter((c) => c[0] === 'scorecard').length, 2);

  await v.click('[data-grant]');
  assert.match(v.text(), /Granted: Sandy is now Sandbox on Code/);
  v.done();
});

test('a scorecard that fails to load says so and can be tried again', async () => {
  let n = 0;
  const api = { scorecard: async () => (++n === 1 ? Promise.reject(new Error('Couldn’t reach kwnt.')) : card()) };
  const v = await mount(b.ScorecardDisclosure, { kwantId: 'k_sandy', taskClass: 'code', api });
  await v.click('[data-scorecard-toggle]');
  assert.match(v.host.querySelector('[role="alert"]').textContent, /Couldn’t reach.*Try again/);
  await v.click('[role="alert"] button');
  assert.ok(v.host.querySelector('[data-scorecard="code"]'));
  v.done();
});

test('tier requests can notify, only with Flight School on, and open the board', () => {
  const ids = (on) => b.notifyKindsFor(false, false, false, on).map((k) => k.id);
  assert.ok(!ids(false).includes('tier_proposed'));
  assert.ok(ids(true).includes('tier_proposed'));
  assert.equal(b.notifyKindsFor(false, false, false, true).find((k) => k.id === 'tier_proposed').label, 'A bot asks for a higher tier');
  // Kept while Flight School is merely off; dropped for a server without it.
  assert.deepEqual(b.notifyOnToSave(['turn_finished', 'tier_proposed'], false, false, false, true), ['turn_finished', 'tier_proposed']);
  assert.deepEqual(b.notifyOnToSave(['turn_finished', 'tier_proposed'], false, false, false, false), ['turn_finished']);
  assert.ok(b.TOAST_KINDS.includes('tier_proposed'));

  const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
  const app = src('App.tsx');
  assert.match(app, /n\.kind === 'tier_proposed'\) openBoardForTier\(\)/);
  assert.match(app, /case 'capability_changed':/);
  assert.match(src('components/MissionBoardView.tsx'), /<TierRequests names=\{names\} now=\{now\} \/>/);
  assert.match(src('components/SettingsModal.tsx'), /notifyOnToSave\(notifyOn, appUpdates, !!proactive, !!discovery, flightSchool !== null\)/);
});
