// Find a tool's matching (composio-simple/design.md "Autocomplete"): from the first letter, typos and
// other names, the typed letters bold, and the ghost completion.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/toolSearch';`);
const apps = ['Gmail', 'Google Calendar', 'Google Sheets', 'GitHub', 'Slack', 'Notion', 'ElevenLabs', 'Microsoft Teams', 'PostgreSQL', 'Figma'].map((name) => ({
  name,
  slug: name.toLowerCase().replace(/\s+/g, ''),
}));
const names = (q) => m.rankTools(q, apps).map((r) => r.entry.name);
const kind = (q, name) => m.matchTool(q, apps.find((a) => a.name === name) ?? { name })?.kind ?? null;

test('one letter matches only the start of a name or of a word in it', () => {
  assert.deepEqual(names('g'), ['Gmail', 'Google Calendar', 'Google Sheets', 'GitHub']);
  assert.deepEqual(names('s'), ['Slack', 'Google Sheets', 'PostgreSQL'], 'start first, then a word start');
  assert.deepEqual(names('q'), [], 'no letter-in-the-middle matches at one character');
});

test('case, spaces, dashes and dots are ignored', () => {
  assert.equal(kind('GOOGLE-sheets', 'Google Sheets'), 'start');
  assert.equal(kind('googlesheets', 'Google Sheets'), 'start');
  assert.equal(kind('g.mail', 'Gmail'), 'start');
});

test('typos find the app: gmial, slak, notoin', () => {
  assert.equal(kind('gmial', 'Gmail'), 'typo');
  assert.equal(kind('slak', 'Slack'), 'typo');
  assert.equal(kind('notoin', 'Notion'), 'typo');
  assert.equal(kind('zzqx', 'Gmail'), null);
  assert.equal(kind('gmi', 'Gmail'), null, 'no typo below 4 characters');
});

test('other names: sheets, 11labs, eleven, pg, twitter', () => {
  assert.equal(kind('sheets', 'Google Sheets'), 'word');
  assert.equal(kind('11labs', 'ElevenLabs'), 'alias');
  assert.equal(kind('eleven', 'ElevenLabs'), 'start');
  assert.equal(kind('labs', 'ElevenLabs'), 'word', 'a capital starts a word');
  assert.equal(kind('pg', 'PostgreSQL'), 'alias');
  assert.equal(kind('teams', 'Microsoft Teams'), 'word');
  assert.equal(kind('twitter', 'X'), 'alias');
});

test('rank: start > word > alias > slug > contains > typo; ties go to the tie breaker', () => {
  const r = m.rankTools('cal', [{ name: 'Localize' }, { name: 'Google Calendar' }, { name: 'Calendly' }]);
  assert.deepEqual(r.map((x) => [x.entry.name, x.match.kind]), [['Calendly', 'start'], ['Google Calendar', 'word'], ['Localize', 'contains']]);
  const tie = m.rankTools('g', [{ name: 'Gong', n: 1 }, { name: 'Gmail', n: 9 }], (e) => e.n);
  assert.deepEqual(tie.map((x) => x.entry.name), ['Gmail', 'Gong']);
});

test('the typed letters are bold where they sit in the name as written; none for alias or typo', () => {
  assert.deepEqual(m.matchTool('gm', { name: 'Gmail' }).bold, [0, 2]);
  assert.deepEqual(m.matchTool('google s', { name: 'Google Sheets' }).bold, [0, 8]);
  assert.deepEqual(m.matchTool('sheets', { name: 'Google Sheets' }).bold, [7, 13]);
  assert.equal(m.matchTool('11labs', { name: 'ElevenLabs', slug: 'elevenlabs' }).bold, null);
  assert.equal(m.matchTool('gmial', { name: 'Gmail' }).bold, null);
});

test('the ghost is the rest of the name after what was typed', () => {
  assert.equal(m.ghostRest('gm', 'Gmail'), 'ail');
  assert.equal(m.ghostRest('google s', 'Google Sheets'), 'heets');
  assert.equal(m.ghostRest('googles', 'Google Sheets'), 'heets');
  assert.equal(m.ghostRest('sheets', 'Google Sheets'), '', 'no ghost for a word match');
  assert.equal(m.ghostRest('Gmail', 'Gmail'), '');
});

test('the live line counts suggestions', () => {
  assert.equal(m.suggestionsLine(0), 'No suggestions');
  assert.equal(m.suggestionsLine(1), '1 suggestion');
  assert.equal(m.suggestionsLine(7), '7 suggestions');
});
