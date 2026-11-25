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

const NAMES = ['work', 'nature', 'books', 'games', 'home', 'rocket', 'art', 'lab'];

test('pixelIconFor: each of the 8 SPACE_ICONS gets its glyph', () => {
  assert.equal(m.SPACE_ICONS.length, 8);
  m.SPACE_ICONS.forEach((i, n) => assert.equal(m.pixelIconFor(i)?.name, NAMES[n], i));
});

test('pixelIconFor: trims and ignores the emoji variation selector', () => {
  assert.equal(m.pixelIconFor('🏠️')?.name, 'home');
  assert.equal(m.pixelIconFor(' 💼 ')?.name, 'work');
});

test('pixelIconFor: 📦, empty, old words and custom emoji fall back (null)', () => {
  for (const v of ['📦', '', 'box', '🦄']) assert.equal(m.pixelIconFor(v), null, JSON.stringify(v));
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

test('every detail pixel sits inside main pixels (never floats on the tile)', () => {
  for (const g of Object.values(m.SPACE_PIXEL_ICONS)) {
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

test('SpaceTile with 🦄, 📦 or an old word keeps the emoji', () => {
  for (const [icon, shown] of [['🦄', '🦄'], ['📦', '📦'], ['box', '📦']]) {
    const html = m.tile({ space: { id: 'spc_a', icon, color: '#10b981' }, size: 40 });
    assert.ok(!html.includes('<svg'), icon);
    assert.ok(html.includes(shown), icon);
    assert.match(html, /font-size:20px/);
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
  // 8 picker buttons plus the 48 px preview tile above them.
  assert.equal((html.match(/data-space-glyph/g) || []).length, 9);
  assert.match(html, /<svg[^>]*width="27"/);
  assert.match(html, /<svg[^>]*width="18"/);
  // A new space takes the first unused colour; the picker previews it.
  assert.match(html, /color-mix\(in srgb, #f59e0b 65%, white\)/);
});

test('Space settings picker uses the same glyph and names', async () => {
  const { readFileSync } = await import('node:fs');
  const src = readFileSync(new URL('../src/components/SpaceSettings.tsx', import.meta.url), 'utf8');
  assert.match(src, /aria-label=\{`Icon: \$\{pixelIconFor\(i\)\?\.name \?\? i\}`\}/);
  assert.match(src, /<PickerGlyph icon=\{i\} colour=\{spaceColor\(\{ id: space\.id, color \}\)\} \/>/);
});
