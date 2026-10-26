// F6 usage visibility: replies show fresh tokens (not cache reads) with the split in the tooltip,
// Settings → Usage shows fresh vs cached and cost per day and bot, and a chain over its token
// budget shows one notice with Continue.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  export { replyTokens, parseTurnUsage, formatCost, freshOf, modelUsageApi } from './src/modelUsage';
  export { parseBudgetPaused, budgetError, budgetValue, chainBudgetApi } from './src/chainBudget';
  export { groupDays, dayLabel } from './src/components/DailyUsage';
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { DailyUsageView } from './src/components/DailyUsage';
  import { BudgetPausedNotice } from './src/components/SystemNotice';
  export const daily = (props) => renderToStaticMarkup(React.createElement(DailyUsageView, props));
  export const notice = (props) => renderToStaticMarkup(React.createElement(BudgetPausedNotice, props));
`);

// The usage split a Claude CLI turn saves in its reply's meta (see tests/fixtures/claude).
const usageMeta = JSON.stringify({
  engine: 'claude',
  usage: { fresh_input: 12, cache_read: 83544, cache_write: 8211, output: 487, cost_usd: 0.4189 },
});

test('a reply shows fresh tokens, not the cache-inclusive total', () => {
  const t = m.replyTokens(92254, usageMeta);
  assert.equal(t.count, 12 + 8211 + 487);
  assert.equal(t.label, '8.7k fresh');
  assert.match(t.detail, /^8,710 fresh tokens: 12 new input, 8,211 written to cache, 487 output\./);
  assert.match(t.detail, /83,544 more read from cache/);
  assert.match(t.detail, /Cost \$0\.419\./);
});

test('replies without a split keep the old total, labelled as including cache reads', () => {
  assert.deepEqual(m.replyTokens(26000, JSON.stringify({ engine: 'codex' })), {
    count: 26000,
    label: '26k tokens',
    detail: '26,000 tokens, cache reads included',
  });
  assert.equal(m.replyTokens(0, null), null);
  assert.equal(m.replyTokens(undefined, 'not json'), null);
  // A split without a cost says nothing about cost
  assert.doesNotMatch(m.replyTokens(10, JSON.stringify({ usage: { fresh_input: 5, output: 5 } })).detail, /Cost/);
});

test('costs are short and never negative', () => {
  assert.equal(m.formatCost(0.4189), '$0.419');
  assert.equal(m.formatCost(12.5), '$12.50');
  assert.equal(m.formatCost(0.0004), '<$0.001');
  assert.equal(m.formatCost(0), '$0');
  assert.equal(m.formatCost(null), null);
  assert.equal(m.formatCost(-1), null);
});

const row = (day, kwant_id, kwant_name, o = {}) => ({
  day, kwant_id, kwant_name, calls: 1, fresh_input_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, output_tokens: 0,
  tools_chars: 0, system_chars: 0, messages_chars: 0, cost_usd: null, ...o,
});
const rows = [
  row('2026-10-26', 'a', 'Ann', { fresh_input_tokens: 100, cache_write_tokens: 900, output_tokens: 1000, cache_read_tokens: 50000, cost_usd: 0.5 }),
  row('2026-10-26', 'b', 'Bob', { fresh_input_tokens: 4000, output_tokens: 1000, cache_read_tokens: 0 }),
  row('2026-10-24', 'gone', null, { output_tokens: 10, cost_usd: 0.25 }),
];

test('Settings → Usage groups days with fresh, cached and cost', () => {
  const days = m.groupDays(rows);
  assert.deepEqual(days.map((d) => [d.day, d.fresh, d.cached, d.cost]), [
    ['2026-10-26', 7000, 50000, 0.5],
    ['2026-10-24', 10, 0, 0.25],
  ]);
  assert.deepEqual(days[0].bots.map((b) => b.kwant_name), ['Bob', 'Ann'], 'heaviest bot first');

  const now = new Date('2026-10-26T15:00:00Z');
  assert.equal(m.dayLabel('2026-10-26', now), 'Today');
  assert.equal(m.dayLabel('2026-10-25', now), 'Yesterday');
  assert.equal(m.dayLabel('2026-10-24', now), 'Sat 24 Oct');

  const html = m.daily({ rows, now });
  assert.match(html, /Today/);
  assert.match(html, /Ann.*2k.*50k.*\$0\.5/s);
  assert.match(html, /Deleted bot/);
  // A day without any reported cost shows none, rather than $0
  assert.doesNotMatch(m.daily({ rows: [rows[1]], now }), /\$/);
});

test('Settings → Usage loading, empty and error states', () => {
  assert.match(m.daily({ rows: null }), /Loading usage/);
  assert.match(m.daily({ rows: [] }), /No bot replies in the last two weeks/);
  assert.match(m.daily({ rows: null, error: 'Request failed (HTTP 500).', onReload: () => {} }), /role="alert".*HTTP 500.*Retry/s);
});

test('the daily endpoint is asked for two weeks', async () => {
  const seen = [];
  globalThis.fetch = async (url, init) => {
    seen.push([init?.method, url]);
    return new Response(JSON.stringify({ days: 14, rows: [] }), { status: 200 });
  };
  await m.modelUsageApi.daily();
  await m.chainBudgetApi.continue('msg 1');
  assert.deepEqual(seen, [
    ['GET', '/api/usage/daily?days=14'],
    ['POST', '/api/chains/msg%201/continue'],
  ]);
});

test('the budget field takes whole numbers up to 100M, 0 for off', () => {
  assert.equal(m.budgetError('200000'), null);
  assert.equal(m.budgetError('200,000'), null);
  assert.equal(m.budgetValue('200,000'), 200000);
  assert.equal(m.budgetError('0'), null);
  assert.match(m.budgetError(''), /whole number/);
  assert.match(m.budgetError('1.5'), /whole number/);
  assert.match(m.budgetError('-1'), /whole number/);
  assert.match(m.budgetError('100000001'), /At most 100,000,000/);
});

const pausedMeta = JSON.stringify({ kind: 'budget_paused', used: 212000, budget: 200000, waiting: ['Bob'], resumed: false });

test('a paused chain shows one notice with Continue', () => {
  const meta = m.parseBudgetPaused(pausedMeta);
  assert.deepEqual(meta, { kind: 'budget_paused', used: 212000, budget: 200000, waiting: ['Bob'], resumed: false });
  assert.equal(m.parseBudgetPaused(JSON.stringify({ kind: 'handoff_refused' })), null);
  assert.equal(m.parseBudgetPaused(null), null);

  const text = 'Paused before handing on to Bob: this request has used 212k fresh tokens, over the 200k budget.';
  const html = m.notice({ messageId: 'n1', text, meta, onContinue: async () => {} });
  assert.match(html, /data-budget-paused/);
  assert.match(html, /Paused before handing on to Bob/);
  assert.match(html, /212k of 200k fresh tokens/);
  assert.match(html, /<button[^>]*type="button"[^>]*>.*Continue<\/span><\/button>/s);
  // Already continued: the button says so and is disabled
  const done = m.notice({ messageId: 'n1', text, meta: { ...meta, resumed: true }, onContinue: async () => {} });
  assert.match(done, /disabled="".*Continued/s);
  // Nowhere to continue from: no button
  assert.doesNotMatch(m.notice({ messageId: 'n1', text, meta }), /<button/);
});
