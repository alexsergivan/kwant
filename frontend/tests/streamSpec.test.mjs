// Mira's spec (ui-refresh/design/spec.md) as applied to the conversation stream: S1 in the action row,
// the heartbeat squares, the three chips, the 44 px sticker hit area and no confetti.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { VerifiedStamp, GoalHeartbeat, EngineChip, LessonsChip } from './src/components/Signature';
  export { verifiedCounts } from './src/components/results/SmartResult';
  export { heartbeatDots } from './src/signature';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const stamp = r(VerifiedStamp);
  export const beat = r(GoalHeartbeat);
  export const engine = r(EngineChip);
  export const lessons = r(LessonsChip);
`);

const check = (status) => ({ id: status + Math.random(), label: 'x', kind: 'k', status, checked_at: null });
const vb = (...st) => ({ type: 'verification', checks: st.map(check) });

test('S1: a Checks block earns the stamp only when every check passed', () => {
  assert.deepEqual(m.verifiedCounts(vb('passed', 'passed', 'passed')), { passed: 3, total: 3 });
  assert.equal(m.verifiedCounts(vb('passed', 'pending')), null);
  assert.equal(m.verifiedCounts(vb('passed', 'failed')), null);
  assert.equal(m.verifiedCounts({ type: 'comparison' }), null);
});

test('S1/S3 stickers: holo, −2° (stamp) and −3° (first), clickable ones inside a 44 px .kw-sticker-hit button', () => {
  const plain = m.stamp({ text: 'Verified · 3/3', label: 'All 3 checks passed' });
  assert.match(plain, /kw-sticker-holo/);
  assert.match(plain, /--kw-tilt:-2deg/);
  assert.doesNotMatch(plain, /<button/);
  const first = m.stamp({ text: 'Verified', label: 'L', first: true, onOpen: () => {} });
  assert.match(first, /--kw-tilt:-3deg/);
  assert.match(first, /<button[^>]*class="kw-sticker-hit"[^>]*>.*<span[^>]*kw-sticker/);
  assert.doesNotMatch(first, /confetti/);
});

test('heartbeat: nothing without data; one picture with the spec label, count and word', () => {
  assert.equal(m.beat({ dots: [] }), '');
  const dots = m.heartbeatDots(['met', 'met', 'not_met', 'met', 'met', 'met', 'met'].map((v) => ({ kind: 'checked', detail: { verdict: v } })));
  const html = m.beat({ dots });
  assert.match(html, /role="img" aria-label="Last 7 checks: 6 ok, 1 failing"/);
  assert.match(html, />6\/7 ok</);
  assert.equal((html.match(/kws-beat-newest/g) || []).length, 2, 'the newest is outlined in the full and tight views');
  assert.equal((html.match(/kws-beat-desk/g) || []).length, 2, 'phones drop the two oldest of 7');
  assert.match(html, /aria-label="Last 5 checks: 4 ok, 1 failing"[^]*>4\/5 ok</, 'the tight view counts the last 5');
});

test('chips: engine (Cpu) never shows cost; failover is info with a pixel baton and the reason as tooltip and label; learned is never holo', () => {
  const e = m.engine({ label: 'Claude · Team' });
  assert.match(e, /kws-chip-engine/);
  assert.match(e, /Claude · Team/);
  assert.doesNotMatch(e, /\$|cost|tokens/i);
  const f = m.engine({ label: 'x', handoff: { from: 'Claude', to: 'Codex', why: 'Claude hit its usage limit · Codex took over' } });
  assert.match(f, /kws-chip-failover/);
  assert.doesNotMatch(f, /amber|warn/);
  assert.match(f, /data-tip="Claude hit its usage limit · Codex took over"/);
  assert.match(f, /aria-label="Claude to Codex\. Claude hit its usage limit · Codex took over"/);
  assert.match(f, /<svg[^>]*width="12"/);
  const l = m.lessons({ text: 'Learned from 3', title: 'From 3 earlier tasks' });
  assert.match(l, /kws-chip-learned/);
  assert.doesNotMatch(l, /holo|kw-sticker/);
});

test('no confetti left in the stream, and the reply header avatar carries the mood', () => {
  const css = readFileSync(new URL('../src/styles/stream.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /confetti/);
  const sig = readFileSync(new URL('../src/components/FirstMoment.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(sig, /Confetti/);
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.equal((chat.match(/kwantId=\{replyKwantId\}/g) || []).length, 2, 'desktop and phone reply header avatars');
});
