// small-avatar-badges (autonomy/small-avatar-badges/design.md "For Pixel", Mira, Nov 26 2026): under 36 px no corner
// mark touches the face. Ported from Mira's design/tools/verify.ts + geometry.ts: every size 20-35, every plate mood,
// the real avatarFaces tables and ModernAvatar's own viewBox / compact zoom; each mark clears the face by >= 0.5 px.
// 36 px and up returns today's values. Plus: an older reply row (face="plain") shows no live mood at all.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernAvatar } from './src/components/ModernAvatar';
  export * from './src/avatarFaces';
  export { smallMark, SMALL_MARKS } from './src/avatarMarks';
  export { moodBadge } from './src/components/AvatarMood';
  export { modelBadgeSize } from './src/engines';
  export const html = (p) => renderToStaticMarkup(React.createElement(ModernAvatar, { name: 'Ada', role: 'Tech Lead', modelBadge: false, ...p }));
`);

// ---- Geometry (Mira's geometry.ts, unchanged maths) ----
const CHEEKS = [[8, 13, 10, 14], [16, 13, 18, 14]];
const cells = (e, m) =>
  [...e.flatMap((k) => [...s.EYES[k], ...(s.LENS_SAFE[k] ?? [])]), ...m.flatMap((k) => s.MOUTHS[k])]
    .map(([x, y, w, h]) => [x, y, x + w, y + h]).concat(CHEEKS);
function toPx(size, [x0, y0, x1, y1], dy) {
  if (size > 20) {
    const k = size < 48 ? 1.2 : 1, cy = size < 48 ? 13.2 : 12, u = size / 26;
    const X = (g) => u * (13 + k * (g - 13)), Y = (g) => u * (cy + k * (g - 12 + dy));
    return [X(x0), Y(y0), X(x1), Y(y1)];
  }
  const u = size / 17, xo = (size - 16 * u) / 2;
  return [xo + (x0 - 5) * u, (y0 + dy - 2.5) * u, xo + (x1 - 5) * u, (y1 + dy - 2.5) * u];
}
/** Everything that can be on the face while a mark shows: life (blink, glances, perk, hop one cell up) from 24 px;
 *  at 20-23 px only the plate's own mood face, plus the landed reply's hop with the finished face. */
function faceZone(size, mood) {
  const moods = mood ? [mood] : Object.keys(s.MOOD_FACE);
  const eyes = moods.map((m) => s.MOOD_FACE[m].eyes), mouths = moods.map((m) => s.MOOD_FACE[m].mouth);
  const out = [];
  if (size >= 24) {
    const e = [...eyes, 'closed', 'glanceL', 'glanceR', 'wide', 'happy'], m = [...mouths, 'o', 'grin'];
    for (const dy of [0, -1]) for (const c of cells(e, m)) out.push(toPx(size, c, dy));
  } else {
    for (const c of cells(eyes, mouths)) out.push(toPx(size, c, 0));
    if (moods.includes('finished')) for (const c of cells(['happy'], ['grin'])) out.push(toPx(size, c, -1));
  }
  return out;
}
/** A mark of m px sitting h px past the avatar's edge on both axes, grown by its ring c. */
function markRect(size, corner, m, h, c) {
  const x0 = corner === 'tr' || corner === 'br' ? size + h - m : -h, y0 = corner === 'tr' || corner === 'tl' ? -h : size + h - m;
  return [x0 - c, y0 - c, x0 + m + c, y0 + m + c];
}
const gap = (a, b) => Math.max(b[0] - a[2], a[0] - b[2], b[1] - a[3], a[1] - b[3]);
const clearance = (size, rect, mood) => Math.min(...faceZone(size, mood).map((f) => gap(rect, f)));

const PLATE = ['thinking', 'finished', 'failed', 'needs_you', 'resting'];
const CLEAR = 0.5;
const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
const ruleOf = (sel) => {
  const m = css.match(new RegExp(`${sel.replace(/[.[\]]/g, '\\$&')} \\{([^}]*)\\}`));
  return m ? m[1] : '';
};
const pxOf = (rule, p) => { const v = rule.match(new RegExp(`(?:^|[\\s;])${p}:\\s*(-?\\d+)px`)); return v ? Number(v[1]) : null; };

test('the mood plate clears every face cell by >= 0.5 px at every size 20-35, in every mood', () => {
  for (let size = 20; size <= 35; size++) {
    const b = s.moodBadge(size);
    for (const mood of PLATE) {
      const c = clearance(size, markRect(size, 'tr', b.px, -b.offset, b.cut), mood);
      assert.ok(c >= CLEAR, `${size} px ${mood}: plate ${b.px} px at ${b.offset} clears by ${c.toFixed(2)}`);
    }
  }
});

test('the status dot, model badge and its low dot clear the face from 24 px; the dev hat from 28 px', () => {
  for (let size = 24; size <= 35; size++) {
    const st = s.smallMark(size, 'status'), mb = s.smallMark(size, 'model');
    const d = s.modelBadgeSize(size);
    assert.equal(d, mb.px, `${size} px: modelBadgeSize agrees with smallMark`);
    const status = clearance(size, markRect(size, 'br', st.px, st.out, 2));
    const model = clearance(size, markRect(size, 'bl', d, mb.out, 2));
    const [bx0, by0] = markRect(size, 'bl', d, mb.out, 0);
    const low = clearance(size, [bx0 + d - 3 - 1, by0 - 1 - 1, bx0 + d + 1 + 1, by0 + 3 + 1]);
    for (const [k, v] of Object.entries({ status, model, low })) assert.ok(v >= CLEAR, `${size} px ${k} clears by ${v.toFixed(2)}`);
    if (size >= 28) {
      const hat = s.smallMark(size, 'hat');
      const c = clearance(size, markRect(size, 'tl', hat.px, hat.out, 1.5));
      assert.ok(c >= CLEAR, `${size} px hat clears by ${c.toFixed(2)}`);
    }
  }
  // The CSS carries the hat's small size and the spark's small offset.
  assert.equal(pxOf(ruleOf('.kw-avatar-sm .kw-dev-hat'), 'width'), 12);
  assert.equal(pxOf(ruleOf('.kw-avatar-sm .kw-dev-hat'), 'height'), 12);
  assert.equal(pxOf(ruleOf('.kw-model-badge[data-small] .kw-mb-spark'), 'left'), -4);
});

test('under 36 px the rendered marks use the small sizes: 10 px glyph-only plate, 8 px dot 1 px out, small root class', () => {
  for (const size of [20, 28, 32, 34, 35]) {
    const h = s.html({ size, mood: 'finished', status: 'available' });
    assert.match(h, /<svg viewBox="1 1 5 5" width="10" height="10"[^>]*class="kw-mood kw-mood-finished/, `${size} px plate`);
    assert.match(h, /top:-4px;right:-4px;border-radius:0/, `${size} px plate corner`);
    assert.match(h, /class="kw-avatar [^"]*kw-avatar-sm/, `${size} px root class`);
    if (size >= 24) assert.match(h, /rounded-full ring-2 ring-zinc-950 w-2 h-2 -bottom-px -right-px/, `${size} px dot`);
    else assert.doesNotMatch(h, /ring-2 ring-zinc-950/, `${size} px: no status dot under 24`);
  }
});

test('36 px and up returns today\'s values, unchanged', () => {
  for (const size of [36, 40, 48, 63]) {
    assert.equal(s.smallMark(size, 'mood'), null);
    assert.equal(s.smallMark(size, 'status'), null);
    assert.equal(s.smallMark(size, 'model'), null);
    assert.equal(s.smallMark(size, 'hat'), null);
    assert.deepEqual(s.moodBadge(size), { cellPx: 2, px: 14, offset: -4, radius: 2, cut: 1, pad: true });
  }
  assert.deepEqual(s.moodBadge(64), { cellPx: 4, px: 28, offset: -6, radius: 4, cut: 2, pad: true });
  assert.deepEqual(s.moodBadge(96), { cellPx: 4, px: 28, offset: -6, radius: 4, cut: 2, pad: true });
  for (const size of [36, 48, 72, 96]) assert.equal(s.modelBadgeSize(size), Math.max(14, Math.min(28, Math.round(size * 0.28))));
  const h = s.html({ size: 36, mood: 'finished', status: 'available' });
  assert.match(h, /<svg viewBox="0 0 7 7" width="14" height="14"/);
  assert.match(h, /top:-4px;right:-4px;border-radius:2px/);
  assert.match(h, /ring-2 ring-zinc-950 bottom-0 right-0 w-2\.5 h-2\.5/);
  assert.doesNotMatch(h, /kw-avatar-sm/);
});

test('ChatView: the phone reply avatar cuts its plate out of the stream background', () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const line = chat.split('\n').find((l) => l.includes('size={REPLY_AVATAR_PHONE} kwantId={replyKwantId}'));
  assert.ok(line, 'the phone reply avatar line');
  assert.match(line, /'--kw-avatar-cut'[^}]*'var\(--kw-surface-main\)'/);
});

// ---- Mira, 0.9.57 media "Known product issues": older reply rows keep the live mood mark ----

test('an older reply row (face="plain") shows no live mood: no plate, no ring, no data-mood, no resting tint', () => {
  for (const mood of ['thinking', 'finished', 'failed', 'needs_you', 'resting', 'working']) {
    const plain = s.html({ size: 20, kwantId: 'k1', mood, face: 'plain' });
    assert.doesNotMatch(plain, /class="kw-mood/, `${mood}: no plate on an older row`);
    assert.doesNotMatch(plain, /kw-avatar-ring/, `${mood}: no working ring on an older row`);
    assert.doesNotMatch(plain, /data-mood=/, `${mood}: no data-mood on an older row`);
    assert.doesNotMatch(plain, /kw-avatar-resting|kw-avatar-working/, `${mood}: no resting or working tint`);
    assert.doesNotMatch(plain, /title="[^"]*(thinking|working|finished|failed|needs|resting)/i, `${mood}: the title is just the name`);
  }
  // The newest reply row (face="mood") still shows its live mood.
  const live = s.html({ size: 20, kwantId: 'k1', mood: 'finished', face: 'mood' });
  assert.match(live, /class="kw-mood kw-mood-finished/);
  assert.match(live, /data-mood="finished"/);
  assert.match(s.html({ size: 20, kwantId: 'k1', mood: 'working' }), /kw-avatar-ring/);
});
