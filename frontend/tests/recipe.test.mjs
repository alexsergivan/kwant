// Recipe Smart Results card (0.7.6, next-076/recipe-result/design.md): scaling a leading amount (fractions
// snap, both ends of a range, anything else as written), ticks and servings kept per message under one
// key, the phone fold (more than 10, show 8), the two copy actions, validRecipe, and the card replacing
// its stretch of the reply exactly once.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
// The viewport the fold reads; flipped per test.
let phone = false;
const listeners = new Set();
g.window.matchMedia = (q) => ({
  get matches() {
    return q.includes('max-width: 639px') ? phone : false;
  },
  addEventListener: (_e, cb) => listeners.add(cb),
  removeEventListener: (_e, cb) => listeners.delete(cb),
});
g.matchMedia = g.window.matchMedia;
const clipboard = { text: null, writeText: async (t) => void (clipboard.text = t) };
Object.defineProperty(g.navigator, 'clipboard', { value: clipboard, configurable: true });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/recipe';
  export { MarkdownRenderer } from './src/components/MarkdownRenderer';
  export { Recipe, validRecipe, FOLD_OVER, FOLD_SHOW, TWO_COLUMNS_MIN } from './src/components/results/Recipe';
  export { RecipeActions } from './src/components/results/RecipeActions';
  export { recipe, recipeFixture, RECIPE_EDGES, RECIPE_CONTRACT } from './src/components/results/recipeFixtures';
  export { blockMarkdown } from './src/components/results/exporters';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
afterEach(() => {
  for (const v of mounted) v.done();
  mounted.clear();
  localStorage.clear();
  sessionStorage.clear();
  clipboard.text = null;
  phone = false;
});
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  const v = {
    host,
    done: () => {
      root.unmount();
      host.remove();
    },
  };
  mounted.add(v);
  return v;
}
const click = async (el) => {
  assert.ok(el, 'element to click');
  el.click();
  await settle();
};
const stew = () => m.validRecipe(m.recipe.results.blocks[0]);
const card = (block = stew(), messageId = 'm1') => h(m.Recipe, { block, messageId, reply: null, ask: () => {} });
// What a sighted cook reads: the screen-reader-only "(was …)" cue left out.
const seen = (el) => {
  const c = el.cloneNode(true);
  c.querySelectorAll('.sr-only').forEach((x) => x.remove());
  return c.textContent;
};
const amounts = (host) => [...host.querySelectorAll('[data-ingredient]')].map(seen);
const btn = (host, label) => host.querySelector(`button[aria-label="${label}"]`);
const byText = (host, text) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === text);

// ---------------------------------------------------------------------------------------------------
// Scaling (design §3)

test('scaling: whole numbers, decimals, fractions, mixed numbers and glyphs', () => {
  const s = (q, k) => m.scaleQty(q, k).qty;
  assert.equal(s('500 g', 1.5), '750 g', '10 or more rounds to a whole number');
  assert.equal(s('1 cup', 1.5), '1½ cups', 'snaps to ½ and the unit goes plural');
  assert.equal(s('1 tbsp', 1.5), '1½ tbsp', 'abbreviations stay as written');
  assert.equal(s('2 tsp', 0.5), '1 tsp');
  assert.equal(s('½ cup', 0.5), '¼ cup');
  assert.equal(s('1/2 cup', 2), '1 cup');
  assert.equal(s('1 1/2 cups', 2), '3 cups');
  assert.equal(s('1½ cups', 2), '3 cups');
  assert.equal(s('1,5 l', 2), '3 l', 'a comma decimal');
  assert.equal(s('1.5 kg', 3), '4½ kg');
  assert.equal(s('1 tbsp', 0.25), '¼ tbsp');
  assert.equal(s('½ tsp', 0.25), '⅛ tsp', '⅛ only below 1');
  assert.equal(s('1 cup', 1 / 3), '⅓ cup');
  assert.equal(s('1 cup', 2 / 3), '⅔ cup');
  assert.equal(s('2 cups', 1.2), '2⅓ cups', 'within 0.07 of a glyph snaps');
  assert.equal(s('2 cups', 1.45), '2.9 cups', 'off every glyph: one decimal with a dot');
  assert.equal(s('2 cloves', 0.5), '1 clove', 'plural to singular at 1');
  assert.equal(s('2 cloves', 0.25), '½ clove', 'singular under 1');
  assert.equal(s('1 onion', 1.5), '1½ onions');
  assert.equal(s('2 Cups', 0.5), '1 Cup', "keeps the reply's capital");
  assert.equal(s('3 potatoes', 1 / 3), '1 potato');
  assert.equal(s('1,000 g', 1.5), '1,500 g', 'a comma before three digits is a thousands separator (contract §1), and it stays (F-3)');
  assert.equal(s('1,000 g', 0.5), '500 g', 'under 1,000 there is nothing to separate');
  assert.equal(s('1000 g', 1.5), '1500 g', 'no separator when the reply wrote none');
  assert.equal(s('1,5 l', 2), '3 l', 'and before one or two it is a decimal');
});

test('F-3: a non-zero amount never shows as 0; ⅛ down to 1/16, else the reply’s own amount', () => {
  const s = (q, k) => m.scaleQty(q, k);
  assert.deepEqual(s('0.25 tsp', 1 / 4), { qty: '⅛ tsp', scaled: true }, '0.0625 is ⅛, not 0');
  assert.deepEqual(s('¼ tsp', 1 / 4), { qty: '⅛ tsp', scaled: true });
  assert.deepEqual(s('0.25 tsp', 1 / 24), { qty: '0.25 tsp', scaled: false }, 'too small to write: kept as written, not marked scaled');
  assert.deepEqual(s('1–2 pinches', 1 / 24), { qty: '1–2 pinches', scaled: false }, 'a range with an unwritable end stays whole');
  assert.equal(m.formatAmount(0.0625), '⅛');
  assert.equal(m.formatAmount(0.03), null);
  assert.equal(m.formatAmount(0), '0');
});

test('F-3: a counted thing followed by another noun keeps its form; units and line ends still flip', () => {
  const s = (q, k, after) => m.scaleQty(q, k, after).qty;
  assert.equal(s('1 egg', 1.5, 'yolk'), '1½ egg', '"1 egg yolk" → "1½ egg yolk", never "eggs yolk"');
  assert.equal(s('2 eggs', 0.5, 'whites'), '1 eggs', 'the reply’s own form stays either way');
  assert.equal(s('1 egg yolk', 3), '3 egg yolk', 'the noun inside the amount counts too');
  assert.equal(s('1 egg', 2, ''), '2 eggs', '"1 egg" → "2 eggs" still flips');
  assert.equal(s('1 egg', 2, ', beaten'), '2 eggs', 'a comma after it is not a noun');
  assert.equal(s('1 onion', 2, ', diced'), '2 onions');
  assert.equal(s('1 cup', 2, 'flour'), '2 cups', 'a unit flips whatever follows');
  assert.equal(s('1 can', 2, '(400 g) chopped tomatoes'), '2 cans');
  const block = { ...stew(), serves: 1, ingredients: [{ id: 'i1', qty: '1 egg', text: 'yolk', group: null }, { id: 'i2', qty: '1 cup', text: 'flour', group: null }] };
  assert.match(m.shoppingListText(block, new Set(), 3), /- 3 egg yolk\n- 3 cups flour/, 'the shopping list follows the same rule');
});

test('scaling: both ends of a range scale; nothing else changes', () => {
  const s = (q, k) => m.scaleQty(q, k);
  assert.equal(s('1–2 tsp', 1.5).qty, '1½–3 tsp');
  assert.equal(s('1-2 tsp', 0.25).qty, '¼–½ tsp', 'a hyphen range is written with an en dash once scaled');
  assert.equal(s('2–3 cups', 2).qty, '4–6 cups', 'the unit follows the top of the range');
  assert.deepEqual(s('A pinch of flaky salt', 2), { qty: 'A pinch of flaky salt', scaled: false });
  assert.deepEqual(s('Zest of 1 lemon', 2), { qty: 'Zest of 1 lemon', scaled: false }, 'a number that is not leading stays');
  assert.deepEqual(s('2 cups', 1), { qty: '2 cups', scaled: false }, 'k = 1 is the reply exactly');
  assert.equal(m.canScale('Salt to taste'), false);
  assert.equal(m.canScale(null), false);
  assert.equal(m.canScale('½'), true);
  assert.equal(m.formatAmount(0.5), '½');
  assert.equal(m.formatAmount(12.4), '12');
  assert.equal(m.formatAmount(0.98), '1', 'snaps up to the next whole number');
});

test('the card: the stepper scales amounts in sky, Back to restores, and the limits hold', async () => {
  const v = await mount(card());
  const out = v.host.querySelector('output');
  assert.equal(out.textContent, '4');
  assert.equal(v.host.querySelector('[data-scaled]'), null, 'nothing is marked scaled at the reply’s own number');
  assert.equal(byText(v.host, 'Back to 4'), undefined);
  await click(btn(v.host, 'More servings'));
  await click(btn(v.host, 'More servings'));
  assert.equal(out.textContent, '6');
  assert.equal(out.getAttribute('aria-label'), '6 servings');
  const a = amounts(v.host);
  assert.ok(a.includes('750 g squash, peeled and cubed'));
  assert.ok(a.includes('1½ onions, diced'), 'qty + text joins with no space before a comma');
  assert.ok(a.includes('1½–3 tsp chili flakes'));
  assert.ok(a.includes('Salt to taste'), 'no qty, as written');
  const scaled = v.host.querySelector('[data-scaled]');
  assert.match(scaled.className, /text-sky-200/);
  assert.match(scaled.className, /font-semibold/);
  assert.equal(scaled.getAttribute('title'), 'In the reply: 1 tbsp');
  await click(byText(v.host, 'Back to 4'));
  assert.equal(out.textContent, '4');
  assert.ok(amounts(v.host).includes('500 g squash, peeled and cubed'));

  for (let i = 0; i < 5; i++) await click(btn(v.host, 'Fewer servings'));
  assert.equal(out.textContent, '1');
  assert.equal(btn(v.host, 'Fewer servings').getAttribute('aria-disabled'), 'true', 'aria-disabled, never disabled');
  assert.equal(btn(v.host, 'Fewer servings').disabled, false);
  assert.ok(amounts(v.host).includes('¼–½ tsp chili flakes'));
  for (let i = 0; i < 30; i++) await click(btn(v.host, 'More servings'));
  assert.equal(out.textContent, '24', 'the range is 1–24');
  assert.equal(btn(v.host, 'More servings').getAttribute('aria-disabled'), 'true');
});

test('F-5: a scaled amount carries a screen-reader cue with the reply’s amount; none at the reply’s own number', async () => {
  const v = await mount(card());
  assert.equal(v.host.querySelector('[data-ingredient] .sr-only'), null);
  await click(btn(v.host, 'More servings'));
  await click(btn(v.host, 'More servings'));
  const oil = v.host.querySelector('[data-ingredient="i1"]');
  assert.equal(oil.querySelector('.sr-only').textContent, ' (was 1 tbsp)');
  assert.equal(oil.textContent, '1½ tbsp (was 1 tbsp) olive oil', 'read out as: new amount, was, the rest of the line');
  const salt = [...v.host.querySelectorAll('[data-ingredient]')].find((el) => el.textContent.includes('Salt to taste'));
  assert.equal(salt.querySelector('.sr-only'), null, 'a line without an amount has no cue');
});

test('F-1: links, bold and code in ingredients, steps and notes are drawn as the reply wrote them', async () => {
  const base = stew();
  const block = {
    ...base,
    ingredients: [
      { id: 'i1', qty: null, text: '**2 cups** flour', group: null },
      { id: 'i2', qty: '1 cup', text: '[sugar](https://example.com/sugar), sifted', group: null },
      { id: 'i3', qty: '1 onion', text: ', diced', group: null },
    ],
    steps: [
      { id: 's1', text: 'Mix it **well**.' },
      { id: 's2', text: 'Bake it; see [the guide](https://example.com/bake) and `200 C`.' },
      { id: 's3', text: '1. Not a list: <b>raw</b> stays text.' },
    ],
    notes: ['Use *cold* butter, see [notes](https://example.com/notes).'],
  };
  const v = await mount(card(block));
  const step = (id) => v.host.querySelector(`[data-step="${id}"]`);
  const ing = (id) => v.host.querySelector(`[data-ingredient="${id}"]`);

  assert.equal(ing('i1').querySelector('strong')?.textContent, '2 cups', 'bold stays bold in an ingredient');
  assert.equal(seen(ing('i1')).trim(), '2 cups flour');
  assert.equal(step('s1').querySelector('strong')?.textContent, 'well', 'bold stays bold in a step');

  const a = step('s2').querySelector('a');
  assert.equal(a?.getAttribute('href'), 'https://example.com/bake', 'a link keeps its target');
  assert.equal(a.textContent, 'the guide');
  assert.equal(a.getAttribute('target'), '_blank', 'opened the way the chat opens a link');
  assert.equal(a.getAttribute('rel'), 'noopener noreferrer');
  assert.equal(step('s2').querySelector('code')?.textContent, '200 C', 'code stays code');
  assert.match(step('s2').textContent, /Bake it; see the guide and 200 C\./);

  assert.equal(step('s3').querySelector('ol, ul, b'), null, 'a leading "1." is text, and raw HTML is never drawn');
  assert.match(step('s3').textContent, /1\. Not a list:/);

  const note = v.host.querySelector('section > div:last-child li');
  assert.equal(note.querySelector('em')?.textContent, 'cold');
  assert.equal(note.querySelector('a')?.getAttribute('href'), 'https://example.com/notes');

  // Scaling changes only the amount; the link in the same line stays a link.
  await click(btn(v.host, 'More servings'));
  await click(btn(v.host, 'More servings'));
  const sugar = ing('i2');
  assert.equal(sugar.querySelector('[data-scaled]').textContent, '1½ cups');
  assert.equal(sugar.querySelector('a')?.getAttribute('href'), 'https://example.com/sugar');
  assert.equal(seen(sugar), '1½ cups sugar, sifted');
  assert.equal(seen(ing('i3')), '1½ onions, diced', 'a line that starts with a comma gets no space');

  // Clicking the link opens it and does not tick the row.
  sugar.querySelector('a').addEventListener('click', (e) => e.preventDefault());
  await click(sugar.querySelector('a'));
  assert.equal(sugar.closest('label').querySelector('input').checked, false);
});

test('F-1: the two copy actions stay plain text', () => {
  const block = {
    ...stew(),
    serves: 2,
    ingredients: [
      { id: 'i1', qty: null, text: '**2 cups** flour', group: null },
      { id: 'i2', qty: '1 cup', text: '[sugar](https://example.com/sugar)', group: null },
    ],
    steps: [
      { id: 's1', text: 'Mix it **well**.' },
      { id: 's2', text: 'Bake it; see [the guide](https://example.com/bake) and `200 C`.' },
    ],
    notes: [],
  };
  const md = m.recipeMarkdown(block, 4);
  assert.match(md, /- 2 cups flour\n- 2 cups sugar\n/);
  assert.match(md, /1\. Mix it well\.\n2\. Bake it; see the guide and 200 C\./);
  assert.doesNotMatch(md, /\*\*|\]\(|`/);
  assert.equal(m.shoppingListText(block, new Set(), 4), 'Shopping list: ' + (block.title ?? 'Recipe') + ' (serves 4)\n- 2 cups flour\n- 2 cups sugar\n');
});

test('F-6: two columns from a 700 px card', () => {
  assert.equal(m.TWO_COLUMNS_MIN, 700);
});

test('no serves: no stepper and nothing scales; serves with nothing to scale: a plain chip', async () => {
  const none = await mount(card(m.validRecipe(m.RECIPE_EDGES.noServes.results.blocks[0])));
  assert.equal(none.host.querySelector('[role="group"][aria-label="Servings"]'), null);
  assert.equal(none.host.querySelector('[data-serves-chip]'), null);
  assert.ok(none.host.textContent.includes('Total'), 'the time chip stays');
  const flat = await mount(card(m.validRecipe(m.RECIPE_EDGES.nothingScales.results.blocks[0]), 'm2'));
  assert.equal(flat.host.querySelector('[aria-label="Servings"]'), null);
  assert.equal(flat.host.querySelector('[data-serves-chip]').textContent.replace(/\s+/g, ' ').trim(), 'Serves 4');
  const bare = await mount(card(m.validRecipe(m.RECIPE_EDGES.bare.results.blocks[0]), 'm3'));
  assert.equal(bare.host.querySelector('h3').nextElementSibling.className.includes('border-t'), true, 'E4: the name, then the divider');
});

// ---------------------------------------------------------------------------------------------------
// Ticks, steps and storage (design §5, §7)

test('ticks, done steps and servings persist per message under one key, by id', async () => {
  const block = stew();
  const v = await mount(card(block));
  const boxes = () => [...v.host.querySelectorAll('[data-ingredient]')].map((el) => el.closest('label').querySelector('input'));
  await click(boxes()[0]);
  await click(boxes()[4]);
  await click(btn(v.host, 'More servings'));
  await click(v.host.querySelector('[data-step="s1"] input'));
  const key = m.recipeStorageKey('m1', 'b-recipe');
  assert.equal(key, 'kwnt:recipe:m1:b-recipe');
  assert.deepEqual(JSON.parse(localStorage.getItem(key)), { have: ['i1', 'i5'], done: ['s1'], serves: 5 });
  assert.equal(v.host.textContent.includes('2 of 14'), true, 'the ingredient count');
  v.done();
  mounted.delete(v);

  const again = await mount(card(block));
  assert.equal(again.host.querySelector('output').textContent, '5', 'a reload keeps the servings');
  assert.equal(again.host.querySelector('[data-ingredient="i1"]').closest('label').querySelector('input').checked, true);
  assert.match(again.host.querySelector('[data-ingredient="i1"]').className, /line-through/);
  assert.equal(again.host.querySelector('[data-step="s1"]').dataset.state, 'done');

  const other = await mount(card(block, 'm2'));
  assert.equal(other.host.querySelector('output').textContent, '4', 'another message has its own key');

  localStorage.setItem(key, JSON.stringify({ have: ['gone', 'i2'], done: [3], serves: 99 }));
  const st = m.readRecipeState('m1', block);
  assert.deepEqual([...st.have], ['i2'], 'unknown ids are dropped');
  assert.deepEqual([...st.done], []);
  assert.equal(st.serves, 4, 'an out-of-range number falls back to the recipe’s');
  localStorage.setItem(key, '{not json');
  assert.equal(m.readRecipeState('m1', block).have.size, 0, 'a broken entry reads as fresh');

  m.saveRecipeState('m1', block, { serves: 4, have: new Set(), done: new Set() });
  assert.equal(localStorage.getItem(key), null, 'nothing kept when it is back to the reply');
});

test('method: next appears only after the first done step, out of order picks the lowest; all done in teal; no line-through', async () => {
  const v = await mount(card());
  const step = (id) => v.host.querySelector(`[data-step="${id}"]`);
  assert.equal(v.host.querySelector('[data-state="next"]'), null, 'nothing highlighted at rest');
  assert.ok(v.host.textContent.includes('7 steps'));
  await click(step('s1').querySelector('input'));
  await click(step('s3').querySelector('input'));
  assert.equal(v.host.querySelector('[data-state="next"]').dataset.step, 's2', 'out of order: the lowest not done');
  assert.ok(step('s2').textContent.includes('(next)'));
  assert.ok(v.host.textContent.includes('2 of 7 done'));
  assert.doesNotMatch(step('s1').innerHTML, /line-through/, 'done steps dim, no strike');
  assert.match(step('s3').querySelector('[aria-hidden]').className, /motion-safe:animate-\[kw-step-done/, 'the step just ticked pops, reduced motion skips it');
  for (const id of ['s2', 's4', 's5', 's6', 's7']) await click(step(id).querySelector('input'));
  assert.equal(v.host.querySelector('[data-state="next"]'), null, 'no next once all are done');
  assert.ok(v.host.textContent.includes('All 7 done'));
  assert.match(v.host.querySelector('.text-ok').className, /text-ok/);
});

// ---------------------------------------------------------------------------------------------------
// Fold (design §4)

test('fold: under 640 px with more than 10 ingredients, 8 show behind "Show all N"; ticks on hidden rows count', async () => {
  phone = true;
  const block = stew();
  const v = await mount(card(block));
  assert.equal(v.host.querySelectorAll('[data-ingredient]').length, m.FOLD_SHOW);
  const more = byText(v.host, 'Show all 14');
  assert.ok(more, 'Show all 14');
  assert.equal(more.getAttribute('aria-expanded'), 'false');
  assert.ok(document.getElementById(more.getAttribute('aria-controls')));
  // Counting runs across groups: 8 of "For the stew" (9 rows) show, so "For the topping" is hidden.
  assert.ok(v.host.textContent.includes('For the stew'));
  assert.equal(v.host.textContent.includes('For the topping'), false, 'a group label shows only with a visible row');
  await click(more);
  assert.equal(v.host.querySelectorAll('[data-ingredient]').length, 14);
  await click(v.host.querySelector('[data-ingredient="i14"]').closest('label').querySelector('input'));
  const fewer = byText(v.host, 'Show fewer');
  fewer.focus();
  await click(fewer);
  assert.equal(document.activeElement, byText(v.host, 'Show all 14'), 'focus stays on the button');
  assert.ok(v.host.textContent.includes('1 of 14'), 'the count covers hidden rows');
  assert.equal(sessionStorage.getItem('kwnt:recipe-open:m1:b-recipe'), null);
});

test('fold: never at 10 or fewer, and never on a wide viewport', async () => {
  phone = true;
  const ten = m.validRecipe(m.recipeFixture({}, Array.from({ length: 10 }, (_, i) => [`${i + 1} g`, `thing ${i + 1}`])).results.blocks[0]);
  const v = await mount(card(ten));
  assert.equal(v.host.querySelectorAll('[data-ingredient]').length, 10);
  assert.equal(byText(v.host, 'Show all 10'), undefined);
  phone = false;
  const w = await mount(card(stew(), 'm9'));
  assert.equal(w.host.querySelectorAll('[data-ingredient]').length, 14);
});

// ---------------------------------------------------------------------------------------------------
// Actions (design §6)

test('Copy recipe: markdown at the shown servings, groups bold, method numbered, notes; no ticks', async () => {
  const block = stew();
  localStorage.setItem(m.recipeStorageKey('m1', block.id), JSON.stringify({ serves: 6, have: ['i1'], done: ['s1'] }));
  const v = await mount(h(m.RecipeActions, { block, messageId: 'm1' }));
  await click(v.host.querySelector('[data-recipe-copy="recipe"]'));
  const t = clipboard.text;
  assert.ok(t.startsWith('## Golden squash and lentil stew\nServes 6 · Prep 15 min · Cook 35 min\n\n### Ingredients\n**For the stew**\n- 1½ tbsp olive oil\n- 1½ onions, diced\n'));
  assert.ok(t.includes('\n\n**For the topping**\n- ¾ cup plain yogurt\n'));
  assert.ok(t.includes('- 750 g squash, peeled and cubed'));
  assert.ok(t.includes('- Salt to taste'));
  assert.ok(t.includes('### Method\n1. Warm the oil'));
  assert.ok(t.includes('7. Serve with'));
  assert.ok(t.endsWith('### Notes\n- Keeps 3 days in the fridge. It thickens, so add a splash of stock when you reheat it.\n- Sweet potato works in place of the squash.\n- Leave out the yogurt for a vegan version.\n'));
  assert.equal(t.includes('[x]'), false);
  assert.ok(v.host.textContent.includes('Copied'));
  assert.ok(v.host.textContent.includes('Copied the recipe for 6 servings'), 'the screen reader line');
  assert.equal(m.blockMarkdown(block).includes('Serves 4'), true, 'Copy data’s Markdown is the reply’s own number');
});

test('Copy shopping list: the unticked ingredients, scaled, no groups; the count follows ticks; 0 left is disabled', async () => {
  const block = stew();
  const v = await mount(h('div', null, card(block), h(m.RecipeActions, { block, messageId: 'm1' })));
  const chip = () => v.host.querySelector('[data-recipe-copy="shopping"]');
  assert.equal(chip().getAttribute('aria-label'), 'Copy shopping list, 14 items');
  await click(btn(v.host, 'More servings'));
  await click(btn(v.host, 'More servings'));
  for (const id of ['i1', 'i2', 'i9']) await click(v.host.querySelector(`[data-ingredient="${id}"]`).closest('label').querySelector('input'));
  assert.equal(chip().getAttribute('aria-label'), 'Copy shopping list, 11 items', 'the footer count is live');
  await click(chip());
  const lines = clipboard.text.trimEnd().split('\n');
  assert.equal(lines[0], 'Shopping list: Golden squash and lentil stew (serves 6)');
  assert.equal(lines.length, 12);
  assert.equal(lines[1], '- 3 cloves garlic, crushed');
  assert.ok(lines.includes('- 750 g squash, peeled and cubed'));
  assert.ok(lines.includes('- 1½–3 tsp chili flakes'));
  assert.equal(lines.some((l) => /For the/.test(l)), false, 'no groups');
  assert.ok(v.host.textContent.includes('Copied 11 items for 6 servings'));

  for (const ing of block.ingredients) {
    const box = v.host.querySelector(`[data-ingredient="${ing.id}"]`).closest('label').querySelector('input');
    if (!box.checked) await click(box);
  }
  await new Promise((r) => setTimeout(r, 2100));
  await settle();
  assert.equal(chip().getAttribute('aria-disabled'), 'true');
  assert.equal(chip().getAttribute('title'), 'Everything is ticked.');
  clipboard.text = null;
  await click(chip());
  assert.equal(clipboard.text, null, 'clicking does nothing');
});

test('shopping list without serves: amounts as written, no "(serves N)"', () => {
  const b = m.validRecipe(m.RECIPE_EDGES.noServes.results.blocks[0]);
  assert.equal(m.shoppingListText(b, new Set(['i2']), 4), 'Shopping list: Garlic flatbreads\n- 2 cups flour\n- ¾ cup plain yogurt\n- 2 cloves garlic, grated\n- A good pinch of salt\n');
});

// ---------------------------------------------------------------------------------------------------
// validRecipe

test('validRecipe accepts the fixtures and fills defaults', () => {
  for (const [name, f] of Object.entries({ stew: m.recipe, ...m.RECIPE_EDGES })) assert.ok(m.validRecipe(f.results.blocks[0]), name);
  const b = { ...m.recipe.results.blocks[0] };
  delete b.serves;
  delete b.times;
  delete b.notes;
  const v = m.validRecipe(b);
  assert.equal(v.serves, null);
  assert.deepEqual(v.times, []);
  assert.deepEqual(v.notes, []);
  assert.ok(m.validRecipe({ ...m.recipe.results.blocks[0], title: null }), 'no title: the card says Recipe');
});

test('validRecipe rejects anything off the contract', () => {
  const b = m.recipe.results.blocks[0];
  const no = (over, why) => assert.equal(m.validRecipe({ ...b, ...over }), null, why);
  no({ type: 'steps' }, 'type');
  no({ source: { kind: 'markdown_list', index: 0 } }, 'a recipe is always a stretch of the reply');
  no({ source: { kind: 'slot' } }, 'never a slot (contract §2)');
  no({ source: { kind: 'markdown_section', index: 0, last_index: 2, last_ordered: false, lead: 4 } }, 'ordered is required');
  no({ source: { kind: 'markdown_section', first: { index: 0, ordered: false }, last: { index: 2, ordered: false }, lead: 4 } }, 'the flat shape only');
  no({ source: { ...b.source, lead: 13 } }, 'lead 0–12');
  no({ serves: 25 }, 'serves ≤ 24');
  no({ serves: 0 }, 'serves ≥ 1');
  no({ serves: 2.5 }, 'serves is a whole number');
  no({ serves: '4' }, 'serves is a number');
  no({ times: [{ label: 'Prep', text: '1' }, { label: 'Cook', text: '2' }, { label: 'Rest', text: '3' }, { label: 'Total', text: '4' }] }, 'at most 3 times');
  no({ times: [{ label: 'Prep' }] }, 'a time needs text');
  no({ ingredients: b.ingredients.slice(0, 1) }, 'at least 2 ingredients');
  no({ ingredients: Array.from({ length: 41 }, (_, i) => ({ id: `x${i}`, text: 'a', qty: null, group: null })) }, 'at most 40');
  no({ ingredients: [b.ingredients[0], { ...b.ingredients[1], id: b.ingredients[0].id }] }, 'unique ids');
  no({ ingredients: [{ ...b.ingredients[0], text: '<img src=x onerror=1>' }, b.ingredients[1]] }, 'no HTML');
  no({ ingredients: [{ ...b.ingredients[0], text: '' }, b.ingredients[1]] }, 'text is never empty');
  no({ ingredients: [{ ...b.ingredients[0], qty: 'A pinch' }, b.ingredients[1]] }, 'qty starts with a digit or a fraction glyph');
  no({ ingredients: [{ ...b.ingredients[0], id: 'x1' }, b.ingredients[1]] }, 'ingredient ids are i{k}');
  no({ ingredients: [{ ...b.ingredients[0], group: 'g'.repeat(101) }, b.ingredients[1]] }, 'group ≤ 100');
  no({ steps: [{ id: 'step1', text: 'Stir.' }, b.steps[1]] }, 'step ids are s{k}');
  no({ steps: [{ id: 's1', text: 'a'.repeat(601) }, b.steps[1]] }, 'step text ≤ 600');
  no({ ingredients: [{ ...b.ingredients[0], qty: 5 }, b.ingredients[1]] }, 'qty is text');
  no({ steps: b.steps.slice(0, 1) }, 'at least 2 steps');
  no({ steps: Array.from({ length: 31 }, (_, i) => ({ id: `s${i}`, text: 'Stir.' })) }, 'at most 30 steps');
  no({ steps: [{ id: 's1', text: '' }, b.steps[1]] }, 'a blank step');
  no({ notes: Array(7).fill('Tip.') }, 'at most 6 notes');
  no({ notes: [3] }, 'notes are text');
  assert.ok(m.validRecipe({ ...b, steps: [{ id: 's1', text: 'a'.repeat(600) }, b.steps[1]], notes: ['n'.repeat(600)] }), '600 is fine');
});

// ---------------------------------------------------------------------------------------------------
// In the reply (design §1, §7)

test('the card replaces its stretch of the reply once; intro and closing stay; no slot title line; Show as text gives it back', async () => {
  const { reply, results } = m.recipe;
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results, messageId: 'm1' }));
  const t = v.host.textContent;
  assert.ok(v.host.querySelector('[data-smart-result="recipe"]'));
  assert.ok(t.startsWith('Here is a cosy one-pot dinner for tonight.'));
  assert.ok(t.includes('Want a quick flatbread to go with it?'));
  assert.equal(t.split('Golden squash and lentil stew').length - 1, 1, 'the dish name shows once');
  assert.equal(t.split('Sweet potato works').length - 1, 1, 'notes once');
  assert.equal(v.host.querySelectorAll('h2').length, 0, 'the reply’s heading went into the card');
  assert.equal(v.host.querySelector('[data-recipe-copy="recipe"]') !== null, true);
  assert.equal([...v.host.querySelectorAll('button')].some((b) => b.textContent.includes('Copy data')), false, 'Copy data is not shown for a recipe');
  const asText = [...v.host.querySelectorAll('button')].find((b) => b.textContent.includes('Show as text'));
  await click(asText);
  assert.equal(v.host.querySelector('[data-smart-result="recipe"]'), null);
  assert.equal(v.host.querySelectorAll('h2').length, 1, 'the stretch as written');
  assert.ok(v.host.textContent.includes('1 tbsp olive oil'));
  localStorage.clear();
});

test('a section whose lists are not where it says, or an invalid recipe, leaves the reply as text', async () => {
  const { reply, results } = m.recipe;
  const blk = results.blocks[0];
  const off = { version: 1, blocks: [{ ...blk, source: { ...blk.source, index: 7 } }] };
  const a = await mount(h(m.MarkdownRenderer, { content: reply, results: off, messageId: 'm1' }));
  assert.equal(a.host.querySelector('[data-smart-result="recipe"]'), null);
  assert.equal(a.host.querySelectorAll('h2').length, 1);
  const bad = { version: 1, blocks: [{ ...blk, steps: [] }] };
  const b = await mount(h(m.MarkdownRenderer, { content: reply, results: bad, messageId: 'm2' }));
  assert.equal(b.host.querySelector('[data-smart-result="recipe"]'), null);
  assert.ok(b.host.textContent.includes('Warm the oil'), 'the reply’s own method');
  assert.equal(b.host.querySelectorAll('h2').length, 1);
});

test('edge states render: long name wraps, 30 steps never fold and badges reach 30', async () => {
  const long = await mount(card(m.validRecipe(m.RECIPE_EDGES.longName.results.blocks[0])));
  assert.match(long.host.querySelector('h3').className, /overflow-wrap:anywhere/);
  const thirty = await mount(card(m.validRecipe(m.RECIPE_EDGES.thirty.results.blocks[0]), 'm30'));
  assert.equal(thirty.host.querySelectorAll('[data-step]').length, 30);
  assert.ok(thirty.host.querySelector('[data-step="s30"]').textContent.includes('30'));
  assert.ok(thirty.host.textContent.includes('30 steps'));
});

test('contract.md §3 exactly: the card sits at root child 1 and hides 1–11; intro and question stay', async () => {
  const { reply, results } = m.RECIPE_CONTRACT;
  const blk = m.validRecipe(results.blocks[0]);
  assert.ok(blk, 'the server’s example passes validRecipe');
  const v = await mount(h(m.MarkdownRenderer, { content: reply, results, messageId: 'mc' }));
  const t = v.host.textContent;
  assert.ok(t.startsWith("Here's a cosy stew for tonight."), 'the intro stays, before the card');
  assert.ok(t.endsWith('Want a vegan version?'), 'the question stays, after it');
  assert.ok(v.host.querySelector('[data-smart-result="recipe"]'));
  assert.equal(v.host.querySelectorAll('h2').length, 0, 'root children 1–11 are not drawn');
  assert.deepEqual([...v.host.querySelectorAll('h3')].map((e) => e.textContent), ['Golden squash and lentil stew'], 'only the card’s name');
  assert.equal(t.split('Keeps 3 days').length - 1, 1, 'notes once');
  assert.deepEqual(amounts(v.host), ['1 tbsp olive oil', '1 onion, diced', '2 cloves garlic, crushed', '500 g squash, peeled and cubed', '1 can (400 g) chopped tomatoes', 'Salt to taste', '½ cup plain yogurt', '1–2 tsp chili flakes'], 'qty + text is each reply line');
  assert.ok(v.host.textContent.includes('For the topping'));
  assert.equal(v.host.querySelectorAll('[data-step]').length, 4);
  await click(btn(v.host, 'More servings'));
  await click(btn(v.host, 'More servings'));
  assert.deepEqual(amounts(v.host).slice(0, 2), ['1½ tbsp olive oil', '1½ onions, diced']);
});
