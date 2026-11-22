// 33r general-hero-shows-working: #general's and a team's faces put who needs you first, then who is working,
// then everyone else (stable), and those chips say their state in their accessible text.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { BotHome } from './src/components/BotHome';
  export { heroOrder, heroState } from './src/botHome';
  export { setMoodNeedsYou, moodTurnStarted, moodOutput, clearMoods } from './src/kwantMood';
  export { turnStarted, clearAllTurns } from './src/kwantActivity';
  export const render = (props) => renderToStaticMarkup(React.createElement(BotHome, { prompts: [], onPick: () => {}, ...props }));
`);

const names = ['Ada', 'Lin', 'Pixel', 'Rigel', 'Mira', 'Iris', 'Juno', 'Vega', 'Kit', 'Radar', 'Scout', 'Luna'];
const team = names.map((n) => ({ id: n.toLowerCase(), name: n, title: `${n}'s job`, avatar_sprite: 'default' }));
const general = { kind: 'general' };

const reset = () => { m.clearMoods(); m.clearAllTurns(); };
// Each chip from its opening tag to the next chip (or the "+N more" chip, or the end of the row).
const row = (html) => { const a = html.indexOf('role="list"'); return html.slice(a, html.indexOf('</span></div>', a)); };
const chips = (html) => row(html).split('<span role="listitem"').slice(1).filter((c) => c.startsWith(' class="kw-chip pl-1"')).map((c) => '<span role="listitem"' + c);
// The name is the text right after the face's wrapper (the avatar <div> and an optional empty dot <span>).
const chipNames = (html) => chips(html).map((c) => c.match(/<\/div>(?:<span[^>]*><\/span>)?<\/span>([^<]+)/)[1]);
const faces = (html) => [...html.slice(0, html.indexOf('<h2')).matchAll(/title="([^"]+)"/g)].map((x) => x[1].split(" ")[0]);

test('order: needs you, then working/thinking, then the rest, each group stable', () => {
  const moods = { kit: 'working', scout: 'needs_you', lin: 'thinking', radar: 'needs_you', mira: 'failed', iris: 'resting', juno: 'finished' };
  const got = m.heroOrder(team, (id) => moods[id]).map((k) => k.name);
  assert.deepEqual(got, ['Radar', 'Scout', 'Lin', 'Kit', 'Ada', 'Pixel', 'Rigel', 'Mira', 'Iris', 'Juno', 'Vega', 'Luna']);
  assert.deepEqual(m.heroOrder(team, () => 'idle').map((k) => k.name), names, 'a quiet space keeps the list order');
  assert.equal(m.heroState('thinking'), 'working');
  assert.equal(m.heroState('needs_you'), 'needs you');
  for (const q of ['idle', 'failed', 'resting', 'finished', undefined]) assert.equal(m.heroState(q), null);
});

test('quiet space renders exactly as before: list order, no marks, no state words, +4 more', () => {
  reset();
  const html = m.render({ home: general, team });
  assert.deepEqual(chipNames(html), names.slice(0, 8));
  assert.ok(!html.includes('data-hero-state') && !html.includes('data-working-dot') && !html.includes('data-mood-dot') && !html.includes('sr-only'));
  assert.match(html, />\+4 more</);
  assert.deepEqual(faces(html), ['Ada', 'Lin', 'Pixel']);
});

test('busy space: Kit (working, 9th) and Scout (needs you, 11th) come into the first chips with their dot and words', () => {
  reset();
  m.turnStarted('kit'); m.moodTurnStarted('kit'); m.moodOutput('kit');
  m.setMoodNeedsYou(['scout']);
  const html = m.render({ home: general, team });
  assert.deepEqual(chipNames(html), ['Scout', 'Kit', 'Ada', 'Lin', 'Pixel', 'Rigel', 'Mira', 'Iris']);
  assert.match(html, />\+4 more</, 'the count stays right');
  const [scout, kit, ada] = chips(html);
  assert.match(scout, /title="Scout, needs you · Scout&#x27;s job"/);
  assert.match(scout, /data-hero-state="needs you"/);
  assert.match(scout, /data-mood-dot=""/);
  assert.match(scout, /Scout<span class="sr-only">, needs you<\/span>/);
  assert.match(kit, /title="Kit, working · Kit&#x27;s job"/);
  assert.match(kit, /data-working-dot=""/);
  assert.match(kit, /class="kw-pulse pointer-events-none absolute rounded-full bg-sky-400"/);
  assert.match(kit, /Kit<span class="sr-only">, working<\/span>/);
  assert.ok(!ada.includes('sr-only') && !ada.includes('data-hero-state'), 'everyone else stays as today');
  assert.match(ada, /title="Ada&#x27;s job"/);
  assert.deepEqual(faces(html).slice(0, 3), ['Ada', 'Scout', 'Kit'], 'Scout takes the front (middle) face, Kit goes right');
  assert.match(html, /z-10 scale-110[^>]*><div[^>]*title="Scout needs you"/, 'the front slot is Scout');
  reset();
});

test('a team home keeps "· lead" on the lead, even when the lead moves down', () => {
  reset();
  m.setMoodNeedsYou(['mira']);
  const home = { kind: 'team', team: { name: 'Ship', purpose: 'Ship it', lead: 'Ada', members: team } };
  const html = m.render({ home, team: team.slice(0, 5) });
  assert.deepEqual(chipNames(html), ['Mira', 'Ada', 'Lin', 'Pixel', 'Rigel']);
  const ada = chips(html).find((c) => c.includes('data-lead'));
  assert.match(ada, /Ada<span class="text-zinc-400">· lead<\/span>/);
  reset();
});

test("a bot's own home is unchanged by other bots' moods", () => {
  reset();
  m.setMoodNeedsYou(['scout']);
  const bot = { id: 'ada', name: 'Ada', title: 'Tech lead', avatar_sprite: 'default', covers: [] };
  const html = m.render({ home: { kind: 'bot', bot }, team });
  assert.ok(!html.includes('data-hero-state') && !html.includes('Scout'));
  reset();
});

test('big faces: a 2-bot space with a state puts the 1st in front; quiet keeps list order', () => {
  reset();
  const two = team.slice(0, 2);
  assert.deepEqual(faces(m.render({ home: general, team: two })), ['Ada', 'Lin']);
  m.setMoodNeedsYou(['lin']);
  const html = m.render({ home: general, team: two });
  assert.deepEqual(faces(html), ['Ada', 'Lin'], 'order Lin, Ada becomes faces [Ada, Lin] with Lin in front');
  assert.match(html, /z-10 scale-110[^>]*><div[^>]*title="Lin needs you"/);
  m.setMoodNeedsYou(['ada']);
  assert.deepEqual(faces(m.render({ home: general, team: [team[0]] })), ['Ada'], 'one bot stays as it is');
  reset();
});
