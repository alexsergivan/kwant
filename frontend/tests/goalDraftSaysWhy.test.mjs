// 34f goal-draft-says-why (Mira's 0.9.40 UX pass, minor d "Silent failed draft"): when the bot's draft turn saves
// no draft, the server sends `goal_draft_failed` with a short reason. The strip stops saying "{bot} is drafting
// checks…", says "{bot} couldn’t draft it: {reason}. Write the checks yourself.", and "Write the checks yourself"
// opens the form with the person's words still in it. A failure for another bot, or another space, changes nothing.
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
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { GoalsStrip } from './src/components/GoalsStrip';
  export { parseGoal, parseGoalDraftFailed } from './src/goals';
  export { emitMission } from './src/missions';
`);

const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
localStorage.setItem('kwnt.goals.open', '1');
const names = { k_patch: 'Patch', k_cody: 'Cody' };
const bots = [{ id: 'k_patch', name: 'Patch' }, { id: 'k_cody', name: 'Cody' }];
const NOW = Date.parse('2026-11-22T19:00:00Z');
const met = b.parseGoal({
  id: 'g1', space_id: 's1', title: 'The site stays up', description: '', kwant_id: 'k_cody', status: 'met', mode: 'suggest',
  interval_minutes: 60, created_by: 'person', last_checked_at: new Date(NOW).toISOString(), next_check_at: new Date(NOW + 3_600_000).toISOString(),
  created_at: new Date(NOW - 3_600_000).toISOString(), updated_at: new Date(NOW).toISOString(),
  checks: [{ id: 'c1', label: 'Site answers', kind: 'http', spec: { url: 'https://kwant.si' }, required: true, freshness_minutes: null, latest: null }],
  open_mission: null,
});
const REASON = 'check 1: spec.url must start with http:// or https://';

const mounted = new Set();
test.afterEach(() => {
  for (const done of mounted) done();
  mounted.clear();
});

async function mountDrafting(goals, { beforeReply } = {}) {
  const calls = [];
  const api = {
    list: async () => goals,
    get: async () => null,
    draft: async (body) => {
      calls.push(body);
      await beforeReply?.();
      return null;
    },
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = b.createRoot(host);
  mounted.add(() => {
    root.unmount();
    host.remove();
  });
  root.render(b.React.createElement(b.GoalsStrip, { names, bots, spaceId: 's1', now: NOW, api, initial: { goals }, openPanel: 'new' }));
  await settle();
  const q = (sel) => document.querySelector(sel);
  const type = async (sel, value) => {
    const el = q(sel);
    assert.ok(el, `no ${sel}`);
    const proto = el.tagName === 'TEXTAREA' ? dom.window.HTMLTextAreaElement.prototype : dom.window.HTMLSelectElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
    el.dispatchEvent(new dom.window.Event(el.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
    await settle();
  };
  await type('[data-goal-describe] [data-field="text"]', 'kwant.si answers with HTTP 200.');
  await type('[data-goal-describe] [data-field="bot"]', 'k_patch');
  q('[data-goal-describe]').dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.equal(calls.length, 1, 'asked for one draft');
  return {
    q,
    line: () => q('[data-goals-drafting]')?.textContent.replace(/\s+/g, ' ').trim() ?? null,
    failed: () => q('[data-goals-draft-failed]')?.textContent.replace(/\s+/g, ' ').trim() ?? null,
    click: async (sel) => {
      const el = q(sel);
      assert.ok(el, `no ${sel}`);
      el.dispatchEvent(new dom.window.MouseEvent('click', { bubbles: true }));
      await settle();
    },
  };
}

for (const [label, goals] of [['an empty strip', []], ['a strip with goals', [met]]]) {
  test(`${label}: the failed draft for the waiting bot replaces the drafting line with the reason`, async () => {
    const s = await mountDrafting(goals);
    assert.equal(s.line(), 'Patch is drafting checks for “kwant.si answers with HTTP 200”. The draft shows here.');
    assert.equal(s.failed(), null);

    // Another bot's failure, or another space's: nothing changes
    b.emitMission({ type: 'goal_draft_failed', data: { space_id: 's1', kwant_id: 'k_cody', reason: 'not this one' } });
    b.emitMission({ type: 'goal_draft_failed', data: { space_id: 's2', kwant_id: 'k_patch', reason: 'not this space' } });
    await settle();
    assert.match(s.line() ?? '', /^Patch is drafting checks/);
    assert.equal(s.failed(), null);

    b.emitMission({ type: 'goal_draft_failed', data: { space_id: 's1', kwant_id: 'k_patch', reason: REASON } });
    await settle();
    assert.equal(s.line(), null, 'no spinner any more');
    assert.equal(s.failed(), `Patch couldn’t draft it: ${REASON}. Write the checks yourself.`);
    assert.equal(s.q('[data-goals-draft-failed]').getAttribute('role'), 'status');
    // A link inside the sentence: it opts out of the phone 44 px button rule so the lines don't open up
    assert.ok(s.q('[data-goals-draft-write]').hasAttribute('data-compact'), 'the inline link is data-compact');

    // Write the checks yourself: the form, with the person's words in it
    await s.click('[data-goals-draft-write]');
    assert.ok(s.q('[data-goal-form]'), 'the form opens');
    assert.equal(s.q('[data-goal-form] [data-field="title"]').value, 'kwant.si answers with HTTP 200.');
    assert.equal(s.failed(), null, 'the line goes once the form is open');
  });
}

test('the failed line can be dismissed, and a reason that already ends in a full stop gets no second one', async () => {
  const s = await mountDrafting([]);
  b.emitMission({ type: 'goal_draft_failed', data: { space_id: 's1', kwant_id: 'k_patch', reason: 'it didn’t propose any checks.' } });
  await settle();
  assert.equal(s.failed(), 'Patch couldn’t draft it: it didn’t propose any checks. Write the checks yourself.');
  s.q('[data-goals-draft-dismiss]').focus();
  await s.click('[data-goals-draft-dismiss]');
  assert.equal(s.failed(), null);
  assert.equal(s.line(), null);
  assert.ok(document.activeElement?.hasAttribute('data-new-goal'), 'keyboard focus goes to New goal, not the page');
});

test('a turn that fails before the draft request is answered still says why', async () => {
  const s = await mountDrafting([], {
    beforeReply: async () => {
      b.emitMission({ type: 'goal_draft_failed', data: { space_id: 's1', kwant_id: 'k_cody', reason: 'not this bot' } });
      b.emitMission({ type: 'goal_draft_failed', data: { space_id: 's1', kwant_id: 'k_patch', reason: REASON } });
      await settle();
    },
  });
  await settle(); // the reply comes after the event's own settle
  assert.equal(s.line(), null, 'no spinner for a draft that already failed');
  assert.equal(s.failed(), `Patch couldn’t draft it: ${REASON}. Write the checks yourself.`);
  await s.click('[data-goals-draft-write]');
  assert.equal(s.q('[data-goal-form] [data-field="title"]').value, 'kwant.si answers with HTTP 200.');
});

test('the wire event is read defensively', () => {
  assert.deepEqual(b.parseGoalDraftFailed({ space_id: 's1', kwant_id: 'k', reason: ' why ' }), { space_id: 's1', kwant_id: 'k', reason: 'why' });
  assert.equal(b.parseGoalDraftFailed({ space_id: 's1', reason: 'x' }), null, 'no bot: nothing to match');
  assert.equal(b.parseGoalDraftFailed(null), null);
  assert.deepEqual(b.parseGoalDraftFailed({ kwant_id: 'k', reason: 42 }), { space_id: null, kwant_id: 'k', reason: 'it didn’t propose any checks' }, 'a missing reason reads as the plain one');
  assert.equal(b.parseGoalDraftFailed({ kwant_id: 'k', reason: 'x'.repeat(500) }).reason.length, 200, 'kept short');
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /case 'goal_draft_failed':[\s\S]{0,300}parseGoalDraftFailed\(event\.data\)[\s\S]{0,200}emitMission\(\{ type: 'goal_draft_failed'/, 'App passes the event on');
});
