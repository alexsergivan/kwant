// reply-bubble-bot-tint (autonomy/reply-bubble-bot-tint/design.md): a bot's reply bubble wears a dark tint of its
// avatar's body colour, from the 23-row table in bubbleTint.ts; creatureFor resolves the same creature the avatar draws.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { BUBBLE_TINTS, bubbleTintStyle } from './src/components/bubbleTint';
  export { PALETTE_NAMES, paletteCreature, creatureFor, pickCreature } from './src/components/ModernAvatar';
  export { setKnownBots } from './src/botLooks';
`);

test('every avatar palette body has exactly one tint row, and the table has no extras', () => {
  assert.equal(m.PALETTE_NAMES.length, 23);
  const bodies = new Set(m.PALETTE_NAMES.map((n) => m.paletteCreature(n).body.toLowerCase()));
  for (const b of bodies) assert.ok(m.BUBBLE_TINTS[b], `tint for ${b}`);
  assert.deepEqual(Object.keys(m.BUBBLE_TINTS).sort(), [...bodies].sort());
  for (const [b, t] of Object.entries(m.BUBBLE_TINTS)) {
    assert.match(t.bg, /^#[0-9a-f]{6}$/, b);
    assert.match(t.line, /^#[0-9a-f]{6}$/, b);
  }
});

test("design.md's spot checks: yellow, slate, lime, green", () => {
  const at = (n) => m.bubbleTintStyle(m.paletteCreature(n).body);
  assert.deepEqual(at('yellow'), { '--kw-bubble-bg': '#221904', '--kw-bubble-line': '#553f00' });
  assert.deepEqual(at('slate'), { '--kw-bubble-bg': '#171a24', '--kw-bubble-line': '#3a4158' });
  assert.deepEqual(at('lime'), { '--kw-bubble-bg': '#171d08', '--kw-bubble-line': '#3a490b' });
  assert.deepEqual(at('green'), { '--kw-bubble-bg': '#0c2011', '--kw-bubble-line': '#194f29' });
  // Upper-case body hex resolves too.
  assert.deepEqual(m.bubbleTintStyle('#FDC934'), at('yellow'));
});

// WCAG relative luminance; the body text (zinc-200, retinted in tailwind.config.js) must stay AA on every tint.
const lum = (hex) => {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };

test('every tint stays dark: the tailwind zinc-500 faint text keeps at least 4.5:1 on the bare tint', async () => {
  const { default: cfg } = await import('../tailwind.config.js');
  const zinc = cfg.theme?.extend?.colors?.zinc ?? cfg.theme?.colors?.zinc;
  assert.ok(zinc?.['500'] && zinc?.['200'], 'kwant retints zinc');
  for (const [b, t] of Object.entries(m.BUBBLE_TINTS)) {
    assert.ok(ratio(zinc['200'], t.bg) >= 12, `${b}: body text ${ratio(zinc['200'], t.bg).toFixed(2)}`);
    assert.ok(ratio(zinc['500'], t.bg) >= 4.5, `${b}: faint text ${ratio(zinc['500'], t.bg).toFixed(2)}`);
  }
});

test('an unknown or missing body gets no tint (the bubble keeps today\'s classes)', () => {
  assert.equal(m.bubbleTintStyle('#123456'), undefined);
  assert.equal(m.bubbleTintStyle(undefined), undefined);
  assert.equal(m.bubbleTintStyle(''), undefined);
});

test('creatureFor is the avatar\'s own pick: name hints, chosen styles and a saved look', () => {
  for (const [role, name] of [['Team', 'Shell'], ['Team', 'Guard'], ['default', 'Nova'], ['user', 'You'], ['Support', 'Support']]) {
    assert.deepEqual(m.creatureFor(role, name), m.pickCreature(role, name), `${role}/${name}`);
  }
  assert.equal(m.creatureFor('Team', 'Shell').body, m.paletteCreature('green').body);
  // A bot's saved look (avatar_sprite) wins over its name, exactly as ModernAvatar draws it.
  m.setKnownBots([{ id: 'k-look', name: 'Shell', avatar_sprite: 'designer' }]);
  const saved = m.creatureFor('Team', 'Shell', 'k-look');
  m.setKnownBots([]);
  assert.equal(saved.body, m.paletteCreature('coral').body, 'the saved designer look (coral), not the name\'s green');
  assert.deepEqual(m.bubbleTintStyle(saved.body), { '--kw-bubble-bg': '#2a1310', '--kw-bubble-line': '#682e27' });
});
