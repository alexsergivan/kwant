// PR 1 of the 0.6.9 UI refresh: bot home helpers, avatar moods, tab signal, stickers, "New" tags.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const store = new Map();
globalThis.localStorage = { getItem: (k) => (store.has(k) ? store.get(k) : null), setItem: (k, v) => void store.set(k, String(v)), removeItem: (k) => void store.delete(k) };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { Sticker } from './src/components/Sticker';
  import { AvatarMood } from './src/components/AvatarMood';
  import { ModernAvatar } from './src/components/ModernAvatar';
  import { MentionPopover } from './src/components/MentionPopover';
  import { BotHome } from './src/components/BotHome';
  export * from './src/botHome';
  export * from './src/kwantMood';
  export * from './src/tabSignal';
  export * from './src/whatsNew';
  export { clampTilt } from './src/components/Sticker';
  export { turnStarted, turnEnded, clearAllTurns } from './src/kwantActivity';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export const sticker = (p, kids = 'Verified') => r(Sticker, p, kids);
  export const mood = (p) => r(AvatarMood, p);
  export const avatar = (p) => r(ModernAvatar, p);
  export const mentions = (p) => r(MentionPopover, p);
  export const botHome = (p) => r(BotHome, p);
`);

// ---- bot home helpers
test('goodAt: the bot\'s own phrases, else the parts of its role; four at most, short, no repeats', () => {
  assert.deepEqual(m.goodAt({ title: 'Nutrition & Diet Advisor' }), ['Nutrition', 'Diet Advisor']);
  assert.deepEqual(m.goodAt({ title: 'x', covers: ['meal plans', 'Meal plans', 'macros'] }), ['meal plans', 'macros']);
  assert.equal(m.goodAt({ title: 'a, b, c, d, e, f' }).length, 4);
  assert.ok(m.goodAt({ title: 'x'.repeat(80) })[0].length <= 32);
});

test('recentWork: newest two chats that are not archived or delegations; undefined until loaded', () => {
  assert.equal(m.recentWork(undefined), undefined);
  const t = (id, at, extra = {}) => ({ id, title: id, updated_at: at, ...extra });
  const out = m.recentWork([t('a', '2026-11-01'), t('b', '2026-11-03'), t('c', '2026-11-02', { archived: true }), t('d', '2026-11-02', { kind: 'delegation' }), t('e', '2026-11-02')]);
  assert.deepEqual(out.map((x) => x.id), ['b', 'e']);
});

test('recipientsOf: the bot whose chat it is plus every @mentioned bot, whole names only', () => {
  const bots = [{ id: '1', name: 'Dev' }, { id: '2', name: 'Dev Ops' }, { id: '3', name: 'Ana.lyst' }];
  assert.deepEqual(m.recipientsOf('hello', bots, '1'), ['1']);
  assert.deepEqual(m.recipientsOf('hi @dev please', bots), ['1']);
  assert.deepEqual(m.recipientsOf('@Dev Ops and @Ana.lyst', bots).sort(), ['1', '2', '3']);
  assert.deepEqual(m.recipientsOf('mail me@dev.com', bots), [], 'an email address is not a mention');
  assert.deepEqual(m.recipientsOf('@Developer', bots), [], 'only whole names');
});

// ---- mood selection
const now = 1_000_000;
const base = { working: false, now };
test('selectMood: needs you beats everything; a running turn beats resting; thinking is a running turn with no output yet', () => {
  assert.equal(m.selectMood({ ...base, working: true, needsYou: true }), 'needs_you');
  assert.equal(m.selectMood({ ...base, working: true, thinking: true }), 'thinking');
  assert.equal(m.selectMood({ ...base, working: true }), 'working');
  const later = new Date(now + 60_000);
  assert.equal(m.selectMood({ ...base, working: true, restingUntil: later }), 'working');
  assert.equal(m.selectMood({ ...base, restingUntil: later }), 'resting');
  assert.equal(m.selectMood({ ...base, restingUntil: new Date(now - 1) }), 'idle', 'a limit that has passed is gone');
});
test('selectMood: finished fades after a moment, failed stays until the next turn or ten minutes, cancelled shows nothing', () => {
  assert.equal(m.selectMood({ ...base, last: { outcome: 'completed', at: now - 1000 } }), 'finished');
  assert.equal(m.selectMood({ ...base, last: { outcome: 'completed', at: now - m.FINISHED_MS - 1 } }), 'idle');
  assert.equal(m.selectMood({ ...base, last: { outcome: 'failed', at: now - 5 * 60_000 } }), 'failed');
  assert.equal(m.selectMood({ ...base, last: { outcome: 'failed', at: now - m.FAILED_MS - 1 } }), 'idle');
  assert.equal(m.selectMood({ ...base, last: { outcome: 'cancelled', at: now } }), 'idle');
  assert.equal(m.selectMood({ ...base, working: true, last: { outcome: 'failed', at: now } }), 'working');
});
test('moodLabel: words for the tooltip, "Resting until 03:00" for quota', () => {
  const at = new Date(2026, 10, 3, 3, 0);
  assert.equal(m.moodLabel('resting', 'Dev', at, new Date(2026, 10, 3, 1, 0).getTime()), 'Resting until 03:00');
  assert.equal(m.moodLabel('needs_you', 'Dev'), 'Dev needs you');
  assert.equal(m.moodLabel('failed', 'Dev'), "Dev's last turn failed");
  assert.equal(m.moodLabel('idle', 'Dev'), null);
  assert.equal(m.moodLabel('working', 'Dev'), null, "working keeps the avatar's own words");
});
test('restingUntil: a family rests when every switched-on account of it is out; auto only when all are', () => {
  const t = (mins) => new Date(Date.now() + mins * 60_000).toISOString();
  const p = (engine, limited_until, enabled = true) => ({ engine, limited_until, enabled });
  const profiles = [p('claude', t(30)), p('claude', t(90)), p('codex', null)];
  assert.ok(m.restingUntil('claude', profiles), 'claude is out');
  const until = m.restingUntil('claude', profiles);
  assert.ok(Math.abs(until.getTime() - Date.parse(t(30))) < 5000, 'the earliest return wins');
  assert.equal(m.restingUntil('codex', profiles), null);
  assert.equal(m.restingUntil('auto', profiles), null, 'codex can still run');
  assert.equal(m.restingUntil('auto', [p('claude', t(30)), p('codex', t(60))]).getTime() > Date.now(), true);
  assert.equal(m.restingUntil('claude', [p('claude', t(30), false)]), null, 'a switched-off account is not a limit');
  assert.equal(m.restingUntil('claude', undefined), null);
});

// ---- the store and the component
test('mood store: thinking until output, finished then fades, failed sticks; noticed bumps the blink counter', () => {
  m.clearMoods();
  m.clearAllTurns();
  m.turnStarted('k1', 't1');
  m.moodTurnStarted('k1');
  assert.equal(m.moodOf('k1'), 'thinking');
  m.moodOutput('k1');
  assert.equal(m.moodOf('k1'), 'working');
  m.turnEnded('k1', 't1');
  m.moodTurnEnded('k1', 'completed');
  assert.equal(m.moodOf('k1'), 'finished');
  m.moodTurnStarted('k1');
  m.moodTurnEnded('k1', 'failed');
  assert.equal(m.moodOf('k1'), 'failed');
  m.setMoodNeedsYou(['k1']);
  assert.equal(m.moodOf('k1'), 'needs_you');
  m.setMoodNeedsYou([]);
  m.setMoodResting({ k1: new Date(Date.now() + 3_600_000) });
  assert.equal(m.moodOf('k1'), 'resting');
  assert.equal(m.noticesOf('k1'), 0);
  m.moodNoticed('k1');
  assert.equal(m.noticesOf('k1'), 1);
  m.clearMoods();
});
test('AvatarMood draws a mark per mood, nothing for working or idle', () => {
  for (const mood of ['thinking', 'finished', 'failed', 'needs_you', 'resting']) assert.match(m.mood({ mood, size: 32 }), new RegExp(`data-mood="${mood}"`), mood);
  assert.equal(m.mood({ mood: 'working', size: 32 }), '');
  assert.equal(m.mood({ mood: 'idle', size: 32 }), '');
});
test('ModernAvatar with a kwantId carries the mood and its words', () => {
  m.clearMoods();
  m.setMoodNeedsYou(['kx']);
  const html = m.avatar({ role: 'qa', name: 'Dev', size: 32, kwantId: 'kx' });
  assert.match(html, /data-mood="needs_you"/);
  assert.match(html, /title="Dev needs you"/);
  m.setMoodNeedsYou([]);
  m.setMoodResting({ kx: new Date(Date.now() + 3_600_000) });
  assert.match(m.avatar({ role: 'qa', name: 'Dev', size: 32, kwantId: 'kx' }), /kw-avatar-resting/);
  m.clearMoods();
  assert.doesNotMatch(m.avatar({ role: 'qa', name: 'Dev', size: 32, kwantId: 'kx' }), /data-mood/);
});

// ---- tab signal
test('tabTitle: "(N) kwant" for N things that need you, the plain title for none, a page title kept', () => {
  assert.equal(m.tabTitle('kwant', 2), '(2) kwant');
  assert.equal(m.tabTitle('(2) kwant', 0), 'kwant');
  assert.equal(m.tabTitle('(2) kwant', 3), '(3) kwant');
  assert.equal(m.tabTitle('Sandbox · kwant bots browser', 1), '(1) Sandbox · kwant bots browser');
  assert.equal(m.tabTitle('kwant', 120), '(99+) kwant');
  assert.equal(m.tabTitle('', 1), '(1) kwant');
});
test('needsYouCount: bots waiting on you plus approvals', () => {
  assert.equal(m.needsYouCount({ a: 'needs_you', b: 'working', c: 'needs_you' }, 1), 3);
  assert.equal(m.needsYouCount({}, 0), 0);
  assert.equal(m.needsYouCount({ a: 'available' }, -2), 0);
});
test('tab signal: title follows the count; the dotted icon is set while working and restored when idle', async () => {
  const link = { href: '/icon.svg', type: 'image/svg+xml' };
  const doc = { title: 'kwant bots', iconLink: () => link, drawDotted: async () => 'data:image/png;base64,AAA' };
  const sig = m.createTabSignal(doc);
  sig.setNeedsYou(2);
  assert.equal(doc.title, '(2) kwant bots');
  sig.setNeedsYou(0);
  assert.equal(doc.title, 'kwant bots');
  await sig.setWorking(true);
  assert.equal(link.href, 'data:image/png;base64,AAA');
  await sig.setWorking(false);
  assert.equal(link.href, '/icon.svg');
  assert.equal(link.type, 'image/svg+xml');
  // Idle again before the image finished drawing: the icon is left alone.
  let release;
  const slow = { title: 't', iconLink: () => link, drawDotted: () => new Promise((r) => (release = r)) };
  const sig2 = m.createTabSignal(slow);
  const pending = sig2.setWorking(true);
  await sig2.setWorking(false);
  release('data:image/png;base64,BBB');
  await pending;
  assert.equal(link.href, '/icon.svg');
});
test('dotGeometry sits in the bottom-right corner inside the icon', () => {
  const g = m.dotGeometry(64);
  assert.ok(g.cx + g.r + g.ring <= 64 + 1 && g.cy + g.r + g.ring <= 64 + 1 && g.cx > 32 && g.cy > 32);
});

// ---- stickers and "New" tags
test('Sticker: tone, clamped tilt, button only when clickable', () => {
  assert.match(m.sticker({ tone: 'holo' }), /kw-sticker-holo/);
  assert.match(m.sticker({ tone: 'holo' }), /--kw-tilt:-2deg/);
  assert.match(m.sticker({}), /kw-sticker-ink/);
  assert.match(m.sticker({ tilt: 9 }), /--kw-tilt:3deg/);
  assert.match(m.sticker({ tilt: -9 }), /--kw-tilt:-3deg/);
  assert.equal(m.clampTilt(Number.NaN, 1), 1);
  assert.match(m.sticker({}), /^<span/);
  assert.match(m.sticker({ onClick() {} }), /^<button type="button"/);
});
test('what\'s new: shown until used once, then gone for good; no storage shows nothing', () => {
  const mem = new Map();
  const s = { getItem: (k) => (mem.has(k) ? mem.get(k) : null), setItem: (k, v) => void mem.set(k, v) };
  assert.equal(m.isNewThing('mention', s), true);
  m.markNewSeen('mention', s);
  assert.equal(m.isNewThing('mention', s), false);
  assert.equal(m.isNewThing('starters', s), true, 'each thing is tracked on its own');
  assert.equal(m.isNewThing('mention', null), false);
  assert.match(m.newKey('mention'), /0\.6\.9/);
});

// ---- mention picker and bot home
test('mention picker: avatar, name and a one-line "good at", options a screen reader can follow', () => {
  const html = m.mentions({ selectedIndex: 1, onSelect() {}, items: [
    { type: 'kwant', id: 'a', name: 'Dev', title: 'Engineer', goodAt: ['code review', 'refactors'], avatar_sprite: 'tech_lead' },
    { type: 'kwant', id: 'b', name: 'Ana', title: 'Analyst' },
  ] });
  assert.match(html, /role="listbox"/);
  assert.equal((html.match(/role="option"/g) ?? []).length, 2);
  assert.match(html, /aria-selected="true"/);
  assert.match(html, /Good at code review · refactors/);
  assert.match(html, />Analyst</, 'falls back to the role when there is no good-at');
  assert.match(html, /kw-avatar/);
});
test('bot home: name, role, good-at chips and starter cards', () => {
  const html = m.botHome({ home: { kind: 'bot', bot: { id: 'n', name: 'Nora', title: 'Nutrition & Diet Advisor', avatar_sprite: 'chef' }, recent: [] }, spaceName: 'Home', team: [], prompts: ['plan my week', 'count macros'], canRun: true, onPick() {} });
  assert.match(html, /Nora/);
  assert.match(html, /Nutrition/);
  assert.match(html, /Plan my week/);
});
