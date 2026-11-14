// 14c shell polish (Pixel, brief autonomy/shell-polish/brief.md): an answer given on a question card shows
// once (the card says "You answered: …", the saved answer message isn't drawn as a second "You" bubble
// while its card is loaded), and the phone chat header's icon buttons carry visible captions.
// Parts 1, 2, 4 and 5 are tested in attentionOrder, ia and boardLiveFade.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'FocusEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS', 'DOMException']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const store = new Map();
const ls = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
dom.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    flushSync(() => root.render(React.createElement(ChatView, p)));
    return { host, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const act = (fn) => flushSync(fn);
  export { foldedAnswerIds, answeredLine } from './src/questions';
  export const questionStub = (msg) => React.createElement('section', { 'data-message-id': msg.id, 'data-card': '' });
`);

const props = (extra = {}) => ({
  title: 'Frontend', subtitle: 'Pixel', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, onCancelDiscussion: () => {}, kwants: [], teams: [],
  home: { kind: 'bot', bot: { id: 'k-front', name: 'Frontend', title: 'Frontend engineer', avatar_sprite: 'Frontend' } }, ...extra,
});

// ---- part 6: an answer shows once ----
const T = '2026-11-14T10:00:00Z';
const card = (id, qid) => ({ id, sender_role: 'kwant', sender_id: 'k-front', text: 'Which region?', created_at: T,
  meta: JSON.stringify({ kind: 'ask_user', question_id: qid, question: 'Which region?', choices: ['US', 'EU'], status: 'answered', answer: 'EU' }) });
const cardAnswer = (id, qid, text = 'EU') => ({ id, sender_role: 'user', sender_id: 'user', text, created_at: T,
  meta: JSON.stringify({ kind: 'question_answer', question_id: qid }) });
const typed = (id, text) => ({ id, sender_role: 'user', sender_id: 'user', text, created_at: T });

test('foldedAnswerIds: the card is loaded, so its answer message folds into it', () => {
  assert.deepEqual([...ui.foldedAnswerIds([card('c1', 'q1'), cardAnswer('a1', 'q1')])], ['a1']);
  // meta may also arrive already parsed
  assert.deepEqual([...ui.foldedAnswerIds([{ ...card('c1', 'q1'), meta: JSON.parse(card('c1', 'q1').meta) }, cardAnswer('a1', 'q1')])], ['a1']);
});

test('foldedAnswerIds: the card is not loaded (an older page), so the answer stays a bubble', () => {
  assert.equal(ui.foldedAnswerIds([cardAnswer('a1', 'q1')]).size, 0);
  assert.equal(ui.foldedAnswerIds([card('c2', 'q2'), cardAnswer('a1', 'q1')]).size, 0, 'another question\'s card does not count');
});

test('foldedAnswerIds: a message typed in the composer to answer always stays', () => {
  assert.equal(ui.foldedAnswerIds([card('c1', 'q1'), typed('t1', 'EU, please')]).size, 0);
  assert.equal(ui.foldedAnswerIds([typed('t1', 'x'), { id: 'bad', meta: '{nope' }]).size, 0, 'bad meta is ignored');
});

test('answeredLine: "You picked <choice>", "You answered:" before typed words, "Answered" with no text', () => {
  assert.deepEqual(ui.answeredLine('EU', ['US', 'EU']), { lead: 'You picked EU', text: null });
  assert.deepEqual(ui.answeredLine('Both, EU first', ['US', 'EU']), { lead: 'You answered:', text: 'Both, EU first' });
  assert.deepEqual(ui.answeredLine(null, ['US']), { lead: 'Answered', text: null });
});

const bubbles = (host) => [...host.querySelectorAll('[data-message-id]')].map((n) => n.getAttribute('data-message-id'));

test('ChatView: with the card loaded the "You" bubble is not drawn; without it, it is', () => {
  let v = ui.mount(props({ messages: [card('c1', 'q1'), cardAnswer('a1', 'q1'), typed('t1', 'thanks')], renderQuestion: ui.questionStub }));
  assert.deepEqual(bubbles(v.host), ['c1', 't1'], 'one answer: on the card; the typed message stays');
  v.unmount();
  v = ui.mount(props({ messages: [cardAnswer('a1', 'q1'), typed('t1', 'thanks')], renderQuestion: ui.questionStub }));
  assert.deepEqual(bubbles(v.host), ['a1', 't1'], 'card on an older page: the bubble stays');
  v.unmount();
  v = ui.mount(props({ messages: [card('c1', 'q1'), cardAnswer('a1', 'q1')] }));
  assert.ok(bubbles(v.host).includes('a1'), 'a chat that draws no cards folds nothing');
  v.unmount();
});

// ---- part 3: the phone header's icon buttons have names ----
test('header: every icon button keeps its aria-label and has a phone caption under the icon', () => {
  const v = ui.mount(props({ onEditKwant: () => {}, onEditInstructions: () => {}, onManageTeam: () => {},
    teamMembers: [{ id: 'k-front', name: 'Frontend', title: 'Eng', avatar_sprite: 'Frontend' }] }));
  const head = v.host.querySelector('[data-chat-header]');
  const btns = [...head.querySelectorAll('.kw-head-btn')];
  const named = btns.map((b) => [b.getAttribute('aria-label'), b.querySelector('[data-head-caption]')?.textContent]);
  assert.deepEqual(named, [['Manage team', 'Team'], ['Pin instructions', 'Pin'], ['Edit Frontend', 'Edit'], ['Bot browser', 'Browser']]);
  for (const b of btns) {
    const cap = b.querySelector('[data-head-caption]');
    assert.match(cap.className, /sm:hidden/, 'the caption is phone-only');
    assert.match(b.className, /max-sm:flex-col/, 'icon above caption on a phone');
    assert.match(b.className, /max-sm:h-11/, '44 px tall on a phone');
    // Label in name: the visible caption is part of the accessible name
    assert.ok(b.getAttribute('aria-label').toLowerCase().includes(cap.textContent.toLowerCase()));
    assert.ok(b.getAttribute('title'), 'desktop keeps its tooltip');
  }
  v.unmount();
});
