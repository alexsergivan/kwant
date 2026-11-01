// The router UI on top of Lin's #197/#198/#200 (router-concierge/contract.md): plain reasons for the
// new bases, Laya's "Who should answer this?" card (open, settled, double tap), "Wrong bot?", covers /
// avoids in bot settings, and a "repeated" hire card that quotes the requests it rests on.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { routeReasonText } from './src/routeReason';
  export { routingLine } from './src/spaces';
  export * from './src/routeChoice';
  export { addTags } from './src/tagList';
  export { parseHireMeta, evidenceText } from './src/botActions';
`);
const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { RouteChoiceCard } from './src/components/RouteChoiceCard';
  import { HireCard } from './src/components/HireCard';
  export { createRoot } from 'react-dom/client';
  export { React, flushSync, RouteChoiceCard, HireCard };
`);

const mount = (Comp, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  const render = (p) => ui.flushSync(() => root.render(ui.React.createElement(Comp, p)));
  render(props);
  return { host, render, unmount: () => (root.unmount(), host.remove()) };
};
const tick = () => new Promise((r) => setTimeout(r, 0));
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await tick();
  await tick();
};

test('reasons: the four new bases read as one plain sentence, never a weak-match explanation', () => {
  const server = 'Laya picked Nutritionist (Diet) as the best fit, though only weakly (fit 0.41)';
  assert.equal(m.routeReasonText({ kwantName: 'Concierge', basis: 'concierge', reason: server }), 'No specialist fits, answering generally.');
  assert.equal(m.routeReasonText({ kwantName: 'Nutritionist', basis: 'ask', reason: 'You chose Nutritionist (Diet) for this' }), 'You picked Nutritionist.');
  assert.equal(
    m.routeReasonText({ kwantName: 'Chef', basis: 'handback', reason: 'Atlas handed this back ("not SEO"), so Chef (Cook) takes it' }),
    'Atlas said it’s not their area, so Chef took it.',
  );
  assert.equal(m.routeReasonText({ kwantName: 'Chef', basis: 'learned', reason: server }), 'You sent a message like this to Chef before.');
  for (const basis of ['concierge', 'ask', 'handback', 'learned']) {
    assert.doesNotMatch(m.routeReasonText({ kwantName: 'X', basis, reason: server }), /weak|fit \d|0\.4|best fit/i, basis);
  }
});

test('reasons: the routing line uses the same words, naming who handed it back from the id', () => {
  const names = { k1: 'Atlas' };
  const line = (r) => m.routingLine(r, 'Home', (id) => names[id]);
  assert.equal(line({ kwant_name: 'Concierge', basis: 'concierge', reason: 'x', candidate_count: 4 }), 'No specialist fits, answering generally.');
  assert.equal(line({ kwant_name: 'Chef', basis: 'handback', handed_back_by: 'k1', reason: 'k1 handed this back', candidate_count: 4 }), 'Atlas said it’s not their area, so Chef took it.');
  assert.equal(line({ kwant_name: 'Chef', basis: 'learned', reason: 'x', candidate_count: 4 }), 'You sent a message like this to Chef before.');
  assert.equal(line({ kwant_name: 'Chef', basis: 'ask', reason: 'x', candidate_count: 4 }), 'You picked Chef.');
});

const card = (over = {}) => ({
  kind: 'route_choice',
  route_choice_id: 'rc1',
  channel_id: 'c1',
  user_message_id: 'u1',
  status: 'open',
  reason: 'Close call between Nutritionist and Chef',
  choices: [
    { kwant_id: 'n', name: 'Nutritionist', title: 'Diet and meal plans' },
    { kwant_id: 'c', name: 'Chef', title: 'Recipes' },
  ],
  general: { label: 'Answer generally', name: 'Concierge', kwant_id: 'g' },
  chosen_kwant_id: null,
  chose_general: false,
  ...over,
});

test('ask card: parse, live updates never reopen a settled card', () => {
  const open = m.parseRouteChoiceMeta(JSON.stringify(card()));
  assert.equal(open.status, 'open');
  assert.equal(m.parseRouteChoiceMeta({ kind: 'hire_proposal' }), null);
  assert.equal(m.parseRouteChoiceMeta(card({ general: { name: 'Concierge', kwant_id: null } })).general.label, 'Answer generally');
  const settled = m.parseRouteChoiceMeta(card({ status: 'resolved', chosen_kwant_id: 'c' }));
  assert.equal(m.routeChoiceView(open, settled).status, 'resolved');
  assert.equal(m.routeChoiceView(settled, open).status, 'resolved', 'a late "open" event does not reopen it');
  let map = m.applyRouteChoice({}, card({ status: 'resolved', chose_general: true }));
  map = m.applyRouteChoice(map, card());
  assert.equal(map.rc1.status, 'resolved');
  assert.equal(m.routeChoiceOutcome(map.rc1), 'You asked for a general answer, so Concierge answers.');
  assert.equal(m.generalHint(m.parseRouteChoiceMeta(card({ general: { name: 'Concierge', kwant_id: null } }))), 'Adds Concierge, a generalist, to answer');
});

test('ask card: open shows two bots and "Answer generally"; a tap posts the pick and settles', async () => {
  const picks = [];
  const meta = m.parseRouteChoiceMeta(card());
  const props = { meta, messageId: 'm1', question: 'What should I eat before a run?', onResolve: async (id, pick) => (picks.push([id, pick]), { kind: 'sent' }) };
  const { host, render, unmount } = mount(ui.RouteChoiceCard, props);
  const section = host.querySelector('[data-route-choice]');
  assert.equal(section.dataset.routeChoice, 'open');
  assert.match(host.textContent, /Who should answer this\?/);
  assert.match(host.textContent, /“What should I eat before a run\?”/);
  const buttons = [...host.querySelectorAll('[role=group] button')];
  assert.deepEqual(buttons.map((b) => b.querySelector('.font-medium').textContent), ['Nutritionist', 'Chef', 'Answer generally']);
  assert.ok(buttons.every((b) => !b.disabled && b.getAttribute('aria-pressed') === 'false'));
  assert.match(host.querySelector('[role=status]').textContent, /Nothing is answered until you choose/);

  await click(buttons[1]);
  assert.deepEqual(picks, [['rc1', { kwant_id: 'c' }]]);
  assert.match(host.querySelector('[role=status]').textContent, /Sent\. Chef is answering\./);
  assert.ok([...host.querySelectorAll('[role=group] button')].every((b) => b.disabled), 'no second pick');

  // route_choice_updated confirms it: the card shows the resolved state.
  render({ ...props, live: m.parseRouteChoiceMeta(card({ status: 'resolved', chosen_kwant_id: 'c' })) });
  assert.equal(host.querySelector('[data-route-choice]').dataset.routeChoice, 'resolved');
  assert.match(host.querySelector('[role=status]').textContent, /You picked Chef\./);
  assert.equal(host.querySelectorAll('[aria-pressed=true]').length, 1);
  assert.match(host.querySelector('[aria-pressed=true]').textContent, /Chef/);
  unmount();
});

test('ask card: a card that loads resolved shows the pick with every button off', () => {
  const meta = m.parseRouteChoiceMeta(card({ status: 'resolved', chose_general: true }));
  const { host, unmount } = mount(ui.RouteChoiceCard, { meta, messageId: 'm2', onResolve: async () => assert.fail('no tap') });
  assert.equal(host.querySelector('[data-route-choice]').dataset.routeChoice, 'resolved');
  assert.match(host.textContent, /You asked for a general answer, so Concierge answers\./);
  assert.ok([...host.querySelectorAll('[role=group] button')].every((b) => b.disabled));
  assert.match(host.querySelector('[aria-pressed=true]').textContent, /Answer generally/);
  unmount();
});

test('ask card: 409 "already chosen" is not an error; other failures are shown and can be retried', async () => {
  const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  assert.deepEqual(await m.resolveResult(json(409, { error: 'already chosen' })), { kind: 'already' });
  assert.deepEqual(await m.resolveResult(json(409, { error: 'no one here can answer generally' })), { kind: 'error', message: 'No one here can answer generally.' });
  assert.deepEqual(await m.resolveResult(json(404, { error: 'no such choice' })), { kind: 'error', message: 'This choice is gone. Send your message again.' });
  assert.deepEqual(await m.resolveResult(json(200, { status: 'resolved' })), { kind: 'sent' });

  const meta = m.parseRouteChoiceMeta(card());
  const already = mount(ui.RouteChoiceCard, { meta, messageId: 'm3', onResolve: async () => ({ kind: 'already' }) });
  await click(already.host.querySelector('[role=group] button'));
  assert.match(already.host.querySelector('[role=status]').textContent, /Already answered\./);
  assert.equal(already.host.querySelector('[role=alert]'), null, 'no error for a double tap');
  assert.ok([...already.host.querySelectorAll('[role=group] button')].every((b) => b.disabled));
  already.unmount();

  const failing = mount(ui.RouteChoiceCard, { meta, messageId: 'm4', onResolve: async () => ({ kind: 'error', message: 'Could not reach the server.' }) });
  await click(failing.host.querySelectorAll('[role=group] button')[2]);
  assert.match(failing.host.querySelector('[role=alert]').textContent, /Could not reach the server/);
  assert.ok([...failing.host.querySelectorAll('[role=group] button')].every((b) => !b.disabled), 'still open to try again');
  failing.unmount();
});

test('send: "asking" means a card is waiting, not an error', () => {
  assert.equal(m.isAskingReply({ status: 'asking', route_choice_id: 'rc1' }), true);
  assert.equal(m.isAskingReply({ status: 'started' }), false);
  assert.equal(m.isAskingReply(null), false);
});

test('wrong bot: each reply resends the person\'s latest saved message before it, not thread or unsaved ones', () => {
  const q = m.questionsOfReplies([
    { id: 'u1', sender_role: 'user', text: 'corn dog recipe' },
    { id: 'r1', sender_role: 'kwant' },
    { id: 's1', sender_role: 'system' },
    { id: 'r2', sender_role: 'kwant' },
    { id: 'u2', sender_role: 'user', thread_id: 't1' },
    { id: 'r3', sender_role: 'kwant' },
    { id: 'temp-1', sender_role: 'user' },
    { id: 'r4', sender_role: 'kwant' },
  ]);
  assert.equal(q.get('r1').id, 'u1');
  assert.equal(q.get('r2').id, 'u1');
  assert.equal(q.has('r3'), false);
  assert.equal(q.has('r4'), false);

  const kwants = [
    { id: 'a', name: 'Atlas', title: 'SEO' },
    { id: 'b', name: 'Chef', title: 'Recipes' },
    { id: 'g', name: 'Concierge', title: 'Everyday questions' },
  ];
  assert.deepEqual(m.rerouteTargetsFor({ name: 'general', lead_kwant_id: 'a', member_kwants: [] }, kwants).map((k) => k.name), ['Atlas', 'Chef']);
  assert.deepEqual(m.rerouteTargetsFor({ name: 'seo', lead_kwant_id: 'a', member_kwants: [] }, kwants).map((k) => k.name), ['Atlas']);
  assert.deepEqual(m.rerouteTargetsFor(null, kwants), []);
});

test('covers / avoids: trimmed, case-insensitive duplicates dropped, at most 20 of 60 characters', () => {
  assert.deepEqual(m.addTags(['Taxes'], '  invoices , taxes,  VAT  returns ').list, ['Taxes', 'invoices', 'VAT returns']);
  assert.match(m.addTags([], 'x'.repeat(61)).error, /60 characters/);
  const full = Array.from({ length: 20 }, (_, i) => `t${i}`);
  const r = m.addTags(full, 'one more');
  assert.equal(r.list.length, 20);
  assert.match(r.error, /Up to 20/);
});

test('hire card: "repeated" reads the server\'s {message_id, at, text} and quotes the three requests', () => {
  const meta = m.parseHireMeta({
    kind: 'hire_proposal', hire_id: 'h1', name: 'FoodGuide', role: 'Local food guide', reason: 'Street food keeps coming up.',
    status: 'pending', requested_by_person: false,
    evidence: {
      basis: 'repeated',
      similar_requests: [
        { message_id: 'm1', at: '2026-10-28T09:00:00Z', text: 'Where is the best street food in Munich?' },
        { message_id: 'm2', at: '2026-10-30T09:00:00Z', text: 'Good street food markets in Munich?' },
        { message_id: 'm3', at: '2026-11-01T09:00:00Z', text: 'Best street food stalls in Munich this weekend?' },
      ],
    },
  });
  assert.equal(meta.evidence.similar_requests.length, 3);
  assert.equal(m.evidenceText(meta.evidence, false), 'You asked for this kind of work 3 times in this space.');
  assert.equal(m.parseHireMeta({ ...meta, kind: 'hire_proposal', evidence: { basis: 'repeated', similar_requests: ['a', 'b'] } }).evidence.similar_requests.length, 2, 'older cards with bare ids');

  const { host, unmount } = mount(ui.HireCard, { meta, messageId: 'hm', kwantName: 'Concierge', onDecide: async () => {} });
  const list = host.querySelector('[aria-label="Your earlier requests"]');
  assert.ok(list, 'the requests are listed');
  const items = [...list.querySelectorAll('li')].map((li) => li.textContent);
  assert.equal(items.length, 3);
  assert.match(items[0], /Where is the best street food in Munich\?/);
  assert.match(items[2], /Nov 1|1 Nov/);
  unmount();
});
