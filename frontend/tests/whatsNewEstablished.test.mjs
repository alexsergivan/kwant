// 0.7.6 F-1: Get started wins over What's new. A brand-new person whose AI is already signed in gets
// the Get started card and never the sheet; a 0.7.5 user who wrote in a chat still gets it; until the
// space and the open chat have loaded, the answer waits.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { showCard, whatsNewEstablished } from './src/gettingStarted';
  export { decideOnLoad } from './src/whatsNewSheet';
`);

// What App.tsx does: the card decision feeds the What's new decision.
const decide = ({ canRun, messages = [], teams = 1, dismissed = false, started = false, loaded = true, messagesLoaded = true }) => {
  const getStartedShowing = m.showCard({ dismissed, canRun, generalMessages: messages.length, teams, started });
  const wroteHere = messages.some((x) => x.sender_role === 'user');
  return { getStartedShowing, established: m.whatsNewEstablished({ loaded, messagesLoaded, canRun, wroteHere, getStartedShowing }) };
};

test('new person with a signed-in AI: Get started shows, What\'s new is recorded silently', () => {
  const r = decide({ canRun: true, messages: [], teams: 1 });
  assert.equal(r.getStartedShowing, true);
  assert.equal(r.established, false);
  assert.equal(m.decideOnLoad(null, '0.7.6', r.established), 'record');
});

test('new person with no AI yet: not established either', () => {
  const r = decide({ canRun: false });
  assert.equal(r.getStartedShowing, true);
  assert.equal(r.established, false);
});

test('0.7.5 user who wrote in the chat gets the sheet', () => {
  const r = decide({ canRun: true, messages: [{ sender_role: 'user' }, { sender_role: 'bot' }], teams: 1 });
  assert.equal(r.getStartedShowing, false);
  assert.equal(r.established, true);
  assert.equal(m.decideOnLoad(null, '0.7.6', r.established), 'show');
});

test('0.7.5 user whose AI works and has more teams gets the sheet even in an empty chat', () => {
  assert.equal(decide({ canRun: true, teams: 3 }).established, true);
});

test('signed-in AI but Get started was dismissed: established', () => {
  assert.equal(decide({ canRun: true, dismissed: true }).established, true);
});

test('signed-in AI with Get started in progress: not established', () => {
  assert.equal(decide({ canRun: true, teams: 4, started: true }).established, false);
});

test('waits until the space has loaded', () => {
  assert.equal(decide({ canRun: true, loaded: false }).established, null);
  assert.equal(m.decideOnLoad(null, '0.7.6', null), 'wait');
});

test('waits until the engines answer', () => {
  assert.equal(decide({ canRun: null }).established, null);
});

test('waits until the open chat\'s messages have loaded, so an upgrading user is never recorded as new', () => {
  assert.equal(decide({ canRun: true, teams: 1, messagesLoaded: false }).established, null);
  assert.equal(decide({ canRun: false, messagesLoaded: false }).established, null);
});

test('a message written here counts even before everything else has loaded', () => {
  assert.equal(m.whatsNewEstablished({ loaded: false, messagesLoaded: false, canRun: null, wroteHere: true, getStartedShowing: false }), true);
});
