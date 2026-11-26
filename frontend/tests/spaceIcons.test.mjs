// 36j space-icons (Mira, ux-a11y-1124/space-icons/design.md): the 8 picker icons draw as 9x9 pixel glyphs in the
// space colour; the stored value stays the emoji, and 📦, custom emoji and old word values keep the emoji.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SpaceTile, NewSpaceModal } from './src/components/Spaces';
  export { SPACE_ICONS } from './src/spaces';
  export * from './src/spaceIcons';
  export const tile = (props) => renderToStaticMarkup(React.createElement(SpaceTile, props));
  export const newSpace = (props) => renderToStaticMarkup(React.createElement(NewSpaceModal, props));
`);

// space-icons-2: 24 icons, 3 rows of 8 (work and money, make and learn, life), in this order.
const NAMES = [
  'work', 'code', 'chart', 'money', 'shop', 'mail', 'chat', 'box',
  'rocket', 'lab', 'art', 'writing', 'photo', 'music', 'books', 'school',
  'home', 'nature', 'games', 'health', 'travel', 'coffee', 'pets', 'star',
];

test('pixelIconFor: each of the 24 SPACE_ICONS gets its glyph, in the design order', () => {
  assert.equal(m.SPACE_ICONS.length, 24);
  m.SPACE_ICONS.forEach((i, n) => assert.equal(m.pixelIconFor(i)?.name, NAMES[n], i));
  assert.equal(Object.keys(m.SPACE_PIXEL_ICONS).length, 24);
});

test('no duplicate emoji or names, and each stored emoji round-trips to the same picker icon', () => {
  const keys = m.SPACE_ICONS.map(m.iconKey);
  assert.equal(new Set(keys).size, 24);
  assert.equal(new Set(m.SPACE_ICONS.map((i) => m.pixelIconFor(i).name)).size, 24);
  // With or without U+FE0F, the stored value finds the same glyph (✉️ ❤️ ✈️ ✏️ carry it).
  for (const i of m.SPACE_ICONS) assert.equal(m.pixelIconFor(i.replace(/\uFE0F/g, '')), m.pixelIconFor(i), i);
  assert.equal(m.pixelIconFor('\u2709')?.name, 'mail');
});

test('pixelIconFor: trims and ignores the emoji variation selector', () => {
  assert.equal(m.pixelIconFor('🏠️')?.name, 'home');
  assert.equal(m.pixelIconFor(' 💼 ')?.name, 'work');
});

test('pixelIconFor: empty, old words and custom emoji fall back (null); 📦 is now a pixel box', () => {
  for (const v of ['', 'box', '🦄', '⚡']) assert.equal(m.pixelIconFor(v), null, JSON.stringify(v));
  assert.equal(m.pixelIconFor('📦')?.name, 'box');
});

test('glyphPx keeps every pixel square', () => {
  const want = { 16: 9, 28: 18, 32: 18, 40: 18, 44: 18, 48: 27 };
  for (const [tile, px] of Object.entries(want)) assert.equal(m.glyphPx(Number(tile)), px, `tile ${tile}`);
});

test('every grid is 9x9 and uses only #, o and .', () => {
  for (const [emoji, g] of Object.entries(m.SPACE_PIXEL_ICONS)) {
    assert.equal(g.rows.length, 9, emoji);
    for (const r of g.rows) assert.match(r, /^[#o.]{9}$/, `${g.name}: ${r}`);
  }
});

// space-icons-2 exception, by design (Mira's design check): "coffee"'s surface is the cup's open top.
// Every other glyph keeps 36j's rule.
const OPEN_DETAIL = new Set(['coffee']);
test('every detail pixel sits inside main pixels (never floats on the tile), except the by-design one', () => {
  for (const g of Object.values(m.SPACE_PIXEL_ICONS)) {
    if (OPEN_DETAIL.has(g.name)) continue;
    g.rows.forEach((r, y) => [...r].forEach((ch, x) => {
      if (ch !== 'o') return;
      const at = (dx, dy) => g.rows[y + dy]?.[x + dx] ?? '.';
      const enclosed = [[1, 0], [-1, 0], [0, 1], [0, -1]].every(([dx, dy]) => at(dx, dy) !== '.');
      assert.ok(enclosed, `${g.name} detail at ${x},${y}`);
    }));
  }
});

test('SpaceTile with 💼 draws an svg glyph and no emoji text', () => {
  const html = m.tile({ space: { id: 'spc_a', icon: '💼', color: '#f59e0b' }, size: 40 });
  assert.match(html, /<svg[^>]*viewBox="0 0 9 9"[^>]*width="18"[^>]*height="18"/);
  assert.match(html, /shape-rendering="crispEdges"/);
  assert.ok(!html.includes('💼'));
  assert.match(html, /color-mix\(in srgb, #f59e0b 65%, white\)/);
  assert.match(html, /var\(--kw-wa-screen\)/);
  // Tile itself unchanged: tint and inset ring in the space colour; no font size on the glyph branch.
  assert.match(html, /background:#f59e0b33/);
  assert.match(html, /box-shadow:inset 0 0 0 1px #f59e0b66/);
  assert.ok(!/font-size/.test(html));
});

test('SpaceTile with 🦄 keeps the emoji; 📦, empty and an old word draw the pixel box', () => {
  const uni = m.tile({ space: { id: 'spc_a', icon: '🦄', color: '#10b981' }, size: 40 });
  assert.ok(!uni.includes('<svg'));
  assert.ok(uni.includes('🦄'));
  assert.match(uni, /font-size:20px/);
  for (const icon of ['📦', '', 'box']) {
    const html = m.tile({ space: { id: 'spc_a', icon, color: '#10b981' }, size: 40 });
    assert.match(html, /data-space-glyph/, JSON.stringify(icon));
    assert.ok(!html.includes('📦'), JSON.stringify(icon));
  }
});

test('SpaceTile sizes: 16 → 9 px glyph, 48 → 27 px', () => {
  assert.match(m.tile({ space: { id: 'x', icon: '🚀', color: '#8b5cf6' }, size: 16 }), /width="9" height="9"/);
  assert.match(m.tile({ space: { id: 'x', icon: '🚀', color: '#8b5cf6' }, size: 48 }), /width="27" height="27"/);
});

test('New space picker: glyphs in the picked colour, named "Icon: <name>"', () => {
  const html = m.newSpace({ spaces: [], onClose: () => {}, onCreated: () => {} });
  for (const n of NAMES) assert.ok(html.includes(`aria-label="Icon: ${n}"`), n);
  assert.ok(!html.includes('aria-label="Icon 💼"'));
  // 24 picker buttons plus the 48 px preview tile above them.
  assert.equal((html.match(/data-space-glyph/g) || []).length, 25);
  assert.match(html, /<svg[^>]*width="27"/);
  assert.match(html, /<svg[^>]*width="18"/);
  // A new space takes the first unused colour; the picker previews it.
  assert.match(html, /color-mix\(in srgb, #f59e0b 65%, white\)/);
});

test('Space settings uses the same picker, in the tile colour, 6 columns below md', async () => {
  const { readFileSync } = await import('node:fs');
  const src = readFileSync(new URL('../src/components/SpaceSettings.tsx', import.meta.url), 'utf8');
  assert.match(src, /const tileColour = spaceColor\(\{ id: space\.id, color \}\);/);
  assert.match(src, /<SpaceIconPicker[\s\S]*?colour=\{tileColour\}[\s\S]*?gridClass="max-md:grid-cols-6 md:grid-cols-8 md:w-max"/);
  assert.ok(!src.includes('PickerGlyph'));
});
