// ui-perf-budget (perf-regressions part D): the rules of perf/ui_budget.mjs, the blocking UI speed and size budget
// that CI job ui-perf runs. Over a limit, a missing limit, a crashed run, a page error or a lost last word must fail;
// only a metric listed under "dropped" (too noisy on runners, numbers in autonomy/ui-perf-budget/impl.md) is skipped.
import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { median, spreadPct, staticImports, htmlAssets, firstLoad, allFiles, check, summaryTable, FRAME_METRICS, BYTE_METRICS, LIFE_METRICS } from '../perf/ui_budget.mjs';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const run = (o = {}) => ({ commitsPerSecond: 30, p95FrameMs: 17, framesOver50: 2, lastWordsShown: true, errors: [], ...o });
const bytes = (v = 100) => Object.fromEntries(BYTE_METRICS.map((k) => [k, { bytes: v, files: [] }]));
const budget = (o = {}) => ({
  frame: Object.fromEntries(FRAME_METRICS.map((k) => [k, { max: 40 }])),
  bytes: Object.fromEntries(BYTE_METRICS.map((k) => [k, { max: 200 }])),
  dropped: {},
  ...o,
});
const failed = (v) => v.rows.filter((r) => !r.ok).map((r) => r.metric);

test('median and spread', () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 3, 2]), 2.5);
  assert.equal(spreadPct([10, 10, 10]), 0);
  assert.equal(spreadPct([9, 10, 11]), 20);
  assert.equal(spreadPct([0, 0, 1]), Infinity);
});

test('within the budget passes', () => {
  const v = check({ runs: [run(), run({ p95FrameMs: 18 }), run()], bytes: bytes() }, budget());
  assert.equal(v.ok, true, failed(v).join());
  assert.equal(v.median.p95FrameMs, 17);
});

test('the median, not one slow run, is checked', () => {
  const v = check({ runs: [run({ p95FrameMs: 90 }), run(), run()], bytes: bytes() }, budget());
  assert.equal(v.ok, true);
  const w = check({ runs: [run({ p95FrameMs: 90 }), run({ p95FrameMs: 41 }), run()], bytes: bytes() }, budget());
  assert.deepEqual(failed(w), ['p95FrameMs']);
});

test('each frame metric over its limit fails', () => {
  for (const k of FRAME_METRICS) {
    const v = check({ runs: [run({ [k]: 41 }), run({ [k]: 41 }), run()], bytes: bytes() }, budget());
    assert.deepEqual(failed(v), [k]);
  }
});

test('each byte metric over its limit fails', () => {
  for (const k of BYTE_METRICS) {
    const b = bytes();
    b[k].bytes = 201;
    assert.deepEqual(failed(check({ runs: [run(), run(), run()], bytes: b }, budget())), [k]);
  }
});

test('a metric with no limit fails instead of being skipped', () => {
  const b = budget();
  delete b.frame.p95FrameMs;
  b.bytes.guestAllGz.max = null;
  assert.deepEqual(failed(check({ runs: [run(), run(), run()], bytes: bytes() }, b)), ['p95FrameMs', 'guestAllGz']);
});

test('only a dropped metric is not checked', () => {
  const v = check({ runs: [run({ framesOver50: 99 }), run({ framesOver50: 99 }), run()], bytes: bytes() }, budget({ dropped: { framesOver50: 'spread 80%' } }));
  assert.equal(v.ok, true);
  assert.equal(v.rows.find((r) => r.metric === 'framesOver50').dropped, true);
});

test('the last words missing in any run fails', () => {
  assert.deepEqual(failed(check({ runs: [run(), run({ lastWordsShown: false }), run()], bytes: bytes() }, budget())), ['lastWordsShown']);
});

test('a crashed run or a page error fails', () => {
  const v = check({ runs: [run(), { error: 'run.mjs exit 1: boom' }, run()], bytes: bytes() }, budget());
  assert.ok(failed(v).includes('run 2'));
  assert.ok(failed(v).includes('lastWordsShown'));
  assert.deepEqual(failed(check({ runs: [run({ errors: ['TypeError: x'] }), run(), run()], bytes: bytes() }, budget())), ['run 1 page errors']);
  assert.equal(check({ runs: [], bytes: bytes() }, budget()).ok, false);
});

test('the summary table names the failing metric', () => {
  const v = check({ runs: [run({ p95FrameMs: 60 }), run({ p95FrameMs: 60 }), run()], bytes: bytes() }, budget());
  const t = summaryTable(v, { spread: { p95FrameMs: 250 } });
  assert.match(t, /UI performance budget: FAIL/);
  assert.match(t, /\| p95FrameMs \| 60 \| 60, 60, 17 \| 250% \| 40 \| \*\*FAIL\*\* \|/);
});

test('static imports and html assets: first load only, lazy chunks left out', () => {
  assert.deepEqual(staticImports('import{h as s,k as d}from"./g-a.js";import"./g-b.js";export*from"./g-c.js";const x=()=>import("./lazy.js")'), ['./g-a.js', './g-b.js', './g-c.js']);
  const html = '<script type="module" crossorigin src="/p/_a/g-e.js"></script><link rel="stylesheet" href="/p/_a/g-s.css"><link rel="icon" href="/p/_a/i.svg"><link rel="modulepreload" href="/p/_a/g-m.js">';
  assert.deepEqual(htmlAssets(html, '/p/_a/'), ['g-e.js', 'g-s.css', 'g-m.js']);
});

test('firstLoad and allFiles gzip what ships', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'uibudget-'));
  const w = (f, s) => fs.writeFileSync(path.join(dir, f), s);
  w('index.html', '<script type="module" src="/p/_a/e.js"></script><link rel="stylesheet" href="/p/_a/s.css">');
  w('e.js', 'import{a}from"./dep.js";const l=()=>import("./lazy.js");');
  w('dep.js', 'export const a=1;');
  w('lazy.js', 'x'.repeat(5000));
  w('s.css', '@font-face{src:url(/p/_a/f.woff2)}');
  w('f.woff2', 'font');
  const fl = firstLoad(dir, 'index.html', '/p/_a/', { fonts: true });
  assert.deepEqual(fl.files, ['index.html', 'dep.js', 'e.js', 'f.woff2', 's.css']);
  const gz = (f) => zlib.gzipSync(fs.readFileSync(path.join(dir, f)), { level: 9 }).length;
  assert.equal(fl.bytes, fl.files.reduce((n, f) => n + gz(f), 0));
  assert.deepEqual(firstLoad(dir, 'index.html', '/p/_a/').files, ['index.html', 'dep.js', 'e.js', 's.css']);
  assert.deepEqual(allFiles(dir, ['.js']).files, ['dep.js', 'e.js', 'lazy.js']);
  fs.rmSync(dir, { recursive: true });
});

test('the budget file sets a limit for every metric it does not drop, and keeps the guest page under 80 KB gz', () => {
  const b = JSON.parse(fs.readFileSync(path.join(root, 'perf', 'ui_budget.json'), 'utf8'));
  const dropped = b.dropped ?? {};
  for (const k of FRAME_METRICS) if (!(k in dropped)) assert.equal(typeof b.frame[k]?.max, 'number', k);
  for (const k of BYTE_METRICS) if (!(k in dropped)) assert.equal(typeof b.bytes[k]?.max, 'number', k);
  assert.ok(!('guestInitialGz' in dropped));
  assert.ok(b.bytes.guestInitialGz.max <= 80 * 1024, 'pocket-apps: the guest page is at most 80 KB gzipped');
  for (const [k, why] of Object.entries(dropped)) assert.ok(FRAME_METRICS.includes(k) && String(why).length > 10, `dropped ${k} needs its numbers`);
});

// Living avatars: the 30-bot life case (perf/stream/life.mjs). Commits and timers are the worst run, p95 the median.
const lifeRun = (o = {}) => ({ lifeCommits: 0, lifeTimers: 1, lifeP95FrameMs: 17, lifeEvents: 60, errors: [], ...o });
const lifeBudget = () => budget({ life: { lifeCommits: { max: 0 }, lifeTimers: { max: 1 }, lifeP95FrameMs: { max: 19.4 } } });
const three = () => [run(), run(), run()];

test('life: within the budget passes; one React commit or a second timer in any run fails', () => {
  assert.equal(check({ runs: three(), bytes: bytes(), life: [lifeRun(), lifeRun(), lifeRun()] }, lifeBudget()).ok, true);
  assert.deepEqual(failed(check({ runs: three(), bytes: bytes(), life: [lifeRun(), lifeRun({ lifeCommits: 1 }), lifeRun()] }, lifeBudget())), ['lifeCommits']);
  assert.deepEqual(failed(check({ runs: three(), bytes: bytes(), life: [lifeRun({ lifeTimers: 2 }), lifeRun(), lifeRun()] }, lifeBudget())), ['lifeTimers']);
  assert.deepEqual(failed(check({ runs: three(), bytes: bytes(), life: [lifeRun({ lifeP95FrameMs: 30 }), lifeRun({ lifeP95FrameMs: 30 }), lifeRun()] }, lifeBudget())), ['lifeP95FrameMs']);
});

test('life: no runs, a crashed run, or avatars that never moved fail', () => {
  assert.ok(failed(check({ runs: three(), bytes: bytes() }, lifeBudget())).includes('lifeEvents'));
  assert.ok(failed(check({ runs: three(), bytes: bytes(), life: [lifeRun(), { error: 'life.mjs exit 1' }, lifeRun()] }, lifeBudget())).includes('life run 2'));
  assert.deepEqual(failed(check({ runs: three(), bytes: bytes(), life: [lifeRun(), lifeRun({ lifeEvents: 0 }), lifeRun()] }, lifeBudget())), ['lifeEvents']);
});

test('the budget file has the life case: 0 commits, 1 timer, a p95 limit', () => {
  const b = JSON.parse(fs.readFileSync(path.join(root, 'perf', 'ui_budget.json'), 'utf8'));
  for (const k of LIFE_METRICS) assert.equal(typeof b.life?.[k]?.max, 'number', k);
  assert.equal(b.life.lifeCommits.max, 0);
  assert.equal(b.life.lifeTimers.max, 1);
});

test('CI runs the budget as a blocking step', () => {
  const ci = fs.readFileSync(path.join(root, '..', '.github', 'workflows', 'ci.yml'), 'utf8');
  const job = ci.slice(ci.indexOf('\n  ui-perf:'), ci.indexOf('\n  site:'));
  assert.match(job, /name: ui-perf\n/);
  assert.match(job, /node perf\/ui_budget\.mjs /);
  assert.doesNotMatch(job, /continue-on-error|--calibrate|\|\| true/);
  assert.match(job, /name: ui-perf-results/);
  const required = fs.readFileSync(path.join(root, '..', 'scripts', 'release_required_checks.txt'), 'utf8').split('\n').map((l) => l.trim());
  assert.ok(required.includes('ui-perf'), 'a release waits for ui-perf');
});
