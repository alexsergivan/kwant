// 0.9.8 first screen for anyone (autonomy/first-screen/design.md, Mira). A: Dev mode and Approvals leave the
// sidebar for Space settings, the header names them, and a one-time pointer says where they went. C: #general's
// three role asks. D: Get started step 3 when the space already has a team. E: the "Connect an AI account" card.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));
g.fetch = g.window.fetch = async () => new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { Sidebar } from './src/components/Sidebar';
  import { SpaceSubline, MovedPointer } from './src/components/FirstScreenBits';
  import { BotHome, AccountCard } from './src/components/BotHome';
  export { React };
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/firstScreen';
  export { suggestedPrompts } from './src/suggestedPrompts';
  export { GetStarted } from './src/components/GetStarted';
  export { MovedPointer, AccountCard };
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  const engines = { engines: [], loading: false, error: null, refresh: () => {} };
  export const sidebar = (p) =>
    r(Sidebar, {
      spaces: [], onSwitchSpace() {}, onCreateSpace() {}, onOpenSpaceSettings() {}, onOpenArchived() {}, onOpenSearch() {}, onOpenRoutines() {},
      channels: [], teams: [], selectedId: '', isChannel: false, onSelectKwant() {}, onSelectChannel() {}, onSelectTeam() {}, onSummonKwant() {}, onCreateTeam() {},
      engines, kwants: [], ...p,
    });
  export const subline = (p) => r(SpaceSubline, p);
  export const pointer = (p) => r(MovedPointer, { phone: false, onOpen() {}, onHide() {}, ...p });
  export const botHome = (p) => r(BotHome, p);
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const settle = async (n = 6) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() => root.render(el));
  return { host, done: () => (root.unmount(), host.remove()) };
}
const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const space = (approvals, counts = { bots: 3, ready_accounts: 1, unread: 0 }) => ({ id: 'spc_1', name: 'Work', icon: 'W', color: null, sort: 0, is_default: true, archived_at: null, created_at: '', general_channel_id: null, counts, approvals });

// ------------------------------------------------------------------ A: the header sub-line

test('A: the header sub-line names Dev mode and Approvals off right after the bot count, in that order', () => {
  const c = { bots: 3, ready_accounts: 1, unread: 0 };
  const words = (s) => m.spaceSublineParts(c, s).map((p) => (p.tone ? `<${p.text}>` : p.text)).join(' · ');
  assert.equal(words({}), '3 bots · 1 account', 'default: unchanged');
  assert.equal(words({ approvals: 'ask' }), '3 bots · 1 account', 'Ask adds nothing');
  assert.equal(words({ approvals: 'smart' }), '3 bots · 1 account', 'When risky adds nothing');
  assert.equal(words({ devOn: true }), '3 bots · <Dev mode> · 1 account');
  assert.equal(words({ approvals: 'off' }), '3 bots · <Approvals off> · 1 account');
  assert.equal(words({ devOn: true, approvals: 'off' }), '3 bots · <Dev mode> · <Approvals off> · 1 account');
  assert.deepEqual(m.spaceSublineParts(undefined, { devOn: true }), [], 'no counts yet: nothing');
});

test('A: the sub-line colours the words (sky, amber) and its title has the whole line', () => {
  const html = m.subline({ counts: { bots: 5, ready_accounts: 2, unread: 0 }, devOn: true, approvals: 'off' });
  assert.match(html, /title="5 bots · Dev mode · Approvals off · 2 accounts"/);
  assert.match(html, /class="text-sky-300" data-subline-status="dev">Dev mode</);
  assert.match(html, /class="text-warn" data-subline-status="off">Approvals off</);
  assert.match(html, /truncate/);
  assert.equal(text(html), '5 bots · Dev mode · Approvals off · 2 accounts');
});

test('A: the sidebar has no Space controls block, Dev mode toast or flip note; the header says Approvals off (desktop and phone)', () => {
  for (const phone of [false, true]) {
    const html = m.sidebar({ space: space('off'), phone, section: phone ? 'chats' : undefined });
    assert.doesNotMatch(html, /data-space-controls|data-sc-row|data-dev-switch|data-dev-on-toast|data-dev-flip/);
    assert.match(html, /data-subline-status="off"[^>]*>Approvals off</);
    assert.match(html, /aria-label="Work chats and bots"/);
  }
  const app = src('components/Sidebar.tsx');
  assert.doesNotMatch(app, /<SpaceControls|<DevOnToast|<DevFlipNote/);
});

// ------------------------------------------------------------------ A: the "moved" pointer

test('A: the pointer shows only to someone who used kwant before, until Open or Hide sets the key', () => {
  assert.equal(m.movedPointerShows({ established: true, seen: false }), true);
  assert.equal(m.movedPointerShows({ established: false, seen: false }), false, 'a brand-new person');
  assert.equal(m.movedPointerShows({ established: null, seen: false }), false, 'still working it out');
  assert.equal(m.movedPointerShows({ established: true, seen: true }), false);
  localStorage.clear();
  assert.equal(m.movedSeen(), false);
  m.markMovedSeen();
  assert.equal(localStorage.getItem('kwnt:moved-space-controls'), '1');
  assert.equal(m.movedSeen(), true);
  localStorage.clear();
});

test('A: the pointer sits under the header with its copy, Open and Hide; nothing without the prop', () => {
  const on = m.sidebar({ space: space('ask'), movedPointer: true, onOpenMoved() {}, onHideMoved() {} });
  const header = on.indexOf('data-space-header');
  const ptr = on.indexOf('data-moved-pointer');
  assert.ok(header > -1 && ptr > header, 'right under the header');
  assert.match(text(on), /Dev mode and Approvals moved to this space's settings\. Open/);
  assert.match(on, /aria-label="Hide"/);
  assert.doesNotMatch(m.sidebar({ space: space('ask') }), /data-moved-pointer/);
});

test('A: Open and Hide call back; on a phone both have a 44 px hit area, 24 px Hide on desktop', () => {
  let opened = 0;
  let hidden = 0;
  const r = mount(h(m.MovedPointer, { phone: true, onOpen: () => opened++, onHide: () => hidden++ }));
  const open = r.host.querySelector('[data-moved-open]');
  const hide = r.host.querySelector('[data-moved-hide]');
  assert.match(open.className, /min-h-11 min-w-11/);
  assert.match(hide.className, /min-h-11 min-w-11/);
  open.click();
  hide.click();
  assert.deepEqual([opened, hidden], [1, 1]);
  r.done();
  assert.match(m.pointer({ phone: false }), /data-moved-hide="true" class="[^"]*h-6 w-6/);
});

test('A: Open goes to Space settings › General and rings the Dev mode card once; App gives the pointer What\'s new\'s "established"', async () => {
  const card = document.createElement('form');
  card.setAttribute('data-dev-settings', '');
  let scrolled = 0;
  card.scrollIntoView = () => scrolled++;
  document.body.appendChild(card);
  m.ringDevSettings(document);
  assert.equal(scrolled, 1);
  assert.ok(card.classList.contains('ring-2') && card.classList.contains('ring-sky-400/55'));
  await new Promise((r) => setTimeout(r, 1400));
  assert.ok(!card.classList.contains('ring-2') && !card.hasAttribute('data-moved-ring'), 'gone after 1.2 s and the fade');
  card.remove();
  const app = src('App.tsx');
  assert.match(app, /movedPointer=\{movedPointerShows\(\{ established: whatsNewEstablished, seen: movedHidden \}\)\}/);
  assert.match(app, /const openMoved = \(\) => \{\s*openSpaceSettings\('general'\);\s*ringDevSettings\(\);\s*hideMoved\(\);/);
  assert.match(app, /<GetStarted[\s\S]*?onDismissed=\{hideMoved\}/, 'finishing Get started (someone new) hides the pointer for good');
});

// ------------------------------------------------------------------ C: #general's three asks

test('C: #general with the Helpers team shows "What do you want done?" and its three role asks, no filler', () => {
  const bots = [
    { name: 'Assistant', title: 'Personal Assistant', avatar_sprite: 'assistant' },
    { name: 'Researcher', title: 'Researcher', avatar_sprite: 'researcher' },
    { name: 'Writer', title: 'Writer', avatar_sprite: 'writer' },
  ];
  const asks = m.suggestedPrompts({ kind: 'general', bots });
  assert.equal(asks.length, 3);
  assert.ok(!asks.includes('Who here can help me with what?'));
});

// ------------------------------------------------------------------ E: the account card

test('E: the account card shows only when nothing can run and Get started is not on screen', () => {
  assert.equal(m.accountCardShows({ canRun: false, getStartedShowing: false }), true);
  assert.equal(m.accountCardShows({ canRun: null, getStartedShowing: false }), false, 'still checking');
  assert.equal(m.accountCardShows({ canRun: undefined, getStartedShowing: false }), false);
  assert.equal(m.accountCardShows({ canRun: true, getStartedShowing: false }), false);
  assert.equal(m.accountCardShows({ canRun: false, getStartedShowing: true }), false, 'Get started covers it');
  const cv = src('components/ChatView.tsx');
  assert.match(cv, /accountCardShows\(\{ canRun, getStartedShowing: !!welcome \}\)/);
});

test('E: the card says what to connect, lists the four ways and opens Accounts; #general swaps its asks for it', () => {
  let opened = 0;
  const r = mount(h(m.AccountCard, { onOpenAccounts: () => opened++ }));
  const region = r.host.querySelector('[role="region"][aria-label="Connect an AI account"]');
  assert.ok(region);
  assert.equal(region.querySelector('h3').textContent, 'Connect an AI account to start');
  assert.deepEqual([...region.querySelectorAll('li')].map((li) => li.textContent), ['Claude Pro or Max', 'ChatGPT plan', 'Grok', 'API key']);
  const btn = r.host.querySelector('[data-connect-account]');
  assert.equal(btn.textContent, 'Connect an account');
  assert.match(btn.className, /h-11 w-full/, '44 px, full width on a phone');
  btn.click();
  assert.equal(opened, 1);
  assert.match(r.host.textContent, /Nothing runs and nothing is made until one is connected\./);
  r.done();
  const home = { kind: 'general' };
  const asleep = m.botHome({ home, team: [], prompts: ['Plan my week'], canRun: false, onPick() {}, accountCard: true });
  assert.match(asleep, /data-account-card/);
  assert.doesNotMatch(text(asleep), /Plan my week/, 'no asks to tap while nothing can run');
  assert.doesNotMatch(text(asleep), /Type @ to ask one bot yourself\./, 'no @ footer while nothing can run');
  const awake = m.botHome({ home, team: [], prompts: ['Plan my week'], canRun: true, onPick() {} });
  assert.doesNotMatch(awake, /data-account-card/);
  assert.match(text(awake), /What do you want done\?/);
  assert.match(text(awake), /Type @ to ask one bot yourself\./);
});

// ------------------------------------------------------------------ D: Get started step 3

const helpersTpl = { id: 'helpers', handle: 'helpers', card_line: 'Plans, research and drafts for everyday asks.' };
const inboxTpl = { id: 'inbox', handle: 'inbox', card_line: 'Sorts and answers your email.' };
const bots3 = [
  { id: 'w', name: 'Writer', avatar_sprite: 'writer' },
  { id: 'a', name: 'Assistant', avatar_sprite: 'assistant' },
  { id: 'r', name: 'Researcher', avatar_sprite: 'researcher' },
];
const helpers = { id: 't1', name: 'helpers', lead_kwant_id: 'a', member_kwants: ['r', 'w'], created_at: '2026-11-13T10:00:00Z' };

test('D: with a team, step 3 is "Your team" with a Start tile of its real names, and hides its template', () => {
  const v = m.teamStepView({ teams: [helpers], bots: bots3, templates: [helpersTpl, inboxTpl] });
  assert.equal(v.title, 'Your team');
  assert.equal(v.intro, 'Your space comes with a team for everyday asks. Start with it, or add a team made for one job.');
  assert.equal(v.start.handle, 'helpers');
  assert.deepEqual(v.start.members.map((b) => b.name), ['Assistant', 'Researcher', 'Writer'], 'lead first');
  assert.equal(v.start.line, 'Assistant, Researcher and Writer. Plans, research and drafts for everyday asks.');
  assert.deepEqual(v.templateIds, ['inbox'], 'Helpers does not show twice');
  // An upgraded space: core-engineering, made from no template; Helpers is offered to add.
  const core = { id: 't0', name: 'core-engineering', lead_kwant_id: 'x', member_kwants: '["y","z"]' };
  const cv = m.teamStepView({ teams: [core], bots: [{ id: 'x', name: 'Architect' }, { id: 'y', name: 'TechLead' }, { id: 'z', name: 'DevOps' }], templates: [helpersTpl, inboxTpl] });
  assert.equal(cv.start.handle, 'core-engineering');
  assert.equal(cv.start.line, 'Architect, TechLead and DevOps.');
  assert.deepEqual(cv.templateIds, ['helpers', 'inbox']);
  // used_in_space and this browser's record also hide a template.
  assert.deepEqual(m.teamStepView({ teams: [core], bots: [], templates: [{ ...inboxTpl, used_in_space: true }, helpersTpl] }).templateIds, ['helpers']);
  assert.deepEqual(m.teamStepView({ teams: [{ ...core, id: 'tx', name: 'mail' }], bots: [], templates: [inboxTpl], madeFrom: (id) => (id === 'tx' ? 'inbox' : null) }).templateIds, []);
});

test('D: with no team, step 3 stays "Pick a team" with no Start tile; the done value is @handle after Start', () => {
  const v = m.teamStepView({ teams: [], bots: bots3, templates: [helpersTpl] });
  assert.equal(v.title, 'Pick a team');
  assert.equal(v.intro, 'Each team has a lead, helpers and skills for one job. You can add more teams later.');
  assert.equal(v.start, null);
  assert.deepEqual(v.templateIds, ['helpers'], 'Helpers is the first template tile');
  assert.equal(m.teamDoneValue({ imported: false, mainSpace: true, startHandle: 'helpers' }), '@helpers');
  assert.equal(m.teamDoneValue({ imported: false, mainSpace: true, startHandle: null }), 'Starter bots');
  assert.equal(m.teamDoneValue({ imported: false, mainSpace: false, startHandle: null }), 'Your bots');
  assert.equal(m.teamDoneValue({ imported: true, mainSpace: true, startHandle: 'helpers' }), 'Your imported bots');
});

const ready = (id) => ({ id, name: id, kind: 'cli', status: 'ready', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: true, uses: null });

test('D: Get started with a team renders the Start tile and "Or add a team for one job"; Start keeps the team and closes the card', async () => {
  localStorage.clear();
  let dismissed = 0;
  const r = mount(h(m.GetStarted, { space: 'fs1', bots: bots3, teams: [helpers], engines: [ready('claude')], reloadEngines() {}, onDismissed: () => dismissed++, hostOverride: null }));
  await settle();
  const heads = [...r.host.querySelectorAll('h3')].map((x) => x.textContent);
  assert.equal(heads[2], 'Step 3 of 3, Your team, open');
  const tile = r.host.querySelector('[data-start-team="helpers"]');
  assert.ok(tile);
  assert.match(tile.textContent, /Start with @helpersAssistant, Researcher and Writer\.Start$/);
  assert.equal(r.host.querySelector('[data-keep-starter]'), null, 'the Start tile replaces Keep');
  assert.equal(r.host.querySelector('[data-add-team-label]').textContent, 'Or add a team for one job');
  assert.match(r.host.textContent, /Your space comes with a team for everyday asks\./);
  r.host.querySelector('[data-start-button]').click();
  await settle();
  assert.equal(dismissed, 1);
  r.done();
  // No team: today's step.
  const r2 = mount(h(m.GetStarted, { space: 'fs2', bots: bots3, teams: [], engines: [ready('claude')], reloadEngines() {}, onDismissed() {}, hostOverride: null }));
  await settle();
  assert.equal([...r2.host.querySelectorAll('h3')][2].textContent, 'Step 3 of 3, Pick a team, open');
  assert.equal(r2.host.querySelector('[data-start-team]'), null);
  assert.ok(r2.host.querySelector('[data-keep-starter]'));
  r2.done();
});

// ------------------------------------------------------------------ F: words on these screens

test('F: "the best fit" replaces "Laya picks" on the first screens, and the welcome says "Welcome to kwant"', () => {
  assert.match(src('App.tsx'), /`The best fit of your \$\{kwants\.length\} \$\{kwants\.length === 1 \? 'bot' : 'bots'\} answers`/);
  assert.match(src('botHome.ts'), /label: 'Best fit', detail: 'The best bot for each message answers\. Type @ and a name to choose one yourself\.'/);
  assert.match(src('components/Sidebar.tsx'), /'The best fit answers here'/);
  assert.match(src('components/SpaceReadyCard.tsx'), /The best fit answers, only from/);
  assert.match(src('components/SetupNeededCard.tsx'), /Sign in with Claude, ChatGPT or Grok, or add an API key\./);
  assert.match(src('components/GetStarted.tsx'), /Open claude\.ai and allow kwant\./);
  assert.match(src('gettingStarted.ts'), /title: 'Welcome to kwant',/);
  assert.match(src('gettingStarted.ts'), /'Last step: meet your team\.'/);
  for (const f of ['App.tsx', 'botHome.ts', 'components/Sidebar.tsx', 'components/SpaceReadyCard.tsx', 'components/FirstRunWelcome.tsx']) {
    assert.doesNotMatch(src(f), /Laya picks who answers|label: 'Laya picks'/, f);
  }
});
