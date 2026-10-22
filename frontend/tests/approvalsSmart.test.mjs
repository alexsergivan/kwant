// Oleks (2026-10-21): "Ask only when risky" next to Ask and Don't ask, so bots get on with ordinary
// work in their folder and a card shows only for risky calls.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };
globalThis.window ??= new EventTarget();

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { SpaceApprovalsSetting, BotApprovalsSetting } from './src/components/ApprovalsSetting';
  export * from './src/approvalMode';
  const r = (C, p) => renderToStaticMarkup(React.createElement(C, p));
  export const spaceSetting = (p) => r(SpaceApprovalsSetting, { spaceId: 'spc_1', spaceName: 'Work', value: 'ask', onChanged: () => {}, ...p });
  export const botSetting = (p) => r(BotApprovalsSetting, { kwantId: 'k1', kwantName: 'Dev', ...p });
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
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

test('the space control offers "Ask only when risky" between Ask and Don\'t ask, with one line on what it does', () => {
  const html = m.spaceSetting({});
  const values = [...html.matchAll(/value="(ask|smart|off)"/g)].map((x) => x[1]);
  assert.deepEqual(values, ['ask', 'smart', 'off']);
  const t = text(html);
  assert.match(t, /Ask only when risky Ordinary work in the bot's folder runs on its own; a card asks before network calls, git fetch and push, publishing, anything outside the folder or anything unclear\./);
  assert.match(t, /Not a sandbox: scripts, make and npm install inside the folder run without a card, and can do anything you can\./);
  assert.match(t, /Ask before running commands \(default\)/, 'Ask stays the default');
});

test('smart is checked when the space is smart, with no "Approvals off" marker', () => {
  const html = m.spaceSetting({ value: 'smart' });
  assert.match(html, /checked="" value="smart"/);
  assert.doesNotMatch(text(html), /Approvals off/);
});

test('the bot override names a smart space, and a smart bot shows no off marker', () => {
  const html = m.botSetting({ initial: { kwant_id: 'k1', approvals: 'inherit', approvals_effective: 'smart', space_approvals: 'smart' } });
  assert.match(text(html), /Same as space \(Ask only when risky\)/);
  assert.doesNotMatch(text(html), /Approvals off/);
  const own = m.botSetting({ initial: { kwant_id: 'k1', approvals: 'smart', approvals_effective: 'smart', space_approvals: 'ask' } });
  assert.match(own, /<option value="smart" selected="">/);
});

test('the model reads smart from the server and older values still mean ask', () => {
  const s = m.parseBotApprovals({ kwant_id: 'k1', approvals: 'smart', approvals_effective: 'smart', space_approvals: 'smart' });
  assert.deepEqual(s, { kwant_id: 'k1', approvals: 'smart', approvals_effective: 'smart', space_approvals: 'smart' });
  assert.equal(m.parseBotApprovals({ kwant_id: 'k1', approvals: 'inherit', space_approvals: 'smart' }).approvals_effective, 'smart');
  assert.equal(m.parseBotApprovals({ kwant_id: 'k1', approvals: 'weird', space_approvals: 'weird' }).approvals_effective, 'ask');
  assert.equal(m.effectiveApprovals('inherit', 'smart'), 'smart');
  assert.equal(m.approvalsOff({ approvals_effective: 'smart' }), false, 'smart is not "off"');
});

test('setSpace sends smart; a 409 for saved passwords comes back as the server\'s sentence', async () => {
  await withFetch(() => json(200, { space_id: 'spc_1', approvals: 'smart', bots: [] }), async (calls) => {
    assert.equal(await m.approvalModeApi.setSpace('spc_1', 'smart'), 'smart');
    assert.deepEqual(JSON.parse(calls[0].init.body), { approvals: 'smart' });
  });
  const refusal = 'approvals can\'t be switched off, or to "Ask only when risky", while passwords are saved on this server: a bot that can run commands could read them. Move them to Bitwarden first.';
  await withFetch(() => json(409, { error: refusal, needs_bitwarden: true, local_count: 2 }), async () => {
    await assert.rejects(m.approvalModeApi.setSpace('spc_1', 'smart'), (e) => e instanceof m.NeedsBitwardenError && e.message.toLowerCase() === refusal.toLowerCase() && e.localCount === 2);
  });
});

test('"Turn approvals back on" also moves smart spaces and bots back to ask', async () => {
  await withFetch(
    (url, init) => {
      if (url === '/api/spaces') return json(200, [{ id: 'spc_1', approvals: 'smart', archived_at: null }]);
      if (url === '/api/spaces?archived=1') return json(200, []);
      if (url === '/api/kwants?space=spc_1') return json(200, [{ id: 'k1', approvals: 'smart' }, { id: 'k2', approvals: 'ask' }]);
      if (init.method === 'PUT' && url.startsWith('/api/spaces/')) return json(200, { approvals: 'ask' });
      if (init.method === 'PUT') return json(200, { kwant_id: 'k1', approvals: 'ask', approvals_effective: 'ask', space_approvals: 'ask' });
      return json(404, {});
    },
    async (calls) => {
      assert.deepEqual(await m.approvalModeApi.turnAllOn(), { changed: 2, failed: 0 });
      const puts = calls.filter((c) => c.init?.method === 'PUT').map((c) => `${c.url} ${c.init.body}`);
      assert.deepEqual(puts, ['/api/spaces/spc_1/approvals {"approvals":"ask"}', '/api/kwants/k1/approvals?space=spc_1 {"approvals":"ask"}']);
    },
  );
});
