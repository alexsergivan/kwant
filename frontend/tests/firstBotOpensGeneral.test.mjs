// 0.9.54 first-bot-opens-general (autonomy/first-bot-opens-general/brief.md, Ada). The space's checklist says
// "#general opens with it", so a space's first bot (not made inside a team) opens #general, not a direct chat.
// The card drops "{space}'s" ("Trips's"). On a phone, #general opens first while its Get started card shows
// (simplify-review Minor d).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' });
for (const k of ['window', 'document', 'navigator', 'localStorage', 'sessionStorage']) {
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SpaceReadyCard } from './src/components/SpaceReadyCard';
  export * from './src/firstBot';
  export const card = (spaceName) => renderToStaticMarkup(
    React.createElement(SpaceReadyCard, { spaceName, tile: null, readyAccounts: 1, botCount: 0, onAddAccount() {}, onNewBot() {} }),
  );
`);
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/\s+/g, ' ');

// ------------------------------------------------------------------ the first bot
test('the first bot in a space opens #general', () => {
  assert.equal(m.afterNewBot({ botsBefore: 0, inTeam: false }), 'general');
});

test('later bots open their own chat, as before', () => {
  assert.equal(m.afterNewBot({ botsBefore: 1, inTeam: false }), 'bot');
  assert.equal(m.afterNewBot({ botsBefore: 7, inTeam: false }), 'bot');
});

test('a bot made inside a team opens the team, first or not', () => {
  assert.equal(m.afterNewBot({ botsBefore: 0, inTeam: true }), 'team');
  assert.equal(m.afterNewBot({ botsBefore: 3, inTeam: true }), 'team');
});

test('#general is found by id, else by name; null when the space has none', () => {
  const chans = [{ id: 'c1', name: 'ops' }, { id: 'c2', name: 'general' }];
  assert.equal(m.generalOf(chans, 'c1')?.id, 'c1');
  assert.equal(m.generalOf(chans, null)?.id, 'c2');
  assert.equal(m.generalOf(chans, 'gone')?.id, 'c2');
  assert.equal(m.generalOf([{ id: 'c1', name: 'ops' }], null), null);
});

test('App opens #general for the first bot, after reading the channels the server just made', () => {
  const app = src('App.tsx');
  const create = app.slice(app.indexOf('const handleCreateAgent'), app.indexOf('const handleCreateChannel'));
  assert.match(create, /afterNewBot\(\{ botsBefore: kwants\.length, inTeam: !!team \}\)/);
  assert.match(create, /generalOf\(/);
  assert.match(create, /handleSelectChannel\(general\)/);
});

// ------------------------------------------------------------------ the card's copy
test('a space named "Trips": no "Trips\'s" anywhere on the card', () => {
  const t = text(m.card('Trips'));
  assert.doesNotMatch(t, /Trips's|Trips’s/);
  assert.match(t, /#general opens with it\./);
  assert.match(t, /The best fit answers, only from bots in Trips\./);
  assert.match(t, /Get Trips ready/);
});

test('the card has no possessive built from the space name', () => {
  assert.doesNotMatch(src('components/SpaceReadyCard.tsx'), /\$\{spaceName\}['’]s/);
});

// ------------------------------------------------------------------ phone: first view (Minor d)
const base = { linked: false, canRun: true, dismissed: false, hasBots: true, inGeneral: true, generalLoaded: true, cardShows: true };

test('phone: an account works and Get started shows in #general -> #general, not Chats', () => {
  assert.equal(m.phoneFirstView(base), 'general');
});

test('phone: waits for #general\'s messages before choosing (the card depends on them)', () => {
  assert.equal(m.phoneFirstView({ ...base, generalLoaded: false }), 'wait');
  assert.equal(m.phoneFirstView({ ...base, canRun: null }), 'wait');
});

test('phone: no card -> the Chats list, as now', () => {
  assert.equal(m.phoneFirstView({ ...base, cardShows: false }), 'list');
  assert.equal(m.phoneFirstView({ ...base, dismissed: true }), 'list');
});

test('phone: a link, another chat already open, or no bots (the checklist) -> left alone', () => {
  assert.equal(m.phoneFirstView({ ...base, linked: true }), 'list');
  assert.equal(m.phoneFirstView({ ...base, inGeneral: false }), 'list');
  assert.equal(m.phoneFirstView({ ...base, hasBots: false }), 'list');
});

test('phone: nothing can run yet -> #general at once (C1, unchanged), unless dismissed', () => {
  assert.equal(m.phoneFirstView({ ...base, canRun: false, generalLoaded: false, cardShows: false }), 'general');
  assert.equal(m.phoneFirstView({ ...base, canRun: false, dismissed: true }), 'list');
});
