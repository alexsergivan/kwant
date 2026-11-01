// Phase C measures (docs/mission-control.md §6, "Measures (Phase C)"): verified tasks per active
// week over 8 rolling weeks, and cost per verified result. An unknown cost is never shown as $0, and
// a server without the measures shows the board as before.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { MeasuresPanel } from './src/components/MissionBoardView';
  export const panel = (p) => renderToStaticMarkup(React.createElement(MeasuresPanel, { timeZone: 'UTC', ...p }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/\s+/g, ' ').trim();

const DAY = 86_400_000;
const NOW = Date.parse('2026-11-01T00:00:00Z');
const weeks = (counts, active = counts.map((n) => n > 0)) =>
  counts.map((verified, i) => ({
    start: new Date(NOW - (counts.length - i) * 7 * DAY).toISOString(),
    end: new Date(NOW - (counts.length - 1 - i) * 7 * DAY).toISOString(),
    verified,
    active: active[i],
  }));
const base = { verified_7d: 2, false_success_7d: 0, needs_you_now: 0, blocked_now: 0, interventions_7d: 0, tokens_7d: 0, tokens_per_verified_7d: null };
const insights = (over = {}) => ({
  ...base,
  verified_weekly: weeks([0, 0, 1, 3, 0, 0, 4, 2], [false, false, true, true, true, false, true, true]),
  active_weeks: 5,
  verified_per_active_week: 2.0,
  cost_7d_usd: 0.65,
  cost_per_verified_7d_usd: 0.325,
  verified_observed_7d: 2,
  verified_priced_7d: 2,
  settled_7d: 4,
  unpriced_7d: 0,
  ...over,
});

test('an older server without the measures shows nothing new', () => {
  assert.equal(m.measuresView(base), null);
  assert.equal(m.measuresView(null), null);
  assert.equal(m.measuresView({ ...base, verified_weekly: [] }), null);
  assert.equal(ui.panel({ insights: base }), '');
  assert.equal(ui.panel({ insights: null }), '');
});

test('dollar amounts read the way people write them, and unknown stays unknown', () => {
  assert.equal(m.usdText(0.325), '$0.33');
  assert.equal(m.usdText(0.004), '<$0.01');
  assert.equal(m.usdText(0), '$0.00');
  assert.equal(m.usdText(12), '$12.00');
  assert.equal(m.usdText(1234.5), '$1,235');
  assert.equal(m.usdText(null), null);
  assert.equal(m.usdText(undefined), null);
  assert.equal(m.usdText(Number.NaN), null);
  assert.equal(m.usdText(-1), null);
});

test('the headline is verified per active week, with the 8-week trend oldest first', () => {
  const html = ui.panel({ insights: insights() });
  const t = text(html);
  assert.match(t, /Verified per active week 2 Average over 5 active weeks of the last 8/);
  assert.equal((html.match(/data-week=/g) ?? []).length, 8);
  assert.match(html, /aria-label="Verified tasks per week, oldest first: 0, 0, 1, 3, 0, 0, 4, 2"/);
  // Every bar has its own words for screen readers, and the last one is this week.
  assert.match(t, /This week: 2 verified/);
  assert.match(t, /Week to Oct 24: 4 verified/);
  assert.match(t, /Week to Sep 12: 0 verified, no activity/);
  // The busiest week is full height; an active empty week is a thin line, an inactive one dashed.
  assert.match(html, /data-week="4"[^>]*>.*?height:100%/);
  assert.match(html, /data-week="0" data-active="true"[^>]*>.*?height:2px/);
  assert.match(html, /data-week="0" data-active="false"[^>]*>.*?border-dashed/);
});

test('cost per verified result shows dollars, the total, and that failed runs are included', () => {
  const t = text(ui.panel({ insights: insights() }));
  assert.match(t, /Cost per verified result \$0\.33 \$0\.65 over 4 settled tasks this week, failed runs included/);
  assert.doesNotMatch(t, /didn’t report/);
});

test('runs without a reported cost are called out with what the figure rests on, never counted as $0', () => {
  // Lin's API: cost_7d_usd / verified_priced_7d. 2 of 3 verified results reported a cost.
  const one = ui.panel({ insights: insights({ verified_observed_7d: 3, verified_priced_7d: 2, unpriced_7d: 1 }) });
  assert.match(one, /data-cost="\$0\.33"/);
  assert.match(text(one), /Based on 2 verified results that reported a cost; 1 run reported none\./);
  assert.match(one, /data-cost-warning/);
  assert.doesNotMatch(text(one), /real figure is higher/);
  const three = text(ui.panel({ insights: insights({ verified_priced_7d: 1, unpriced_7d: 3 }) }));
  assert.match(three, /Based on 1 verified result that reported a cost; 3 runs reported none\./);

  // Nothing reported a cost at all: "Not reported", not $0.00.
  const none = ui.panel({ insights: insights({ cost_7d_usd: null, cost_per_verified_7d_usd: null, verified_priced_7d: 0, unpriced_7d: 4 }) });
  assert.match(none, /data-cost="Not reported"/);
  assert.match(text(none), /None of the 4 settled tasks this week reported a cost/);
  assert.doesNotMatch(text(none), /\$0/);
  assert.doesNotMatch(none, /data-cost-warning/);
});

test('failed runs reported a cost but no verified result did: no per-result figure, and no $0', () => {
  const html = ui.panel({ insights: insights({ cost_7d_usd: 0.4, cost_per_verified_7d_usd: null, verified_priced_7d: 0, unpriced_7d: 2 }) });
  assert.match(html, /data-cost="Not reported"/);
  assert.match(text(html), /None of the 2 verified results reported a cost; \$0\.40 came from other runs/);
  assert.doesNotMatch(html, /data-cost="\$/);
});

test('money spent with nothing verified shows what was spent, not a per-result price', () => {
  const html = ui.panel({ insights: insights({ verified_observed_7d: 0, verified_7d: 0, cost_7d_usd: 0.15, cost_per_verified_7d_usd: null, settled_7d: 1 }) });
  assert.match(html, /data-cost="—"/);
  assert.match(text(html), /\$0\.15 spent on 1 settled task with nothing verified yet/);
});

test('a new space: no activity yet, no settled tasks, and a flat trend', () => {
  const html = ui.panel({
    insights: insights({
      verified_7d: 0,
      verified_weekly: weeks([0, 0, 0, 0, 0, 0, 0, 0]),
      active_weeks: 0,
      verified_per_active_week: null,
      cost_7d_usd: null,
      cost_per_verified_7d_usd: null,
      verified_observed_7d: 0,
      settled_7d: 0,
      unpriced_7d: 0,
    }),
  });
  const t = text(html);
  assert.match(t, /Verified per active week — No task activity in the last 8 weeks yet/);
  assert.match(t, /Cost per verified result — No tasks settled this week/);
  assert.doesNotMatch(t, /\$/);
});

test('the board shows the measures under the week strip', async () => {
  const src = await import('node:fs').then((fs) => fs.readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8'));
  assert.match(src, /<InsightsStrip [^>]*\/>\s*<MeasuresPanel insights=\{insights\} \/>/);
});
