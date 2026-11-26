// Living avatars (autonomy/living-avatars/design.md, Mira, Nov 26): mood faces from the table, one shared clock
// for idle life (1 timer, at most 3 moving, 400 ms between starts, 1.5 s between list blinks, paused when hidden
// or off screen, nothing under reduced motion), the three moments, and the rail's one foil sweep after a save.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernAvatar, AVATAR_STYLES, pickCreature, quirkOf } from './src/components/ModernAvatar';
  export * from './src/avatarFaces';
  export { createLife, ZONES, MAX_ACTIVE, MIN_GAP, LIST_GAP, BLINK, HOP, NOTICE_BLINK, HOVER_DELAY } from './src/avatarLife';
  export { railJust, onRailJustEnd, RAIL_JUST_BACKSTOP_MS } from './src/railJust';
  export { AVATAR_STYLES, pickCreature, quirkOf };
  export const svg = (p) => renderToStaticMarkup(React.createElement(ModernAvatar, { size: 28, modelBadge: false, ...p }));
`);

const MOODS = ['idle', 'thinking', 'working', 'finished', 'failed', 'needs_you', 'resting'];
const creatures = () => s.AVATAR_STYLES.map((st) => [st.id, s.pickCreature(st.id, st.label)]);
const rects = (html) => [...html.matchAll(/<rect x="(\d+)" y="(\d+)" width="(\d+)" height="(\d+)" fill="([^"]+)"/g)].map(([, x, y, w, h, fill]) => ({ x: +x, y: +y, w: +w, h: +h, fill }));

// ---- Faces ----

test('every mood maps to an eye and mouth frame from the table', () => {
  for (const m of MOODS) {
    const f = s.MOOD_FACE[m];
    assert.ok(f && s.EYES[f.eyes] && s.MOUTHS[f.mouth], m);
  }
  assert.equal(s.faceMood(undefined), 'idle');
  assert.equal(s.faceMood('nonsense'), 'idle');
});

test('glasses always use the lens-safe frames; the headband does except for glances; lens-safe frames stay inside x10-15, y10-12', () => {
  for (const [id, c] of creatures()) {
    for (const e of Object.keys(s.LENS_SAFE)) {
      const got = s.eyeCells(c, e);
      if (c.accessory === 'glasses') assert.deepEqual(got, s.LENS_SAFE[e], `${id} ${e}`);
      else if (c.accessory === 'headband') assert.deepEqual(got, e.startsWith('glance') ? s.EYES[e] : s.LENS_SAFE[e], `${id} ${e}`);
      else assert.deepEqual(got, s.EYES[e], `${id} ${e}`);
    }
  }
  // Scoped to the lens-safe frames: sad's brows (y8-9) still cross the rim, flagged to Mira in impl.md.
  for (const [e, cells] of Object.entries(s.LENS_SAFE))
    for (const [x, y, w, h] of cells) assert.ok(x >= 10 && x + w <= 16 && y >= 10 && y + h <= 13, `${e} [${x},${y},${w},${h}]`);
});

test('house gets eyes only, every cell sits on the 26-grid, and each mood draws a different face', () => {
  for (const [id, c] of creatures()) {
    for (const m of MOODS) {
      const f = s.MOOD_FACE[m];
      if (c.accessory === 'house') assert.equal(s.mouthCells(c, f.mouth).length, 0, `${id} has no mouth`);
      for (const [x, y, w, h] of [...s.eyeCells(c, f.eyes), ...s.mouthCells(c, f.mouth)])
        assert.ok(x >= 0 && y >= 0 && x + w <= 26 && y + h <= 26, `${id} ${m}`);
    }
  }
  assert.equal(s.AVATAR_STYLES.length >= 28, true, 'all 28 creatures are covered');
  const faces = new Set(MOODS.map((m) => s.svg({ role: 'mouse', name: 'Pixel', mood: m === 'idle' ? undefined : m }).replace(/data-mood="[^"]*"|<svg viewBox="0 0 7 7"[\s\S]*?<\/svg>/g, '')));
  assert.equal(faces.size, MOODS.length, 'seven different faces');
});

test('face swaps only the face cells: the badge stays, and plain keeps the idle face whatever the mood', () => {
  const idle = s.svg({ role: 'mouse', name: 'Pixel' });
  const plainFailed = s.svg({ role: 'mouse', name: 'Pixel', mood: 'failed', face: 'plain' });
  const failed = s.svg({ role: 'mouse', name: 'Pixel', mood: 'failed' });
  const body = (h) => rects(h.replace(/<svg viewBox="0 0 7 7"[\s\S]*?<\/svg>/g, ''));
  assert.deepEqual(body(plainFailed), body(idle), 'plain = the idle face');
  assert.notDeepEqual(body(failed), body(idle));
  assert.match(failed, /data-mood="failed"/, 'the badge is still there');
  // Outside x9-17 / y8-16 nothing changes between moods.
  const outside = (h) => body(h).filter((r) => r.x + r.w <= 9 || r.x >= 17 || r.y + r.h <= 8 || r.y >= 16);
  for (const m of MOODS.slice(1)) assert.deepEqual(outside(s.svg({ role: 'mouse', name: 'Pixel', mood: m })), outside(idle), m);
});

test('house still draws its door in every mood and never a mouth', () => {
  for (const m of MOODS) {
    const r = rects(s.svg({ role: 'home', name: 'House', mood: m === 'idle' ? undefined : m }));
    assert.ok(r.some((e) => e.fill === '#8f5b4a' && e.x === 11 && e.y === 16), m);
  }
});

test('a live avatar renders its frames as hidden groups (no re-render per frame); a still one renders none', () => {
  const live = s.svg({ role: 'mouse', name: 'Pixel', size: 96, life: 'full', kwantId: 'k1' });
  for (const f of ['closed', 'glanceL', 'glanceR', 'perk', 'happy']) assert.match(live, new RegExp(`kw-life kw-life-${f}`), f);
  assert.match(live, /kw-face-eyes/);
  assert.match(live, /data-life-zone="full"/);
  assert.doesNotMatch(live, /data-life=/, 'React never sets the frame');
  const still = s.svg({ role: 'mouse', name: 'Pixel', size: 28 });
  assert.doesNotMatch(still, /kw-life/);
  assert.doesNotMatch(s.svg({ role: 'mouse', name: 'Pixel', size: 16, life: 'list' }), /kw-life/, 'too small for life');
  assert.doesNotMatch(s.svg({ role: 'mouse', name: 'Pixel', size: 96, life: 'full', asleep: true }), /kw-life /, 'asleep stays still');
});

test('only each bot’s newest reply shows its mood face', () => {
  const msgs = [
    { id: 'a', sender_role: 'kwant', sender_id: 'k1' },
    { id: 'b', sender_role: 'user', sender_id: 'u' },
    { id: 'c', sender_role: 'kwant', sender_id: 'k2' },
    { id: 'd', sender_role: 'kwant', sender_id: 'k1' },
  ];
  assert.deepEqual([...s.newestReplyIds(msgs)].sort(), ['c', 'd']);
});

// ---- The shared clock ----

function fakeEnv({ reduced = false, seed = 1 } = {}) {
  let t = 0;
  let n = 0;
  const timers = new Map();
  let r = seed;
  const env = {
    now: () => t,
    setTimeout: (fn, ms) => (timers.set(++n, { fn, at: t + ms }), n),
    clearTimeout: (id) => timers.delete(id),
    random: () => ((r = (r * 16807) % 2147483647) / 2147483647),
    reduced: () => reduced,
  };
  const advance = (ms, each) => {
    const end = t + ms;
    for (;;) {
      const next = [...timers.entries()].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > end) break;
      timers.delete(next[0]);
      t = next[1].at;
      next[1].fn();
      each?.(t);
    }
    t = end;
  };
  return { env, advance, timerCount: () => timers.size, now: () => t };
}
function el() {
  const attrs = {};
  const log = [];
  return { attrs, log, setAttribute(k, v) { attrs[k] = v; if (k === 'data-life') log.push(v); }, removeAttribute(k) { delete attrs[k]; } };
}
const moving = (els) => els.filter((e) => 'data-life' in e.attrs || 'data-hop' in e.attrs).length;

test('one timer for 30 avatars; at most 3 move at once; starts are 400 ms apart; list blinks 1.5 s apart', () => {
  const f = fakeEnv();
  const life = s.createLife(f.env);
  const els = [];
  for (let i = 0; i < 30; i++) {
    const e = el();
    els.push(e);
    life.register(e, { zone: i < 28 ? 'list' : i === 28 ? 'header' : 'full', kwantId: `k${i}`, quirk: i % 2 ? 'twitch' : 'none' });
  }
  let maxTimers = 0;
  let maxMoving = 0;
  const listStarts = [];
  const before = new Map(els.map((e) => [e, e.log.length]));
  f.advance(120000, (t) => {
    maxTimers = Math.max(maxTimers, f.timerCount());
    maxMoving = Math.max(maxMoving, moving(els));
    for (const [i, e] of els.entries()) {
      if (e.log.length > before.get(e)) {
        if (i < 28) listStarts.push(t);
        before.set(e, e.log.length);
      }
    }
  });
  assert.ok(maxTimers <= 1, `timers ${maxTimers}`);
  assert.ok(maxMoving <= s.MAX_ACTIVE, `moving ${maxMoving}`);
  assert.ok(life.stats().started === 30);
  assert.ok(listStarts.length > 5, 'list avatars do blink');
  const lsorted = [...new Set(listStarts)].sort((a, b) => a - b);
  // Double blinks re-close the same avatar within 220 ms: count a new start only after a gap > 300 ms.
  const lstarts = lsorted.filter((t, i) => i === 0 || t - lsorted[i - 1] > 300);
  for (let i = 1; i < lstarts.length; i++) assert.ok(lstarts[i] - lstarts[i - 1] >= s.LIST_GAP - 1e-6, `list gap ${lstarts[i] - lstarts[i - 1]}`);
});

test('starts are at least 400 ms apart', () => {
  const f = fakeEnv({ seed: 7 });
  const life = s.createLife(f.env);
  const els = Array.from({ length: 12 }, () => el());
  els.forEach((e, i) => life.register(e, { zone: 'full', kwantId: `k${i}` }));
  const starts = [];
  const prev = new Map(els.map((e) => [e, false]));
  f.advance(60000, (t) => {
    for (const e of els) {
      const on = 'data-life' in e.attrs;
      if (on && !prev.get(e) && !(e.log.length > 1 && t - (e.lastOff ?? -1e9) < 150)) starts.push(t);
      if (!on && prev.get(e)) e.lastOff = t;
      prev.set(e, on);
    }
  });
  const uniq = [...new Set(starts)].sort((a, b) => a - b);
  assert.ok(uniq.length > 10);
  for (let i = 1; i < uniq.length; i++) assert.ok(uniq[i] - uniq[i - 1] >= s.MIN_GAP - 1e-6, `gap ${uniq[i] - uniq[i - 1]}`);
});

test('nothing moves while the window is hidden or the avatar is off screen; no timer while hidden', () => {
  const f = fakeEnv();
  const life = s.createLife(f.env);
  const a = el();
  const b = el();
  life.register(a, { zone: 'full', kwantId: 'a' });
  life.register(b, { zone: 'full', kwantId: 'b' });
  life.setVisible(b, false);
  f.advance(30000);
  assert.equal(b.log.length, 0, 'off screen: still');
  assert.ok(a.log.length > 0, 'on screen: alive');
  life.setHidden(true);
  assert.equal(f.timerCount(), 0, 'hidden: no timer');
  const n = a.log.length;
  f.advance(30000);
  assert.equal(a.log.length, n);
  assert.equal(moving([a, b]), 0);
  life.setHidden(false);
  f.advance(10000);
  assert.ok(a.log.length > n, 'resumes on show');
});

test('reduced motion: no scheduler starts, no timer, no moments', () => {
  const f = fakeEnv({ reduced: true });
  const life = s.createLife(f.env);
  const a = el();
  life.register(a, { zone: 'full', kwantId: 'a' });
  life.perk('a');
  life.hello('a');
  f.advance(60000);
  assert.equal(f.timerCount(), 0);
  assert.equal(a.log.length, 0);
  assert.equal(life.stats().registered, 0);
});

test('list avatars only blink; full avatars also glance or twitch when idle', () => {
  const f = fakeEnv({ seed: 3 });
  const life = s.createLife(f.env);
  const list = el();
  const full = el();
  life.register(list, { zone: 'list', kwantId: 'l', quirk: 'twitch' });
  life.register(full, { zone: 'full', kwantId: 'f', quirk: 'twitch' });
  f.advance(120000);
  assert.deepEqual([...new Set(list.log)], ['closed']);
  assert.ok(full.log.some((x) => x !== 'closed'), 'full has quirks');
});

test('moments: perk after the 160 ms blink (wide + hop), hello hops happy, mood entry hops; failed swaps at once', () => {
  const f = fakeEnv();
  const life = s.createLife(f.env);
  const a = el();
  life.register(a, { zone: 'header', kwantId: 'a' });
  life.perk('a');
  assert.equal(a.attrs['data-life'], undefined, 'today’s blink plays first');
  f.advance(s.NOTICE_BLINK);
  assert.equal(a.attrs['data-life'], 'perk');
  assert.ok('data-hop' in a.attrs);
  f.advance(s.HOP);
  assert.equal(a.attrs['data-life'], undefined, 'back to rest');
  life.hello('a');
  assert.equal(a.attrs['data-life'], 'happy');
  assert.ok('data-hop' in a.attrs);
  f.advance(1000);
  life.setMood(a, 'needs_you');
  assert.equal(a.attrs['data-life'], 'closed', 'blink-through');
  f.advance(100);
  assert.ok('data-hop' in a.attrs, 'needs you hops');
  f.advance(1000);
  life.setMood(a, 'failed');
  assert.equal(a.attrs['data-life'], undefined, 'failed: the face swaps at once, no motion');
  assert.equal(a.attrs['data-hop'], undefined);
});

test('hello waits up to 5 s for the new bot’s avatar to mount', () => {
  const f = fakeEnv();
  const life = s.createLife(f.env);
  life.hello('new');
  f.advance(1000);
  const a = el();
  life.register(a, { zone: 'list', kwantId: 'new' });
  assert.equal(a.attrs['data-life'], 'happy');
});

test('hovering a list row blinks after 150 ms then glances', () => {
  const f = fakeEnv();
  const life = s.createLife(f.env);
  const a = el();
  life.register(a, { zone: 'list', kwantId: 'a' });
  life.hover(a);
  f.advance(s.HOVER_DELAY - 1);
  assert.equal(a.log.length, 0);
  f.advance(1);
  assert.equal(a.attrs['data-life'], 'closed');
  f.advance(400);
  assert.equal(a.attrs['data-life'], 'glanceR');
});

test('no frame flips faster than every 100 ms (at most 3 flips in any 333 ms per avatar apart from a double blink)', () => {
  const f = fakeEnv({ seed: 11 });
  const life = s.createLife(f.env);
  const a = el();
  life.register(a, { zone: 'full', kwantId: 'a', quirk: 'twitch' });
  const flips = [];
  let last = JSON.stringify(a.attrs);
  f.advance(120000, (t) => {
    const now = JSON.stringify(a.attrs);
    if (now !== last) flips.push(t);
    last = now;
  });
  for (let i = 1; i < flips.length; i++) assert.ok(flips[i] - flips[i - 1] >= 100, `flip gap ${flips[i] - flips[i - 1]}`);
});

// ---- The rail moment ----

test('railJust: set once, no restart while on, cleared by the sweep or the 400 ms backstop', async () => {
  let seen = 0;
  const off = s.railJust.subscribe(() => seen++);
  assert.equal(s.railJust.set('s1'), true);
  assert.equal(s.railJust.get(), 's1');
  assert.equal(s.railJust.set('s2'), false, 'no restart while on');
  s.onRailJustEnd({ animationName: 'kw-sticker-press' });
  assert.equal(s.railJust.get(), 's1', 'the press ending does not end it');
  s.onRailJustEnd({ animationName: 'kw-foil-sweep' });
  assert.equal(s.railJust.get(), null);
  s.railJust.set('s3');
  await new Promise((r) => setTimeout(r, s.RAIL_JUST_BACKSTOP_MS + 30));
  assert.equal(s.railJust.get(), null, 'backstop');
  off();
  assert.equal(seen, 4);
});
