// Recipe fixtures (design.md §9): the made-up "Golden squash and lentil stew" from Mira's mocks and its
// edge states. Every value is the reply's text; qty + text is the reply's line (§11).

import type { RecipeBlock, RecipeIngredient } from '../../smartResults';
import type { Fixture } from './fixtures';

type Line = [qty: string | null, text: string, group?: string];

const ing = (lines: Line[]): RecipeIngredient[] => lines.map(([qty, text, group], i) => ({ id: `i${i + 1}`, qty, text, group: group ?? null }));
const steps = (texts: string[]) => texts.map((text, i) => ({ id: `s${i + 1}`, text }));
const join = (qty: string | null, text: string) => (!qty ? text : text.startsWith(',') ? qty + text : `${qty} ${text}`);

const STEW_LINES: Line[] = [
  ['1 tbsp', 'olive oil', 'For the stew'],
  ['1 onion', ', diced', 'For the stew'],
  ['2 cloves', 'garlic, crushed', 'For the stew'],
  ['2 tsp', 'smoked paprika', 'For the stew'],
  ['500 g', 'squash, peeled and cubed', 'For the stew'],
  ['1 cup', 'red lentils, rinsed', 'For the stew'],
  ['1 can', '(400 g) chopped tomatoes', 'For the stew'],
  ['3 cups', 'vegetable stock', 'For the stew'],
  [null, 'Salt to taste', 'For the stew'],
  ['½ cup', 'plain yogurt', 'For the topping'],
  ['1 lemon', ', zested', 'For the topping'],
  ['2 tbsp', 'chopped parsley', 'For the topping'],
  ['1–2 tsp', 'chili flakes', 'For the topping'],
  [null, 'A pinch of flaky salt', 'For the topping'],
];
const STEW_STEPS = [
  'Warm the oil in a large pot over medium heat. Add the onion and cook for 5 minutes, until soft.',
  'Stir in the garlic and paprika and cook for 1 minute.',
  'Add the squash, lentils, tomatoes and stock, and bring to a boil.',
  'Lower the heat and simmer for 25 minutes, stirring now and then, until the lentils break down.',
  'Meanwhile, mix the yogurt with the lemon zest.',
  'Taste the stew and add salt.',
  'Serve with a spoon of the lemon yogurt, the parsley, the chili flakes and a pinch of flaky salt.',
];
const STEW_NOTES = [
  'Keeps 3 days in the fridge. It thickens, so add a splash of stock when you reheat it.',
  'Sweet potato works in place of the squash.',
  'Leave out the yogurt for a vegan version.',
];

/** The reply as the Chef bot wrote it, and its block. Lists: ul 0 (stew), ul 1 (topping), ol 0 (method), ul 2 (notes). */
export function recipeFixture(over: Partial<RecipeBlock> = {}, lines: Line[] = STEW_LINES): Fixture {
  const block: RecipeBlock = {
    id: 'b-recipe',
    type: 'recipe',
    title: 'Golden squash and lentil stew',
    source: { kind: 'markdown_section', index: 0, ordered: false, last_index: 2, last_ordered: false, lead: 4 },
    provenance: { kind: 'reply_text' },
    serves: 4,
    times: [
      { label: 'Prep', text: '15 min' },
      { label: 'Cook', text: '35 min' },
    ],
    ingredients: ing(lines),
    steps: steps(STEW_STEPS),
    notes: STEW_NOTES,
    ...over,
  };
  const groups: string[] = [];
  for (const i of block.ingredients) if (i.group && !groups.includes(i.group)) groups.push(i.group);
  const loose = block.ingredients.filter((i) => !i.group);
  const list = (xs: RecipeIngredient[]) => xs.map((i) => `- ${join(i.qty, i.text)}`).join('\n');
  const head = [block.serves ? `Serves ${block.serves}` : '', ...block.times.map((t) => `${t.label} ${t.text}`)].filter(Boolean).join(' · ');
  const reply = [
    'Here is a cosy one-pot dinner for tonight.',
    `## ${block.title ?? 'Recipe'}`,
    ...(head ? [head] : []),
    '### Ingredients',
    ...(loose.length ? [list(loose)] : []),
    ...groups.flatMap((g) => [`**${g}**`, list(block.ingredients.filter((i) => i.group === g))]),
    '### Method',
    block.steps.map((s, i) => `${i + 1}. ${s.text}`).join('\n'),
    ...(block.notes.length ? ['### Notes', block.notes.map((n) => `- ${n}`).join('\n')] : []),
    'Want a quick flatbread to go with it?',
  ].join('\n\n');
  // Before the first list: the name, the head line, "Ingredients" and, when the list opens with a group, its label.
  const lead = 2 + (head ? 1 : 0) + (!loose.length && groups.length ? 1 : 0);
  const uls = (loose.length ? 1 : 0) + groups.length + (block.notes.length ? 1 : 0);
  const first = loose.length || groups.length ? { index: 0, ordered: false } : { index: 0, ordered: true };
  const last = block.notes.length ? { index: uls - 1, ordered: false } : { index: 0, ordered: true };
  const source = { kind: 'markdown_section' as const, index: first.index, ordered: first.ordered, last_index: last.index, last_ordered: last.ordered, lead };
  return { reply, results: { version: 1, blocks: [{ ...block, source }] } };
}

export const recipe = recipeFixture();

/** E1–E6 and the fold (design §9). */
export const RECIPE_EDGES = {
  /** E1: a long name, three times, groups. */
  longName: recipeFixture({
    title: 'Slow-roasted golden squash, red lentil and smoked paprika stew with a bright lemon and herb yogurt',
    times: [
      { label: 'Prep', text: '15 min' },
      { label: 'Cook', text: '35 min' },
      { label: 'Total', text: '50 min' },
    ],
  }),
  /** E2: no times. */
  noTimes: recipeFixture({ title: 'Lemon yogurt dip', times: [], notes: [] }, [
    ['½ cup', 'plain yogurt'],
    ['1 lemon', ', zested'],
    ['2 tbsp', 'chopped parsley'],
    [null, 'Salt to taste'],
  ]),
  /** E3: no serves. */
  noServes: recipeFixture({ title: 'Garlic flatbreads', serves: null, times: [{ label: 'Total', text: 'about 30 min' }], notes: [] }, [
    ['2 cups', 'flour'],
    ['1 tsp', 'baking powder'],
    ['¾ cup', 'plain yogurt'],
    ['2 cloves', 'garlic, grated'],
    [null, 'A good pinch of salt'],
  ]),
  /** E3b: serves, but nothing can scale. */
  nothingScales: recipeFixture({ title: 'Green salad', notes: [] }, [
    [null, 'A head of lettuce'],
    [null, 'A handful of soft herbs'],
    [null, 'Olive oil and lemon to taste'],
  ]),
  /** E4: no serves, no times, no notes. */
  bare: recipeFixture({ title: 'Herb oil', serves: null, times: [], notes: [], steps: steps(['Blend the herbs with the oil.', 'Strain through a fine sieve.']) }, [
    ['½ cup', 'olive oil'],
    [null, 'A handful of soft herbs'],
  ]),
  /** E6: 30 steps. */
  thirty: recipeFixture({
    title: 'Layered vegetable bake',
    times: [
      { label: 'Prep', text: '40 min' },
      { label: 'Cook', text: '1 h 10 min' },
      { label: 'Total', text: '1 h 50 min' },
    ],
    notes: [],
    steps: steps(Array.from({ length: 30 }, (_, i) => `Step text for part ${i + 1} of the bake, kept short in this sheet.`)),
  }),
} satisfies Record<string, Fixture>;

/** contract.md §3 exactly: Vega's backend unit test (recipe_tests::the_contract_example_is_exact). */
export const RECIPE_CONTRACT: Fixture = {
  reply: `Here's a cosy stew for tonight.

## Golden squash and lentil stew

**Serves 4** · Prep 15 min · Cook 35 min

### Ingredients

**For the stew**
- 1 tbsp olive oil
- 1 onion, diced
- 2 cloves garlic, crushed
- 500 g squash, peeled and cubed
- 1 can (400 g) chopped tomatoes
- Salt to taste

**For the topping:**
- ½ cup plain yogurt
- 1–2 tsp chili flakes

### Method

1. Warm the oil in a large pot over medium heat. Add the onion and cook for 5 minutes, until soft.
2. Stir in the garlic and cook for 1 minute.
3. Add the squash and tomatoes and simmer for 25 minutes.
4. Serve with the yogurt and chili flakes.

### Notes

- Keeps 3 days in the fridge.
- Sweet potato works in place of the squash.

Want a vegan version?
`,
  results: {
    version: 1,
    blocks: [
      {
        type: 'recipe',
        id: 'b0',
        title: 'Golden squash and lentil stew',
        source: { kind: 'markdown_section', index: 0, ordered: false, last_index: 2, last_ordered: false, lead: 4 },
        serves: 4,
        times: [
          { label: 'Prep', text: '15 min' },
          { label: 'Cook', text: '35 min' },
        ],
        ingredients: [
          { id: 'i0', text: 'olive oil', qty: '1 tbsp', group: 'For the stew' },
          { id: 'i1', text: ', diced', qty: '1 onion', group: 'For the stew' },
          { id: 'i2', text: 'garlic, crushed', qty: '2 cloves', group: 'For the stew' },
          { id: 'i3', text: 'squash, peeled and cubed', qty: '500 g', group: 'For the stew' },
          { id: 'i4', text: '(400 g) chopped tomatoes', qty: '1 can', group: 'For the stew' },
          { id: 'i5', text: 'Salt to taste', qty: null, group: 'For the stew' },
          { id: 'i6', text: 'plain yogurt', qty: '½ cup', group: 'For the topping' },
          { id: 'i7', text: 'chili flakes', qty: '1–2 tsp', group: 'For the topping' },
        ],
        steps: [
          { id: 's0', text: 'Warm the oil in a large pot over medium heat. Add the onion and cook for 5 minutes, until soft.' },
          { id: 's1', text: 'Stir in the garlic and cook for 1 minute.' },
          { id: 's2', text: 'Add the squash and tomatoes and simmer for 25 minutes.' },
          { id: 's3', text: 'Serve with the yogurt and chili flakes.' },
        ],
        notes: ['Keeps 3 days in the fridge.', 'Sweet potato works in place of the squash.'],
        provenance: { kind: 'reply_text' },
      },
    ],
  },
};
