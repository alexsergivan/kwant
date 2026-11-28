// inbox-question-job-and-time (m8, autonomy/inbox-question-job-and-time/brief.md): a NEEDS YOU question asked inside a
// hand-off shows a muted second line "for <job title> · <lead>"; one asked in a direct chat (no job) shows none.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
g.fetch = async () => ({ ok: false, status: 404, json: async () => ({}) });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as fx from './src/doneAwayFixture';
  export * as store from './src/inboxStore';
  export { InboxNew } from './src/components/InboxNew';
  export { questionJob } from './src/away';
`);
const { React, createRoot, fx, store: s } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

const NOW = fx.AWAY_INBOX.now;
const question = (id, extra = {}) => ({
  kind: 'question',
  id,
  kwant_id: 'k-res',
  kwant_name: 'Researcher',
  title: 'Which caterer budget?',
  summary: 'Choices: Up to 2000 · Up to 4000',
  status: 'open',
  timestamp: NOW,
  link: { thread_id: 'th-1', channel_id: null, direct_kwant_id: null },
  choices: ['Up to 2000', 'Up to 4000'],
  group: 'waiting_on_you',
  sort_key: [1, 0, 0, 'Researcher', id],
  unread: true,
  ...extra,
});

async function mount(items) {
  document.body.innerHTML = '';
  sessionStorage.clear();
  window.history.replaceState(null, '', '/inbox');
  const data = { ...JSON.parse(JSON.stringify(fx.AWAY_INBOX)), items, counts: { waiting_on_you: items.length, failed_or_stuck: 0, to_review: 0, finished: 0, badge: items.length } };
  s.inboxStore.reset();
  s.inboxSource.get = async () => data;
  s.inboxStore.patch({ data });
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: true, now: Date.parse(NOW), placeName: fx.awayPlaceName }));
  await settle();
  return { el, root, q: (x) => el.querySelector(x) };
}

test('questionJob: "for <job> · <lead>" only on a question asked in a hand-off', () => {
  const { questionJob } = m;
  assert.equal(questionJob({ kind: 'question', job_title: 'Compare three caterers', job_lead: 'Planner' }), 'for Compare three caterers · Planner');
  assert.equal(questionJob({ kind: 'question', job_title: ' Compare three caterers ', job_lead: null }), 'for Compare three caterers', 'lead gone: the job alone');
  assert.equal(questionJob({ kind: 'question', job_title: null, job_lead: 'Planner' }), '', 'no job, no line');
  assert.equal(questionJob({ kind: 'question' }), '', 'an older server sends neither');
  assert.equal(questionJob({ kind: 'approval', job_title: 'Compare three caterers', job_lead: 'Planner' }), '', 'questions only');
});

test('the question row shows the job line under the question; a direct-chat question shows none', async () => {
  const { q, root } = await mount([
    question('q-job', { job_title: 'Compare three caterers', job_lead: 'Planner' }),
    question('q-direct', { title: 'Which city?', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k-res' }, job_title: null, job_lead: null }),
  ]);
  const row = q('[data-row="q-job"]');
  assert.ok(row, 'the question row is listed');
  const line = row.querySelector('[data-question-job]');
  assert.ok(line, 'the job line is drawn');
  assert.equal(line.textContent, 'for Compare three caterers · Planner');
  assert.match(line.className, /\btruncate\b/, 'one line, clipped with an ellipsis');
  assert.match(line.className, /\btext-zinc-400\b/, 'muted');
  assert.equal(line.getAttribute('title'), 'for Compare three caterers · Planner', 'the whole line on hover when clipped');
  // It sits right under the question, inside the row's option (read with it by a screen reader)
  const option = row.querySelector('[role="option"]');
  assert.ok(option.contains(line));
  assert.ok(option.textContent.includes('Which caterer budget?for Compare three caterers · Planner'), option.textContent);
  // The answer buttons are still there
  assert.deepEqual([...row.querySelectorAll('[role="group"] button')].map((b) => b.textContent), ['Up to 2000', 'Up to 4000']);
  assert.equal(q('[data-row="q-direct"] [data-question-job]'), null, 'no job, no line');
  root.unmount();
});
