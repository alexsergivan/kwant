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
  import { BrowserSwitcherMenu, BrowserSwitcherRow, GlobeSlot, LiveMissPage, WatchGlobe } from './src/components/WatchLive';
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
    const comp = { Sidebar, WorkingStrip, BrowserSwitcherMenu, BrowserSwitcherRow, GlobeSlot, LiveMissPage }[C];
    const render = (q) => flushSync(() => root.render(React.createElement(comp, C === 'Sidebar' ? sidebarProps(q) : q)));
    render(p);
    return { host, render, flush: (fn) => flushSync(fn), unmount: () => { root.unmount(); host.remove(); } };
  };
`);

const React_p = (t) => ({ $$typeof: Symbol.for('react.element'), type: 'p', key: null, ref: null, props: { children: t }, _owner: null });
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

test('globe: 12 px violet Globe, labelled with the host; a 12 px box whose hit area overflows to 24 px (44 on a phone)', () => {
  const desk = m.globe({ name: 'Rook', host: 'shop.example.com', onWatch() {} });
  assert.match(desk, /aria-label="Watch Rook’s page: shop.example.com"/);
  assert.match(desk, /title="Watch Rook’s page: shop.example.com"/);
  assert.match(desk, /\bh-3 w-3\b/, 'the box is the icon: the row keeps its height (Mira 1)');
  assert.match(desk, /after:-inset-1\.5/, '12 + 2×6 = 24 px hit area');
  assert.match(desk, /kw-globe-in/);
  assert.match(desk, /text-\[#9d8fff\]/);
  assert.match(desk, /width="12"/);
  const phone = m.globe({ name: 'Rook', host: 'x', phone: true, onWatch() {} });
  assert.match(phone, /after:-inset-4/, '12 + 2×16 = 44 px hit area');
  assert.doesNotMatch(phone, /h-11|-my-/, 'no box that makes the row taller');
  assert.match(phone, /data-compact=""/, 'out of the global 44 px touch min-height');
});

test('sidebar: globes on the DM bot row, the team row, a member bot row and a channel row', () => {
  const b = m.noteBrowser(m.noteBrowser(m.noteBrowser({}, 't_shop', 'https://shop.example.com', 1), 'k_rook', 'https://mail.example.com', 2), 'c_gen', 'https://git.example.com', 3);
  const html = m.sidebar({ kwants: [rook, mira], teams: [team], channels: [general], browsingOwners: b, browsingBots: { k_mira: b.t_shop }, onWatchBrowser() {} });
  const labels = [...html.matchAll(/data-watch-globe=""[^>]*?aria-label="([^"]+)"/g)].map((x) => x[1]);
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
  // Phone: muted and as small as the line (28 px), tapping like 44 px through kws-hit.
  const phone = m.strip({ active: true, name: 'Rook', title: 'coder', onWatch() {}, phone: true });
  const watch = /<button[^>]*data-watch-button=""[^>]*>/.exec(phone)[0];
  for (const c of ['data-compact', 'kws-hit', ' -mr-2 ', 'h-7', 'text-xs', 'text-zinc-400']) assert.ok(watch.includes(c), c);
  assert.doesNotMatch(watch, /text-\[#b8adff\]|min-h-11/);
  assert.match(phone, /width="12"/);
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

// ---------------------------------------------------------------- 0.7.7 review round (Mira 1–6, QA F1–F6)

test('Mira 2: the bot row globe is pushed to the end of the name line, like team and channel rows', () => {
  const b = m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 1);
  const html = m.sidebar({ kwants: [rook], browsingOwners: b, onWatchBrowser() {} });
  assert.match(html, /data-watch-globe=""[^>]*class="[^"]*\bml-auto\b/);
});

test('Mira 3: every row on a chat whose browser holds an open step hides its globe (bot rows too)', () => {
  const b = m.noteBrowser({}, 't_shop', 'https://shop.example.com', 1);
  const html = m.sidebar({ kwants: [rook, mira], teams: [team], browsingOwners: b, browsingBots: { k_mira: b.t_shop }, browserNeedsYou: new Set(['t_shop']), onWatchBrowser() {} });
  assert.doesNotMatch(html, /data-watch-globe/, 'neither @storefront nor Mira (running in it) shows one');
  const other = m.sidebar({ kwants: [rook, mira], teams: [team], browsingOwners: b, browsingBots: { k_mira: b.t_shop }, browserNeedsYou: new Set(['c_gen']), onWatchBrowser() {} });
  assert.equal((other.match(/data-watch-globe/g) || []).length, 2, 'another chat’s step changes nothing here');
});

test('Mira 4: the menu hangs 4 px under the whole two-line title block, with the sub line inside it', () => {
  const v = m.mount('BrowserSwitcherMenu', { entries, current: 'k_rook', onPick() {}, title: '@Rook', sub: React_p('Live Chromium') });
  const block = v.host.querySelector('[data-switcher-block]');
  assert.match(block.textContent, /Live Chromium/);
  v.flush(() => v.host.querySelector('[data-browser-switcher]').click());
  const menu = v.host.querySelector('[role="menu"]');
  assert.equal(menu.parentElement, block);
  assert.match(menu.className, /top-\[calc\(100%\+4px\)\]/);
  assert.doesNotMatch(menu.className, /\bmt-/);
  v.unmount();
});

test('Mira 5: a chat that needs you says so: "Needs you" in amber on desk, an amber dot on the phone', () => {
  const needy = [{ ...entries[1], needsYou: true }, entries[0]];
  const v = m.mount('BrowserSwitcherMenu', { entries: needy, current: 'k_rook', onPick() {}, title: '@Rook' });
  v.flush(() => v.host.querySelector('[data-browser-switcher]').click());
  const rows = [...v.host.querySelectorAll('[role="menuitemradio"]')];
  const tag = rows[0].querySelector('[data-switch-needs-you]');
  assert.equal(tag.textContent, 'Needs you');
  assert.match(tag.className, /text-\[11px\]/);
  assert.match(tag.className, /text-\[#fcd34d\]/);
  assert.doesNotMatch(rows[0].textContent, /git\.example\.com/, 'it replaces the host');
  assert.match(rows[1].textContent, /shop\.example\.com/);
  v.unmount();
  const html = m.row({ entries: needy, current: 'k_rook', onPick() {} });
  assert.match(html, /aria-label="@storefront, on git.example.com, needs you"/);
  assert.match(html, /data-switch-needs-you=""[^>]*class="[^"]*h-2 w-2[^"]*bg-\[#fcd34d\][^"]*ring-2 ring-\[#1c1838\]/);
  assert.equal((html.match(/data-switch-needs-you/g) || []).length, 1);
});

test('Mira 6: the disabled phone row says "Hand back first to switch", tied to each avatar by aria-describedby', () => {
  const v = m.mount('BrowserSwitcherRow', { entries, current: 'k_rook', onPick() {}, disabled: true });
  const hint = v.host.querySelector('[data-switch-hint]');
  assert.equal(hint.textContent, 'Hand back first to switch');
  assert.match(hint.className, /text-\[11px\]/);
  assert.match(hint.className, /text-\[#9a94c4\]/);
  for (const b of v.host.querySelectorAll('[data-switch-owner]')) {
    assert.equal(b.getAttribute('aria-disabled'), 'true');
    assert.equal(b.getAttribute('aria-describedby'), hint.id);
  }
  v.render({ entries, current: 'k_rook', onPick() {} });
  assert.equal(v.host.querySelector('[data-switch-hint]'), null);
  assert.equal(v.host.querySelector('[data-switch-owner]').getAttribute('aria-describedby'), null);
  v.unmount();
});

test('nice-to-have: "@Rook" in the menu as in the header; a 10 px rounded ring with offset 2 on the phone', () => {
  const v = m.mount('BrowserSwitcherMenu', { entries, current: 'k_rook', onPick() {}, title: '@Rook' });
  v.flush(() => v.host.querySelector('[data-browser-switcher]').click());
  assert.match(v.host.querySelector('[role="menuitemradio"]').textContent, /^@Rook/);
  v.unmount();
  const html = m.row({ entries, current: 'k_rook', onPick() {} });
  assert.match(html, /rounded-\[10px\] ring-2 ring-\[#b8adff\] ring-offset-2/);
});

test('F2: the globe fades out over 150 ms (unclickable meanwhile), then goes', async () => {
  const b = m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 1);
  const watched = [];
  const v = m.mount('GlobeSlot', { browsing: b.k_rook, name: 'Rook', onWatch: (o) => watched.push(o) });
  assert.match(v.host.querySelector('[data-watch-globe]').className, /kw-globe-in/);
  v.render({ browsing: undefined, name: 'Rook', onWatch: (o) => watched.push(o) });
  const g = v.host.querySelector('[data-watch-globe]');
  assert.ok(g, 'still there while it fades');
  assert.match(g.className, /kw-globe-out/);
  assert.equal(g.getAttribute('tabindex'), '-1');
  g.click();
  assert.deepEqual(watched, []);
  await new Promise((r) => setTimeout(r, 200));
  assert.equal(v.host.querySelector('[data-watch-globe]'), null);
  v.unmount();
});

test('F1 parse: a broken live link is still a live link, to no chat', () => {
  assert.deepEqual(m.parseSpacePath('/s/spc_1/b//live'), { spaceId: 'spc_1', live: true });
  assert.deepEqual(m.parseSpacePath('/s/spc_1/b/k_rook/live/x'), { spaceId: 'spc_1', live: true });
  assert.deepEqual(m.parseSpacePath('/s/spc_1/c//live'), { spaceId: 'spc_1', live: true });
  assert.deepEqual(m.parseSpacePath('/s/spc_1/b/nope/live'), { spaceId: 'spc_1', botId: 'nope', live: true });
});

test('F1 page: "This bot isn’t browsing now" with a button to the chat, focused', () => {
  let opened = 0;
  const v = m.mount('LiveMissPage', { chatLabel: '@Rook', onOpenChat: () => opened++ });
  assert.match(v.host.textContent, /This bot isn’t browsing now/);
  const btn = [...v.host.querySelectorAll('button')].find((b) => b.textContent === 'Open @Rook');
  assert.ok(btn);
  assert.equal(document.activeElement, btn);
  btn.click();
  assert.equal(opened, 1);
  v.unmount();
});

test('F1 seed: a page opened mid-turn counts a running turn whose newest tool is a browser tool', () => {
  const steps = (...tools) => tools.map((tool) => ({ kind: 'tool', tool, detail: tool.endsWith('open') ? 'https://shop.example.com/cart' : undefined }));
  const turns = [
    { ...running('t1', 'k_rook', 'c_team'), steps: steps('Bash', 'mcp__kwnt__browser_open') },
    { ...running('t2', 'k_mira', undefined), steps: steps('mcp__kwnt__browser_open', 'Bash') },
  ];
  const out = m.seedBrowsing({}, turns, [teamCh], teamOf, 5);
  assert.deepEqual(Object.keys(out), ['t_shop']);
  assert.equal(out.t_shop.host, 'shop.example.com');
  assert.equal(m.isBrowserTool('browser_navigate'), true);
  assert.equal(m.isBrowserTool('mcp__other__browser_open'), true);
  assert.equal(m.isBrowserTool('my_browser'), false);
  const known = m.noteBrowser({}, 't_shop', 'https://other.example.com', 9);
  assert.equal(m.seedBrowsing(known, turns, [teamCh], teamOf, 10), known, 'a live event wins over the seed');
});

test('F6: an event right after turn_started survives the prune; an ended turn’s owner goes', () => {
  // The old code pruned with a key from the render before the event: that key had no owner yet.
  let map = {};
  const turns = [running('t1', 'k_rook', 'c_team')];
  map = m.noteBrowser(map, 't_shop', 'https://shop.example.com', 1);
  assert.equal(m.pruneBrowsing(map, turns, [teamCh], teamOf), map, 'kept: its turn runs');
  assert.deepEqual(m.pruneBrowsing(map, [], [teamCh], teamOf), {}, 'gone once no turn drives it');
});

// ---------------------------------------------------------------- 0.7.7 nits (Mira recheck N1–N4)

test('N1: the team row globe has no side margin, so it lines up with channel and bot globes', () => {
  const b = m.noteBrowser({}, 't_shop', 'https://shop.example.com', 1);
  const html = m.sidebar({ kwants: [rook], teams: [team], browsingOwners: b, onWatchBrowser() {} });
  assert.match(html, /data-watch-globe/, 'the team row shows a globe');
  assert.doesNotMatch(html, /\bmx-1\.5\b/);
});

test('N1 recheck: the team row globe comes last, after the member count and team settings, like the bot rows', () => {
  const b = m.noteBrowser({}, 't_shop', 'https://shop.example.com', 1);
  for (const p of [{}, { phone: true, section: 'chats' }]) {
    const v = m.mount('Sidebar', { kwants: [rook, mira], teams: [team], browsingOwners: b, onWatchBrowser() {}, onManageTeam() {}, ...p });
    const row = v.host.querySelector('[data-team-row="t_shop"]');
    const g = row.querySelector('[data-watch-globe]');
    assert.ok(g, JSON.stringify(p));
    assert.equal(g.parentElement.lastElementChild, g, 'nothing after the globe on its row');
    assert.ok(g.previousElementSibling?.matches('button[aria-label^="Team settings"]'), 'team settings sits before it');
    assert.doesNotMatch(g.className, /\bml-auto\b/);
    v.unmount();
  }
});

test('F2 phone: a bot whose turn ends stays in Working while its globe fades, then moves to Direct chats', async () => {
  const b = m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 1);
  const base = { kwants: [rook, mira], phone: true, section: 'chats', onWatchBrowser() {} };
  const v = m.mount('Sidebar', { ...base, browsingOwners: b, busyKwants: { k_rook: 'Working on the direct chat' } });
  const sec = () => v.host.querySelector('[data-bot-row="k_rook"]')?.closest('[data-chats-section]')?.getAttribute('data-chats-section');
  assert.equal(sec(), 'working');
  const before = v.host.querySelector('[data-bot-row="k_rook"]');
  v.render({ ...base, browsingOwners: {}, busyKwants: {} });
  assert.equal(sec(), 'working', 'kept in Working for the fade');
  assert.equal(v.host.querySelector('[data-bot-row="k_rook"]'), before, 'the same row: not unmounted and remounted');
  const g = v.host.querySelector('[data-watch-globe]');
  assert.ok(g && g.getAttribute('data-leaving') === 'true' && /kw-globe-out/.test(g.className), 'the globe is fading out');
  await new Promise((r) => setTimeout(r, 220));
  assert.equal(sec(), 'direct', 'then under Direct chats');
  assert.equal(v.host.querySelector('[data-watch-globe]'), null);
  v.unmount();
});

test('F2 phone: under reduced motion the row moves at once, with no fade', () => {
  const was = window.matchMedia;
  window.matchMedia = (q) => ({ matches: /reduce/.test(q), media: q, addEventListener() {}, removeEventListener() {} });
  try {
    const base = { kwants: [rook, mira], phone: true, section: 'chats', onWatchBrowser() {} };
    const v = m.mount('Sidebar', { ...base, browsingOwners: m.noteBrowser({}, 'k_rook', 'https://shop.example.com', 1), busyKwants: { k_rook: 'Working' } });
    v.render({ ...base, browsingOwners: {}, busyKwants: {} });
    assert.equal(v.host.querySelector('[data-bot-row="k_rook"]').closest('[data-chats-section]').getAttribute('data-chats-section'), 'direct');
    assert.equal(v.host.querySelector('[data-watch-globe]'), null);
    v.unmount();
  } finally {
    window.matchMedia = was;
  }
});

test('N2 /b/: a bot link keeps "This bot isn’t browsing now"', () => {
  const v = m.mount('LiveMissPage', { chatLabel: '@Rook', kind: 'bot', onOpenChat() {} });
  assert.match(v.host.textContent, /This bot isn’t browsing now/);
  assert.match(v.host.textContent, /When it opens a page, this link shows it live\./);
  v.unmount();
});

test('N2 /c/: a team or channel link says "No bot is browsing here now"', () => {
  const v = m.mount('LiveMissPage', { chatLabel: '#general', kind: 'place', onOpenChat() {} });
  assert.match(v.host.textContent, /No bot is browsing here now/);
  assert.match(v.host.textContent, /When a bot opens a page here, this link shows it live\./);
  assert.doesNotMatch(v.host.textContent, /This bot/);
  assert.ok([...v.host.querySelectorAll('button')].some((b) => b.textContent === 'Open #general'));
  v.unmount();
});

test('N3: with no name the button says "Open the chat", never "Open @"', () => {
  for (const chatLabel of ['', '@', '#']) {
    const v = m.mount('LiveMissPage', { chatLabel, onOpenChat() {} });
    const texts = [...v.host.querySelectorAll('button')].map((b) => b.textContent);
    assert.ok(texts.includes('Open the chat'), `label ${JSON.stringify(chatLabel)}`);
    assert.ok(!texts.some((t) => /^Open [@#]?$/.test(t)));
    v.unmount();
  }
});

test('N4: on a phone with Watch showing, the time hides below 400 px so the host fits', () => {
  const timer = (html) => html.match(/<span class="([^"]*)"[^>]*data-working-elapsed=""/)[1];
  const withWatch = timer(m.strip({ active: true, name: 'Rook', title: 'coder', onWatch() {}, phone: true }));
  assert.match(withWatch, /\bhidden\b/);
  assert.match(withWatch, /min-\[400px\]:inline/);
  const noWatch = timer(m.strip({ active: true, name: 'Rook', title: 'coder', phone: true }));
  assert.doesNotMatch(noWatch, /\bhidden\b/, 'without Watch the time stays');
  const desk = timer(m.strip({ active: true, name: 'Rook', title: 'coder', onWatch() {}, phone: false }));
  assert.doesNotMatch(desk, /\bhidden\b/, 'desk unchanged');
});
