// thread-waiting-once (0.9.60, Mira's m6 from the team-job UX pass, autonomy/thread-waiting-once/brief.md): when a
// bot's question waits, the chat says so once on the card and once on the Needs-you strip. The amber banner is
// not drawn while the strip shows the same question; the strip takes its ghost Stop / "Stop team" (not in a
// thread: the thread header has Stop) and the team rail. Without a matching strip the banner stays as before.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const w = await loadBundled(`export * from './src/waitingState';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { NeedsYouStrip } from './src/components/NeedsYouStrip';
  export const strip = (asks, over = {}) => React.createElement(NeedsYouStrip, { asks, onAction: () => {}, onDismiss: () => {}, onShow: () => true, hereKey: 'x', now: Date.parse('2026-11-27T16:43:00Z'), ...over });
  export const html = (el) => renderToStaticMarkup(el);
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
`);

const bots = [{ id: 'k-ann', name: 'Ann' }, { id: 'k-ben', name: 'Ben' }];
const question = (kwant_id = 'k-ann', kwant_name = 'Ann', id = 'q1') => ({
  kind: 'question', id, kwant_id, kwant_name, title: "What's the lunch budget per person?", summary: '', status: 'open', dismissable: false,
  timestamp: '2026-11-27T16:42:00Z', link: { thread_id: null, channel_id: null, direct_kwant_id: kwant_id },
});
const msgs = [
  { id: 'u1', sender_role: 'user', sender_id: 'me', text: 'Plan our spring offer.' },
  { id: 'b1', sender_role: 'kwant', sender_id: 'k-ben', text: 'Here is my take.' },
];
const teamProps = (over = {}) => ({
  title: 'spring-launch', subtitle: '', messages: msgs, streamingText: '', streamingThinking: '', isStreaming: true,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, onCancelDiscussion: () => {},
  kwants: bots, teams: [], awaitingAnswer: true, awaitingBot: { id: 'k-ann', name: 'Ann' }, home: { kind: 'team' },
  needsYouStrip: ui.strip([question()]), needsYouOpen: true, stripQuestionBots: ['k-ann'],
  ...over,
});
const ownProps = (over = {}) =>
  teamProps({ title: 'Ann', home: { kind: 'bot', bot: { id: 'k-ann', name: 'Ann' } }, messages: msgs.slice(0, 1), ...over });
const stripOf = (html) => /<section[^>]*data-needs-you-strip[\s\S]*?<\/section>/.exec(html)?.[0] ?? '';
const count = (html, re) => (html.match(re) ?? []).length;

test('the banner yields only to a strip showing the same open question', () => {
  const y = w.bannerYieldsToStrip;
  assert.equal(y({ askOnly: true, askerId: 'k-ann', stripQuestionBots: ['k-ann'] }), true);
  assert.equal(y({ askOnly: true, askerId: 'k-ann', stripQuestionBots: ['k-ben'] }), false, 'another bot asks');
  assert.equal(y({ askOnly: true, askerId: 'k-ann', stripQuestionBots: [] }), false, 'no strip here');
  assert.equal(y({ askOnly: true, askerId: 'k-ann' }), false, 'strip not told');
  assert.equal(y({ askOnly: true, askerId: null, stripQuestionBots: ['k-ann'] }), false, 'asker unknown');
  assert.equal(y({ askOnly: false, askerId: 'k-ann', stripQuestionBots: ['k-ann'] }), false, 'not the ask-only state');
  assert.equal(y({ askOnly: true, askerId: 'k-ann', stripQuestionBots: ['k-ann'], stripAside: true }), false, 'strip stepped aside');
});

test('team chat while a question waits: no banner, one strip with "Stop team" and the rail', () => {
  const html = ui.chat(teamProps());
  assert.doesNotMatch(html, /data-waiting-banner|Waiting for you: Ann/, 'the banner is not drawn');
  const s = stripOf(html);
  assert.equal(count(html, /data-needs-you-strip=/g), 1);
  assert.match(s, /Go to the question/);
  assert.match(s, /<button[^>]*data-strip-stop=""[^>]*>.*Stop team<\/button>/);
  assert.match(s, /data-waiting-rail[\s\S]*Ben[\s\S]*You pick[\s\S]*Ann wraps up/, 'the rail moves into the strip');
  assert.equal(count(html, /Stop team/g), 1, 'one Stop, nowhere else');
  assert.doesNotMatch(html, /Stop Discussion/, 'no red Stop in the composer');
  assert.match(html, /aria-label="Send \(answers the question\)"/, 'Send still answers');
});

test("a bot's own chat: no banner, the strip has a plain ghost Stop and no rail", () => {
  const html = ui.chat(ownProps());
  assert.doesNotMatch(html, /data-waiting-banner/);
  const s = stripOf(html);
  const stop = /<button[^>]*data-strip-stop=""[^>]*>/.exec(s)?.[0];
  assert.ok(stop, 'Stop is in the strip');
  assert.match(stop, /kw-btn-ghost/, 'a ghost, like the banner had and like Done, clear it');
  assert.match(s, /data-strip-stop=""[^>]*>.*>Stop<\/button>/);
  assert.doesNotMatch(html, /Stop team|data-waiting-rail/);
  // Stop sits before the primary button, the same pair as the banner's Answer / Stop and the strip's Done / Go.
  assert.ok(s.indexOf('data-strip-stop') < s.indexOf('data-strip-action'));
});

test('a thread: no banner and no strip Stop (the thread header has Stop)', () => {
  const html = ui.chat(ownProps({ threadKind: 'delegation' }));
  assert.doesNotMatch(html, /data-waiting-banner|data-strip-stop/);
  assert.match(stripOf(html), /Go to the question/);
});

test('without a matching strip the banner stays as before', () => {
  for (const over of [{ stripQuestionBots: ['k-ben'], needsYouStrip: ui.strip([question('k-ben', 'Ben')]) }, { stripQuestionBots: [], needsYouStrip: ui.strip([]) }, { stripQuestionBots: undefined }]) {
    const html = ui.chat(teamProps(over));
    assert.match(html, /data-waiting-banner/, JSON.stringify(over.stripQuestionBots));
    assert.match(html, /data-waiting-stop=""[^>]*>.*Stop team<\/button>/);
    assert.doesNotMatch(html, /data-strip-stop/, 'Stop stays on the banner only');
  }
});

test('other waits keep their view, and after the answer nothing waits', () => {
  const browser = ui.chat(teamProps({ awaitingBrowser: true }));
  assert.doesNotMatch(browser, /data-waiting-banner|data-strip-stop/, 'a browser step never had the banner');
  const after = ui.chat(teamProps({ isStreaming: false, awaitingAnswer: false, awaitingBot: null, needsYouStrip: ui.strip([]), stripQuestionBots: [], needsYouOpen: false }));
  assert.doesNotMatch(after, /data-waiting-banner|data-strip-stop|Stop team|data-needs-you-strip=/);
  assert.match(after, /aria-label="Send"/);
});

test("the strip's Stop shows on any ask up front (the banner yielded it); the rail only on the asker's question", () => {
  const turn = { bot: 'k-ann', rail: [{ name: 'Ben', state: 'done' }, { name: 'You pick', state: 'you' }], stopLabel: 'Stop team', onStop: () => {} };
  // QA (Rigel): an older ask shown first must not leave the chat without a Stop.
  const ben = ui.html(ui.strip([question('k-ben', 'Ben'), question()], { waitTurn: turn }));
  assert.match(ben, /data-strip-stop/, 'Stop stays reachable when another ask is up front');
  assert.equal((ben.match(/data-strip-stop/g) || []).length, 1);
  assert.doesNotMatch(ben, /data-waiting-rail/, 'the rail goes with the asker only');
  const ann = ui.html(ui.strip([question()], { waitTurn: turn }));
  assert.match(ann, /data-strip-stop/);
  assert.match(ann, /data-waiting-rail/);
  const thread = ui.html(ui.strip([question('k-ben', 'Ben'), question()], { waitTurn: { ...turn, onStop: undefined } }));
  assert.doesNotMatch(thread, /data-strip-stop/, 'threads keep the header Stop only');
  const phone = ui.html(ui.strip([question()], { waitTurn: turn, phone: true }));
  assert.match(/<button[^>]*data-strip-stop=""[^>]*>/.exec(phone)[0], /h-11[^"]*flex-1/, '44 px on a phone, sharing the row');
  const none = ui.html(ui.strip([question()]));
  assert.doesNotMatch(none, /data-strip-stop|data-waiting-rail/, 'no wait turn: the strip as before');
});
