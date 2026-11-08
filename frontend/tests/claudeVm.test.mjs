// "Run Claude inside the VM": the setting under the VM card, its chips and status line
// (design/ui.md states A–H, design/ui-check-446.md fixes 1, 5–9).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/claudeVm';
  export { parseComputerSettings, vmCopy, computerModes, saveIsolateClaude } from './src/computerMode';
  export { ClaudeIsolationRow } from './src/components/ClaudeIsolationRow';
  export { initialSwitchState } from './src/computerMode';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const NA = (reason = 'not_supported_yet') => ({ state: 'not_isolated', reason, detail: null, progress: null });
// Shaped like src/server/computer.rs view().
const view = ({ on = false, available = true, reason = null, claude = { state: on ? 'isolated' : 'off', reason: null, detail: null, progress: null }, mode = 'vm' } = {}) => ({
  mode,
  available: { local: true, vm: true },
  reasons: { local: null, vm: null },
  isolates: ['browser', 'kwnt_shell'],
  isolate_claude: on,
  isolate_claude_toggle: { available, reason },
  agents: { claude, codex: NA(), grok: NA() },
});
const vmOf = (v) => m.parseClaudeVm(v);
const api = (raw, extra = {}) => ({
  state: { ...m.initialSwitchState, settings: m.parseComputerSettings(raw) },
  select: () => {},
  dismissError: () => {},
  setWindow: () => {},
  windowPending: false,
  setIsolateClaude: () => {},
  refresh: async () => {},
  isoPending: null,
  isoError: null,
  ...extra,
});
const render = (raw, extra) => m.renderToStaticMarkup(m.createElement(m.ClaudeIsolationRow, { api: api(raw, extra) }));

test('parseClaudeVm reads the contract; an older server has no row', () => {
  const vm = vmOf(view({ on: true }));
  assert.equal(vm.on, true);
  assert.deepEqual(vm.toggle, { available: true, reason: null });
  assert.equal(vm.agents.claude.state, 'isolated');
  assert.equal(vm.agents.codex.reason, 'not_supported_yet');
  assert.equal(m.parseClaudeVm({ mode: 'vm' }), null);
  assert.equal(m.parseComputerSettings({ ...view(), isolate_claude: undefined }).claudeVm, null);
  // progress is clamped and needs a positive total
  const p = vmOf(view({ on: true, claude: { state: 'preparing', progress: { done_bytes: 900, total_bytes: 860 } } }));
  assert.deepEqual(p.agents.claude.progress, { done_bytes: 860, total_bytes: 860 });
  assert.equal(vmOf(view({ on: true, claude: { state: 'preparing', progress: { done_bytes: 1, total_bytes: 0 } } })).agents.claude.progress, null);
});

test('A: off by default on Linux — enabled switch, the hint, all three chips neutral', () => {
  const r = m.rowModel(vmOf(view()));
  assert.deepEqual([r.checked, r.disabled, r.status], [false, false, null]);
  assert.equal(r.hint, m.ISOLATION_HINT);
  const html = render(view());
  assert.match(html, /role="switch"/);
  assert.match(html, /aria-checked="false"/);
  assert.doesNotMatch(html, /aria-disabled/);
  assert.match(html, /web search included/);
  assert.match(html, /aria-label="Where each AI runs right now"/);
  assert.match(html, /Claude: not isolated/);
  assert.match(html, /Codex: not isolated yet/);
  assert.match(html, /Grok: not isolated yet/);
});

test('Mac: disabled with its reason, no status line', () => {
  const raw = view({ available: false, reason: 'mac_later', claude: { state: 'off', reason: 'mac_later' } });
  const r = m.rowModel(vmOf(raw));
  assert.deepEqual([r.checked, r.disabled, r.status], [false, true, null]);
  assert.equal(r.hint, 'Coming to Mac later. For now, Claude runs on this computer in both modes.');
  const html = render(raw);
  assert.match(html, /aria-disabled="true"/);
  assert.match(html, /Coming to Mac later/);
});

test('Windows gets its own line (fix 8)', () => {
  const raw = view({ available: false, reason: 'not_supported_yet', claude: { state: 'off', reason: 'not_supported_yet' } });
  assert.equal(m.rowModel(vmOf(raw)).hint, 'Only on Linux for now. Claude runs on this computer in both modes.');
  assert.match(render(raw), /Only on Linux for now/);
});

test('Docker down, switch off: disabled, quiet "Start Docker to turn this on."', () => {
  const raw = view({ available: false, reason: 'docker_unavailable', claude: { state: 'off' } });
  const r = m.rowModel(vmOf(raw));
  assert.equal(r.disabled, true);
  assert.deepEqual(r.status, { kind: 'quiet', text: 'Start Docker to turn this on.' });
  assert.match(render(raw), /data-claude-iso-status="quiet"/);
});

test('Docker down, switch on: stays usable (fix 9), Stopped chip, Check again', () => {
  const raw = view({ on: true, available: false, reason: 'docker_unavailable', claude: { state: 'unavailable', reason: 'docker_unavailable' } });
  const r = m.rowModel(vmOf(raw));
  assert.deepEqual([r.checked, r.disabled], [true, false]);
  assert.equal(r.status.action, 'check');
  assert.match(r.status.text, /^Docker isn’t running, so Claude can’t reply\./);
  const html = render(raw);
  assert.doesNotMatch(html, /aria-disabled/);
  assert.match(html, /data-tone="stopped"/);
  assert.match(html, /Claude: stopped/);
  assert.match(html, /data-claude-iso-action="check"/);
  assert.match(html, />Check again</);
});

test('B: getting ready without progress — spinner chip, no bar, two lines (fix 6)', () => {
  const raw = view({ on: true, claude: { state: 'preparing', progress: null } });
  const vm = vmOf(raw);
  const chip = m.chipFor('claude', vm.agents.claude);
  assert.deepEqual([chip.tone, chip.label, chip.indeterminate], ['preparing', 'Getting ready', true]);
  const html = render(raw);
  assert.match(html, /role="status"/);
  assert.doesNotMatch(html, /role="progressbar"/);
  assert.match(html, /downloads about 860 MB and takes a few minutes/);
  assert.match(html, /Turning this off lets Claude reply on this computer meanwhile; the setup keeps going\./);
  assert.match(html, /animate-spin/);
  assert.equal(m.pollInterval(vm), 1000);
});

test('B with measured progress: percent chip and a bar', () => {
  const raw = view({ on: true, claude: { state: 'preparing', progress: { done_bytes: 430_000_000, total_bytes: 860_000_000 } } });
  assert.equal(m.chipFor('claude', vmOf(raw).agents.claude).label, 'Getting ready · 50%');
  const html = render(raw);
  assert.match(html, /role="progressbar"/);
  assert.match(html, /430 of 860 MB/);
  assert.match(html, /scaleX\(0\.5\)/);
});

test('C: isolated — teal chip, no status line, no polling', () => {
  const raw = view({ on: true });
  const vm = vmOf(raw);
  assert.equal(m.rowModel(vm).status, null);
  assert.equal(m.pollInterval(vm), null);
  const html = render(raw);
  assert.match(html, /aria-checked="true"/);
  assert.match(html, /data-tone="isolated"/);
  assert.match(html, /Claude: isolated/);
});

test('E: download failed — the server detail and Try again (fix 7)', () => {
  const raw = view({ on: true, claude: { state: 'unavailable', reason: 'download_failed', detail: 'the image pull timed out.' } });
  const r = m.rowModel(vmOf(raw));
  assert.equal(r.status.text, 'The setup stopped: the image pull timed out. Claude can’t reply until it finishes.');
  assert.equal(r.status.action, 'retry');
  assert.match(render(raw), />Try again</);
  assert.equal(m.pollInterval(vmOf(raw)), 10_000);
});

test('download needed, relay down, sign-in expired and an unknown reason each get a line', () => {
  const line = (reason, detail = null) => m.stoppedLine({ state: 'unavailable', reason, detail, progress: null });
  assert.equal(line('download_needed').action, 'retry');
  assert.match(line('relay_down').text, /couldn’t connect Claude to your sign-in/);
  assert.equal(line('relay_down').action, null);
  assert.match(line('sign_in_expired').text, /sign-in has expired/);
  assert.match(line('weird_code').text, /\(code weird_code\)/);
  assert.match(line('weird_code', 'The disk is full.').text, /\(The disk is full\)/);
});

test('Local mode or API-key bot: neutral chip with the server detail as its title', () => {
  const detail = 'Bots using an API key run inside kwant; their tools already use the VM.';
  const chip = m.chipFor('claude', { state: 'not_isolated', reason: null, detail, progress: null });
  assert.deepEqual([chip.tone, chip.label, chip.title], ['neutral', 'Not isolated', detail]);
});

test('the row shows only in VM mode', () => {
  assert.equal(render(view({ mode: 'local' })), '');
});

test('saving: the switch shows the pending value and is busy; errors show', () => {
  const html = render(view(), { isoPending: true, isoError: 'Start Docker to turn this on.' });
  assert.match(html, /aria-checked="true"/);
  assert.match(html, /aria-busy="true"/);
  assert.match(html, /Start Docker to turn this on\./);
});

test('PUT errors read in people’s words, never as a raw code (fix 5)', () => {
  assert.equal(m.putErrorText(400, { error: 'mac_later' }), m.PLATFORM_HINTS.mac_later);
  assert.equal(m.putErrorText(400, { error: 'x', code: 'docker_unavailable' }), 'Start Docker to turn this on.');
  assert.equal(m.putErrorText(400, { error: 'Docker is not running.', code: 'docker_unavailable' }), 'Start Docker to turn this on.');
  assert.equal(m.putErrorText(400, { error: 'Something human.' }), 'Something human.');
  assert.equal(m.putErrorText(500, { error: 'boom_code' }), 'Couldn’t change this (HTTP 500).');
});

test('saveIsolateClaude PUTs the flag and maps a 400', async () => {
  const orig = globalThis.fetch;
  try {
    let body;
    globalThis.fetch = async (_u, init) => {
      body = init.body;
      return new Response(JSON.stringify(view({ on: true, claude: { state: 'preparing' } })), { status: 200 });
    };
    const r = await m.saveIsolateClaude(true);
    assert.deepEqual(JSON.parse(body), { isolate_claude: true });
    assert.equal(r.ok, true);
    assert.equal(r.settings.claudeVm.agents.claude.state, 'preparing');
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'mac_later' }), { status: 400 });
    assert.deepEqual(await m.saveIsolateClaude(true), { ok: false, error: m.PLATFORM_HINTS.mac_later });
  } finally {
    globalThis.fetch = orig;
  }
});

test('the live region speaks at the start, the end and a failure only', () => {
  const s = (state, reason = null) => ({ state, reason, detail: null, progress: null });
  assert.equal(m.announcement(s('off'), s('preparing')), 'Getting the VM ready for Claude.');
  assert.equal(m.announcement(s('preparing'), s('preparing')), null);
  assert.equal(m.announcement(s('preparing'), s('isolated')), 'Claude now runs inside the VM.');
  assert.match(m.announcement(s('isolated'), s('unavailable', 'docker_unavailable')), /^Claude stopped\. Docker isn’t running/);
  assert.equal(m.announcement(undefined, s('isolated')), null);
});

test('VM card copy: setting on names Claude and warns about Codex and Grok', () => {
  const on = m.vmCopy(['browser', 'kwnt_shell'], true);
  assert.equal(on.label, 'VM (browser, shell and Claude isolated)');
  assert.match(on.note, /They won’t run in a folder Claude used inside the VM\./);
  assert.equal(m.vmCopy(['browser', 'kwnt_shell'], false).label, 'VM (browser and built-in shell isolated)');
});

test('queued for a VM spot: Lin’s wording, with or without slots', () => {
  assert.equal(m.queuedVmText(2), 'Waiting for a free spot in the VM. 2 replies run at once there. This one starts as soon as one finishes.');
  assert.equal(m.queuedVmText(1), 'Waiting for a free spot in the VM. 1 reply runs at once there. This one starts as soon as one finishes.');
  assert.equal(m.queuedVmText(), 'Waiting for a free spot in the VM. This one starts as soon as one finishes.');
});

test('the in-line sentence reads as calm waiting (qa-446: up to ~5 min behind approval cards)', () => {
  assert.equal(m.queuedText('agent_busy'), 'Waiting for this bot’s other replies to finish.');
  assert.equal(m.queuedText('vm_busy', 2), m.queuedVmText(2));
  assert.match(m.queuedText('server_busy'), /starts as soon as a slot frees up/);
  for (const r of ['agent_busy', 'server_busy', 'vm_busy']) assert.doesNotMatch(m.queuedText(r, 2), /timeout|timed out|error|stuck|minutes/i);
});
