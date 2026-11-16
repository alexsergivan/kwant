// Times: one clock everywhere (roadmap 22c, autonomy/times-one-clock-everywhere/brief.md). The last spots that
// built their own time strings now go through src/clock.ts: the bot computer's terminal log, Import history, a
// verification's time and a goal's check-square tooltip. The Pushed meta line keeps its branch whole. Each site is
// pinned in en-US (12 h) and en-GB (24 h) by passing the locale; the app passes none (the device's own).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

process.env.TZ = 'Asia/Kolkata'; // a device that is not on UTC, so a UTC slice would show
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export { stampClock, secondsText, dateClock } from './src/clock';
  export { terminalTime } from './src/components/VirtualMachinePane';
  export { RecentImports } from './src/components/ImportDialog';
  export { Verification, formatWhen } from './src/components/results/Verification';
  export { GoalHeartbeat, beatWhen } from './src/components/Signature';
  export { parsePrepush, storyOf } from './src/dev/prepush';
  export * as fx from './src/dev/prepushFixtures';
  export { PrepushCard } from './src/components/dev/PrepushCard';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);
const h = m.createElement;
const html = (C, props) => m.renderToStaticMarkup(h(C, props));

const AT = '2026-11-16T14:35:12.345Z'; // 8:05:12 PM in Kolkata; the old code showed "14:35:12" (UTC)
const NOW = Date.parse('2026-11-17T09:00:00Z');
const LAST_YEAR = '2025-03-04T14:35:00Z';

test('22c stampClock: always the day, the year only when not this year, in en-US and en-GB', () => {
  assert.deepEqual([m.stampClock(AT, NOW, 'en-US'), m.stampClock(AT, NOW, 'en-GB')], ['Nov 16, 8:05\u00a0PM', '16 Nov, 20:05']);
  assert.deepEqual([m.stampClock(AT, Date.parse(AT), 'en-US'), m.stampClock(AT, Date.parse(AT), 'en-GB')], ['Nov 16, 8:05\u00a0PM', '16 Nov, 20:05'], 'today keeps the day too');
  assert.deepEqual([m.stampClock(LAST_YEAR, NOW, 'en-US'), m.stampClock(LAST_YEAR, NOW, 'en-GB')], ['Mar 4, 2025, 8:05\u00a0PM', '4 Mar 2025, 20:05']);
  assert.equal(m.stampClock('nope', NOW), null);
  assert.equal(m.stampClock(null, NOW), null);
});

test('22c terminal log: the device clock with seconds, not the UTC slice of the ISO string', () => {
  assert.deepEqual([m.terminalTime(AT, 'en-US'), m.terminalTime(AT, 'en-GB')], ['8:05:12\u00a0PM', '20:05:12']);
  assert.notEqual(m.terminalTime(AT, 'en-GB'), '14:35:12', 'not UTC');
  assert.equal(m.terminalTime(AT), m.secondsText(AT), 'the app passes no locale: the device default');
  assert.equal(m.terminalTime('just now'), 'just now', 'an unreadable timestamp shows as it came');
  const src = readFileSync(new URL('../src/components/VirtualMachinePane.tsx', import.meta.url), 'utf8');
  assert.match(src, /className="shrink-0 whitespace-nowrap[^"]*" data-terminal-time/, 'the time never wraps or shrinks');
  assert.match(src, /className="min-w-0 break-words">\$ \{log\.command\}/, 'a long command wraps beside it');
  assert.doesNotMatch(src, /split\('T'\)\[1\]/);
});

test('22c Import history: the row reads the one clock, with " · undone" kept', () => {
  const rec = { id: 'imp_1', created_at: AT, status: 'undone', created: { kwants: [{ id: 'k', name: 'Inbox helper' }] } };
  const out = html(m.RecentImports, { records: [rec], onUndo: () => {}, busyId: null });
  assert.ok(out.includes(`${m.stampClock(AT)} · undone`), out);
  const bad = html(m.RecentImports, { records: [{ ...rec, created_at: 'soon', status: 'applied' }], onUndo: () => {}, busyId: null });
  assert.ok(bad.includes('soon'), 'an unreadable time shows as it came');
  assert.doesNotMatch(bad, /Invalid Date/);
});

test('22c Verification: a check time in en-US and en-GB; an unreadable one shows as it came', () => {
  assert.deepEqual([m.formatWhen(AT, NOW, 'en-US'), m.formatWhen(AT, NOW, 'en-GB')], ['Nov 16, 8:05\u00a0PM', '16 Nov, 20:05']);
  assert.equal(m.formatWhen('later', NOW), 'later');
  assert.equal(m.formatWhen(null, NOW), null);
  const block = {
    id: 'v1', kind: 'verification', summary: '1 of 1 passed', target: { title: 'Deploy' },
    checks: [{ id: 'c1', label: 'Site loads', status: 'passed', checked_at: AT }],
  };
  const out = html(m.Verification, { block });
  assert.ok(out.includes(m.stampClock(AT)), out);
});

test('22c goal check squares: the tooltip time in en-US and en-GB', () => {
  assert.deepEqual([m.beatWhen(AT, NOW, 'en-US'), m.beatWhen(AT, NOW, 'en-GB')], ['Nov 16, 8:05\u00a0PM', '16 Nov, 20:05']);
  assert.equal(m.beatWhen('nope', NOW), undefined);
  assert.equal(m.beatWhen(undefined, NOW), undefined);
  const out = html(m.GoalHeartbeat, { dots: [{ tone: 'ok', label: 'Passed', at: AT }], status: 'ok' });
  assert.ok(out.includes(`title="Passed · ${m.stampClock(AT)}"`), out);
});

test('22c Pushed meta line: the branch stays whole, cut with …, the full name in its title', () => {
  const c = m.parsePrepush(m.fx.PP_NEXT_PUSHED);
  const out = html(m.PrepushCard, { card: c, bot: 'Pixel' });
  const tag = /<code class="dv-sha dv-sha-whole" title="([^"]+)" data-pushed-branch="true">([^<]+)<\/code>/.exec(out);
  assert.ok(tag, out);
  assert.equal(tag[1], tag[2], 'title carries the full branch name');
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  assert.match(css, /\.dv-sha\.dv-sha-whole \{[^}]*white-space: nowrap;[^}]*overflow: hidden;[^}]*text-overflow: ellipsis;/);
  assert.match(css, /\.dv-sha\.dv-sha-whole \{[^}]*max-width: 100%/);
  const src = readFileSync(new URL('../src/components/dev/PrepushCard.tsx', import.meta.url), 'utf8');
  assert.equal((src.match(/dv-sha dv-sha-wrap/g) ?? []).length >= 1, true, 'the branch in the prose sentences still wraps');
});
