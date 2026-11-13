// Settings → Usage history (0.9.9): a 7/30-day trend with the window total, spend per model,
// and the switch that loads the other window.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  export { trendDays, modelShares, metricText } from './src/components/DailyUsage';
  export { usageModelLabel } from './src/engines';
  export { modelUsageApi } from './src/modelUsage';
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { DailyUsageView } from './src/components/DailyUsage';
  export const view = (props) => renderToStaticMarkup(React.createElement(DailyUsageView, props));
`);

const now = new Date('2026-11-13T15:00:00Z');
const row = (day, kwant_id, o = {}) => ({
  day, kwant_id, kwant_name: kwant_id.toUpperCase(), calls: 1, fresh_input_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0,
  output_tokens: 0, tools_chars: 0, system_chars: 0, messages_chars: 0, cost_usd: null, ...o,
});
const model = (id, o = {}) => ({
  model: id, calls: 1, fresh_input_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, output_tokens: 0, tools_chars: 0,
  system_chars: 0, messages_chars: 0, cost_usd: null, ...o,
});
const costRows = [
  row('2026-11-13', 'a', { output_tokens: 1000, cost_usd: 1.5 }),
  row('2026-11-13', 'b', { output_tokens: 9000 }),
  row('2026-11-10', 'a', { output_tokens: 500, cost_usd: 0.25 }),
];

test('the trend has one day per slot, today last, empty days kept as gaps', () => {
  const t = m.trendDays(costRows, 7, now);
  assert.equal(t.metric, 'cost');
  assert.deepEqual(t.days.map((d) => d.day), ['2026-11-07', '2026-11-08', '2026-11-09', '2026-11-10', '2026-11-11', '2026-11-12', '2026-11-13']);
  assert.deepEqual(t.days.map((d) => d.value), [0, 0, 0, 0.25, 0, 0, 1.5]);
  assert.deepEqual(t.days.map((d) => d.used), [false, false, false, true, false, false, true]);
  assert.equal(t.total, 1.75);
  assert.equal(m.trendDays(costRows, 30, now).days.length, 30);
});

test('without any reported cost the trend counts fresh tokens', () => {
  const t = m.trendDays([row('2026-11-13', 'a', { fresh_input_tokens: 200_000, output_tokens: 1_000_000 })], 7, now);
  assert.equal(t.metric, 'tokens');
  assert.equal(m.metricText(t.metric, t.total), '1.2M tokens');
});

test('model shares follow the window metric, biggest first', () => {
  const models = [model('gpt-5', { output_tokens: 9000 }), model('claude-opus-5-5', { output_tokens: 1500, cost_usd: 1.5 }), model('claude-haiku-5-5', { cost_usd: 0.5 })];
  assert.deepEqual(m.modelShares(models, 'cost').map((r) => [r.model, r.share]), [['claude-opus-5-5', 75], ['claude-haiku-5-5', 25], ['gpt-5', 0]]);
  assert.deepEqual(m.modelShares(models, 'tokens').map((r) => [r.model, r.share]), [['gpt-5', 86], ['claude-opus-5-5', 14], ['claude-haiku-5-5', 0]]);
  assert.deepEqual(m.modelShares([], 'cost'), []);
});

test('model labels come from the engine list, else the id', () => {
  const engines = [{ id: 'claude', models: [{ id: 'claude-opus-5-5', label: 'Claude Opus 5.5', default: true }] }, { id: 'openai_api', models: [{ id: 'gpt-5', label: 'GPT-5', default: true }] }];
  assert.equal(m.usageModelLabel('claude-opus-5-5', engines), 'Claude · Opus 5.5');
  assert.equal(m.usageModelLabel('gpt-5', engines), 'Codex · GPT-5');
  assert.equal(m.usageModelLabel('claude', engines), 'Claude');
  assert.equal(m.usageModelLabel('mystery-1', engines), 'mystery-1');
  assert.equal(m.usageModelLabel('mystery-1'), 'mystery-1');
});

test('the panel shows the window total, a bar per day, models and the day list', () => {
  const models = [model('claude-opus-5-5', { cost_usd: 1.75 }), model('gpt-5', { output_tokens: 9000 })];
  const html = m.view({ rows: costRows, models, range: 7, onRange: () => {}, now });
  assert.match(html, /\$1\.75<\/span><span[^>]*> in the last 7 days/);
  assert.equal(html.match(/data-day="/g).length, 7);
  assert.match(html, /aria-label="Today: \$1\.50"/);
  assert.match(html, /aria-label="Wed 11 Nov: no AI use"/);
  assert.match(html, /data-usage-readout="true"><\/span>/, "nothing named until a bar is hovered or focused");
  // One tab stop for the bars: today's.
  assert.equal(html.match(/<li tabindex="0"/g).length, 1);
  assert.match(html, /aria-pressed="true"[^>]*data-usage-range="7"/);
  assert.match(html, /By model.*claude-opus-5-5.*\$1\.75.*100%.*gpt-5.*No cost reported/s);
  assert.doesNotMatch(html, /No cost reported<\/span>[^<]*·/, "no share after a model with no cost");
  assert.match(html, /<span class="shrink-0 text-\[11px\] text-zinc-500">No cost reported<\/span><\/div>/, "plain words, no 0% share");
  assert.match(html, /By day.*Today/s);
  // An older server without `models`: no By model block, the rest works.
  assert.doesNotMatch(m.view({ rows: costRows, now }), /By model/);
});

test('the switch asks the server for 7 or 30 days', async () => {
  const seen = [];
  globalThis.fetch = async (url) => {
    seen.push(url);
    return new Response(JSON.stringify({ days: 30, rows: [], models: [] }), { status: 200 });
  };
  await m.modelUsageApi.daily(7);
  await m.modelUsageApi.daily(30);
  assert.deepEqual(seen, ['/api/usage/daily?days=7', '/api/usage/daily?days=30']);
});
