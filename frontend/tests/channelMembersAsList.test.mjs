// channel-members-as-list (Iris, follow-up to #1301/#1337): /api/channels sends each channel's `member_kwants`
// as a JSON string, the way /api/teams did. App kept channels raw, so its open-chat line
//   memberIds: [currentChannel.lead_kwant_id, ...(currentChannel.member_kwants ?? [])]
// spread the string into characters, and an approval from a channel's non-lead member never showed in it.
// Every /api/channels load now goes through normalizeChannels. Each test bundles its own imports.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'localStorage', 'sessionStorage']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

// /api/channels as the server answers it: Researcher is a member, Assistant leads.
const serverChannels = [
  { id: 'c-research', name: 'research', lead_kwant_id: 'k-lead', member_kwants: '["k-lead","k-b2"]' },
];

test('normalizeChannels turns the server’s member_kwants string into ids, and junk into []', async () => {
  const m = await loadBundled(`export { normalizeChannels } from './src/botDelete';`);
  const [c] = m.normalizeChannels(serverChannels);
  assert.deepEqual(c.member_kwants, ['k-lead', 'k-b2']);
  assert.equal(c.id, 'c-research');
  assert.equal(c.lead_kwant_id, 'k-lead');
  // An array stays as it is; junk gives [].
  assert.deepEqual(m.normalizeChannels([{ id: 'a', member_kwants: ['x', 3, 'y'] }])[0].member_kwants, ['x', 'y']);
  for (const junk of ['nope', null, 42, undefined, '{"a":1}', '']) {
    assert.deepEqual(m.normalizeChannels([{ id: 'j', member_kwants: junk }])[0].member_kwants, [], String(junk));
  }
  // Not a list gives no channels; non-objects in it are dropped.
  for (const answer of [null, 'x', 7, { error: 'nope' }]) assert.deepEqual(m.normalizeChannels(answer), []);
  assert.equal(m.normalizeChannels([null, 'x', serverChannels[0]]).length, 1);
});

test('an approval from a non-lead member shows in that channel', async () => {
  const m = await loadBundled(`export { normalizeChannels } from './src/botDelete'; export { approvalsForChat } from './src/approvals';`);
  const pending = [
    { id: 'ap-1', kwant_id: 'k-b2', tool: 'Bash' },
    { id: 'ap-2', kwant_id: 'k-elsewhere', tool: 'Bash' },
  ];
  const [c] = m.normalizeChannels(serverChannels);
  // The same line App builds for the open channel.
  const chat = { kind: 'channel', channelId: c.id, memberIds: [c.lead_kwant_id, ...(c.member_kwants ?? [])] };
  assert.deepEqual(m.approvalsForChat(pending, chat).map((a) => a.id), ['ap-1']);
  // The raw string spread gives characters, which is what hid the card.
  const raw = serverChannels[0];
  const rawChat = { kind: 'channel', channelId: raw.id, memberIds: [raw.lead_kwant_id, ...raw.member_kwants] };
  assert.deepEqual(m.approvalsForChat(pending, rawChat).map((a) => a.id), []);
});

test('every /api/channels load in App goes through normalizeChannels', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const loads = app.match(/fetch\('\/api\/channels'/g) ?? [];
  const normalized = (app.match(/normalizeChannels<Channel>\(/g) ?? []).length;
  assert.ok(loads.length >= 12, `found ${loads.length} loads`);
  assert.equal(normalized, loads.length, 'one normalizeChannels per /api/channels load (GETs and the POST answer)');
  // No channels list goes into state straight from a JSON answer.
  assert.doesNotMatch(app, /setChannels\(\s*(await\s+\w+\.json\(\)|cRes)\s*\)/);
});
