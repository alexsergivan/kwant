// Parallel Universe (plan item 7): every number shows whether it was measured, estimated from a dated
// price or guessed; the form checks the server's caps before saving; a busy server and an interrupted
// run read calmly and honestly.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/parallelUniverse';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ComparisonReport, RunProgress, ScenarioDetail, ScenarioForm, ScenarioList, KindBadge } from './src/components/ParallelUniverseView';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const report = r(ComparisonReport);
  export const progress = r(RunProgress);
  export const detail = r(ScenarioDetail);
  export const form = r(ScenarioForm);
  export const list = r(ScenarioList);
  export const badge = r(KindBadge);
`);

const noop = () => {};
const at = '2026-10-20T20:00:00Z';
const verified = (metric, value, unit) => ({ metric, value, unit, tag: 'verified', command: 'docker run … oha', raw_output: '{}', measured_at: at });
const estimate = (value, stale = false) => ({
  metric: 'monthly_cost', value, unit: 'USD', tag: 'estimate', source_url: 'https://www.digitalocean.com/pricing/droplets', as_of: '2026-10-20', stale, assumptions: [],
});
const guess = { metric: 'monthly_cost', value: 0, unit: 'USD', tag: 'speculation', note: 'no priced plan chosen for this option' };

// Shaped like run::Report in src/parallel/run.rs.
const reportOf = (over = {}) => ({
  run_id: 'r1', scenario_id: 's1', title: 'Leave managed hosting?', question: '', status: 'finished', started_at: at, finished_at: at, build_network_used: false,
  criteria: [],
  options: [
    {
      key: 'vps', label: 'DigitalOcean droplet', plan: 'do-basic-1gb', status: 'ok', error_code: null, error: null, build_network_used: false,
      measured: [verified('p95_latency', 57.5, 'ms'), verified('requests_per_sec', 591, '1/s'), verified('memory_after_load', 23.5, 'MB')],
      cost: estimate(6),
      notes: [{ criterion: 'rollback', text: 'Keep the previous image tagged.', tag: 'speculation', note: 'general note for vm plans' }],
    },
    { key: 'lambda', label: 'AWS Lambda', plan: null, status: 'failed', error_code: 'health_timeout', error: 'the app never answered', build_network_used: false, measured: [], cost: guess, notes: [] },
  ],
  leaders: [{ criterion: 'p95_latency', option: 'vps', value: 57.5, unit: 'ms', basis: 'verified' }],
  caveats: ['These numbers are from this server, not the provider’s hardware.'],
  ...over,
});
const branch = (key, status, extra = {}) => ({ key, label: key.toUpperCase(), status, error_code: null, error: null, figures: [], cleanup_errors: [], build_network_used: false, started_at: null, finished_at: null, ...extra });
const runOf = (status, branches) => ({ id: 'r1', scenario_id: 's1', status, scenario: {}, branches, cleanup_errors: [], started_at: at, finished_at: null, swept_at: null });
const scenario = { id: 's1', title: 'Leave managed hosting?', question: 'VPS or serverless?', options: [{ key: 'vps', label: 'Droplet' }], build_network: false };

test('each tag maps to one visible label, and anything unknown is a guess, never measured', () => {
  assert.equal(m.figureKind('verified').label, 'Measured');
  assert.equal(m.figureKind('estimate').label, 'Estimate');
  assert.equal(m.figureKind('speculation').label, 'Guess');
  assert.equal(m.figureKind('something_new').label, 'Guess');
  assert.match(ui.badge({ tag: 'estimate' }), />Estimate</);
});

test('the comparison labels every figure and never shows an estimate as measured', () => {
  const html = ui.report({ report: reportOf() });
  // 3 measured figures + the leader's basis.
  assert.equal(html.match(/>Measured</g).length, 4);
  // The cost, with "about", its source, date and the Estimate label.
  assert.match(html, /about 6\.00 USD/);
  assert.match(html, />Estimate</);
  assert.match(html, /Prices as of 2026-10-20/);
  assert.match(html, /href="https:\/\/www\.digitalocean\.com\/pricing\/droplets"[^>]*target="_blank"[^>]*rel="noopener noreferrer"/);
  assert.match(html, />digitalocean\.com</);
  // No plan: no number at all, labelled a guess.
  assert.match(html, /unknown/);
  assert.doesNotMatch(html, /0\.00 USD/);
  // Notes are guesses.
  assert.match(html, /Keep the previous image tagged/);
  assert.ok(html.match(/>Guess</g).length >= 2);
  assert.doesNotMatch(html, /Price may be out of date/);
  assert.match(ui.report({ report: reportOf({ options: [{ ...reportOf().options[0], cost: estimate(6, true) }] }) }), /Price may be out of date/);
});

test('leaders and caveats show only when the report gives them', () => {
  const html = ui.report({ report: reportOf() });
  assert.match(html, /Fastest:/);
  assert.doesNotMatch(html, /Cheapest/);
  assert.match(html, /Keep in mind/);
  assert.match(html, /not the provider’s hardware/);
  const bare = ui.report({ report: reportOf({ leaders: [], caveats: [] }) });
  assert.doesNotMatch(bare, /Fastest|Cheapest|aria-label="Leaders"|Keep in mind/);
  // A failed option says why and has no numbers.
  assert.match(html, /the app never answered/);
  assert.match(html, /Nothing measured — the test didn’t finish/);
});

test('a price source that is not https is shown, not linked', () => {
  const r = reportOf();
  r.options[0].cost = { ...estimate(6), source_url: 'javascript:alert(1)' };
  const html = ui.report({ report: r });
  assert.doesNotMatch(html, /href="javascript/);
  assert.match(html, /javascript:alert\(1\)/);
});

test('values read as a person would say them', () => {
  assert.equal(m.formatValue(57.5, 'ms'), '57.5 ms');
  assert.equal(m.formatValue(591.2, '1/s'), '591 /s');
  assert.equal(m.formatValue(1, 'ratio'), '100 %');
  assert.equal(m.formatValue(0.987, 'ratio'), '98.7 %');
  assert.equal(m.formatValue(5.123, 'USD'), '5.12 USD');
  assert.equal(m.metricLabel('p95_latency'), '95% answered within');
  assert.equal(m.metricLabel('cold_start'), 'Cold start');
});

const option = (over = {}) => ({ ...m.emptyOption(), label: 'Droplet', image: 'kwnt-executor:v1', ...over });
const draft = (over = {}) => ({ ...m.emptyScenario(), title: 'Leave managed hosting?', options: [option()], ...over });

test('a valid draft becomes the body the server expects', () => {
  const { errors, body } = m.validateScenario(draft({ options: [option(), option({ label: 'Droplet', plan: 'do-basic-1gb' })] }), new Set(['do-basic-1gb']));
  assert.deepEqual(errors, {});
  assert.equal(body.options.length, 2);
  assert.deepEqual(body.options.map((o) => o.key), ['droplet', 'droplet-2'], 'keys come from the names and stay unique');
  assert.deepEqual(body.options[0].limits, { cpus: 1, memory_mb: 512, pids: 128, timeout_secs: 300 });
  assert.deepEqual(body.options[0].app, { port: 8080, health_path: '/', image: 'kwnt-executor:v1' });
  assert.equal(body.options[1].plan, 'do-basic-1gb');
  assert.equal(body.build_network, false);
  assert.equal(body.traffic.monthly_requests, 100000);
});

test('the form refuses what the server would: caps, ranges, paths', () => {
  const v = (d) => m.validateScenario(d).errors;
  assert.match(v(draft({ title: '  ' })).title, /name/);
  assert.match(v(draft({ options: [option(), option(), option(), option()] })).options, /At most 3/);
  assert.match(v(draft({ options: [] })).options, /at least one/);
  // 3 × 1100 MB is over the 3072 MB the options may reserve together.
  const big = v(draft({ options: [option({ memory_mb: '1100' }), option({ memory_mb: '1100' }), option({ memory_mb: '1100' })] }));
  assert.match(big.memory_total, /3300 MB together.*3072 MB/);
  const o = v(draft({ options: [option({ cpus: '4', memory_mb: '32', pids: '1000', timeout_secs: '10.5', port: '0', health_path: 'health' })] }));
  assert.match(o['options.0.cpus'], /0\.1 to 2/);
  assert.match(o['options.0.memory_mb'], /64 to 2048/);
  assert.match(o['options.0.pids'], /16 to 512/);
  assert.match(o['options.0.timeout_secs'], /30 to 900/);
  assert.match(o['options.0.port'], /1 to 65535/);
  assert.match(o['options.0.health_path'], /Starts with \//);
  assert.match(v(draft({ options: [option({ image: '--privileged' })] }))['options.0.image'], /Docker image/);
  assert.match(v(draft({ options: [option({ source: 'context', context: 'app' })] }))['options.0.context'], /full folder path/);
  assert.match(v(draft({ options: [option({ source: 'context', context: '/tmp/../etc' })] }))['options.0.context'], /without/);
  assert.match(v(draft({ options: [option({ source: 'context', context: '/srv/app', dockerfile: '../Dockerfile' })] }))['options.0.dockerfile'], /inside the folder/);
  assert.match(m.validateScenario(draft({ options: [option({ plan: 'nope' })] }), new Set(['do-basic-1gb'])).errors['options.0.plan'], /price table/);
  assert.match(v(draft({ load_concurrency: '100' })).load_concurrency, /1 to 64/);
  assert.match(v(draft({ monthly_requests: '1.5' })).monthly_requests, /whole number/);
});

test('editing keeps option keys, so past runs still line up', () => {
  const saved = {
    id: 's1', title: 'T', question: '', requirements: { notes: 'x', min_memory_mb: null, needs_persistent_disk: false, regions: [] },
    traffic: { monthly_requests: 10, peak_rps: 1, avg_response_kb: 2, avg_duration_ms: 3 }, load: { requests: 100, concurrency: 2, path: '/' },
    criteria: [{ key: 'cost', weight: 5 }], build_network: true, created_at: at, updated_at: at,
    options: [{ key: 'vps', label: 'Renamed droplet', plan: null, app: { context: '/srv/app', dockerfile: null, port: 80, health_path: '/' }, limits: { cpus: 0.5, memory_mb: 256, pids: 64, timeout_secs: 60 }, ops_notes: '', rollback_notes: '' }],
  };
  const d = m.draftFrom(saved);
  assert.equal(d.options[0].source, 'context');
  assert.equal(d.build_network, true);
  const { body } = m.validateScenario(d);
  assert.equal(body.options[0].key, 'vps');
  assert.equal(body.options[0].app.context, '/srv/app');
});

test('the form shows the memory total, the 3-option cap and an off-by-default build-network box with its warning', () => {
  const html = ui.form({ initial: m.emptyScenario(), editing: false, prices: [], saving: false, serverError: null, onSave: noop, onCancel: noop });
  assert.match(html, /Memory reserved: 512 of 3072 MB/);
  assert.match(html, /\(1 of 3\)/);
  assert.match(html, /Add an option/);
  assert.match(html, /<input type="checkbox"(?![^>]*checked)[^>]*>/);
  assert.match(html, /Let the build step use the internet/);
  assert.match(html, /The app itself is always tested offline/);
  const full = ui.form({ initial: draft({ options: [option(), option(), option()] }), editing: false, prices: [], saving: false, serverError: null, onSave: noop, onCancel: noop });
  assert.doesNotMatch(full, /Add an option/);
  assert.match(full, /the most options a scenario can have/);
});

test('form errors are tied to their fields for screen readers', () => {
  const html = ui.form({ initial: draft({ title: '', options: [option({ memory_mb: '5000' })] }), editing: false, prices: [], saving: false, serverError: null, initialShowErrors: true, onSave: noop, onCancel: noop });
  assert.match(html, /aria-invalid="true"/);
  assert.match(html, /Give the scenario a name/);
  assert.match(html, /From 64 to 2048/);
  assert.match(html, /Fix the 3 highlighted fields to save/);
  const id = html.match(/aria-invalid="true" aria-describedby="([^"]+)"/)[1];
  assert.match(html, new RegExp(`id="${id}"[^>]*>Give the scenario a name`));
});

test('a 409 busy is a calm notice, not an error', () => {
  const e = m.errorFrom(409, { error: 'a run is already going; wait for it to finish', code: 'busy' });
  assert.equal(e.busy, true);
  assert.match(e.message, /Another run is going/);
  assert.equal(m.errorFrom(409, { error: 'conflict' }).busy, false);
  assert.match(m.errorFrom(400, { error: 'fly: memory_mb must be between 64 and 2048' }).message, /^Fly: memory_mb .*2048\.$/);
  assert.match(m.errorFrom(403, {}).message, /Only you/);
  assert.match(m.errorFrom(500, null).message, /HTTP 500/);

  const html = ui.detail({ scenario, runs: [], current: null, starting: false, busy: true, notice: null, onStart: noop, onPickRun: noop, onEdit: noop, onDelete: noop });
  assert.match(html, /Another run is going right now/);
  assert.match(html, /border-amber-500/);
  assert.doesNotMatch(html, /rose-500/);
  assert.match(html, /<button type="button" disabled=""[^>]*>.*Start a run/);
});

test('a running run shows per-option progress; the start button waits', () => {
  const run = runOf('running', [branch('vps', 'ok'), branch('fly', 'running'), branch('lambda', 'pending')]);
  const p = ui.progress({ run });
  assert.match(p, /1 of 3 options tried/);
  assert.match(p, /role="progressbar"[^>]*aria-valuemax="3" aria-valuenow="1"/);
  assert.match(p, />Done<.*>VPS</);
  assert.match(p, />Testing<.*>FLY</);
  assert.match(p, />Waiting<.*>LAMBDA</);
  const pending = reportOf({ status: 'running', options: reportOf().options.map((o) => ({ ...o, status: 'pending', measured: [] })), leaders: [] });
  const html = ui.detail({ scenario, runs: [run], current: { run, report: pending }, starting: false, busy: false, notice: null, onStart: noop, onPickRun: noop, onEdit: noop, onDelete: noop });
  assert.match(html, /Running…/);
  assert.doesNotMatch(html, /Another run is going/);
  assert.doesNotMatch(html, /Comparison/, 'nothing to compare until an option finishes');
});

test('an interrupted run says so plainly and keeps what finished', () => {
  const run = runOf('interrupted', [branch('vps', 'ok'), branch('fly', 'running'), branch('lambda', 'pending')]);
  const p = ui.progress({ run });
  assert.match(p, />Interrupted</);
  assert.match(p, /The server stopped before this run finished/);
  assert.doesNotMatch(p, /progressbar/);
  assert.equal(m.runProgress(run).done, 1);
  const failed = ui.progress({ run: runOf('finished', [branch('vps', 'failed', { error: 'the app never answered' })]) });
  assert.match(failed, />Failed<.*the app never answered/);
  assert.equal(m.runStatus('archived').label, 'Archived');
});

test('the empty list explains what a scenario is and offers to make one', () => {
  const html = ui.list({ scenarios: [], onOpen: noop, onNew: noop });
  assert.match(html, /data-empty/);
  assert.match(html, /No scenarios yet/);
  assert.match(html, /New scenario/);
  const one = ui.list({ scenarios: [{ ...scenario, options: [{ label: 'Droplet' }, { label: 'Fly' }] }], onOpen: noop, onNew: noop });
  assert.match(one, /2 options: Droplet · Fly/);
});
