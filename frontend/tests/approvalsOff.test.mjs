// Oleks (2026-10-20): a switch that turns every approval off, per space with a per-bot override,
// and a marker wherever a bot runs without asking.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SpaceApprovalsSetting, BotApprovalsSetting, ApprovalsOffBadge } from './src/components/ApprovalsSetting';
  import { Sidebar } from './src/components/Sidebar';
  export * from './src/approvalMode';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export const spaceSetting = (p) => r(SpaceApprovalsSetting, { spaceId: 'spc_1', spaceName: 'Work', value: 'ask', onChanged: () => {}, ...p });
  export const botSetting = (p) => r(BotApprovalsSetting, { kwantId: 'k1', kwantName: 'Dev', ...p });
  export const badge = (p) => r(ApprovalsOffBadge, p);
  const engines = { engines: [], loading: false, error: null, refresh: () => {} };
  export const sidebar = (p) =>
    r(Sidebar, {
      spaces: [], onSwitchSpace() {}, onCreateSpace() {}, onOpenSpaceSettings() {}, onOpenArchived() {}, onOpenSearch() {}, onOpenRoutines() {},
      channels: [], teams: [], selectedId: '', isChannel: false, onSelectKwant() {}, onSelectChannel() {}, onSelectTeam() {}, onSummonKwant() {}, onCreateTeam() {},
      engines, ...p,
    });
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((x) => text(x[1]));
const space = (approvals) => ({ id: 'spc_1', name: 'Work', icon: '💼', color: null, sort: 0, is_default: false, archived_at: null, created_at: '', general_channel_id: null, counts: { bots: 2, ready_accounts: 1, unread: 0 }, approvals });
const bot = (id, name, approvals, effective) => ({ id, name, title: 'Coder', persona_prompt: '', avatar_sprite: 'coder', driver_type: 'claude', preferred_model: 'claude:sonnet', approvals, approvals_effective: effective });

// ---------------------------------------------------------------- the model

test('effective value: the bot override wins, inherit follows the space, older servers mean ask', () => {
  assert.equal(m.effectiveApprovals('inherit', 'off'), 'off');
  assert.equal(m.effectiveApprovals('ask', 'off'), 'ask');
  assert.equal(m.effectiveApprovals('off', 'ask'), 'off');
  assert.equal(m.approvalsOff({ approvals_effective: 'off' }), true);
  assert.equal(m.approvalsOff({ approvals_effective: 'ask' }), false);
  assert.equal(m.approvalsOff({}), false, 'no field (older server) is not "off"');
  assert.equal(m.approvalsOff(null), false);
});

test('parseBotApprovals reads Lin\'s shape and fills a missing effective value', () => {
  assert.deepEqual(m.parseBotApprovals({ kwant_id: 'k1', approvals: 'inherit', approvals_effective: 'off', space_approvals: 'off' }), {
    kwant_id: 'k1', approvals: 'inherit', approvals_effective: 'off', space_approvals: 'off',
  });
  assert.equal(m.parseBotApprovals({ kwant_id: 'k1', approvals: 'off', space_approvals: 'ask' }).approvals_effective, 'off');
  assert.equal(m.parseBotApprovals({ kwant_id: 'k1', approvals: 'weird' }).approvals, 'inherit');
  assert.equal(m.parseBotApprovals(null), null);
  assert.equal(m.parseBotApprovals([]), null);
});

test('the space line counts bots that differ from the space', () => {
  assert.equal(m.spaceOffLine('ask', [{ approvals_effective: 'ask' }]), null);
  assert.equal(m.spaceOffLine(undefined, []), null);
  assert.equal(m.spaceOffLine('off', [{ approvals_effective: 'off' }]), 'Approvals off');
  assert.equal(m.spaceOffLine('off', [{ approvals_effective: 'ask' }, { approvals_effective: 'off' }]), 'Approvals off (1 bot still asks)');
  assert.equal(m.spaceOffLine('ask', [{ approvals_effective: 'off' }, { approvals_effective: 'off' }]), 'Approvals off for 2 bots');
});

// ---------------------------------------------------------------- the API calls

const withFetch = async (impl, fn) => {
  const calls = [];
  const prev = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return impl(url, init);
  };
  try {
    await fn(calls);
  } finally {
    globalThis.fetch = prev;
  }
};
const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

test('setSpace PUTs {approvals} to the space and returns the new value', async () => {
  await withFetch(() => json(200, { space_id: 'spc_1', approvals: 'off', bots: [] }), async (calls) => {
    assert.equal(await m.approvalModeApi.setSpace('spc_1', 'off'), 'off');
    assert.equal(calls[0].url, '/api/spaces/spc_1/approvals');
    assert.equal(calls[0].init.method, 'PUT');
    assert.deepEqual(JSON.parse(calls[0].init.body), { approvals: 'off' });
  });
});

test('setBot PUTs the override and returns the effective value', async () => {
  await withFetch(() => json(200, { kwant_id: 'k 1', approvals: 'ask', approvals_effective: 'ask', space_approvals: 'off' }), async (calls) => {
    const s = await m.approvalModeApi.setBot('k 1', 'ask');
    assert.equal(calls[0].url, '/api/kwants/k%201/approvals');
    assert.deepEqual(JSON.parse(calls[0].init.body), { approvals: 'ask' });
    assert.equal(s.approvals_effective, 'ask');
    assert.equal(s.space_approvals, 'off');
  });
});

test('errors are sentences: 403 (a bot token), 409 (archived), no endpoint, no network', async () => {
  await withFetch(() => json(403, { error: 'only the person changes approvals' }), async () => {
    await assert.rejects(m.approvalModeApi.setSpace('s', 'off'), /Only you can change approvals/);
  });
  await withFetch(() => json(409, { error: 'archived' }), async () => {
    await assert.rejects(m.approvalModeApi.setSpace('s', 'off'), /archived\. Restore it/);
  });
  await withFetch(() => new Response('', { status: 405 }), async () => {
    await assert.rejects(m.approvalModeApi.setBot('k', 'off'), /can't change approvals yet|can’t change approvals yet/);
  });
  await withFetch(() => { throw new TypeError('fetch failed'); }, async () => {
    await assert.rejects(m.approvalModeApi.setBot('k', 'off'), /Couldn’t reach the server/);
  });
  await withFetch(() => new Response('', { status: 404 }), async () => {
    assert.equal(await m.approvalModeApi.bot('k'), null, 'an older server: the bot control hides');
  });
});

// ---------------------------------------------------------------- the space control

test('space control: Ask is the default and checked, Don\'t ask explains itself, no confirm yet', () => {
  const html = m.spaceSetting({});
  const t = text(html);
  assert.match(html, /role="radiogroup" aria-label="Approvals in Work"/);
  assert.match(t, /Ask before running commands \(default\)/);
  assert.match(t, /Don't ask Bots run everything without approval\./);
  assert.match(html, /checked="" value="ask"/);
  assert.doesNotMatch(html, /checked="" value="off"/);
  assert.doesNotMatch(html, /alertdialog/);
  assert.doesNotMatch(t, /Approvals off/, 'no marker while asking');
});

test('space control: the confirm says plainly what off means, and Keep asking is the safe default focus', () => {
  const html = m.spaceSetting({ initialConfirm: true });
  const t = text(html);
  assert.match(html, /role="alertdialog"/);
  assert.match(t, /Turn off approvals for every bot in Work\?/);
  assert.match(t, /Shell commands run without asking you\./);
  assert.match(t, /written or deleted anywhere/);
  assert.match(t, /Tools \(MCP servers, browser, web\) run without asking\./);
  assert.match(t, /No approval cards appear\. Everything that runs is still recorded\./);
  assert.match(t, /Bots can't change this\./);
  assert.deepEqual(buttons(html).slice(-2), ['Keep asking', 'Turn approvals off']);
  assert.match(html, /<button type="button"[^>]*autofocus=""[^>]*>Keep asking/i);
});

test('space control when off: Don\'t ask is checked, the marker shows, and bots that still ask are named', () => {
  const html = m.spaceSetting({ value: 'off', overrides: { ask: ['Reviewer'], off: [] } });
  const t = text(html);
  assert.match(html, /checked="" value="off"/);
  assert.match(t, /Approvals off/);
  assert.match(t, /Still asks, set on the bot: Reviewer\./);
});

// ---------------------------------------------------------------- the bot override

test('bot override: three choices, "Same as space" names the space value, the marker when it runs free', () => {
  const html = m.botSetting({ initial: { kwant_id: 'k1', approvals: 'inherit', approvals_effective: 'off', space_approvals: 'off' } });
  const opts = [...html.matchAll(/<option[^>]*value="([^"]+)"[^>]*>([^<]*)</g)].map((x) => [x[1], text(x[2])]);
  assert.deepEqual(opts, [
    ['inherit', "Same as space (Don't ask)"],
    ['ask', 'Ask before running commands'],
    ['off', "Don't ask"],
  ]);
  assert.match(html, /<label for="([^"]+)"[^>]*>Approvals<\/label>/);
  assert.match(text(html), /Approvals off/);
});

test('bot override: an asking bot shows no marker; an older server shows nothing', () => {
  const html = m.botSetting({ initial: { kwant_id: 'k1', approvals: 'ask', approvals_effective: 'ask', space_approvals: 'off' } });
  assert.doesNotMatch(text(html), /Approvals off/);
  assert.match(html, /<option value="ask" selected="">/);
  assert.equal(m.botSetting({ initial: null }), '');
});

// ---------------------------------------------------------------- markers

test('the marker has an icon and words, not colour alone', () => {
  const full = m.badge({});
  assert.match(full, /<svg/);
  assert.equal(text(full), 'Approvals off');
  const compact = m.badge({ compact: true });
  assert.match(compact, /<svg[^>]*aria-hidden="true"/);
  assert.match(compact, /class="sr-only">Approvals off</, 'screen readers hear it in the bot list');
  assert.match(m.badge({ onClick: () => {} }), /^<button type="button"/);
});

test('sidebar: the bot list marks only bots that run without asking, and the space header says so', () => {
  const html = m.sidebar({ space: space('ask'), kwants: [bot('k1', 'Dev', 'off', 'off'), bot('k2', 'Writer', 'inherit', 'ask')] });
  const rows = html.split('group/fleet').slice(1);
  assert.equal(rows.length, 2);
  assert.match(rows[0], /Dev[\s\S]*sr-only">Approvals off/);
  assert.doesNotMatch(rows[1], /Approvals off/);
  assert.match(text(html), /Approvals off for 1 bot/);
});

test('sidebar: space off shows the header line; nothing at all while every bot asks', () => {
  assert.match(text(m.sidebar({ space: space('off'), kwants: [bot('k1', 'Dev', 'inherit', 'off')] })), /Approvals off/);
  const asking = m.sidebar({ space: space('ask'), kwants: [bot('k1', 'Dev', 'inherit', 'ask')] });
  assert.doesNotMatch(text(asking), /Approvals off/);
  const older = m.sidebar({ space: space(undefined), kwants: [bot('k1', 'Dev', undefined, undefined)] });
  assert.doesNotMatch(text(older), /Approvals off/);
});
