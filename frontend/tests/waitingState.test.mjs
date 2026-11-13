// 0.9.7 waiting state (Mira's UX audit #2, autonomy/waiting-state/brief.md): while the only open thing in
// a chat is a bot's question, the chat waits for the person. One amber banner replaces the "summarizing"
// banner, the working card and the "Paused" box hide, the composer keeps Send, and Stop lives only on the
// banner's ghost button. The question card says when it was asked and has no spinner or running clock.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const w = await loadBundled(`export * from './src/waitingState';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { QuestionCard } from './src/components/QuestionCard';
  import { WaitingBanner } from './src/components/WaitingBanner';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  export const question = (props) => renderToStaticMarkup(React.createElement(QuestionCard, props));
  export const banner = (props) => renderToStaticMarkup(React.createElement(WaitingBanner, props));
`);

const ask = { streaming: true, awaitingAnswer: true };

test('only an ask is open: a running turn paused on a question, nothing else going on', () => {
  assert.equal(w.onlyAskOpen(ask), true);
  assert.equal(w.onlyAskOpen({ ...ask, streaming: false }), false, 'no turn running');
  assert.equal(w.onlyAskOpen({ ...ask, awaitingAnswer: false }), false, 'working, not waiting');
  assert.equal(w.onlyAskOpen({ ...ask, awaitingBrowser: true }), false, 'a browser step has its own card');
  assert.equal(w.onlyAskOpen({ ...ask, approvalOpen: true }), false, 'an approval is open too');
  assert.equal(w.onlyAskOpen({ ...ask, queued: true }), false, 'waiting in line');
  assert.equal(w.onlyAskOpen({ ...ask, writing: true }), false, 'words are streaming');
});

test('banner text names the bot and never says anyone is summarizing or speaking', () => {
  assert.equal(w.waitingBannerText({ kind: 'answer', bot: 'Ann' }), 'Waiting for you: Ann needs one answer to finish');
  assert.equal(w.waitingBannerText({ kind: 'answer', bot: '  ' }), 'Waiting for you: A bot needs one answer to finish');
  assert.equal(w.waitingStopLabel(true), 'Stop team');
  assert.equal(w.waitingStopLabel(false), 'Stop');
  const html = ui.banner({ lines: [{ kind: 'answer', bot: 'Ann' }], stopLabel: 'Stop team', onStop: () => {}, onShow: () => {} });
  assert.match(html, /role="status"/);
  assert.match(html, /Waiting for you: Ann needs one answer to finish/);
  assert.match(html, /data-waiting-stop=""[^>]*>.*Stop team<\/button>/);
  assert.doesNotMatch(html, /summariz|speaking|animate-spin|bg-red/);
  // Several lines fit (a waiting check can join later); none renders nothing.
  const two = ui.banner({ lines: [{ kind: 'answer', bot: 'Ann' }, { kind: 'answer', bot: 'Ben' }], stopLabel: 'Stop' });
  assert.equal(two.match(/data-waiting-line="answer"/g).length, 2);
  assert.equal(ui.banner({ lines: [], stopLabel: 'Stop' }), '');
});

test('asked N min ago: just now, minutes, hours; server times without a zone are UTC', () => {
  const now = Date.parse('2026-11-13T07:30:00Z');
  assert.equal(w.askedAgo('2026-11-13T07:29:40Z', now), 'asked just now');
  assert.equal(w.askedAgo('2026-11-13T07:29:00Z', now), 'asked 1 min ago');
  assert.equal(w.askedAgo('2026-11-13 07:18:00', now), 'asked 12 min ago');
  assert.equal(w.askedAgo('2026-11-13T05:10:00+00:00', now), 'asked 2 h ago');
  assert.equal(w.askedAgo('2026-11-13T07:31:00Z', now), 'asked just now', 'a clock a little ahead');
  assert.equal(w.askedAgo('', now), null);
  assert.equal(w.askedAgo('nonsense', now), null);
});

const bots = [{ id: 'k-ann', name: 'Ann' }, { id: 'k-ben', name: 'Ben' }, { id: 'k-cody', name: 'Cody' }];
const botOf = (id) => bots.find((b) => b.id === id || b.name === id) ?? null;
const turn = [
  { sender_role: 'user', sender_id: 'me' },
  { sender_role: 'kwant', sender_id: 'k-ben' }, // an earlier turn
  { sender_role: 'user', sender_id: 'me' },
  { sender_role: 'kwant', sender_id: 'k-ann' },
  { sender_role: 'kwant', sender_id: 'Ben' }, // older messages name the sender
  { sender_role: 'kwant', sender_id: 'k-cody' },
  { sender_role: 'kwant', sender_id: 'k-ben' },
];

test('turn rail and sidebar Done lines: teammates who replied since your message, not the asker', () => {
  assert.deepEqual(w.turnRail(turn, 'k-ann', botOf), [
    { name: 'Ben', state: 'done' },
    { name: 'Cody', state: 'done' },
    { name: 'You pick', state: 'you' },
    { name: 'Ann wraps up', state: 'next' },
  ]);
  assert.deepEqual(w.turnRail(turn.slice(0, 4), 'k-ann', botOf), [], 'nobody else replied yet: no rail');
  assert.deepEqual(w.doneLines(turn, { id: 'k-ann' }, true, botOf), { 'k-ben': 'Done · replied here', 'k-cody': 'Done · replied here' });
  assert.deepEqual(w.doneLines(turn, { id: 'k-ann' }, false, botOf), {}, 'a single bot chat');
  assert.deepEqual(w.doneLines(turn, null, true, botOf), {}, 'nothing waits on you');
});

const msgs = [
  { id: 'u1', sender_role: 'user', sender_id: 'me', text: 'Plan our spring offer.' },
  { id: 'b1', sender_role: 'kwant', sender_id: 'k-ben', text: 'Here is my take.' },
];
const chatProps = (over = {}) => ({
  title: 'spring-launch', subtitle: '', messages: msgs, streamingText: '', streamingThinking: '', isStreaming: true,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, onCancelDiscussion: () => {},
  kwants: bots, teams: [], awaitingAnswer: true, awaitingBot: { id: 'k-ann', name: 'Ann' },
  discussionState: { inProgress: true, currentSpeaker: 'Ann', currentTurn: 3, totalTurns: 3, kind: 'discussion', phase: 'summary' },
  ...over,
});
const submit = (html) => /<button[^>]*type="submit"[^>]*>/.exec(html)?.[0];

test('team turn waiting on an answer: amber banner, no summarizing, no working card, Send stays', () => {
  const html = chat(chatProps());
  assert.match(html, /data-waiting-banner/);
  assert.match(html, /Waiting for you: Ann needs one answer to finish/);
  assert.doesNotMatch(html, /summarizing/, 'nobody is summarizing while the turn waits');
  assert.doesNotMatch(html, /Paused on the question above/);
  assert.doesNotMatch(html, /Waiting for your answer/, 'the live block with its label is gone');
  assert.doesNotMatch(html, /Stop Discussion/, 'no red Stop in the composer or the old banner');
  assert.match(html, /Stop team<\/button>/);
  assert.match(html, /data-waiting-rail[\s\S]*Ben[\s\S]*You pick[\s\S]*Ann wraps up/);
  assert.match(submit(html), /aria-label="Send \(answers the question\)"/);
});

function chat(props) {
  return ui.chat(props);
}

test('single bot waiting on an answer: banner Stop, no team words', () => {
  const html = chat(chatProps({ title: 'Ann', discussionState: null, home: { kind: 'bot', bot: { id: 'k-ann', name: 'Ann' } }, messages: msgs.slice(0, 1) }));
  assert.match(html, /Waiting for you: Ann needs one answer to finish/);
  assert.match(html, /data-waiting-stop=""[^>]*>.*>Stop<\/button>/);
  assert.doesNotMatch(html, /Stop team|data-waiting-rail/);
  assert.ok(submit(html), 'Send is there');
});

test('anything else open keeps today\'s view: a browser step, words streaming, nothing waiting', () => {
  for (const over of [{ awaitingBrowser: true }, { streamingText: 'Here is' }, { awaitingAnswer: false }]) {
    const html = chat(chatProps(over));
    assert.doesNotMatch(html, /data-waiting-banner/, JSON.stringify(over));
  }
  const working = chat(chatProps({ awaitingAnswer: false }));
  assert.match(working, /Stop Discussion/, 'the discussion banner and red Stop come back');
  // After answering, nothing runs: plain Send, no banner.
  const after = chat(chatProps({ isStreaming: false, awaitingAnswer: false, discussionState: null }));
  assert.doesNotMatch(after, /data-waiting-banner|Stop team/);
  assert.match(submit(after), /aria-label="Send"/);
});

const meta = { kind: 'question', question_id: 'q1', question: 'Which spring offer should we run?', choices: ['15% off', '20% off'], status: 'open' };

test('ask card: amber border, asked N min ago, no spinner, choices at least 44 px everywhere', () => {
  const live = { id: 'q1', kwant_id: 'k-ann', question: meta.question, choices: meta.choices, status: 'open', created_at: new Date(Date.now() - 3 * 60000).toISOString(), turn_id: 't1' };
  const html = ui.question({ meta, live, messageId: 'm1', kwantName: 'Ann', waiting: true, onAnswer: async () => {}, onDismiss: async () => {} });
  assert.match(html, /border-amber-400\/60/);
  assert.match(html, /data-asked-ago="">.*asked 3 min ago/);
  assert.doesNotMatch(html, /animate-spin/);
  const choices = [...html.matchAll(/<button([^>]*aria-pressed[^>]*)>/g)].map((x) => x[1]);
  assert.equal(choices.length, 2);
  for (const c of choices) assert.match(c, /min-h-\[48px\] max-sm:!min-h-\[48px\] sm:min-h-\[44px\]/);
  // An answered card says nothing about when it was asked.
  const done = ui.question({ meta: { ...meta, status: 'answered', answer: '15% off' }, live: { ...live, status: 'answered' }, messageId: 'm1', kwantName: 'Ann', onAnswer: async () => {}, onDismiss: async () => {} });
  assert.doesNotMatch(done, /asked .* ago|asked just now/);
});

test('the sidebar draws Done lines quietly, and never over a busy or needs-you line', () => {
  const src = readFileSync(new URL('../src/components/Sidebar.tsx', import.meta.url), 'utf8');
  assert.match(src, /const doneLine = !busyLine && !warn \? doneKwants\[k\.id\] : undefined;/);
  assert.match(src, /data-bot-done=""/);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /doneKwants=\{doneKwants\}/);
  assert.match(app, /awaitingBot=\{awaitingBotHere\}/);
});

const d = await loadBundled(`export * from './src/discussionThreads';`);

test('earlier replies fold into the existing "Team discussed" row only while the team waits on an answer', () => {
  const ms = [
    { id: 'u', sender_id: 'me', sender_role: 'user' },
    { id: 'a1', sender_id: 'Ann', reasoning_content: 'Team Discussion • Turn 1' },
    { id: 'b1', sender_id: 'Ben', reasoning_content: 'Team Discussion • Turn 2' },
    { id: 'q', sender_id: 'Ann', reasoning_content: '' }, // the question card
  ];
  assert.deepEqual(d.groupDiscussionThreads(ms).map((i) => i.kind), ['message', 'message', 'message', 'message'], 'today: inline');
  const folded = d.groupDiscussionThreads(ms, { foldOpen: true });
  assert.deepEqual(folded.map((i) => i.kind), ['message', 'thread', 'message']);
  assert.deepEqual(folded[1].speakers, ['Ann', 'Ben']);
  assert.equal(folded[2].message.id, 'q', 'the question stays out of the fold');
  // An earlier unfinished run stays inline; only the last run folds.
  const two = [...ms.slice(0, 2), { id: 'x', sender_id: 'me', sender_role: 'user' }, ...ms.slice(2)];
  assert.deepEqual(d.groupDiscussionThreads(two, { foldOpen: true }).map((i) => i.kind), ['message', 'message', 'message', 'thread', 'message']);
});
