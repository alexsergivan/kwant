// One look per bot: an avatar that only knows a bot's title or name draws the bot's saved style,
// so the sidebar, the Ask chips, chat replies and the working line all show the same creature.
// (Oleks, Nov 10: TechLead from the coding team was violet in the Ask chips, cyan in its replies.)
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernAvatar } from './src/components/ModernAvatar';
  export * from './src/botLooks';
  // Only the drawing: the wrapper's title and data attributes don't matter here.
  export const look = (p) => {
    const html = renderToStaticMarkup(React.createElement(ModernAvatar, { size: 36, ...p }));
    return html.slice(html.indexOf('<svg'), html.indexOf('</svg>'));
  };
`);

// The coding team's lead: saved as Architect, while its name hints at the Tech Lead look.
const team = [
  { id: 'k-lead', name: 'TechLead', avatar_sprite: 'architect' },
  { id: 'k-fe', name: 'Frontend', avatar_sprite: 'tech_lead' },
  { id: 'k-be', name: 'Backend', avatar_sprite: 'devops' },
];

test('before the bot list loads, a title or name hint draws as it always did', () => {
  s.setKnownBots([]);
  assert.notEqual(s.look({ role: 'Tech Lead', name: 'TechLead' }), s.look({ role: 'architect', name: 'TechLead' }));
  assert.equal(s.look({ role: 'Tech Lead', name: 'TechLead' }), s.look({ role: 'tech_lead', name: 'TechLead' }));
});

test('every place draws the saved style: by style, by title with an id, by name alone', () => {
  s.setKnownBots(team);
  for (const b of team) {
    const saved = s.look({ role: b.avatar_sprite, name: b.name, kwantId: b.id }); // sidebar, Ask chips
    assert.equal(s.look({ role: 'Tech Lead', name: b.name, kwantId: b.id }), saved, `${b.name}: reply with a title`);
    assert.equal(s.look({ role: b.name, name: b.name }), saved, `${b.name}: working line, name only`);
    assert.equal(s.look({ name: b.name }), saved, `${b.name}: no role at all`);
    assert.equal(s.look({ role: b.name.toLowerCase(), name: b.name.toUpperCase() }), saved, `${b.name}: any case`);
  }
});

test('a picked style still wins, so previews show the choice and not the saved look', () => {
  s.setKnownBots(team);
  assert.equal(s.avatarStyleFor('qa', 'k-lead', 'TechLead', (r) => r === 'qa'), 'qa');
  assert.notEqual(s.look({ role: 'qa', name: 'TechLead', kwantId: 'k-lead' }), s.look({ role: 'architect', name: 'TechLead' }));
  // The person is never drawn as a bot that shares their name.
  assert.equal(s.look({ role: 'user', name: 'TechLead' }), s.look({ role: 'user', name: 'Someone' }));
});

test('a name two bots share is not guessed; the id still finds each one', () => {
  s.setKnownBots([
    { id: 'a', name: 'Helper', avatar_sprite: 'chef' },
    { id: 'b', name: 'helper', avatar_sprite: 'qa' },
    { id: 'c', name: 'Twin', avatar_sprite: 'qa' },
    { id: 'd', name: 'Twin', avatar_sprite: 'qa' },
  ]);
  assert.equal(s.savedStyleOf(undefined, 'Helper'), undefined);
  assert.equal(s.savedStyleOf('a', 'Helper'), 'chef');
  assert.equal(s.savedStyleOf('b', 'Helper'), 'qa');
  // Same name, same style: no conflict to avoid.
  assert.equal(s.savedStyleOf(undefined, 'Twin'), 'qa');
  assert.equal(s.avatarStyleFor('Helper Bot', undefined, 'Helper', () => false), 'Helper Bot');
});

test('a bot with no saved style keeps its hint, and an unknown id falls back to the name', () => {
  s.setKnownBots([{ id: 'x', name: 'Plain', avatar_sprite: '' }, { id: 'y', name: 'Named', avatar_sprite: 'writer' }]);
  assert.equal(s.avatarStyleFor('Plain', 'x', 'Plain', () => false), 'Plain');
  assert.equal(s.avatarStyleFor('Named', 'gone', 'Named', () => false), 'writer');
});

test('changes reach subscribers', () => {
  let n = 0;
  const off = s.subscribeBotLooks(() => { n += 1; });
  const before = s.botLooksRev();
  s.setKnownBots(team);
  assert.equal(n, 1);
  assert.ok(s.botLooksRev() > before);
  off();
  s.setKnownBots([]);
  assert.equal(n, 1);
});
