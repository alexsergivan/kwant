import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/engines';
  export { EnginesPanel } from './src/components/EnginesPanel';
  export { ModelPicker } from './src/components/ModelPicker';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

// Shaped like EngineReport in src/drivers/engines.rs (GET /api/engines).
const engine = (id, over = {}) => ({
  id,
  name: { claude: 'Claude Code CLI', codex: 'Codex CLI', grok: 'Grok CLI', anthropic_api: 'Anthropic API key', openai_api: 'OpenAI API key', xai_api: 'xAI API key', ollama: 'Ollama' }[id],
  kind: id.endsWith('_api') ? 'api' : id === 'ollama' ? 'local' : 'cli',
  status: 'ready',
  detail: null,
  version: null,
  models: [],
  checked_at: '2026-10-17T19:00:00Z',
  ...over,
});
const claudeModels = [
  { id: 'opus', label: 'Opus', default: true },
  { id: 'sonnet', label: 'Sonnet', default: false },
  { id: 'haiku', label: 'Haiku', default: false },
  { id: 'fable', label: 'Fable', default: false },
];
// What Lin's live run on this server reported.
const live = [
  engine('claude', { version: '2.1.296', detail: 'Signed in (claude.ai)', models: claudeModels }),
  engine('codex', { status: 'signed_out', version: '0.162.1', detail: 'Not signed in: run `codex login`' }),
  engine('grok', { status: 'signed_out', version: '1.0.62', detail: 'Not signed in: run `grok login`' }),
  engine('anthropic_api', { status: 'not_installed', detail: 'No API key: add one in Settings' }),
  engine('openai_api', { status: 'not_installed', detail: 'No API key: add one in Settings' }),
  engine('xai_api', { status: 'not_installed', detail: 'No API key: add one in Settings' }),
  engine('ollama', { status: 'not_installed', detail: 'Ollama is not running' }),
];

const h = m.createElement;
const panel = (engines, over = {}) =>
  m.renderToStaticMarkup(
    h(m.EnginesPanel, {
      api: { engines, error: null, rechecking: false, recheck: async () => {}, reload: async () => {}, setEnabled: async () => {}, ...over },
      now: Date.parse('2026-10-17T19:02:00Z'),
    }),
  );
const picker = (value, engines) => m.renderToStaticMarkup(h(m.ModelPicker, { value, onChange: () => {}, engines, onOpenSettings: () => {} }));
const options = (html, selectIndex) => {
  const select = html.split('<select')[selectIndex + 1].split('</select>')[0];
  return [...select.matchAll(/<option[^>]*value="([^"]*)"[^>]*>([^<]*)</g)].map((x) => [x[1], x[2]]);
};

test('every status renders an honest label, never "Connected"', () => {
  const cases = [
    ['ready', 'Ready'],
    ['signed_out', 'Not signed in'],
    ['not_installed', 'Not installed'],
    ['error', 'Error'],
    ['checking', 'Checking…'],
  ];
  for (const [status, label] of cases) {
    assert.equal(m.statusView({ status }).label, label);
    const html = panel([engine('claude', { status, detail: status === 'ready' ? null : 'why' }), engine('codex')]);
    // Not-installed rows are folded away until opened.
    if (status === 'not_installed') {
      assert.match(html, /Not available on this computer \(1\)/);
      continue;
    }
    assert.ok(html.includes(`data-status="${status}"`), status);
    assert.ok(html.includes(`>${label}</span>`), `${status} → ${label}`);
    assert.doesNotMatch(html, /Connected/);
  }
  assert.equal(m.statusView({ status: 'ready', enabled: false }).label, 'Off');
  assert.equal(m.statusView({ status: 'not_installed', kind: 'api' }).label, 'No key');
  assert.equal(m.statusView({ status: 'not_installed', kind: 'local' }).label, 'Not running');
});

test('the reason shows when an engine is not ready, with its command as code', () => {
  const html = panel(live);
  assert.match(html, /Not signed in: run <code[^>]*>codex login<\/code>/);
  assert.match(html, /v0\.162\.1/);
  assert.doesNotMatch(html, /Signed in \(claude\.ai\)/, 'no detail line for a ready engine');
  assert.match(html, /4 models · Opus, Sonnet, Haiku…/);
  assert.match(html, /Re-check/);
  assert.match(html, /Checked 2 min ago/);
});

test('ready engines come first; not-installed ones fold under "Not available on this computer"', () => {
  const sorted = m.sortEngines([
    engine('ollama', { status: 'not_installed' }),
    engine('codex', { status: 'signed_out' }),
    engine('grok', { status: 'error' }),
    engine('claude'),
    engine('xai_api', { enabled: false }),
    engine('openai_api', { status: 'checking' }),
  ]);
  assert.deepEqual(sorted.map((e) => e.id), ['claude', 'openai_api', 'grok', 'codex', 'xai_api', 'ollama']);

  const html = panel(live);
  const order = [...html.matchAll(/data-engine="([a-z_]+)"/g)].map((x) => x[1]);
  assert.deepEqual(order, ['claude', 'codex', 'grok'], 'absent ones are folded away by default');
  assert.match(html, /Not available on this computer \(4\)/);
  assert.match(html, /aria-expanded="false"/);
});

test('the headline gives way to the route summary once something is ready', () => {
  // With something ready, the route summary answers "what runs" instead of a count.
  assert.match(panel(live), /What runs your kwants/);
  assert.doesNotMatch(panel(live), /no API keys are needed/);
  assert.match(panel(live.map((e) => ({ ...e, status: 'signed_out' }))), /No engine can run right now/);
  assert.match(panel(null), /Checking which engines can run/);
});

test('the on/off switch appears only when the server reports it', () => {
  assert.doesNotMatch(panel(live), /role="switch"/);
  const html = panel(live.map((e) => ({ ...e, enabled: e.id !== 'codex' })));
  assert.match(html, /role="switch" aria-checked="true" aria-labelledby="engine-claude-name"/);
  assert.match(html, /data-engine="codex" data-status="off"/);
  assert.match(html, /role="switch" aria-checked="false" aria-labelledby="engine-codex-name"/);
});

test('the picker offers only engines that can run, plus Auto', () => {
  const engines = options(picker('auto', live), 0).map(([v]) => v);
  assert.deepEqual(engines, ['auto', 'claude']);
  const both = live.map((e) => (e.id === 'xai_api' ? { ...e, status: 'ready', models: [{ id: 'grok-4.6', label: 'grok-4.6', default: true }] } : e));
  assert.deepEqual(options(picker('auto', both), 0), [
    ['auto', 'Auto (Laya picks)'],
    ['claude', 'Claude · Claude Code CLI'],
    ['grok', 'Grok (xAI) · xAI API key'],
  ]);
  // Switched off counts as not offered.
  const off = live.map((e) => (e.id === 'claude' ? { ...e, enabled: false } : e));
  assert.deepEqual(m.familyOptions(off), []);
});

test('models are grouped Fast, Balanced, Deep with Auto first and the default marked', () => {
  const html = picker('claude', live);
  assert.deepEqual(options(html, 1), [
    ['', 'Auto (Laya picks)'],
    ['haiku', 'Haiku'],
    ['sonnet', 'Sonnet'],
    ['opus', 'Opus (default)'],
    ['fable', 'Fable'],
  ]);
  assert.deepEqual([...html.matchAll(/<optgroup label="([^"]+)"/g)].map((x) => x[1]), ['Fast', 'Balanced', 'Deep', 'Other']);
  // The server's tier wins over the name.
  assert.equal(m.tierOf({ id: 'x', label: 'Opus', default: false, tier: 'fast' }), 'fast');
  assert.equal(m.tierOf({ id: 'gpt-5.5-mini', label: 'GPT-5.5 mini', default: false }), 'fast');
  // Auto engine leaves the model to Laya.
  assert.match(picker('auto', live).split('<select')[2], /disabled=""/);
});

test('picks round-trip through preferred_model, and old values still read', () => {
  assert.equal(m.encodePick('auto', 'x'), 'auto');
  assert.equal(m.encodePick('claude', null), 'claude');
  assert.equal(m.encodePick('claude', 'sonnet'), 'claude/sonnet');
  assert.equal(m.encodePick('ollama', null), 'local_ollama');
  assert.deepEqual(m.parsePick('claude/sonnet'), { family: 'claude', model: 'sonnet' });
  assert.deepEqual(m.parsePick('ollama/llama3.2'), { family: 'ollama', model: 'llama3.2' });
  assert.deepEqual(m.parsePick('local_ollama'), { family: 'ollama', model: null });
  assert.deepEqual(m.parsePick('claude-opus-5-5'), { family: 'claude', model: 'claude-opus-5-5' });
  assert.deepEqual(m.parsePick(''), { family: 'auto', model: null });
  assert.deepEqual(m.parsePick('mystery'), { family: null, model: 'mystery' });
  assert.equal(m.pickLabel('claude/sonnet', live), 'Claude · Sonnet');
  assert.equal(m.pickLabel('grok'), 'Grok');
  assert.equal(m.pickLabel('auto'), 'Auto');
});

test('a saved engine that can no longer run stays selected with a warning', () => {
  const html = picker('codex', live);
  assert.ok(options(html, 0).some(([v, l]) => v === 'codex' && l === 'Codex (OpenAI) (can&#x27;t run now)'));
  assert.match(html, /<option value="codex" selected="">/);
  assert.match(html, /Codex \(OpenAI\) can(&#x27;|')t run right now/);
  assert.match(html, /Not signed in: run codex login\. This kwant won(&#x27;|')t switch engine by itself/);
  assert.match(html, /aria-invalid="true"/);
  assert.deepEqual(m.pickHealth('codex', live), { kind: 'engine_unavailable', family: 'codex', reason: 'Not signed in: run `codex login`' });
  // A model the engine stopped offering.
  assert.deepEqual(m.pickHealth('claude/claude-3-5-sonnet', live), { kind: 'model_missing', family: 'claude', model: 'claude-3-5-sonnet' });
  assert.match(picker('claude/claude-3-5-sonnet', live), /claude-3-5-sonnet \(not offered now\)/);
  // Still being checked is not a warning yet.
  assert.deepEqual(m.pickHealth('codex', live.map((e) => (e.id === 'codex' ? { ...e, status: 'checking' } : e))), { kind: 'loading' });
  assert.deepEqual(m.pickHealth('claude/sonnet', live), { kind: 'ok' });
});

test('with nothing ready the picker points to Settings', () => {
  const none = live.map((e) => ({ ...e, status: 'signed_out' }));
  const html = picker('auto', none);
  assert.deepEqual(options(html, 0).map(([v]) => v), ['auto']);
  assert.match(html, /No engine can run right now/);
  assert.match(html, /Open Settings/);
  assert.match(picker('auto', null), /Checking which engines can run/);
  // An older server without /api/engines keeps every engine on offer.
  assert.deepEqual(options(picker('auto', []), 0).map(([v]) => v), ['auto', 'claude', 'codex', 'grok', 'ollama']);
});

test('a reply says which engine and model answered it', () => {
  assert.equal(m.ranOnLabel(JSON.stringify({ ran_on: { engine: 'claude', model: 'sonnet', model_label: 'Sonnet' } })), 'Claude · Sonnet');
  assert.equal(m.ranOnLabel(JSON.stringify({ ran_on: { engine: 'anthropic_api', model: 'opus' } }), live), 'Claude · Opus');
  assert.equal(m.ranOnLabel(JSON.stringify({ ran_on: { engine: 'codex' } })), 'Codex');
  // Lin's final shape: EngineId + family + full model_label.
  assert.equal(m.ranOnLabel(JSON.stringify({ ran_on: { engine: 'claude', family: 'claude', model: 'haiku', model_label: 'Claude Haiku (latest)', tier: 'fast', picked_by: 'laya' } })), 'Claude · Haiku');
  assert.equal(m.ranOnLabel(JSON.stringify({ ran_on: { engine: 'anthropic_api', family: 'claude', model: 'claude-opus-5-5', model_label: null } })), 'Claude · claude-opus-5-5');
  assert.equal(m.statusView({ status: 'disabled' }).label, 'Off');
  assert.equal(m.ranOnLabel(JSON.stringify({ delegation: {} })), null);
  assert.equal(m.ranOnLabel('not json'), null);
  assert.equal(m.ranOnLabel(null), null);
});
