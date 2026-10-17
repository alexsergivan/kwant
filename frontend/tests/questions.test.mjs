import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const q = await loadBundled(`export * from './src/questions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { QuestionCard } from './src/components/QuestionCard';
  export const render = (props) => renderToStaticMarkup(React.createElement(QuestionCard, props));
`);

const metaJson = (over = {}) =>
  JSON.stringify({ kind: 'ask_user', question_id: 'q1', question: 'Which region?', choices: ['EU', 'US'], status: 'open', answer: null, ...over });
const rec = (over = {}) => ({
  id: 'q1', kwant_id: 'cto', channel_id: null, direct_kwant_id: null, thread_id: 'th1', turn_id: 't1',
  question: 'Which region?', choices: ['EU', 'US'], status: 'open', answer: null, message_id: 'm1',
  created_at: '2026-10-17T10:00:00Z', answered_at: null, ...over,
});

test('card meta parses from a string or object and ignores other kinds', () => {
  const m = q.parseQuestionMeta(metaJson());
  assert.deepEqual(m.choices, ['EU', 'US']);
  assert.equal(m.status, 'open');
  assert.equal(q.parseQuestionMeta({ kind: 'ask_user', question_id: 'x', question: 'y', choices: ['a', 1, 'b'], status: 'weird' }).status, 'open');
  assert.deepEqual(q.parseQuestionMeta({ kind: 'ask_user', question_id: 'x', question: 'y', choices: ['a', 1, 'b'] }).choices, ['a', 'b']);
  assert.equal(q.parseQuestionMeta('{"kind":"delegation_started"}'), null);
  assert.equal(q.parseQuestionMeta('not json'), null);
  assert.equal(q.parseQuestionMeta(null), null);
});

test('the live record wins over stale meta', () => {
  const m = q.parseQuestionMeta(metaJson());
  assert.equal(q.questionView(m, rec({ status: 'answered', answer: 'EU' })).answer, 'EU');
  assert.equal(q.questionView(m, rec({ id: 'other', status: 'cancelled' })).status, 'open', 'another question’s record is ignored');
  assert.equal(q.questionView(m, null), m);
});

test('question_updated: newest state, closed never reopens', () => {
  let map = q.hydrateQuestions([rec()]);
  const same = q.applyQuestion(map, rec());
  assert.equal(same, map);
  map = q.applyQuestion(map, rec({ status: 'answered', answer: 'US' }));
  assert.equal(map.q1.status, 'answered');
  assert.equal(q.applyQuestion(map, rec()), map, 'a late open event loses');
});

test('contexts, waiting turns and the newest question per agent', () => {
  const map = q.hydrateQuestions([
    rec(),
    rec({ id: 'q2', thread_id: null, direct_kwant_id: 'cto', turn_id: null, created_at: '2026-10-17T11:00:00Z' }),
    rec({ id: 'q3', thread_id: null, channel_id: 'general', turn_id: 't3', status: 'answered' }),
  ]);
  assert.equal(q.questionContextId(map.q1), 'th1');
  assert.equal(q.questionContextId(map.q2), 'cto');
  assert.deepEqual(Object.keys(q.openQuestionsByContext(map)).sort(), ['cto', 'th1']);
  assert.deepEqual([...q.turnsAwaitingAnswer(map)], ['t1'], 'only open questions with a live turn');
  assert.equal(q.latestOpenQuestion(map, 'cto').id, 'q2');
  assert.equal(q.latestOpenQuestion(map, 'nobody'), null);
});

test('typed answers are checked before sending', () => {
  assert.match(q.answerTextError('  '), /Type an answer/);
  assert.match(q.answerTextError('x'.repeat(2001)), /2000/);
  assert.equal(q.answerTextError('EU please'), null);
});

const props = (over = {}) => ({
  meta: q.parseQuestionMeta(metaJson()), messageId: 'm1', kwantName: 'CTO', waiting: true,
  onAnswer: async () => {}, onDismiss: async () => {}, ...over,
});

test('an open card offers the choices, a typed answer and skip; the agent is waiting, not busy', () => {
  const html = ui.render(props());
  assert.match(html, /Which region\?/);
  assert.equal((html.match(/aria-pressed="false"/g) || []).length, 2);
  assert.doesNotMatch(html, /<button[^>]*disabled=""[^>]*aria-pressed/);
  assert.match(html, /Or type your own answer/);
  assert.match(html, /Skip question/);
  assert.match(html, /CTO is waiting for your answer\. You can keep chatting meanwhile\./);
  assert.doesNotMatch(html, /busy|elsewhere/i);
});

test('after the wait ran out the card stays answerable', () => {
  const html = ui.render(props({ waiting: false }));
  assert.match(html, /stopped waiting\. Answer any time/);
  assert.match(html, /Send answer/);
});

test('answered and dismissed cards are closed', () => {
  const answered = ui.render(props({ live: rec({ status: 'answered', answer: 'US' }) }));
  assert.match(answered, /aria-pressed="true"/);
  assert.match(answered, /Answered/);
  assert.doesNotMatch(answered, /Send answer|Skip question/);
  const typed = ui.render(props({ live: rec({ status: 'answered', answer: 'Both, EU first' }) }));
  assert.match(typed, /Both, EU first/, 'a typed answer is shown');
  const dismissed = ui.render(props({ meta: q.parseQuestionMeta(metaJson({ status: 'cancelled' })) }));
  assert.match(dismissed, /Dismissed/);
  assert.equal((dismissed.match(/disabled=""/g) || []).length, 2, 'every choice disabled');
});

test('a turn paused on a question reads "Waiting for you", not "Working"', async () => {
  const th = await loadBundled(`export * from './src/threads';`);
  const turns = th.hydrateTurns([
    { turn_id: 't1', kwant_id: 'cto', context_id: 'cto', thread_id: null, state: 'running', since: '2026-10-17T10:00:00Z' },
  ]);
  const direct = { id: 'cto', delegation: null };
  assert.equal(th.threadBadge(direct, turns).label, 'Working');
  assert.deepEqual(th.threadBadge(direct, turns, q.turnsAwaitingAnswer({ q1: rec({ turn_id: 't1' }) })), { kind: 'waiting', label: 'Waiting for you' });
  // A second request it is working on in the same chat wins.
  const both = th.hydrateTurns([
    { turn_id: 't1', kwant_id: 'cto', context_id: 'cto', thread_id: null, state: 'running', since: '2026-10-17T10:00:00Z' },
    { turn_id: 't2', kwant_id: 'cto', context_id: 'cto', thread_id: null, state: 'running', since: '2026-10-17T10:01:00Z' },
  ]);
  assert.equal(th.threadBadge(direct, both, new Set(['t1'])).kind, 'working');
});
