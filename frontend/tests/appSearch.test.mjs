// One list for any app (one-app-search/design.md §2, §3): appKey, byMaker, mergeRows and signInOf, table style.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { appKey, byMaker, presetPublisher, mergeRows, signInOf, leadTier, otherWaysLabel, moreOtherWays, sourceWords } from './src/appSearch';
  export { MCP_PRESETS } from './src/mcp';
`);

const app = (slug, name, over = {}) => ({ slug, name, description: '', tools: null, connected: false, ...over });
const dirEntry = (registry_name, title, over = {}) => ({
  registry_name,
  title,
  description: '',
  publisher: { label: registry_name.split('/')[0].split('.').reverse().join('.'), kind: 'domain' },
  kind: 'remote',
  url: null,
  inputs: [],
  ...over,
});

const COMPOSIO = [app('stripe', 'Stripe', { tools: 30 }), app('elevenlabs', 'ElevenLabs', { tools: 12 }), app('gmail', 'Gmail', { tools: 61 }), app('airtable', 'Airtable', { tools: 20 })];
const STRIPE_DIR = [
  // Stripe's own server is the built-in Stripe (same address): listed once.
  dirEntry('com.stripe/mcp', 'Stripe', { url: 'https://mcp.stripe.com' }),
  dirEntry('com.insightfulpipe/stripe', 'Stripe MCP by InsightfulPipe', { url: 'https://mcp.insightfulpipe.com/stripe', inputs: [{ name: 'STRIPE_API_KEY', secret: true }] }),
  dirEntry('io.github.someone/stripe-billing-reports', 'Stripe Billing Reports', { publisher: { label: 'github.com/someone', kind: 'github' }, kind: 'local', command: 'npx' }),
];
const ELEVEN_DIR = [
  dirEntry('io.elevenlabs/mcp', 'ElevenLabs', { url: 'https://api.us.elevenlabs.io/mcp' }),
  dirEntry('io.github.fan/elevenlabs-mcp', 'ElevenLabs MCP', { publisher: { label: 'github.com/fan', kind: 'github' }, kind: 'local', command: 'uvx elevenlabs-mcp', inputs: [{ name: 'ELEVENLABS_API_KEY', secret: true }] }),
  dirEntry('io.github.eleventy/eleventy', 'Eleventy', { publisher: { label: 'github.com/eleventy', kind: 'github' }, kind: 'local', command: 'npx' }),
];
const merge = (q, over = {}) => m.mergeRows(q, { presets: m.MCP_PRESETS, composio: COMPOSIO, directory: null, connections: [], ...over });
const ids = (row) => [row.lead.id, ...row.others.map((o) => o.id)];

test('appKey: one key per app, whatever the listing calls it', () => {
  for (const [name, key] of [
    ['Stripe', 'stripe'],
    ['Stripe MCP by InsightfulPipe', 'stripe'],
    ['ElevenLabs MCP', 'elevenlabs'],
    ['elevenlabs', 'elevenlabs'],
    ['11labs', 'elevenlabs'],
    ['Official Notion MCP Server', 'notion'],
    ['Linear (official)', 'linear'],
    ['Google Calendar', 'googlecalendar'],
    ['gcal', 'googlecalendar'],
    ['Postgres', 'postgresql'],
    ['Stripe Billing Reports', 'stripebillingreports'],
    ['MCP', 'mcp'],
    ['', ''],
  ])
    assert.equal(m.appKey(name), key, name);
});

test('presetPublisher: the maker\'s own domain wins over the address (GitHub), else the last two labels', () => {
  const by = (id) => m.presetPublisher(m.MCP_PRESETS.find((p) => p.id === id));
  assert.equal(by('github'), 'github.com', 'api.githubcopilot.com is GitHub\'s');
  assert.equal(by('notion'), 'notion.com');
  assert.equal(by('fetch'), null, 'a local server has no publisher');
});

test('byMaker: a domain publisher, its own address, and the app named after it', () => {
  for (const [entry, want, why] of [
    [STRIPE_DIR[0], true, 'stripe.com at mcp.stripe.com'],
    [STRIPE_DIR[1], false, 'insightfulpipe.com publishing a Stripe server'],
    [ELEVEN_DIR[0], true, 'elevenlabs.io at api.us.elevenlabs.io'],
    [ELEVEN_DIR[1], false, 'github publisher'],
    [dirEntry('io.github.stripe/mcp', 'Stripe', { publisher: { label: 'stripe.com', kind: 'domain' }, url: 'https://mcp.stripe.com' }), false, 'io.github.* is never the maker'],
    [dirEntry('com.stripe/mcp', 'Stripe', { url: 'https://stripe.evil.example' }), false, 'address not on the domain'],
    [dirEntry('com.stripe/mcp', 'Stripe', { kind: 'local', url: null }), false, 'no address to check'],
    [dirEntry('com.acme/mcp', 'Acme', { url: 'https://acme.com/x', by_maker: false }), false, 'the server field wins'],
    [dirEntry('com.acme/mcp', 'Widgets', { url: 'https://x.example', by_maker: true }), true, 'the server field wins'],
  ])
    assert.equal(m.byMaker(entry), want, why);
});

test('mergeRows: "stripe" folds InsightfulPipe and Composio under the official Stripe; Billing Reports stays its own row', () => {
  const rows = merge('stripe', { directory: STRIPE_DIR });
  assert.deepEqual(rows.map((r) => r.key), ['app-stripe', 'app-stripebillingreports']);
  const [stripe, reports] = rows;
  assert.deepEqual(ids(stripe), ['b-stripe', 'c-stripe', 'd-com.insightfulpipe/stripe']);
  assert.equal(stripe.lead.origin, 'official');
  assert.equal(stripe.lead.by, 'stripe.com');
  assert.deepEqual(stripe.others.map((o) => o.origin), ['composio', 'community']);
  assert.equal(reports.lead.origin, 'community');
  assert.equal(reports.others.length, 0);
});

test('mergeRows: "eleven" puts the official ElevenLabs first, with Composio and community under it; Eleventy below', () => {
  const rows = merge('eleven', { directory: ELEVEN_DIR });
  assert.deepEqual(rows.map((r) => r.key), ['app-elevenlabs', 'app-eleventy']);
  assert.deepEqual(ids(rows[0]), ['d-io.elevenlabs/mcp', 'c-elevenlabs', 'd-io.github.fan/elevenlabs-mcp']);
  assert.equal(rows[0].lead.origin, 'official');
  assert.equal(m.otherWaysLabel(rows[0].others), '2 other ways to add it');
});

test('mergeRows: what is already connected leads, even over an official server', () => {
  const composio = COMPOSIO.map((a) => (a.slug === 'elevenlabs' ? { ...a, connected: true } : a));
  const [row] = merge('eleven', { composio, directory: ELEVEN_DIR });
  assert.equal(row.lead.id, 'c-elevenlabs');
  assert.deepEqual(row.others.map((o) => o.id), ['d-io.elevenlabs/mcp', 'd-io.github.fan/elevenlabs-mcp']);
});

test('mergeRows: the highlight keeps its app when the public list lands', () => {
  // Before the public list answers, ElevenLabs is Composio's; after, the official server leads the same row.
  const before = merge('eleven');
  const after = merge('eleven', { directory: ELEVEN_DIR });
  assert.equal(before[0].key, 'app-elevenlabs');
  assert.equal(before[0].lead.id, 'c-elevenlabs');
  assert.equal(after[0].key, before[0].key);
  assert.equal(after[0].lead.id, 'd-io.elevenlabs/mcp');
  // A pressed row keeps its lead for this query.
  const pinned = merge('eleven', { directory: ELEVEN_DIR, pinned: { elevenlabs: 'c-elevenlabs' } });
  assert.equal(pinned[0].lead.id, 'c-elevenlabs');
});

test('mergeRows: lead order and row order', () => {
  const rows = merge('a', { directory: null });
  for (const r of rows) for (const o of r.others) assert.ok(m.leadTier(r.lead) <= m.leadTier(o), `${r.key}: ${r.lead.id} before ${o.id}`);
  // Match strength first: "air" starts Airtable's name, so it is first.
  assert.equal(merge('air')[0].key, 'app-airtable');
  assert.deepEqual(merge('   '), []);
});

test('mergeRows: one listing never shows twice', () => {
  const rows = merge('stripe', { directory: [...STRIPE_DIR, STRIPE_DIR[1]] });
  const all = rows.flatMap(ids);
  assert.equal(new Set(all).size, all.length);
});

test('signInOf: one sentence per way, from the §3 table', () => {
  const rows = merge('stripe', { directory: STRIPE_DIR });
  const eleven = merge('eleven', { directory: ELEVEN_DIR });
  const [stripe, reports] = rows;
  const by = (row, id) => [row.lead, ...row.others].find((s) => s.id === id);
  for (const [s, opts, icon, text, button] of [
    [by(stripe, 'b-stripe'), { rowName: 'Stripe', composioAdded: true }, 'account', 'Sign in with your Stripe account', 'Connect'],
    [by(stripe, 'c-stripe'), { rowName: 'Stripe', composioAdded: true }, 'account', 'Sign in with your Stripe account', 'Connect'],
    [by(stripe, 'c-stripe'), { rowName: 'Stripe', composioAdded: false }, 'account', 'Sign in with your Stripe account · free Composio key first', 'Connect'],
    [by(stripe, 'c-stripe'), { rowName: 'Stripe', composioAdded: true, other: true }, 'account', 'Sign in with your account', 'Connect'],
    [by(stripe, 'd-com.insightfulpipe/stripe'), { rowName: 'Stripe', composioAdded: true }, 'key', 'Your own Stripe API key', 'Add'],
    [reports.lead, { rowName: reports.name, composioAdded: true }, 'local', 'Runs on this computer · needs Node.js', 'Add'],
    [eleven[0].lead, { rowName: 'ElevenLabs', composioAdded: false }, 'account', 'Sign in with your ElevenLabs account', 'Connect'],
    [by(eleven[0], 'd-io.github.fan/elevenlabs-mcp'), { rowName: 'ElevenLabs', composioAdded: false }, 'key', 'Your own ElevenLabs API key', 'Add'],
    [eleven[1].lead, { rowName: 'Eleventy', composioAdded: false }, 'local', 'Runs on this computer · needs Node.js', 'Add'],
  ]) {
    const got = m.signInOf(s, opts);
    assert.equal(got.icon, icon, s.id);
    if (text) assert.equal(got.text, text, s.id);
    if (button) assert.equal(got.button, button, s.id);
  }
});

test('signInOf: the public list entries without the server fields', () => {
  const src = (entry, row = 'Acme') => merge(row, { composio: [], presets: [], directory: [entry] })[0].lead;
  for (const [entry, text] of [
    [dirEntry('com.acme/mcp', 'Acme', { url: 'https://mcp.acme.com' }), 'Sign in with your Acme account'],
    [dirEntry('io.github.x/acme', 'Acme', { publisher: { label: 'github.com/x', kind: 'github' }, url: 'https://acme.x.dev' }), 'Signs in when you add it, if it asks'],
    [dirEntry('io.github.x/acme', 'Acme', { publisher: { label: 'github.com/x', kind: 'github' }, kind: 'local', command: 'uvx acme' }), 'Runs on this computer · needs uv'],
    [dirEntry('io.github.x/acme', 'Acme', { publisher: { label: 'github.com/x', kind: 'github' }, kind: 'local', command: 'docker run acme' }), 'Runs on this computer'],
    [dirEntry('io.github.x/acme', 'Acme', { publisher: { label: 'github.com/x', kind: 'github' }, inputs: [{ name: 'ACME_KEY' }, { name: 'ACME_SECRET', secret: true }] }), '2 keys from Acme'],
    [dirEntry('io.github.x/acme', 'Acme', { publisher: { label: 'github.com/x', kind: 'github' }, sign_in: 'none' }), 'No sign-in needed'],
    [dirEntry('io.github.x/acme', 'Acme', { publisher: { label: 'github.com/x', kind: 'github' }, sign_in: 'account' }), 'Sign in with your Acme account'],
  ])
    assert.equal(m.signInOf(src(entry), { rowName: 'Acme', composioAdded: true }).text, text, JSON.stringify(entry));
});

test('words: the other-ways link, the overflow line and the source for screen readers', () => {
  const [stripe] = merge('stripe', { directory: STRIPE_DIR });
  for (const [got, want] of [
    [m.otherWaysLabel([stripe.others[0]]), 'Also through Composio'],
    [m.otherWaysLabel([stripe.others[1]]), 'Also as a community server'],
    [m.otherWaysLabel([stripe.lead]), 'Also as an official server'],
    [m.otherWaysLabel(stripe.others), '2 other ways to add it'],
    [m.moreOtherWays(1), 'and 1 more community server'],
    [m.moreOtherWays(3), 'and 3 more community servers'],
    [m.sourceWords(stripe.lead), 'official server by stripe.com'],
    [m.sourceWords(stripe.others[0]), 'through Composio'],
    [m.sourceWords(stripe.others[1]), 'community server by insightfulpipe.com'],
  ])
    assert.equal(got, want);
});
