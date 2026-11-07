// 0.7.7 Watch live (W1–W3, design/w0-answers.md): the browsing set, the /live address, the globe on
// sidebar rows, Watch on the working strip, and the panel's switcher.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { Sidebar } from './src/components/Sidebar';
  import { WorkingStrip } from './src/components/WorkingStrip';
  import { BrowserSwitcherMenu, BrowserSwitcherRow, WatchGlobe } from './src/components/WatchLive';
  export * from './src/watchLive';
  export { parseSpacePath, spacePath } from './src/spaces';
  export { targetFromQuery } from './src/push';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  const engines = { engines: [], loading: false, error: null, refresh: () => {} };
  const space = { id: 'spc_1', name: 'Work', icon: '💼', color: null, sort: 0, is_default: false, archived_at: null, created_at: '', general_channel_id: null, counts: { bots: 2, ready_accounts: 1, unread: 0 }, approvals: 'ask' };
  export const sidebarProps = (p) => ({
    space, spaces: [], onSwitchSpace() {}, onCreateSpace() {}, onOpenSpaceSettings() {}, onOpenArchived() {}, onOpenSearch() {}, onOpenRoutines() {},
    channels: [], teams: [], selectedId: '', isChannel: false, onSelectKwant() {}, onSelectChannel() {}, onSelectTeam() {}, onSummonKwant() {}, onCreateTeam() {},
    engines, ...p,
  });
  export const sidebar = (p) => r(Sidebar, sidebarProps(p));
  export const strip = (p) => r(WorkingStrip, p);
  export const globe = (p) => r(WatchGlobe, p);
  export const row = (p) => r(BrowserSwitcherRow, p);
  export const mount = (C, p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const comp = { Sidebar, WorkingStrip, BrowserSwitcherMenu, BrowserSwitcherRow }[C];
    const render = (q) => flushSync(() => root.render(React.createElement(comp, C === 'Sidebar' ? sidebarProps(q) : q)));
    render(p);
    return { host, render, flush: (fn) => flushSync(fn), unmount: () => { root.unmount(); host.remove(); } };
  };
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const bot = (id, name) => ({ id, name, title: 'Shopper', persona_prompt: '', avatar_sprite: 'coder', driver_type: 'claude', preferred_model: 'claude:sonnet' });
const rook = bot('k_rook', 'Rook');
const mira = bot('k_mira', 'Mira');
const team = { id: 't_shop', name: 'storefront', icon: 'users', lead_kwant_id: 'k_rook', member_kwants: ['k_rook', 'k_mira'] };
const teamCh = { id: 'c_team', name: 'team-storefront', lead_kwant_id: 'k_rook', member_kwants: ['k_mira'], routing_mode: 'lead' };
const general = { id: 'c_gen', name: 'general', lead_kwant_id: 'k_rook', member_kwants: [], routing_mode: 'lead' };
const teamOf = (c) => (c.id === 'c_team' ? team : undefined);
const running = (turn_id, kwant_id, context_id) => ({ turn_id, kwant_id, context_id, state: 'running' });

// ---------------------------------------------------------------- the browsing set

test('hostOf: the page host without www, the address itself when it isn’t one', () => {
  assert.equal(m.hostOf('https://www.shop.example.com/cart?x=1'), 'shop.example.com');
  assert.equal(m.hostOf('http://localhost:5173/'), 'localhost:5173');
  assert.equal(m.hostOf('about:blank'), '');
  assert.equal(m.hostOf(''), '');
  assert.equal(m.hostOf(null), '');
});

test('noteBrowser adds an owner, refreshes its time, and keeps its page when an event names none', () => {
  let b = m.noteBrowser({}, 'k_rook', 'https://shop.example.com/', 1000);
  assert.deepEqual(b.k_rook, { owner: 'k_rook', url: 'https://shop.example.com/', host: 'shop.example.com', at: 1000 });
  b = m.noteBrowser(b, 'k_rook', undefined, 2000);
  assert.equal(b.k_rook.host, 'shop.example.com', 'a screenshot without a url keeps the page');
  assert.equal(b.k_rook.at, 2000);
  assert.equal(m.noteBrowser(b, 'default', 'https://x.example.com', 3000), b, 'the default sandbox is never a chat');
  assert.equal(m.noteBrowser(b, undefined, 'https://x.example.com', 3000), b);
});

test('expireBrowsing drops an owner after about 60 s with no event; same map when nothing expired', () => {
  const b = m.noteBrowser(m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 0), 'c_gen', 'https://mail.example.com', 50_000);
  assert.equal(m.expireBrowsing(b, 59_999), b);
  const after = m.expireBrowsing(b, 60_000);
  assert.deepEqual(Object.keys(after), ['c_gen']);
  assert.equal(m.BROWSING_TTL_MS, 60_000);
  assert.deepEqual(Object.keys(m.dropBrowsing(b, 'k_rook')), ['c_gen']);
  assert.equal(m.dropBrowsing(b, 'nope'), b);
});

test('ownerOfTurn matches the server: team channel → team, channel → channel, DM or thread → the bot', () => {
  const channels = [teamCh, general];
  assert.equal(m.ownerOfTurn(running('t1', 'k_mira', 'c_team'), channels, teamOf), 't_shop');
  assert.equal(m.ownerOfTurn(running('t2', 'k_rook', 'c_gen'), channels, teamOf), 'c_gen');
  assert.equal(m.ownerOfTurn(running('t3', 'k_rook', 'k_rook'), channels, teamOf), 'k_rook');
  assert.equal(m.ownerOfTurn(running('t4', 'k_rook', 'th_42'), channels, teamOf), 'k_rook', 'a thread shares the bot’s browser');
});

test('ownerStillRunning: a running turn keeps the owner; turn end (or only queued turns) drops it', () => {
  const turns = [running('t1', 'k_mira', 'c_team'), { turn_id: 't2', kwant_id: 'k_rook', context_id: 'k_rook', state: 'queued' }];
  assert.equal(m.ownerStillRunning('t_shop', turns, [teamCh], teamOf), true);
  assert.equal(m.ownerStillRunning('t_shop', turns, [teamCh], teamOf, 't1'), false, 'its last turn ended');
  assert.equal(m.ownerStillRunning('k_rook', turns, [teamCh], teamOf), false, 'queued turns don’t browse');
});

test('Q1 (a)+(b): the owner row, plus every bot whose running turn is in that chat', () => {
  const b = m.noteBrowser({}, 't_shop', 'https://shop.example.com/', 10);
  const g = m.globeRows(b, [running('t1', 'k_mira', 'c_team'), running('t2', 'k_rook', 'k_rook')], [teamCh], teamOf);
  assert.deepEqual(Object.keys(g.owners), ['t_shop']);
  assert.deepEqual(Object.keys(g.bots), ['k_mira'], 'Rook works in its DM, whose browser is idle');
  assert.equal(g.bots.k_mira.owner, 't_shop', 'Mira’s globe opens the team chat, not her DM');
});

test('Q2 order: chats that need you first, then the most recent page; unknown owners are left out', () => {
  let b = m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 100);
  b = m.noteBrowser(b, 'c_gen', 'https://mail.example.com', 300);
  b = m.noteBrowser(b, 't_shop', 'https://git.example.com/acme/shop', 200);
  b = m.noteBrowser(b, 'gone', 'https://x.example.com', 400);
  const names = { k_rook: { kind: 'bot', label: 'Rook' }, c_gen: { kind: 'channel', label: 'general' }, t_shop: { kind: 'team', label: 'storefront' } };
  const e = m.switchEntries(b, (o) => names[o] ?? null, (o) => o === 'k_rook');
  assert.deepEqual(e.map((x) => x.owner), ['k_rook', 'c_gen', 't_shop']);
  assert.equal(e[0].needsYou, true);
  assert.equal(e[1].host, 'mail.example.com');
});

test('watchLabel reads "Watch Rook’s page: shop.example.com"', () => {
  assert.equal(m.watchLabel('Rook', 'shop.example.com'), 'Watch Rook’s page: shop.example.com');
  assert.equal(m.watchLabel('Rook', ''), 'Watch Rook’s page');
});

// ---------------------------------------------------------------- W2: the /live address

test('W2: /live round-trips for bots and channels; an older tail still falls back to the space', () => {
  assert.equal(m.spacePath('spc_1', { botId: 'k_rook', live: true }), '/s/spc_1/b/k_rook/live');
  assert.equal(m.spacePath('spc_1', { channelId: 'c_team', live: true }), '/s/spc_1/c/c_team/live');
  assert.equal(m.spacePath('spc_1', { botId: 'k_rook' }), '/s/spc_1/b/k_rook', 'closing the panel rewrites back to the chat');
  assert.deepEqual(m.parseSpacePath('/s/spc_1/b/k_rook/live'), { spaceId: 'spc_1', botId: 'k_rook', live: true });
  assert.deepEqual(m.parseSpacePath('/s/spc_1/c/c_team/live/'), { spaceId: 'spc_1', channelId: 'c_team', live: true });
  assert.deepEqual(m.parseSpacePath('/s/spc_1/b/k_rook'), { spaceId: 'spc_1', botId: 'k_rook' });
  assert.deepEqual(m.parseSpacePath('/s/spc_1/b/k_rook/elsewhere'), { spaceId: 'spc_1', legacy: true });
});

test('W2: the notification query takes &live=1 only with a chat', () => {
  assert.deepEqual(m.targetFromQuery('?space=spc_1&kwant=k_rook&live=1'), { spaceId: 'spc_1', channelId: null, botId: 'k_rook', live: true });
  assert.deepEqual(m.targetFromQuery('?space=spc_1&kwant=k_rook'), { spaceId: 'spc_1', channelId: null, botId: 'k_rook' });
  assert.deepEqual(m.targetFromQuery('?space=spc_1&live=1'), { spaceId: 'spc_1', channelId: null, botId: null });
});

// ---------------------------------------------------------------- W1: the globe

test('globe: 12 px violet Globe, labelled with the host, 24 px hit (44 on a phone)', () => {
  const desk = m.globe({ name: 'Rook', host: 'shop.example.com', onWatch() {} });
  assert.match(desk, /aria-label="Watch Rook’s page: shop.example.com"/);
  assert.match(desk, /title="Watch Rook’s page: shop.example.com"/);
  assert.match(desk, /h-6 w-6/);
  assert.match(desk, /kw-globe-in/);
  assert.match(desk, /text-\[#9d8fff\]/);
  assert.match(desk, /width="12"/);
  assert.match(m.globe({ name: 'Rook', host: 'x', phone: true, onWatch() {} }), /h-11 w-11/);
});

test('sidebar: globes on the DM bot row, the team row, a member bot row and a channel row', () => {
  const b = m.noteBrowser(m.noteBrowser(m.noteBrowser({}, 't_shop', 'https://shop.example.com', 1), 'k_rook', 'https://mail.example.com', 2), 'c_gen', 'https://git.example.com', 3);
  const html = m.sidebar({ kwants: [rook, mira], teams: [team], channels: [general], browsingOwners: b, browsingBots: { k_mira: b.t_shop }, onWatchBrowser() {} });
  const labels = [...html.matchAll(/data-watch-globe="" aria-label="([^"]+)"/g)].map((x) => x[1]);
  assert.ok(labels.includes('Watch Rook’s page: mail.example.com'), labels.join(' | '));
  assert.ok(labels.includes('Watch Mira’s page: shop.example.com'));
  assert.ok(labels.includes('Watch @storefront’s page: shop.example.com'));
  assert.ok(labels.includes('Watch #general’s page: git.example.com'));
  assert.doesNotMatch(html, /<button[^>]*>(?:(?!<\/button>)[\s\S])*<button/, 'never a button inside a button');
});

test('sidebar: no globe while the row needs you in the browser, and none without the handler', () => {
  const b = m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 1);
  const needs = m.sidebar({ kwants: [rook], browsingOwners: b, needsYou: { k_rook: 1 }, onWatchBrowser() {} });
  assert.doesNotMatch(needs, /data-watch-globe/);
  assert.match(text(needs), /Needs you in the browser/);
  assert.doesNotMatch(m.sidebar({ kwants: [rook], browsingOwners: b }), /data-watch-globe/);
  const teamNeeds = m.sidebar({ kwants: [rook], teams: [team], browsingOwners: m.noteBrowser({}, 't_shop', 'https://x.example.com', 1), browserNeedsYou: new Set(['t_shop']), onWatchBrowser() {} });
  assert.doesNotMatch(teamNeeds, /data-watch-globe/);
});

test('sidebar: clicking the globe opens the owner, not the row', () => {
  const b = m.noteBrowser({}, 't_shop', 'https://shop.example.com', 1);
  const watched = [];
  const opened = [];
  const v = m.mount('Sidebar', { kwants: [rook, mira], teams: [team], browsingOwners: b, browsingBots: { k_mira: b.t_shop }, onWatchBrowser: (o) => watched.push(o), onSelectKwant: (k) => opened.push(k.id), onSelectTeam: (t) => opened.push(t.id) });
  const g = [...v.host.querySelectorAll('[data-watch-globe]')].find((el) => el.getAttribute('aria-label').startsWith('Watch Mira'));
  assert.ok(g);
  g.click();
  assert.deepEqual(watched, ['t_shop']);
  assert.deepEqual(opened, [], 'the row underneath did not open');
  v.unmount();
});

// ---------------------------------------------------------------- W1: Watch on the working strip

test('working strip: Watch (Eye 13 px) while browsing, labelled for the bot; gone without it', () => {
  const html = m.strip({ active: true, name: 'Rook', title: 'coder', onWatch() {}, phone: false });
  assert.match(html, /data-watch-button=""/);
  assert.match(html, /aria-label="Watch the page Rook is on"/);
  assert.match(html, /textbtn/);
  assert.match(html, /width="13"/);
  assert.doesNotMatch(m.strip({ active: true, name: 'Rook', title: 'coder', phone: false }), /data-watch-button/);
  assert.match(m.strip({ active: true, name: 'Rook', title: 'coder', onWatch() {}, phone: true }), /min-h-\[44px\]/);
});

// ---------------------------------------------------------------- W3: the switcher

const entries = [
  { owner: 'k_rook', url: 'https://shop.example.com', host: 'shop.example.com', at: 2, kind: 'bot', label: 'Rook', sprite: 'coder', needsYou: false },
  { owner: 't_shop', url: 'https://git.example.com', host: 'git.example.com', at: 1, kind: 'team', label: 'storefront', needsYou: false },
];

test('switcher menu: menu button, role=menu rows with host and a check on the current one; picking opens it', () => {
  const picked = [];
  const v = m.mount('BrowserSwitcherMenu', { entries, current: 'k_rook', onPick: (o) => picked.push(o), title: '@Rook' });
  const btn = v.host.querySelector('[data-browser-switcher]');
  assert.equal(btn.getAttribute('aria-haspopup'), 'menu');
  assert.equal(btn.getAttribute('aria-expanded'), 'false');
  v.flush(() => btn.click());
  const menu = v.host.querySelector('[role="menu"]');
  assert.ok(menu);
  assert.match(menu.className, /min-w-\[240px\]/);
  assert.match(menu.className, /bg-\[#231d4b\]/);
  const rows = [...menu.querySelectorAll('[role="menuitemradio"]')];
  assert.equal(rows.length, 2);
  assert.equal(rows[0].getAttribute('aria-checked'), 'true');
  assert.match(rows[1].textContent, /@storefront/);
  assert.match(rows[1].textContent, /git\.example\.com/);
  assert.equal(document.activeElement, rows[0], 'focus lands on the current row');
  v.flush(() => menu.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true })));
  assert.equal(document.activeElement, rows[1]);
  v.flush(() => rows[1].click());
  assert.deepEqual(picked, ['t_shop']);
  assert.equal(v.host.querySelector('[role="menu"]'), null, 'the menu closes');
  v.unmount();
});

test('switcher menu: Esc closes and returns focus; during Take over it is disabled with "Hand back first"', () => {
  const v = m.mount('BrowserSwitcherMenu', { entries, current: 'k_rook', onPick() {}, title: '@Rook' });
  const btn = v.host.querySelector('[data-browser-switcher]');
  v.flush(() => btn.click());
  v.flush(() => v.host.querySelector('[role="menu"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  assert.equal(v.host.querySelector('[role="menu"]'), null);
  assert.equal(document.activeElement, btn);
  v.render({ entries, current: 'k_rook', onPick() {}, title: '@Rook', disabled: true });
  assert.equal(btn.getAttribute('title'), 'Hand back first');
  assert.equal(btn.getAttribute('aria-disabled'), 'true');
  v.flush(() => btn.click());
  assert.equal(v.host.querySelector('[role="menu"]'), null, 'no menu while you have the browser');
  v.unmount();
});

test('switcher phone row: 44 px avatars, a ring on the current one, "Rook, on shop.example.com"', () => {
  const html = m.row({ entries, current: 'k_rook', onPick() {} });
  assert.match(html, /aria-label="Rook, on shop.example.com"/);
  assert.match(html, /aria-label="@storefront, on git.example.com"/);
  assert.equal((html.match(/h-11 w-11/g) || []).length, 2);
  assert.equal((html.match(/ring-2 ring-\[#b8adff\]/g) || []).length, 1);
  const picked = [];
  const v = m.mount('BrowserSwitcherRow', { entries, current: 'k_rook', onPick: (o) => picked.push(o), disabled: true });
  v.host.querySelector('[data-switch-owner="t_shop"]').click();
  assert.deepEqual(picked, [], 'disabled during Take over');
  v.render({ entries, current: 'k_rook', onPick: (o) => picked.push(o) });
  v.host.querySelector('[data-switch-owner="t_shop"]').click();
  v.host.querySelector('[data-switch-owner="k_rook"]').click();
  assert.deepEqual(picked, ['t_shop'], 'the current one does nothing');
  v.unmount();
});
