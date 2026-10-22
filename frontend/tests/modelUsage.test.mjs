// Per-bot usage view for GET /api/kwants/:id/model-usage (the Usage tab in a bot's settings).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  export { compactTokens, cacheShare, purposeLabel, modelUsageApi } from './src/modelUsage';
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { UsageView } from './src/components/UsagePanel';
  export const view = (props) => renderToStaticMarkup(React.createElement(UsageView, { kwantName: 'Analyst', onReload: () => {}, ...props }));
  export { UsageView };
`);

const sums = (o = {}) => ({
  calls: 0, fresh_input_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, output_tokens: 0,
  tools_chars: 0, system_chars: 0, messages_chars: 0, ...o,
});
// Shaped like the response in grok-harness-ideas/notes.md, Job Y item 6.
const usage = {
  kwant_id: 'k1', since: null,
  totals: sums({ calls: 18, fresh_input_tokens: 720, cache_read_tokens: 24000, cache_write_tokens: 3000, output_tokens: 216 }),
  by_purpose: [
    { purpose: 'reply', ...sums({ calls: 14, fresh_input_tokens: 600, cache_read_tokens: 20000, output_tokens: 180 }) },
    { purpose: 'summary', ...sums({ calls: 1, fresh_input_tokens: 80, cache_write_tokens: 3000, output_tokens: 30 }) },
    { purpose: 'tool_round', ...sums({ calls: 3, fresh_input_tokens: 40, cache_read_tokens: 4000, output_tokens: 6 }) },
  ],
  recent: [
    { id: 18, context_id: 'c', purpose: 'tool_round', model: 'claude-opus-5-5', fresh_input_tokens: 40, cache_read_tokens: 1500,
      cache_write_tokens: 0, output_tokens: 12, reported: true, tools_chars: 1, system_chars: 1, messages_chars: 1, created_at: '2026-10-21T22:01:40Z' },
    { id: 17, context_id: 'c', purpose: 'reply', model: 'gpt-x', fresh_input_tokens: 0, cache_read_tokens: 0,
      cache_write_tokens: 0, output_tokens: 0, reported: false, tools_chars: 1, system_chars: 1, messages_chars: 1, created_at: '2026-10-21T22:01:00Z' },
  ],
};

test('token counts are compact and the cache share counts every input token', () => {
  assert.equal(m.compactTokens(0), '0');
  assert.equal(m.compactTokens(999), '999');
  assert.equal(m.compactTokens(1500), '1.5k');
  assert.equal(m.compactTokens(24000), '24k');
  assert.equal(m.compactTokens(19500), '19.5k');
  assert.equal(m.compactTokens(123456), '123k');
  assert.equal(m.compactTokens(2_500_000), '2.5M');
  assert.equal(m.compactTokens(-3), '0');
  assert.equal(m.compactTokens(NaN), '0');
  // 24000 read of 720 + 24000 + 3000 input.
  assert.equal(m.cacheShare(usage.totals), 87);
  assert.equal(m.cacheShare(sums()), null);
  assert.equal(m.purposeLabel('tool_round'), 'Tool rounds');
  assert.equal(m.purposeLabel('tool_round', true), 'Tool round');
  assert.equal(m.purposeLabel('something_new'), 'something_new');
});

test('the four token kinds, purposes and recent calls render', () => {
  const html = m.view({ usage, error: null });
  assert.match(html, /18 calls, 87% of input from the cache/);
  assert.match(html, /data-kind="fresh_input_tokens">720</);
  assert.match(html, /data-kind="cache_read_tokens">24k</);
  assert.match(html, /data-kind="cache_write_tokens">3k</);
  assert.match(html, /data-kind="output_tokens">216</);
  assert.match(html, /Replies \(14\)/);
  assert.match(html, /Summaries/);
  assert.match(html, /Tool rounds/);
  assert.match(html, /fresh 40 · cache read 1.5k · cache write 0 · out 12/);
  assert.match(html, /data-call="tool_round"><div[^>]*><span[^>]*>Tool round<\/span>/);
  assert.match(html, /claude-opus-5-5/);
  // A call the provider gave no numbers for says so instead of showing zeros.
  assert.match(html, /The provider sent no token counts for this call\./);
  // Newest first, as the server sends it.
  assert.ok(html.indexOf('data-call="tool_round"') < html.indexOf('data-call="reply"'));
});

test('loading, error and empty states', () => {
  assert.match(m.view({ usage: null, error: null }), /role="status".*Loading usage/s);
  const err = m.view({ usage: null, error: "Couldn't load usage." });
  assert.match(err, /role="alert"/);
  assert.match(err, /Retry/);
  const empty = m.view({ usage: { ...usage, totals: sums(), by_purpose: [], recent: [] }, error: null });
  assert.match(empty, /data-usage-empty/);
  assert.match(empty, /No model calls recorded for Analyst yet/);
  assert.doesNotMatch(empty, /data-usage=/);
});

test('Retry and Refresh call onReload', () => {
  let calls = 0;
  const onReload = () => calls++;
  const find = (node, pred) => {
    if (!node || typeof node !== 'object') return null;
    if (Array.isArray(node)) for (const c of node) { const f = find(c, pred); if (f) return f; }
    else {
      if (pred(node)) return node;
      return find(node.props?.children, pred);
    }
    return null;
  };
  const isBtn = (n) => n.type === 'button' && typeof n.props?.onClick === 'function';
  find(m.UsageView({ kwantName: 'A', usage: null, error: 'x', onReload }), isBtn).props.onClick();
  find(m.UsageView({ kwantName: 'A', usage, error: null, onReload }), isBtn).props.onClick();
  assert.equal(calls, 2);
});

test('the API asks for the bot by id with a limit', async () => {
  const seen = [];
  const orig = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    seen.push([String(url), init?.method]);
    return new Response(JSON.stringify(usage), { status: 200, headers: { 'content-type': 'application/json' } });
  };
  try {
    const got = await m.modelUsageApi.get('k/1');
    assert.equal(got.totals.calls, 18);
  } finally {
    globalThis.fetch = orig;
  }
  assert.match(seen[0][0], /\/api\/kwants\/k%2F1\/model-usage\?limit=20$/);
});
