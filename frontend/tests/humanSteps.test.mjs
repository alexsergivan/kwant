import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const h = await loadBundled(`export * from './src/humanSteps';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { HumanStepCard } from './src/components/HumanStepCard';
  export const render = (props) => renderToStaticMarkup(React.createElement(HumanStepCard, props));
`);

const meta = (over = {}) => ({
  kind: 'human_step', human_step_id: 'hs1', step_kind: 'sign_in', reason: 'Log in to GitHub so I can open the PR.',
  url: 'https://www.github.com/login?return_to=/secret/path', sandbox_target: 'vm', source: 'bot', status: 'open', ...over,
});
const rec = (over = {}) => ({
  id: 'hs1', kwant_id: 'pixel', channel_id: null, direct_kwant_id: 'pixel', thread_id: null, turn_id: 't1', kind: 'sign_in',
  reason: 'r', url: null, sandbox_target: null, source: 'bot', status: 'open', message_id: 'm1', created_at: '2026-10-22T10:00:00Z', closed_at: null, ...over,
});
const props = (over = {}) => ({ meta: h.parseHumanStepMeta(meta()), messageId: 'm1', kwantName: 'Pixel', waiting: true, onOpenBrowser: () => {}, onDone: async () => {}, onSkip: async () => {}, ...over });

test('meta parses, defaults odd values, ignores other kinds', () => {
  assert.equal(h.parseHumanStepMeta(JSON.stringify(meta())).step_kind, 'sign_in');
  assert.equal(h.parseHumanStepMeta(meta({ step_kind: 'weird', status: 'x' })).step_kind, 'other');
  assert.equal(h.parseHumanStepMeta(meta({ status: 'x' })).status, 'open');
  assert.equal(h.parseHumanStepMeta({ kind: 'ask_user' }), null);
  assert.equal(h.parseHumanStepMeta('nope'), null);
});

test('only the domain of the address is shown', () => {
  assert.equal(h.domainOf('https://www.github.com/login?x=1'), 'github.com');
  assert.equal(h.domainOf('not a url'), null);
  assert.equal(h.domainOf(null), null);
});

test('records: newest state, closed never reopens; badges and waits', () => {
  let map = h.applyHumanStep({}, rec());
  map = h.applyHumanStep(map, rec({ id: 'hs2', kwant_id: 'pixel', turn_id: 't2', created_at: '2026-10-22T11:00:00Z' }));
  assert.deepEqual(h.openStepsByKwant(map), { pixel: 2 });
  assert.equal(h.latestOpenStep(map, 'pixel').id, 'hs2');
  assert.deepEqual([...h.turnsAwaitingStep(map)].sort(), ['t1', 't2']);
  map = h.applyHumanStep(map, rec({ status: 'done' }));
  assert.equal(h.applyHumanStep(map, rec()).hs1.status, 'done');
  assert.deepEqual(h.openStepsByKwant(map), { pixel: 1 });
  assert.equal(h.latestOpenStep(map, 'other'), null);
  assert.equal(h.humanStepView(h.parseHumanStepMeta(meta()), rec({ status: 'declined' })).status, 'declined');
});

test('the Done label fits the kind', () => {
  assert.equal(h.KIND_TEXT.sign_in.done, 'Done, I’m signed in');
  assert.equal(h.KIND_TEXT.captcha.done, 'Done, I solved it');
  for (const [kind, t] of Object.entries(h.KIND_TEXT)) {
    const html = ui.render(props({ meta: h.parseHumanStepMeta(meta({ step_kind: kind })) }));
    assert.ok(html.includes(t.heading), kind);
    assert.ok(html.includes(t.done.replace('’', '&#x27;')) || html.includes(t.done), kind);
  }
});

test('open card: reason, domain only, three buttons, waiting line', () => {
  const html = ui.render(props());
  assert.match(html, /Pixel<\/span> needs you in the browser/);
  assert.match(html, /Log in to GitHub/);
  assert.match(html, />github\.com</);
  assert.doesNotMatch(html, /secret\/path|return_to/);
  assert.match(html, /Open browser/);
  assert.match(html, />Skip</);
  assert.match(html, /is paused and waiting for you/);
  assert.equal((html.match(/<button/g) || []).length, 3);
});

test('closed and stale cards', () => {
  assert.doesNotMatch(ui.render(props({ live: rec({ status: 'done' }) })), /<button/);
  assert.match(ui.render(props({ live: rec({ status: 'done' }) })), /Done\. Pixel is carrying on/);
  assert.match(ui.render(props({ live: rec({ status: 'declined' }) })), /Skipped/);
  assert.match(ui.render(props({ live: rec({ status: 'cancelled' }) })), /reply was stopped/);
  assert.match(ui.render(props({ waiting: false })), /stopped waiting/);
  assert.match(ui.render(props({ meta: h.parseHumanStepMeta(meta({ source: 'auto' })) })), /spotted automatically/);
});

test('captcha target: finite box only, kind defaults to puzzle', () => {
  assert.deepEqual(h.parseCaptchaTarget({ x: 460, y: 180, width: 360, height: 380 }), { kind: 'puzzle', x: 460, y: 180, width: 360, height: 380 });
  assert.equal(h.parseCaptchaTarget({ x: 1, y: 1, width: 0, height: 5 }), null);
  assert.equal(h.parseCaptchaTarget({ x: 'a', y: 1, width: 5, height: 5 }), null);
  assert.equal(h.parseCaptchaTarget(null), null);
  const m = h.parseHumanStepMeta(meta({ step_kind: 'captcha', target: { kind: 'slider', x: 1, y: 2, width: 3, height: 4 } }));
  assert.equal(m.target.kind, 'slider');
  assert.equal(h.parseHumanStepMeta(meta({ status: 'resolved_auto' })).status, 'resolved_auto');
  assert.equal(h.parseHumanStepMeta(meta({ status: 'timed_out' })).status, 'timed_out');
});

test('captcha step for a sandbox: open first, a just-closed one lingers a minute', () => {
  const now = Date.parse('2026-10-22T12:00:00Z');
  const box = { kind: 'puzzle', x: 1, y: 2, width: 30, height: 40 };
  let map = h.applyHumanStep({}, rec({ id: 'c1', kind: 'captcha', sandbox_target: 'pixel', target: box }));
  map = h.applyHumanStep(map, rec({ id: 's1', kind: 'sign_in', sandbox_target: 'pixel', created_at: '2026-10-22T11:59:00Z' }));
  assert.equal(h.captchaStepFor(map, 'pixel', now).id, 'c1');
  assert.equal(h.captchaStepFor(map, 'other', now), null);
  // The box moves while the step stays open
  map = h.applyHumanStep(map, rec({ id: 'c1', kind: 'captcha', sandbox_target: 'pixel', target: { ...box, x: 9 } }));
  assert.equal(map.c1.target.x, 9);
  map = h.applyHumanStep(map, rec({ id: 'c1', kind: 'captcha', sandbox_target: 'pixel', status: 'resolved_auto', closed_at: '2026-10-22T11:59:40Z' }));
  assert.equal(h.captchaStepFor(map, 'pixel', now).status, 'resolved_auto');
  assert.equal(h.captchaStepFor(map, 'pixel', now + 60_000), null);
});

test('captcha card: Solve it is the main button and opens solve mode; auto outcomes', () => {
  const calls = [];
  const cm = h.parseHumanStepMeta(meta({ step_kind: 'captcha' }));
  const html = ui.render(props({ meta: cm, onOpenBrowser: (...a) => calls.push(a) }));
  assert.match(html, /Solve it/);
  assert.doesNotMatch(html, /Open browser/);
  assert.match(html, /continues by itself/);
  assert.equal((html.match(/<button/g) || []).length, 3);
  assert.match(ui.render(props({ meta: cm, live: rec({ kind: 'captcha', status: 'resolved_auto' }) })), /Solved\. Pixel continues\./);
  assert.match(ui.render(props({ meta: cm, live: rec({ kind: 'captcha', status: 'timed_out' }) })), /moved on without it/);
  assert.doesNotMatch(ui.render(props({ meta: cm, live: rec({ kind: 'captcha', status: 'timed_out' }) })), /<button/);
});
