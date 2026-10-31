// Star on GitHub card: shows once after real use (5 finished turns, 2 days since first launch),
// never while a bot needs the person, "Maybe later" snoozes 2 weeks, Star or "Don't ask again" end it.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { StarAskView, StarAskSettingsLink } from './src/components/StarAsk';
  export * from './src/starAsk';
  const noop = () => {};
  export const card = () => renderToStaticMarkup(React.createElement(StarAskView, { onStar: noop, onLater: noop, onNever: noop }));
  export const settingsLink = () => renderToStaticMarkup(React.createElement(StarAskSettingsLink, { store: { starred: noop } }));
`);

const DAY = 24 * 60 * 60 * 1000;
const T0 = Date.UTC(2026, 9, 31, 12);
const free = { needsYou: false, waiting: false };

const memStore = (init = {}) => {
  const data = { ...init };
  return { data, getItem: (k) => (k in data ? data[k] : null), setItem: (k, v) => (data[k] = String(v)) };
};

const ready = (extra = {}) => ({ firstSeen: T0, turns: s.STAR_MIN_TURNS, ...extra });

test('first run never asks, and saves the first-seen time right away', () => {
  const mem = memStore();
  const store = s.createStarAskStore(mem, () => T0);
  assert.deepEqual(store.get(), { firstSeen: T0, turns: 0 });
  assert.equal(JSON.parse(mem.data[s.STAR_ASK_KEY]).firstSeen, T0);
  assert.equal(s.shouldAskForStar(store.get(), free, T0), false);
});

test('needs 5 finished turns and 2 days', () => {
  assert.equal(s.shouldAskForStar(ready({ turns: 4 }), free, T0 + 3 * DAY), false);
  assert.equal(s.shouldAskForStar(ready(), free, T0 + 2 * DAY - 1), false);
  assert.equal(s.shouldAskForStar(ready(), free, T0 + 2 * DAY), true);
});

test('never while a bot needs the person or a question/approval waits', () => {
  const later = T0 + 3 * DAY;
  assert.equal(s.shouldAskForStar(ready(), { needsYou: true, waiting: false }, later), false);
  assert.equal(s.shouldAskForStar(ready(), { needsYou: false, waiting: true }, later), false);
});

test('Maybe later snoozes for 2 weeks, then asks again', () => {
  const now = T0 + 3 * DAY;
  const snoozed = s.snoozeStarAsk(ready(), now);
  assert.equal(s.shouldAskForStar(snoozed, free, now), false);
  assert.equal(s.shouldAskForStar(snoozed, free, now + 14 * DAY - 1), false);
  assert.equal(s.shouldAskForStar(snoozed, free, now + 14 * DAY), true);
});

test('Star and Don\'t ask again end it for good', () => {
  for (const how of ['starred', 'never']) {
    const done = s.finishStarAsk(ready(), how);
    assert.equal(s.shouldAskForStar(done, free, T0 + 365 * DAY), false, how);
    assert.equal(s.countTurn(done), done, 'stops counting');
  }
});

test('turn count stops at the threshold', () => {
  let st = { firstSeen: T0, turns: 0 };
  for (let i = 0; i < 20; i++) st = s.countTurn(st);
  assert.equal(st.turns, s.STAR_MIN_TURNS);
});

test('the store counts turns, persists every choice and survives a reload', () => {
  const mem = memStore();
  let now = T0;
  const store = s.createStarAskStore(mem, () => now);
  let calls = 0;
  store.subscribe(() => calls++);
  for (let i = 0; i < 5; i++) store.turnFinished();
  now = T0 + 2 * DAY;
  assert.equal(s.shouldAskForStar(store.get(), free, now), true);
  store.later();
  assert.equal(JSON.parse(mem.data[s.STAR_ASK_KEY]).snoozedUntil, now + s.STAR_SNOOZE_MS);
  const reloaded = s.createStarAskStore(mem, () => now + s.STAR_SNOOZE_MS);
  assert.equal(s.shouldAskForStar(reloaded.get(), free, now + s.STAR_SNOOZE_MS), true);
  reloaded.never();
  assert.equal(s.createStarAskStore(mem, () => now).get().done, 'never');
  assert.ok(calls >= 6);
});

test('broken or foreign stored values start the clock over instead of asking', () => {
  for (const raw of ['{', '"x"', '{"turns":9}', 'null']) {
    assert.deepEqual(s.parseStarAsk(raw, T0), { firstSeen: T0, turns: 0 }, raw);
  }
  assert.deepEqual(s.parseStarAsk('{"firstSeen":1,"turns":-3,"done":"maybe"}', T0), { firstSeen: 1, turns: 0 });
});

test('storage that throws never breaks the app', () => {
  const bad = { getItem: () => { throw new Error('off'); }, setItem: () => { throw new Error('off'); } };
  const store = s.createStarAskStore(bad, () => T0);
  store.turnFinished();
  store.starred();
  assert.equal(store.get().done, 'starred');
});

test('the card and settings link open the repo in the browser, with no opener', () => {
  const html = s.card();
  assert.match(html, /kwant bots run on coffee and GitHub stars/);
  assert.match(html, /Star on GitHub/);
  assert.match(html, /Maybe later/);
  assert.match(html, /Don’t ask again/);
  assert.match(html, /href="https:\/\/github.com\/alexsergivan\/kwant" target="_blank" rel="noopener noreferrer"/);
  const link = s.settingsLink();
  assert.match(link, /Star kwant bots on GitHub/);
  assert.match(link, /rel="noopener noreferrer"/);
});
