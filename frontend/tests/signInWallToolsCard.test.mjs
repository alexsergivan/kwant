// 0.9.55 sign-in-wall-tools-card (autonomy/sign-in-wall-uses-tools, on Lin's #1262): when a bot hits a site's sign-in
// wall but has tools for that site that can't connect, the step is fixed in Tools › MCP servers, not in the browser.
// The card, the Needs-you strip, the Inbox row, the sidebar line and the bot's status say "in Tools" and offer
// Open Tools. Browser steps (sign_in, two_factor, captcha, bot-raised other) read exactly as before.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as h from './src/humanSteps';
  export * as ask from './src/askHere';
  export { HumanStepCard } from './src/components/HumanStepCard';
  export { NeedsYouStrip } from './src/components/NeedsYouStrip';
  export { Sidebar } from './src/components/Sidebar';
  export { InboxNew } from './src/components/InboxNew';
  export { focusedServerId } from './src/components/McpServersModal';
  export * as store from './src/inboxStore';
  export { INBOX_FIXTURE_EMPTY } from './src/api/inbox';
  export { itemLabel } from './src/away';
  export { busyLines } from './src/botStatus';
`);
const { React, h, ask } = m;
const el = React.createElement;
const noop = () => {};
const doc = (node) => new DOMParser().parseFromString(`<body>${m.renderToStaticMarkup(node)}</body>`, 'text/html');
const text = (d) => d.body.textContent.replace(/\s+/g, ' ');

const REASON = 'Architect’s github tools can’t connect: the key saved for github was rejected. Put a new key in Tools › MCP servers, then press Done.';
// Lin's step (#1262): kind other, source auto, no sandbox, with the marker.
const toolsMeta = (over = {}) => ({
  kind: 'human_step', human_step_id: 'hs9', step_kind: 'other', reason: REASON, url: 'https://github.com/login', sandbox_target: null,
  source: 'auto', status: 'open', fix_in: 'tools', tools_server: 'github', tools_server_id: 'mcp-7', ...over,
});
const signMeta = (over = {}) => ({
  kind: 'human_step', human_step_id: 'hs1', step_kind: 'sign_in', reason: 'Sign in to tiktok.com so I can post.', url: 'https://www.tiktok.com/login',
  sandbox_target: 'vm', source: 'auto', status: 'open', ...over,
});
const card = (raw, over = {}) => {
  const calls = [];
  const props = {
    meta: h.parseHumanStepMeta(raw), messageId: 'm1', kwantName: 'Architect', waiting: true,
    onOpenBrowser: (...a) => calls.push(['browser', ...a]), onOpenTools: (...a) => calls.push(['tools', ...a]),
    onDone: async () => {}, onSkip: async () => {}, ...over,
  };
  return { d: doc(el(m.HumanStepCard, props)), calls, props };
};

test('the marker parses from the card meta; without it, nothing changes', () => {
  assert.deepEqual(h.parseHumanStepMeta(toolsMeta()).tools, { server: 'github', serverId: 'mcp-7' });
  assert.equal(h.parseHumanStepMeta(signMeta()).tools, null);
  assert.equal(h.parseHumanStepMeta(toolsMeta({ fix_in: 'browser' })).tools, null, 'only fix_in "tools" counts');
  assert.deepEqual(h.toolsFixOf({ fix_in: 'tools' }), { server: null, serverId: null }, 'no server named: Tools still opens');
  assert.equal(h.toolsFixOf(null), null);
  // The live record carries it when the saved meta does not (a card saved before the marker)
  const live = { id: 'hs9', kind: 'other', status: 'open', fix_in: 'tools', tools_server: 'github', tools_server_id: 'mcp-7' };
  assert.deepEqual(h.humanStepView(h.parseHumanStepMeta(toolsMeta({ fix_in: undefined, tools_server: undefined, tools_server_id: undefined })), live).tools, { server: 'github', serverId: 'mcp-7' });
});

test('card: a Tools step says "needs you in Tools", offers Open Tools on that server, no browser', () => {
  const { d, calls } = card(toolsMeta());
  const t = text(d);
  assert.match(t, /Architect needs you in Tools/);
  assert.doesNotMatch(t, /in the browser/);
  assert.doesNotMatch(t, /spotted automatically/);
  assert.ok(t.includes(REASON), 'the reason stays as Lin wrote it');
  const open = d.querySelector('[data-open-tools]');
  assert.ok(open, 'Open Tools is there');
  assert.equal(open.textContent.trim(), 'Open Tools');
  assert.doesNotMatch(t, /Open browser/);
  // Mira D1-D3: the heading names whose tools broke, Done says what it claims, no footer repeating the reason
  assert.equal(d.querySelector('h3').textContent, 'Architect’s tools need a fix');
  assert.doesNotMatch(t, /Your help is needed/);
  const done = [...d.querySelectorAll('button')].find((b) => /Done/.test(b.textContent));
  assert.equal(done.textContent.trim(), 'Done, it’s fixed');
  assert.match(t, /Skip/);
  assert.doesNotMatch(t, /Open Tools and fix it there/);
  assert.equal(calls.length, 0);
});

test('card: a Tools step without onOpenTools shows no dead button, Done becomes the main one', () => {
  const { d } = card(toolsMeta(), { onOpenTools: undefined });
  assert.equal(d.querySelector('[data-open-tools]'), null);
  assert.doesNotMatch(text(d), /Open browser/);
});

test('guard: a plain auto sign-in step reads exactly as before', () => {
  const { d } = card(signMeta());
  const t = text(d);
  assert.match(t, /needs you in the browser/);
  assert.match(t, /spotted automatically/);
  assert.match(t, /Open browser/);
  assert.equal(d.querySelector('[data-open-tools]'), null);
  assert.doesNotMatch(t, /in Tools/);
  // and every browser kind, bot- or auto-raised, has no Tools words
  for (const step_kind of ['sign_in', 'two_factor', 'captcha', 'other']) {
    for (const source of ['bot', 'auto']) {
      const tt = text(card(signMeta({ step_kind, source })).d);
      assert.doesNotMatch(tt, /Tools/, `${step_kind}/${source}`);
      assert.doesNotMatch(tt, /need a fix|it’s fixed/, `${step_kind}/${source}`);
    }
  }
  // its heading, Done and footer are the sign-in ones
  assert.equal(d.querySelector('h3').textContent, 'Sign in needed');
  assert.ok([...d.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Done, I’m signed in'));
  assert.match(t, /Open the browser and do it there, then press Done\./);
});

test('D5: a card closed by a typed reply says so, without "file"; a Tools one has no browser words', () => {
  for (const raw of [toolsMeta(), signMeta()]) {
    const live = { id: raw.human_step_id, kind: raw.step_kind, status: 'left_for_message', source: 'auto', fix_in: raw.fix_in };
    const { d } = card(raw, { live, waiting: false });
    const t = text(d);
    assert.match(t, /Closed: Architect picks it up from your message\./);
    assert.doesNotMatch(t, /file you sent|reading the file/);
    assert.equal(d.querySelectorAll('button').length, 0);
    assert.ok(d.querySelector('svg.lucide-message-square'), 'the message icon, not the paperclip');
    if (raw.fix_in) assert.doesNotMatch(t, /browser/);
  }
});

// Needs-you items from /api/away carry the step id only; the frontend joins the step's marker on.
const link = { thread_id: null, channel_id: null, direct_kwant_id: 'arch' };
const awayStep = (over = {}) => ({
  kind: 'stopped', id: 'hs9', human_step_id: 'hs9', kwant_id: 'arch', kwant_name: 'Architect', title: REASON, summary: REASON, status: 'open',
  timestamp: '2026-11-26T10:00:00Z', link, message_id: 'm1', ...over,
});
const stepRec = (over = {}) => ({
  id: 'hs9', kwant_id: 'arch', channel_id: null, direct_kwant_id: 'arch', thread_id: null, turn_id: 't9', kind: 'other', reason: REASON, url: null,
  sandbox_target: null, source: 'auto', status: 'open', message_id: 'm1', created_at: '2026-11-26T10:00:00Z', closed_at: null,
  fix_in: 'tools', tools_server: 'github', tools_server_id: 'mcp-7', ...over,
});

test('withToolsFix copies the marker from the step onto its Needs-you item, and is a no-op otherwise', () => {
  const plain = awayStep({ id: 'hs1', human_step_id: 'hs1' });
  const items = [awayStep(), plain];
  const out = h.withToolsFix(items, { hs9: stepRec(), hs1: stepRec({ id: 'hs1', fix_in: undefined, tools_server: undefined, tools_server_id: undefined }) });
  assert.equal(out[0].fix_in, 'tools');
  assert.equal(out[0].tools_server_id, 'mcp-7');
  assert.equal(out[1], plain);
  const same = [plain];
  assert.equal(h.withToolsFix(same, {}), same, 'the same array when nothing changed');
});

test('strip words: a Tools step is "Tools step" with Open Tools; a browser step is unchanged', () => {
  const tools = awayStep({ fix_in: 'tools', tools_server: 'github', tools_server_id: 'mcp-7' });
  const a = ask.askAction(tools);
  assert.equal(a, 'tools');
  assert.equal(ask.askKind(tools, a), 'Tools step');
  assert.equal(ask.ACTION_LABEL.tools, 'Open Tools');
  assert.equal(ask.askHint(tools, a, 'Architect'), 'Fix it in Tools, then press Done on its card in the chat.');
  assert.equal(ask.actionName(a, 'Architect'), 'Open Tools to fix Architect’s tools');
  assert.equal(ask.switchFor('use_step_decline', tools), 'tools');
  // guard
  const browser = awayStep({ human_step_id: 'h', id: 'h' });
  const b = ask.askAction(browser);
  assert.equal(b, 'browser');
  assert.equal(ask.askKind(browser, b), 'Browser step');
  assert.equal(ask.askHint(browser, b, 'Quant'), 'Do it in Quant’s browser, then press Done there.');
  assert.equal(ask.switchFor('use_step_decline', browser), 'browser');
});

test('strip: renders Tools step and Open Tools, and the action is "tools"', () => {
  const tools = awayStep({ fix_in: 'tools', tools_server: 'github', tools_server_id: 'mcp-7' });
  const d = doc(el(m.NeedsYouStrip, { asks: [tools], onAction: noop, onDismiss: async () => {}, onShow: noop }));
  const t = text(d);
  assert.match(t, /Tools step/);
  assert.match(t, /Open Tools/);
  assert.doesNotMatch(t, /Browser step|browser/);
});

test('Inbox row: a Tools step says "Needs you in Tools" with Open Tools, from the step map', () => {
  const data = JSON.parse(JSON.stringify(m.INBOX_FIXTURE_EMPTY));
  data.items = [{ ...awayStep(), group: 'waiting_on_you', sort_key: [0, 0, 0, 'Architect', 'hs9'] }];
  data.counts.waiting_on_you = 1;
  m.store.inboxStore.reset();
  m.store.inboxStore.patch({ data });
  const d = doc(el(m.InboxNew, { noLoad: true, wide: false, onOpen: noop, onOpenTools: noop, steps: { hs9: stepRec() } }));
  const row = d.querySelector('[data-row="hs9"]');
  assert.ok(row, 'the row is listed');
  assert.match(row.textContent, /Needs you in Tools/);
  const btn = row.querySelector('[data-open-tools]');
  assert.ok(btn);
  assert.equal(btn.textContent.trim(), 'Open Tools');
  assert.match(btn.className, /h-11/, '44 px on a phone');
  // guard: without the marker it is the plain Stopped row, no Tools button
  const d2 = doc(el(m.InboxNew, { noLoad: true, wide: false, onOpen: noop, onOpenTools: noop, steps: {} }));
  const row2 = d2.querySelector('[data-row="hs9"]');
  assert.doesNotMatch(row2.textContent, /Needs you in Tools/);
  assert.match(row2.textContent, /Stopped/);
  assert.equal(row2.querySelector('[data-open-tools]'), null);
});

test('labels: itemLabel and the bot status line', () => {
  assert.deepEqual(m.itemLabel({ kind: 'stopped', status: 'open', fix_in: 'tools' }), { label: 'Needs you in Tools', tone: 'amber' });
  assert.deepEqual(m.itemLabel({ kind: 'stopped', status: 'open' }), { label: 'Stopped', tone: 'amber' });
  const turn = (turn_id, kwant_id, context_id) => ({ turn_id, kwant_id, context_id, state: 'running' });
  const lines = m.busyLines({
    turns: [turn('a', 'arch', 'open'), turn('b', 'lin', 'room'), turn('c', 'ed', 'open')], openCtx: 'open',
    awaiting: new Set(['a', 'b', 'c']), browser: new Set(['c']), tools: new Set(['a', 'b']), describe: () => 'Engineering Room',
  });
  assert.deepEqual(lines, { arch: 'Needs you in Tools here', lin: 'Needs you in Tools in Engineering Room', ed: 'Needs you in the browser here' });
});

test('open steps split by where they are fixed', () => {
  const map = { hs9: stepRec(), hs1: stepRec({ id: 'hs1', turn_id: 't1', fix_in: undefined }) };
  assert.deepEqual(h.openStepsByKwant(map), { arch: 2 });
  assert.deepEqual(h.openStepsByKwant(map, true), { arch: 1 });
  assert.deepEqual(h.openStepsByKwant(map, false), { arch: 1 });
  assert.deepEqual([...h.turnsAwaitingStep(map, true)], ['t9']);
  assert.deepEqual([...h.turnsAwaitingStep(map, false)], ['t1']);
});

const space = { id: 'spc_default', name: 'Trailhead', icon: '', color: '', sort: 0, is_default: true, archived_at: null, created_at: '',
  general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 }, approvals: 'on' };
const bot = (id, name) => ({ id, name, title: 'Role', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const sidebar = (needsYou, needsYouTools) => text(doc(el(m.Sidebar, {
  space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop, onOpenSearch: noop,
  kwants: [bot('arch', 'Architect'), bot('ben', 'Ben')], channels: [], teams: [], selectedId: null, isChannel: false,
  onSelectKwant: noop, onSelectChannel: noop, onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop,
  busyKwants: {}, botStatus: { arch: 'needs_you', ben: 'needs_you' }, needsYou, needsYouTools,
})));

test('sidebar: a bot whose open steps are all Tools fixes says "Needs you in Tools"; mixed or browser stays browser', () => {
  const t = sidebar({ arch: 1, ben: 2 }, { arch: 1, ben: 1 });
  assert.match(t, /Needs you in Tools/);
  assert.equal(t.match(/Needs you in Tools/g).length, 1, 'only Architect');
  assert.match(t, /Needs you in the browser/, 'Ben has a browser step too');
});

test('Tools opens on the server row: by id, else one unique name, else the list top', () => {
  const servers = [{ id: 'mcp-7', name: 'github' }, { id: 'mcp-8', name: 'Notion' }, { id: 'mcp-9', name: 'notion' }];
  assert.equal(m.focusedServerId(servers, { id: 'mcp-7', name: 'x', seq: 1 }), 'mcp-7');
  assert.equal(m.focusedServerId(servers, { id: null, name: 'GitHub', seq: 1 }), 'mcp-7');
  assert.equal(m.focusedServerId(servers, { id: 'gone', name: 'notion', seq: 1 }), null, 'two named alike: no guess');
  assert.equal(m.focusedServerId(servers, null), null);
});
