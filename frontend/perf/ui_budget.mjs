// ui-perf-budget (perf-regressions part D): the UI's speed and size budget, a blocking CI step (ci.yml, job ui-perf).
//   1. Runs the stream harness (perf/stream/README.md) `--runs` times (default 3) and takes the median of
//      commitsPerSecond, p95FrameMs and framesOver50; lastWordsShown must be true in every run.
//   2. Gzips (level 9) what the normal build ships: the app's first load (dist/index.html plus the JS and CSS it pulls
//      in statically), every app JS/CSS chunk, the guest page's first load (dist/pocket/index.html, its static JS/CSS and
//      the fonts that CSS names) and the whole guest bundle.
//   3. Checks both against perf/ui_budget.json. Over a limit, a crashed run, a page error or a missing last word fails
//      (exit 1). Writes --out (JSON) and a table to $GITHUB_STEP_SUMMARY.
// Usage (from frontend/, after `npm run build` and `npx vite build --config perf/stream/vite.config.mts`):
//   node perf/ui_budget.mjs [--dist dist] [--budget perf/ui_budget.json] [--runs 3] [--out results.json] [--calibrate]
// --calibrate measures and reports without limits (used only to set them; CI never passes it).
// Needs PLAYWRIGHT_CORE and CHROMIUM for the harness, like run.mjs.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const FRAME_METRICS = ['commitsPerSecond', 'p95FrameMs', 'framesOver50'];
export const BYTE_METRICS = ['appInitialGz', 'appAllGz', 'guestInitialGz', 'guestAllGz'];
// Living avatars (autonomy/living-avatars): 30 bots on the shared clock for 20 s (perf/stream/life.mjs), once per run.
// Commits and timers are the worst run (life must never render React or hold a second timer); p95 is the median.
export const LIFE_METRICS = ['lifeCommits', 'lifeTimers', 'lifeP95FrameMs'];
const LIFE_WORST = new Set(['lifeCommits', 'lifeTimers']);

export const gz = (buf) => zlib.gzipSync(buf, { level: 9 }).length;

export const median = (xs) => {
  const s = xs.slice().sort((a, b) => a - b);
  if (!s.length) return NaN;
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

// (max - min) / median, in percent; 0 when every value is 0.
export const spreadPct = (xs) => {
  const m = median(xs);
  const d = Math.max(...xs) - Math.min(...xs);
  if (d === 0) return 0;
  return m === 0 ? Infinity : (100 * d) / m;
};

// Static imports of a built ES module: `import{a}from"./x.js"`, `import"./x.js"`, `export*from"./x.js"`.
// Dynamic `import("./x.js")` is a lazy chunk and not part of the first load.
export const staticImports = (code) => {
  const out = new Set();
  for (const m of code.matchAll(/\b(?:import|export)\s*(?:[^"'`();]*?\bfrom\s*)?["'](\.{1,2}\/[^"']+\.js)["']/g)) out.add(m[1]);
  return [...out];
};

// Files an HTML page loads up front: module scripts, stylesheets and modulepreloads under `prefix`.
export const htmlAssets = (html, prefix) => {
  const out = [];
  for (const tag of html.match(/<(?:script|link)\b[^>]*>/g) ?? []) {
    const url = tag.match(/\b(?:src|href)="([^"]+)"/)?.[1];
    if (!url || !url.startsWith(prefix)) continue;
    if (tag.startsWith('<link') && !/rel="(?:stylesheet|modulepreload)"/.test(tag)) continue;
    out.push(url.slice(prefix.length));
  }
  return out;
};

// fonts=true also counts the font files the first-load CSS names (the guest budget includes its Inter subset).
export const firstLoad = (dir, htmlName, prefix, { fonts = false } = {}) => {
  const html = fs.readFileSync(path.join(dir, htmlName));
  const files = new Set();
  const visit = (rel) => {
    const abs = path.normalize(path.join(dir, rel));
    if (files.has(abs) || !fs.existsSync(abs)) return;
    files.add(abs);
    const text = fs.readFileSync(abs, 'utf8');
    if (abs.endsWith('.js')) for (const imp of staticImports(text)) visit(path.relative(dir, path.join(path.dirname(abs), imp)));
    if (abs.endsWith('.css') && fonts)
      for (const m of text.matchAll(/url\(\s*["']?([^"')]+\.woff2)["']?\s*\)/g)) {
        const u = m[1];
        visit(u.startsWith(prefix) ? u.slice(prefix.length) : path.relative(dir, path.join(path.dirname(abs), u)));
      }
  };
  for (const rel of htmlAssets(html.toString(), prefix)) visit(rel);
  return { bytes: gz(html) + [...files].reduce((n, f) => n + gz(fs.readFileSync(f)), 0), files: [htmlName, ...[...files].map((f) => path.relative(dir, f)).sort()] };
};

export const allFiles = (dir, exts) => {
  const files = fs.existsSync(dir) ? fs.readdirSync(dir, { recursive: true }).filter((f) => exts.some((e) => f.endsWith(e))).sort() : [];
  return { bytes: files.reduce((n, f) => n + gz(fs.readFileSync(path.join(dir, f))), 0), files };
};

export const measureBytes = (dist) => {
  const pocket = path.join(dist, 'pocket');
  return {
    appInitialGz: firstLoad(dist, 'index.html', '/'),
    appAllGz: allFiles(path.join(dist, 'assets'), ['.js', '.css']),
    guestInitialGz: firstLoad(pocket, 'index.html', '/p/_a/', { fonts: true }),
    guestAllGz: allFiles(pocket, ['.js', '.css', '.html', '.woff2']),
  };
};

// One verdict per metric. budget: { frame: { name: { max } }, bytes: { name: { max } }, dropped: { name: why } }.
// A metric in FRAME_METRICS/BYTE_METRICS with no limit and not dropped fails: a budget can't skip a metric by omission.
export const check = ({ runs, bytes, life }, budget) => {
  const rows = [];
  const dropped = budget.dropped ?? {};
  for (const [i, r] of runs.entries()) {
    if (r.error) rows.push({ metric: `run ${i + 1}`, value: r.error, limit: 'completes', ok: false });
    else if (r.errors?.length) rows.push({ metric: `run ${i + 1} page errors`, value: r.errors.join(' | ').slice(0, 300), limit: 'none', ok: false });
  }
  const good = runs.filter((r) => !r.error);
  rows.push({ metric: 'lastWordsShown', value: good.map((r) => r.lastWordsShown).join(', ') || 'no runs', limit: 'true in every run', ok: good.length === runs.length && good.length > 0 && good.every((r) => r.lastWordsShown === true) });
  const med = {};
  for (const k of FRAME_METRICS) {
    const xs = good.map((r) => r[k]).filter((x) => typeof x === 'number');
    med[k] = xs.length === good.length && xs.length ? median(xs) : NaN;
    const lim = budget.frame?.[k]?.max;
    if (k in dropped) rows.push({ metric: k, value: med[k], limit: `dropped: ${dropped[k]}`, ok: true, dropped: true, runs: xs });
    else rows.push({ metric: k, value: med[k], limit: lim, ok: Number.isFinite(med[k]) && typeof lim === 'number' && med[k] <= lim, runs: xs });
  }
  // The life case: checked whenever the budget has it (a budget with "life" and no life runs fails).
  if (budget.life) {
    const lr = life ?? [];
    for (const [i, r] of lr.entries()) {
      if (r.error) rows.push({ metric: `life run ${i + 1}`, value: r.error, limit: 'completes', ok: false });
      else if (r.errors?.length) rows.push({ metric: `life run ${i + 1} page errors`, value: r.errors.join(' | ').slice(0, 300), limit: 'none', ok: false });
    }
    const lg = lr.filter((r) => !r.error);
    rows.push({ metric: 'lifeEvents', value: lg.map((r) => r.lifeEvents).join(', ') || 'no runs', limit: '> 0 in every run', ok: lg.length > 0 && lg.length === lr.length && lg.every((r) => r.lifeEvents > 0) });
    for (const k of LIFE_METRICS) {
      const xs = lg.map((r) => r[k]).filter((x) => typeof x === 'number');
      const v = xs.length === lg.length && xs.length ? (LIFE_WORST.has(k) ? Math.max(...xs) : median(xs)) : NaN;
      med[k] = v;
      const lim = budget.life[k]?.max;
      rows.push({ metric: k, value: v, limit: lim, ok: Number.isFinite(v) && typeof lim === 'number' && v <= lim, runs: xs });
    }
  }
  for (const k of BYTE_METRICS) {
    const v = bytes[k]?.bytes;
    const lim = budget.bytes?.[k]?.max;
    if (k in dropped) rows.push({ metric: k, value: v, limit: `dropped: ${dropped[k]}`, ok: true, dropped: true });
    else rows.push({ metric: k, value: v, limit: lim, ok: typeof v === 'number' && typeof lim === 'number' && v <= lim });
  }
  return { ok: rows.every((r) => r.ok), rows, median: med };
};

export const summaryTable = ({ ok, rows }, { calibrate = false, spread = {} } = {}) => {
  const fmt = (v) => (typeof v === 'number' ? (Number.isInteger(v) ? String(v) : v.toFixed(1)) : String(v ?? '—'));
  const lines = [
    `### UI performance budget: ${calibrate ? 'CALIBRATION (no limits checked)' : ok ? 'PASS' : 'FAIL'}`,
    '',
    '| metric | value (median for frame metrics) | runs | spread | limit | result |',
    '|---|---|---|---|---|---|',
  ];
  for (const r of rows) {
    const sp = r.metric in spread ? `${Number.isFinite(spread[r.metric]) ? spread[r.metric].toFixed(0) : '∞'}%` : '';
    const res = calibrate ? '—' : r.dropped ? 'not checked' : r.ok ? 'PASS' : '**FAIL**';
    lines.push(`| ${r.metric} | ${fmt(r.value)} | ${r.runs ? r.runs.map(fmt).join(', ') : ''} | ${sp} | ${calibrate ? '—' : fmt(r.limit)} | ${res} |`);
  }
  return lines.join('\n') + '\n';
};

const runHarness = (args, script = 'run.mjs') => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const p = spawnSync(process.execPath, [path.join(here, 'stream', script), ...args], { encoding: 'utf8', timeout: 180000, maxBuffer: 1 << 24 });
  const line = (p.stdout ?? '').trim().split('\n').reverse().find((l) => l.startsWith('{'));
  if (p.status !== 0 || !line) return { error: `${script} exit ${p.status ?? p.signal}: ${(p.stderr || p.stdout || '').trim().slice(-400)}` };
  try {
    return JSON.parse(line);
  } catch (e) {
    return { error: `unreadable output: ${line.slice(0, 200)}` };
  }
};

const main = () => {
  const arg = (k, d) => {
    const i = process.argv.indexOf(`--${k}`);
    return i > 0 ? process.argv[i + 1] : d;
  };
  const calibrate = process.argv.includes('--calibrate');
  const dist = arg('dist', 'dist');
  const budgetPath = arg('budget', path.join(path.dirname(fileURLToPath(import.meta.url)), 'ui_budget.json'));
  const budget = JSON.parse(fs.readFileSync(budgetPath, 'utf8'));
  const runsN = Number(arg('runs', budget.harness?.runs ?? 3));
  const hargs = budget.harness?.args ?? ['--n', '300', '--tokens', '2000', '--rate', '50'];
  const out = arg('out', '');

  const bytes = measureBytes(dist);
  for (const k of BYTE_METRICS) console.log(`${k}: ${bytes[k].bytes} B gz (${bytes[k].files.length} files)`);
  const runs = [];
  for (let i = 0; i < runsN; i++) {
    const t = Date.now();
    const r = runHarness(hargs);
    runs.push(r);
    console.log(`run ${i + 1}/${runsN} (${((Date.now() - t) / 1000).toFixed(0)} s): ${r.error ?? JSON.stringify(r)}`);
  }
  const life = [];
  if (budget.life)
    for (let i = 0; i < runsN; i++) {
      const t = Date.now();
      const r = runHarness(budget.life.args ?? ['--seconds', '20'], 'life.mjs');
      life.push(r);
      console.log(`life ${i + 1}/${runsN} (${((Date.now() - t) / 1000).toFixed(0)} s): ${r.error ?? JSON.stringify(r)}`);
    }
  const verdict = check({ runs, bytes, life }, budget);
  const ok = (xs) => xs.filter((r) => !r.error);
  const spread = {
    ...Object.fromEntries(FRAME_METRICS.map((k) => [k, spreadPct(ok(runs).map((r) => r[k]))])),
    ...(budget.life ? Object.fromEntries(LIFE_METRICS.map((k) => [k, spreadPct(ok(life).map((r) => r[k]))])) : {}),
  };
  const table = summaryTable(verdict, { calibrate, spread });
  console.log(table);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, table);
  const result = {
    ok: verdict.ok,
    calibrate,
    commit: process.env.GITHUB_SHA ?? null,
    run: process.env.GITHUB_RUN_ID ? `${process.env.GITHUB_RUN_ID}/${process.env.GITHUB_RUN_ATTEMPT ?? 1}` : null,
    harnessArgs: hargs,
    median: verdict.median,
    spreadPct: spread,
    bytes: Object.fromEntries(BYTE_METRICS.map((k) => [k, bytes[k]])),
    checks: verdict.rows,
    runs,
    life,
  };
  if (out) fs.writeFileSync(out, JSON.stringify(result, null, 1));
  if (calibrate) {
    // Calibration still fails on a broken run: limits must come from runs that worked.
    const broken = verdict.rows.filter((r) => !r.ok && (r.metric.startsWith('run ') || r.metric === 'lastWordsShown'));
    process.exit(broken.length ? 1 : 0);
  }
  process.exit(verdict.ok ? 0 : 1);
};

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) main();
