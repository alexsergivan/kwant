import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const { suggestedPrompts, roleOf, MAX_PROMPTS, MAX_TEAM_PROMPTS } = await loadBundled(`export * from './src/suggestedPrompts';`);

const DEV_PROMPTS = ['Analyze system architecture tradeoffs', 'Write a fast Python script for data processing', 'Review git status and security policies'];
const kitchen = [
  { name: 'HeadChef', title: 'Head Chef' },
  { name: 'Nutritionist', title: 'Nutritionist' },
  { name: 'Sous', title: 'Sous Chef' },
  { name: 'Pastry', title: 'Pastry Chef' },
  { name: 'Grocer', title: 'Grocery Planner' },
];

test("a kitchen team gets the lead's and two teammates' cooking asks, never developer prompts", () => {
  // What Oleks saw: a freshly hired Kitchen team offered "Analyze system architecture tradeoffs".
  const out = suggestedPrompts({ kind: 'team', members: kitchen, lead: 'HeadChef', purpose: 'Hired together: a team of 5.' });
  assert.deepEqual(out, [
    "@HeadChef plan this week's dinners",
    "@Nutritionist check this week's meals are balanced",
    '@Sous write a prep list for tonight',
    'What can each of you help me with?',
  ]);
  for (const p of DEV_PROMPTS) assert.ok(!out.includes(p));
});

test('the lead goes first even when listed last, and two bots with one role get one ask', () => {
  const out = suggestedPrompts({
    kind: 'team',
    members: [{ name: 'Cook2', title: 'Chef' }, { name: 'Cook1', title: 'Chef de cuisine' }, { name: 'Boss', title: 'Head Chef' }],
    lead: 'Boss',
  });
  assert.deepEqual(out, ["@Boss plan this week's dinners", 'What can each of you help me with?']);
});

test('an engineering team still gets engineering asks', () => {
  const out = suggestedPrompts({
    kind: 'team',
    members: [
      { name: 'Architect', title: 'Systems Architect' },
      { name: 'TechLead', title: 'Staff Engineer & Prototyper' },
      { name: 'DevOps', title: 'Systems & Security Engineer' },
    ],
    lead: 'Architect',
  });
  assert.deepEqual(out, [
    '@Architect sketch the architecture for a new feature',
    '@TechLead build a quick prototype',
    "@DevOps check my server's security",
    'What can each of you help me with?',
  ]);
});

test("unknown titles fall back to the team's purpose for the lead, else a neutral opener", () => {
  const members = [{ name: 'Ann', title: 'Wizard' }, { name: 'Bo', title: 'Sidekick' }];
  assert.deepEqual(suggestedPrompts({ kind: 'team', members, lead: 'Ann', purpose: 'Plans our weekly menu' }), [
    "@Ann plan this week's dinners",
    'What can each of you help me with?',
  ]);
  assert.deepEqual(suggestedPrompts({ kind: 'team', members, lead: 'Bo' }), ['@Bo what should we tackle first?', 'What can each of you help me with?']);
  assert.deepEqual(suggestedPrompts({ kind: 'team', members: [members[0]] }), ['@Ann what should we tackle first?', 'What can you help me with?']);
  assert.deepEqual(suggestedPrompts({ kind: 'team', members: [] }), []);
});

test('names that cannot be @mentioned are not used in a mention', () => {
  const out = suggestedPrompts({ kind: 'team', members: [{ name: 'Head Chef', title: 'Head Chef' }, { name: 'Sous', title: 'Sous Chef' }], lead: 'Head Chef' });
  assert.deepEqual(out, ['@Sous write a prep list for tonight', 'What can each of you help me with?']);
});

test("a one-to-one chat uses that bot's role, or its persona when the title says nothing", () => {
  assert.deepEqual(suggestedPrompts({ kind: 'bot', bot: { name: 'Nutritionist', title: 'Nutritionist' } }), [
    "Check this week's meals are balanced",
    'Make a high-protein meal plan',
    'What can you help me with?',
  ]);
  assert.equal(suggestedPrompts({ kind: 'bot', bot: { name: 'X', title: 'Helper', persona_prompt: 'You are a seasoned SEO strategist.' } })[0], 'Find keywords I could rank for');
  assert.deepEqual(suggestedPrompts({ kind: 'bot', bot: { name: 'X', title: 'Helper' } }), ['What can you help me with?']);
});

test("#general draws on the space's bots without naming them, and is empty with no bots", () => {
  const out = suggestedPrompts({ kind: 'general', bots: [{ name: 'DevOps', title: 'Systems & Security Engineer' }, ...kitchen] });
  assert.deepEqual(out, ["Check my server's security", "Plan this week's dinners", 'Who here can help me with what?']);
  assert.ok(out.every((p) => !p.includes('@')));
  assert.deepEqual(suggestedPrompts({ kind: 'general', bots: [] }), []);
});

test('never more than three (four in a team), each short enough for a phone chip', () => {
  const many = Array.from({ length: 30 }, (_, i) => ({ name: `B${i}`, title: ['Chef', 'Writer', 'Lawyer', 'Tutor', 'SEO lead'][i % 5] }));
  for (const ctx of [{ kind: 'team', members: many }, { kind: 'general', bots: many }, { kind: 'bot', bot: many[0] }]) {
    const out = suggestedPrompts(ctx);
    assert.ok(out.length <= (ctx.kind === 'team' ? MAX_TEAM_PROMPTS : MAX_PROMPTS), JSON.stringify(out));
    assert.ok(out.every((p) => p.length <= 50), JSON.stringify(out));
  }
});

test('specific roles win over general ones', () => {
  assert.equal(roleOf({ name: 'a', title: 'Sous Chef' }).asks[0], 'write a prep list for tonight');
  assert.equal(roleOf({ name: 'a', title: 'DevOps Engineer' }).asks[0], "check my server's security");
  assert.equal(roleOf({ name: 'a', title: 'QA and Release Engineer' }).asks[0], 'write a test plan for the next release');
  assert.equal(roleOf({ name: 'a', title: 'Frontend Engineer' }).asks[0], 'review the sign-up screen');
  assert.equal(roleOf({ name: 'a', title: 'Wizard' }), null);
});

test('the chat no longer hardcodes developer prompts', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  for (const p of DEV_PROMPTS) assert.ok(!src.includes(p), p);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /suggestedPrompts\(/);
});

// Rigel's review of #61: one qualifier word used to pick the field, so these got another trade's prompts.
const askOf = (title) => roleOf({ name: 'a', title })?.asks[0] ?? null;

test('a qualifier word does not pick the field: the job at the end of the title does', () => {
  assert.equal(askOf('Product Photographer'), 'describe a cover image idea');
  assert.equal(askOf('Interior Designer'), 'suggest a layout for my living room');
  assert.equal(askOf('Press Release Writer'), 'outline a blog post');
  assert.equal(askOf('Life Coach'), 'help me set goals for this month');
  // ...while the phrases that do name those fields still work.
  assert.equal(askOf('Product Manager'), 'turn this idea into a short spec');
  assert.equal(askOf('UI/UX Designer'), 'review the sign-up screen');
  assert.equal(askOf('Release Engineer'), 'write a test plan for the next release');
  assert.equal(askOf('Fitness Coach'), 'plan my workouts for the week');
  assert.equal(askOf('Platform Engineer'), "check my server's security");
});

test("a storytelling platform's purpose gets story prompts, not server security", () => {
  const out = suggestedPrompts({ kind: 'team', members: [{ name: 'Ann', title: 'Wizard' }, { name: 'Bo', title: 'Sidekick' }], lead: 'Ann', purpose: 'Platform for Kids Storytelling' });
  assert.deepEqual(out, ['@Ann write a bedtime story', 'What can each of you help me with?']);
  assert.equal(askOf('Platform for Kids Storytelling'), 'write a bedtime story');
});

test('the five titles from the review, as one team, each get their own field', () => {
  const out = suggestedPrompts({
    kind: 'team',
    members: [
      { name: 'Snap', title: 'Product Photographer' },
      { name: 'Deco', title: 'Interior Designer' },
      { name: 'Press', title: 'Press Release Writer' },
      { name: 'Guide', title: 'Life Coach' },
    ],
    lead: 'Snap',
  });
  assert.deepEqual(out, [
    '@Snap describe a cover image idea',
    '@Deco suggest a layout for my living room',
    '@Press outline a blog post',
    'What can each of you help me with?',
  ]);
  for (const p of out) assert.doesNotMatch(p, /spec|sign-up|test plan|workout|server/);
});

test('a member or bot with a missing name or title is skipped, not a crash', () => {
  const members = [{ name: undefined, title: 'Chef' }, { name: '  ', title: 'Chef' }, null, { name: 'Sous', title: undefined, persona_prompt: 42 }, { name: 'Nut', title: 'Nutritionist' }];
  assert.deepEqual(suggestedPrompts({ kind: 'team', members, lead: 'Nut' }), ["@Nut check this week's meals are balanced", 'What can each of you help me with?']);
  assert.deepEqual(suggestedPrompts({ kind: 'general', bots: [{ title: 'Chef' }, { name: 'Nut', title: 'Nutritionist' }] }), [
    "Check this week's meals are balanced",
    'Who here can help me with what?',
  ]);
  assert.deepEqual(suggestedPrompts({ kind: 'team', members: [{ title: 'Chef' }] }), []);
  assert.deepEqual(suggestedPrompts({ kind: 'bot', bot: {} }), ['What can you help me with?']);
});
