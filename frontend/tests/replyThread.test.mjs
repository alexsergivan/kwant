// bubble-not-alert (autonomy/bubble-not-alert/design.md): a bot reply has no box; its creature thread takes the
// avatar's body colour from replyThread.ts, and creatureFor resolves the same creature the avatar draws.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, existsSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { replyThreadStyle } from './src/components/replyThread';
  export { PALETTE_NAMES, paletteCreature, creatureFor, pickCreature } from './src/components/ModernAvatar';
  export { setKnownBots } from './src/botLooks';
`);

test('every avatar palette gives its own body hex as the thread colour', () => {
  assert.equal(m.PALETTE_NAMES.length, 23);
  for (const name of m.PALETTE_NAMES) {
    const body = m.paletteCreature(name).body;
    assert.deepEqual(m.replyThreadStyle(body), { '--kw-thread': body }, name);
  }
  // Upper-case hex is a hex too.
  assert.deepEqual(m.replyThreadStyle('#FDC934'), { '--kw-thread': '#FDC934' });
});

test('no colour, an empty string or a non-hex gives no thread colour (CSS falls back to the hairline)', () => {
  assert.equal(m.replyThreadStyle(undefined), undefined);
  assert.equal(m.replyThreadStyle(''), undefined);
  assert.equal(m.replyThreadStyle('red'), undefined);
  assert.equal(m.replyThreadStyle('#fff'), undefined);
  assert.equal(m.replyThreadStyle('#12345g'), undefined);
  assert.equal(m.replyThreadStyle('#123456;background:red'), undefined, 'never a CSS injection');
});

test("creatureFor is the avatar's own pick: name hints, chosen styles and a saved look", () => {
  for (const [role, name] of [['Team', 'Shell'], ['Team', 'Guard'], ['default', 'Nova'], ['user', 'You'], ['Support', 'Support']]) {
    assert.deepEqual(m.creatureFor(role, name), m.pickCreature(role, name), `${role}/${name}`);
  }
  assert.equal(m.creatureFor('Team', 'Shell').body, m.paletteCreature('green').body);
  m.setKnownBots([{ id: 'k-look', name: 'Shell', avatar_sprite: 'designer' }]);
  const saved = m.creatureFor('Team', 'Shell', 'k-look');
  m.setKnownBots([]);
  assert.equal(saved.body, m.paletteCreature('coral').body, "the saved designer look (coral), not the name's green");
  assert.deepEqual(m.replyThreadStyle(saved.body), { '--kw-thread': saved.body });
});

test('the 0.9.55 tint table and its CSS are gone', () => {
  assert.equal(existsSync(new URL('../src/components/bubbleTint.ts', import.meta.url)), false);
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /kw-bubble-tint|--kw-bubble-bg|--kw-bubble-line/);
});

test('modern.css: the plain reply and its thread, as design.md specifies', () => {
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const block = (sel) => {
    const at = css.indexOf(`${sel} {`);
    assert.ok(at >= 0, sel);
    return css.slice(at, css.indexOf('}', at));
  };
  const plain = block('.kw-reply-plain');
  assert.match(plain, /position: relative;/);
  assert.match(plain, /padding-left: 1\.5rem;/);
  assert.doesNotMatch(plain, /background|border|border-radius|box-shadow/, 'no box');
  const thread = block('.kw-reply-plain::before');
  for (const rule of [/left: 9px;/, /top: 4px;/, /bottom: 4px;/, /width: 2px;/, /border-radius: 1px;/,
    /background: var\(--kw-thread, var\(--kw-hairline-strong\)\);/, /opacity: 0\.5;/, /pointer-events: none;/,
    /transition: opacity 150ms var\(--kw-ease\);/]) assert.match(thread, rule);
  assert.match(css, /:is\(\.group\\\/msg, \[data-live-row\]\):is\(:hover, :focus-within\) \.kw-reply-plain::before \{ opacity: 0\.9; \}/);
  assert.match(css, /@media \(min-width: 640px\) \{\s+\.kw-reply-plain \{ padding-left: 0; min-height: 36px; \}\s+\.kw-reply-plain::before \{ left: -33px; top: 10px; \}/);
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\s+\.kw-reply-plain::before \{ transition: none; \}/);
  assert.match(css, /\.kw-reply-plain :is\(p, ul, ol, h1, h2, h3, h4, h5, h6, blockquote\) \{ max-width: 42rem; \}/);
});
